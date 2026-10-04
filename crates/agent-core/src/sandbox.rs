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
//!
//! The workspace is writable, but not all of it: `.git/hooks` and `.git/config` name programs git
//! runs on its own, outside the sandbox, so they are read-only (`git_protected_paths`):
//! - Linux: bind mounts in the child's own mount namespace, which needs the user namespace the
//!   network isolation creates. On a Landlock-only host, and for an approved `network` command,
//!   they stay writable (Landlock cannot carve a folder out of a writable one).
//! - macOS: `deny file-write*` rules after the allow, for every command.
//! - Windows: the container's ACL on those paths is cut loose from the workspace's and gives it
//!   read and execute only. Deny entries do not work for an AppContainer SID; see `protect_git`.
//!
//! Credentials under the home directory (`secret_home_paths`: `~/.ssh`, `~/.aws`, `~/.npmrc`, ...)
//! are hidden from a command that has no network: an empty mount over them on Linux (same
//! condition as above), `deny file-read*` on macOS, never granted on Windows. An approved `network`
//! command runs with them visible, since `git push` over ssh or `npm publish` need them.
//!
//! The environment of the child is an allowlist (`scrub_env`), not the app's: no API tokens.
//! `argv` that names `.git` never reaches the sandbox as an automatic command: it asks first
//! (`tools::command::escalate_for_paths`), which is what covers the paths no mount can.

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
///
/// The child starts with an allowlisted environment, not the app's: see [`scrub_env`].
pub fn command(program: &str, exec: &SandboxExec) -> io::Result<Command> {
    let mut command = platform_command(program, exec)?;
    scrub_env(&mut command, exec.allow_network);
    Ok(command)
}

fn platform_command(program: &str, exec: &SandboxExec) -> io::Result<Command> {
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

/// Variables a command may inherit: where things are (PATH, home, temp, the toolchains), the
/// locale and terminal, and what Windows needs to start a process at all. Everything else stays
/// behind: API tokens (`GH_TOKEN`, `NPM_TOKEN`, `AWS_*`, `OPENAI_API_KEY`), `SSH_AUTH_SOCK`, and
/// the variables that make a tool run a program (`NODE_OPTIONS`, `LD_PRELOAD`, `RUSTC_WRAPPER`,
/// `GIT_SSH_COMMAND`). A command that needs one has to be run by the user.
const ENV_ALLOWED: &[&str] = &[
    "PATH",
    "HOME",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "USER",
    "USERNAME",
    "USERDOMAIN",
    "LOGNAME",
    "TEMP",
    "TMP",
    "TMPDIR",
    "LANG",
    "LANGUAGE",
    "TZ",
    "TERM",
    "COLORTERM",
    "NO_COLOR",
    "SHELL",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "CARGO_TARGET_DIR",
    "RUST_BACKTRACE",
    "GOPATH",
    "GOROOT",
    "JAVA_HOME",
    "BUN_INSTALL",
    // Windows: without these, processes (and the C runtime) do not start or find their DLLs.
    "SYSTEMROOT",
    "WINDIR",
    "SYSTEMDRIVE",
    "COMSPEC",
    "PATHEXT",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "PROGRAMDATA",
    "COMMONPROGRAMFILES",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "OS",
];

/// Added for an approved `network` command: how to reach the network, not what to sign in with.
const ENV_NETWORK: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "CURL_CA_BUNDLE",
    "REQUESTS_CA_BUNDLE",
    "NODE_EXTRA_CA_CERTS",
    "SSH_AUTH_SOCK",
];

/// Whether a variable of this name reaches the command. Case-insensitive: Windows names are.
fn env_allowed(name: &str, allow_network: bool) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.starts_with("LC_")
        || upper.starts_with("XDG_")
        || ENV_ALLOWED.contains(&upper.as_str())
        || (allow_network && ENV_NETWORK.contains(&upper.as_str()))
}

/// Replaces the inherited environment with the allowlisted part of it.
fn scrub_env(command: &mut Command, allow_network: bool) {
    scrub_env_from(command, std::env::vars_os(), allow_network);
}

fn scrub_env_from(
    command: &mut Command,
    vars: impl Iterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    allow_network: bool,
) {
    command.env_clear();
    for (name, value) in vars {
        if name
            .to_str()
            .is_some_and(|name| env_allowed(name, allow_network))
        {
            command.env(name, value);
        }
    }
}

/// Credentials under the home directory. A sandboxed command has no business reading them: they
/// are read-denied where the platform can (Linux with user namespaces, macOS); on Windows the
/// container was never granted them.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn secret_home_paths() -> Vec<PathBuf> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    vec![
        home.join(".ssh"),
        home.join(".aws"),
        home.join(".config").join("gcloud"),
        home.join(".docker").join("config.json"),
        home.join(".npmrc"),
        home.join(".git-credentials"),
    ]
}

