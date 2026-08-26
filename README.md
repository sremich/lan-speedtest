# lan-speedtest

A LAN speed test in the spirit of speed.cloudflare.com — download, upload, idle
and loaded latency, jitter, packet loss and quality ratings — that runs
entirely inside your own network and never contacts Cloudflare's edge.

![A completed run](https://raw.githubusercontent.com/wiki/sremich/lan-speedtest/images/overview.png)

Cloudflare open-sourced their measurement engine as
[`@cloudflare/speedtest`](https://github.com/cloudflare/speedtest), but not the
site that drives it. This project is the rest: a Rust backend that satisfies the
engine's endpoint contract, a front end that drives it, a coturn relay for the
packet-loss stage, and the container that serves it all.

Built for a home lab, where the useful question is not "how fast is the pipe"
but "is this connection healthy" — which is exactly what latency under load,
jitter and packet loss answer and a plain throughput test does not.

## What it measures

| | |
|---|---|
| Download / upload | Bandwidth in each direction |
| Latency, idle | Round trip on an unloaded link |
| Latency, loaded | Round trip while saturating down and up — the number that exposes bufferbloat |
| Jitter | Variation in round trip |
| Packet loss | UDP burst relayed back to the browser through your own TURN server |
| Suitability | Streaming, gaming and video-call ratings, from the engine's own scoring |
| Live traces | Download and upload drawn as they are measured, with the reported percentile marked |
| Distribution | Every sample, per transfer size and latency phase: min, max, mean, median, 25th/75th percentile and outliers |
| Raw throughput | Parallel-stream figure, on demand, reported separately from the engine's single-stream result |
| History | Every run stored, with a trend chart, per-client filtering, and a permalink that redraws the run rather than summarising it |
| Descriptions | A free-text note per run — where you were, on what, what you were testing |
| Location tags | Optional tag per run (living room, upstairs office, …); filter the history by location to find the network's weak spots |
| Compare | Two runs side by side, with the difference computed and signed by improvement rather than by arithmetic |
| Metrics | Optional Prometheus endpoint: the latest run per client in base units, plus bufferbloat factor and AIM quality scores per service |

## Nothing leaves your network

This is the property the project exists for, so it is enforced in four places
rather than trusted:

- The engine's result-reporting endpoint is pinned to `null` server-side. Left
  alone, it POSTs every completed run to Cloudflare.
- The front end re-checks the configuration before starting and refuses to run
  if anything could reach off-LAN.
- A contract test asserts the served configuration contains no absolute URLs.
- An end-to-end test drives a full run in a real browser and fails if a single
  request leaves the origin.

Nothing in the running system contacts anything outside your network — not at
startup, not during a test, not when a run is stored.

## Quick start

```sh
git clone https://github.com/sremich/lan-speedtest.git
cd lan-speedtest
cp .env.example .env          # fill in; never commit it
docker compose up -d
```

Then open the host in a browser. The test starts on load.

Comment out the `SPEEDTEST_TLS_*` lines for a first run — they point at a
certificate you do not have yet, and the service refuses to start rather than
silently falling back to plain HTTP.

Full walkthrough: [Quick Start](https://github.com/sremich/lan-speedtest/wiki/Quick-Start). Running from source:
[Development](https://github.com/sremich/lan-speedtest/wiki/Development).

## Configuration

One TOML file with named measurement profiles — `lan-1g`, `lan-2.5g`,
`lan-10g`, `quick` — selectable with `SPEEDTEST_PROFILE` and applied on
restart, no rebuild. That setting is the *default*: a picker in the page lets a
client choose another, or `Auto`, which sizes the test to the link it measures.
The same goes for what the page calls itself: `SPEEDTEST_SITE_NAME` (or
`server.site_name`) sets the heading and the tab title, so two of these on one
LAN are tellable apart without rebuilding either.

Profiles matter more than they look. The engine's defaults are tuned for
internet paths and are actively wrong on a LAN: leave `loadedRequestMinDuration`
at its 250 ms default and no LAN transfer is slow enough to count as loading the
connection, so loaded latency is never measured — which silently removes
**every** quality rating, with no error shown.
[Configuration](https://github.com/sremich/lan-speedtest/wiki/Configuration) covers the sizing rules.

Optional point-in-time snapshots of the history database are available via
`server.history_backup_dir` (or environment variable). When set, the daily
maintenance pass writes a consistent copy to that directory — useful for
off-site backup or as a last-known-good copy of the current history.

## Who ran the test

A run is attributed by the address the connection came from — there are no
accounts here — and labelled from three sources in order of authority: a name
you typed, a hostname from reverse DNS, then the address itself. The address
stays visible either way, because a friendly label that replaced it would make
the history impossible to correlate with a DHCP lease table or a switch port.

Reverse DNS is off by default and range-restricted even when on, and behind a
reverse proxy `X-Forwarded-For` is believed only from a peer you have named as
a proxy. [Client Identity](https://github.com/sremich/lan-speedtest/wiki/Client-Identity) has the details,
including the two things that genuinely cannot be recovered: an address
translated by a Tailscale subnet router, and a browser's own private address.

## Documentation

The [wiki](https://github.com/sremich/lan-speedtest/wiki) is the reference.
[Engine Contract](https://github.com/sremich/lan-speedtest/wiki/Engine-Contract) is the page to read before
touching the backend: it records the `@cloudflare/speedtest` request and
response contract as verified from the package's own sources, along with the
two engine behaviours that shape the whole design.

## Status

Shipping and in use. Backend, front end, packet-loss relay, TLS, and results
history are all in place and covered by tests at two tiers on every push.

Releases below 1.0.0 were GitHub pre-releases; from 1.0.0 they are full
releases. A 10 GbE saturation check against real hardware was planned and
waived, so the backend is untested at that rate; the loopback throughput floor
in CI stands in for it. "No external traffic" is proven by the end-to-end suite
on every push rather than by a manual packet capture.

Standing up the host is out of scope. The project ships the application and the
coturn relay configuration; any Debian-family machine with Docker will serve
it. See [Deployment](https://github.com/sremich/lan-speedtest/wiki/Deployment).

Two limitations worth knowing up front, both inherited from the engine:

- It measures with a **single** sequential HTTP stream and reads every response
  body through `r.text()`, so its download figure is bounded by that design
  rather than by your link. The backend is verified separately not to be the
  bottleneck, and the page offers a parallel-stream harness as a distinct
  number — on a 1 GbE link the two typically differ by close to 2×, which is
  the design showing, not a fault.
- **Quality ratings need Chrome on a fast LAN.** Firefox coarsens resource
  timing to ~1 ms, so a sub-millisecond round trip rounds to zero — and the
  engine treats a zero loaded latency as unavailable and returns no scores. The
  page says so rather than showing an empty panel. Bandwidth, jitter and packet
  loss are unaffected.

## Licence

MIT — see [LICENSE](LICENSE).

Third-party notices are in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md),
which is also served by the running application at `/THIRD-PARTY-NOTICES.md`.
`@cloudflare/speedtest` is MIT, and its notice ships with every build.

Not affiliated with, sponsored by, or endorsed by Cloudflare, Inc. Cloudflare
is a trademark of Cloudflare, Inc. This project is built on their open-source
`@cloudflare/speedtest` engine, used under the MIT licence.
