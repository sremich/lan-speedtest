//! HTTP surface.
//!
//! `/__down` and `/__up` implement the request contract of
//! `@cloudflare/speedtest` (verified against the 1.13.1 sources — see
//! `docs/wiki/Engine-Contract.md`). Two details in there are easy to get
//! wrong and both are load-bearing:
//!
//! 1. The engine's `server-timing` parser only accepts the metric names
//!    `cfRequestDuration` (and the `cfReqDur` / `cfRequestDur` abbreviations)
//!    or a sum of `cfSpeed*` entries. A plain `server-timing: dur=1.2` is
//!    silently ignored.
//! 2. Upload speed is derived from time-to-first-byte alone. If we answer
//!    before the request body is fully drained, upload figures become
//!    fiction. `up()` therefore reads the body to completion first.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::extract::{ConnectInfo, FromRequestParts, Query, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use tower_http::services::{ServeDir, ServeFile};

use crate::config::{Config, Measurement};
use crate::history::{self, History, HistoryError, ResultSubmission};
use crate::payload::PayloadSource;

/// The engine's parser requires this exact metric name (case-insensitive).
/// Values below 0.01 ms are treated as absent by the engine, which is fine:
/// it then falls back to the profile's `estimatedServerTime`.
const SERVER_TIMING_METRIC: &str = "cfRequestDuration";

pub const VERSION: &str = env!("APP_VERSION");
pub const GIT_SHA: &str = env!("APP_GIT_SHA");

#[derive(Clone)]
pub struct AppState {
    pub payload: PayloadSource,
    pub config: Arc<Config>,
    /// Absent when history is disabled, which is how the contract tests and the
    /// e2e suite run.
    pub history: Option<Arc<History>>,
}

/// Query string of a measurement request.
///
/// Unknown parameters are ignored on purpose: the engine appends its own
/// (`during=download` while measuring loaded latency, for instance) and a
/// stricter parser would reject perfectly valid traffic.
#[derive(Debug, Deserialize)]
pub struct TransferQuery {
    #[serde(default)]
    pub bytes: Option<String>,
}

/// A `bytes` parameter that could not be read as a count.
///
/// Deliberately small: carrying a whole `Response` as the error variant makes
/// every `Result` in this module 128 bytes wide, which clippy rightly objects
/// to on a hot path.
#[derive(Debug)]
pub struct InvalidBytes(String);

impl IntoResponse for InvalidBytes {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid 'bytes' parameter: {:?}", self.0),
        )
            .into_response()
    }
}

impl TransferQuery {
    /// The engine always sends `bytes`; absence means 0 (a bare latency ping).
    fn parse(&self) -> Result<u64, InvalidBytes> {
        match self.bytes.as_deref() {
            None | Some("") => Ok(0),
            Some(raw) => raw
                .trim()
                .parse::<u64>()
                .map_err(|_| InvalidBytes(raw.to_string())),
        }
    }
}

pub fn router(state: AppState) -> Router {
    let static_dir = state.config.server.static_dir.clone();
    let index = format!("{static_dir}/index.html");

    // SPA-style fallback so a deep link still resolves to the app shell.
    let statics = ServeDir::new(&static_dir).fallback(ServeFile::new(&index));

    Router::new()
        .route("/__down", get(down))
        .route("/__up", post(up))
        .route("/api/status", get(status))
        .route("/api/profile", get(profile))
        .route("/api/profiles", get(profiles))
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/results", post(record_result))
        .route("/api/history", get(list_history))
        .route("/api/clients", get(list_clients))
        .fallback_service(statics)
        .with_state(state)
}

/// The client's address, or `None` when the server was started without
/// connect info (which is how the backend-only tests run).
///
/// Taken from the connection, never from a header. `X-Forwarded-For` is
/// trivially spoofable and there is no proxy in front of this service anyway,
/// so trusting it would let any client attribute a run to any address it liked.
///
/// Infallible on purpose: a missing peer address should degrade to "unknown"
/// rather than reject an otherwise valid request.
#[derive(Debug, Clone, Copy)]
pub struct ClientAddr(pub Option<SocketAddr>);

impl ClientAddr {
    pub fn ip(&self) -> String {
        self.0
            .map(|a| a.ip().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    }
}

impl<S: Send + Sync> FromRequestParts<S> for ClientAddr {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(ClientAddr(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ConnectInfo(addr)| *addr),
        ))
    }
}

fn user_agent(headers: &HeaderMap) -> String {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(300)
        .collect()
}

