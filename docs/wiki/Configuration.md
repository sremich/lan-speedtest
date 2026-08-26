# Configuration

Two places: `config/speedtest.toml` for everything, and environment variables
that override it. Environment always wins, so a container can be retuned
without rebuilding an image.

Unknown keys in the TOML file are a **hard error** rather than being ignored —
a typo must not leave you believing a setting took effect. The one exception is
`site_name`: a blank value falls back to the default rather than rendering an
empty heading, because refusing to boot over a cosmetic string would be worse
than the mistake.

## Measurement profiles

A profile decides which stages run, how large each transfer is, and how the
engine interprets the results. Select one with `profile = "..."` or
`SPEEDTEST_PROFILE`.

| Profile | For |
|---|---|
| `lan-1g` | 1 GbE clients |
| `lan-2.5g` | 2.5 GbE clients |
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

Four engine behaviours constrain the numbers, and all four are easy to get
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

Set it below the duration of the profile's smallest non-warm-up transfer — in
**both** directions. A unit test checks this against each shipped profile's
nominal link speed and fails the build if a profile drifts. It has already
caught a real mistake.

**3. Pings need room inside a transfer.** `loaded_latency_throttle` is the
minimum gap between loaded-latency pings; the engine's 400 ms default yields at
most one sample inside a short LAN transfer, and jitter needs two.

**4. Do not go above ~250 MB.** The engine reads every response body through
`r.text()`, decoding the whole payload into a JavaScript string. Past roughly
250 MB that costs more than it buys.

## Profile keys

| Key | Meaning |
|---|---|
| `description` | Shown in the profile picker and the page footer |
| `measurements` | The stage list (above) |
| `estimated_server_time` | Fallback when a response carries no usable `server-timing`. `0.0` is right for this backend |
| `measure_download_loaded_latency` | Measure latency during download |
| `measure_upload_loaded_latency` | Measure latency during upload |
| `loaded_request_min_duration` | See sizing note 2 |
| `loaded_latency_throttle` | See sizing note 3 |
| `nominal_bps` | The link speed these transfer sizes were chosen for |
| `auto_selectable` | Whether `Auto` in the picker may choose this profile |

## Choosing a profile

`profile` (or `SPEEDTEST_PROFILE`) is the **default**, not a fixed
setting for everyone. A picker in the page lets a client choose another; the
choice is remembered per browser and recorded with every stored run, so
mixed-profile history stays honest.

**`Auto`** measures the link with one short transfer and then picks the largest
`auto_selectable` profile whose `nominal_bps` is within a factor of two of what
it measured. The factor of two is deliberate: a single stream reaches only part
of a fast link, so requiring the full nominal rate would always choose the
smaller profile.

This matters more than it looks. The profiles differ in transfer size, and a
size chosen for the wrong link measures the wrong thing — 25 MB on 10 GbE
finishes in about 20 ms, which is mostly request overhead rather than
throughput.

Mark only real link profiles `auto_selectable`. `quick` and `e2e-packetloss`
are sized for loopback and would badly under-measure any real client.

Changing the profile does **not** start a run. It clears the figures on screen
— they were measured under the old profile, and leaving them under the new
label would misattribute them — and waits for Retest.

## Server keys

All under `[server]`.

| Key | Default | Meaning |
|---|---|---|
| `site_name` | `LAN Speed Test` | Page heading and browser tab title |
| `bind` | `0.0.0.0:8080` | Plain-HTTP listen address |
| `max_transfer_bytes` | 2 GiB | Hard per-request ceiling. Startup fails if a profile exceeds it |
| `download_chunk_bytes` | 4 MiB | Shared payload buffer, sliced per request |
| `static_dir` | `static` | Built front-end assets |
| `tls_bind` | `0.0.0.0:443` | HTTPS listen address |
| `tls_cert_file` | unset | PEM chain. Setting exactly one of the pair is a startup error |
| `tls_key_file` | unset | PEM private key |
| `history_db` | `data/history.db` | SQLite file. **Empty disables history entirely** |
| `autostart` | `true` | Measure as soon as the page loads |
| `metrics` | `false` | Serve `/metrics` — see [History and Metrics](History-and-Metrics.md) |
| `retain_runs_days` | `0` | Delete runs older than this; 0 keeps forever |
| `retain_samples_days` | `0` | Drop the sample blob, keep the run; 0 keeps forever |
| `trusted_proxies` | `[]` | CIDR blocks whose `X-Forwarded-For` may be believed. Empty means the connection's peer decides, which is the right default with no proxy in front |

