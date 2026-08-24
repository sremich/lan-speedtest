# Testing

Four tiers. Each gates something specific, and the higher ones are deliberately
rare so that day-to-day work stays fast.

| Tier | What it is | Gates |
|---|---|---|
| 1. Unit | `cargo test` — config parsing, payload body, and the engine contract driven over a real socket | Every push, every image build |
| 2. e2e | Playwright drives the real engine in Chromium and Firefox against the real backend; a loopback throughput floor; a packet-loss run against real coturn; the built image is booted and probed | Backend routes, front-end engine wiring, measurement profiles |
| 3. Live | Provisioning against the real hypervisor; a full run from a LAN browser; relay verified via Trickle-ICE | Provisioning or coturn changes, before shipping |
| 4. Hardware | 10 GbE saturation and a packet capture proving no external traffic | **1.0.0.** Until it passes, releases stay pre-releases |

## Running them

```sh
# Tier 1 + the throughput floor
cargo test --manifest-path backend/Cargo.toml --release -- --nocapture

# Tier 2 (needs both built first)
npx --prefix frontend vite build
cargo build --release --manifest-path backend/Cargo.toml
npx --prefix frontend playwright install --with-deps chromium firefox
npm --prefix frontend run test:e2e

# Tier 2, packet loss (needs coturn)
docker compose -f docker-compose.e2e.yml up -d
# start the backend with SPEEDTEST_PROFILE=e2e-packetloss and the TURN vars set,
# then:
SPEEDTEST_E2E_PACKETLOSS=1 npx --prefix frontend playwright test packetloss
```

The packet-loss suite skips itself when `SPEEDTEST_E2E_PACKETLOSS` is unset,
rather than failing on an absent relay.

## What the tests actually protect

Most of them exist because the corresponding failure is **silent** — the
engine keeps working and reports a wrong number.

- **`logAimApiUrl` is null.** Otherwise every completed result is POSTed to
  Cloudflare. Asserted in the contract tests, re-asserted in the browser, and
  backed by an end-to-end check that no request leaves the origin for the whole
  run.
- **`server-timing` matches the engine's parser.** The test transcribes the
  engine's own regex and also asserts that the obvious wrong spelling would be
  rejected, so the test cannot rot into tautology.
- **`/__up` withholds its response until the body is drained.** Fed a slow
  body, the response must not arrive early. Upload speed is time-to-first-byte
  alone, so answering early makes uploads look instantaneous.
- **Measurement responses forbid caching.** Otherwise the browser serves
  repeat requests from cache and the engine measures the cache.
- **Profiles keep loaded latency measurable.** Checked against each profile's
  nominal link speed; a profile whose smallest transfer is faster than its own
  `loaded_request_min_duration` fails the build. This one has already caught a
  real mistake.
- **The packet-loss stage is dropped when no relay is configured**, and both
  TURN credentials appear when one is.

## The throughput floor

`backend/tests/throughput.rs` pulls several parallel streams over loopback and
asserts a floor, taking the best of three passes because shared runners are
noisy. It prints what it achieved so the number the floor was chosen against
stays visible in the log.

It is **not** a prediction of browser-reported speed. The engine is
single-stream and decodes every body through `r.text()`, so it always reads
lower. This test asserts one thing: the backend is not the bottleneck. It
catches regressions of kind — per-request allocation, an accidental full-body
buffer — which cost an order of magnitude.
