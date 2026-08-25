//! Entry point: load config, build the router, serve until signalled.

use std::path::PathBuf;
use std::sync::Arc;

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
