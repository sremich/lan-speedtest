//! Proxmox operations, driven through `pvesh` and `pct` on the node.
//!
//! The important behaviour here is not the command building; it is
//! `ensure_ours`, which is what stops this tool from ever modifying a guest it
//! did not create.

use anyhow::{bail, Context};
use serde::Deserialize;

use crate::config::{Guest, OWNER_TAG};
use crate::runner::{argv, Runner};

/// A container as Proxmox reports it.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Lxc {
    pub vmid: u32,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    /// Semicolon-separated in Proxmox's own representation.
    #[serde(default)]
    pub tags: Option<String>,
}

impl Lxc {
    pub fn tag_list(&self) -> Vec<String> {
        self.tags
            .as_deref()
            .unwrap_or("")
            .split(';')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// Exact tag match — a guest tagged `not-speedtest-provisioned` is not ours.
    pub fn is_ours(&self) -> bool {
        self.tag_list().iter().any(|t| t == OWNER_TAG)
    }

    pub fn is_running(&self) -> bool {
        self.status.as_deref() == Some("running")
    }
}

pub struct Proxmox<'a> {
    runner: &'a dyn Runner,
    node: String,
}

impl<'a> Proxmox<'a> {
    pub fn new(runner: &'a dyn Runner, node: &str) -> Self {
        Self {
            runner,
            node: node.to_string(),
        }
    }

    /// Confirms we are talking to a Proxmox node at all, before doing anything.
    pub async fn preflight(&self) -> anyhow::Result<String> {
        let out = self.runner.run(&argv(["pveversion"])).await?;
        if !out.succeeded() {
            bail!(
                "`pveversion` failed on the node — is this a Proxmox host and does the \
                 SSH user have permission? ({})",
                out.stderr.trim()
            );
        }
        Ok(out.stdout.trim().to_string())
    }

    pub async fn list_lxc(&self) -> anyhow::Result<Vec<Lxc>> {
        let out = self
            .runner
            .run(&argv([
                "pvesh",
                "get",
                &format!("/nodes/{}/lxc", self.node),
                "--output-format",
                "json",
            ]))
            .await?;

        if !out.succeeded() {
            bail!("listing containers failed: {}", out.stderr.trim());
        }
        serde_json::from_str(&out.stdout)
            .with_context(|| format!("unexpected JSON from pvesh: {}", out.stdout))
    }

    pub async fn find_lxc(&self, vmid: u32) -> anyhow::Result<Option<Lxc>> {
        Ok(self.list_lxc().await?.into_iter().find(|c| c.vmid == vmid))
    }

    /// The ownership guard.
    ///
    /// `Ok(Some(_))` when the VMID exists and carries our tag, `Ok(None)` when
    /// it does not exist, and an error when it exists but was not created by
    /// us. That last case must never be papered over: the VMID belongs to
    /// something else, and continuing would reconfigure a guest this tool has
    /// no business touching.
    pub async fn ensure_ours(&self, vmid: u32) -> anyhow::Result<Option<Lxc>> {
        let Some(existing) = self.find_lxc(vmid).await? else {
            return Ok(None);
        };

        if !existing.is_ours() {
            bail!(
                "REFUSING TO CONTINUE: container {vmid} on node {} already exists \
                 (name {:?}, tags {:?}) and does not carry the '{OWNER_TAG}' tag. \
                 This tool only manages guests it created, so nothing has been changed. \
                 Choose a different vmid.",
                self.node,
                existing.name.as_deref().unwrap_or("<unnamed>"),
                existing.tags.as_deref().unwrap_or("<none>")
            );
        }

        Ok(Some(existing))
    }

