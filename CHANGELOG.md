# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions are always
three-part X.Y.Z (bugfix +0.0.1, minor +0.1.0, major +1.0.0). On release,
move the Unreleased entries into a new version section, bump `VERSION`,
commit, then tag.

## [1.5.1] - 2026-08-26

### Fixed

- **`/metrics` returned the app shell instead of a 404 when metrics were
  turned off.** The route was left unmounted rather than mounted and
  refusing — which looks equivalent and is not, because the SPA fallback
  answers anything unrouted. On a deployment that actually serves the front
  end, a scrape received `200 text/html` and an HTML page, and would have
  reported a permanently healthy target with no series in it. It is now always
  routed and returns 404 from the handler when disabled.

  The test that was meant to cover this passed for the wrong reason: its
  config pointed `static_dir` at a directory that did not exist, so the
  fallback failed and produced the 404 it was hoping for. It now writes a real
  static directory, asserts the fallback is genuinely answering, and only then
  asserts that `/metrics` is refused.

## [1.5.0] - 2026-08-26

What measured it, whether to measure at all, and what changed.

### Added

- **Auto-start can be turned off.** A run moves hundreds of megabytes, and
  opening the page to read the history or change a setting should not do that
  behind your back — least of all during the video call that made you suspect
  the network. On by default, so nothing changes unless you ask: a per-browser
  toggle beside Retest, `?autostart=0` for one visit, and `server.autostart`
  (or `SPEEDTEST_AUTOSTART`) for the deployment's default. The URL override is
  deliberately not remembered — a link someone sends you should not silently
  reconfigure your browser.

- **A compare page**, `/compare.html?a=N&b=M`. Pick two runs in the history and
  it computes the difference rather than leaving you to eye two columns of
  formatted numbers. Signed by *improvement* rather than by arithmetic: more
  bandwidth and less latency are both good, and a table that coloured them the
  same way would be worse than one with no colour at all. A change under 2% is
  reported as no change, because two runs of the same link differ by that much
  every time.

  If either run predates 1.3.1, the latency rows are marked and explained —
  their difference is mostly our own `TCP_NODELAY` fix, not the network.

- **`/metrics`**, in Prometheus text format, off unless `server.metrics` is
  set: the body names every client that has run a test, which is more than an
  unauthenticated endpoint should offer by default. Exports the most recent run
  per client in base units (seconds, ratios), and **omits** a figure that was
  never measured rather than exporting it as zero — no packet-loss stage and 0%
  packet loss are different claims.

- **Retention**, `retain_runs_days` and `retain_samples_days`, both off by
  default. Two windows because the costs differ by orders of magnitude: a
  summary row is a couple of hundred bytes and is what a trend is made of; the
  sample blob behind it can be a quarter of a megabyte and stops being
  interesting long before the run does. Samples can therefore be released while
  the run is kept.

- **A `lan-2.5g` profile.** The widest gap in the shipped set: 2.5 GbE is now
  the default uplink on Wi-Fi 7 access points and current motherboards, and a
  2.5G client running `lan-1g` finishes each transfer too fast for loaded
  latency to be measurable.

- **A light theme**, with a toggle on every page. Three states, not two: an
  explicit choice is remembered and wins, and no choice follows the operating
  system — including while the page is open, so a laptop that switches at
  sunset takes the page with it.

- **A web manifest and icon**, so the page can be added to a phone's home
  screen — which is where it is most useful, walking around looking for the
  dead spot.

- **Every run records the build that measured it.** A stored figure is only
  interpretable if you know what produced it: everything recorded before 1.3.1
  carries up to 40 ms of our own `TCP_NODELAY` stall in its latency, and until
  now nothing distinguished those rows from correct ones. New runs carry their
  version; a run whose version is missing or below 1.3.1 is marked in the
  history table and on its own page, with what the caveat actually is.

  The recorded number is never adjusted — it is what was measured. Only latency
  is flagged, because the bug delayed small responses and never touched bulk
  transfers, which is why it survived three releases with throughput looking
  right.

### Changed

- **Changing the profile clears the result instead of starting a new run.** It
  re-ran immediately so that yesterday's figures never sat under today's
  profile name — a real concern, and clearing solves it too. On `lan-10g` a
  brushed dropdown was several gigabytes down the wire before you could react.

