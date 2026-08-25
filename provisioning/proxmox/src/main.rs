//! Idempotent provisioning for the speed test guest.
//!
//! From nothing: creates the container, installs Docker and coturn, issues the
//! TLS certificate and deploys the application. Run again and nothing changes.
//!
//! Everything is driven over SSH to the Proxmox node using `pct` and `pvesh`.
//! That matters for more than convenience: `pct exec` reaches the guest through
//! the hypervisor rather than the network, so the first run works before the
//! guest has a DHCP lease — no chicken-and-egg with the reservation.

use std::path::PathBuf;

use anyhow::bail;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use speedtest_provision::config::Config;
use speedtest_provision::proxmox::Proxmox;
use speedtest_provision::runner::{Runner, SshRunner};
use speedtest_provision::setup::{self, Secrets};

#[derive(Parser)]
#[command(name = "speedtest-provision", about = "Provision the speed test guest")]
struct Cli {
    /// Provisioning configuration.
    #[arg(long, default_value = "provisioning/proxmox/provision.toml")]
    config: PathBuf,

    /// Repository root, used to locate the files pushed to the guest.
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the MAC the guest will use, and change nothing.
    ///
    /// Run this before `apply` so the DHCP reservation can be created first —
    /// otherwise the guest comes up on whatever address the server hands out.
    Mac,
    /// Read-only: report what exists and what would change.
    Plan,
    /// Create and configure the guest. Safe to re-run.
    Apply {
        /// Skip the TLS step (useful when the DNS token is not to hand).
        #[arg(long)]
        skip_tls: bool,
    },
    /// Re-check an existing guest: health, and that renewal is scheduled.
    Verify,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("PROVISION_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .without_time()
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let cfg = Config::load(&cli.config)?;

    // `mac` answers from configuration alone, so it works with no connectivity.
    if matches!(cli.command, Command::Mac) {
        let mac = cfg.guest.effective_mac();
        println!("{mac}");
        eprintln!();
        eprintln!("Create a DHCP reservation for this MAC on the address you want the");
        eprintln!("guest to have, then run `apply`. The MAC is derived from the vmid and");
        eprintln!("is stable across rebuilds, so the reservation keeps working.");
        return Ok(());
    }

    let ssh = SshRunner::new(
        &cfg.proxmox.ssh_user,
        &cfg.proxmox.ssh_host,
        cfg.proxmox.ssh_identity.as_deref(),
    );
    run(&cli, &cfg, &ssh).await
}

async fn run(cli: &Cli, cfg: &Config, runner: &dyn Runner) -> anyhow::Result<()> {
    let pve = Proxmox::new(runner, &cfg.proxmox.node);
    let vmid = cfg.guest.vmid;

    let version = pve.preflight().await?;
    tracing::info!(
        "node {} — {}",
        cfg.proxmox.node,
        version.lines().next().unwrap_or(&version)
    );

    // The ownership guard runs before anything else, in every mode.
    let existing = pve.ensure_ours(vmid).await?;

    match &cli.command {
        Command::Mac => unreachable!("handled before connecting"),

        Command::Plan => {
            match &existing {
                None => {
                    tracing::info!("container {vmid} does not exist — apply would create it");
                    tracing::info!("  hostname {}", cfg.guest.hostname);
                    tracing::info!(
                        "  mac      {} (pin a DHCP reservation to this)",
                        cfg.guest.effective_mac()
                    );
                    tracing::info!("  net0     {}", cfg.guest.net0());
                    tracing::info!("  tags     {}", cfg.guest.tags());
                }
                Some(c) => {
                    tracing::info!(
                        "container {vmid} exists and is ours (status {}) — apply would \
                         re-run configuration, which is a no-op if nothing has drifted",
                        c.status.as_deref().unwrap_or("unknown")
                    );
                }
            }
            Ok(())
        }

        Command::Apply { skip_tls } => {
            let secrets = if *skip_tls {
                None
            } else {
                Some(Secrets::from_env()?)
            };
            apply(cli, cfg, &pve, existing.is_some(), secrets.as_ref()).await
        }

        Command::Verify => {
            if existing.is_none() {
                bail!("container {vmid} does not exist — nothing to verify");
            }
            setup::verify_renewal_scheduled(&pve, vmid, "/etc/speedtest/tls").await?;
            setup::wait_for_health(&pve, vmid, 3).await?;
            report_address(&pve, vmid).await;
            Ok(())
        }
    }
}

async fn apply(
    cli: &Cli,
    cfg: &Config,
    pve: &Proxmox<'_>,
    already_exists: bool,
    secrets: Option<&Secrets>,
) -> anyhow::Result<()> {
    let vmid = cfg.guest.vmid;
    let root = &cli.repo_root;

    if !already_exists {
        tracing::info!("creating container {vmid} ({})", cfg.guest.hostname);
        tracing::info!("  pinned MAC {}", cfg.guest.effective_mac());
        pve.create(&cfg.guest, None).await?;
    } else {
        tracing::info!("container {vmid} already exists — reconfiguring in place");
    }

    // Starting an already-running container is not an error worth failing on.
    let current = pve.find_lxc(vmid).await?;
    if !current.map(|c| c.is_running()).unwrap_or(false) {
        tracing::info!("starting container {vmid}");
        pve.start(vmid).await?;
        wait_until_responsive(pve, vmid).await?;
    }

    setup::install_base(pve, vmid).await?;
    setup::install_ssh_key(pve, vmid, &expand_home(&cfg.guest.ssh_authorized_key)).await?;
    setup::install_docker(pve, vmid).await?;

    let listen_ip = pve.guest_ipv4(vmid).await?.ok_or_else(|| {
        anyhow::anyhow!(
            "the guest has no IPv4 address on eth0. It is configured for DHCP with \
                 MAC {} — is the reservation in place and the bridge/VLAN correct?",
            cfg.guest.effective_mac()
        )
    })?;
    tracing::info!("guest address {listen_ip}");

    if let Some(s) = secrets {
        setup::install_coturn(
            pve,
            vmid,
            &root.join("provisioning/coturn/turnserver.conf.template"),
            &root.join("provisioning/coturn/install-coturn.sh"),
            &listen_ip,
            s,
        )
        .await?;

        setup::setup_tls(pve, vmid, s).await?;
    } else {
        tracing::warn!("--skip-tls: coturn and TLS were not configured");
    }

    if let Some(s) = secrets {
        setup::registry_login(pve, vmid, s).await?;
    }

    let env_body = deploy_env(cfg, secrets, &listen_ip);
    setup::deploy_app(
        pve,
        vmid,
        &root.join("docker-compose.yml"),
        &root.join("config/speedtest.toml"),
        &env_body,
    )
    .await?;

    setup::wait_for_health(pve, vmid, 30).await?;

    tracing::info!("done. Container {vmid} is provisioned and serving.");
    report_address(pve, vmid).await;
    Ok(())
}

/// Expands a leading `~` so the config can name a key the ordinary way.
fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// The `.env` written to the guest for compose.
fn deploy_env(cfg: &Config, secrets: Option<&Secrets>, listen_ip: &str) -> String {
    let mut lines = vec![
        "# Written by speedtest-provision. Do not edit by hand.".to_string(),
        format!("SPEEDTEST_PROFILE={}", cfg.guest.measurement_profile),
        "SPEEDTEST_BIND=0.0.0.0:8080".to_string(),
        "SPEEDTEST_LOG=info".to_string(),
        // Explicit rather than relying on the config file's relative default,
        // so the path in the container is unambiguous against the bind mount.
        "SPEEDTEST_HISTORY_DB=/app/data/history.db".to_string(),
    ];
    // Omitted entirely when unset, so the shipped config's own default wins
    // rather than being overridden with a blank.
    if let Some(name) = &cfg.guest.site_name {
        lines.push(format!("SPEEDTEST_SITE_NAME={}", name.trim()));
    }
    if cfg.guest.reverse_dns {
        lines.push("SPEEDTEST_REVERSE_DNS=1".to_string());
        if let Some(resolver) = &cfg.guest.dns_resolver {
            lines.push(format!("SPEEDTEST_DNS_RESOLVER={}", resolver.trim()));
        }
    }
    match secrets {
        Some(s) => {
            // TLS is configured only alongside the certificate we just issued.
            lines.push("SPEEDTEST_TLS_BIND=0.0.0.0:443".into());
            lines.push("SPEEDTEST_TLS_CERT_FILE=/etc/speedtest/tls/fullchain.pem".into());
            lines.push("SPEEDTEST_TLS_KEY_FILE=/etc/speedtest/tls/privkey.pem".into());
            lines.push("SPEEDTEST_TURN_ENABLED=true".into());
            lines.push(format!("SPEEDTEST_TURN_URI={listen_ip}:3478"));
            lines.push(format!("SPEEDTEST_TURN_USER={}", s.turn_user));
            lines.push(format!("SPEEDTEST_TURN_PASS={}", s.turn_pass));
        }
        None => lines.push("SPEEDTEST_TURN_ENABLED=false".into()),
    }
    lines.join("\n")
}

/// Waits for the guest to run commands, which it cannot do the instant it starts.
async fn wait_until_responsive(pve: &Proxmox<'_>, vmid: u32) -> anyhow::Result<()> {
    for i in 1..=30 {
        if pve.exec(vmid, "true").await?.succeeded() {
            return Ok(());
        }
        if i == 30 {
            bail!("container {vmid} started but never became responsive to `pct exec`");
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    Ok(())
}

async fn report_address(pve: &Proxmox<'_>, vmid: u32) {
    if let Ok(Some(ip)) = pve.guest_ipv4(vmid).await {
        tracing::info!("guest is reachable at https://{ip}/ (and at its DNS name)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed example, which is the only one CI can see — a real
    /// `provision.toml` carries the node's address and is gitignored.
    fn example() -> Config {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("provision.toml.example");
        Config::load(&path).expect("the committed example parses and validates")
    }

    #[test]
    fn the_deploy_env_carries_the_reverse_dns_switch_only_when_asked() {
        let mut cfg = example();

        cfg.guest.reverse_dns = false;
        cfg.guest.dns_resolver = None;
        let off = deploy_env(&cfg, None, "10.0.0.1");
        assert!(
            !off.contains("SPEEDTEST_REVERSE_DNS"),
            "the key is omitted rather than set to a falsey string, so the \
             shipped config's own default stays in charge"
        );

        cfg.guest.reverse_dns = true;
        let on = deploy_env(&cfg, None, "10.0.0.1");
        assert!(on.lines().any(|l| l == "SPEEDTEST_REVERSE_DNS=1"));
        assert!(
            !on.contains("SPEEDTEST_DNS_RESOLVER"),
            "an unset resolver must not become an empty one — the service \
             reads /etc/resolv.conf in that case"
        );

        cfg.guest.dns_resolver = Some("  10.0.0.53:53  ".into());
        let explicit = deploy_env(&cfg, None, "10.0.0.1");
        assert!(explicit
            .lines()
            .any(|l| l == "SPEEDTEST_DNS_RESOLVER=10.0.0.53:53"));
    }

    #[test]
    fn the_deploy_env_is_one_key_per_line_with_no_quoting() {
        // Compose reads this file itself and does no unquoting whatsoever, so
        // a value containing a newline would inject an unrelated variable.
        // Asserted against the real thing rather than against an escaped
        // string, because the property is what compose will make of it.
        let cfg = example();
        let body = deploy_env(&cfg, None, "10.0.0.1");
        for line in body.lines().filter(|l| !l.starts_with('#')) {
            let (key, _) = line
                .split_once('=')
                .unwrap_or_else(|| panic!("line {line:?} is not KEY=VALUE"));
            assert!(
                key.chars().all(|c| c.is_ascii_uppercase() || c == '_'),
                "key {key:?} is not a plain environment variable name"
            );
        }
    }

    #[test]
    fn turn_credentials_appear_only_alongside_a_certificate() {
        // Without secrets the relay is switched off rather than half
        // configured: with a uri but no credentials the engine falls back to
        // fetching them from Cloudflare, which is the one thing this project
        // exists to prevent.
        let cfg = example();
        let body = deploy_env(&cfg, None, "10.0.0.1");
        assert!(body.contains("SPEEDTEST_TURN_ENABLED=false"));
        assert!(!body.contains("SPEEDTEST_TURN_URI"));
        assert!(!body.contains("SPEEDTEST_TURN_PASS"));
    }
}
