# Deployment

> **Status: partial.** The container and relay halves are in place as of
> 0.3.0. Automated guest provisioning and TLS land in 0.4.0; until then the
> guest is prepared by hand using the notes below.

## Target shape

One Debian LXC on the hypervisor, running:

- the application as a Docker container (backend + built front end in one
  image), and
- coturn natively from the distribution package.

The container uses host networking. The measurement path should not cross an
extra NAT hop, and coturn on the same guest needs the relay range reachable at
the address its candidates advertise.

## Sizing

Give the guest real resources. The predecessor host ran on 1 core and 512 MB,
which is nowhere near enough for a backend expected to saturate 10 GbE — the
whole point of the Rust data path is wasted if the guest cannot schedule it.

## Deploying the application

```sh
cp .env.example .env      # fill in; never commit it
docker compose pull
docker compose up -d
docker compose logs -f
```

Confirm the build that is actually running:

```sh
curl -s http://127.0.0.1:8080/api/status
```

`version` must match the release you intended, and `gitSha` identifies the
exact commit. Both are baked in at image build time.

After every release goes green, bump the image pin in `docker-compose.yml` and
commit it.

## Relay

See [TURN and Packet Loss](TURN-and-Packet-Loss.md). In short: run the
installer with the credentials from `.env`, then verify a `relay` candidate
appears via Trickle-ICE before believing any packet-loss figure.

## TLS

The wildcard certificate is issued on the guest with an ACME client using the
DNS-01 challenge, and installed where the terminating service reads it. The
renewal hook restarts that service.

**Verify the renewal is actually scheduled.** An installed ACME client with no
cron entry and no systemd timer renews nothing, and the failure is invisible
until the certificate expires:

```sh
crontab -l -u root
systemctl list-timers --all | grep -i acme
openssl x509 -in /path/to/fullchain.pem -noout -subject -enddate
```

This has already happened once on the predecessor host — the install script
reported success and left no schedule behind.

## Firewall

| Port | Protocol | Purpose |
|---|---|---|
| 443 | TCP | The application |
| 3478 | UDP | TURN control |
| 49160-49200 | UDP | TURN relay range |

The relay range is defined once in the coturn template; read it from there
rather than duplicating the numbers.

## Planned for 0.4.0

- Idempotent guest creation, from nothing to serving in one command
- A pinned MAC address on the guest's interface, so a DHCP reservation
  survives a rebuild — a freshly created guest otherwise gets a random MAC and
  a different address, which defeats the point of reproducible provisioning
- coturn and Docker installed and configured by the same run
- ACME issuance plus a verified renewal schedule
