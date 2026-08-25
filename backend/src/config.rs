//! Server configuration and measurement profiles.
//!
//! The measurement profile decides which stages the browser-side engine runs
//! and how big each transfer is. The engine's own defaults are tuned for
//! internet paths and finish far too quickly to load a 1-10 GbE LAN, so the
//! profile is server-side and swappable without rebuilding the front end.
//!
//! Profile field names are deliberately camelCase, matching
//! `@cloudflare/speedtest`'s `MeasurementConfig` exactly, so a profile in this
//! file can be read straight against the engine's documentation.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Shown when nothing names this deployment. Deliberately generic: the point
/// of the setting is that an operator replaces it.
const DEFAULT_SITE_NAME: &str = "LAN Speed Test";
/// Default TCP port. 8080 inside the container; TLS termination and 443 are
/// handled outside it.
const DEFAULT_BIND: &str = "0.0.0.0:8080";
/// Refuses transfers larger than this. Guards against a hand-edited profile
/// (or a stray client) asking for something that would stall a worker.
const DEFAULT_MAX_TRANSFER_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Shared download buffer size. Large enough that per-frame overhead vanishes,
/// small enough to stay resident in cache.
const DEFAULT_DOWNLOAD_CHUNK_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Name of the active entry in `[profiles]`.
    pub profile: String,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub turn: TurnConfig,
    pub profiles: BTreeMap<String, Profile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// Display name for this deployment: the page heading and the browser
    /// tab title.
    ///
    /// Runtime rather than build-time, so renaming an installation needs
    /// neither a rebuild nor a new image — set `SPEEDTEST_SITE_NAME` (or edit
    /// this) and restart. One image can therefore serve several sites.
    #[serde(default = "default_site_name")]
    pub site_name: String,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_max_transfer_bytes")]
    pub max_transfer_bytes: u64,
    #[serde(default = "default_download_chunk_bytes")]
    pub download_chunk_bytes: usize,
    /// Directory of built front-end assets. A missing directory is not fatal —
    /// the API still serves, which keeps backend-only tests simple.
    #[serde(default = "default_static_dir")]
    pub static_dir: String,
    /// Where to listen for HTTPS, when a certificate is configured.
    ///
    /// The service terminates TLS itself rather than sitting behind a proxy:
    /// a proxy hop would land inside the very path being measured, and it is
    /// one more thing to keep configured and renewed.
    #[serde(default = "default_tls_bind")]
    pub tls_bind: String,
    /// Full certificate chain. TLS is enabled only when both this and the key
    /// are set; plain HTTP on `bind` continues either way, which is what the
    /// container health check uses.
    #[serde(default)]
    pub tls_cert_file: Option<String>,
    #[serde(default)]
    pub tls_key_file: Option<String>,
    /// Where completed runs are stored. Empty disables history entirely, which
    /// is what the contract tests and the e2e suite use.
    #[serde(default = "default_history_db")]
    pub history_db: String,
    /// CIDR blocks whose `X-Forwarded-For` header may be believed.
    ///
    /// Empty by default, and that default is the safe one: with no proxy in
    /// front, honouring the header would let anyone on the LAN attribute a run
    /// to any address they chose. List a proxy here only if you run one.
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
    #[serde(default)]
    pub reverse_dns: ReverseDnsConfig,
}

/// Naming clients by reverse lookup.
///
/// Restricted to explicit address ranges on purpose. Unrestricted, a
/// public-facing deployment would send PTR queries for internet addresses to
/// whatever upstream resolver it has — a quiet leak, for a cosmetic label.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReverseDnsConfig {
    #[serde(default)]
    pub enabled: bool,
    /// `host:port`. Empty means the first `nameserver` in `/etc/resolv.conf`.
    #[serde(default)]
    pub resolver: String,
    /// Only addresses inside these blocks are ever looked up.
    #[serde(default = "default_reverse_dns_ranges")]
    pub ranges: Vec<String>,
    #[serde(default = "default_reverse_dns_timeout_ms")]
    pub timeout_ms: u64,
    /// How long a resolved (or unresolvable) name is trusted before another
    /// lookup is worth making.
    #[serde(default = "default_reverse_dns_ttl_secs")]
    pub ttl_secs: u64,
}

