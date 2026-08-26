# Troubleshooting

Most failures here are silent — the engine keeps working and shows a wrong or
missing number. This page is organised by symptom.

## No quality ratings, everything else fine

The ratings section shows the "no rating available" note, or is empty.

Every AIM experience takes `loadedLatencyIncrease` as an input, and the engine
computes that only when loaded latency is a **truthy** number. If no transfer
size qualified as loading the connection, loaded latency stays `0`, which is
falsy, and the engine returns no scores whatsoever.

There are two causes, and the loaded-latency cells tell them apart.

**If loaded latency shows a real number**, the threshold is wrong.
`loaded_request_min_duration` is sitting above the duration of the profile's
transfers, so no size qualified. The engine's default is 250 ms; a 250 MB
download at 10 Gbps takes 200 ms. Lower the threshold or enlarge the
transfers. See [Configuration](Configuration.md).

**If loaded latency shows `<0.01 ms`**, the reading is genuinely zero and there
is nothing to fix. This is most likely in **Firefox**, which coarsens resource
timing to ~1 ms (Chrome coarsens to ~0.1 ms). On a LAN with a sub-millisecond
round trip, every Firefox latency reading rounds to exactly 0 — and 0 is
falsy, so the engine treats loaded latency as unavailable and returns no
scores.

This cannot be fixed from our side: the engine reads latency only from
`PerformanceResourceTiming`, and forking it is a non-goal. Use Chrome when you
want the quality ratings on a fast LAN. Bandwidth, jitter and packet loss are
unaffected.

## Latency is tens of milliseconds on a LAN, and bimodal

**Symptom.** Idle latency is mostly sub-millisecond but some samples land near
40 ms; loaded-download latency sits in a tight cluster around 25-30 ms while
loaded-upload is much lower. It reads like asymmetric bufferbloat.

**Cause, if you are running anything before 1.3.1.** The TLS listener did not
set `TCP_NODELAY`. Nagle's algorithm withholds the second part of a small
write until the first is acknowledged, and the peer's delayed-ACK timer holds
that acknowledgement for up to 40 ms (`TCP_DELACK_MIN` on Linux). The engine's
latency probe is `GET /__down?bytes=0` — exactly that kind of small response —
so the stall was reported as network latency.

**Confirming it is this and not your network.** Probe a *different* host on the
same LAN at the same moment. Real congestion delays both; this delays only the
speed test:

```bash
curl -sk -o /dev/null -w 'ttfb=%{time_starttransfer} tls=%{time_appconnect}
' https://speedtest.example/api/health
curl -sk -o /dev/null -w 'ttfb=%{time_starttransfer} tls=%{time_appconnect}
' https://other-host.example/
```

Subtract `tls` from `ttfb`. If the speed test is tens of milliseconds and the
other host is not, it is this. Comparing plain HTTP against HTTPS on the *same*
server is the other tell — plain was unaffected.

**Note this is invisible on `127.0.0.1`**, where the round trip is too short
for an acknowledgement to be delayed. Reproducing it needs a real link.