- **Latency is shown to two decimals.** One was enough when a LAN round trip
  read tens of milliseconds; since 1.3.1 it reads about 0.6 ms, where a tenth
  is a sixth of the figure. The detail-view distributions already used two, so
  the headline was the coarser number sitting above the finer ones.

### Fixed

- **`x-forwarded-for` was read from the first header line only.** The header is
  legally repeatable and proxies differ — some extend one comma-joined list,
  others append a line per hop. Since the client is found by walking the chain
  from the right past the trusted proxies, dropping the tail meant believing
  the wrong end of it. All lines are now joined before parsing.

- **The history index did not match the query's sort order.** It covered
  `recorded_at` while `recent` orders by `recorded_at DESC, id DESC`, leaving
  SQLite to sort the ties in memory.

## [1.4.0] - 2026-08-25

The run, described and explained.

### Added

- **A live stage line above the step strip**, with a spinner, the stage's name
  in that stage's own colour, and the payload it is moving — "Measuring
  download · 100 MB". The strip says what the run is *made of*; this says what
  it is doing right now. The spinner is tied to the engine's own running state,
  so pausing stops it rather than leaving it pretending to work, and a finished
  run hides it rather than freezing it.

- **A description per run.** A free-text note stored against the run — where
  you were, on what device, what you were testing — editable from the history
  table and from the run's own page. Per-run rather than per-client on purpose:
  it is exactly the thing that differs between two runs from the same machine.
  Capped at 280 characters, counted in characters so the cap does not depend on
  the alphabet and truncation cannot split a character in half.

### Changed

- **The distribution tooltip is written in words.** It opens with a sentence
  explaining what the marks actually mean, then labels every figure — "25th
  percentile", not "p25". It is a styled tooltip rather than a native `title`,
  which could not be styled, could not hold the explanation, and only appeared
  after a delay. The row under the pointer is highlighted so the tooltip is
  anchored to something. The legend now says "Average" to match.

- **A distribution row is hovered anywhere along its band.** Previously the
  pointer had to land on a mark, and on a LAN the values cluster so tightly
  that the box is a few pixels wide with the average dot sitting on top of it.
  The band is now the target and the marks are pointer-transparent.

- **The step strip spreads across the width.** It was fixed-width chevrons
  packed at the left, so on a wide monitor it stopped well short. Chevrons and
  the gaps between them now grow together, bounded by how many chevrons there
  are so a short profile spreads sensibly rather than scattering a dozen of
  them across 1600px. Below a minimum width it wraps to a second row instead of
  compressing — on a phone a strip that crosses twice reads far better than
  forty-seven chevrons squeezed into 390px.

### Fixed

- **The "90th percentile" label sat at different heights on the two traces.**
  Its position was clamped to a minimum while the line it labels was not, so
  wherever the clamp bit, the label drifted off its own line — which is why
  download and upload disagreed. The label is now pinned to the line and the
  gap is a fixed CSS offset, identical on every chart. Where there is no room
  above, it flips below by the same amount rather than sliding.

## [1.3.1] - 2026-08-25

The latency figures become true.

### Fixed

- **The TLS listener never set `TCP_NODELAY`, inflating every latency figure
  by up to 40 ms.** `axum_server::bind_rustls` wraps `DefaultAcceptor`, which
  hands the accepted socket on untouched, so Nagle's algorithm withheld the
  second half of a small response until the peer's delayed-ACK timer fired —
  `TCP_DELACK_MIN`, which is exactly 40 ms on Linux.

  The engine's latency probe is `GET /__down?bytes=0`. Its response is precisely
  the small, split write that trips this, so the stall was reported to the user
  as network latency. Measured on the deployed guest, 25 idle probes each:

  | endpoint                    | before   | after  |
  |-----------------------------|----------|--------|
  | `http://…:8080/api/health`  |  0.6 ms  | 0.6 ms |
  | `https://…/api/health`      | 41.9 ms  | 0.7 ms |
  | `https://…/__down?bytes=0`  | 38.4 ms  | 0.7 ms |

  Plain HTTP was unaffected only by luck — hyper emitted those responses as a
  single write. Both listeners are pinned now.

  This is what the "unexplained asymmetric bufferbloat" in the notes actually
  was. It is invisible on loopback, where the round trip is too short for the
  acknowledgement to be delayed, which is why every test tier passed
  throughout. Stored runs from before this fix carry latency figures inflated
  by the same amount.

