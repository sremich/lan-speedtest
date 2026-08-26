# The `@cloudflare/speedtest` contract

**Pinned version: 1.13.1.** Verified by reading the package's own sources, not
from documentation or memory — the published tarball ships a complete
sourcemap (`dist/speedtest.js.map`) whose `sourcesContent` contains the
original TypeScript. Recover it with:

```js
const map = JSON.parse(fs.readFileSync('node_modules/@cloudflare/speedtest/dist/speedtest.js.map', 'utf8'));
map.sources.forEach((src, i) => fs.writeFileSync(path.basename(src), map.sourcesContent[i]));
```

Re-run that and re-check this page on every version bump. The tests in
`backend/tests/contract.rs` encode what is written here; if the engine changes,
both must move together.

---

## Endpoints the engine calls

| Stage | Request | Notes |
|---|---|---|
| Download | `GET {downloadApiUrl}?bytes=N` | Response body must be exactly N bytes |
| Upload | `POST {uploadApiUrl}?bytes=N` | Body is `'0'.repeat(N)` sent as a plain string; response body is discarded |
| Latency | `GET {downloadApiUrl}?bytes=0` | **There is no separate ping endpoint** — a latency stage is a zero-byte download |

The engine may append its own query parameters (`during=download` while
measuring loaded latency, for example). The server must ignore unknown
parameters rather than reject them.

Source: `src/engines/BandwidthEngine/BandwidthEngine.ts`, and
`src/index.ts` (the `case 'latency'` branch constructs a `BandwidthEngine`
with `dir: 'down', bytes: 0`).

## Response requirements

### `server-timing` — exact spelling required

The engine's parser matches only these, case-insensitively:

```
/(?:^|,\s*)cfReq(?:uest)?Dur(?:ation)?;\s*dur=([0-9.]+)/i
/(?:^|,\s*)cfSpeed[a-zA-Z]*;\s*dur=([0-9.]+)/gi   (summed)
```

So `cfRequestDuration;dur=1.234`, `cfRequestDur;dur=…` and `cfReqDur;dur=…`
all work. **A plain `server-timing: dur=1.234` is silently ignored**, and the
engine falls back to the configured `estimatedServerTime` — wrong numbers, no
error. We emit `cfRequestDuration;dur=N`.

Values at or below `0.01` ms are treated as absent (`SERVER_TIME_MIN_DURATION`).
On a fast LAN backend the real handler time is often below that, so the
fallback path is normal and `estimatedServerTime: 0` is the right profile
setting.

### Caching

Responses must be uncacheable. The engine issues the same
`GET …?bytes=N` repeatedly; without `cache-control: no-store` the browser
serves the 2nd..Nth from cache and the engine measures the cache.

### Content length

`transferSize` from `PerformanceResourceTiming` is what download speed is
computed from, falling back to `bytes × 1.005` when unavailable. We send a real
`content-length` (an exact `size_hint` on the response body lets hyper skip
chunked framing). The engine warns to the console if `transferSize` is under
the requested size or more than 5% over it.

### Cross-origin

`transferSize` is zeroed for cross-origin responses without
`Timing-Allow-Origin`. We serve the front end and the API from the same origin,
so no CORS or TAO headers are needed anywhere. Keep it that way.

## Two behaviours that shape the backend

### 1. Upload speed is time-to-first-byte only

```ts
const calcUploadDuration = ({ ttfb }) => ttfb;   // BandwidthEngine.ts
```

There is no separate "body sent" measurement. **The server must not send any
response byte until it has drained the entire request body**, or every upload
measures as near-instantaneous. `contract.rs` asserts this by feeding the body
slowly and checking the response is withheld.

### 2. The bandwidth engine is single-stream and sequential

`#nextMeasurement()` is invoked only from inside the previous `fetch`'s
`.then()`. One request is in flight at a time, ever. On top of that, every
response is read through `r.text()`, decoding the whole payload into a
JavaScript string before the timing is recorded.

Consequences:

- Browser-reported download speed is bounded by single-stream TCP plus
  main-thread string decoding, not by the server. It will read lower than the
  link can carry.
- Payload sizes above ~250 MB cost more in decoding than they buy in
  measurement, which is why the profiles stop there.
- A parallel-stream harness is the way to get a raw throughput number. The page
  has one, on demand, and reports it separately so the two are never confused —
  see [Reading the Results](Reading-the-Results.md#raw-throughput).

## Configuration keys that keep traffic on the LAN

| Key | Default | Ours | Why |
|---|---|---|---|
| `logAimApiUrl` | `https://speed.cloudflare.com/__results` | `null` | **Every completed result is POSTed here otherwise.** The single most important setting. |
| `logMeasurementApiUrl` | `null` | `null` | Pinned so a future default cannot switch it on |
| `downloadApiUrl` | `https://speed.cloudflare.com/__down` | `/__down` | Relative, same-origin |
| `uploadApiUrl` | `https://speed.cloudflare.com/__up` | `/__up` | Relative, same-origin |
| `turnServerUri` | `turn.speed.cloudflare.com:50000` | our relay | `host:port`, **no scheme** — the engine builds `turn:{uri}?transport=udp` |
| `turnServerUser` / `turnServerPass` | `null` | set | With **either** unset the engine fetches credentials from `turnServerCredsApiUrl`, which defaults to a Cloudflare endpoint. Both must be present. |
| `rpkiInvalidHost` | `invalid.rpki.cloudflare.com` | untouched | Only used by an `rpki` measurement stage, which we never configure |

The `v4Reachability`, `v6Reachability`, `rpki` and `nxdomain` stages all fetch
external hosts, but they are *measurement types* — they run only if listed in
`measurements`. Our profiles never list them, and `assertLanOnly()` in
`frontend/src/api.ts` refuses to start if one appears.

## Measurement stage shapes

```ts
{ type: 'latency',    numPackets }
{ type: 'download',   bytes, count, bypassMinDuration? }
{ type: 'upload',     bytes, count, bypassMinDuration? }
{ type: 'packetLoss', numPackets, batchSize, batchWaitTime, responsesWaitTime, connectionTimeout? }
```

Stages run in array order. Once a single request of a direction exceeds
`bandwidthFinishRequestDuration` (default 1000 ms), further rounds of that
direction are skipped. **On a LAN almost nothing reaches 1000 ms, so every
listed stage runs** — which is why the profiles in `config/speedtest.toml` are
short lists of large transfers rather than the engine's long default ramp.

## AIM scoring

Ratings come from `results.getScores()` and are computed entirely in the
browser from the summary — no network call. Three experiences
(`streaming`, `gaming`, `rtc`), each scored from a subset of metrics and
bucketed into `bad | poor | average | good | great`. Thresholds live in
`src/config/internalConfig.ts`; we do not override them.

| Experience | Inputs |
|---|---|
| `streaming` | latency, packetLoss, download, loadedLatencyIncrease |
| `gaming` | latency, packetLoss, loadedLatencyIncrease |
| `rtc` | latency, jitter, packetLoss, loadedLatencyIncrease |

## Timing precision

`PerformanceResourceTiming` is coarsened for privacy — roughly 0.1 ms in
Chrome, 1 ms in Firefox. On a LAN with sub-millisecond round trips, absolute
latency and jitter are largely quantisation artefacts. Bandwidth and the
*loaded-minus-idle* latency delta stay meaningful. The UI says so when the
readings sit at the resolution floor rather than presenting them as precise.

---

See also: [Architecture](Architecture.md) ·
[HTTP API](HTTP-API.md) ·
[Configuration](Configuration.md) ·
[Testing](Testing.md)
