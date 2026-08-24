# Configuration

Two places: `config/speedtest.toml` for everything, and environment variables
that override it. Environment always wins, so a container can be retuned
without rebuilding an image.

## Measurement profiles

A profile decides which stages run, how large each transfer is, and how the
engine interprets the results. Select one with `profile = "..."` or
`SPEEDTEST_PROFILE`.

Shipped profiles:

| Profile | For |
|---|---|
| `lan-1g` | 1 GbE clients |
| `lan-10g` | 10 GbE clients |
| `quick` | Fast smoke test; used by the e2e suite |
| `e2e-packetloss` | Packet-loss e2e, paired with `docker-compose.e2e.yml` |

Measurement entries mirror the engine's own `MeasurementConfig` exactly,
camelCase included, so they can be read against its documentation:

```toml
measurements = [
  { type = "latency",  numPackets = 20 },
  { type = "download", bytes = 25000000, count = 4 },
  { type = "upload",   bytes = 25000000, count = 4 },
  { type = "packetLoss", numPackets = 1000, batchSize = 10,
    batchWaitTime = 10, responsesWaitTime = 3000 },
]
```

## Sizing a profile

Three engine behaviours constrain the numbers, and all three are easy to get
wrong in a way that produces plausible but wrong output.

**1. Every listed stage will run.** The engine stops issuing further rounds of
a direction once one request exceeds `bandwidthFinishRequestDuration`
(1000 ms). Almost nothing on a LAN reaches that, so the whole list executes.
Keep lists short and transfers large.

**2. Transfers must be slow enough to count as "load".**
`loaded_request_min_duration` filters which transfer sizes contribute
loaded-latency samples. The engine's default is 250 ms. A 250 MB download at
10 Gbps takes 200 ms — under the default, *nothing* qualifies, loaded latency
stays 0, and because 0 is falsy the engine then returns **no quality ratings at
all**. There is no error; the section is simply empty.

Set it below the duration of the profile's smallest non-warm-up transfer. A
unit test checks this against each shipped profile's nominal link speed and
fails the build if a profile drifts.

**3. Pings need room inside a transfer.** `loaded_latency_throttle` is the
minimum gap between loaded-latency pings; the engine's 400 ms default yields at
most one sample inside a short LAN transfer, and jitter needs two.

**4. Do not go above ~250 MB.** The engine reads every response body through
`r.text()`, decoding the whole payload into a JavaScript string. Past roughly
250 MB that costs more than it buys.

## Profile keys

| Key | Meaning |
|---|---|
| `description` | Shown in the page footer |
| `measurements` | The stage list (above) |
| `estimated_server_time` | Fallback when a response carries no usable `server-timing`. `0.0` is right for this backend |
| `measure_download_loaded_latency` | Measure latency during download |
| `measure_upload_loaded_latency` | Measure latency during upload |
| `loaded_request_min_duration` | See sizing note 2 |
| `loaded_latency_throttle` | See sizing note 3 |

## Server keys

| Key | Default | Meaning |
|---|---|---|
| `bind` | `0.0.0.0:8080` | Listen address inside the container |
| `max_transfer_bytes` | 2 GiB | Hard per-request ceiling. Startup fails if a profile exceeds it |
| `download_chunk_bytes` | 4 MiB | Shared payload buffer, sliced per request |
| `static_dir` | `static` | Built front-end assets |

## TURN

```toml
[turn]
enabled = false
uri  = ""     # host:port, no scheme
user = ""
pass = ""     # supply via SPEEDTEST_TURN_PASS
```

When `enabled` is false the packet-loss stage is stripped from the profile sent
to the browser, so the engine does not stall waiting on a relay that is not
there. When true, all three of uri/user/pass must be set — startup fails
otherwise, because a half-configured relay makes the engine fetch credentials
from Cloudflare instead.

See [TURN and Packet Loss](TURN-and-Packet-Loss.md).

## Environment variables

| Variable | Overrides |
|---|---|
| `SPEEDTEST_CONFIG` | Path to the TOML file |
| `SPEEDTEST_PROFILE` | `profile` |
| `SPEEDTEST_BIND` | `server.bind` |
| `SPEEDTEST_STATIC_DIR` | `server.static_dir` |
| `SPEEDTEST_TURN_ENABLED` | `turn.enabled` |
| `SPEEDTEST_TURN_URI` | `turn.uri` |
| `SPEEDTEST_TURN_USER` | `turn.user` |
| `SPEEDTEST_TURN_PASS` | `turn.pass` |
| `SPEEDTEST_LOG` | Log filter (`info`, `debug`, …) |

Unknown keys in the TOML file are a hard error rather than being ignored — a
typo must not leave you believing a setting took effect.
