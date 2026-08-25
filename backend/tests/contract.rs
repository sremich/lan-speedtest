//! Tier 1 — the engine contract, driven over a real socket.
//!
//! These tests stand in for the browser: they issue the exact requests
//! `@cloudflare/speedtest` 1.13.1 issues and assert on what it reads back.
//! If one of these fails, the engine will produce wrong numbers rather than
//! an error, which is the failure mode worth guarding hardest against.

use std::time::{Duration, Instant};

use lan_speedtest::{router, AppState, Config, PayloadSource};

const CONFIG: &str = "
profile = 'test'
[server]
max_transfer_bytes = 50000000
download_chunk_bytes = 65536
static_dir = 'does-not-exist'
[profiles.test]
description = 'test profile'
measurements = [
  { type = 'latency', numPackets = 5 },
  { type = 'download', bytes = 1000000, count = 2 },
  { type = 'packetLoss', numPackets = 100, batchSize = 10, batchWaitTime = 10, responsesWaitTime = 500 },
]
";

/// Boots the real router on an ephemeral port and returns its base URL.
async fn serve(config_toml: &str) -> String {
    let config: Config = toml::from_str(config_toml).expect("test config parses");
    let payload = PayloadSource::new(config.server.download_chunk_bytes);
    let state = AppState::new(config, payload, None);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router(state)).await.unwrap();
    });
    format!("http://{addr}")
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap()
}

// --- download ---------------------------------------------------------------

#[tokio::test]
async fn down_returns_exactly_the_requested_number_of_bytes() {
    let base = serve(CONFIG).await;
    let c = client();

    // Sizes either side of the 64 KiB internal buffer, so partial final
    // frames are covered too.
    for n in [1u64, 65_535, 65_536, 65_537, 1_000_000, 5_000_000] {
        let body = c
            .get(format!("{base}/__down?bytes={n}"))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(body.len() as u64, n, "bytes={n}");
    }
}

#[tokio::test]
async fn down_advertises_content_length_rather_than_chunking() {
    // A known length lets hyper skip chunked framing, and lets the browser
    // account transferSize accurately — which is what download speed is
    // computed from.
    let base = serve(CONFIG).await;
    let r = client()
        .get(format!("{base}/__down?bytes=1000000"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.content_length(), Some(1_000_000));
}

#[tokio::test]
async fn down_with_bytes_zero_is_the_latency_ping() {
    // The engine has no separate ping endpoint: a latency stage is
    // `GET /__down?bytes=0`. It must succeed and return an empty body.
    let base = serve(CONFIG).await;
    let r = client()
        .get(format!("{base}/__down?bytes=0"))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    assert!(r.headers().get("server-timing").is_some());
    assert_eq!(r.bytes().await.unwrap().len(), 0);
}

#[tokio::test]
async fn down_carries_a_server_timing_header_the_engine_can_parse() {
    let base = serve(CONFIG).await;
    let r = client()
        .get(format!("{base}/__down?bytes=1000"))
        .send()
        .await
        .unwrap();

    let st = r
        .headers()
        .get("server-timing")
        .expect("server-timing present")
        .to_str()
        .unwrap()
        .to_string();

    // The engine matches /cfReq(?:uest)?Dur(?:ation)?;\s*dur=([0-9.]+)/i and
    // nothing else. A bare `dur=` would be silently dropped.
    assert!(
        st.starts_with("cfRequestDuration;dur="),
        "server-timing was {st:?}"
    );
    let dur: f64 = st
        .trim_start_matches("cfRequestDuration;dur=")
        .parse()
        .unwrap();
    assert!((0.0..1000.0).contains(&dur), "implausible duration {dur}");
}

#[tokio::test]
async fn measurement_responses_forbid_caching() {
    // Without this the browser answers the 2nd..Nth identical request from
    // cache and the engine measures the cache instead of the network.
    let base = serve(CONFIG).await;
    let r = client()
        .get(format!("{base}/__down?bytes=1000"))
        .send()
        .await
        .unwrap();
    let cc = r.headers().get("cache-control").unwrap().to_str().unwrap();
    assert!(cc.contains("no-store"), "cache-control was {cc}");
}

#[tokio::test]
async fn down_ignores_query_parameters_it_does_not_know() {
    // The engine appends `during=download` while measuring loaded latency.
    let base = serve(CONFIG).await;
    let r = client()
        .get(format!(
            "{base}/__down?bytes=1024&during=download&measId=abc"
        ))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    assert_eq!(r.bytes().await.unwrap().len(), 1024);
}

#[tokio::test]
async fn down_rejects_a_malformed_or_oversized_request() {
    let base = serve(CONFIG).await;
    let c = client();

    let bad = c
        .get(format!("{base}/__down?bytes=not-a-number"))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);

    let huge = c
        .get(format!("{base}/__down?bytes=99999999999"))
        .send()
        .await
        .unwrap();
    assert_eq!(huge.status(), 413);
}

