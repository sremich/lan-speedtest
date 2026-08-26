//! lan-speedtest — LAN speed test backend.
//!
//! Serves the front end and satisfies the download/upload endpoint contract of
//! `@cloudflare/speedtest`, so the browser-side engine runs entirely against
//! this host. Nothing here talks to the internet.
//!
//! Exposed as a library so the contract tests in `tests/` can drive the real
//! router rather than a reimplementation of it.

pub mod config;
pub mod history;
pub mod metrics;
pub mod net;
pub mod netid;
pub mod payload;
pub mod routes;

pub use config::Config;
pub use history::History;
pub use payload::PayloadSource;
pub use routes::{router, AppState, GIT_SHA, VERSION};