    /// The `pct create` argv for a guest.
    pub fn create_argv(&self, g: &Guest, ssh_pubkey_path: Option<&str>) -> Vec<String> {
        let mut a = vec![
            "pct".to_string(),
            "create".into(),
            g.vmid.to_string(),
            g.ostemplate.clone(),
            "--hostname".into(),
            g.hostname.clone(),
            "--cores".into(),
            g.cores.to_string(),
            "--memory".into(),
            g.memory_mb.to_string(),
            "--swap".into(),
            g.swap_mb.to_string(),
            "--rootfs".into(),
            g.rootfs.clone(),
            "--net0".into(),
            g.net0(),
            "--features".into(),
            g.features.clone(),
            "--tags".into(),
            g.tags(),
            "--onboot".into(),
            if g.onboot { "1" } else { "0" }.into(),
            "--unprivileged".into(),
            if g.unprivileged { "1" } else { "0" }.into(),
        ];
        if let Some(key) = ssh_pubkey_path {
            a.push("--ssh-public-keys".into());
            a.push(key.to_string());
        }
        a
    }

    pub async fn create(&self, g: &Guest, ssh_pubkey_path: Option<&str>) -> anyhow::Result<()> {
        let out = self
            .runner
            .run(&self.create_argv(g, ssh_pubkey_path))
            .await?;
        if !out.succeeded() {
            bail!(
                "creating container {} failed: {}{}",
                g.vmid,
                out.stdout.trim(),
                out.stderr.trim()
            );
        }
        Ok(())
    }

    pub async fn start(&self, vmid: u32) -> anyhow::Result<()> {
        let out = self
            .runner
            .run(&argv(["pct", "start", &vmid.to_string()]))
            .await?;
        if !out.succeeded() {
            bail!("starting container {vmid} failed: {}", out.stderr.trim());
        }
        Ok(())
    }

    /// Runs a command inside the container.
    ///
    /// `pct exec` reaches the guest through the hypervisor rather than the
    /// network, so configuration works before the guest has an address — which
    /// is what makes a first run possible before the DHCP reservation exists.
    pub async fn exec(&self, vmid: u32, command: &str) -> anyhow::Result<crate::runner::Output> {
        self.runner
            .run(&argv([
                "pct",
                "exec",
                &vmid.to_string(),
                "--",
                "bash",
                "-lc",
                // A fresh Debian container has no locales generated, so
                // anything invoking perl floods stderr with warnings that
                // bury the actual error.
                &format!("export LANG=C.UTF-8 LC_ALL=C.UTF-8; {command}"),
            ]))
            .await
    }

    /// Runs a command inside the container, failing if it does.
    ///
    /// The command is redacted before it reaches the error, because these
    /// commands carry credentials as leading environment assignments and an
    /// error message ends up in logs, terminals and chat transcripts.
    pub async fn exec_checked(&self, vmid: u32, command: &str) -> anyhow::Result<String> {
        let out = self.exec(vmid, command).await?;
        if !out.succeeded() {
            bail!(
                "in container {vmid}, `{}` exited {}: {}{}",
                redact_secrets(command),
                out.status,
                out.stdout.trim(),
                out.stderr.trim()
            );
        }
        Ok(out.stdout)
    }

    /// Copies a file from the node into the container.
    pub async fn push(&self, vmid: u32, node_path: &str, guest_path: &str) -> anyhow::Result<()> {
        // The destination directory may not exist yet on a fresh guest.
        if let Some(dir) = guest_path.rsplit_once('/').map(|(d, _)| d) {
            if !dir.is_empty() {
                self.exec_checked(vmid, &format!("mkdir -p {dir}")).await?;
            }
        }
        let out = self
            .runner
            .run(&argv([
                "pct",
                "push",
                &vmid.to_string(),
                node_path,
                guest_path,
            ]))
            .await?;
        if !out.succeeded() {
            bail!(
                "pushing {node_path} into container {vmid} at {guest_path} failed: {}",
                out.stderr.trim()
            );
        }
        Ok(())
    }

