# TURN and packet loss

The packet-loss stage needs a relay of your own. This page covers what the
engine actually does with it, how to stand one up, and how to prove it works
before blaming anything else.

## How the measurement works

The engine does not ping anything to measure loss. It opens **two**
`RTCPeerConnection`s in the same browser tab, forces `iceTransportPolicy` to
`relay` so both must use the TURN server, and sends a numbered UDP burst from
one to the other. Every message therefore travels browser → relay → browser,
and loss is the fraction of numbers that never arrive.

Two consequences worth internalising:

- The relay is **required**. Without one there is no packet-loss figure, and
  because every quality rating takes packet loss as an input, the ratings shift
  rather than disappear (missing loss scores as zero points, not as "unknown").
- Each peer connection gets its **own relay allocation**, so the relay must be
  willing to forward from one of its own allocations to another.

## Credentials

The engine builds the `RTCPeerConnection` client-side, so the TURN username and
password necessarily reach the browser. There is no way around this short of
short-lived credentials.

Therefore: use values that exist only for this, on the LAN, and are reused
nowhere else. The password is supplied through `SPEEDTEST_TURN_PASS` and is
never committed.

**Both** user and password must be set. With either one missing the engine
silently falls back to fetching credentials from `turnServerCredsApiUrl`, which
defaults to a Cloudflare endpoint — a quiet breach of the whole point of this
project. Configuration validation rejects a half-configured relay at startup,
and a test asserts both fields are present.

Per-test HMAC credentials (`use-auth-secret`) minted by the backend are the
intended replacement; they remove the shared static secret, though the browser
still sees *a* credential.

## Configuration

`turnServerUri` is a bare `host:port` with **no scheme** — the engine builds
`turn:{uri}?transport=udp` itself. Writing a scheme in produces a malformed URL
and no relay candidate.

## Setting up coturn

Run coturn natively on the host, not in a container: the relay needs a
contiguous UDP port range, and publishing dozens of UDP ports through Docker's
userland proxy inserts a hop into the exact path being measured.

```sh
set -a && . ./.env && set +a
sudo -E provisioning/coturn/install-coturn.sh
```

Four variables must be in the environment, and the script exits with a named
error if any is missing:

| Variable | What it is |
|---|---|
| `LISTEN_IP` | The host's LAN address, which coturn binds and relays on. Not a wildcard — the relay should answer on the measured path and nowhere else |
| `TURN_USER` | Must match `SPEEDTEST_TURN_USER` |
| `TURN_PASS` | Must match `SPEEDTEST_TURN_PASS` |
| `TURN_REALM` | The realm, usually the host's FQDN. There is no default |

The first three sit in the "Relay setup only" block at the bottom of
`.env.example`; `TURN_REALM` is commented out there and has to be uncommented.

The script renders the config template, refuses to proceed if any placeholder
is left unsubstituted, installs the result readable by the user coturn drops
to, restarts the service only when something actually changed, and verifies
that the configured address is listening on UDP 3478. Re-running it is a no-op.

The relay range is `49160-49200`, defined once in the template and read back
out of it by the installer so the firewall rule and the config cannot drift.

## Verifying a relay

Trickle-ICE is the quickest check:

<https://webrtc.github.io/samples/src/content/peerconnection/trickle-ice/>

Enter the TURN URI with `?transport=udp`, plus the username and password. A
working relay yields at least one candidate of type **`relay`**. Anything less
means the packet-loss stage cannot work — and a reported 0% would be
meaningless.

The e2e suite automates exactly this check, then runs a full measurement
through the relay.

`turnutils_uclient` is the obvious tool to reach for, and is a trap here: it
will happily report 0% loss against a relay whose configuration is being
ignored, because a default coturn permits anonymous allocations. It proves the
daemon is alive, not that your credentials or realm work.

## Loopback peers

coturn **denies loopback peer addresses by default**. When both relay
allocations are on the loopback address — which is the case for the
containerised test relay — the two peers cannot reach each other, and the
engine reports `ICE connection timeout!` rather than anything mentioning
permissions.

The e2e compose file passes `--allow-loopback-peers` for that reason, and only
for that reason. The production template keeps loopback denied, along with
link-local and multicast ranges: a relay on a LAN address has no business
forwarding to any of them.

---

See also: [Deployment](Deployment.md) ·
[Configuration](Configuration.md) ·
[Troubleshooting](Troubleshooting.md) ·
[Reading the Results](Reading-the-Results.md)
