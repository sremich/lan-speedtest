//! HTTP surface.
//!
//! `/__down` and `/__up` implement the request contract of
//! `@cloudflare/speedtest` (verified against the 1.13.1 sources — see
//! `https://github.com/sremich/lan-speedtest/wiki/Engine-Contract`). Two details in there are easy to get
//! wrong and both are load-bearing:
//!
//! 1. The engine's `server-timing` parser only accepts the metric names
//!    `cfRequestDuration` (and the `cfReqDur` / `cfRequestDur` abbreviations)
//!    or a sum of `cfSpeed*` entries. A plain `server-timing: dur=1.2` is
//!    silently ignored.
//! 2. Upload speed is derived from time-to-first-byte alone. If we answer
//!    before the request body is fully drained, upload figures become
//!    fiction. `up()` therefore reads the body to completion first.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{ConnectInfo, FromRequestParts, Path, Query, Request, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use tower_http::services::{ServeDir, ServeFile};

use crate::config::{Config, Measurement};
use crate::history::{self, History, HistoryError, ResultSubmission};
use crate::netid::{self, Cidr};
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
    /// Proxies whose `X-Forwarded-For` may be believed, parsed once at
    /// startup. Config validation has already proved these parse.
    pub trusted_proxies: Arc<Vec<Cidr>>,
    /// Reverse-lookup settings, absent when the feature is off.
    pub reverse_dns: Option<Arc<ReverseDns>>,
}

/// Everything a reverse lookup needs, resolved from config once.
#[derive(Debug)]
pub struct ReverseDns {
    /// Only addresses inside one of these are ever looked up.
    pub ranges: Vec<Cidr>,
    /// Explicit resolver, or `None` to read `/etc/resolv.conf` at lookup time.
    pub resolver: Option<SocketAddr>,
    pub timeout: Duration,
    pub ttl: Duration,
}

impl ReverseDns {
    /// The resolver to ask, preferring the configured one.
    ///
    /// `/etc/resolv.conf` is read per lookup rather than cached: in a
    /// container it can be rewritten underneath us, and a stale resolver fails
    /// silently.
    pub fn resolver(&self) -> Option<SocketAddr> {
        if let Some(explicit) = self.resolver {
            return Some(explicit);
        }
        let contents = std::fs::read_to_string("/etc/resolv.conf").ok()?;
        netid::resolver_from_resolv_conf(&contents)
    }

    pub fn covers(&self, ip: IpAddr) -> bool {
        self.ranges.iter().any(|c| c.contains(ip))
    }
}

impl AppState {
    /// Derives the parsed forms config validation has already checked.
    pub fn new(config: Config, payload: PayloadSource, history: Option<Arc<History>>) -> Self {
        let trusted = config
            .server
            .trusted_proxies
            .iter()
            .filter_map(|raw| Cidr::parse(raw).ok())
            .collect::<Vec<_>>();

        let rdns = config.server.reverse_dns.clone();
        let reverse_dns = rdns.enabled.then(|| {
            Arc::new(ReverseDns {
                ranges: rdns
                    .ranges
                    .iter()
                    .filter_map(|r| Cidr::parse(r).ok())
                    .collect(),
                resolver: rdns.resolver.parse().ok(),
                timeout: Duration::from_millis(rdns.timeout_ms),
                ttl: Duration::from_secs(rdns.ttl_secs),
            })
        });

        Self {
            payload,
            config: Arc::new(config),
            history,
            trusted_proxies: Arc::new(trusted),
            reverse_dns,
        }
    }
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

    // Everything that changes stored state, grouped so the same-origin guard is
    // applied once. A fourth mutating endpoint added here inherits it; one
    // added below does not, which is why these three live apart.
    let mutating = Router::new()
        .route("/api/results", post(record_result))
        .route("/api/results/{id}/note", post(set_note))
        .route("/api/clients/{ip}/name", post(set_client_name))
        // `route_layer`, not `layer`: the guard should run for these paths and
        // not for anything that falls through to the static handler.
        .route_layer(middleware::from_fn(same_origin));

