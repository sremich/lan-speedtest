//! Entry point: load config, build the router, serve until signalled.

use std::path::PathBuf;
use std::sync::Arc;

use tracing_subscriber::EnvFilter;

use lan_speedtest::routes;
use lan_speedtest::{AppState, Config, PayloadSource, GIT_SHA, VERSION};

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

    let state = AppState {
        payload,
        config: Arc::new(config),
    };

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!("listening on {bind}");

    axum::serve(listener, routes::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

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