impl Default for ReverseDnsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            resolver: String::new(),
            ranges: default_reverse_dns_ranges(),
            timeout_ms: default_reverse_dns_timeout_ms(),
            ttl_secs: default_reverse_dns_ttl_secs(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            site_name: default_site_name(),
            bind: default_bind(),
            max_transfer_bytes: default_max_transfer_bytes(),
            download_chunk_bytes: default_download_chunk_bytes(),
            static_dir: default_static_dir(),
            tls_bind: default_tls_bind(),
            tls_cert_file: None,
            tls_key_file: None,
            history_db: default_history_db(),
            trusted_proxies: Vec::new(),
            reverse_dns: ReverseDnsConfig::default(),
        }
    }
}

impl ServerConfig {
    /// TLS is on only when both halves are present. One without the other is a
    /// misconfiguration rather than a partial success, and `validate` rejects it.
    pub fn tls(&self) -> Option<(&str, &str)> {
        match (&self.tls_cert_file, &self.tls_key_file) {
            (Some(c), Some(k)) if !c.is_empty() && !k.is_empty() => Some((c, k)),
            _ => None,
        }
    }
}

/// TURN relay used by the engine's packet-loss stage.
///
/// These credentials necessarily reach the browser — the engine builds the
/// `RTCPeerConnection` client-side — so they are LAN-only long-term
/// credentials, never anything reused elsewhere. The password is supplied via
/// `SPEEDTEST_TURN_PASS` and never committed.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub uri: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub pass: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(default)]
    pub description: String,
    pub measurements: Vec<Measurement>,
    /// Passed through to the engine as `estimatedServerTime` — the fallback
    /// used when a response carries no usable `server-timing` value.
    #[serde(default)]
    pub estimated_server_time: f64,
    #[serde(default = "default_true")]
    pub measure_download_loaded_latency: bool,
    #[serde(default = "default_true")]
    pub measure_upload_loaded_latency: bool,
    /// Minimum request duration (ms) for a transfer size to count as having
    /// loaded the connection.
    ///
    /// The engine defaults this to 250 ms, which is fine on an internet path
    /// and catastrophic on a LAN: a 250 MB download at 10 Gbps takes 200 ms,
    /// so *every* size is discarded, loaded latency stays 0, and because 0 is
    /// falsy the engine then emits no AIM scores at all. Tune it below the
    /// fastest transfer a profile expects. See docs/wiki/Engine-Contract.md.
    #[serde(default = "default_loaded_request_min_duration")]
    pub loaded_request_min_duration: f64,
    /// Minimum interval (ms) between loaded-latency pings. The engine's 400 ms
    /// default yields at most one sample inside a short LAN transfer, and
    /// jitter needs two.
    #[serde(default = "default_loaded_latency_throttle")]
    pub loaded_latency_throttle: f64,
    /// Nominal link speed this profile's transfer sizes were chosen for, in
    /// bits per second.
    ///
    /// One source of truth for two consumers: `Auto` in the front end picks
    /// the fastest profile a measured link can justify, and
    /// `shipped_profiles_keep_loaded_latency_measurable` checks the transfer
    /// sizes against it. Those two used to disagree because the test carried
    /// its own hard-coded table.
    #[serde(default)]
    pub nominal_bps: Option<f64>,
    /// Whether `Auto` may choose this profile.
    ///
    /// Off by default: the small profiles exist for smoke tests and CI, and
    /// auto-selecting one would silently under-measure a real link.
    #[serde(default)]
    pub auto_selectable: bool,
}

/// One engine measurement stage. Mirrors `MeasurementConfig` from
/// `@cloudflare/speedtest`; unset fields are omitted so the engine applies its
/// own defaults rather than receiving nulls.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Measurement {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub num_packets: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bypass_min_duration: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub batch_size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub batch_wait_time: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub responses_wait_time: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub connection_timeout: Option<u32>,
}

