//! Tier 1 — the provisioning flow against a scripted hypervisor.
//!
//! Two properties are worth more than everything else in this crate, and both
//! are asserted here rather than discovered on a live node:
//!
//! 1. A second run changes nothing (the milestone's stated done-when).
//! 2. A guest we did not create is never touched, under any circumstances.

use std::path::Path;

use speedtest_provision::config::{Config, Guest, OWNER_TAG};
use speedtest_provision::proxmox::Proxmox;
use speedtest_provision::runner::{FakeRunner, Output};
use speedtest_provision::setup;

fn guest() -> Guest {
    toml::from_str(
        r#"
vmid = 11050
hostname = "speed"
ostemplate = "local:vztmpl/debian-13-standard_13.0-1_amd64.tar.zst"
cores = 4
memory_mb = 4096
rootfs = "local-lvm:16"
bridge = "vmbr0"
"#,
    )
    .expect("guest fixture parses")
}

fn listing(entries: serde_json::Value) -> String {
    entries.to_string()
}

fn ours() -> String {
    listing(serde_json::json!([{
        "vmid": 11050, "name": "speed", "status": "running", "tags": OWNER_TAG
    }]))
}

// --- the ownership guard ----------------------------------------------------

#[tokio::test]
async fn a_guest_we_did_not_create_is_never_touched() {
    // Stephen's rule: provisioning must never touch a Proxmox guest it did not
    // create. A VMID collision is exactly how that would happen, so the guard
    // has to hold before any mutating command is even considered.
    let fake = FakeRunner::new(vec![
        Output::ok("pve-manager/8.2.4"),
        Output::ok(listing(serde_json::json!([{
            "vmid": 11050, "name": "prod-db", "status": "running", "tags": "production;database"
        }]))),
    ]);
    let pve = Proxmox::new(&fake, "pve");

    pve.preflight().await.unwrap();
    let err = pve.ensure_ours(11050).await.unwrap_err().to_string();

    assert!(err.contains("REFUSING TO CONTINUE"), "{err}");
    assert!(
        err.contains("prod-db"),
        "the message must name the guest: {err}"
    );
    assert!(err.contains("nothing has been changed"), "{err}");

    // Nothing destructive was attempted — not creation, not start, not exec.
    for forbidden in [
        "pct create",
        "pct start",
        "pct exec",
        "pct push",
        "pct destroy",
    ] {
        assert!(
            !fake.ran_anything_matching(forbidden),
            "ran `{forbidden}` against a guest that is not ours: {:?}",
            fake.command_lines()
        );
    }
}

#[tokio::test]
async fn an_untagged_guest_is_refused_even_when_the_name_matches() {
    // A guest called "speed" that we did not tag is still not ours. Matching on
    // anything but the ownership tag would be a trap.
    let fake = FakeRunner::new(vec![Output::ok(listing(serde_json::json!([{
        "vmid": 11050, "name": "speed", "status": "stopped", "tags": ""
    }])))]);
    let pve = Proxmox::new(&fake, "pve");
    assert!(pve.ensure_ours(11050).await.is_err());
}

#[tokio::test]
async fn a_lookalike_tag_does_not_grant_ownership() {
    let fake = FakeRunner::new(vec![Output::ok(listing(serde_json::json!([{
        "vmid": 11050, "name": "speed", "status": "running",
        "tags": "not-speedtest-provisioned"
    }])))]);
    let pve = Proxmox::new(&fake, "pve");
    assert!(pve.ensure_ours(11050).await.is_err());
}

// --- idempotency ------------------------------------------------------------

