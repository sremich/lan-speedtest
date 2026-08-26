# Deployment

What a permanent installation needs beyond [Quick Start](Quick-Start.md): a
host, the container, a relay, TLS, and a firewall.

The project ships the application and the relay configuration. It does not ship
anything that creates or manages the host — that is deliberately out of scope,
and any Debian-family machine with Docker will do: a VM, an LXC guest, a spare
mini PC.

## The host

Give it real resources. **4 cores and 4 GB is a sensible floor.** This is the
one component whose slowness is indistinguishable from a slow network — a
backend that cannot keep up reads as a bad result rather than as an error, so
under-provisioning it quietly corrupts every measurement it serves.

If the host is a container guest, it needs to be able to run Docker inside
itself (on Proxmox LXC, that means `nesting=1`).

Give it a **fixed address**, by DHCP reservation or static configuration. The
TURN relay binds a specific address and advertises it in ICE candidates, so a
host whose address moves breaks the packet-loss stage until the relay is
reconfigured.

## The application

```sh
git clone https://github.com/sremich/lan-speedtest.git /opt/speedtest
cd /opt/speedtest
cp .env.example .env
docker compose up -d
```

The compose file pulls a published image — `ghcr.io/sremich/lan-speedtest`,
pinned to an exact tag — and mounts three things from the host:

| Mounted | Why |
|---|---|
| `./config/speedtest.toml` | So a measurement profile can be retuned and the service restarted without pulling a new image |
| `./data` | Stored run history, surviving image updates |
| `/etc/speedtest/tls` | The certificate and key, read-only |

It uses **host networking**. The measurement path should not cross a NAT hop,
and coturn runs natively on the same host.

Check it came up:

```sh
curl -s http://127.0.0.1:8080/api/status
```

`version` must be the release you intended and `gitSha` identifies the exact
commit; both are baked in at image build time.

## TLS

The service **terminates TLS itself**. There is no reverse proxy, for two
reasons: a proxy hop would sit inside the very path being measured, and it is
one more thing to keep configured and renewed.

```sh
# in .env
SPEEDTEST_TLS_BIND=0.0.0.0:443
SPEEDTEST_TLS_CERT_FILE=/etc/speedtest/tls/fullchain.pem
SPEEDTEST_TLS_KEY_FILE=/etc/speedtest/tls/privkey.pem
```

- Both must be set. **Exactly one is a startup error**, not a silent fallback
  to plain HTTP.
- Plain HTTP keeps listening on 8080 for the container health check.
- The binary carries `cap_net_bind_service` and compose grants the matching
  capability, so it binds 443 without running as root.
- The certificate is mounted group-readable by gid 10001, granted to the
  container as a supplementary group.

### Obtaining and renewing the certificate is yours to arrange

This project does not issue certificates and does not renew them. It reads two
files. Getting valid material into those files — and keeping it valid — is the
operator's job, whether that is an internal CA, a wildcard from a public CA
over ACME DNS-01, or a certificate copied in by hand.

Two things are worth building in from the start, because both have a way of
being discovered late:

**Restart the service after renewal.** The certificate is read once, at
startup. A renewal that replaces the files without restarting the container
leaves the old material being served until something else happens to restart
it. If you use acme.sh or certbot, put the restart in the renewal hook.

**Verify the schedule exists, rather than trusting the installer that claimed
to create it.** An ACME client that is installed, holds a valid certificate,
and has no cron entry or timer anywhere will renew nothing — and it looks
completely healthy right up to the day it expires. This is not hypothetical;
it is the exact state a predecessor host in this project turned out to be in,
after its install script reported success.

```sh
crontab -l -u root | grep -i acme
systemctl list-timers --all | grep -i acme
openssl x509 -in /etc/speedtest/tls/fullchain.pem -noout -subject -enddate
```

Check the end state, not the output of the thing that was supposed to produce
it.

## The TURN relay

The packet-loss stage needs a relay. Without one the stage is stripped from the
profile served to the browser, and because every quality rating takes packet
loss as an input, the ratings shift rather than disappear.

coturn runs **natively on the host, not in a container**: the relay needs a
contiguous UDP port range, and publishing dozens of UDP ports through Docker's
userland proxy inserts a hop into the exact path being measured.

```sh
set -a && . ./.env && set +a
sudo -E provisioning/coturn/install-coturn.sh
```

The script renders `/etc/turnserver.conf` from its template, refuses to proceed
if any placeholder is left unsubstituted, restarts the service only when
something actually changed, and verifies that the configured address is really
listening on UDP 3478. It is idempotent — re-running it is a no-op.

It requires all four of `LISTEN_IP`, `TURN_USER`, `TURN_PASS` and `TURN_REALM`
in the environment, and exits with a named error if any is missing. The first
three come from the "Relay setup only" block at the bottom of `.env.example`;
`TURN_REALM` has no default and must be set — the host's FQDN is the usual
choice.

`TURN_USER` and `TURN_PASS` must match the `SPEEDTEST_TURN_*` values the
application serves to the browser, or authentication fails and the stage times
out. See [TURN and Packet Loss](TURN-and-Packet-Loss.md), which also covers
verifying a relay candidate before blaming anything else.

## Firewall

| Port | Protocol | Purpose |
|---|---|---|
| 443 | TCP | The application |
| 8080 | TCP | Plain HTTP, if you are serving without TLS |
| 3478 | UDP | TURN control |
| 49160-49200 | UDP | TURN relay range |

The relay range is defined once in the coturn template and read back out of it
by the installer, so the rule and the config cannot drift.

## Updating a release

```sh
docker compose pull && docker compose up -d
curl -s http://127.0.0.1:8080/api/status
```

The image tag in `docker-compose.yml` is pinned to an exact version rather than
`latest`, so an update is a deliberate edit to that line followed by a pull —
never something that happens because a container restarted.

---

See also: [Quick Start](Quick-Start.md) ·
[Configuration](Configuration.md) ·
[TURN and Packet Loss](TURN-and-Packet-Loss.md) ·
[Troubleshooting](Troubleshooting.md)