    Router::new()
        .route("/__down", get(down))
        .route("/__up", post(up))
        .route("/api/status", get(status))
        .route("/api/profile", get(profile))
        .route("/api/profiles", get(profiles))
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/results/{id}", get(get_result))
        .route("/api/history", get(list_history))
        .route("/api/clients", get(list_clients))
        .merge(mutating)
        // Always routed, and 404 from the handler when it is turned off.
        //
        // It used to be left unrouted instead, which looked equivalent and was
        // not: the SPA fallback answers anything unrouted, so on a deployment
        // that actually serves the front end `/metrics` returned 200 and the
        // app shell. A scrape would have read an HTML page as an exposition
        // and reported a permanently healthy target with no series in it.
        .route("/metrics", get(metrics))
        .fallback_service(statics)
        .with_state(state)
}

/// Refuses a state-changing request that a browser says came from somewhere
/// else.
///
/// This is a rebinding/CSRF guard, not authentication. The tool is
/// deliberately unauthenticated on a trusted LAN — anyone who can reach it can
/// record a run, and that is the intended design. What this stops is a page on
/// an unrelated origin *silently* driving those endpoints in a visitor's
/// browser: a script on `evil.example` cannot forge the `Origin` header, so
/// its cross-site POST is refused rather than filing a run or renaming a
/// client under the visitor's address.
///
/// A request with **no** `Origin` at all is allowed through unchanged. curl,
/// the test suite and anything that is not a browser never send one, and a
/// header the attacker's browser attaches automatically is only useful when it
/// is present. Refusing on absence would break every non-browser caller while
/// adding nothing: the case being defended against always has the header.
async fn same_origin(req: Request, next: Next) -> Response {
    // `Origin` is a single value by definition; a request carrying two of them
    // is malformed, and `to_str` failing means it was not even ASCII. Both are
    // treated as a mismatch rather than as absence.
    if let Some(origin) = req.headers().get(header::ORIGIN) {
        let origin = origin.to_str().unwrap_or("");
        // HTTP/2 carries the authority in the pseudo-header rather than in
        // `Host`, which reaches us on the URI.
        let host = req
            .headers()
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .or_else(|| req.uri().authority().map(|a| a.as_str()));

        let matches = host.is_some_and(|host| origin_matches_host(origin, host));
        if !matches {
            return (StatusCode::FORBIDDEN, "cross-origin request refused").into_response();
        }
    }
    next.run(req).await
}

/// Whether an `Origin` header value names the same host and port as the
/// request's `Host`.
///
/// The scheme is deliberately ignored — the same deployment answers on both
/// http and https, and which one a browser used says nothing about where the
/// page came from. The *port* is not ignored, because a different port on the
/// same host is a different origin to the browser and should be one here too.
/// A port that is the default for the origin's scheme is dropped from both
/// sides first, so `https://speedtest.example` and `speedtest.example:443` are
/// recognised as the one origin.
///
/// `Origin: null` — a sandboxed iframe, a `file://` page, some
/// cross-origin redirects — never matches. That is precisely the case where
/// the browser has declined to say where the request came from, so there is
/// nothing to compare and the answer is no.
fn origin_matches_host(origin: &str, host: &str) -> bool {
    let Some(origin) = origin_authority(origin) else {
        return false;
    };
    let origin = normalized_authority(origin);
    !origin.is_empty() && origin == normalized_authority(host)
}

/// The `host[:port]` part of a serialised origin, or `None` if it does not look
/// like one — which includes the literal `null`.
fn origin_authority(origin: &str) -> Option<&str> {
    let (_scheme, rest) = origin.split_once("://")?;
    // An origin carries no path, but truncating at one costs nothing and keeps
    // a stray `https://host/` from being compared as a host called `host/`.
    Some(rest.split(['/', '?', '#']).next().unwrap_or(rest))
}

/// Lower-cased `host[:port]` with a default port and any IPv6 brackets removed,
/// so two spellings of one origin compare equal.
fn normalized_authority(authority: &str) -> String {
    let (host, port) = split_port(authority);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    match port {
        None | Some("80") | Some("443") => host.to_ascii_lowercase(),
        Some(port) => format!("{}:{}", host.to_ascii_lowercase(), port),
    }
}

