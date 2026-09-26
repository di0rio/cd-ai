//! OS sandbox for `run_command` (SPEC §20.3, plan 017).
//!
//! - Linux: Landlock (filesystem) and, unless the command is an approved `network` one, a
//!   user+network namespace so the child has no route off the machine.
//! - macOS: `sandbox-exec` with a Seatbelt profile: writes only inside the workspace, the temp
//!   dirs and the package caches, and no network.
//! - Windows: an AppContainer, started through this same executable acting as launcher (see
//!   [`init`]). The container reaches only what was granted to it by ACL: the workspace, the
//!   package caches and, read-only, the toolchains on PATH. No network, not even loopback.
//!
//! Everywhere, the writable set leaves out what runs later *outside* the sandbox
//! (`~/.cargo/bin`, `~/.rustup`, `~/.bun/bin`, a global npm prefix): a command must not plant a
//! binary the user will execute next. A host without a working sandbox reports it unavailable:
//! FULL ACCESS stays off and write commands keep asking (D5/D6).

use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// What the UI and the policy need to know. `available` is filesystem AND network block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SandboxStatus {
    pub available: bool,
    pub filesystem: bool,
    pub network_block: bool,
    pub platform: String,
    /// Short reason in pt-BR: shown when FULL ACCESS is off.
    pub detail: String,
}

impl SandboxStatus {
    pub fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            filesystem: self.filesystem,
            network_block: self.network_block,
        }
    }
}

/// The two bits [`crate::permissions::policy`] reads. Tests pass this explicitly so they
/// never depend on the host kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SandboxCapabilities {
    pub filesystem: bool,
    pub network_block: bool,
}

impl SandboxCapabilities {
    pub fn ready(self) -> bool {
        self.filesystem && self.network_block
    }
}

/// How a single spawn should be constrained.
#[derive(Debug, Clone)]
pub struct SandboxExec {
    pub workspace: PathBuf,
    /// True only for an approved `network` command (D3).
    pub allow_network: bool,
}

/// Call first thing in `main`. On Windows, a process started as the sandbox launcher runs the
/// command and exits here, and any other process registers its own executable as that launcher;
/// without this call the Windows sandbox reports itself unavailable. No-op elsewhere.
pub fn init() {
    #[cfg(windows)]
    windows::init();
}

/// Process-wide probe, so every engine sees the same answer.
pub fn status() -> &'static SandboxStatus {
    static STATUS: OnceLock<SandboxStatus> = OnceLock::new();
    STATUS.get_or_init(probe)
}

/// Whether this process can enter a user+network namespace. Independent of Landlock TCP deny.
pub fn net_namespace_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::net_namespace_available()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

fn probe() -> SandboxStatus {
    #[cfg(target_os = "linux")]
    {
        linux::probe()
    }
    #[cfg(target_os = "macos")]
    {
        macos::probe()
    }
    #[cfg(windows)]
    {
        windows::probe()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        SandboxStatus {
            available: false,
            filesystem: false,
            network_block: false,
            platform: "other".to_string(),
            detail: "este sistema não tem sandbox no cd-ai".to_string(),
        }
    }
}

/// The command that spawns `program` under the sandbox; the caller adds the arguments, the cwd
/// and the stdio. A plain command when this host cannot isolate. An error when it can but setting
/// up the isolation failed: the command must never run "unsandboxed by accident".
pub fn command(program: &str, exec: &SandboxExec) -> io::Result<Command> {
    #[cfg(target_os = "linux")]
    {
        let mut command = Command::new(program);
        linux::constrain(&mut command, exec.clone());
        Ok(command)
    }
    #[cfg(target_os = "macos")]
    {
        Ok(macos::command(program, exec))
    }
    #[cfg(windows)]
    {
        windows::command(program, exec)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = exec;
        Ok(Command::new(program))
    }
}