#[tokio::test]
async fn a_second_run_creates_nothing() {
    // The milestone's done-when: running again changes nothing. Everything the
    // fake reports is "already in the desired state".
    let fake = FakeRunner::new(vec![
        Output::ok("pve-manager/8.2.4"), // preflight
        Output::ok(ours()),              // ensure_ours -> exists, ours
        Output::ok(ours()),              // find_lxc -> already running
    ]);
    let pve = Proxmox::new(&fake, "pve");

    pve.preflight().await.unwrap();
    let existing = pve.ensure_ours(11050).await.unwrap();
    assert!(
        existing.is_some(),
        "the guest should be recognised as existing"
    );

    let current = pve.find_lxc(11050).await.unwrap().unwrap();
    assert!(current.is_running());

    assert!(
        !fake.ran_anything_matching("pct create"),
        "an existing guest must not be recreated: {:?}",
        fake.command_lines()
    );
    assert!(
        !fake.ran_anything_matching("pct start"),
        "a running guest must not be restarted: {:?}",
        fake.command_lines()
    );
}

#[tokio::test]
async fn docker_is_not_reinstalled_when_it_is_already_working() {
    // The probe succeeds, so the whole install must be skipped. Reinstalling
    // would restart the daemon and take the service down for no reason.
    let fake = FakeRunner::new(vec![Output::ok("")]); // `command -v docker` succeeds
    let pve = Proxmox::new(&fake, "pve");

    setup::install_docker(&pve, 11050).await.unwrap();

    assert_eq!(
        fake.calls.lock().unwrap().len(),
        1,
        "{:?}",
        fake.command_lines()
    );
    assert!(!fake.ran_anything_matching("download.docker.com"));
    assert!(!fake.ran_anything_matching("apt-get install"));
}

#[tokio::test]
async fn docker_is_installed_when_it_is_absent() {
    let fake = FakeRunner::new(vec![
        Output::fail(1, "not found"), // probe fails
        Output::ok(""),               // install script
        Output::ok("Docker Compose version v2.29.0"),
    ]);
    let pve = Proxmox::new(&fake, "pve");

    setup::install_docker(&pve, 11050).await.unwrap();

    assert!(fake.ran_anything_matching("download.docker.com"));
    // And the install is proved to work rather than assumed.
    assert!(fake.ran_anything_matching("docker compose version"));
}

// --- the certificate-renewal check -----------------------------------------

#[tokio::test]
async fn provisioning_fails_when_renewal_is_not_actually_scheduled() {
    // This is the predecessor host's exact failure: acme.sh installed, a valid
    // certificate, and no cron entry at all — so nothing would ever renew it.
    // The install script there reported success. We check the end state.
    let fake = FakeRunner::new(vec![
        Output::ok("# m h dom mon dow command\n"), // crontab: no acme entry
    ]);
    let pve = Proxmox::new(&fake, "pve");

    let err = setup::verify_renewal_scheduled(&pve, 11050, "/etc/speedtest/tls")
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("NOT scheduled"), "{err}");
    assert!(
        err.contains("predecessor host"),
        "the message should explain why we check: {err}"
    );
}

#[tokio::test]
async fn renewal_check_passes_when_the_cron_entry_and_certificate_are_both_present() {
    let fake = FakeRunner::new(vec![
        Output::ok("30 0 * * 0 /root/.acme.sh/acme.sh --cron --home /root/.acme.sh\n"),
        Output::ok("subject=CN=*.example.com\nnotAfter=Nov 22 15:14:20 2026 GMT\n"),
    ]);
    let pve = Proxmox::new(&fake, "pve");
    setup::verify_renewal_scheduled(&pve, 11050, "/etc/speedtest/tls")
        .await
        .expect("a scheduled renewal with a readable certificate should pass");
}

#[tokio::test]
async fn a_renewal_that_never_reloads_the_service_is_flagged() {
    // Renewing the certificate but never restarting the service means the old
    // material keeps being served until someone notices — 90 days later.
    let fake = FakeRunner::new(vec![
        Output::ok(
            "30 0 * * 0 /root/.acme.sh/acme.sh --cron
",
        ),
        Output::ok(
            "subject=CN=*.example.com
notAfter=Nov 22 15:14:20 2026 GMT
",
        ),
        Output::ok(""), // no Le_ReloadCmd configured
    ]);
    let pve = Proxmox::new(&fake, "pve");

    // Still a pass — the certificate does renew — but the hook is probed for.
    setup::verify_renewal_scheduled(&pve, 11050, "/etc/speedtest/tls")
        .await
        .expect("a missing reload hook is a warning, not a hard failure");
    assert!(
        fake.ran_anything_matching("Le_ReloadCmd"),
        "the reload hook should be checked: {:?}",
        fake.command_lines()
    );
}

