# Release history

Versions are three-part `X.Y.Z`. Releases below 1.0.0 were marked as GitHub
pre-releases; from 1.0.0 they are full releases. The 10 GbE saturation check
was waived, and "no external traffic" is proven by an end-to-end test on every
push rather than by a manual packet capture.

## 1.3.1

The latency figures become true.

- **`TCP_NODELAY` was never set on the TLS listener**, so small HTTPS responses
  stalled up to 40 ms on Nagle waiting for a delayed ACK. The engine's latency
  probe is exactly such a response, so the stall was reported as network
  latency. Idle, on the deployed guest: HTTPS 41.9 ms mean against plain HTTP
  0.6 ms; 0.7 ms after the fix.
- This was the "unexplained asymmetric bufferbloat" recorded since 1.0.0. It
  was neither asymmetric nor bufferbloat.
- Invisible on loopback, where the round trip is too short for an
  acknowledgement to be delayed — which is why every test tier passed
  throughout. The guard is now the return type of `net::tls_acceptor`.
- Latency stored by earlier versions is inflated by up to 40 ms.

## 1.3.0

Whose run it was, and how to get back to it.

- **Permalinks.** Every run stores its samples, and `/result.html?id=N`
  redraws it — the same traces, distributions and ratings — through the same
  renderer the live page uses. History rows link to it; a finished run links to
  itself.
- **Reverse DNS**, off by default and restricted to configured private ranges
  even when on. The lookup runs after a run is stored, never on the request
  path, and both hits and misses are cached. Enabled for the deployed guest via
  `guest.reverse_dns` in `provision.toml`.
- **Editable client names**, which beat a resolved hostname, which beats the
  address. The address stays visible either way.
- **Trusted-proxy `X-Forwarded-For`** via `server.trusted_proxies`, believed
  only from a peer named there.
- **The address is classified** — loopback, LAN, Tailscale or carrier NAT,
  link-local, public — with a note explaining what that means for the number
  shown. See [Client Identity](Client-Identity.md), which also records what
  cannot be recovered at all.

## 1.2.0

The measurement made legible.

- A step strip replaces the progress bar: one chevron per request the profile
  will issue, drawn before the run starts, with per-stage detail and results on
  hover.
- Hover detail on the traces — speed, payload, round trip and request duration
  for the individual sample under the pointer.
- A profile picker with `Auto`. The profile was fixed server-side, which is why
  every stored run said `lan-1g`.
- Traces are monotone cubic curves: smooth, but unable to overshoot between
  samples.
- The headline is one band across the top. Measured against
  speed.cloudflare.com at six viewport widths, which showed the reference caps
  at 1200px and centres rather than filling the window.

## 1.1.0

A name of its own, and a layout that survives being resized.

- The heading and tab title come from `server.site_name`
  (`SPEEDTEST_SITE_NAME`, or `guest.site_name` in `provision.toml`), so two
  installations on one LAN are tellable apart and renaming one is a restart
  rather than a rebuild.
- The page widens to 1320px, and the headline moves through three arrangements
  instead of one. The two bandwidth cards stay side by side down to 620px.
- Charts are drawn at their real pixel size rather than scaled to fit, which
  had been scaling their labels along with them — the same plot rendered 6px
  text in a narrow window and 17px on a wide monitor.

## 1.0.0

Running in production, with the interface reworked against the current
speed.cloudflare.com.

- Live bandwidth traces per direction with the reported percentile marked.
- Loaded latency and jitter shown per direction; packet loss as a received bar.
- Sample counts on every distribution, pause/resume, measured-at, client
  address.
- No server-location map: its tiles come from an external host, which this
  project must not contact. The client address replaces the useful half.

## 0.6.1

Nine defects found by the first live provisioning run against a real node — a
credential reaching an error message, an ACME hook aborting before the renewal
cron was installed, certificate permissions that would have failed at the next
renewal, and a coturn configuration that was never read. See the changelog.

## 0.6.0

The detail view and raw throughput.

- Box plots per transfer size and per latency phase: min, max, mean, median,
  p25/p75, whiskers to 1.5×IQR and outliers as dots. The headline figures are
  single percentiles and hide how consistent a run was, which is exactly what
  exposes a failing cable or a duplex mismatch.
