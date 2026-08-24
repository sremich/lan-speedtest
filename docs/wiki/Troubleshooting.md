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

**If loaded latency shows `<0.1 ms`**, the reading is genuinely zero and there
is nothing to fix. This is most likely in **Firefox**, which coarsens resource
timing to ~1 ms (Chrome coarsens to ~0.1 ms). On a LAN with a sub-millisecond
round trip, every Firefox latency reading rounds to exactly 0 — and 0 is
falsy, so the engine treats loaded latency as unavailable and returns no
scores.

This cannot be fixed from our side: the engine reads latency only from
`PerformanceResourceTiming`, and forking it is a non-goal. Use Chrome when you
want the quality ratings on a fast LAN. Bandwidth, jitter and packet loss are
unaffected.

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

In order of likelihood:

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

Verify the renewal is actually scheduled, rather than trusting an install
script's output:

```sh
crontab -l -u root
systemctl list-timers --all | grep -i acme
openssl x509 -in /path/to/fullchain.pem -noout -enddate
```

An installed ACME client with no cron entry and no timer renews nothing. This
has already happened once on the predecessor host, which is why it is worth
checking explicitly rather than assuming.
