# Deployment

One command, from nothing to serving. Run it again and nothing changes.

## What provisioning does

1. Creates the LXC with a **pinned MAC**, DHCP networking, `nesting=1` for
   Docker, and an ownership tag.
2. Installs base packages, Docker (from upstream, for the compose plugin), and
   coturn.
3. Issues the wildcard certificate over ACME DNS-01 and installs a weekly
   renewal — then **verifies the schedule actually exists**.
4. Deploys the application with compose and waits for it to answer its own
   health check.

## Prerequisites

- SSH access to the Proxmox node as a user that can run `pct` and `pvesh`
  (in practice root), with key authentication.
- `.env` filled in from `.env.example` — the DNS token, the ACME contact and
  zone, and the TURN credentials.
- `provisioning/proxmox/provision.toml` pointed at your node.

The provisioner reads those values from its **environment**, not from the file
directly, so `.env` has to be exported into the shell first. `plan` is
read-only and needs none of them, which makes it easy to believe everything is
configured until `apply` stops on the first missing key:

```bash
set -a && . <(tr -d '' < .env) && set +a
```

The `tr` is not decorative — an `.env` saved on Windows has CRLF line endings,
and the trailing carriage return ends up inside the value.

## Why SSH and not the REST API

The node is driven with `pct` and `pvesh` over SSH. Beyond avoiding an API
token, this buys something the REST API cannot: **`pct exec` reaches the guest
through the hypervisor rather than the network.** Configuration therefore works
before the guest has a DHCP lease, which removes the chicken-and-egg between
"the guest needs an address to be configured" and "the address comes from a
reservation you cannot make until the MAC exists".

The trade-off is honest: root on the hypervisor is broader access than a
scoped token would be.

## The order that matters

```bash
speedtest-provision mac
```

Prints the MAC and changes nothing. **Create the DHCP reservation for it
first.** A freshly created container otherwise gets a random MAC, so a rebuild
lands on a different address and the reservation stops working — which defeats
the point of reproducible provisioning. The MAC is derived from the VMID, so it
is stable and reproducible without a lookup table.

```bash
speedtest-provision plan      # read-only: what exists, what would change
speedtest-provision apply     # create and configure; safe to re-run
speedtest-provision verify    # health, and that renewal is really scheduled
```

`--skip-tls` runs everything except coturn and the certificate, which is useful
before the DNS token is to hand.

## The ownership guard

Provisioning tags every guest it creates and **refuses to modify any guest that
does not carry that tag**. If the configured VMID is already taken by something
else, the run stops before any mutating command, names the guest it found, and
changes nothing.

This is checked in every mode, including `plan`, and is the most heavily tested
behaviour in the crate. Matching is on an exact tag: a guest tagged
`not-speedtest-provisioned` is not ours.

## Sizing

Give the guest real resources — 4 cores and 4 GB is a sensible floor. The
predecessor ran on 1 core and 512 MB, which is nowhere near enough for a backend
whose entire job is to not be the bottleneck.

## TLS

The service **terminates TLS itself**. There is no reverse proxy, for two
reasons: a proxy hop would sit inside the very path being measured, and it is
one more thing to keep configured and renewed.

- Both `tls_cert_file` and `tls_key_file` must be set. Exactly one is a startup
  error, not a silent fallback to plain HTTP.
- Plain HTTP keeps listening on 8080 for the container health check.
- The binary carries `cap_net_bind_service`, and compose grants the matching
  capability, so it can bind 443 without running as root.

### Renewal

Renewal is scheduled weekly, and the ACME hook restarts the service so new
material is actually served. Provisioning **verifies both** rather than trusting
the installer's output, because the predecessor host is in exactly the failure
state that produces: acme.sh installed, certificate valid, and no cron entry or
timer anywhere. Nothing would ever have renewed it.

To check by hand:

```bash
crontab -l -u root | grep acme
openssl x509 -in /etc/speedtest/tls/fullchain.pem -noout -subject -enddate
```

## Firewall

| Port | Protocol | Purpose |
|---|---|---|
| 443 | TCP | The application |
| 3478 | UDP | TURN control |
| 49160-49200 | UDP | TURN relay range |

The relay range is defined once in the coturn template and read back out of it
by the installer, so the rule and the config cannot drift.

## Updating a release

```bash
docker compose pull && docker compose up -d
curl -s http://127.0.0.1:8080/api/status
```

`version` must match the release you intended and `gitSha` identifies the exact
commit; both are baked in at image build time. After every release goes green,
bump the image pin in `docker-compose.yml` and commit it.