/// Splits `host:port`, knowing that an IPv6 literal is full of colons that are
/// not the port separator.
fn split_port(authority: &str) -> (&str, Option<&str>) {
    let colon = match authority.rfind(']') {
        // Bracketed: only a colon after the closing bracket can be a port.
        Some(bracket) => authority[bracket..].find(':').map(|i| bracket + i),
        // Unbracketed and more than one colon is a bare IPv6 literal, which has
        // no port at all — `::1:8080` is an address, not `::1` on port 8080.
        None if authority.find(':') != authority.rfind(':') => None,
        None => authority.find(':'),
    };
    match colon {
        Some(i) => (&authority[..i], Some(&authority[i + 1..])),
        None => (authority, None),
    }
}

/// Prometheus exposition of the most recent run per client.
async fn metrics(State(state): State<AppState>) -> Response {
    if !state.config.server.metrics {
        // Off: say so definitively rather than letting the static fallback
        // answer with the app shell.
        return (StatusCode::NOT_FOUND, "metrics are not enabled").into_response();
    }

    let Some(history) = state.history.as_ref() else {
        // Configured on, but nothing to report from. Still a valid body, so a
        // scrape sees a live target rather than an error it has to alert on.
        return (
            [(header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)],
            crate::metrics::render(VERSION, GIT_SHA, 0, &[]),
        )
            .into_response();
    };

    let total = history.count().unwrap_or(0);
    let latest = history.latest_per_client().unwrap_or_default();
    (
        [(header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)],
        crate::metrics::render(VERSION, GIT_SHA, total, &latest),
    )
        .into_response()
}

/// The version Prometheus itself advertises for the text format.
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// The connection's peer address and any forwarding header, or `None` when the
/// server was started without connect info (which is how the backend-only
/// tests run).
///
/// The header is captured but **not** trusted here. `effective` decides
/// whether to believe it, and only does so when the peer is one of the
/// configured trusted proxies — otherwise anyone on the LAN could attribute a
/// run to any address they liked by setting one header.
///
/// Infallible on purpose: a missing peer address should degrade to "unknown"
/// rather than reject an otherwise valid request.
#[derive(Debug, Clone)]
pub struct ClientAddr {
    pub peer: Option<SocketAddr>,
    pub forwarded_for: Option<String>,
}

impl ClientAddr {
    /// The address to attribute this request to.
    pub fn effective(&self, state: &AppState) -> Option<IpAddr> {
        self.peer.map(|peer| {
            netid::effective_client(
                peer.ip(),
                self.forwarded_for.as_deref(),
                &state.trusted_proxies,
            )
        })
    }

    pub fn ip(&self, state: &AppState) -> String {
        self.effective(state)
            .map(|a| a.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    }
}

impl<S: Send + Sync> FromRequestParts<S> for ClientAddr {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Every `x-forwarded-for` line, not just the first. The header is
        // legally repeatable and proxies split on this: some append a new line
        // per hop rather than extending one comma-joined list. Reading only
        // the first drops the hops after it, which for a chain means trusting
        // the wrong end of it. Joined here so the parser sees one list, and
        // truncated after joining so the cap bounds the whole thing.
        let joined = parts
            .headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect::<Vec<_>>()
            .join(",");
        let forwarded_for = if joined.is_empty() {
            None
        } else {
            Some(joined.chars().take(400).collect())
        };
        Ok(ClientAddr {
            forwarded_for,
            peer: parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ConnectInfo(addr)| *addr),
        })
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
            // Snapshot failures come from the background pass, not from a
            // request. If one ever reaches a handler it is ours, not the
            // caller's.
            HistoryError::Snapshot { .. } | HistoryError::SnapshotOverwritesDatabase { .. } => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
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
    let client_ip = client.ip(&state);
    let result = history.record(&submission, &client_ip, &user_agent(&headers), &recorded_at);

    // Naming the client happens off the response path. A resolver that is slow
    // or absent must not delay the POST that ends a test run.
    if result.is_ok() {
        if let Some(ip) = client.effective(&state) {
            maybe_resolve_name(&state, ip);
        }
    }

