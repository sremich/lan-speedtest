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
    serve_with(CONFIG, history).await
}

/// The same, with a config of the caller's choosing.
async fn serve_with(config_toml: &str, history: Option<Arc<History>>) -> String {
    let config: Config = toml::from_str(config_toml).unwrap();
    let state = AppState::new(config, PayloadSource::new(4096), history);

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
    // The cap grew in 1.3.0, because a submission now carries every sample and
    // not just the summary. Deriving the body from the constant keeps this
    // test about the behaviour rather than about a particular number.
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve(Some(history)).await;

    let res = client()
        .post(format!("{base}/api/results"))
        .header("content-type", "application/json")
        .body("x".repeat(lan_speedtest::history::MAX_SUMMARY_BYTES + 1))
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

#[tokio::test]
async fn a_run_can_be_read_back_by_id_with_every_sample() {
    // Item 6 of the request: the history page could start a new test but not
    // return you to a result. A permalink needs the samples, not just the
    // headline, or the page it opens is not the page you left.
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve(Some(history)).await;
    let c = client();

    let mut body = submission(9.4e8, "lan-1g");
    body["points"] = serde_json::json!({
        "download": [{ "bps": 9.4e8, "bytes": 25_000_000, "ping": 0.7 }],
        "upload": [{ "bps": 5.0e8, "bytes": 10_000_000, "ping": 0.9 }]
    });

    let created: serde_json::Value = c
        .post(format!("{base}/api/results"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_i64().unwrap();

    let res = c
        .get(format!("{base}/api/results/{id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let run: serde_json::Value = res.json().await.unwrap();

    // The headline is flattened in alongside the detail, so one fetch renders
    // the whole page.
    assert_eq!(run["id"], id);
    assert_eq!(run["download"], 9.4e8);
    assert_eq!(run["profile"], "lan-1g");
    assert_eq!(run["summary"]["jitter"], 0.1);
    assert_eq!(run["points"]["download"][0]["bps"], 9.4e8);
    assert_eq!(run["points"]["upload"][0]["bytes"], 10_000_000);

    // A link to a run that never existed is a 404, not an empty result that
    // renders as a run of zeroes.
    let missing = c
        .get(format!("{base}/api/results/{}", id + 999))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
}

#[tokio::test]
async fn a_client_can_be_given_a_name_and_have_it_taken_away() {
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve(Some(history)).await;
    let c = client();

    c.post(format!("{base}/api/results"))
        .json(&submission(9.4e8, "lan-1g"))
        .send()
        .await
        .unwrap();

    let named = c
        .post(format!("{base}/api/clients/127.0.0.1/name"))
        .json(&serde_json::json!({ "name": "  Study desktop  " }))
        .send()
        .await
        .unwrap();
    assert_eq!(named.status(), 204);

    let runs: serde_json::Value = c
        .get(format!("{base}/api/history"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        runs[0]["clientName"], "Study desktop",
        "the name should be trimmed and attached to the run"
    );
    assert_eq!(
        runs[0]["clientIp"], "127.0.0.1",
        "and the address kept, because the name is a label rather than a replacement"
    );

    // Clearing it falls back to the address rather than leaving an empty label.
    let cleared = c
        .post(format!("{base}/api/clients/127.0.0.1/name"))
        .json(&serde_json::json!({ "name": "" }))
        .send()
        .await
        .unwrap();
    assert_eq!(cleared.status(), 204);

    let runs: serde_json::Value = c
        .get(format!("{base}/api/history"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(runs[0]["clientName"].is_null());
}

#[tokio::test]
async fn a_name_for_something_that_is_not_an_address_is_refused() {
    // The path segment keys a table. Accepting anything at all would let
    // history fill with rows nothing can ever match.
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve(Some(history)).await;

    let res = client()
        .post(format!("{base}/api/clients/not-an-address/name"))
        .json(&serde_json::json!({ "name": "nope" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
}

#[tokio::test]
async fn a_trusted_proxy_may_speak_for_the_client_behind_it() {
    // Item 5 of the request: behind a reverse proxy every run was attributed to
    // the proxy. The header is believed only when the connection comes from a
    // configured proxy — which is why the untrusted case above still records
    // the peer.
    const PROXIED: &str = "
profile = 'test'
[server]
static_dir = 'does-not-exist'
trusted_proxies = ['127.0.0.0/8']
[profiles.test]
measurements = [ { type = 'download', bytes = 1000, count = 1 } ]
";
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve_with(PROXIED, Some(history)).await;

    let res = client()
        .post(format!("{base}/api/results"))
        // Two hops: the rightmost is the trusted proxy itself, so the address
        // to believe is the one before it.
        .header("x-forwarded-for", "203.0.113.9, 127.0.0.1")
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
    assert_eq!(runs[0]["clientIp"], "203.0.113.9");
}

#[tokio::test]
async fn a_proxy_chain_split_across_repeated_headers_is_read_whole() {
    // `x-forwarded-for` is a repeatable header, and proxies differ: some extend
    // one comma-joined list, others append a line per hop. Reading only the
    // first line drops every hop after it — and since the address to believe is
    // found by walking the chain from the right, dropping the tail means
    // believing the wrong end of it. Same chain as the test above, delivered as
    // two header lines instead of one, and it must reach the same conclusion.
    const PROXIED: &str = "
profile = 'test'
[server]
static_dir = 'does-not-exist'
trusted_proxies = ['127.0.0.0/8']
[profiles.test]
measurements = [ { type = 'download', bytes = 1000, count = 1 } ]
";
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve_with(PROXIED, Some(history)).await;

    let res = client()
        .post(format!("{base}/api/results"))
        .header("x-forwarded-for", "203.0.113.9")
        .header("x-forwarded-for", "127.0.0.1")
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
    assert_eq!(runs[0]["clientIp"], "203.0.113.9");
}

#[tokio::test]
async fn a_run_records_the_build_that_measured_it() {
    // A stored figure is only interpretable if you know what produced it:
    // everything recorded before 1.3.1 has its latency inflated by up to 40 ms
    // by the Nagle bug, and without this there is no way to tell those rows
    // from correct ones.
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve_with(CONFIG, Some(history)).await;

    let res = client()
        .post(format!("{base}/api/results"))
        .json(&submission(1.0e9, "lan-1g"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 201);
    let id = res.json::<serde_json::Value>().await.unwrap()["id"]
        .as_i64()
        .unwrap();

    let listed: serde_json::Value = client()
        .get(format!("{base}/api/history"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed[0]["appVersion"], lan_speedtest::VERSION);

    // And on the permalink, which reads through a different query.
    let one: serde_json::Value = client()
        .get(format!("{base}/api/results/{id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(one["appVersion"], lan_speedtest::VERSION);
}

#[tokio::test]
async fn status_says_what_kind_of_address_the_client_has() {
    // "Why does it always say 10.42.7.3?" is answerable only if the page can
    // say what sort of address it is looking at.
    let base = serve(None).await;
    let status: serde_json::Value = client()
        .get(format!("{base}/api/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(status["clientIp"], "127.0.0.1");
    assert_eq!(status["clientKind"], "loopback");
    assert!(status["clientKindLabel"].as_str().unwrap().len() > 3);
}

/// A one-shot DNS server that answers whatever it is asked with `answer`.
///
/// Real enough to exercise the whole path — the query goes out over UDP and
/// the reply is parsed by the same code a real resolver's would be — without
/// depending on the machine running the tests having a reverse zone.
async fn fake_resolver(answer: &'static str) -> SocketAddr {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();

    tokio::spawn(async move {
        let mut buf = [0u8; 1232];
        let Ok((read, from)) = socket.recv_from(&mut buf).await else {
            return;
        };
        if read < 13 {
            return;
        }

        let mut reply = Vec::new();
        reply.extend_from_slice(&buf[0..2]); // the query's id, which is checked
        reply.extend_from_slice(&0x8180u16.to_be_bytes()); // response, no error
        reply.extend_from_slice(&1u16.to_be_bytes()); // one question
        reply.extend_from_slice(&1u16.to_be_bytes()); // one answer
        reply.extend_from_slice(&[0, 0, 0, 0]);
        reply.extend_from_slice(&buf[12..read]); // the question, echoed back

        reply.extend_from_slice(&0xC00Cu16.to_be_bytes()); // pointer to the question name
        reply.extend_from_slice(&12u16.to_be_bytes()); // PTR
        reply.extend_from_slice(&1u16.to_be_bytes()); // IN
        reply.extend_from_slice(&300u32.to_be_bytes()); // ttl

        let mut rdata = Vec::new();
        for label in answer.split('.') {
            rdata.push(label.len() as u8);
            rdata.extend_from_slice(label.as_bytes());
        }
        rdata.push(0);
        reply.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
        reply.extend_from_slice(&rdata);

        let _ = socket.send_to(&reply, from).await;
    });

    addr
}

#[tokio::test]
async fn a_client_is_named_from_its_ptr_record() {
    // Item 4 of the request. The lookup is fired after the run is stored and
    // never on the request path, so this waits for the name to appear rather
    // than expecting it in the POST's own response.
    let resolver = fake_resolver("study-desktop.lan").await;
    let config = format!(
        "
profile = 'test'
[server]
static_dir = 'does-not-exist'
[server.reverse_dns]
enabled = true
resolver = '{resolver}'
# Loopback, because that is where the test client connects from. The shipped
# default deliberately does not include it.
ranges = ['127.0.0.0/8']
[profiles.test]
measurements = [ {{ type = 'download', bytes = 1000, count = 1 }} ]
"
    );

    let history = Arc::new(History::in_memory().unwrap());
    let base = serve_with(&config, Some(history)).await;
    let c = client();

    let res = c
        .post(format!("{base}/api/results"))
        .json(&submission(9.4e8, "lan-1g"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 201);

    let mut hostname = serde_json::Value::Null;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let runs: serde_json::Value = c
            .get(format!("{base}/api/history"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        hostname = runs[0]["hostname"].clone();
        if !hostname.is_null() {
            break;
        }
    }
    assert_eq!(hostname, "study-desktop.lan");
}

#[tokio::test]
async fn an_address_outside_the_configured_ranges_is_never_looked_up() {
    // The range restriction is the whole safety property: without it, a
    // deployment reachable from the internet would send a PTR query about
    // every visitor to its upstream resolver. The resolver here would answer
    // if asked, so a name appearing at all would be the failure.
    let resolver = fake_resolver("should-never-be-asked.example").await;
    let config = format!(
        "
profile = 'test'
[server]
static_dir = 'does-not-exist'
[server.reverse_dns]
enabled = true
resolver = '{resolver}'
ranges = ['192.168.0.0/16']
[profiles.test]
measurements = [ {{ type = 'download', bytes = 1000, count = 1 }} ]
"
    );

    let history = Arc::new(History::in_memory().unwrap());
    let base = serve_with(&config, Some(history)).await;
    let c = client();

    c.post(format!("{base}/api/results"))
        .json(&submission(9.4e8, "lan-1g"))
        .send()
        .await
        .unwrap();

    // Long enough that a lookup, if one were made, would have answered.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let runs: serde_json::Value = c
        .get(format!("{base}/api/history"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        runs[0]["hostname"].is_null(),
        "127.0.0.1 is outside the configured ranges and must not be looked up"
    );
}

#[tokio::test]
async fn a_run_can_be_annotated_after_the_fact() {
    // The note is per-run, not per-client: "upstairs landing, laptop on
    // battery, checking the new AP" is exactly what differs between two runs
    // from the same machine.
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve(Some(history)).await;
    let c = client();

    let created: serde_json::Value = c
        .post(format!("{base}/api/results"))
        .json(&submission(9.4e8, "lan-1g"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_i64().unwrap();

    let res = c
        .post(format!("{base}/api/results/{id}/note"))
        .json(&serde_json::json!({ "note": "  upstairs landing, laptop on battery  " }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);

    // Visible both in the list and on the run itself, so the history table can
    // show it and the permalink can too.
    let runs: serde_json::Value = c
        .get(format!("{base}/api/history"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(runs[0]["note"], "upstairs landing, laptop on battery");

    let one: serde_json::Value = c
        .get(format!("{base}/api/results/{id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(one["note"], "upstairs landing, laptop on battery");

    // Clearing it returns to no note rather than an empty string, so the front
    // end has one thing to test for.
    c.post(format!("{base}/api/results/{id}/note"))
        .json(&serde_json::json!({ "note": "" }))
        .send()
        .await
        .unwrap();
    let runs: serde_json::Value = c
        .get(format!("{base}/api/history"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(runs[0]["note"].is_null());
}

#[tokio::test]
async fn a_note_is_capped_in_characters_and_refused_for_a_missing_run() {
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve(Some(history)).await;
    let c = client();

    let created: serde_json::Value = c
        .post(format!("{base}/api/results"))
        .json(&submission(9.4e8, "lan-1g"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_i64().unwrap();

    // Multi-byte on purpose: a byte cap would both cut this far shorter than
    // intended and risk splitting a character in half.
    let long = "é".repeat(400);
    c.post(format!("{base}/api/results/{id}/note"))
        .json(&serde_json::json!({ "note": long }))
        .send()
        .await
        .unwrap();

    let runs: serde_json::Value = c
        .get(format!("{base}/api/history"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let stored = runs[0]["note"].as_str().unwrap();
    assert_eq!(stored.chars().count(), 280);
    assert!(stored.chars().all(|ch| ch == 'é'), "a character was split");

    // A note about a run that does not exist is a 404, not a silent success.
    let missing = c
        .post(format!("{base}/api/results/{}/note", id + 999))
        .json(&serde_json::json!({ "note": "nowhere" }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
}

#[tokio::test]
async fn metrics_is_refused_unless_it_is_turned_on_even_when_static_files_exist() {
    // The body names every client that has run a test — more than an
    // unauthenticated endpoint should hand out by default.
    //
    // The static directory is real here, and that is the entire point of this
    // test. `/metrics` used to be left unrouted when disabled, which looks
    // equivalent to refusing and is not: the SPA fallback answers anything
    // unrouted, so a deployment that actually serves the front end returned
    // 200 and an HTML page. The first version of this test pointed at a
    // directory that did not exist, so the fallback failed and produced the
    // 404 it was hoping for — passing while testing nothing.
    let dir = std::env::temp_dir().join(format!("speedtest-static-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "<!doctype html><title>app</title>").unwrap();

    let config = format!(
        "
profile = 'test'
[server]
static_dir = '{}'
[profiles.test]
measurements = [ {{ type = 'download', bytes = 1000, count = 1 }} ]
",
        dir.display()
    );

    let history = Arc::new(History::in_memory().unwrap());
    let base = serve_with(&config, Some(history)).await;

    // The fallback really is serving, so a 404 below is a decision and not an
    // accident of a missing directory.
    let shell = client()
        .get(format!("{base}/no-such-page"))
        .send()
        .await
        .unwrap();
    assert_eq!(shell.status(), 200, "the SPA fallback should be answering");

    let res = client()
        .get(format!("{base}/metrics"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        404,
        "metrics must be refused, not answered by the static fallback"
    );
    let body = res.text().await.unwrap();
    assert!(
        !body.contains("<!doctype html"),
        "a scrape must not receive the app shell: {body}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn metrics_reports_the_latest_run_per_client_in_base_units() {
    const WITH_METRICS: &str = "
profile = 'test'
[server]
static_dir = 'does-not-exist'
metrics = true
[profiles.test]
measurements = [ { type = 'download', bytes = 1000, count = 1 } ]
";
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve_with(WITH_METRICS, Some(history)).await;

    // Two runs from the same client: the scrape is a question about now, so
    // only the newer one should appear.
    for bps in [5.0e8, 9.4e8] {
        let res = client()
            .post(format!("{base}/api/results"))
            .json(&submission(bps, "lan-1g"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 201);
    }

    let res = client()
        .get(format!("{base}/metrics"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert!(res
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/plain"));

    let body = res.text().await.unwrap();
    assert!(body.contains("speedtest_build_info{"), "{body}");
    assert!(body.contains("speedtest_history_runs_total 2"), "{body}");

    let series: Vec<&str> = body
        .lines()
        .filter(|l| l.starts_with("speedtest_download_bits_per_second{"))
        .collect();
    assert_eq!(series.len(), 1, "one series per client, got: {series:?}");
    assert!(series[0].ends_with(" 940000000"), "{series:?}");
}

// --- same-origin guard ------------------------------------------------------
//
// The mutating endpoints refuse a request whose `Origin` names somewhere else.
// It is not authentication — the tool has none and is not meant to — but a page
// on another origin should not be able to file runs or rename clients in a
// visitor's browser without anyone noticing.

#[tokio::test]
async fn a_cross_origin_post_is_refused_and_writes_nothing() {
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve(Some(history)).await;
    let c = client();

    // One genuine run first, so "nothing was written" is a statement about the
    // refused requests rather than about an empty database.
    let res = c
        .post(format!("{base}/api/results"))
        .json(&submission(9.4e8, "lan-1g"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 201);
    let id = res.json::<serde_json::Value>().await.unwrap()["id"]
        .as_i64()
        .unwrap();

    c.post(format!("{base}/api/clients/127.0.0.1/name"))
        .json(&serde_json::json!({ "name": "the workshop pi" }))
        .send()
        .await
        .unwrap();

    // Now the same three requests as a page on evil.example would make them.
    // `Origin: null` is in here too: a sandboxed iframe sends that, and it is
    // the browser declining to say where the request came from.
    for origin in ["http://evil.example", "null"] {
        let refused = c
            .post(format!("{base}/api/results"))
            .header("origin", origin)
            .json(&submission(1.0, "forged"))
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), 403, "origin {origin} should be refused");

        let refused = c
            .post(format!("{base}/api/results/{id}/note"))
            .header("origin", origin)
            .json(&serde_json::json!({ "note": "forged" }))
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), 403, "origin {origin} should be refused");

        let refused = c
            .post(format!("{base}/api/clients/127.0.0.1/name"))
            .header("origin", origin)
            .json(&serde_json::json!({ "name": "forged" }))
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), 403, "origin {origin} should be refused");
    }

    // The status code is the visible half; this is the half that matters.
    let runs: serde_json::Value = c
        .get(format!("{base}/api/history"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let runs = runs.as_array().unwrap();
    assert_eq!(runs.len(), 1, "a refused run must not be stored: {runs:?}");
    assert_eq!(runs[0]["download"], 9.4e8);
    assert!(
        runs[0]["note"].as_str().unwrap_or("").is_empty(),
        "a refused note must not be written: {:?}",
        runs[0]["note"]
    );
    assert_eq!(
        runs[0]["clientName"], "the workshop pi",
        "a refused rename must not overwrite the real name"
    );
}

#[tokio::test]
async fn a_same_origin_post_is_accepted() {
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve(Some(history)).await;
    let c = client();

    // The origin is the base URL the request is being sent to, and the `Host`
    // header it is compared against is generated by the client from that same
    // URL — so this asserts the round trip rather than a hand-written pair.
    let res = c
        .post(format!("{base}/api/results"))
        .header("origin", &base)
        .json(&submission(9.4e8, "lan-1g"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 201);
    let id = res.json::<serde_json::Value>().await.unwrap()["id"]
        .as_i64()
        .unwrap();

    let res = c
        .post(format!("{base}/api/results/{id}/note"))
        .header("origin", &base)
        .json(&serde_json::json!({ "note": "from the page itself" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);

    let res = c
        .post(format!("{base}/api/clients/127.0.0.1/name"))
        .header("origin", &base)
        .json(&serde_json::json!({ "name": "the workshop pi" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);

    let runs: serde_json::Value = c
        .get(format!("{base}/api/history"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(runs[0]["note"], "from the page itself");
    assert_eq!(runs[0]["clientName"], "the workshop pi");
}

#[tokio::test]
async fn a_request_with_no_origin_at_all_is_left_alone() {
    // curl, a script, the rest of this suite. Nothing that is not a browser
    // sends `Origin`, and refusing on its absence would break every one of them
    // while adding nothing — the case being guarded against always has it.
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
    let id = res.json::<serde_json::Value>().await.unwrap()["id"]
        .as_i64()
        .unwrap();

    let res = c
        .post(format!("{base}/api/results/{id}/note"))
        .json(&serde_json::json!({ "note": "curled in" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);

    let res = c
        .post(format!("{base}/api/clients/127.0.0.1/name"))
        .json(&serde_json::json!({ "name": "unnamed no longer" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);
}

#[tokio::test]
async fn reading_the_history_is_not_affected_by_the_guard() {
    // The guard is on the endpoints that change something. A cross-origin read
    // is already governed by the browser's own CORS rules, and refusing one
    // here would buy nothing while breaking a permalink opened from elsewhere.
    let history = Arc::new(History::in_memory().unwrap());
    let base = serve(Some(history)).await;
    let c = client();

    let id = c
        .post(format!("{base}/api/results"))
        .json(&submission(9.4e8, "lan-1g"))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["id"]
        .as_i64()
        .unwrap();

    for path in ["api/history", "api/clients", "api/status"] {
        let res = c
            .get(format!("{base}/{path}"))
            .header("origin", "http://evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 200, "{path} should still be readable");
    }

    let res = c
        .get(format!("{base}/api/results/{id}"))
        .header("origin", "http://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
}