impl IntoResponse for HistoryError {
    fn into_response(self) -> Response {
        let status = match self {
            HistoryError::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
            HistoryError::Json(_) | HistoryError::NothingMeasured => StatusCode::BAD_REQUEST,
            HistoryError::TooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
        };
        if status.is_server_error() {
            tracing::error!("history: {self}");
        }
        (status, self.to_string()).into_response()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Recorded {
    id: i64,
    recorded_at: String,
}

/// `POST /api/results` — store a completed run.
async fn record_result(
    State(state): State<AppState>,
    client: ClientAddr,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some(history) = state.history.as_ref() else {
        // Not an error: history is optional, and the front end should not have
        // to care whether this deployment keeps results.
        return (StatusCode::ACCEPTED, "history is disabled").into_response();
    };

    if body.len() > history::MAX_SUMMARY_BYTES {
        return HistoryError::TooLarge {
            bytes: body.len(),
            limit: history::MAX_SUMMARY_BYTES,
        }
        .into_response();
    }

    let submission: ResultSubmission = match serde_json::from_str(&body) {
        Ok(s) => s,
        Err(e) => return HistoryError::Json(e).into_response(),
    };

    let recorded_at = history::now_rfc3339();
    match history.record(
        &submission,
        &client.ip(),
        &user_agent(&headers),
        &recorded_at,
    ) {
        Ok(id) => (
            StatusCode::CREATED,
            axum::Json(Recorded { id, recorded_at }),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct HistoryQuery {
    #[serde(default)]
    limit: Option<u32>,
    /// Restrict to one client. `mine` means the requesting address, which is
    /// how the page offers "just this machine" without the browser needing to
    /// know its own LAN address.
    #[serde(default)]
    client: Option<String>,
}

/// `GET /api/history` — stored runs, newest first.
async fn list_history(
    State(state): State<AppState>,
    client: ClientAddr,
    Query(q): Query<HistoryQuery>,
) -> Response {
    let Some(history) = state.history.as_ref() else {
        return axum::Json(Vec::<()>::new()).into_response();
    };

    let me = client.ip();
    let filter = match q.client.as_deref() {
        None | Some("") | Some("all") => None,
        Some("mine") => Some(me.as_str()),
        Some(other) => Some(other),
    };

    match history.recent(q.limit.unwrap_or(100), filter) {
        Ok(runs) => axum::Json(runs).into_response(),
        Err(e) => e.into_response(),
    }
}

/// `GET /api/clients` — who has run tests, most recently active first.
async fn list_clients(State(state): State<AppState>) -> Response {
    let Some(history) = state.history.as_ref() else {
        return axum::Json(Vec::<()>::new()).into_response();
    };
    match history.clients() {
        Ok(clients) => axum::Json(clients).into_response(),
        Err(e) => e.into_response(),
    }
}

/// `GET /__down?bytes=N` — N bytes of throwaway payload.
async fn down(State(state): State<AppState>, Query(q): Query<TransferQuery>) -> Response {
    let started = Instant::now();

    let bytes = match q.parse() {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };

    if bytes > state.config.server.max_transfer_bytes {
        return over_cap(bytes, state.config.server.max_transfer_bytes);
    }

    // Timed up to here only: the handler's work is done and the body is about
    // to start streaming, which is exactly the interval the engine wants to
    // subtract from time-to-first-byte.
    let body = Body::new(state.payload.body(bytes));
    let mut resp = Response::new(body);
    measurement_headers(resp.headers_mut(), started);
    resp
}

/// `POST /__up?bytes=N` — drains the body, then answers.
async fn up(State(state): State<AppState>, Query(q): Query<TransferQuery>, body: Body) -> Response {
    use http_body_util::BodyExt;

    let started = Instant::now();

    let declared = match q.parse() {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };

    if declared > state.config.server.max_transfer_bytes {
        return over_cap(declared, state.config.server.max_transfer_bytes);
    }

    // Read to completion before responding. Frames are dropped as they arrive,
    // so a 250 MB upload costs one frame of memory, not 250 MB — and, crucially,
    // no response byte leaves before the last request byte lands.
    let cap = state.config.server.max_transfer_bytes;
    let mut received: u64 = 0;
    let mut stream = body.into_data_stream();
    while let Some(frame) = stream.frame().await {
        match frame {
            Ok(f) => {
                if let Some(data) = f.data_ref() {
                    received += data.len() as u64;
                    // A client that keeps sending past the cap is not the
                    // engine; stop rather than absorb it indefinitely.
                    if received > cap {
                        return over_cap(received, cap);
                    }
                }
            }
            // A truncated upload is the client's problem, but the measurement
            // is void — say so rather than reporting a fast, short transfer.
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        }
    }

    let mut resp = Response::new(Body::empty());
    measurement_headers(resp.headers_mut(), started);
    resp.headers_mut()
        .insert("x-received-bytes", header_value(received.to_string()));
    resp
}

fn over_cap(requested: u64, cap: u64) -> Response {
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        format!("requested {requested} bytes, server cap is {cap}"),
    )
        .into_response()
}

/// Headers every measurement response carries.
fn measurement_headers(headers: &mut header::HeaderMap, started: Instant) {
    let dur_ms = started.elapsed().as_secs_f64() * 1000.0;

    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    // Without this the browser serves the second identical `bytes=N` request
    // from cache and the engine measures the cache, not the network.
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, no-cache, must-revalidate"),
    );
    headers.insert(
        "server-timing",
        header_value(format!("{SERVER_TIMING_METRIC};dur={dur_ms:.3}")),
    );
}

fn header_value(s: String) -> HeaderValue {
    HeaderValue::from_str(&s).unwrap_or_else(|_| HeaderValue::from_static(""))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    /// What this deployment calls itself. Drives the heading and tab title, so
    /// two installations on one LAN are tellable apart.
    site_name: String,
    version: &'static str,
    git_sha: &'static str,
    profile: String,
    history_enabled: bool,
    /// The requesting client's address.
    ///
    /// Stands in for the "server location" panel on speed.cloudflare.com,
    /// which needs external map tiles — those would leave the LAN, which is
    /// the one thing this project must not do. Knowing which machine you are
    /// testing from is the genuinely useful half of that panel anyway.
    client_ip: String,
    server_profile_description: String,
}

async fn status(State(state): State<AppState>, client: ClientAddr) -> impl IntoResponse {
    axum::Json(Status {
        site_name: state.config.server.site_name.clone(),
        version: VERSION,
        git_sha: GIT_SHA,
        profile: state.config.profile.clone(),
        history_enabled: state.history.is_some(),
        client_ip: client.ip(),
        server_profile_description: state.config.active_profile().description.clone(),
    })
}

/// Engine configuration handed to the browser.
///
/// Everything the front end passes to `new SpeedTest({...})` originates here,
/// so the settings that keep traffic on the LAN are decided server-side rather
/// than trusting the bundle to have been built correctly.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EngineConfig {
    download_api_url: &'static str,
    upload_api_url: &'static str,
    /// `null` disables the engine's aggregate reporting to Cloudflare. This is
    /// the single most important field in the response.
    log_aim_api_url: Option<()>,
    /// Also `null`; per-measurement logging is off by default but pinned here
    /// so a future engine default cannot switch it on.
    log_measurement_api_url: Option<()>,
    auto_start: bool,
    measurements: Vec<Measurement>,
    estimated_server_time: f64,
    measure_download_loaded_latency: bool,
    measure_upload_loaded_latency: bool,
    /// Both of these override engine defaults that are wrong for a LAN — see
    /// `Profile` in config.rs for why leaving them alone silently removes the
    /// loaded-latency figures and every AIM rating.
    loaded_request_min_duration: f64,
    loaded_latency_throttle: f64,
    /// Present only when a TURN relay is configured. With both user and pass
    /// set, the engine never contacts `turnServerCredsApiUrl`.
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_server_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_server_user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_server_pass: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileResponse {
    profile: String,
    description: String,
    packet_loss_enabled: bool,
    engine_config: EngineConfig,
}

/// Which profile to hand out. Absent means the server's configured default.
#[derive(Debug, Deserialize)]
struct ProfileQuery {
    #[serde(default)]
    name: Option<String>,
}

/// One entry in the profile picker.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileSummary {
    name: String,
    description: String,
    /// The link speed the profile's transfer sizes were chosen for.
    nominal_bps: Option<f64>,
    /// Whether automatic selection may choose it.
    auto_selectable: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfilesResponse {
    /// The server's configured profile, used when the client expresses no
    /// preference.
    default: String,
    profiles: Vec<ProfileSummary>,
}

/// `GET /api/profiles` — what the picker can offer.
async fn profiles(State(state): State<AppState>) -> impl IntoResponse {
    let cfg = &state.config;
    axum::Json(ProfilesResponse {
        default: cfg.profile.clone(),
        profiles: cfg
            .profiles
            .iter()
            .map(|(name, p)| ProfileSummary {
                name: name.clone(),
                description: p.description.clone(),
                nominal_bps: p.nominal_bps,
                auto_selectable: p.auto_selectable,
            })
            .collect(),
    })
}

async fn profile(State(state): State<AppState>, Query(q): Query<ProfileQuery>) -> Response {
    let cfg = &state.config;

    // A client may ask for a different profile, but only by name and only one
    // that exists — the name indexes the configured map and is never used to
    // build anything.
    let name = match q.name {
        Some(n) if !n.is_empty() => n,
        _ => cfg.profile.clone(),
    };
    let Some(active) = cfg.profiles.get(&name) else {
        return (StatusCode::BAD_REQUEST, format!("unknown profile '{name}'")).into_response();
    };

    let turn_on = cfg.turn.enabled;

    // Drop the packet-loss stage when no relay is configured, rather than
    // letting the engine stall on a connection that cannot be established.
    let measurements: Vec<Measurement> = active
        .measurements
        .iter()
        .filter(|m| turn_on || m.kind != "packetLoss")
        .cloned()
        .collect();

    axum::Json(ProfileResponse {
        profile: name,
        description: active.description.clone(),
        packet_loss_enabled: turn_on,
        engine_config: EngineConfig {
            download_api_url: "/__down",
            upload_api_url: "/__up",
            log_aim_api_url: None,
            log_measurement_api_url: None,
            auto_start: false, // the front end starts the run explicitly
            measurements,
            estimated_server_time: active.estimated_server_time,
            measure_download_loaded_latency: active.measure_download_loaded_latency,
            measure_upload_loaded_latency: active.measure_upload_loaded_latency,
            loaded_request_min_duration: active.loaded_request_min_duration,
            loaded_latency_throttle: active.loaded_latency_throttle,
            turn_server_uri: turn_on.then(|| cfg.turn.uri.clone()),
            turn_server_user: turn_on.then(|| cfg.turn.user.clone()),
            turn_server_pass: turn_on.then(|| cfg.turn.pass.clone()),
        },
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine's own regex, transcribed from
    /// `src/engines/BandwidthEngine/BandwidthEngine.ts` (1.13.1). If our header
    /// stops matching this, latency silently falls back to a fixed estimate.
    fn engine_accepts(server_timing: &str) -> Option<f64> {
        // (?:^|,\s*)cfReq(?:uest)?Dur(?:ation)?;\s*dur=([0-9.]+)  — case-insensitive
        let hay = server_timing.to_ascii_lowercase();
        for part in hay.split(',') {
            let part = part.trim();
            for name in ["cfrequestduration", "cfrequestdur", "cfreqdur"] {
                if let Some(rest) = part.strip_prefix(name) {
                    let rest = rest.trim_start();
                    if let Some(v) = rest.strip_prefix(';') {
                        if let Some(v) = v.trim_start().strip_prefix("dur=") {
                            if let Ok(parsed) = v.trim().parse::<f64>() {
                                // SERVER_TIME_MIN_DURATION in the engine.
                                return (parsed > 0.01).then_some(parsed);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    #[test]
    fn our_server_timing_header_matches_the_engine_parser() {
        let mut headers = header::HeaderMap::new();
        let started = Instant::now();
        std::thread::sleep(std::time::Duration::from_millis(2));
        measurement_headers(&mut headers, started);

        let value = headers.get("server-timing").unwrap().to_str().unwrap();
        assert!(
            value.starts_with("cfRequestDuration;dur="),
            "unexpected header: {value}"
        );
        let parsed =
            engine_accepts(value).unwrap_or_else(|| panic!("engine would reject {value:?}"));
        assert!(parsed >= 2.0, "expected >=2ms, got {parsed}");
    }

    #[test]
    fn a_plain_dur_header_would_not_be_accepted() {
        // Guards the assumption that made this worth testing: the obvious
        // `server-timing: dur=1.2` spelling is ignored by the engine.
        assert!(engine_accepts("dur=1.2").is_none());
        assert!(engine_accepts("total;dur=1.2").is_none());
        assert!(engine_accepts("cfRequestDuration;dur=1.2").is_some());
        assert!(engine_accepts("cfReqDur;dur=1.2").is_some());
    }

    #[test]
    fn measurement_responses_are_not_cacheable() {
        let mut headers = header::HeaderMap::new();
        measurement_headers(&mut headers, Instant::now());
        let cc = headers
            .get(header::CACHE_CONTROL)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cc.contains("no-store"), "cache-control was {cc}");
    }

    #[test]
    fn bytes_parameter_parsing() {
        let q = |v: Option<&str>| TransferQuery {
            bytes: v.map(str::to_string),
        };
        assert_eq!(q(Some("100000")).parse().unwrap(), 100_000);
        assert_eq!(q(Some("0")).parse().unwrap(), 0);
        // A latency ping with no parameter at all is still valid.
        assert_eq!(q(None).parse().unwrap(), 0);
        assert_eq!(q(Some("")).parse().unwrap(), 0);
        assert!(q(Some("abc")).parse().is_err());
        assert!(q(Some("-1")).parse().is_err());
    }
}
