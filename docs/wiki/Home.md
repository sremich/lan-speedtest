# lan-speedtest

A LAN speed test that behaves like speed.cloudflare.com — download, upload,
idle and loaded latency, jitter, packet loss and quality ratings — but runs
entirely inside your own network and never contacts Cloudflare's edge.

Cloudflare open-sourced the measurement engine as
[`@cloudflare/speedtest`](https://github.com/cloudflare/speedtest); the front
end at speed.cloudflare.com is not open source. This project supplies the rest:
a backend that satisfies the engine's endpoint contract, a front end that
drives it, a coturn relay for the packet-loss stage, and the provisioning to
stand it all up.

> **This wiki is public-safe.** No real addresses, hostnames or credentials
> appear anywhere in it. Placeholders are used throughout — substitute your own.

## Pages

| Page | What it covers |
|---|---|
| [Engine Contract](Engine-Contract.md) | **Start here before touching the backend.** The `@cloudflare/speedtest` request/response contract as verified from source, the config keys that keep traffic local, and the two engine behaviours that shape the whole design |
| [Architecture](Architecture.md) | How the pieces fit: request flow, what runs where, why each choice |
| [Reading the Results](Reading-the-Results.md) | What each figure means, the distribution view, and why raw throughput is a separate number |
| [Configuration](Configuration.md) | `config/speedtest.toml`, measurement profiles, environment variables |
| [Development](Development.md) | Local setup, build, run, the WSL/OneDrive specifics |
| [Testing](Testing.md) | The four tiers, what each gates, how to run them |
| [TURN and Packet Loss](TURN-and-Packet-Loss.md) | coturn setup, credentials, verifying a relay candidate |
| [Deployment](Deployment.md) | The guest, the container, TLS |
| [Troubleshooting](Troubleshooting.md) | Symptoms and their causes, especially the silent ones |
| [Release History](Release-History.md) | What shipped in each version |

## The short version

- **Backend** — Rust/Axum. Serves `/__down` and `/__up` plus the built front
  end. Download payloads are refcounted slices of one pre-allocated buffer, so
  there is no per-request allocation in the data path.
- **Front end** — TypeScript/Vite, no framework. Drives the engine, renders
  live results and AIM ratings, dark by default.
- **coturn** — natively on the same guest, for the packet-loss stage. The
  engine measures loss by relaying a UDP burst from the browser back to itself.
- **Config** — one TOML file with named measurement profiles. The engine's own
  defaults are tuned for internet paths and are actively wrong on a LAN; see
  [Configuration](Configuration.md).

## Three things that will bite you

1. **`logAimApiUrl` defaults to a Cloudflare endpoint** and the engine POSTs
   every completed result to it. It must be `null`. Pinned server-side,
   re-checked client-side, and asserted by tests at two tiers.
2. **`server-timing` must be spelled `cfRequestDuration;dur=N`.** Any other
   spelling — including the obvious `dur=N` — is silently ignored, and latency
   quietly falls back to a fixed estimate.
3. **`loadedRequestMinDuration` defaults to 250 ms**, above the duration of
   almost any LAN transfer. Leave it alone and loaded latency is never
   measured, which in turn removes *every* quality rating with no error shown.