## [1.3.0] - 2026-08-25

Whose run it was, and how to get back to it.

### Added

- **Permalinks to a stored run.** The history page could start a new test but
  not take you back to a result, so every run you navigated away from was gone.
  Every sample is now stored with the run, and `/result.html?id=N` *redraws*
  it — the same traces, the same distributions, the same ratings — rather than
  summarising it. History rows link to it, and a finished run offers a link to
  itself.

  The rendering is shared with the live page rather than reimplemented: a
  stored run should look identical to the run you watched, and it only stays
  identical if there is one implementation of it.

- **Client hostnames, from reverse DNS.** Off by default, and restricted to
  configured address ranges even when on: unrestricted, a deployment reachable
  from the internet would send a PTR query for every visiting address to its
  upstream resolver — a quiet outbound leak for a cosmetic label. The lookup
  runs after a run is stored, never on the request path, and both hits and
  misses are cached.

  The DNS client is hand-rolled rather than a resolver dependency: one query
  type, one record type, over UDP. Compression pointers are followed with a
  jump cap and every offset is bounds-checked, because a PTR response is the
  one input here that is not under local control. Returned names are
  length-capped and character-restricted before being stored.

- **Editable client names.** A typed name beats a resolved hostname, which
  beats the address. The address stays visible either way — a label that
  replaced it would make the history impossible to correlate with a DHCP lease
  table or a switch port. Names are per-client, not per-run.

- **Trusted-proxy `X-Forwarded-For`.** Behind a reverse proxy every run was
  attributed to the proxy. Naming the proxy in `server.trusted_proxies` makes
  the header believable *from that peer only*; the header from anyone else is
  still ignored, because otherwise any client on the LAN could file a run under
  any address it liked.

- **The address is classified and labelled.** loopback, LAN,
  `Tailscale or carrier NAT`, link-local, or public — with a note on hover
  explaining what that means for the number shown. See
  [Client Identity](docs/wiki/Client-Identity.md), which also records the
  honest negative results: an address translated by a Tailscale subnet router
  cannot be recovered at all, and a browser will not disclose its own private
  address to a page.

### Changed

- The stored-result size cap is larger, because a submission now carries every
  sample rather than only the summary. Samples over their own cap are dropped
  and the run kept: the measurement succeeded, and losing it over the size of
  its detail would not be an improvement.

- Provisioning gained `guest.reverse_dns` and `guest.dns_resolver`, written
  into the guest's `.env`, so switching hostname lookups on is a re-run of
  `apply` rather than a rebuild. It is on in the shipped `provision.toml`.

- The trace hover, the box plots and the packet-loss bar moved into
  `frontend/src/runview.ts` and are shared with the permalink rather than
  reimplemented for it.

## [1.2.0] - 2026-08-25

The measurement made legible.

### Added

- **A step strip instead of a progress bar.** One chevron per request the
  profile will issue, grouped and coloured by stage, drawn *before* the run
  starts — so the shape of the work is visible up front rather than only in
  hindsight. Hovering gives the stage's payload size, request count, position
  in the profile, and what it measured.

  The model was checked against the reference rather than guessed: the engine's
  own default profile has 25 stages whose fourth entry is
  `{ type: 'download', bytes: 1e5, count: 9 }`, which is exactly the
  "Payload: 100 kB, Requests: 9, Step: 4 of 25" speed.cloudflare.com shows on
  hover.

  A warm-up round is labelled as one. It is exempt from
  `loaded_request_min_duration` and its samples are excluded from the bandwidth
  figures, so it reports no measurement — which looked like a failure until it
  said why.

- **Hover detail on the traces.** A guide line, a dot on the curve, and what
  that individual request was: speed, payload size, round trip, and how long it
  took. The engine has recorded all of this per sample since the beginning and
  none of it was reachable.

- **A profile picker, with `Auto`.** The profile was fixed server-side, which
  is why every stored run said `lan-1g`. It is now a choice, remembered per
  browser and still recorded with each run. `Auto` measures the link with one
  short transfer and picks the largest `auto_selectable` profile within a
  factor of two of what it measured — a single stream reaches only part of a
  fast link, so demanding the full nominal rate would always choose the smaller
  profile.

  New `nominal_bps` and `auto_selectable` profile keys. `nominal_bps` also
  replaces the hard-coded table in the profile-sizing test, which had been free
  to drift away from the profiles it was checking.