**Fix.** Upgrade to 1.3.1 or later. Latency figures stored before it are
inflated by up to 40 ms and cannot be corrected retrospectively — which is why
a comparison involving one of those runs greys its latency rows rather than
reporting the fix as a network improvement. See
[History and Metrics](History-and-Metrics.md#when-latency-cannot-be-compared).

## Latency looks quantised, or reads 0

Expected on a LAN. Browsers coarsen resource timing for privacy — roughly
0.1 ms in Chrome, 1 ms in Firefox — so a sub-millisecond round trip is below
what can be measured in a browser at all. The page says so automatically when
readings sit at the floor.

Read the loaded-minus-idle delta rather than the absolute numbers. That
difference is real even when the absolute values are not.

## Latency is suspiciously uniform across runs

Check that `server-timing` is being emitted and spelled correctly:

```sh
curl -sD - -o /dev/null 'http://HOST:8080/__down?bytes=1024' | grep -i server-timing
```

Expect `server-timing: cfRequestDuration;dur=0.012`. The engine accepts only
`cfRequestDuration` (or `cfRequestDur` / `cfReqDur`), or a sum of `cfSpeed*`
entries. A plain `dur=` is ignored and latency falls back to the profile's
fixed `estimated_server_time`.

Values at or below 0.01 ms are also treated as absent, which is normal and
harmless for this backend — it simply means the `0.0` fallback applies.

## Upload speed looks impossibly high

The response is being sent before the request body has been drained. Upload
speed is derived from time-to-first-byte alone, so an early answer makes every
upload look instantaneous. The backend drains first by construction and a test
asserts it; if this appears, something in front of the backend — a proxy, a
load balancer — is answering early.

## Download speed is far below the link rate

Expected, and usually not a fault. The engine issues **one request at a time**
and reads every response through `r.text()`, decoding the whole payload into a
JavaScript string. Browser-reported speed is bounded by single-stream TCP plus
main-thread decoding.

To find out whether the *backend* is the limit, take the browser out of it:

```sh
curl -s -o /dev/null -w '%{speed_download} bytes/s\n' \
  'http://HOST:8080/__down?bytes=2000000000'
```

## Packet loss shows nothing, or the run errors with an ICE timeout

**Check first: is coturn actually reading its configuration?** This is the one
that has bitten in practice, and its symptoms point everywhere except at the
cause.

```sh
journalctl -u coturn | grep -E "Cannot find config|EXPLICIT LISTENER|Default realm"
```

`Cannot find config file` or `NO EXPLICIT LISTENER ADDRESS(ES) ARE CONFIGURED`
means coturn could not read its config and is running on **defaults** — no
realm, no credentials, no relay range. It does not fail in that case; it warns
and carries on, answering STUN and looking healthy while every authenticated
allocation from a browser fails with nothing but an ICE timeout to show for it.

The usual cause is file permissions: coturn drops privileges to its own user, so
a `root:root 0600` config is unreadable to it. It should be
`root:<coturn user> 0640`:

```sh
stat -c '%U:%G %a' /etc/turnserver.conf     # expect root:turnserver 640
systemctl show coturn -p User --value       # the user it drops to
```

A healthy start names your own values back to you:

```
Listener address to use: <your LAN address>
Default realm: <your realm>
```

### A warning about `turnutils_uclient`

It is the obvious tool to reach for, and it will happily report 0% loss against
a relay whose configuration is being ignored — a default coturn permits
anonymous allocations. It proves the daemon is alive, not that your credentials
or realm work. Only a client that authenticates against the configured realm
tests what a browser actually does.

Then, in order of likelihood:

1. **No relay configured.** With TURN disabled the stage is stripped from the
   profile deliberately. Check `/api/profile` for `"packetLossEnabled": true`.
2. **The TURN URI has a scheme.** It must be a bare `host:port`; the engine
   adds the scheme and transport itself.
3. **Loopback peers denied.** If both relay allocations are on the loopback
   address, coturn blocks them by default. See
   [TURN and Packet Loss](TURN-and-Packet-Loss.md).
4. **Relay range unreachable.** UDP 3478 plus `49160-49200` must be open
   between client and guest.
5. **No relay candidate at all.** Confirm with Trickle-ICE before debugging
   anything else.

## The run refuses to start with a configuration error

The front end checks, before starting, that nothing in the configuration could
reach off-LAN: absolute endpoint URLs, a non-null reporting URL, an incomplete
TURN credential pair, or any external reachability stage. The message names the
specific problem. This is deliberate — it fails loudly rather than measuring
and quietly phoning home.

## Repeat requests return instantly

Cache. Measurement responses must carry `cache-control: no-store`; without it
the browser answers the 2nd..Nth identical request from cache and the engine
measures the cache. Check for an intermediary rewriting the header.

## The release workflow refuses to run

It fails while any scaffold `TODO` marker remains anywhere in the tree. That is
intentional. Fix the marker; never weaken the check.

## Certificate stopped renewing

This project reads two files and does not renew them — issuing and renewing the
certificate is the operator's job. Two failure modes account for almost every
case.

**The renewal never runs.** An ACME client that is installed, holds a valid
certificate, and has no cron entry and no timer anywhere renews nothing, and
looks entirely healthy until the day it expires. Verify the schedule exists
rather than trusting the installer that claimed to create it:

```sh
crontab -l -u root | grep -i acme
systemctl list-timers --all | grep -i acme
openssl x509 -in /etc/speedtest/tls/fullchain.pem -noout -enddate
```

**The renewal runs but nothing picks it up.** The certificate is read once, at
startup. New material on disk is not served until the service restarts, so the
renewal hook has to restart the container. If `openssl x509` on the file shows
a later expiry than the certificate your browser is being handed, this is it.

---

See also: [Deployment](Deployment.md) ·
[Configuration](Configuration.md) ·
[TURN and Packet Loss](TURN-and-Packet-Loss.md) ·
[Reading the Results](Reading-the-Results.md)
