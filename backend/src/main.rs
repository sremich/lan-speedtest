//! Entry point: load config, build the router, serve until signalled.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::serve::ListenerExt;
use tracing_subscriber::EnvFilter;

use lan_speedtest::net;
use lan_speedtest::routes;
use lan_speedtest::{AppState, Config, History, PayloadSource, GIT_SHA, VERSION};

const DEFAULT_CONFIG_PATH: &str = "config/speedtest.toml";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("SPEEDTEST_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // `--version` has to work without a config file present.
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("lan-speedtest {VERSION} ({GIT_SHA})");
        return Ok(());
    }

    let config_path: PathBuf = std::env::var("SPEEDTEST_CONFIG")
        .unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string())
        .into();

    let config = Config::load(&config_path).map_err(|e| {
        tracing::error!("{config_path:?}: {e}");
        e
    })?;

    let payload = PayloadSource::new(config.server.download_chunk_bytes);
    let bind = config.server.bind.clone();
    let profile_name = config.profile.clone();
    let static_dir = config.server.static_dir.clone();
    let turn_enabled = config.turn.enabled;
    let stages = config.active_profile().measurements.len();

    tracing::info!(
        version = VERSION,
        git_sha = GIT_SHA,
        profile = %profile_name,
        stages,
        payload_buffer_bytes = payload.chunk_len(),
        static_dir = %static_dir,
        packet_loss = turn_enabled,
        "starting"
    );
    if !PathBuf::from(&static_dir).is_dir() {
        tracing::warn!("{static_dir}: no such directory — API only, no front end");
    }

    let tls = config
        .server
        .tls()
        .map(|(c, k)| (c.to_string(), k.to_string(), config.server.tls_bind.clone()));

    // History is optional: an empty path turns it off, which is how the
    // contract tests and the e2e suite run.
    let history = if config.server.history_db.trim().is_empty() {
        tracing::info!("history disabled (no database path configured)");
        None
    } else {
        let path = PathBuf::from(&config.server.history_db);
        let db = History::open(&path).map_err(|e| {
            tracing::error!("could not open the history database at {path:?}: {e}");
            e
        })?;
        tracing::info!(
            "history at {} ({} runs stored)",
            path.display(),
            db.count().unwrap_or(0)
        );
        Some(Arc::new(db))
    };

    // Database maintenance, if any of it was asked for. Runs once at startup
    // and then daily — a long-lived homelab service is restarted rarely enough
    // that startup-only would let a database grow for months between prunes.
    if let Some(db) = history.clone() {
        let runs_days = config.server.retain_runs_days;
        let samples_days = config.server.retain_samples_days;

        // Checked here rather than on the first pass a day from now: an
        // unwritable directory, or one that would put the snapshot on top of
        // the live database, should stop the service starting.
        let backup_dir = match config.server.backup_dir() {
            None => None,
            Some(dir) => {
                let destination = db.prepare_snapshot_dir(dir).map_err(|e| {
                    tracing::error!("server.history_backup_dir: {e}");
                    e
                })?;
                tracing::info!("history snapshots to {}", destination.display());
                Some(dir.to_path_buf())
            }
        };

        if runs_days > 0 || samples_days > 0 || backup_dir.is_some() {
            spawn_maintenance(db, runs_days, samples_days, backup_dir);
        }
    }

    let state = AppState::new(config, payload, history);
    let app = routes::router(state);

    // Plain HTTP always listens: it is what the container health check uses,
    // and what the e2e suite drives. HTTPS is added alongside it when a
    // certificate is configured, rather than replacing it.
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!("listening on http://{bind}");

    let http = {
        let app = app.clone();
        // Nagle off — see net.rs. A 40 ms delayed-ACK stall on a small
        // response is reported to the user as network latency.
        let listener = listener.tap_io(net::set_nodelay);
        tokio::spawn(async move {
            // `_with_connect_info` is what makes the peer address available;
            // without it every stored run would be attributed to "unknown".
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown_signal())
            .await
        })
    };

    match tls {
        None => {
            tracing::info!("no certificate configured — HTTP only");
            http.await??;
        }
        Some((cert, key, tls_bind)) => {
            // rustls refuses to guess when more than one crypto provider is
            // linked in, so name one before any TLS work happens.
            let _ = rustls::crypto::ring::default_provider().install_default();

            let tls_config = axum_server::tls_rustls::RustlsConfig::from_pem_file(&cert, &key)
                .await
                .map_err(|e| {
                    // A missing or unreadable certificate is worth naming
                    // precisely: it is usually a bind mount that is not there.
                    format!("could not load TLS material ({cert}, {key}): {e}")
                })?;

            let addr: std::net::SocketAddr = tls_bind.parse()?;
            tracing::info!("listening on https://{tls_bind}");

            let handle = axum_server::Handle::new();
            let shutdown_handle = handle.clone();
            tokio::spawn(async move {
                shutdown_signal().await;
                shutdown_handle.graceful_shutdown(Some(std::time::Duration::from_secs(5)));
            });

            // Not `bind_rustls`: that wraps `DefaultAcceptor`, which passes
            // the socket through untouched and leaves Nagle on. See net.rs.
            axum_server::bind(addr)
                .acceptor(net::tls_acceptor(tls_config))
                .handle(handle)
                .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                .await?;
            http.abort();
        }
    }

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => tracing::warn!("SIGTERM handler unavailable: {e}"),
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!("shutting down");
}

