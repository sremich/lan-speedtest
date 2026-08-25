//! Provisioning configuration.
//!
//! Everything non-secret lives in a TOML file that is committed; the API token
//! arrives through the environment and is never written anywhere.

use std::path::Path;

use serde::Deserialize;

/// Tag applied to every guest this tool creates.
///
/// It is also the ownership marker: the tool refuses to modify any existing
/// guest that does not carry it. This is the guard behind the project rule
/// that provisioning must never touch a guest it did not create.
pub const OWNER_TAG: &str = "speedtest-provisioned";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub proxmox: Proxmox,
    pub guest: Guest,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proxmox {
    /// Address of the node to SSH to.
    pub ssh_host: String,
    /// SSH user. Needs to be able to run `pct` and `pvesh`, so in practice root.
    #[serde(default = "default_ssh_user")]
    pub ssh_user: String,
    /// Optional explicit identity file; otherwise the agent/default key is used.
    #[serde(default)]
    pub ssh_identity: Option<String>,
    /// Node name as Proxmox knows it, which is not necessarily the address.
    pub node: String,
}

fn default_ssh_user() -> String {
    "root".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guest {
    pub vmid: u32,
    pub hostname: String,
    /// Container template, as `storage:vztmpl/filename`.
    pub ostemplate: String,
    pub cores: u32,
    /// Megabytes. The predecessor ran on 512 MB, which is far too little for a
    /// backend expected to saturate 10 GbE.
    pub memory_mb: u32,
    #[serde(default)]
    pub swap_mb: u32,
    /// Root filesystem, as `storage:size-in-gib`.
    pub rootfs: String,
    pub bridge: String,
    #[serde(default)]
    pub vlan_tag: Option<u32>,
    /// Pinned MAC address.
    ///
    /// A freshly created container is assigned a random MAC, so a DHCP
    /// reservation made against one build would not survive a rebuild — which
    /// defeats the point of reproducible provisioning. Pinning it here makes
    /// the reservation stable. Derived from the VMID by `default_mac` if unset.
    #[serde(default)]
    pub mac: Option<String>,
    #[serde(default = "default_true")]
    pub unprivileged: bool,
    /// `nesting=1` is required for Docker inside an unprivileged container.
    #[serde(default = "default_features")]
    pub features: String,
    #[serde(default = "default_true")]
    pub onboot: bool,
    /// Extra tags alongside the ownership tag.
    #[serde(default)]
    pub extra_tags: Vec<String>,
    /// Which `[profiles.*]` in config/speedtest.toml the deployed service runs.
    #[serde(default = "default_measurement_profile")]
    pub measurement_profile: String,
    /// What the deployed service calls itself in the browser.
    ///
    /// Unset leaves the shipped default in place. Written into the guest's
    /// `.env` as `SPEEDTEST_SITE_NAME`, so changing it here and re-running
    /// `apply` renames the site.
    #[serde(default)]
    pub site_name: Option<String>,
    /// Names clients in the history from their PTR records.
    ///
    /// Off unless asked for, and range-restricted by the shipped config even
    /// when on — see docs/wiki/Client-Identity.md for why that restriction is
    /// not optional. Written into the guest's `.env`, so switching it is a
    /// re-run of `apply` rather than a rebuild.
    #[serde(default)]
    pub reverse_dns: bool,
    /// `host:port` for the resolver to ask. Unset lets the service read the
    /// container's own `/etc/resolv.conf`, which is usually right; set it when
    /// the LAN's PTR zone lives somewhere the container does not point at.
    #[serde(default)]
    pub dns_resolver: Option<String>,
    /// Public key installed for root on the guest, so it is reachable by SSH.
    ///
    /// Installed on every run rather than only at creation: `pct create` takes
    /// a key, but a guest that already exists never goes through creation
    /// again, and access should survive a re-run either way.
    #[serde(default = "default_authorized_key")]
    pub ssh_authorized_key: String,
}

fn default_true() -> bool {
    true
}
fn default_features() -> String {
    "nesting=1,keyctl=1".to_string()
}
fn default_measurement_profile() -> String {
    "lan-1g".to_string()
}
fn default_authorized_key() -> String {
    "~/.ssh/id_ed25519.pub".to_string()
}

impl Guest {
    /// A stable, locally-administered MAC derived from the VMID.
    ///
    /// `02:` marks it locally administered, so it cannot collide with a
    /// manufacturer address. The remaining octets encode the VMID, which keeps
    /// the value reproducible without a lookup table.
    pub fn default_mac(vmid: u32) -> String {
        let b = vmid.to_be_bytes();
        format!("02:5E:{:02X}:{:02X}:{:02X}:{:02X}", b[0], b[1], b[2], b[3])
    }

    pub fn effective_mac(&self) -> String {
        self.mac
            .clone()
            .unwrap_or_else(|| Self::default_mac(self.vmid))
    }

    /// The `net0` value Proxmox expects, as a comma-separated parameter list.
    pub fn net0(&self) -> String {
        let mut s = format!(
            "name=eth0,bridge={},hwaddr={},ip=dhcp",
            self.bridge,
            self.effective_mac()
        );
        if let Some(tag) = self.vlan_tag {
            s.push_str(&format!(",tag={tag}"));
        }
        s
    }

    /// Tags as Proxmox stores them: semicolon-separated, ownership tag first.
    pub fn tags(&self) -> String {
        let mut tags = vec![OWNER_TAG.to_string()];
        tags.extend(self.extra_tags.iter().cloned());
        tags.join(";")
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("could not read {}: {e}", path.display()))?;
        let cfg: Config = toml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("could not parse {}: {e}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.guest.vmid < 100 {
            anyhow::bail!("vmid {} is below Proxmox's minimum of 100", self.guest.vmid);
        }
        if let Some(mac) = &self.guest.mac {
            let ok = mac.split(':').count() == 6
                && mac
                    .split(':')
                    .all(|o| o.len() == 2 && o.chars().all(|c| c.is_ascii_hexdigit()));
            if !ok {
                anyhow::bail!("mac {mac:?} is not six colon-separated hex octets");
            }
        }
        if !self.guest.ostemplate.contains(":vztmpl/") {
            anyhow::bail!(
                "ostemplate {:?} should look like 'storage:vztmpl/filename'",
                self.guest.ostemplate
            );
        }
        // The name is written into a compose `env_file`, which is parsed
        // line by line with no quoting. A newline would inject an unrelated
        // variable; a '#' can start a comment. Reject both here rather than
        // producing a guest that boots with a silently truncated name.
        if let Some(name) = &self.guest.site_name {
            if name.trim().is_empty() {
                anyhow::bail!("site_name is set but blank — remove it to keep the default");
            }
            if let Some(bad) = name.chars().find(|c| matches!(c, '\n' | '\r' | '#')) {
                anyhow::bail!(
                    "site_name {name:?} contains {bad:?}, which a compose env_file cannot carry"
                );
            }
        }
        if self.guest.unprivileged && !self.guest.features.contains("nesting=1") {
            anyhow::bail!(
                "features {:?} lacks nesting=1, which Docker needs in an unprivileged container",
                self.guest.features
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            features: default_features(),
            onboot: true,
            extra_tags: vec![],
            measurement_profile: default_measurement_profile(),
            site_name: None,
            reverse_dns: false,
            dns_resolver: None,
            ssh_authorized_key: default_authorized_key(),
        }
    }

    #[test]
    fn derived_mac_is_stable_and_locally_administered() {
        let a = Guest::default_mac(11050);
        let b = Guest::default_mac(11050);
        assert_eq!(a, b, "the same vmid must always yield the same MAC");
        assert_ne!(a, Guest::default_mac(11051));

        // Locally administered (bit 1 of the first octet set) and unicast
        // (bit 0 clear), so it cannot collide with a manufacturer address.
        let first = u8::from_str_radix(a.split(':').next().unwrap(), 16).unwrap();
        assert_eq!(first & 0b10, 0b10, "not locally administered: {a}");
        assert_eq!(first & 0b1, 0, "not unicast: {a}");
    }

    #[test]
    fn net0_pins_the_mac_and_requests_dhcp() {
        // Both halves matter: DHCP is what Stephen asked for, and the pinned
        // MAC is what makes the reservation survive a rebuild.
        let g = guest();
        let net0 = g.net0();
        assert!(net0.contains("ip=dhcp"), "{net0}");
        assert!(
            net0.contains(&format!("hwaddr={}", g.effective_mac())),
            "{net0}"
        );
        assert!(net0.contains("bridge=vmbr0"), "{net0}");
        assert!(
            !net0.contains("tag="),
            "no VLAN configured, so no tag: {net0}"
        );
    }

    #[test]
    fn net0_carries_the_vlan_tag_when_one_is_configured() {
        let mut g = guest();
        g.vlan_tag = Some(11);
        assert!(g.net0().contains(",tag=11"), "{}", g.net0());
    }

    #[test]
    fn tags_always_include_the_ownership_marker_first() {
        let mut g = guest();
        assert_eq!(g.tags(), OWNER_TAG);
        g.extra_tags = vec!["lan".into(), "speedtest".into()];
        assert_eq!(g.tags(), format!("{OWNER_TAG};lan;speedtest"));
    }

    #[test]
    fn an_explicit_mac_overrides_the_derived_one() {
        let mut g = guest();
        g.mac = Some("AA:BB:CC:DD:EE:FF".into());
        assert_eq!(g.effective_mac(), "AA:BB:CC:DD:EE:FF");
    }

    fn cfg_with(guest: Guest) -> Config {
        Config {
            proxmox: Proxmox {
                ssh_host: "10.0.0.1".into(),
                ssh_user: "root".into(),
                ssh_identity: None,
                node: "pve".into(),
            },
            guest,
        }
    }

    #[test]
    fn validation_rejects_a_site_name_a_compose_env_file_cannot_carry() {
        // The name lands in an env_file, which has no quoting: a newline would
        // inject an unrelated variable and a '#' can start a comment, so the
        // guest would come up with a name nobody asked for.
        for bad in ["Rack\nSPEEDTEST_TURN_ENABLED=false", "Rack #1", "   "] {
            let mut g = guest();
            g.site_name = Some(bad.into());
            assert!(
                cfg_with(g).validate().is_err(),
                "site_name {bad:?} should be rejected"
            );
        }

        let mut g = guest();
        g.site_name = Some("Rack Room Speed Test".into());
        assert!(cfg_with(g).validate().is_ok());
    }

    #[test]
    fn validation_rejects_a_malformed_mac() {
        let mut g = guest();
        g.mac = Some("zz:bb:cc:dd:ee:ff".into());
        assert!(cfg_with(g).validate().is_err());
    }

    #[test]
    fn validation_rejects_docker_without_nesting() {
        // Silently losing nesting would produce a guest where the container
        // runtime simply does not work, discovered much later.
        let mut g = guest();
        g.features = "keyctl=1".into();
        assert!(cfg_with(g).validate().is_err());
    }

    #[test]
    fn validation_rejects_an_ostemplate_that_is_not_a_template_reference() {
        let mut g = guest();
        g.ostemplate = "debian-13.tar.zst".into();
        assert!(cfg_with(g).validate().is_err());
    }

    #[test]
    fn validation_accepts_the_shipped_shape() {
        assert!(cfg_with(guest()).validate().is_ok());
    }
}