#[tokio::test]
async fn renewal_check_fails_when_the_certificate_is_missing() {
    let fake = FakeRunner::new(vec![
        Output::ok("30 0 * * 0 /root/.acme.sh/acme.sh --cron\n"),
        Output::fail(1, "No such file or directory"),
    ]);
    let pve = Proxmox::new(&fake, "pve");
    assert!(
        setup::verify_renewal_scheduled(&pve, 11050, "/etc/speedtest/tls")
            .await
            .is_err()
    );
}

// --- health -----------------------------------------------------------------

#[tokio::test]
async fn health_check_surfaces_container_logs_when_it_never_comes_up() {
    let fake = FakeRunner::new(vec![
        Output::fail(7, "connection refused"),
        Output::ok("speedtest | panicked at config.rs: unknown profile"),
    ]);
    let pve = Proxmox::new(&fake, "pve");

    let err = setup::wait_for_health(&pve, 11050, 1)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("did not become healthy"), "{err}");
    assert!(
        err.contains("unknown profile"),
        "logs should be included: {err}"
    );
}

#[tokio::test]
async fn health_check_passes_as_soon_as_the_service_answers() {
    let fake = FakeRunner::new(vec![Output::ok("ok")]);
    let pve = Proxmox::new(&fake, "pve");
    setup::wait_for_health(&pve, 11050, 5).await.unwrap();
    assert_eq!(fake.calls.lock().unwrap().len(), 1);
}

// --- shape of what gets created --------------------------------------------

#[tokio::test]
async fn creation_pins_the_mac_and_tags_the_guest_as_ours() {
    let fake = FakeRunner::new(vec![Output::ok("")]);
    let pve = Proxmox::new(&fake, "pve");
    pve.create(&guest(), None).await.unwrap();

    let line = fake.command_lines().join(" ");
    assert!(line.contains("pct create 11050"), "{line}");
    // DHCP with a pinned MAC: the reservation must survive a rebuild.
    assert!(line.contains("ip=dhcp"), "{line}");
    assert!(line.contains("hwaddr=02:5E:00:00:2B:2A"), "{line}");
    // Without the tag, the next run would refuse to touch what we just made.
    assert!(line.contains(OWNER_TAG), "{line}");
    assert!(line.contains("nesting=1"), "docker needs nesting: {line}");
}

#[tokio::test]
async fn the_derived_mac_is_stable_across_runs() {
    // The entire point of pinning: a rebuild must land on the same address.
    let a = guest().effective_mac();
    let b = guest().effective_mac();
    assert_eq!(a, b);
    assert_eq!(a, Guest::default_mac(11050));
}

// --- the shipped config -----------------------------------------------------

#[test]
fn the_committed_provision_toml_is_valid_and_carries_no_secrets() {
    let path = Path::new("provision.toml");
    let cfg = Config::load(path).expect("shipped provision.toml should parse and validate");

    assert_eq!(cfg.guest.vmid, 11050);
    assert!(cfg.guest.features.contains("nesting=1"));

    // Look at assignments rather than prose. Substring matching flags the word
    // "secret" in a comment that exists precisely to say secrets do not belong
    // here, which is the opposite of a finding.
    let raw = std::fs::read_to_string(path).unwrap();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_lowercase();
        let value = value.trim().trim_matches('"').trim_matches('\'');

        let sensitive = [
            "token",
            "password",
            "passwd",
            "secret",
            "credential",
            "pass",
        ]
        .iter()
        .any(|s| key.contains(s));
        assert!(
            !(sensitive && !value.is_empty()),
            "provision.toml assigns a value to a sensitive-looking key: {line}"
        );
    }
    assert!(
        !raw.contains("BEGIN") || !raw.contains("PRIVATE KEY"),
        "provision.toml contains an embedded private key"
    );
}