- Bandwidth is grouped by transfer size, never pooled — a small transfer
  measures round-trip overhead more than throughput.
- Parallel-stream raw throughput, on demand, reported as a separate number with
  an explanation of why it is not comparable to the engine's figure.

## 0.5.0

Results history.

- Every completed run is stored in SQLite with its client, profile, full
  summary and ratings; the raw engine summary is kept verbatim so nothing is
  lost to a missing column.
- `/history` lists runs and charts the download/upload trend, filterable by
  client. Hand-drawn SVG rather than a charting library.
- Client attribution comes from the connection, never from a header — a
  spoofable header must not decide who a run belongs to.
- History is optional and degrades quietly when disabled, so the front end
  need not know whether this deployment keeps results.

## 0.4.0

Provisioning and TLS.

- Idempotent provisioner driving the node over SSH with `pct`/`pvesh`. Creates
  the guest, installs Docker and coturn, issues the certificate, deploys the
  application, and waits for it to be healthy.
- Guests are tagged on creation, and the tool refuses to modify any guest that
  does not carry that tag — checked in every mode, before any mutating command.
- MAC pinned and derived from the VMID, so a DHCP reservation survives a
  rebuild. `mac` prints it without changing anything, to be run first.
- The service terminates TLS itself; no reverse proxy, because a proxy hop
  would sit inside the measured path.
- Certificate renewal and its reload hook are verified after installation
  rather than trusted — the predecessor host is in exactly the failure state
  that catches.

## 0.3.0

Milestones 0 through 3 in one pass: the scaffold resolved, the backend, the
front end, and the packet-loss relay.

**Backend**

- Rust/Axum service implementing the `@cloudflare/speedtest` 1.13.1 contract:
  `/__down`, `/__up`, and the zero-byte latency ping.
- Download payloads are refcounted slices of one pre-allocated buffer — no
  per-request allocation, and an exact size hint so a real `content-length` is
  sent instead of chunked framing.
- Uploads are drained frame by frame and answered only once complete, because
  upload speed is derived from time-to-first-byte alone.
- `server-timing: cfRequestDuration;dur=N`, the only spelling the engine's
  parser accepts.
- Serves the built front end; `/api/status` exposes version and git SHA,
  `/api/profile` serves the engine configuration.

**Front end**

- TypeScript/Vite, no framework. Auto-starts on load, updates live, and shows
  download, upload, idle and loaded latency, jitter, packet loss and duration.
- AIM ratings for streaming, gaming and video calls, read from the engine's own
  scoring.
- Dark theme; version and git SHA in the footer.
- Refuses to start a run whose configuration could reach off-LAN, naming the
  specific problem.
- States plainly when latency readings sit at the browser's timing resolution
  rather than implying precision that does not exist.

**Packet loss**

- coturn configuration written from scratch, with an idempotent installer that
  refuses to leave placeholders unsubstituted.
- Relay denied to loopback, link-local and multicast peers.
- Containerised relay for the e2e suite, including an automated Trickle-ICE
  equivalent that fails if no `relay` candidate is gathered.

**Configuration**

- Named measurement profiles in one TOML file, selectable without a rebuild.
- `loaded_request_min_duration` and `loaded_latency_throttle` exposed and tuned
  per profile. The engine's defaults silently remove loaded latency and every
  quality rating on a LAN; see [Configuration](Configuration.md).
- Unknown keys are a hard error, so a typo cannot look like a working setting.

**Testing**

- Tier 1: 33 backend tests covering config parsing, the payload body, and the
  engine contract driven over a real socket.
- Tier 2: Playwright in Chromium and Firefox, including an assertion that no
  request leaves the origin for an entire run; a packet-loss suite against real
  coturn; a loopback throughput floor; and a CI job that boots the built image
  and probes it.

**Known limitations**

- Browser-reported download speed is bounded by the engine's single-stream,
  `r.text()`-decoding design rather than by the link. A separate parallel-stream
  harness is planned for 0.6.0 and will be reported as a distinct number.
- Guest provisioning and TLS automation are not yet implemented (0.4.0).
- Results are not persisted (0.5.0).
