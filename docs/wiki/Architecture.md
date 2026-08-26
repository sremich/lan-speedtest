# Architecture

## Request flow

```
browser
  │  GET /                    → static bundle (Rust serves it)
  │  GET /api/status          → version, git SHA, active profile
  │  GET /api/profile         → the full engine configuration
  │
  │  ── engine starts ──
  │  GET /__down?bytes=0      → latency ping (no separate ping endpoint)
  │  GET /__down?bytes=N      → N bytes of throwaway payload
  │  POST /__up?bytes=N       → N bytes discarded, answered only once drained
  │  UDP  ⇄ coturn ⇄ UDP      → packet-loss burst, browser to itself via relay
  │
  │  ── run finishes ──
  │  POST /api/results        → stored, with its samples, for the permalink
  │
  └─ all of the above is same-origin. Nothing else is contacted.
```

Every route is listed in [HTTP API](HTTP-API.md).

## What runs where

| Component | Where | Why |
|---|---|---|
| Backend + front end | One container on the host | One image, one pin to bump, reproducible |
| coturn | Natively on the same host | A contiguous UDP relay range through Docker's userland proxy adds a hop to the exact path being measured |
| TLS termination | In the backend itself | A proxy hop would sit inside the path being measured |
| Measurement engine | The visitor's browser | It is the thing doing the measuring; the backend only has to feed it |
| Stored history | SQLite on a host volume | Survives image updates; a single file to back up or delete |

## The four pages

| Page | What it is |
|---|---|
| `/` | The test. Measures on load unless told not to |
| `/history.html` | Stored runs, a trend chart, per-client filtering, and the entry point to Compare |
| `/result.html?id=N` | One stored run, redrawn from its samples |
| `/compare.html?a=N&b=M` | Two runs side by side, with the difference computed |

All four are static files served by the same binary, and all four read the same
JSON endpoints.

## Why the backend looks like this

**Rust/Axum.** The backend has to push bytes faster than a 10 GbE client can
pull them across several connections. It is also the one component whose
slowness would be indistinguishable from a slow network — a backend bottleneck
reads as a bad result, not as an error.

**One pre-allocated buffer.** A download response is a sequence of refcounted
slices of a single shared buffer, so serving 250 MB allocates nothing. The
response body reports an exact size hint, which lets the HTTP layer send a real
`content-length` and skip chunked framing.

**Uploads are drained, never buffered.** Frames are counted and dropped as they
arrive. A 250 MB upload costs one frame of memory. Critically, the response is
withheld until the last request byte lands — see
[Engine Contract](Engine-Contract.md) for why that is not optional.

**Configuration is server-side.** The front end asks the backend what to
measure. That keeps the settings which decide whether traffic stays local in
one auditable place rather than baked into a JavaScript bundle.

**`TCP_NODELAY` on both listeners.** The engine's latency probe is a small
response, and Nagle's algorithm plus the peer's delayed-ACK timer can hold one
for up to 40 ms. That was a real bug through three releases: it read as network
latency, and it was invisible on loopback because the round trip there is too
short for an acknowledgement to be delayed.

## Why the front end is deliberately small

No framework. The engine does the measuring, the browser does the rendering,
and the page has one screen and one interaction. A framework would be more code
to audit for the one property that matters here: that nothing else gets
fetched.

The charts, box plots and interpolation are a few hundred lines of hand-written
TypeScript with unit tests, for the same reason.

## Trust boundary

Everything inside the LAN. Nothing in the running system contacts anything
outside it — not at startup, not during a test, not when a run is stored.

The front end refuses to start a run whose configuration could reach outside —
absolute endpoint URLs, a non-null reporting URL, an incomplete TURN credential
pair, or any of the engine's external reachability probe stages. It fails
loudly rather than measuring and quietly reporting.

That property is enforced in four independent places rather than trusted:

1. `routes.rs` pins the engine's two reporting URLs to `null` server-side.
2. `assertLanOnly()` in `frontend/src/api.ts` re-checks the served
   configuration in the browser and refuses to start otherwise.
3. A contract test asserts the served configuration contains no absolute URLs.
4. An end-to-end test drives a full run in a real browser and fails if a single
   request leaves the origin.

The engine's own default for `logAimApiUrl` really is a Cloudflare endpoint
that receives every completed result, which is what makes the pinning load
bearing rather than decorative.

---

See also: [Engine Contract](Engine-Contract.md) ·
[HTTP API](HTTP-API.md) ·
[Deployment](Deployment.md) ·
[Development](Development.md)