### `[server.reverse_dns]`

Off by default. See [Client Identity](Client-Identity.md) for what it does,
what it deliberately will not do, and why the range restriction is not
optional.

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `false` | Look up client hostnames at all |
| `resolver` | `""` | `host:port`. Empty reads the first `nameserver` from `/etc/resolv.conf` |
| `ranges` | RFC 1918, `100.64.0.0/10`, `fc00::/7` | The only addresses ever looked up |
| `timeout_ms` | `500` | Per-query timeout |
| `ttl_secs` | `21600` | How long a name — or a remembered miss — is trusted |

Every CIDR and the resolver address are parsed at startup, so a typo is a
refusal to boot rather than a feature that silently never works.

## Auto-start

The page measures as soon as it loads. That is what makes it useful as a
bookmark, and it is on by default.

It is also several hundred megabytes, which is the last thing you want during
the video call that made you suspicious of the network. So it can be turned
off, three ways, most specific first:

| How | Scope |
|---|---|
| `?autostart=0` on the URL | This visit only, never remembered |
| The **Auto-start** toggle beside Retest | This browser, remembered |
| `server.autostart` / `SPEEDTEST_AUTOSTART` | The deployment's default |

The URL override is deliberately not remembered: a link someone sends you
should not silently reconfigure your browser.

`SPEEDTEST_AUTOSTART` reads negatives as well as `0` — `off`, `false` and `no`
all work, because "off" is a more natural thing to write for a switch called
autostart, and silently ignoring it would leave the deployment measuring on
every page load while the operator believed otherwise.

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
| `SPEEDTEST_SITE_NAME` | `server.site_name` |
| `SPEEDTEST_BIND` | `server.bind` |
| `SPEEDTEST_STATIC_DIR` | `server.static_dir` |
| `SPEEDTEST_TLS_BIND` | `server.tls_bind` |
| `SPEEDTEST_TLS_CERT_FILE` | `server.tls_cert_file` |
| `SPEEDTEST_TLS_KEY_FILE` | `server.tls_key_file` |
| `SPEEDTEST_HISTORY_DB` | `server.history_db`; an empty value disables history |
| `SPEEDTEST_AUTOSTART` | `server.autostart` |
| `SPEEDTEST_METRICS` | `server.metrics` |
| `SPEEDTEST_RETAIN_RUNS_DAYS` | `server.retain_runs_days` |
| `SPEEDTEST_RETAIN_SAMPLES_DAYS` | `server.retain_samples_days` |
| `SPEEDTEST_TRUSTED_PROXIES` | `server.trusted_proxies` (comma-separated) |
| `SPEEDTEST_REVERSE_DNS` | `server.reverse_dns.enabled` (`1`/`true`/`yes`) |
| `SPEEDTEST_DNS_RESOLVER` | `server.reverse_dns.resolver` |
| `SPEEDTEST_TURN_ENABLED` | `turn.enabled` |
| `SPEEDTEST_TURN_URI` | `turn.uri` |
| `SPEEDTEST_TURN_USER` | `turn.user` |
| `SPEEDTEST_TURN_PASS` | `turn.pass` |
| `SPEEDTEST_LOG` | Log filter (`info`, `debug`, …) |

## Naming a deployment

The heading and tab title come from the server, not from the built bundle, so
one image can serve several installations and renaming one needs a restart
rather than a rebuild. Two of these on the same LAN are otherwise
indistinguishable in a browser tab.

```sh
echo 'SPEEDTEST_SITE_NAME=Rack Room Speed Test' >> .env
docker compose up -d
```

The name goes into a compose `env_file`, which has no quoting, so keep it to
one line and avoid `#`.

## Retention

Both windows are **off by default** (`0` = keep forever). Quietly deleting a
homelab's measurement history because a default said so is not a behaviour
anyone should have to discover and opt out of. See
[History and Metrics](History-and-Metrics.md#retention) for what the two
windows do and why there are two.

---

See also: [Quick Start](Quick-Start.md) ·
[Engine Contract](Engine-Contract.md) ·
[History and Metrics](History-and-Metrics.md) ·
[Client Identity](Client-Identity.md) ·
[Troubleshooting](Troubleshooting.md)
