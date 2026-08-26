# Testing

Three tiers. Each gates something specific, and the higher ones are
deliberately rare so that day-to-day work stays fast.

| Tier | What it is | Gates |
|---|---|---|
| 1. Unit | `cargo test` — config parsing, payload body, and the engine contract driven over a real socket; shellcheck over the coturn installer | Every push, every image build |
| 2. e2e | Playwright drives the real engine in Chromium and Firefox against the real backend; a loopback throughput floor; a history suite; a packet-loss run against real coturn; the built image is booted and probed | Backend routes, front-end engine wiring, measurement profiles |
| 3. On hardware | A full run from a LAN browser against a real deployment, and a relay verified with Trickle-ICE | Before shipping a change to the relay, TLS, or anything in the data path |

CI runs every tier-1 and tier-2 job on every push: **Backend**, **Shell**,
**e2e**, **History**, **Packet loss via TURN**, and **Docker image**.

## Running them

```sh
# Tier 1 — backend, plus the throughput floor
cargo test --manifest-path backend/Cargo.toml --release -- --nocapture

# Tier 1 — front end
npm --prefix frontend run typecheck
npm --prefix frontend run test:unit

# Tier 1 — the relay installer
shellcheck provisioning/coturn/install-coturn.sh

# Tier 2 (needs both built first)
npm --prefix frontend run build
cargo build --release --manifest-path backend/Cargo.toml
(cd frontend && npx playwright install --with-deps chromium firefox)
npm --prefix frontend run test:e2e

# Tier 2, packet loss (needs coturn)
docker compose -f docker-compose.e2e.yml up -d
# start the backend with SPEEDTEST_PROFILE=e2e-packetloss and the TURN vars set,
# then:
SPEEDTEST_E2E_NO_SERVER=1 SPEEDTEST_E2E_PACKETLOSS=1   npx playwright test packetloss                            # from frontend/
```

The packet-loss and history suites skip themselves when their environment
variable is unset, rather than failing on an absent relay or an absent
database.

## What the tests actually protect

Most of them exist because the corresponding failure is **silent** — the engine
keeps working and reports a wrong number.

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
- **Measurement responses forbid caching.** Otherwise the browser serves repeat
  requests from cache and the engine measures the cache.
- **Profiles keep loaded latency measurable.** Checked against each profile's
  nominal link speed; a profile whose smallest transfer is faster than its own
  `loaded_request_min_duration` fails the build. This one has already caught a
  real mistake.
- **The packet-loss stage is dropped when no relay is configured**, and both
  TURN credentials appear when one is.
- **`/metrics` refuses rather than falling through.** With metrics off the
  route must return 404 from its own handler; a test that only checked the
  status code once passed for the wrong reason, because its `static_dir`
  pointed at a directory that did not exist and so the app-shell fallback
  failed too. It now asserts the fallback is genuinely answering first.

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

## What is not covered

A 10 GbE saturation check against real hardware was planned and **waived**. The
claim it would have supported — that the backend can saturate a 10 GbE link —
is therefore untested at that rate, and the throughput floor over loopback is
what stands in for it.

The other thing that check would have proven, that no traffic leaves the
network, is instead proven by the end-to-end suite on every push: a real
browser drives a complete run and the test fails if a single request leaves the
origin. That is a stronger guarantee than a one-off packet capture, because it
runs again every time anything changes.

---

See also: [Development](Development.md) ·
[Engine Contract](Engine-Contract.md) ·
[TURN and Packet Loss](TURN-and-Packet-Loss.md) ·
[Troubleshooting](Troubleshooting.md)