fn default_site_name() -> String {
    DEFAULT_SITE_NAME.to_string()
}
fn default_bind() -> String {
    DEFAULT_BIND.to_string()
}
fn default_max_transfer_bytes() -> u64 {
    DEFAULT_MAX_TRANSFER_BYTES
}
fn default_download_chunk_bytes() -> usize {
    DEFAULT_DOWNLOAD_CHUNK_BYTES
}
fn default_static_dir() -> String {
    "static".to_string()
}
fn default_tls_bind() -> String {
    "0.0.0.0:443".to_string()
}
fn default_history_db() -> String {
    "data/history.db".to_string()
}
fn default_reverse_dns_ranges() -> Vec<String> {
    vec![
        "10.0.0.0/8".into(),
        "172.16.0.0/12".into(),
        "192.168.0.0/16".into(),
        "100.64.0.0/10".into(),
        "fc00::/7".into(),
    ]
}
fn default_reverse_dns_timeout_ms() -> u64 {
    500
}
fn default_reverse_dns_ttl_secs() -> u64 {
    6 * 60 * 60
}
fn default_true() -> bool {
    true
}
/// Deliberately far below the engine's own 250 ms; see the field docs.
fn default_loaded_request_min_duration() -> f64 {
    50.0
}
fn default_loaded_latency_throttle() -> f64 {
    50.0
}

#[derive(Debug)]
pub enum ConfigError {
    Read(std::io::Error),
    Parse(toml::de::Error),
    UnknownProfile {
        requested: String,
        known: Vec<String>,
    },
    LargestTransferExceedsCap {
        bytes: u64,
        cap: u64,
    },
    TurnEnabledWithoutCredentials,
    HalfConfiguredTls,
    BadCidr {
        field: &'static str,
        detail: String,
    },
    BadResolver(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(e) => write!(f, "could not read config file: {e}"),
            Self::Parse(e) => write!(f, "could not parse config file: {e}"),
            Self::UnknownProfile { requested, known } => write!(
                f,
                "profile '{requested}' is not defined; known profiles: {}",
                known.join(", ")
            ),
            Self::LargestTransferExceedsCap { bytes, cap } => write!(
                f,
                "profile requests a {bytes}-byte transfer but server.max_transfer_bytes is {cap}"
            ),
            Self::TurnEnabledWithoutCredentials => write!(
                f,
                "turn.enabled is true but uri/user/pass are not all set \
                 (pass is normally supplied via SPEEDTEST_TURN_PASS)"
            ),
            Self::HalfConfiguredTls => write!(
                f,
                "exactly one of server.tls_cert_file / server.tls_key_file is set. \
                 Set both to serve HTTPS, or neither to serve plain HTTP"
            ),
            Self::BadCidr { field, detail } => write!(f, "server.{field}: {detail}"),
            Self::BadResolver(detail) => write!(
                f,
                "server.reverse_dns.resolver: {detail} (expected host:port, e.g. 10.0.0.1:53)"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path).map_err(ConfigError::Read)?;
        let mut cfg: Config = toml::from_str(&raw).map_err(ConfigError::Parse)?;
        cfg.apply_env_overrides();
        cfg.validate()?;
        Ok(cfg)
    }