- `GET /api/profiles`, and `GET /api/profile?name=`. An unknown name is refused
  rather than quietly served the default, which would let a stale choice
  measure something other than what it says.

### Changed

- **Traces are drawn as monotone cubic curves.** Smooth, but constrained to
  stay within the range of each pair of samples. An ordinary cardinal or
  Catmull-Rom spline overshoots, so a link ramping from nothing to 900 Mbps
  would be drawn dipping below zero on the way up and peaking above the fastest
  sample ever recorded — smooth, and false.

- **The headline is one band across the top**, divided by hairlines, rather
  than three separate cards. This follows the reference's arrangement, checked
  by measuring speed.cloudflare.com at six viewport widths rather than from
  screenshots.

  Worth recording, because it inverts the obvious assumption: **the reference
  never uses the full window.** It caps at 1200px and centres, at every width
  above that — at 1920 it leaves 360px of margin each side. This page now caps
  at 1440, slightly wider than the reference, and steps 3 columns → 2 (at
  1200px) → 1 (at 768px).

- The 90th-percentile label sits on a chip, because dim text over a filled
  gradient is unreadable.

### Fixed

- The profile picker's widest option set the width of the whole page, pushing
  it past a phone's viewport by 23px. Caught by the scaling test added in
  1.1.0, which now also names the offending element rather than only reporting
  how far over it went.

## [1.1.0] - 2026-08-25

A name of its own, and a layout that survives being resized.

### Added

- **The deployment names itself.** The page heading and the browser tab title
  come from `server.site_name`, overridable with `SPEEDTEST_SITE_NAME` and
  settable from `provision.toml` as `guest.site_name`. Two of these on one LAN
  were otherwise indistinguishable in a tab strip, and the name was previously
  hard-coded into the bundle — so renaming an installation meant a rebuild.

  Provisioning rejects a name containing a newline or a `#`. The value lands in
  a compose `env_file`, which has no quoting: a newline would inject an
  unrelated variable and a `#` can start a comment, so the guest would come up
  named something nobody asked for.

  A blank name falls back to the default rather than refusing to boot.
  Everything else in the config file fails loudly on a bad value; this one is
  cosmetic enough that an empty heading is the worse outcome.

### Changed

- **The layout scales with the window instead of sitting in a fixed column.**
  The page now widens to 1320px, and the headline moves through three
  arrangements rather than one: two traces beside a vitals column on a desktop,
  two traces above a row of vitals on a laptop, and a single stack on a phone.
  The download and upload cards stay side by side down to 620px, which is where
  the comparison between them stops being readable anyway. The live traces
  take their height from the viewport, so a tall window shows more of the shape
  of a run.

- **Charts are drawn at their real pixel size.** The box plots and the history
  trend previously used a fixed drawing scaled to fit, which scaled their text
  and row heights along with it — the same plot rendered 6px labels in a narrow
  window and 17px ones on a wide monitor. They are now measured and drawn to
  the container's actual width, and redraw on resize. The 90th-percentile
  marker label moved out of the stretched trace SVG into HTML for the same
  reason: text inside a `preserveAspectRatio="none"` viewBox is squashed by
  whatever the current aspect ratio happens to be.

  An end-to-end test now walks four viewports from 390px to 1920px and asserts
  that nothing overflows horizontally, that the drawings are never stretched,
  and that a label is the same size at every one of them.

## [1.0.0] - 2026-08-25

The first full release: deployed, serving, and no longer a pre-release.

### Added

- **Live bandwidth traces.** Each direction is drawn as a filled area over its
  samples as they arrive, with the reported 90th percentile marked. A headline
  percentile says what the link did on average; the shape says whether it
  climbed and held or spiked and collapsed, and only the second is a problem.

- **Per-direction loaded latency and jitter**, shown inline with ↓/↑ markers
  rather than as separate cards, plus packet loss as a received bar, sample
  counts on every distribution label, a pause/resume control, a measured-at
  timestamp, and the client's own address in the footer.

  The sample counts matter more than they look: a distribution drawn from two
  samples deserves less trust than one drawn from twenty, and the reader had no
  way to tell which they were looking at.

### Changed

- The results page follows the current speed.cloudflare.com layout.

### Deployed

- Running on the deployment guest with an auto-renewing wildcard certificate, coturn for
  the packet-loss stage, and history enabled. Packet loss confirmed at 0% from
  a browser on the LAN.