/// One-time host preparation that needs an administrator (`cd-ai sandbox-setup`). Only Windows
/// has any: it lets the AppContainer list `C:\`, `C:\Users` and the other folders above the
/// profile and above `extra`. Returns what was done, one line each.
pub fn setup_host(extra: &[PathBuf]) -> Result<Vec<String>, String> {
    #[cfg(windows)]
    {
        windows::setup(extra)
    }
    #[cfg(not(windows))]
    {
        let _ = extra;
        Ok(vec!["nada a preparar neste sistema".to_string()])
    }
}

/// The AppContainer launcher command even when the probe reports the host unprepared. Only for
/// the end to end test; everything else goes through [`command`].
#[cfg(windows)]
#[doc(hidden)]
pub fn contained_command(program: &str, exec: &SandboxExec) -> io::Result<Command> {
    windows::contained_command(program, exec)
}

/// Package-manager caches a build or an install needs to write, when they exist. Only caches:
/// never a directory whose contents run later outside the sandbox.
#[cfg_attr(
    not(any(target_os = "linux", target_os = "macos", windows)),
    allow(dead_code)
)]
fn cache_roots() -> Vec<PathBuf> {
    let home = home_dir();
    let mut roots = Vec::new();
    // Cargo's download cache and the lock files it takes even for an offline build.
    if let Some(cargo) = cargo_home() {
        for name in [
            "registry",
            "git",
            ".package-cache",
            ".package-cache-mutate",
            ".global-cache",
        ] {
            roots.push(cargo.join(name));
        }
    }
    if let Some(home) = &home {
        roots.push(home.join(".bun").join("install").join("cache"));
        roots.push(home.join(".npm").join("_cacache"));
        roots.push(home.join(".npm").join("_logs"));
    }
    if cfg!(windows) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            roots.push(local.join("npm-cache"));
            roots.push(local.join("pnpm").join("store"));
            roots.push(local.join("go-build"));
            roots.push(local.join("pip").join("cache"));
            roots.push(local.join("uv").join("cache"));
        }
    } else if let Some(home) = &home {
        let (cache_dir, pnpm_store) = if cfg!(target_os = "macos") {
            (
                home.join("Library").join("Caches"),
                home.join("Library").join("pnpm").join("store"),
            )
        } else {
            (
                std::env::var_os("XDG_CACHE_HOME")
                    .filter(|dir| !dir.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".cache")),
                home.join(".local").join("share").join("pnpm").join("store"),
            )
        };
        roots.push(pnpm_store);
        // Each tool's own cache dir, but not the cache dir itself (no new one can be planted),
        // and not the caches holding programs that run later: pre-commit hooks, the package
        // managers corepack downloads.
        if let Ok(entries) = std::fs::read_dir(&cache_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if RUNS_LATER.iter().any(|skip| name == *skip) {
                    continue;
                }
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    roots.push(entry.path());
                }
            }
        }
    }
    roots.retain(|root| root.exists());
    roots
}

/// Cache directories whose contents are executed later, outside the sandbox.
const RUNS_LATER: &[&str] = &["pre-commit", "node", "pnpm"];

fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

fn cargo_home() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".cargo")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_names_the_platform() {
        let status = status();
        assert!(!status.platform.is_empty());
        assert_eq!(status.available, status.filesystem && status.network_block);
        #[cfg(target_os = "linux")]
        assert_eq!(status.platform, "linux");
        #[cfg(windows)]
        {
            assert_eq!(status.platform, "windows");
            // Unit tests never call `init`, so there is no launcher to start the container.
            assert!(!status.available);
        }
    }

    #[test]
    fn caches_never_include_what_runs_outside_the_sandbox() {
        let roots = cache_roots();
        let Some(home) = home_dir() else {
            return;
        };
        let cargo = cargo_home().unwrap();
        for forbidden in [
            cargo.join("bin"),
            cargo.join("config.toml"),
            home.join(".rustup"),
            home.join(".bun").join("bin"),
            home.join(".npm").join("_npx"),
            home.join(".local").join("share").join("pnpm"),
            home.join(".local").join("bin"),
            home.join(".cache").join("pre-commit"),
            home.join(".cache").join("node").join("corepack"),
        ] {
            assert!(
                !roots.iter().any(|root| forbidden.starts_with(root)),
                "{forbidden:?} would be writable through {roots:?}"
            );
        }
    }
}