// --- upload -----------------------------------------------------------------

#[tokio::test]
async fn up_accepts_the_engines_payload_and_reports_what_it_read() {
    let base = serve(CONFIG).await;
    // The engine sends `'0'.repeat(n)` as a plain string body.
    let payload = "0".repeat(1_000_000);
    let r = client()
        .post(format!("{base}/__up?bytes=1000000"))
        .body(payload)
        .send()
        .await
        .unwrap();

    assert!(r.status().is_success());
    assert_eq!(
        r.headers()
            .get("x-received-bytes")
            .unwrap()
            .to_str()
            .unwrap(),
        "1000000"
    );
    assert!(r.headers().get("server-timing").is_some());
}

#[tokio::test]
async fn up_does_not_answer_before_the_whole_body_has_arrived() {
    // Upload speed is derived from time-to-first-byte alone
    // (`calcUploadDuration = ({ttfb}) => ttfb`). Answering early would make
    // every upload look instantaneous. Feed the body slowly and assert the
    // response is withheld until the last chunk is in.
    use futures_util::stream;

    let base = serve(CONFIG).await;
    let chunk_delay = Duration::from_millis(60);
    let chunks = 5;

    let body_stream = stream::unfold(0usize, move |i| async move {
        if i >= chunks {
            return None;
        }
        tokio::time::sleep(chunk_delay).await;
        let chunk: Result<bytes::Bytes, std::io::Error> =
            Ok(bytes::Bytes::from(vec![b'0'; 10_000]));
        Some((chunk, i + 1))
    });

    let started = Instant::now();
    let r = client()
        .post(format!("{base}/__up?bytes=50000"))
        .body(reqwest::Body::wrap_stream(body_stream))
        .send()
        .await
        .unwrap();
    let elapsed = started.elapsed();

    assert!(r.status().is_success());
    assert_eq!(
        r.headers()
            .get("x-received-bytes")
            .unwrap()
            .to_str()
            .unwrap(),
        "50000"
    );
    // Response headers must not have arrived before the final chunk was sent.
    let minimum = chunk_delay * chunks as u32;
    assert!(
        elapsed >= minimum,
        "responded after {elapsed:?}, before the body finished at {minimum:?} \
         — upload timing would be wrong"
    );
}

#[tokio::test]
async fn up_refuses_a_body_larger_than_the_cap() {
    let base = serve(CONFIG).await;
    let r = client()
        .post(format!("{base}/__up?bytes=99999999999"))
        .body("0".repeat(1024))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 413);
}

// --- api --------------------------------------------------------------------

