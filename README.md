# lan-speedtest

A LAN speed test in the spirit of speed.cloudflare.com — download, upload, idle
and loaded latency, jitter, packet loss and quality ratings — that runs
entirely inside your own network and never contacts Cloudflare's edge.

Cloudflare open-sourced their measurement engine as
[`@cloudflare/speedtest`](https://github.com/cloudflare/speedtest), but not the
site that drives it. This project is the rest: a Rust backend that satisfies the
engine's endpoint contract, a front end that drives it, a coturn relay for the
packet-loss stage, and the provisioning to stand it up.

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

The only external interaction anywhere in the project is the ACME DNS-01
challenge at certificate-renewal time — on the guest, on a timer, never during
a test.

## Quick start

```sh
cp .env.example .env          # fill in; never commit it
docker compose up -d
```

Then open the host in a browser. The test starts on load.

To run from source, see [docs/wiki/Development.md](docs/wiki/Development.md).

## Configuration

One TOML file with named measurement profiles — `lan-1g`, `lan-10g`, `quick` —
selectable with `SPEEDTEST_PROFILE` and applied on restart, no rebuild.

Profiles matter more than they look. The engine's defaults are tuned for
internet paths and are actively wrong on a LAN: leave
`loadedRequestMinDuration` at its 250 ms default and no LAN transfer is slow
enough to count as loading the connection, so loaded latency is never measured
— which silently removes **every** quality rating, with no error shown.
[docs/wiki/Configuration.md](docs/wiki/Configuration.md) covers the sizing
rules.

## Documentation

The [wiki](docs/wiki/Home.md) is the reference.
[Engine Contract](docs/wiki/Engine-Contract.md) is the page to read before
touching the backend: it records the `@cloudflare/speedtest` request and
response contract as verified from the package's own sources, along with the
two engine behaviours that shape the whole design.

## Status

Pre-1.0 and pre-release by policy. The backend, front end and packet-loss relay
work and are covered by tests at two tiers. Guest provisioning and TLS
automation (0.4.0) and results history (0.5.0) are still to come; 1.0.0 is
gated on 10 GbE validation and a packet capture on real hardware.

Two limitations worth knowing up front, both inherited from the engine:

- It measures with a **single** sequential HTTP stream and reads every response
  body through `r.text()`, so browser-reported download speed is bounded by
  that design rather than by your link. The backend is verified separately not
  to be the bottleneck, and a parallel-stream throughput harness is planned for
  0.6.0 as a distinct number.
- **Quality ratings need Chrome on a fast LAN.** Firefox coarsens resource
  timing to ~1 ms, so a sub-millisecond round trip rounds to zero — and the
  engine treats a zero loaded latency as unavailable and returns no scores. The
  page says so rather than showing an empty panel. Bandwidth, jitter and packet
  loss are unaffected.

## Licence

Not currently licensed for redistribution. `@cloudflare/speedtest` is MIT.