    /// Copies a local file onto the node itself (staging for `pct push`).
    pub async fn upload_to_node(
        &self,
        local: &std::path::Path,
        node_path: &str,
    ) -> anyhow::Result<()> {
        self.runner.upload(local, node_path).await
    }

    /// Escape hatch for a bare command on the node, e.g. tidying staged files.
    pub async fn run_raw(&self, parts: &[&str]) -> anyhow::Result<crate::runner::Output> {
        self.runner
            .run(&parts.iter().map(|s| s.to_string()).collect::<Vec<_>>())
            .await
    }

    /// The guest's IPv4 address on eth0, once DHCP has supplied one.
    pub async fn guest_ipv4(&self, vmid: u32) -> anyhow::Result<Option<String>> {
        let out = self
            .exec(vmid, "ip -4 -json addr show dev eth0 2>/dev/null || true")
            .await?;
        if !out.succeeded() || out.stdout.trim().is_empty() {
            return Ok(None);
        }
        Ok(parse_ipv4_from_ip_json(&out.stdout))
    }
}

/// Masks credential values in a shell command before it is logged or returned
/// in an error.
///
/// Provisioning passes secrets as leading `NAME='value'` assignments, so a
/// failure would otherwise print the TURN password or the ACME token verbatim.
/// This has happened once; the rule is that a secret never reaches an error
/// string, not that we remember to be careful at each call site.
pub fn redact_secrets(command: &str) -> String {
    const SENSITIVE: [&str; 6] = ["PASS", "TOKEN", "SECRET", "CREDENTIAL", "PASSWD", "KEY"];

    let mut out = String::with_capacity(command.len());
    let mut rest = command;

    while let Some(eq) = rest.find("='") {
        // The assignment name is the run of NAME-ish characters before '='.
        let name_start = rest[..eq]
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .map(|i| i + 1)
            .unwrap_or(0);
        let name = &rest[name_start..eq];

        let looks_sensitive = !name.is_empty()
            && SENSITIVE
                .iter()
                .any(|s| name.to_ascii_uppercase().contains(s));

        // Find the end of the single-quoted value.
        let value_start = eq + 2;
        let Some(close) = end_of_quoted_value(&rest[value_start..]) else {
            break;
        };
        let value_end = value_start + close + 1;

        if looks_sensitive {
            out.push_str(&rest[..value_start]);
            out.push_str("***'");
        } else {
            out.push_str(&rest[..value_end]);
        }
        rest = &rest[value_end..];
    }

    out.push_str(rest);
    out
}

