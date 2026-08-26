# Development

## Prerequisites

- Rust stable (1.82+)
- Node 22+
- Docker (for the container build and the coturn relay used in tests)

## Setup

```sh
git clone https://github.com/sremich/lan-speedtest.git
cd lan-speedtest

npm --prefix frontend ci
cargo build --release --manifest-path backend/Cargo.toml
```

If your working copy sits inside a synced folder (OneDrive, Dropbox), build
out of tree — a Cargo target directory generates an enormous amount of sync
churn:

```sh
export CARGO_TARGET_DIR=$HOME/.cargo-target/speedtest
```

Never trust a synced `node_modules` or target directory from another machine.
Recreate them.

## Run it

```sh
npm --prefix frontend run build            # -> frontend/dist

SPEEDTEST_CONFIG=config/speedtest.toml \
SPEEDTEST_STATIC_DIR=frontend/dist \
SPEEDTEST_PROFILE=quick \
  ./backend/target/release/lan-speedtest
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
cargo fmt    --manifest-path backend/Cargo.toml
cargo clippy --manifest-path backend/Cargo.toml --all-targets -- -D warnings
cargo test   --manifest-path backend/Cargo.toml --release -- --nocapture
npm --prefix frontend run typecheck
npm --prefix frontend run test:unit
```

See [Testing](Testing.md) for the tiers above these.

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

## Regenerating the documentation screenshots

The screenshots in this wiki are produced by `frontend/scripts/screenshots.mjs`
against a real backend. Nothing in them is mocked or drawn: they are a genuine
run of whatever link the script is pointed at, which is the point — a
screenshot of fabricated numbers teaches the reader to expect a page that does
not exist.

Start a backend with history enabled, a relay configured, and trusted proxies
set, then run the script:

```sh
docker compose -f docker-compose.e2e.yml up -d      # the test relay

SPEEDTEST_CONFIG=config/speedtest.toml \
SPEEDTEST_STATIC_DIR=frontend/dist \
SPEEDTEST_PROFILE=lan-1g \
SPEEDTEST_HISTORY_DB=/tmp/shots.db \
SPEEDTEST_METRICS=1 \
SPEEDTEST_TRUSTED_PROXIES=127.0.0.1/32 \
SPEEDTEST_TURN_ENABLED=true SPEEDTEST_TURN_URI=127.0.0.1:3478 \
SPEEDTEST_TURN_USER=e2e SPEEDTEST_TURN_PASS=e2e-not-a-secret \
  ./backend/target/release/lan-speedtest &

node frontend/scripts/screenshots.mjs
```

Three notes on that invocation:

- **The relay has to be reachable from the browser that Playwright drives**, on
  the same address the engine is handed. Both peer connections relay through
  it, so if the browser cannot reach `127.0.0.1:3478` the run ends in an ICE
  timeout and every screenshot carries an error banner.
- **`SPEEDTEST_TRUSTED_PROXIES` is what produces more than one client** in the
  history screenshots. The script sends `X-Forwarded-For` from a couple of
  extra addresses, through the same path a real reverse proxy would use.
- `SHOT_BASE` and `SHOT_OUT` override the backend URL and the output directory.

Start from an empty history database. The script adds runs but does not remove
them, so re-running it against a database that already has runs in it produces
a history screenshot with more rows than it should have.

## Versioning

The version lives **only** in the `VERSION` file at the repo root.
`backend/build.rs` reads it (or the `APP_VERSION` build argument that CI
passes) and exposes it as a compile-time constant. `Cargo.toml` and
`package.json` both carry an inert `0.0.0` that is never used to express a
release; do not hand-edit them. CI refuses any tag that disagrees with
`VERSION`.

The git SHA comes from `APP_GIT_SHA` at build time and shows as `unknown` in a
local build that does not set it.

## A note on local throughput figures

Loopback is not a link. On one development machine WSL2's loopback measured
0.70 Gbps with a bare Python socket and 0.64 Gbps with `nc` — any local
throughput number is bounded by that, not by the code.

Local figures are useful for spotting order-of-magnitude regressions and
nothing else. Real throughput is a question for real hardware.

---

See also: [Testing](Testing.md) ·
[Architecture](Architecture.md) ·
[Engine Contract](Engine-Contract.md) ·
[Configuration](Configuration.md)
