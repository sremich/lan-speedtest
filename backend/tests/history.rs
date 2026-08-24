//! Tier 1 — the history endpoints, over a real socket.
//!
//! The milestone's done-when is "after three test runs from two clients, the
//! history page shows all three with correct client attribution", so that is
//! asserted directly against the HTTP surface rather than the storage layer.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use lan_speedtest::{router, AppState, Config, History, PayloadSource};

const CONFIG: &str = "
profile = 'test'
[server]
static_dir = 'does-not-exist'
[profiles.test]
measurements = [ { type = 'download', bytes = 1000, count = 1 } ]
";

/// Boots the router with history enabled and connect info wired, which is what
/// makes the peer address available for attribution.
async fn serve(history: Option<Arc<History>>) -> String {
    let config: Config = toml::from_str(CONFIG).unwrap();
    let state = AppState {
        payload: PayloadSource::new(4096),
        config: Arc::new(config),
        history,
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router(state).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    format!("http://{addr}")
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
}

fn submission(download: f64, profile: &str) -> serde_json::Value {
    serde_json::json!({
        "summary": {
            "download": download,
            "upload": 5.0e8,
            "latency": 0.6,
            "jitter": 0.1,
            "downLoadedLatency": 1.1,
            "upLoadedLatency": 0.9,
            "packetLoss": 0.0,
            "totalDurationMs": 6100.0
        },
        "scores": { "streaming": "great", "gaming": "good", "rtc": "great" },
        "profile": profile
    })
}

#[tokio::test]
async fn a_completed_run_is_stored_and_read_back() {
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve(Some(history)).await;
    let c = client();

    let res = c
        .post(format!("{base}/api/results"))
        .json(&submission(9.4e8, "lan-1g"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 201);

    let created: serde_json::Value = res.json().await.unwrap();
    assert!(created["id"].as_i64().unwrap() > 0);
    assert!(created["recordedAt"].as_str().unwrap().ends_with('Z'));

    let runs: serde_json::Value = c
        .get(format!("{base}/api/history"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let runs = runs.as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["download"], 9.4e8);
    assert_eq!(runs[0]["profile"], "lan-1g");
    assert_eq!(runs[0]["scores"]["streaming"], "great");
    // Attribution comes from the connection, so a loopback client is 127.0.0.1.
    assert_eq!(runs[0]["clientIp"], "127.0.0.1");
    // And the user agent is captured.
    assert!(runs[0]["userAgent"].is_string());
}

#[tokio::test]
async fn runs_are_attributed_to_the_connecting_client_not_a_header() {
    // Trusting X-Forwarded-For would let any client file a run under any
    // address. There is no proxy in front of this service, so the connection is
    // the only trustworthy source.
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve(Some(history)).await;

    let res = client()
        .post(format!("{base}/api/results"))
        .header("x-forwarded-for", "203.0.113.9")
        .header("x-real-ip", "203.0.113.9")
        .json(&submission(1.0e9, "lan-1g"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 201);

    let runs: serde_json::Value = client()
        .get(format!("{base}/api/history"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(
        runs[0]["clientIp"], "127.0.0.1",
        "a spoofable header must not decide attribution"
    );
}

#[tokio::test]
async fn three_runs_are_all_listed_with_their_clients() {
    // The milestone's done-when. Both runs here come from loopback, so the
    // per-client split is exercised through the storage layer's own test; what
    // this proves is that every run is listed, newest first, over HTTP.
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve(Some(history.clone())).await;
    let c = client();

    for download in [1.0e9, 2.0e9, 3.0e9] {
        let res = c
            .post(format!("{base}/api/results"))
            .json(&submission(download, "lan-1g"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 201);
    }

    // A second client, injected directly so the address differs.
    history
        .record(
            &serde_json::from_value(submission(4.0e9, "lan-10g")).unwrap(),
            "10.0.0.77",
            "Firefox",
            "2026-08-24T23:59:00Z",
        )
        .unwrap();

    let runs: Vec<serde_json::Value> = c
        .get(format!("{base}/api/history"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(runs.len(), 4);

    let clients: Vec<serde_json::Value> = c
        .get(format!("{base}/api/clients"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(clients.len(), 2, "two distinct clients: {clients:?}");

    let counts: std::collections::BTreeMap<&str, i64> = clients
        .iter()
        .map(|c| (c["clientIp"].as_str().unwrap(), c["runs"].as_i64().unwrap()))
        .collect();
    assert_eq!(counts.get("127.0.0.1"), Some(&3));
    assert_eq!(counts.get("10.0.0.77"), Some(&1));

    // Filtering by a specific client returns only that client's runs.
    let mine: Vec<serde_json::Value> = c
        .get(format!("{base}/api/history?client=10.0.0.77"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0]["profile"], "lan-10g");

    // `mine` resolves to the requesting address without the browser needing to
    // know its own LAN address.
    let ours: Vec<serde_json::Value> = c
        .get(format!("{base}/api/history?client=mine"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ours.len(), 3);
}

#[tokio::test]
async fn a_result_with_no_measurements_is_rejected() {
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve(Some(history)).await;

    let res = client()
        .post(format!("{base}/api/results"))
        .json(&serde_json::json!({ "summary": {}, "scores": {}, "profile": "quick" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
    assert!(res.text().await.unwrap().contains("no measurements"));
}

#[tokio::test]
async fn an_oversized_payload_is_refused() {
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve(Some(history)).await;

    let res = client()
        .post(format!("{base}/api/results"))
        .header("content-type", "application/json")
        .body("x".repeat(128 * 1024))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 413);
}

#[tokio::test]
async fn malformed_json_is_a_client_error_not_a_crash() {
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve(Some(history)).await;

    let res = client()
        .post(format!("{base}/api/results"))
        .header("content-type", "application/json")
        .body("{not json at all")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
}

#[tokio::test]
async fn with_history_disabled_the_endpoints_degrade_quietly() {
    // The front end should not have to know whether this deployment keeps
    // results, so posting must not look like a failure.
    let base = serve(None).await;
    let c = client();

    let res = c
        .post(format!("{base}/api/results"))
        .json(&submission(1.0e9, "quick"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        202,
        "a disabled history should accept and ignore"
    );

    let runs: Vec<serde_json::Value> = c
        .get(format!("{base}/api/history"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(runs.is_empty());

    let status: serde_json::Value = c
        .get(format!("{base}/api/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["historyEnabled"], false);
}

#[tokio::test]
async fn status_reports_history_as_enabled_when_it_is() {
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve(Some(history)).await;

    let status: serde_json::Value = client()
        .get(format!("{base}/api/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["historyEnabled"], true);
}