### Not included

- **No server-location map.** The reference site shows one; map tiles come from
  an external host, which is the single thing this project must not do, and the
  end-to-end origin test would fail on it — correctly. On a LAN the useful half
  of that panel is knowing which machine you are testing from, so the client
  address is shown instead.

- The 10 GbE saturation check that originally gated 1.0.0 is waived at
  the maintainer's request. The criterion behind it had already been restated as "the
  backend is provably not the bottleneck", which CI covers at 34-51 Gbps over
  loopback. The other tier-4 item is not waived: "no external traffic" is
  proven on every push by a test that drives a full run in a real browser and
  fails if one request leaves the origin.

## [0.6.1] - 2026-08-25

Nine fixes from the first live provisioning run, none of which the mocked suite
could have caught. Mocks assert on the commands issued; they cannot observe
file ownership, uid/gid arithmetic, what a daemon does when it cannot write its
log, or how a tool encodes its own config.

### Security

- **A failed `pct exec` echoed its command verbatim**, including a leading
  `TURN_PASS` assignment. Credentials are now redacted at that boundary rather
  than relying on each call site to be careful. The first attempt was defeated
  by shell-escaped quotes, so the scanner understands quote continuation.

### Fixed

- **The ACME reload hook aborted provisioning before the renewal cron was
  installed**, leaving a valid certificate with nothing to renew it — precisely
  the state the predecessor host is in.
- **Certificate permissions.** Written `root:root 0600` while the service runs
  unprivileged, and rewritten by acme.sh on every renewal, so fixing it once
  would have failed again about ninety days later. Applied at install and
  re-applied by the hook.
- **The image's gid did not match its uid**, because `useradd --uid` lets the
  distribution pick any free gid — defeating group-readable mounts.
- **A bind mount shadowed the image's data directory** and Docker created the
  host side as root, crash-looping the unprivileged service.
- **coturn logged nowhere at all**: it drops to a user that cannot write under
  `/var/log`, and `no-stdout-log` suppressed the fallback.
- **coturn ran on defaults for hours** because its config was `root:root 0600`
  and coturn drops privileges before reading it. It now fails rather than
  degrading — a daemon that quietly falls back to defaults is worse than one
  that exits.
- Three diagnostics that lied: a false "reload hook not configured" warning
  from grepping base64, a placeholder check tripping on its own explanatory
  comment, and CRLF line endings turning `set -euo pipefail` into an invalid
  option with a message naming neither the file nor line endings.
- **Box-plot whiskers could be drawn inside their own box.** With few samples
  and an extreme outlier the quartile can lie beyond the last non-outlier
  sample. Whiskers are clamped to the box, so the invariant holds by
  construction rather than by tolerance.

### Changed

- `provision.toml` is gitignored with a committed example. It names the
  hypervisor and the guest address, which is internal topology this repository
  should not carry.

## [0.6.0] - 2026-08-25

The detail view, and raw throughput as a separate number.

### Added

- **Per-measurement distributions.** A collapsible detail section shows a box
  plot for every download and upload transfer size and for each latency phase:
  minimum, maximum, mean, median, 25th and 75th percentiles, whiskers to the
  furthest sample within 1.5×IQR, and a dot per outlier. Hovering a row gives
  the full five-number summary.

  The headline figures are single percentiles, which say nothing about how
  consistent a run was — and a link that averages 940 Mbps by alternating
  between 500 and 1300 is a very different network from one that holds 940
  throughout. That second case is what a failing cable or a duplex mismatch
  looks like, and it was previously invisible.

  Bandwidth is grouped by transfer size rather than pooled: a small transfer
  measures round-trip overhead more than throughput, so mixing sizes would make
  a healthy link look wildly inconsistent.

- **Parallel-stream raw throughput**, on demand and reported as a distinct
  number. The engine issues one request at a time and decodes every response
  through `r.text()`, so its download figure is bounded by single-stream TCP
  plus main-thread decoding rather than by the link. This pulls several streams
  concurrently and discards bytes through the streaming reader, so nothing
  materialises the payload. The UI explains that the two measure different
  things, because otherwise someone will compare them.

- Percentiles use the type-7 (linear interpolation) definition, matching NumPy
  and Excel, so a figure shown here is what someone gets checking by hand.

### Tests

