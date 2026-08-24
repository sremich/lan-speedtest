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
  └─ all of the above is same-origin. Nothing else is contacted.
```

## What runs where

| Component | Where | Why |
|---|---|---|
| Backend + front end | Docker container on the guest | One image, one pin to bump, reproducible |
| coturn | Natively on the same guest | A contiguous UDP relay range through Docker's userland proxy adds a hop to the exact path being measured |
| TLS termination | On the guest | Wildcard certificate via ACME DNS-01 |
| Measurement engine | The visitor's browser | It is the thing doing the measuring; the backend only has to feed it |

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

## Why the front end is deliberately small

No framework. The engine does the measuring, the browser does the rendering,
and the page has one screen and one interaction. A framework would be more
code to audit for the one property that matters here: that nothing else gets
fetched.

## Trust boundary

Everything inside the LAN. The only external interaction anywhere in the
project is the ACME DNS-01 challenge at certificate-renewal time, which happens
on the guest, on a timer, and never during a test.

The front end refuses to start a run whose configuration could reach outside —
absolute endpoint URLs, a non-null reporting URL, an incomplete TURN credential
pair, or any of the engine's external reachability probe stages. It fails
loudly rather than measuring and quietly reporting.