    /// Environment wins over the file, so a container can be retuned without
    /// rebuilding an image or bind-mounting a new config.
    pub fn apply_env_overrides(&mut self) {
        if let Some(v) = non_empty_env("SPEEDTEST_PROFILE") {
            self.profile = v;
        }
        if let Some(v) = non_empty_env("SPEEDTEST_SITE_NAME") {
            self.server.site_name = v;
        }
        if let Some(v) = non_empty_env("SPEEDTEST_BIND") {
            self.server.bind = v;
        }
        if let Some(v) = non_empty_env("SPEEDTEST_STATIC_DIR") {
            self.server.static_dir = v;
        }
        if let Some(v) = non_empty_env("SPEEDTEST_TLS_BIND") {
            self.server.tls_bind = v;
        }
        if let Ok(v) = std::env::var("SPEEDTEST_HISTORY_DB") {
            // Deliberately accepts an empty value: that is how history is
            // turned off, so `non_empty_env` would be wrong here.
            self.server.history_db = v.trim().to_string();
        }
        if let Some(v) = non_empty_env("SPEEDTEST_TLS_CERT_FILE") {
            self.server.tls_cert_file = Some(v);
        }
        if let Some(v) = non_empty_env("SPEEDTEST_TLS_KEY_FILE") {
            self.server.tls_key_file = Some(v);
        }
        if let Some(v) = non_empty_env("SPEEDTEST_TURN_URI") {
            self.turn.uri = v;
        }
        if let Some(v) = non_empty_env("SPEEDTEST_TURN_USER") {
            self.turn.user = v;
        }
        if let Some(v) = non_empty_env("SPEEDTEST_TURN_PASS") {
            self.turn.pass = v;
        }
        if let Some(v) = non_empty_env("SPEEDTEST_TRUSTED_PROXIES") {
            self.server.trusted_proxies = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Some(v) = non_empty_env("SPEEDTEST_REVERSE_DNS") {
            self.server.reverse_dns.enabled =
                matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
        }
        if let Some(v) = non_empty_env("SPEEDTEST_DNS_RESOLVER") {
            self.server.reverse_dns.resolver = v;
        }
        if let Some(v) = non_empty_env("SPEEDTEST_TURN_ENABLED") {
            self.turn.enabled = matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
        }

        self.normalise_site_name();
    }

    /// Trims the display name, and refuses to leave it blank.
    ///
    /// A blank name is a mistake rather than an instruction to render an empty
    /// heading, and it is cosmetic enough not to be worth refusing to boot
    /// over. Everything else in this file fails loudly; this one falls back on
    /// purpose.
    fn normalise_site_name(&mut self) {
        self.server.site_name = self.server.site_name.trim().to_string();
        if self.server.site_name.is_empty() {
            self.server.site_name = default_site_name();
        }
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let profile =
            self.profiles
                .get(&self.profile)
                .ok_or_else(|| ConfigError::UnknownProfile {
                    requested: self.profile.clone(),
                    known: self.profiles.keys().cloned().collect(),
                })?;

        // Catch a profile that would be rejected at request time by the
        // transfer cap — better to fail at startup than mid-test.
        if let Some(max) = profile.measurements.iter().filter_map(|m| m.bytes).max() {
            if max > self.server.max_transfer_bytes {
                return Err(ConfigError::LargestTransferExceedsCap {
                    bytes: max,
                    cap: self.server.max_transfer_bytes,
                });
            }
        }

        if self.turn.enabled
            && (self.turn.uri.is_empty() || self.turn.user.is_empty() || self.turn.pass.is_empty())
        {
            return Err(ConfigError::TurnEnabledWithoutCredentials);
        }

        // Half-configured TLS would otherwise fall back to plain HTTP, which
        // looks like it worked right up until a browser refuses to connect.
        if self.server.tls_cert_file.is_some() != self.server.tls_key_file.is_some() {
            return Err(ConfigError::HalfConfiguredTls);
        }

        // Parsed at startup rather than per request. A typo in a trusted-proxy
        // block would otherwise fail open — the block simply never matching —
        // which looks identical to a correctly configured deployment.
        for raw in &self.server.trusted_proxies {
            crate::netid::Cidr::parse(raw).map_err(|detail| ConfigError::BadCidr {
                field: "trusted_proxies",
                detail,
            })?;
        }
        for raw in &self.server.reverse_dns.ranges {
            crate::netid::Cidr::parse(raw).map_err(|detail| ConfigError::BadCidr {
                field: "reverse_dns.ranges",
                detail,
            })?;
        }
        if !self.server.reverse_dns.resolver.is_empty() {
            self.server
                .reverse_dns
                .resolver
                .parse::<std::net::SocketAddr>()
                .map_err(|e| ConfigError::BadResolver(e.to_string()))?;
        }

        Ok(())
    }

    pub fn active_profile(&self) -> &Profile {
        // validate() proved this key exists.
        &self.profiles[&self.profile]
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = "
profile = 'lan'
[profiles.lan]
measurements = [
  { type = 'latency', numPackets = 20 },
  { type = 'download', bytes = 100000, count = 4 },
]
";

    fn parse(s: &str) -> Config {
        let c: Config = toml::from_str(s).unwrap();
        c.validate().unwrap();
        c
    }

    #[test]
    fn parses_a_minimal_profile_and_applies_defaults() {
        let c = parse(MINIMAL);
        assert_eq!(c.profile, "lan");
        assert_eq!(c.server.bind, DEFAULT_BIND);
        assert_eq!(c.server.max_transfer_bytes, DEFAULT_MAX_TRANSFER_BYTES);
        assert!(c.active_profile().measure_download_loaded_latency);
        assert_eq!(c.active_profile().measurements.len(), 2);
    }

    #[test]
    fn measurements_serialise_to_the_engine_shape() {
        let c = parse(MINIMAL);
        let json = serde_json::to_value(&c.active_profile().measurements).unwrap();
        // camelCase keys, "type" not "kind", and no null padding for the
        // fields this stage does not use.
        assert_eq!(json[0]["type"], "latency");
        assert_eq!(json[0]["numPackets"], 20);
        assert!(json[0].get("bytes").is_none());
        assert_eq!(json[1]["type"], "download");
        assert_eq!(json[1]["bytes"], 100000);
        assert_eq!(json[1]["count"], 4);
    }

    #[test]
    fn packet_loss_stage_round_trips_every_field() {
        let c = parse(
            "
profile = 'p'
[profiles.p]
measurements = [
  { type = 'packetLoss', numPackets = 1000, batchSize = 10, batchWaitTime = 10, responsesWaitTime = 3000, connectionTimeout = 5000 },
]
",
        );
        let json = serde_json::to_value(&c.active_profile().measurements[0]).unwrap();
        assert_eq!(json["type"], "packetLoss");
        assert_eq!(json["numPackets"], 1000);
        assert_eq!(json["batchSize"], 10);
        assert_eq!(json["batchWaitTime"], 10);
        assert_eq!(json["responsesWaitTime"], 3000);
        assert_eq!(json["connectionTimeout"], 5000);
    }

    #[test]
    fn unknown_active_profile_is_rejected() {
        let mut c: Config = toml::from_str(MINIMAL).unwrap();
        c.profile = "nope".into();
        assert!(matches!(
            c.validate(),
            Err(ConfigError::UnknownProfile { .. })
        ));
    }

    #[test]
    fn profile_exceeding_the_transfer_cap_is_rejected_at_startup() {
        let mut c: Config = toml::from_str(MINIMAL).unwrap();
        c.server.max_transfer_bytes = 1000;
        assert!(matches!(
            c.validate(),
            Err(ConfigError::LargestTransferExceedsCap {
                bytes: 100000,
                cap: 1000
            })
        ));
    }

    #[test]
    fn turn_enabled_without_credentials_is_rejected() {
        let mut c: Config = toml::from_str(MINIMAL).unwrap();
        c.turn.enabled = true;
        c.turn.uri = "turn.example:3478".into();
        assert!(matches!(
            c.validate(),
            Err(ConfigError::TurnEnabledWithoutCredentials)
        ));
    }

    #[test]
    fn shipped_profiles_keep_loaded_latency_measurable() {
        // Guards the real config file, not just a fixture: every profile's
        // loaded-latency threshold must sit below the duration of its own
        // smallest non-warmup transfer, or the engine collects no loaded
        // latency and drops all AIM ratings. Checked against the profile's
        // nominal link speed.
        let raw = std::fs::read_to_string("../config/speedtest.toml")
            .expect("config/speedtest.toml is readable from the crate dir");
        let cfg: Config = toml::from_str(&raw).expect("shipped config parses");

        // Each profile states the link speed it was sized for, so this checks
        // the config against itself rather than against a table here that
        // could drift away from it. The two e2e profiles run over loopback,
        // which is far quicker than any real link: a hosted CI runner measured
        // 51 Gbps. Sizing them for anything slower let transfers finish under
        // the threshold, which silently removed every AIM rating in Firefox
        // on CI.
        for (name, profile) in &cfg.profiles {
            assert!(
                profile.nominal_bps.is_some(),
                "profile '{name}' has no nominal_bps, so nothing checks its transfer sizes"
            );
            assert!(
                profile.loaded_request_min_duration < 250.0,
                "profile '{name}' leaves loaded_request_min_duration at or above                  the engine default"
            );
            assert!(profile.loaded_latency_throttle < 400.0, "profile '{name}'");

            // Bytes per second, from the profile's own stated link speed.
            let bps = profile.nominal_bps.unwrap_or_default() / 8.0;
            // Ignore warm-up rounds, which are explicitly exempted.
            let smallest = profile
                .measurements
                .iter()
                .filter(|m| m.bypass_min_duration != Some(true))
                .filter_map(|m| m.bytes)
                .filter(|&b| b > 0)
                .min();

            if let Some(bytes) = smallest {
                let ms = (bytes as f64 / bps) * 1000.0;
                assert!(
                    ms > profile.loaded_request_min_duration,
                    "profile '{name}': its smallest transfer ({bytes} bytes) takes                      ~{ms:.0}ms at {bps:.0} B/s, which is below its                      loaded_request_min_duration of {}ms — that size would                      contribute no loaded-latency samples",
                    profile.loaded_request_min_duration
                );
            }
        }
    }

    #[test]
    fn site_name_comes_from_config_and_falls_back_when_blank() {
        let c = parse(MINIMAL);
        assert_eq!(c.server.site_name, DEFAULT_SITE_NAME);

        let mut named: Config = toml::from_str(&format!(
            "{MINIMAL}
[server]
site_name = '  Rack Room Speed Test  '
"
        ))
        .unwrap();
        named.normalise_site_name();
        assert_eq!(
            named.server.site_name, "Rack Room Speed Test",
            "surrounding whitespace should not reach the heading"
        );

        // Blank must not render an empty <h1>.
        let mut blank: Config = toml::from_str(&format!(
            "{MINIMAL}
[server]
site_name = '   '
"
        ))
        .unwrap();
        blank.normalise_site_name();
        assert_eq!(blank.server.site_name, DEFAULT_SITE_NAME);
    }

    #[test]
    fn tls_is_off_unless_both_halves_are_present() {
        let mut c: Config = toml::from_str(MINIMAL).unwrap();
        assert!(c.server.tls().is_none(), "no TLS by default");

        c.server.tls_cert_file = Some("/tls/fullchain.pem".into());
        c.server.tls_key_file = Some("/tls/privkey.pem".into());
        assert_eq!(
            c.server.tls(),
            Some(("/tls/fullchain.pem", "/tls/privkey.pem"))
        );
    }

    #[test]
    fn half_configured_tls_is_rejected_rather_than_quietly_serving_http() {
        // Falling back to plain HTTP here would look like success right up
        // until a browser refuses to connect.
        let mut c: Config = toml::from_str(MINIMAL).unwrap();
        c.server.tls_cert_file = Some("/tls/fullchain.pem".into());
        assert!(matches!(c.validate(), Err(ConfigError::HalfConfiguredTls)));

        c.server.tls_cert_file = None;
        c.server.tls_key_file = Some("/tls/privkey.pem".into());
        assert!(matches!(c.validate(), Err(ConfigError::HalfConfiguredTls)));
    }

    #[test]
    fn typos_in_the_config_are_not_silently_ignored() {
        // deny_unknown_fields: a mistyped key must fail loudly rather than
        // leaving the operator believing a setting took effect.
        let parsed = toml::from_str::<Config>(
            "
profile = 'lan'
[server]
bnid = '0.0.0.0:9'
[profiles.lan]
measurements = []
",
        );
        assert!(parsed.is_err());
    }
}
