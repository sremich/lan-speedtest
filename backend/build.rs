//! Surfaces the project version and git SHA into the binary.
//!
//! The version lives ONLY in the repo-root `VERSION` file. In CI/Docker the
//! release workflow passes it as a build arg (exported as `APP_VERSION`), so
//! that wins; a local build falls back to reading `../VERSION` directly.
//! Cargo.toml's own `version` field is deliberately not consulted.

use std::path::Path;

fn main() {
    let version = std::env::var("APP_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| read_trimmed("../VERSION"))
        .unwrap_or_else(|| "0.0.0".to_string());

    let git_sha = std::env::var("APP_GIT_SHA")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=APP_VERSION={version}");
    println!("cargo:rustc-env=APP_GIT_SHA={git_sha}");
    println!("cargo:rerun-if-changed=../VERSION");
    println!("cargo:rerun-if-env-changed=APP_VERSION");
    println!("cargo:rerun-if-env-changed=APP_GIT_SHA");
}

fn read_trimmed(p: &str) -> Option<String> {
    let path = Path::new(p);
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