#[tokio::test]
async fn status_exposes_version_and_git_sha() {
    let base = serve(CONFIG).await;
    let v: serde_json::Value = client()
        .get(format!("{base}/api/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(v["version"].is_string());
    assert!(v["gitSha"].is_string());
    assert_eq!(v["profile"], "test");
    assert_eq!(
        v["siteName"], "LAN Speed Test",
        "an unnamed deployment still needs something to put in the heading"
    );
}

#[tokio::test]
async fn profiles_are_listed_for_the_picker_and_selectable_by_name() {
    let base = serve(
        "
profile = 'test'
[server]
static_dir = 'does-not-exist'
[profiles.test]
description = 'the default'
nominal_bps = 1000000000.0
auto_selectable = true
measurements = [{ type = 'download', bytes = 1000, count = 2 }]
[profiles.other]
description = 'the other one'
measurements = [{ type = 'latency', numPackets = 7 }]
",
    )
    .await;

    let list: serde_json::Value = client()
        .get(format!("{base}/api/profiles"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["default"], "test");
    let names: Vec<&str> = list["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["other", "test"]);
    let test_entry = &list["profiles"][1];
    assert_eq!(test_entry["autoSelectable"], true);
    assert_eq!(test_entry["nominalBps"], 1_000_000_000.0);
    // Not marked auto-selectable, so automatic selection must not offer it.
    assert_eq!(list["profiles"][0]["autoSelectable"], false);

    // No name asked for: the server's default.
    let default: serde_json::Value = client()
        .get(format!("{base}/api/profile"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(default["profile"], "test");

    // Asking by name hands out that profile, and reports it as the one in
    // use — history records this value, so it has to be the truth.
    let other: serde_json::Value = client()
        .get(format!("{base}/api/profile?name=other"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(other["profile"], "other");
    assert_eq!(other["engineConfig"]["measurements"][0]["numPackets"], 7);
}

#[tokio::test]
async fn an_unknown_profile_name_is_refused_rather_than_silently_defaulted() {
    // Quietly serving the default would let a stale bookmark measure something
    // other than what it says it is measuring.
    let base = serve(CONFIG).await;
    let res = client()
        .get(format!("{base}/api/profile?name=../../etc/passwd"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
}

#[tokio::test]
async fn status_carries_the_configured_site_name() {
    // The heading is decided server-side so that renaming an installation is
    // a config change and a restart, not a rebuild.
    let base = serve(&CONFIG.replace(
        "[server]",
        "[server]
site_name = 'Rack Room'",
    ))
    .await;
    let v: serde_json::Value = client()
        .get(format!("{base}/api/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["siteName"], "Rack Room");
}

#[tokio::test]
async fn profile_pins_the_settings_that_keep_traffic_on_the_lan() {
    // The single most important assertion in the suite: the engine reports
    // completed results to Cloudflare unless logAimApiUrl is null.
    let base = serve(CONFIG).await;
    let v: serde_json::Value = client()
        .get(format!("{base}/api/profile"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let engine = &v["engineConfig"];
    assert!(
        engine["logAimApiUrl"].is_null(),
        "results would be reported externally"
    );
    assert!(engine["logMeasurementApiUrl"].is_null());
    assert_eq!(engine["downloadApiUrl"], "/__down");
    assert_eq!(engine["uploadApiUrl"], "/__up");

    // No absolute URL anywhere in the payload — every endpoint stays relative
    // to this origin.
    let raw = v.to_string();
    assert!(!raw.contains("http://"), "absolute URL in profile: {raw}");
    assert!(!raw.contains("https://"), "absolute URL in profile: {raw}");
    assert!(
        !raw.contains("cloudflare.com"),
        "cloudflare endpoint in profile: {raw}"
    );
}

#[tokio::test]
async fn packet_loss_stage_is_dropped_when_no_relay_is_configured() {
    // turn.enabled defaults to false here; without a relay the stage cannot
    // complete, so it must not reach the browser.
    let base = serve(CONFIG).await;
    let v: serde_json::Value = client()
        .get(format!("{base}/api/profile"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["packetLossEnabled"], false);
    let kinds: Vec<&str> = v["engineConfig"]["measurements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["type"].as_str().unwrap())
        .collect();
    assert!(!kinds.contains(&"packetLoss"), "stages were {kinds:?}");
    assert!(kinds.contains(&"latency"));
    assert!(kinds.contains(&"download"));
}

#[tokio::test]
async fn packet_loss_stage_and_credentials_appear_once_a_relay_is_configured() {
    let with_turn = CONFIG.replace(
        "[profiles.test]",
        "[turn]\nenabled = true\nuri = 'relay.test:3478'\nuser = 'speedtest'\npass = 'secret'\n\n[profiles.test]",
    );
    let base = serve(&with_turn).await;
    let v: serde_json::Value = client()
        .get(format!("{base}/api/profile"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["packetLossEnabled"], true);
    let engine = &v["engineConfig"];
    assert_eq!(engine["turnServerUri"], "relay.test:3478");
    // Both user and pass must be present: with either missing, the engine
    // falls back to fetching credentials from turnServerCredsApiUrl, which
    // defaults to a Cloudflare endpoint.
    assert_eq!(engine["turnServerUser"], "speedtest");
    assert_eq!(engine["turnServerPass"], "secret");

    let kinds: Vec<&str> = engine["measurements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["type"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"packetLoss"), "stages were {kinds:?}");
}

#[tokio::test]
async fn profile_overrides_the_engine_defaults_that_break_lan_measurement() {
    // Regression guard. The engine's `loadedRequestMinDuration` default of
    // 250 ms discards every transfer size whose requests finish faster than
    // that. On a LAN nothing is that slow — a 250 MB download at 10 Gbps takes
    // ~200 ms — so loaded latency stays 0, and because 0 is falsy the engine's
    // `loadedLatencyIncrease` comes back undefined and it emits NO AIM scores
    // at all. The whole quality-rating feature disappears silently.
    //
    // These two values must therefore always be sent, and always be well under
    // the fastest transfer the profile expects.
    let base = serve(CONFIG).await;
    let v: serde_json::Value = client()
        .get(format!("{base}/api/profile"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let engine = &v["engineConfig"];
    let min_duration = engine["loadedRequestMinDuration"]
        .as_f64()
        .expect("loadedRequestMinDuration must be sent to the engine");
    let throttle = engine["loadedLatencyThrottle"]
        .as_f64()
        .expect("loadedLatencyThrottle must be sent to the engine");

    assert!(
        min_duration < 250.0,
        "loadedRequestMinDuration is {min_duration}ms — at or above the engine          default, which would discard every LAN-speed transfer"
    );
    assert!(
        throttle < 400.0,
        "loadedLatencyThrottle is {throttle}ms — at or above the engine default,          which yields too few samples inside a short LAN transfer"
    );
}
