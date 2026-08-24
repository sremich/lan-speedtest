# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions are always
three-part X.Y.Z (bugfix +0.0.1, minor +0.1.0, major +1.0.0). On release,
move the Unreleased entries into a new version section, bump `VERSION`,
commit, then tag.

## [Unreleased]

## [0.4.0] - 2026-08-24

Provisioning and TLS. One command takes a Proxmox node from nothing to a
serving guest; running it again changes nothing.

### Added

- **Idempotent provisioner** (`speedtest-provision`), driving the node with
  `pct`/`pvesh` over SSH. `pct exec` reaches the guest through the hypervisor,
  so the first run works before the guest has a DHCP lease — which removes the
  chicken-and-egg with the address reservation.
- **Ownership guard.** Every guest created is tagged, and the tool refuses to
  modify any guest that does not carry that tag, in every mode including
  `plan`. Matching is on an exact tag, so a lookalike does not grant ownership.
- **Pinned MAC address**, derived from the VMID and stable across rebuilds, so
  a DHCP reservation survives a rebuild. `speedtest-provision mac` prints it
  and changes nothing, to be run before the reservation is made.
- Docker installed from upstream (Debian's package has no compose plugin) and
  proved to work rather than assumed; skipped entirely when already present.
- coturn installed and configured from the committed template.
- **TLS terminated by the service itself**, with no reverse proxy — a proxy hop
  would sit inside the path being measured. Plain HTTP keeps serving the health
  check. The binary carries `cap_net_bind_service` so it binds 443 as a
  non-root user.
- ACME DNS-01 issuance with a weekly renewal and a reload hook that restarts
  the service, both **verified after the fact** rather than trusted.
- `plan`, `apply`, `verify` and `mac` subcommands.

### Tests

- 15 provisioning integration tests plus 23 unit tests, all against a scripted
  fake hypervisor: the ownership guard, a second run creating nothing, Docker
  skipped when present, and the renewal checks.
- Hostile values (quotes, `$(...)`, backticks, semicolons) are round-tripped
  through a real shell to prove a credential cannot break out of a command.
- 3 TLS tests: HTTPS serves, the measurement contract holds over TLS, and a
  missing certificate is reported clearly.
- `shellcheck` over the coturn installer in CI.

### Changed

- `docker-compose.yml` mounts the certificate directory and grants
  `NET_BIND_SERVICE`.
- Half-configured TLS (one of cert/key) is now a startup error rather than a
  silent fallback to plain HTTP.

### Known limitations

- Tier-3 live validation against the real node has not been run yet; the flow
  is covered only by mocked tests so far.

## [0.3.0] - 2026-08-24

Milestones 0 through 3 delivered in one pass: scaffold resolved, backend, front
end, and packet-loss relay. First release; the version reflects three +0.1.0
milestone increments.

### Added

**Backend (Rust/Axum)**

- `/__down` and `/__up` implementing the `@cloudflare/speedtest` 1.13.1
  contract, including the zero-byte latency ping (the engine has no separate
  ping endpoint).
- Zero-copy download payloads: refcounted slices of one pre-allocated buffer,
  with an exact size hint so a real `content-length` is sent rather than
  chunked framing.
- Upload bodies drained frame by frame and answered only once complete, since
  upload speed is derived from time-to-first-byte alone.
- `server-timing: cfRequestDuration;dur=N` — the only spelling the engine's
  parser accepts.
- `no-store` on measurement responses, so repeat requests are not served from
  the browser cache.
- Static serving of the built front end; `/api/status` (version, git SHA,
  profile), `/api/profile` (engine configuration), `/api/health`.
- Per-request transfer cap, validated against the active profile at startup.

**Front end (TypeScript/Vite)**

- Drives the engine directly: auto-start on load, live updates, and a final
  summary of download, upload, idle and loaded latency, jitter, packet loss and
  duration.
- AIM suitability ratings for streaming, gaming and video calls.
- Dark theme; version and git SHA in the footer.
- Pre-flight check that refuses to start a run whose configuration could reach
  off-LAN, naming the specific problem.
- Explicit note when latency readings sit at the browser's timing resolution,
  rather than implying precision that does not exist.
- Explanatory message when the engine returns no ratings, instead of an empty
  panel.

**Packet loss / TURN**

- coturn configuration written from scratch, with peers denied to loopback,
  link-local and multicast ranges.
- Idempotent installer that renders the template from environment values,
  refuses to proceed with unsubstituted placeholders, restarts only on change,
  and verifies the listener.
- Containerised relay for tier-2 testing.

**Configuration**

- Named measurement profiles (`lan-1g`, `lan-10g`, `quick`, `e2e-packetloss`)
  in one TOML file, selectable by environment without a rebuild.
- `loaded_request_min_duration` and `loaded_latency_throttle` exposed per
  profile and tuned for LAN speeds — the engine's defaults silently suppress
  loaded latency and every quality rating on a fast link.
- Unknown configuration keys rejected outright.

**Testing and CI**

- 33 backend tests: config parsing, payload body, and the engine contract
  driven over a real socket.
- Playwright suite in Chromium and Firefox, including a full-run assertion that
  no request leaves the origin.
- Packet-loss suite against real coturn, including an automated Trickle-ICE
  equivalent that fails when no `relay` candidate is gathered.
- Loopback throughput floor, best-of-three, printing the achieved figure.
- CI jobs for tier 1, tier 2, packet loss, and a built-image smoke test that
  checks the baked-in version, the served front end and the `server-timing`
  header.
- Release workflow wired to the real toolchain.

**Documentation**

- Wiki under `docs/wiki/`, including the engine contract as verified from the
  package's own sources.
- README, deployment and troubleshooting notes.

### Known limitations

- Browser-reported download speed is bounded by the engine's single-stream,
  `r.text()`-decoding design rather than by the link. A parallel-stream harness
  is planned for 0.6.0 and will be reported separately.
- Quality ratings require a browser whose timing resolution is finer than the
  path's round trip. Firefox coarsens to ~1 ms, so on a sub-millisecond LAN its
  latency readings round to zero and the engine returns no scores; the page
  explains this instead of showing an empty panel. Chrome (~0.1 ms) is
  unaffected, as are bandwidth, jitter and packet loss.
- Guest provisioning and TLS automation are not yet implemented (0.4.0).
- Results are not persisted (0.5.0).
- Tier-4 hardware validation outstanding; releases stay pre-release until it
  passes.

[Unreleased]: https://github.com/sremich/self-hosted-cloudflare-speedtest/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/sremich/self-hosted-cloudflare-speedtest/releases/tag/v0.4.0
[0.3.0]: https://github.com/sremich/self-hosted-cloudflare-speedtest/releases/tag/v0.3.0