- Vitest for the front end: 24 unit tests over the percentile and summary
  maths, including Tukey fences, zero-IQR sets, single samples, and the
  guarantee that whiskers stay inside min/max and never invent a value at the
  fence.
- Playwright: the detail view renders a distribution per measurement with a
  legend and per-row tooltips; box geometry is checked numerically (median
  inside its box, whiskers spanning it); the raw harness produces a labelled
  figure and stays on-origin.

### Fixed

- A zero-spread distribution drew its minimum-width box anchored at p25, which
  put it beside the whisker rather than on it and read as a real offset. Now
  centred on the median.

## [0.5.0] - 2026-08-24

Results history. Runs no longer vanish when the tab closes.

### Added

- **SQLite-backed history.** Every completed run is POSTed back and stored with
  its timestamp, client address, user agent, profile, full summary and quality
  ratings. The raw engine summary is kept verbatim alongside the extracted
  columns, so a metric without a column of its own is not lost.
- **`/history` page**: a table of stored runs and a hand-drawn SVG trend chart
  of download and upload, with a per-client filter. No charting library — the
  front end's only dependency stays the measurement engine itself.
- `GET /api/history` (with `?client=mine|all|<ip>` and `?limit=`),
  `GET /api/clients`, `POST /api/results`.
- `client_names` table and lookup, so the `[later]` friendly-naming feature
  needs no migration when it arrives.
- `historyEnabled` on `/api/status`; the front end hides the history link when
  the deployment does not keep results.

### Notable behaviour

- **Attribution comes from the connection, never a header.** Trusting
  `X-Forwarded-For` would let any client file a run under any address, and
  there is no proxy in front of this service. Asserted by a test that sends a
  spoofed header and checks it is ignored.
- Storing a result is best-effort: a storage failure never presents as a failed
  speed test, because the measurement itself succeeded.
- History is optional. With no database path configured the endpoints degrade
  quietly — `POST /api/results` returns 202 rather than an error — so the front
  end does not need to know whether this deployment keeps results.
- A run with no measurements at all is refused rather than stored; a partial
  run is kept, because it is still a real observation.

### Fixed

- Chart Y-axis labels were clipped by a too-narrow gutter, rendering
  "250 Mbps" as "50 Mbps" — the chart read an order of magnitude slow, with no
  error anywhere. Now uses a compact tick format and a wider gutter, with a
  test that measures each label's bounding box against the SVG edge.

### Tests

- 8 history integration tests over a real socket, including the milestone's
  done-when (three runs, two clients, correct attribution) and the
  header-spoofing guard.
- 5 Playwright tests: the run is stored, the page lists and charts it, the
  client filter narrows it, nothing leaves the origin, and axis labels are not
  clipped.

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

[Unreleased]: https://github.com/sremich/self-hosted-cloudflare-speedtest/compare/v1.5.1...HEAD
[1.5.1]: https://github.com/sremich/self-hosted-cloudflare-speedtest/releases/tag/v1.5.1
[1.5.0]: https://github.com/sremich/self-hosted-cloudflare-speedtest/releases/tag/v1.5.0
[1.4.0]: https://github.com/sremich/self-hosted-cloudflare-speedtest/releases/tag/v1.4.0
[1.3.1]: https://github.com/sremich/self-hosted-cloudflare-speedtest/releases/tag/v1.3.1
[1.3.0]: https://github.com/sremich/self-hosted-cloudflare-speedtest/releases/tag/v1.3.0
[1.2.0]: https://github.com/sremich/self-hosted-cloudflare-speedtest/releases/tag/v1.2.0
[1.1.0]: https://github.com/sremich/self-hosted-cloudflare-speedtest/releases/tag/v1.1.0
[1.0.0]: https://github.com/sremich/self-hosted-cloudflare-speedtest/releases/tag/v1.0.0
[0.6.1]: https://github.com/sremich/self-hosted-cloudflare-speedtest/releases/tag/v0.6.1
[0.6.0]: https://github.com/sremich/self-hosted-cloudflare-speedtest/releases/tag/v0.6.0
[0.5.0]: https://github.com/sremich/self-hosted-cloudflare-speedtest/releases/tag/v0.5.0
[0.4.0]: https://github.com/sremich/self-hosted-cloudflare-speedtest/releases/tag/v0.4.0
[0.3.0]: https://github.com/sremich/self-hosted-cloudflare-speedtest/releases/tag/v0.3.0
