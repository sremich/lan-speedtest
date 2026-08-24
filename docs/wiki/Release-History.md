# Release history

Versions are three-part `X.Y.Z`. While the major version is 0, every release is
a GitHub pre-release — tier-4 validation (10 GbE saturation and a packet
capture proving no external traffic) gates 1.0.0.

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
