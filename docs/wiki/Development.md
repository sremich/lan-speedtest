# Development

## Prerequisites

- Rust stable (1.82+)
- Node 22+
- Docker (for the container build and the coturn relay used in tests)

Development happens on Windows; builds and containers happen in WSL Debian.

## Setup

```sh
git clone <repo-url>
cd <repo>

# Build out-of-tree. The working copy is OneDrive-synced and a Cargo target
# directory will otherwise generate an enormous amount of sync churn.
export CARGO_TARGET_DIR=$HOME/.cargo-target/speedtest

npm --prefix frontend ci
cargo build --release --manifest-path backend/Cargo.toml
```

Never trust a synced `node_modules` or target directory from another machine —
recreate them.

## Run it

```sh
npx --prefix frontend vite build            # -> frontend/dist

SPEEDTEST_CONFIG=config/speedtest.toml \
SPEEDTEST_STATIC_DIR=frontend/dist \
SPEEDTEST_PROFILE=quick \
  $CARGO_TARGET_DIR/release/lan-speedtest
```

Then open `http://127.0.0.1:8080/`. The test starts on load.

For front-end iteration, run Vite's dev server instead — it proxies `/__down`,
`/__up` and `/api` to a backend on port 8080, preserving the same-origin
property the engine depends on:

```sh
npm --prefix frontend run dev
```

## Checks

```sh
cargo fmt   --manifest-path backend/Cargo.toml
cargo clippy --manifest-path backend/Cargo.toml --all-targets -- -D warnings
cargo test  --manifest-path backend/Cargo.toml --release -- --nocapture
npm --prefix frontend run typecheck
```

## Container

```sh
docker build \
  --build-arg VERSION="$(cat VERSION)" \
  --build-arg GIT_SHA="$(git rev-parse --short=12 HEAD)" \
  -t speedtest:dev .
docker run --rm -p 8080:8080 speedtest:dev
```

Run the real container whenever the Dockerfile or entrypoint changes — the
image being buildable is not the same as the image working.

## Versioning

The version lives **only** in the `VERSION` file at the repo root.
`backend/build.rs` reads it (or the `APP_VERSION` build argument that CI
passes) and exposes it as a compile-time constant. `Cargo.toml` and
`package.json` both carry an inert `0.0.0` that is never used to express a
release; do not hand-edit them. CI refuses any tag that disagrees with
`VERSION`.

## A note on this dev machine

Its WSL2 loopback is slow — measured at 0.70 Gbps with a bare Python socket and
0.64 Gbps with `nc`. Any local throughput figure is bounded by that, not by the
code. Local numbers are useful only for spotting order-of-magnitude
regressions; real throughput is a tier-4 question answered on hardware.
