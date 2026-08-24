//! Command execution, behind a trait.
//!
//! Everything this tool does to the hypervisor is a command run over SSH. That
//! is abstracted here for two reasons: the whole provisioning flow can then be
//! unit-tested against a scripted fake with no Proxmox anywhere near it, and
//! the one place that actually touches a real system stays small enough to
//! read in full.

use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::Mutex;

use anyhow::{bail, Context};
use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(stdout: impl Into<String>) -> Self {
        Self {
            status: 0,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    pub fn fail(status: i32, stderr: impl Into<String>) -> Self {
        Self {
            status,
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }

    pub fn succeeded(&self) -> bool {
        self.status == 0
    }
}

#[async_trait]
pub trait Runner: Send + Sync {
    /// Runs a command on the hypervisor and returns its result.
    ///
    /// A non-zero exit is data, not an error — callers routinely probe for
    /// things that may legitimately be absent. Transport failures are errors.
    async fn run(&self, argv: &[String]) -> anyhow::Result<Output>;

    /// Copies a local file to a path on the hypervisor.
    async fn upload(&self, local: &std::path::Path, remote: &str) -> anyhow::Result<()>;
}

/// Convenience for building an argv without ceremony at every call site.
pub fn argv<const N: usize>(parts: [&str; N]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Runs commands on the Proxmox node over SSH.
pub struct SshRunner {
    target: String,
    /// Extra `ssh` options, e.g. an explicit identity file.
    opts: Vec<String>,
}

impl SshRunner {
    pub fn new(user: &str, host: &str, identity: Option<&str>) -> Self {
        let mut opts = vec![
            // Fail rather than hang waiting for a passphrase or a password:
            // this tool is meant to be runnable unattended.
            "-o".into(),
            "BatchMode=yes".into(),
            "-o".into(),
            "ConnectTimeout=15".into(),
            // The node's host key is pinned on first use rather than blindly
            // accepted every time.
            "-o".into(),
            "StrictHostKeyChecking=accept-new".into(),
        ];
        if let Some(id) = identity {
            opts.push("-i".into());
            opts.push(id.to_string());
        }
        Self {
            target: format!("{user}@{host}"),
            opts,
        }
    }
}

#[async_trait]
impl Runner for SshRunner {
    async fn run(&self, argv: &[String]) -> anyhow::Result<Output> {
        // Pass the command as a single quoted string so the remote shell sees
        // exactly the arguments intended, spaces and all.
        let remote = argv
            .iter()
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" ");

        tracing::debug!("ssh {} -- {remote}", self.target);

        let out = tokio::process::Command::new("ssh")
            .args(&self.opts)
            .arg(&self.target)
            .arg("--")
            .arg(&remote)
            .stdin(Stdio::null())
            .output()
            .await
            .with_context(|| format!("running ssh to {}", self.target))?;

        Ok(Output {
            status: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    async fn upload(&self, local: &std::path::Path, remote: &str) -> anyhow::Result<()> {
        let out = tokio::process::Command::new("scp")
            .args(&self.opts)
            .arg(local)
            .arg(format!("{}:{remote}", self.target))
            .stdin(Stdio::null())
            .output()
            .await
            .with_context(|| format!("scp {} to {}", local.display(), self.target))?;

        if !out.status.success() {
            bail!(
                "scp {} -> {remote} failed: {}",
                local.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }
}

/// Single-quotes an argument for a POSIX remote shell.
fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=,@+".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A scripted `Runner` for tests.
///
/// Records every command it is given and replies from a queue, so a test can
/// assert both on what the tool decided to do and on what it did *not* do.
pub struct FakeRunner {
    replies: Mutex<VecDeque<Output>>,
    pub calls: Mutex<Vec<Vec<String>>>,
    pub uploads: Mutex<Vec<(String, String)>>,
    default: Output,
}

impl FakeRunner {
    pub fn new(replies: Vec<Output>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            calls: Mutex::new(Vec::new()),
            uploads: Mutex::new(Vec::new()),
            default: Output::ok(""),
        }
    }

    /// Every command the tool ran, joined for readable assertions.
    pub fn command_lines(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.join(" "))
            .collect()
    }

    pub fn ran_anything_matching(&self, needle: &str) -> bool {
        self.command_lines().iter().any(|c| c.contains(needle))
    }
}

#[async_trait]
impl Runner for FakeRunner {
    async fn run(&self, argv: &[String]) -> anyhow::Result<Output> {
        self.calls.lock().unwrap().push(argv.to_vec());
        let next = self.replies.lock().unwrap().pop_front();
        Ok(next.unwrap_or_else(|| self.default.clone()))
    }

    async fn upload(&self, local: &std::path::Path, remote: &str) -> anyhow::Result<()> {
        self.uploads
            .lock()
            .unwrap()
            .push((local.display().to_string(), remote.to_string()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_arguments_are_left_alone() {
        assert_eq!(shell_quote("pct"), "pct");
        assert_eq!(shell_quote("11050"), "11050");
        assert_eq!(shell_quote("local-lvm:16"), "local-lvm:16");
        assert_eq!(
            shell_quote("name=eth0,bridge=vmbr0"),
            "name=eth0,bridge=vmbr0"
        );
    }

    #[test]
    fn arguments_with_shell_metacharacters_are_quoted() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("a;rm -rf /"), "'a;rm -rf /'");
        assert_eq!(shell_quote("$(whoami)"), "'$(whoami)'");
        assert_eq!(shell_quote(""), "''");
    }

    /// Feeds a quoted value through a real shell and returns what it received.
    ///
    /// Pattern-matching the escaped text makes a poor test: correctly escaped
    /// output legitimately contains sequences that look dangerous — `'; rm -rf
    /// / #` becomes `''\''; rm -rf / #'`, which a shell reads back as one
    /// harmless literal. What actually matters is the round trip, so ask a
    /// shell rather than guessing.
    #[cfg(unix)]
    fn roundtrip_through_shell(s: &str) -> String {
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("printf '%s' {}", shell_quote(s)))
            .output()
            .expect("running /bin/sh");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[cfg(unix)]
    #[test]
    fn hostile_values_reach_the_hypervisor_unchanged() {
        // The failure this guards is a config value closing the quote and
        // executing as a command on the hypervisor.
        for hostile in [
            "it's",
            "'; rm -rf / #",
            "$(whoami)",
            "`id`",
            "semi;colon && chain || other",
            "back\\slash",
            "a\tb",
            "'",
            "",
        ] {
            assert_eq!(
                roundtrip_through_shell(hostile),
                hostile,
                "value was altered or interpreted by the shell: {hostile:?}"
            );
        }
    }

    #[tokio::test]
    async fn the_fake_records_calls_and_replays_replies() {
        let fake = FakeRunner::new(vec![Output::ok("first"), Output::fail(2, "nope")]);
        assert_eq!(fake.run(&argv(["a", "b"])).await.unwrap().stdout, "first");
        let second = fake.run(&argv(["c"])).await.unwrap();
        assert_eq!(second.status, 2);
        // Exhausted queue falls back to a benign success.
        assert!(fake.run(&argv(["d"])).await.unwrap().succeeded());

        assert_eq!(fake.command_lines(), vec!["a b", "c", "d"]);
        assert!(fake.ran_anything_matching("a b"));
        assert!(!fake.ran_anything_matching("zzz"));
    }
}