/// Index of the closing quote of a shell single-quoted value.
///
/// `s` starts immediately after the opening quote. A value containing a quote
/// is written `'aa'\''bb'`, which is still one value — treating the first `'`
/// as the end would mask only the head and leave the rest of a password in
/// plain sight.
fn end_of_quoted_value(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            if s[i..].starts_with("'\\''") {
                i += 4;
                continue;
            }
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Pulls the first global IPv4 out of `ip -4 -json addr show`.
pub fn parse_ipv4_from_ip_json(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v.as_array()?
        .iter()
        .flat_map(|iface| iface.get("addr_info")?.as_array().cloned())
        .flatten()
        .find_map(|a| {
            let local = a.get("local")?.as_str()?;
            let scope = a.get("scope").and_then(|s| s.as_str()).unwrap_or("global");
            (scope == "global" && local != "127.0.0.1").then(|| local.to_string())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{FakeRunner, Output};

    fn lxc(tags: Option<&str>) -> Lxc {
        Lxc {
            vmid: 11050,
            name: Some("speed".into()),
            status: Some("running".into()),
            tags: tags.map(str::to_string),
        }
    }

    fn guest() -> Guest {
        Guest {
            vmid: 11050,
            hostname: "speed".into(),
            ostemplate: "local:vztmpl/debian-13-standard_13.0-1_amd64.tar.zst".into(),
            cores: 4,
            memory_mb: 4096,
            swap_mb: 512,
            rootfs: "local-lvm:16".into(),
            bridge: "vmbr0".into(),
            vlan_tag: None,
            mac: None,
            unprivileged: true,
            features: "nesting=1,keyctl=1".into(),
            onboot: true,
            extra_tags: vec![],
            measurement_profile: "lan-1g".into(),
            site_name: None,
            ssh_authorized_key: "~/.ssh/id_ed25519.pub".into(),
        }
    }

    #[test]
    fn ownership_is_decided_by_an_exact_tag_match() {
        assert!(!lxc(None).is_ours());
        assert!(!lxc(Some("")).is_ours());
        assert!(!lxc(Some("production;database")).is_ours());
        assert!(lxc(Some(OWNER_TAG)).is_ours());
        assert!(lxc(Some(&format!("lan;{OWNER_TAG};speedtest"))).is_ours());
    }

    #[test]
    fn a_tag_that_merely_contains_the_marker_does_not_count() {
        // Substring matching would be a real hazard here: a guest tagged
        // "not-speedtest-provisioned" is emphatically not ours.
        assert!(!lxc(Some("not-speedtest-provisioned")).is_ours());
        assert!(!lxc(Some("speedtest-provisioned-old")).is_ours());
    }

    #[tokio::test]
    async fn an_untagged_guest_on_our_vmid_stops_everything() {
        // The single most important test in this crate. Stephen's rule is that
        // provisioning must never touch a guest it did not create, and a VMID
        // collision is exactly how that would happen.
        let listing = serde_json::to_string(&vec![serde_json::json!({
            "vmid": 11050, "name": "someone-elses-db", "status": "running",
            "tags": "production"
        })])
        .unwrap();
        let fake = FakeRunner::new(vec![Output::ok(listing)]);
        let pve = Proxmox::new(&fake, "pve");

        let err = pve.ensure_ours(11050).await.unwrap_err().to_string();
        assert!(err.contains("REFUSING TO CONTINUE"), "{err}");
        assert!(err.contains("someone-elses-db"), "{err}");

        // And nothing beyond the read-only listing was attempted.
        let lines = fake.command_lines();
        assert_eq!(lines.len(), 1, "extra commands ran: {lines:?}");
        assert!(lines[0].starts_with("pvesh get"), "{lines:?}");
        assert!(!fake.ran_anything_matching("pct create"));
        assert!(!fake.ran_anything_matching("pct start"));
        assert!(!fake.ran_anything_matching("pct exec"));
    }

    #[tokio::test]
    async fn a_tagged_guest_is_recognised_as_ours() {
        let listing = serde_json::to_string(&vec![serde_json::json!({
            "vmid": 11050, "name": "speed", "status": "running",
            "tags": OWNER_TAG
        })])
        .unwrap();
        let fake = FakeRunner::new(vec![Output::ok(listing)]);
        let pve = Proxmox::new(&fake, "pve");

        let found = pve
            .ensure_ours(11050)
            .await
            .unwrap()
            .expect("should be found");
        assert!(found.is_ours());
        assert!(found.is_running());
    }

    #[tokio::test]
    async fn an_absent_vmid_is_not_an_error() {
        let fake = FakeRunner::new(vec![Output::ok("[]")]);
        let pve = Proxmox::new(&fake, "pve");
        assert!(pve.ensure_ours(11050).await.unwrap().is_none());
    }

    #[test]
    fn create_pins_the_mac_requests_dhcp_and_tags_the_guest() {
        let fake = FakeRunner::new(vec![]);
        let pve = Proxmox::new(&fake, "pve");
        let line = pve.create_argv(&guest(), Some("/tmp/key.pub")).join(" ");

        assert!(line.contains("pct create 11050"), "{line}");
        assert!(line.contains("ip=dhcp"), "{line}");
        assert!(line.contains("hwaddr=02:5E:00:00:2B:2A"), "{line}");
        assert!(line.contains(&format!("--tags {OWNER_TAG}")), "{line}");
        assert!(line.contains("--features nesting=1,keyctl=1"), "{line}");
        assert!(line.contains("--unprivileged 1"), "{line}");
        assert!(line.contains("--ssh-public-keys /tmp/key.pub"), "{line}");
    }

    #[tokio::test]
    async fn preflight_fails_loudly_when_the_host_is_not_proxmox() {
        let fake = FakeRunner::new(vec![Output::fail(
            127,
            "bash: pveversion: command not found",
        )]);
        let pve = Proxmox::new(&fake, "pve");
        let err = pve.preflight().await.unwrap_err().to_string();
        assert!(err.contains("is this a Proxmox host"), "{err}");
    }

    #[test]
    fn guest_address_is_parsed_from_ip_json() {
        let json = r#"[{"ifname":"eth0","addr_info":[
            {"family":"inet","local":"10.0.0.50","prefixlen":24,"scope":"global"}]}]"#;
        assert_eq!(parse_ipv4_from_ip_json(json).as_deref(), Some("10.0.0.50"));
    }

    #[test]
    fn link_local_and_loopback_are_not_mistaken_for_a_lease() {
        let json = r#"[{"ifname":"lo","addr_info":[
            {"family":"inet","local":"127.0.0.1","prefixlen":8,"scope":"host"}]}]"#;
        assert_eq!(parse_ipv4_from_ip_json(json), None);
        assert_eq!(parse_ipv4_from_ip_json("[]"), None);
    }

    #[test]
    fn credentials_never_survive_into_an_error_message() {
        // Regression test for a real leak: a coturn install failure printed the
        // TURN password into the terminal, and from there into a chat
        // transcript. The rule is that a secret cannot reach an error string,
        // not that we remember to be careful at each call site.
        let cmd = "TURN_USER='speedtest' TURN_PASS='hunter2' TURN_REALM='example.com' \
                   LISTEN_IP='10.0.0.50' /opt/speedtest/install-coturn.sh";
        let safe = redact_secrets(cmd);

        assert!(!safe.contains("hunter2"), "password leaked: {safe}");
        assert!(safe.contains("TURN_PASS='***'"), "{safe}");
        // Non-secret context must survive, or the error becomes useless.
        assert!(safe.contains("TURN_USER='speedtest'"), "{safe}");
        assert!(safe.contains("LISTEN_IP='10.0.0.50'"), "{safe}");
        assert!(safe.contains("install-coturn.sh"), "{safe}");
    }

    #[test]
    fn every_sensitive_name_shape_is_masked() {
        for (name, secret) in [
            ("CF_Token", "cf-abc123"),
            ("ACME_TOKEN", "tok-xyz"),
            ("SPEEDTEST_TURN_PASS", "pw-123"),
            ("MY_SECRET", "s-456"),
            ("API_KEY", "k-789"),
            ("DB_PASSWD", "p-000"),
        ] {
            let cmd = format!("{name}='{secret}' run.sh");
            let safe = redact_secrets(&cmd);
            assert!(!safe.contains(secret), "{name} leaked: {safe}");
        }
    }

    #[test]
    fn redaction_leaves_ordinary_commands_untouched() {
        for plain in [
            "apt-get update",
            "systemctl restart coturn",
            "curl -fsS http://127.0.0.1:8080/api/health",
            "LISTEN_IP='10.0.0.1' echo hi",
        ] {
            assert_eq!(redact_secrets(plain), plain);
        }
    }

    #[test]
    fn redaction_handles_a_shell_escaped_quote_in_the_value() {
        // Shell-escaped secrets contain a '\''-style sequence; the masker must
        // not stop early and spill the remainder of the password.
        let cmd = r"TURN_PASS='aa'\''bb' next.sh";
        let safe = redact_secrets(cmd);
        assert!(!safe.contains("bb"), "leaked the tail of the value: {safe}");
    }
}