/// Prunes, reclaims and snapshots — now, and once a day thereafter.
///
/// The cutoffs are recomputed on every pass rather than once at startup: a
/// window computed at boot would stop moving, so a service left running for a
/// month would quietly stop pruning anything.
///
/// A deployment can want the snapshot without either retention window, so this
/// runs whenever any of the three is configured. With both windows at zero the
/// prune deletes nothing and the pass is a WAL truncation and a copy, which is
/// exactly what a nightly backup should be.
fn spawn_maintenance(
    db: Arc<History>,
    runs_days: u32,
    samples_days: u32,
    backup_dir: Option<PathBuf>,
) {
    tracing::info!(
        "retention: runs {} days, samples {} days (0 = keep)",
        runs_days,
        samples_days
    );

    tokio::spawn(async move {
        const DAY: Duration = Duration::from_secs(24 * 60 * 60);
        loop {
            let cutoff = |days: u32| {
                if days == 0 {
                    return None;
                }
                let at = time::OffsetDateTime::now_utc() - time::Duration::days(i64::from(days));
                at.format(&time::format_description::well_known::Rfc3339)
                    .ok()
            };

            let runs_cutoff = cutoff(runs_days);
            let samples_cutoff = cutoff(samples_days);

            // Blocking work on a blocking thread: rusqlite is synchronous, and
            // a DELETE over a large table — or a VACUUM INTO of the whole
            // database — would otherwise stall the runtime that is meant to be
            // serving a measurement at the time.
            let db = db.clone();
            let dir = backup_dir.clone();
            let pass = tokio::task::spawn_blocking(move || {
                let pruned = db.prune(runs_cutoff.as_deref(), samples_cutoff.as_deref())?;
                // After the prune, so the copy is of the pruned database and
                // not of the pages that were about to go.
                let snapshot = match &dir {
                    Some(dir) => Some(db.snapshot(dir)?),
                    None => None,
                };
                Ok::<_, lan_speedtest::history::HistoryError>((pruned, snapshot))
            })
            .await;

            match pass {
                Ok(Ok((p, snapshot))) => {
                    if p.runs_deleted > 0 || p.samples_cleared > 0 || p.bytes_reclaimed > 0 {
                        tracing::info!(
                            "retention: deleted {} run(s), released samples from {}, \
                             returned {} KiB to the filesystem",
                            p.runs_deleted,
                            p.samples_cleared,
                            p.bytes_reclaimed / 1024
                        );
                    } else {
                        tracing::debug!("retention: nothing to prune");
                    }
                    if let Some(bytes) = snapshot {
                        tracing::info!("history snapshot written ({} KiB)", bytes / 1024);
                    }
                }
                Ok(Err(e)) => tracing::warn!("maintenance pass failed: {e}"),
                Err(e) => tracing::warn!("maintenance task panicked: {e}"),
            }

            tokio::time::sleep(DAY).await;
        }
    });
}