/// `.git/hooks` and `.git/config` of the workspace, which name programs git runs on its own (a
/// hook on `git commit`, `core.fsmonitor` on `git status`). They stay read-only for a sandboxed
/// command, which otherwise has the whole workspace writable. Only a `.git` directory counts: in a
/// linked worktree `.git` is a file, and the real one lives outside the workspace and the sandbox.
pub(crate) fn git_protected_paths(workspace: &std::path::Path) -> Vec<PathBuf> {
    let git = workspace.join(".git");
    if !git.is_dir() {
        return Vec::new();
    }
    vec![git.join("hooks"), git.join("config")]
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
    if let Some(cargo) = cargo_home() {
        roots.extend(cargo_cache_roots(&cargo));
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

/// Cargo's download cache and the lock files it takes even for an offline build. Not
/// `registry/src` and `git/checkouts`, where the unpacked sources of every dependency live: every
/// later build, outside the sandbox too, compiles (and runs `build.rs` from) what is there. A
/// dependency never built before cannot be unpacked inside the sandbox, so its first build fails
/// here and has to be run by the user.
#[cfg_attr(
    not(any(target_os = "linux", target_os = "macos", windows, test)),
    allow(dead_code)
)]
fn cargo_cache_roots(cargo: &std::path::Path) -> Vec<PathBuf> {
    [
        "registry/index",
        "registry/cache",
        "git/db",
        ".package-cache",
        ".package-cache-mutate",
        ".global-cache",
    ]
    .iter()
    .map(|name| cargo.join(name))
    .collect()
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
            cargo.join("registry").join("src"),
            cargo.join("git").join("checkouts"),
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

    #[test]
    fn cargo_caches_are_listed_by_name_not_as_a_whole() {
        // The unpacked sources are what a later build compiles: never writable by a sandboxed
        // command, even when the download caches next to them are.
        let cargo = PathBuf::from("cargo-home");
        let roots = cargo_cache_roots(&cargo);
        assert!(roots.contains(&cargo.join("registry/index")), "{roots:?}");
        assert!(roots.contains(&cargo.join("registry/cache")), "{roots:?}");
        assert!(roots.contains(&cargo.join("git/db")), "{roots:?}");
        assert!(!roots.contains(&cargo.join("registry")), "{roots:?}");
        assert!(!roots.contains(&cargo.join("registry/src")), "{roots:?}");
        assert!(!roots.contains(&cargo.join("git")), "{roots:?}");
        assert!(!roots.contains(&cargo.join("git/checkouts")), "{roots:?}");
    }

    #[test]
    fn env_allowlist_keeps_the_toolchain_and_drops_secrets() {
        for name in [
            "PATH",
            "Path",
            "HOME",
            "USERPROFILE",
            "TEMP",
            "TMPDIR",
            "LANG",
            "LC_ALL",
            "SystemRoot",
            "windir",
            "ComSpec",
            "PATHEXT",
            "CARGO_HOME",
            "RUSTUP_HOME",
            "XDG_CACHE_HOME",
        ] {
            assert!(env_allowed(name, false), "{name}");
        }
        for name in [
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "NPM_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "OPENAI_API_KEY",
            "SSH_AUTH_SOCK",
            "NODE_OPTIONS",
            "LD_PRELOAD",
            "RUSTC_WRAPPER",
            "RUSTFLAGS",
            "GIT_SSH_COMMAND",
            "HTTP_PROXY",
        ] {
            assert!(!env_allowed(name, false), "{name}");
        }
        // An approved network command also gets the way to reach the network, never a token.
        assert!(env_allowed("HTTPS_PROXY", true));
        assert!(env_allowed("SSH_AUTH_SOCK", true));
        assert!(!env_allowed("GH_TOKEN", true));
    }

    #[test]
    fn a_sandboxed_command_does_not_inherit_the_secrets_of_the_app() {
        let vars = [
            ("CD_AI_TEST_SECRET_TOKEN", "hunter2-hunter2"),
            ("GH_TOKEN", "ghp_x"),
            ("PATH", "/usr/bin"),
            ("LC_ALL", "C"),
        ]
        .map(|(name, value)| (name.into(), value.into()));
        let mut command = Command::new("true");
        scrub_env_from(&mut command, vars.into_iter(), false);
        let envs: Vec<_> = command.get_envs().collect();
        let names: Vec<_> = envs
            .iter()
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.iter().any(|name| name == "PATH"), "{names:?}");
        assert!(names.iter().any(|name| name == "LC_ALL"), "{names:?}");
    }

    #[test]
    fn git_hooks_and_config_are_protected_only_for_a_real_git_dir() {
        let workspace = tempfile::tempdir().unwrap();
        assert!(git_protected_paths(workspace.path()).is_empty());
        // A linked worktree has a `.git` file, not a directory.
        std::fs::write(workspace.path().join(".git"), "gitdir: /elsewhere\n").unwrap();
        assert!(git_protected_paths(workspace.path()).is_empty());
        std::fs::remove_file(workspace.path().join(".git")).unwrap();
        std::fs::create_dir(workspace.path().join(".git")).unwrap();
        assert_eq!(
            git_protected_paths(workspace.path()),
            vec![
                workspace.path().join(".git").join("hooks"),
                workspace.path().join(".git").join("config"),
            ]
        );
    }

    #[test]
    fn credential_paths_live_under_the_home_directory() {
        let Some(home) = home_dir() else {
            return;
        };
        let paths = secret_home_paths();
        assert!(paths.iter().all(|path| path.starts_with(&home)));
        for name in [".ssh", ".aws", ".npmrc", ".git-credentials"] {
            assert!(paths.contains(&home.join(name)), "{name}");
        }
        assert!(paths.contains(&home.join(".config").join("gcloud")));
        assert!(paths.contains(&home.join(".docker").join("config.json")));
    }
}
