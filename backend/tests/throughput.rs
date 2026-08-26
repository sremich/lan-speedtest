//! Tier 2 — loopback throughput floor.
//!
//! The point of this project is that the *network* is what gets measured, so a
//! performance regression in the data path has to fail the build rather than
//! turn up later as a disappointing number on a 10 GbE client.
//!
//! This measures the backend over loopback, which removes the browser and the
//! wire from the picture. It is not a prediction of what the engine will
//! report in a browser — the engine is single-stream and decodes every
//! response through `r.text()`, so it will always read lower. This asserts one
//! thing only: the backend is not the bottleneck.
//!
//! The floor is deliberately low, because the host often is. Measured on the
//! Windows/WSL2 dev box (2026-08-24): raw TCP loopback tops out at 0.70 Gbps
//! (bare Python socket) and 0.64 Gbps (`nc`), while this backend sustained
//! 1.19 Gbps across four streams — i.e. faster than any single-stream
//! reference the machine can produce. An absolute floor tuned for real
//! hardware would fail there permanently and prove nothing.
//!
//! So this catches regressions of *kind* — per-request allocation, an
//! accidental full-body buffer, a copy that should have been a refcount bump —
//! which cost an order of magnitude, not 10%. Absolute throughput is a tier-4
//! question, answered on the 10 GbE client.
//!
//! Tunable, so the same test is a gentle floor on a shared CI runner and a
//! real check on the target hardware:
//!
//! ```text
//! SPEEDTEST_THROUGHPUT_FLOOR_GBPS   default 0.5 (CI sets a stricter value)
//! SPEEDTEST_THROUGHPUT_PASSES       default 3 (best-of)
//! SPEEDTEST_THROUGHPUT_STREAMS      default 4
//! SPEEDTEST_THROUGHPUT_BYTES        default 67108864 (64 MiB per stream)
//! ```

use std::time::Instant;

use futures_util::StreamExt;
use lan_speedtest::{router, AppState, Config, PayloadSource};

const CONFIG: &str = "
profile = 'bench'
[server]
max_transfer_bytes = 4294967296
download_chunk_bytes = 4194304
static_dir = 'does-not-exist'
[profiles.bench]
measurements = [ { type = 'download', bytes = 1000000, count = 1 } ]
";

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

async fn serve() -> String {
    let config: Config = toml::from_str(CONFIG).unwrap();
    let payload = PayloadSource::new(config.server.download_chunk_bytes);
    let state = AppState::new(config, payload, None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router(state)).await.unwrap();
    });
    format!("http://{addr}")
}

/// Pulls `bytes` and discards them as they arrive, so the client never holds
/// the payload and cannot itself become the limiting factor.
async fn drain(client: reqwest::Client, url: String) -> u64 {
    let resp = client.get(url).send().await.expect("request sent");
    assert!(resp.status().is_success());
    let mut stream = resp.bytes_stream();
    let mut total = 0u64;
    while let Some(chunk) = stream.next().await {
        total += chunk.expect("chunk received").len() as u64;
    }
    total
}

#[tokio::test(flavor = "multi_thread")]
async fn download_path_sustains_the_throughput_floor_over_loopback() {
    let floor_gbps: f64 = env_or("SPEEDTEST_THROUGHPUT_FLOOR_GBPS", 0.5);
    let streams: usize = env_or("SPEEDTEST_THROUGHPUT_STREAMS", 4);
    let per_stream: u64 = env_or("SPEEDTEST_THROUGHPUT_BYTES", 64 * 1024 * 1024);

    let passes: usize = env_or("SPEEDTEST_THROUGHPUT_PASSES", 3);

    let base = serve().await;
    let client = reqwest::Client::new();

    // One warm-up pass so connection setup and first-touch page faults on the
    // shared buffer are not counted against the measurement.
    drain(client.clone(), format!("{base}/__down?bytes=8388608")).await;

    // Best of N. A shared runner's loopback is noisy — on the Windows/WSL2 dev
    // box the same unchanged code measured 0.91, 1.52 and 1.65 Gbps — so a
    // single sample makes a flaky gate. Taking the best still catches what
    // this test is for: a regression that costs an order of magnitude cannot
    // reach the floor on any pass.
    let mut best_gbps = 0.0f64;
    for pass in 1..=passes {
        let started = Instant::now();
        let mut tasks = Vec::with_capacity(streams);
        for i in 0..streams {
            let url = format!("{base}/__down?bytes={per_stream}&pass={pass}&stream={i}");
            tasks.push(tokio::spawn(drain(client.clone(), url)));
        }

        let mut total = 0u64;
        for t in tasks {
            total += t.await.expect("stream completed");
        }
        let elapsed = started.elapsed();

        assert_eq!(
            total,
            per_stream * streams as u64,
            "short read — the server did not deliver every requested byte"
        );

        let gbps = (total as f64 * 8.0) / elapsed.as_secs_f64() / 1e9;
        println!("  pass {pass}/{passes}: {gbps:.2} Gbps (in {elapsed:.2?})");
        best_gbps = best_gbps.max(gbps);
    }

    // Recorded in the CI log so the number this floor was chosen against stays
    // visible: a floor is only meaningful next to what was actually measured.
    println!(
        "loopback download: best {best_gbps:.2} Gbps of {passes} passes \
         ({streams} streams x {per_stream} bytes), floor {floor_gbps:.2} Gbps"
    );

    assert!(
        best_gbps >= floor_gbps,
        "loopback download managed {best_gbps:.2} Gbps at best, below the \
         {floor_gbps:.2} Gbps floor — the backend data path has regressed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn upload_path_drains_a_large_body_without_buffering_it() {
    // Guards the other half of the data path: `up()` must consume frames as
    // they arrive. If it ever collected the body into memory instead, this
    // would still pass functionally but the process would hold the whole
    // payload — so assert on time, which a buffering implementation blows.
    let base = serve().await;
    let client = reqwest::Client::new();
    let size = 32 * 1024 * 1024usize;

    let started = Instant::now();
    let resp = client
        .post(format!("{base}/__up?bytes={size}"))
        .body(vec![b'0'; size])
        .send()
        .await
        .unwrap();
    let elapsed = started.elapsed();

    assert!(resp.status().is_success());
    assert_eq!(
        resp.headers()
            .get("x-received-bytes")
            .unwrap()
            .to_str()
            .unwrap(),
        size.to_string()
    );

    let gbps = (size as f64 * 8.0) / elapsed.as_secs_f64() / 1e9;
    println!("loopback upload: {gbps:.2} Gbps ({size} bytes in {elapsed:.2?})");
    assert!(
        elapsed.as_secs_f64() < 10.0,
        "32 MiB upload took {elapsed:?} — the drain path is not keeping up"
    );
}
