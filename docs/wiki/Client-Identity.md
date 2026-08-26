# Client identity

Who a run belongs to, how far that can honestly be established, and where it
cannot be established at all.

There is no login and no cookie: this is a LAN tool with no accounts. A run is
attributed by the only thing the server can observe for itself — the address
the connection came from — and then labelled, as well as it can be, from three
sources in order of authority.

![Client names and addresses in the history](images/history-table.png)

*One client has been given a name; the others show the address they connected
from. The address is never replaced by the label — it is what correlates a run
with a DHCP lease or a switch port.*

## What names a client

| Source | Where it comes from | Wins over |
|---|---|---|
| A typed name | Entered on the history page, stored server-side against the address | Everything |
| A resolved hostname | A PTR lookup, off the request path, only for configured ranges | The address |
| The address | The connection's peer, or a trusted proxy's `X-Forwarded-For` | — |

The address is always shown somewhere even when a name is set. A friendly label
that replaced it outright would make the history impossible to correlate with
anything else on the network — a DHCP lease table, a switch port, a firewall
log.

Names are per-address, not per-run. Renaming a client relabels its whole
history, which is the point: the machine did not become a different machine
between runs.

## Reverse DNS

Off by default. When switched on it is restricted to explicit address ranges,
and both halves of that sentence matter.

```toml
[server.reverse_dns]
enabled = true
resolver = ""            # empty reads the first nameserver in /etc/resolv.conf
ranges = ["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "100.64.0.0/10", "fc00::/7"]
timeout_ms = 500
ttl_secs = 21600
```

- **Restricted by range.** Unrestricted, a deployment reachable from the
  internet would send a PTR query for every address that visited it to whatever
  upstream resolver it happens to have. That is a quiet outbound leak in
  exchange for a cosmetic label, so the ranges are a hard filter rather than a
  suggestion. An address outside them is never looked up.
- **Never on the request path.** The lookup is fired after a run is stored and
  the response has already gone out. A resolver that is slow, wrong, or absent
  cannot delay or fail a measurement.
- **Misses are cached too.** A client with no PTR record is remembered as
  having no name for `ttl_secs`, so it is not re-queried after every run.
- **Answers are treated as untrusted text.** A PTR record is supplied by
  whoever controls the reverse zone. Names are length-capped and restricted to
  ASCII letters, digits, `.`, `-` and `_`; anything else is discarded rather
  than stored and rendered.

The DNS client is about two hundred lines in `backend/src/netid.rs` rather than
a resolver crate: one query type, one record type, over UDP, with a bounded
response. Compression pointers are followed with a jump cap and every offset is
bounds-checked, because a malicious response is the one input here that is not
under local control.

## Which address, behind a proxy

By default the connection's own peer address decides, and `X-Forwarded-For` is
ignored. That is deliberate. With nothing in front of the service, believing
the header would let anyone on the LAN file a run under any address they liked
by setting one request header.

When there *is* a reverse proxy, name it:

```toml
[server]
trusted_proxies = ["10.0.0.5/32"]
```

The header is then walked right to left, discarding hops that are themselves
trusted proxies, and the first address that is not becomes the client. An
untrusted peer's header is still ignored, so adding a proxy does not open the
spoofing hole for everyone else.

## What cannot be recovered

Three questions come up repeatedly. Two have answers and one does not.

**"It always shows a LAN address even though I reached it over the internet."**
If a reverse proxy terminates the connection, the server genuinely only sees
the proxy. Configure `trusted_proxies` and the real address comes back.

**"It shows a `100.64.x.x` address when I connect over Tailscale."** If you
reach the service over a Tailscale **subnet router**, the router rewrites the
source address as it forwards. That translation happens at layer 3 and leaves
no header behind — the original address is not recorded anywhere in the packet
that arrives, so no amount of server-side work recovers it. Connecting to the
service's own Tailscale address rather than through a subnet route gives the
real client address, because then there is no translation.

**"Can the browser just tell me its own private address?"** No. It used to be
possible via WebRTC host candidates, and every major browser closed that in
about 2019 by replacing the local address with a random mDNS name. The
obfuscation lifts only after the page is granted camera or microphone
permission, which is a wildly disproportionate thing to ask for a label on a
history row. A typed name is the better answer, and it is why one exists.

## The address classification

Every page that shows an address also says what sort of address it is, because
the honest answer to "why is it that number?" differs by kind:

| Shown | Meaning |
|---|---|
| loopback | The test is being served to the machine running it |
| LAN | RFC 1918 — the client, or the last hop that rewrote it |
| Tailscale or carrier NAT | `100.64.0.0/10` — translated on the way, original not recoverable |
| link-local | Assigned without a DHCP server |
| public | The connection reached the server from outside the LAN |

## A note on personal data

Stored runs carry the client's IP address, any hostname resolved for it, the
browser's user agent and whatever was typed into a description; `/metrics`
exposes the most recent run per client, keyed by address. Under the GDPR and UK
GDPR an IP address is personal data.

For a household or personal deployment that is unlikely to matter. If you run
this somewhere it covers other people — an office, a club, a shared building —
treat the stored addresses as personal data and handle them the way you already
handle your other logs: a retention window (see
[History and Metrics](History-and-Metrics.md#retention)) rather than keeping
everything forever, and `/metrics` left off or firewalled unless something is
actually scraping it.

---

See also: [History and Metrics](History-and-Metrics.md) ·
[Configuration](Configuration.md) ·
[Troubleshooting](Troubleshooting.md)