    match result {
        Ok(id) => (
            StatusCode::CREATED,
            axum::Json(Recorded { id, recorded_at }),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}

/// Looks up what a client is called, if that is switched on and due.
///
/// Deliberately fire-and-forget: the name is a label on a history row, and
/// nothing about a completed measurement should depend on DNS.
fn maybe_resolve_name(state: &AppState, ip: IpAddr) {
    let (Some(rdns), Some(history)) = (state.reverse_dns.clone(), state.history.clone()) else {
        return;
    };
    if !rdns.covers(ip) {
        return;
    }

    let key = ip.to_string();

    // A remembered answer — including a remembered miss — stands until its TTL
    // expires. Without this, a client with no PTR record is looked up again
    // after every single run.
    if let Ok(Some((_, resolved_at))) = history.resolved_name(&key) {
        if let Some(age) = history::age_since(&resolved_at) {
            if age < rdns.ttl {
                return;
            }
        }
    }

    tokio::spawn(async move {
        let Some(resolver) = rdns.resolver() else {
            tracing::debug!("reverse dns: no resolver available");
            return;
        };
        let found = netid::reverse_lookup(resolver, ip, rdns.timeout)
            .await
            .unwrap_or_default();
        let at = history::now_rfc3339();
        if let Err(e) = history.record_resolved_name(&ip.to_string(), &found, &at) {
            tracing::debug!("reverse dns: could not store name: {e}");
        }
    });
}

/// `GET /api/results/{id}` — one run in full, for a permalink.
async fn get_result(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let Some(history) = state.history.as_ref() else {
        return (StatusCode::NOT_FOUND, "history is disabled").into_response();
    };
    match history.by_id(id) {
        Ok(Some(run)) => axum::Json(run).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "no such run").into_response(),
        Err(e) => e.into_response(),
    }
}

/// The longest note kept. Long enough for "upstairs landing, laptop on
/// battery, checking the new AP", short enough that the history stays a table.
const MAX_NOTE_CHARS: usize = 280;

#[derive(Debug, Deserialize)]
struct NoteBody {
    #[serde(default)]
    note: String,
}

/// `POST /api/results/{id}/note` — annotate a run, or clear the note.
///
/// The note belongs to the run, not the client: it records what was being
/// tested and from where, which is exactly the thing that differs between two
/// runs from the same machine.
async fn set_note(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    axum::Json(body): axum::Json<NoteBody>,
) -> Response {
    let Some(history) = state.history.as_ref() else {
        return (StatusCode::NOT_FOUND, "history is disabled").into_response();
    };

    // Counted in characters, not bytes, so the cap does not depend on which
    // alphabet someone writes in — and truncation cannot split a code point.
    let note: String = body.note.trim().chars().take(MAX_NOTE_CHARS).collect();

    match history.set_note(id, &note) {
        Ok(true) => (StatusCode::NO_CONTENT, ()).into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "no such run").into_response(),
        Err(e) => e.into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct NameBody {
    #[serde(default)]
    name: String,
}

/// `POST /api/clients/{ip}/name` — name a client, or clear the name.
///
/// Unauthenticated, like everything else here: this is a LAN tool with no
/// accounts, and the same is already true of submitting a result. The value is
/// length-capped and stored as text, never interpreted.
async fn set_client_name(
    State(state): State<AppState>,
    Path(ip): Path<String>,
    axum::Json(body): axum::Json<NameBody>,
) -> Response {
    let Some(history) = state.history.as_ref() else {
        return (StatusCode::NOT_FOUND, "history is disabled").into_response();
    };

    // The path segment must be an address. Anything else is a client bug, and
    // accepting it would let history accumulate rows keyed by nonsense.
    if ip.parse::<IpAddr>().is_err() {
        return (StatusCode::BAD_REQUEST, "not an IP address").into_response();
    }

    let name: String = body.name.trim().chars().take(60).collect();
    match history.set_client_name(&ip, &name) {
        Ok(()) => (StatusCode::NO_CONTENT, ()).into_response(),
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

    let me = client.ip(&state);
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
    /// What kind of address that is: `lan`, `cgnat`, `public`, …
    ///
    /// A `10.x` address that arrived through a Tailscale subnet router is
    /// indistinguishable from a LAN client — the router rewrote the source
    /// before the packet reached us. Saying which kind it is, is the honest
    /// amount of certainty available.
    client_kind: &'static str,
    client_kind_label: &'static str,
    server_profile_description: String,
    /// Whether the page should measure as soon as it loads.
    ///
    /// The deployment's default only. A browser that has been told otherwise
    /// keeps its own choice — this is what a first visit does.
    autostart: bool,
}

async fn status(State(state): State<AppState>, client: ClientAddr) -> impl IntoResponse {
    let ip = client.effective(&state);
    let kind = ip.map(netid::classify);
    axum::Json(Status {
        site_name: state.config.server.site_name.clone(),
        version: VERSION,
        git_sha: GIT_SHA,
        profile: state.config.profile.clone(),
        history_enabled: state.history.is_some(),
        client_ip: ip
            .map(|a| a.to_string())
            .unwrap_or_else(|| "unknown".into()),
        client_kind: kind.map(netid::Kind::slug).unwrap_or("unknown"),
        client_kind_label: kind.map(netid::Kind::label).unwrap_or("unknown"),
        server_profile_description: state.config.active_profile().description.clone(),
        autostart: state.config.server.autostart,
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
    fn an_origin_matches_the_host_it_was_served_from() {
        // The pairs a browser actually produces against this service: the
        // engine's own fetches, a bookmark on the LAN name, an address literal.
        for (origin, host) in [
            ("http://speedtest.example:8080", "speedtest.example:8080"),
            ("https://speedtest.example", "speedtest.example"),
            ("http://192.0.2.10:8080", "192.0.2.10:8080"),
            ("http://[2001:db8::1]:8080", "[2001:db8::1]:8080"),
            // The scheme is not part of the comparison — the same deployment
            // answers on both, and the page came from the same place either way.
            ("http://speedtest.example:8080", "speedtest.example:8080"),
            // A default port is the same origin whether it is spelled out or
            // not; the browser omits it, a hand-typed URL may not.
            ("https://speedtest.example", "speedtest.example:443"),
            ("http://speedtest.example:80", "speedtest.example"),
            // Host names are case-insensitive.
            ("http://SpeedTest.Example:8080", "speedtest.example:8080"),
        ] {
            assert!(
                origin_matches_host(origin, host),
                "{origin} should be same-origin with {host}"
            );
        }
    }

    #[test]
    fn a_foreign_origin_never_matches() {
        for (origin, host) in [
            // The case this guard exists for: a page somewhere else driving our
            // endpoints in a visitor's browser.
            ("http://evil.example", "speedtest.example:8080"),
            // A different port on the same host is a different origin to the
            // browser, so it is one here too.
            ("http://speedtest.example:8081", "speedtest.example:8080"),
            ("http://speedtest.example", "speedtest.example:8080"),
            ("http://speedtest.example:8080", "speedtest.example"),
            // A prefix or suffix of the host is not the host.
            ("http://speedtest.example.evil.test", "speedtest.example"),
            ("http://evilspeedtest.example", "speedtest.example"),
            // A sandboxed iframe or a file:// page. The browser is declining to
            // say where the request came from, which is not a match.
            ("null", "speedtest.example"),
            ("", "speedtest.example"),
            // Not an origin at all.
            ("speedtest.example", "speedtest.example"),
            ("http://", "speedtest.example"),
        ] {
            assert!(
                !origin_matches_host(origin, host),
                "{origin} must not be treated as same-origin with {host}"
            );
        }
    }

    #[test]
    fn an_ipv6_literal_is_not_mistaken_for_a_host_and_port() {
        // The last colon of `2001:db8::1` is not a port separator. Reading it as
        // one would make two different addresses compare equal.
        assert_eq!(
            split_port("[2001:db8::1]:8080"),
            ("[2001:db8::1]", Some("8080"))
        );
        assert_eq!(split_port("[2001:db8::1]"), ("[2001:db8::1]", None));
        assert_eq!(split_port("2001:db8::1"), ("2001:db8::1", None));
        assert_eq!(split_port("192.0.2.10:8080"), ("192.0.2.10", Some("8080")));
        assert_eq!(split_port("192.0.2.10"), ("192.0.2.10", None));
        assert!(!origin_matches_host(
            "http://[2001:db8::2]:8080",
            "[2001:db8::1]:8080"
        ));
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
