//! Tier 2 — HTTPS actually serves.
//!
//! The service terminates TLS itself rather than sitting behind a proxy, so
//! "the certificate loads and a browser can connect" is our problem to prove.
//! These tests generate a throwaway self-signed certificate with `openssl` and
//! drive the real server over HTTPS; they skip themselves if `openssl` is
//! absent rather than failing for an unrelated reason.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use lan_speedtest::{router, AppState, Config, PayloadSource};

const CONFIG: &str = "
profile = 'test'
[server]
static_dir = 'does-not-exist'
[profiles.test]
measurements = [ { type = 'download', bytes = 1000, count = 1 } ]
";

fn openssl_available() -> bool {
    Command::new("openssl")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A throwaway self-signed certificate for 127.0.0.1.
fn self_signed(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let cert = dir.join("fullchain.pem");
    let key = dir.join("privkey.pem");

    let out = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=IP:127.0.0.1,DNS:localhost",
            "-keyout",
        ])
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .output()
        .ok()?;

    out.status.success().then_some((cert, key))
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("speedtest-tls-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("creating a temp dir");
    dir
}

/// Boots the server with TLS on an ephemeral port; returns the HTTPS base URL.
async fn serve_tls(cert: &Path, key: &Path) -> String {
    // Matches what main.rs does: name the provider before any TLS work.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut config: Config = toml::from_str(CONFIG).unwrap();
    config.server.tls_cert_file = Some(cert.display().to_string());
    config.server.tls_key_file = Some(key.display().to_string());

    let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key)
        .await
        .expect("loading the generated certificate");

    let state = AppState {
        payload: PayloadSource::new(4096),
        config: Arc::new(config),
        history: None,
    };

    // Bind first to learn the port, then hand the socket to axum-server.
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = std_listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum_server::from_tcp_rustls(std_listener, tls)
            .serve(router(state).into_make_service())
            .await
            .unwrap();
    });

    // Give the listener a moment to come up before the first connect.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    format!("https://{addr}")
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        // The certificate is self-signed by construction here; the point of
        // the test is that TLS works at all, not that this cert is trusted.
        .danger_accept_invalid_certs(true)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap()
}

#[tokio::test]
async fn the_service_serves_https_with_a_configured_certificate() {
    if !openssl_available() {
        eprintln!("skipping: openssl not available");
        return;
    }
    let dir = temp_dir("serves");
    let Some((cert, key)) = self_signed(&dir) else {
        eprintln!("skipping: could not generate a test certificate");
        return;
    };

    let base = serve_tls(&cert, &key).await;
    let res = client()
        .get(format!("{base}/api/health"))
        .send()
        .await
        .expect("HTTPS request should succeed");

    assert!(res.status().is_success());
    assert_eq!(res.text().await.unwrap(), "ok");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_measurement_contract_still_holds_over_https() {
    // TLS must not change what the engine sees. In production every browser
    // request goes over HTTPS, so this is the path that actually matters.
    if !openssl_available() {
        eprintln!("skipping: openssl not available");
        return;
    }
    let dir = temp_dir("contract");
    let Some((cert, key)) = self_signed(&dir) else {
        eprintln!("skipping: could not generate a test certificate");
        return;
    };

    let base = serve_tls(&cert, &key).await;
    let c = client();

    let res = c
        .get(format!("{base}/__down?bytes=100000"))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());

    let server_timing = res
        .headers()
        .get("server-timing")
        .expect("server-timing must survive TLS")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        server_timing.starts_with("cfRequestDuration;dur="),
        "{server_timing}"
    );

    let cache_control = res
        .headers()
        .get("cache-control")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(cache_control.contains("no-store"));

    assert_eq!(res.bytes().await.unwrap().len(), 100_000);

    // And uploads, which are the half that depends on response timing.
    let up = c
        .post(format!("{base}/__up?bytes=50000"))
        .body("0".repeat(50_000))
        .send()
        .await
        .unwrap();
    assert!(up.status().is_success());
    assert_eq!(
        up.headers()
            .get("x-received-bytes")
            .unwrap()
            .to_str()
            .unwrap(),
        "50000"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_missing_certificate_file_is_reported_clearly() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    // The usual cause is a bind mount that is not there, and a vague error
    // sends you looking in the wrong place.
    let err = axum_server::tls_rustls::RustlsConfig::from_pem_file(
        "/nonexistent/fullchain.pem",
        "/nonexistent/privkey.pem",
    )
    .await
    .expect_err("loading a missing certificate should fail");

    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("no such file")
            || msg.to_lowercase().contains("cannot find")
            || msg.to_lowercase().contains("not found"),
        "unhelpful error for a missing certificate: {msg}"
    );
}
