//! OS sandbox for `run_command` (SPEC §20.3, plan 017).
//!
//! Linux applies Landlock (filesystem) and, when the command is not an approved network
//! class, a user+network namespace so the child has no route off the machine. Other
//! platforms report the sandbox unavailable: FULL ACCESS stays off and write commands keep
//! asking (D5/D6).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

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
    #[cfg(not(target_os = "linux"))]
    {
        SandboxStatus {
            available: false,
            filesystem: false,
            network_block: false,
            platform: if cfg!(windows) { "windows" } else { "other" }.to_string(),
            detail: "sandbox só existe no Linux nesta versão (Fase 7)".to_string(),
        }
    }
}

/// Installs the constraint on `command`. No-op when this host cannot isolate. Failures
/// inside the child abort the spawn: the command must not run "unsandboxed by accident".
pub fn constrain(command: &mut Command, exec: SandboxExec) {
    #[cfg(target_os = "linux")]
    linux::constrain(command, exec);
    #[cfg(not(target_os = "linux"))]
    let _ = (command, exec);
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::ffi::CString;
    use std::io;
    use std::os::unix::process::CommandExt;

    const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1 << 0;
    const LANDLOCK_RULE_PATH_BENEATH: u32 = 1;

    const FS_EXECUTE: u64 = 1 << 0;
    const FS_WRITE_FILE: u64 = 1 << 1;
    const FS_READ_FILE: u64 = 1 << 2;
    const FS_READ_DIR: u64 = 1 << 3;
    const FS_REMOVE_DIR: u64 = 1 << 4;
    const FS_REMOVE_FILE: u64 = 1 << 5;
    const FS_MAKE_CHAR: u64 = 1 << 6;
    const FS_MAKE_DIR: u64 = 1 << 7;
    const FS_MAKE_REG: u64 = 1 << 8;
    const FS_MAKE_SOCK: u64 = 1 << 9;
    const FS_MAKE_FIFO: u64 = 1 << 10;
    const FS_MAKE_BLOCK: u64 = 1 << 11;
    const FS_MAKE_SYM: u64 = 1 << 12;
    const FS_REFER: u64 = 1 << 13;
    const FS_TRUNCATE: u64 = 1 << 14;
    const FS_IOCTL_DEV: u64 = 1 << 15;

    const NET_BIND_TCP: u64 = 1 << 0;
    const NET_CONNECT_TCP: u64 = 1 << 1;

    const FS_READ: u64 = FS_EXECUTE | FS_READ_FILE | FS_READ_DIR;
    const FS_WRITE: u64 = FS_WRITE_FILE
        | FS_REMOVE_DIR
        | FS_REMOVE_FILE
        | FS_MAKE_CHAR
        | FS_MAKE_DIR
        | FS_MAKE_REG
        | FS_MAKE_SOCK
        | FS_MAKE_FIFO
        | FS_MAKE_BLOCK
        | FS_MAKE_SYM
        | FS_REFER
        | FS_TRUNCATE
        | FS_IOCTL_DEV;

    #[repr(C)]
    struct RulesetAttr {
        handled_access_fs: u64,
        handled_access_net: u64,
        scoped: u64,
    }

    #[repr(C)]
    struct PathBeneath {
        allowed_access: u64,
        parent_fd: i32,
    }

    static NETNS: OnceLock<bool> = OnceLock::new();

    pub fn net_namespace_available() -> bool {
        *NETNS.get_or_init(probe_netns)
    }

    pub fn probe() -> SandboxStatus {
        let abi = landlock_abi();
        let filesystem = abi >= 1;
        let netns = net_namespace_available();
        let landlock_net = abi >= 4;
        let network_block = netns || landlock_net;
        let available = filesystem && network_block;
        let mut parts = Vec::new();
        if filesystem {
            parts.push(format!("landlock fs (ABI {abi})"));
        }
        if netns {
            parts.push("user/net namespace".to_string());
        } else if landlock_net {
            parts.push("landlock TCP deny".to_string());
        }
        let detail = if available {
            parts.join(" + ")
        } else if parts.is_empty() {
            "este kernel não oferece Landlock nem user namespace".to_string()
        } else {
            format!(
                "sandbox incompleto ({}); FULL ACCESS indisponível",
                parts.join(" + ")
            )
        };
        SandboxStatus {
            available,
            filesystem,
            network_block,
            platform: "linux".to_string(),
            detail,
        }
    }

    pub fn constrain(command: &mut Command, exec: SandboxExec) {
        let snapshot = super::status().clone();
        if !snapshot.filesystem && !snapshot.network_block {
            return;
        }
        let write_roots = write_roots(&exec.workspace);
        let isolate_net = snapshot.network_block && !exec.allow_network;
        let use_netns = isolate_net && net_namespace_available();
        let use_landlock_net = isolate_net && !use_netns;
        let use_landlock_fs = snapshot.filesystem;
        if !use_netns && !use_landlock_fs && !use_landlock_net {
            return;
        }

        // pre_exec runs after fork, before exec. Not strictly async-signal-safe (opens files),
        // which is how every practical Landlock wrapper does it.
        unsafe {
            command.pre_exec(move || {
                if use_netns {
                    enter_netns()?;
                }
                if use_landlock_fs || use_landlock_net {
                    apply_landlock(&write_roots, use_landlock_net)?;
                }
                Ok(())
            });
        }
    }

    fn probe_netns() -> bool {
        let mut command = tiny_true();
        unsafe {
            command.pre_exec(enter_netns);
        }
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn tiny_true() -> Command {
        for path in ["/usr/bin/true", "/bin/true"] {
            if Path::new(path).exists() {
                return Command::new(path);
            }
        }
        Command::new("true")
    }

    fn enter_netns() -> io::Result<()> {
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };
        let rc = unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        write_proc("/proc/self/setgroups", b"deny")?;
        let uid_map = format!("{uid} {uid} 1\n");
        let gid_map = format!("{gid} {gid} 1\n");
        write_proc("/proc/self/uid_map", uid_map.as_bytes())?;
        write_proc("/proc/self/gid_map", gid_map.as_bytes())?;
        Ok(())
    }

    fn write_proc(path: &str, data: &[u8]) -> io::Result<()> {
        let c_path = CString::new(path).map_err(io::Error::other)?;
        let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let wrote = unsafe { libc::write(fd, data.as_ptr().cast(), data.len()) };
        unsafe { libc::close(fd) };
        if wrote < 0 || wrote as usize != data.len() {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn landlock_abi() -> i64 {
        let abi = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<RulesetAttr>(),
                0usize,
                LANDLOCK_CREATE_RULESET_VERSION,
            )
        };
        if abi < 0 { 0 } else { abi }
    }

    fn apply_landlock(write_roots: &[PathBuf], deny_tcp: bool) -> io::Result<()> {
        let abi = landlock_abi();
        if abi < 1 {
            return Err(io::Error::other("landlock indisponível"));
        }
        let mut handled_fs = FS_READ
            | FS_WRITE_FILE
            | FS_REMOVE_DIR
            | FS_REMOVE_FILE
            | FS_MAKE_CHAR
            | FS_MAKE_DIR
            | FS_MAKE_REG
            | FS_MAKE_SOCK
            | FS_MAKE_FIFO
            | FS_MAKE_BLOCK
            | FS_MAKE_SYM;
        if abi >= 2 {
            handled_fs |= FS_REFER;
        }
        if abi >= 3 {
            handled_fs |= FS_TRUNCATE;
        }
        if abi >= 5 {
            handled_fs |= FS_IOCTL_DEV;
        }
        let handled_net = if deny_tcp && abi >= 4 {
            NET_BIND_TCP | NET_CONNECT_TCP
        } else {
            0
        };

        let attr = RulesetAttr {
            handled_access_fs: handled_fs,
            handled_access_net: handled_net,
            scoped: 0,
        };
        let size = if abi >= 6 {
            std::mem::size_of::<RulesetAttr>()
        } else if abi >= 4 {
            16
        } else {
            8
        };
        let fd = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &attr as *const RulesetAttr,
                size,
                0u32,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = fd as i32;

        let result = (|| {
            add_path(fd, Path::new("/"), FS_READ & handled_fs, handled_fs)?;
            for root in write_roots {
                add_path(fd, root, (FS_READ | FS_WRITE) & handled_fs, handled_fs)?;
            }
            for device in [
                "/dev/null",
                "/dev/zero",
                "/dev/urandom",
                "/dev/random",
                "/dev/tty",
            ] {
                let _ = add_path(
                    fd,
                    Path::new(device),
                    (FS_READ | FS_WRITE_FILE | FS_IOCTL_DEV) & handled_fs,
                    handled_fs,
                );
            }
            let rc = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
            if rc != 0 {
                return Err(io::Error::last_os_error());
            }
            let rc = unsafe { libc::syscall(libc::SYS_landlock_restrict_self, fd, 0u32) };
            if rc < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })();
        unsafe { libc::close(fd) };
        result
    }

    fn add_path(ruleset: i32, path: &Path, allowed: u64, handled: u64) -> io::Result<()> {
        if !path.exists() {
            return Ok(());
        }
        let allowed = allowed & handled;
        if allowed == 0 {
            return Ok(());
        }
        let c_path = CString::new(path.to_string_lossy().as_bytes()).map_err(io::Error::other)?;
        let parent_fd = unsafe { libc::open(c_path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
        if parent_fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let rule = PathBeneath {
            allowed_access: allowed,
            parent_fd,
        };
        let rc = unsafe {
            libc::syscall(
                libc::SYS_landlock_add_rule,
                ruleset,
                LANDLOCK_RULE_PATH_BENEATH,
                &rule as *const PathBeneath,
                0u32,
            )
        };
        unsafe { libc::close(parent_fd) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn write_roots(workspace: &Path) -> Vec<PathBuf> {
        let mut roots = vec![workspace.to_path_buf()];
        for extra in [
            std::env::temp_dir(),
            PathBuf::from("/tmp"),
            PathBuf::from("/var/tmp"),
        ] {
            push_unique(&mut roots, extra);
        }
        if let Ok(tmpdir) = std::env::var("TMPDIR") {
            push_unique(&mut roots, PathBuf::from(tmpdir));
        }
        if let Ok(cache) = std::env::var("XDG_CACHE_HOME") {
            push_unique(&mut roots, PathBuf::from(cache));
        }
        if let Ok(cargo) = std::env::var("CARGO_HOME") {
            push_unique(&mut roots, PathBuf::from(cargo));
        }
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            for rel in [
                ".cache",
                ".bun",
                ".cargo",
                ".rustup",
                ".npm",
                ".local/share",
                ".local/state",
            ] {
                push_unique(&mut roots, home.join(rel));
            }
        }
        roots
    }

    fn push_unique(roots: &mut Vec<PathBuf>, path: PathBuf) {
        if path.as_os_str().is_empty() {
            return;
        }
        if roots.iter().any(|seen| seen == &path) {
            return;
        }
        roots.push(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn status_names_the_platform() {
        let status = status();
        assert!(!status.platform.is_empty());
        assert_eq!(status.available, status.filesystem && status.network_block);
        #[cfg(target_os = "linux")]
        assert_eq!(status.platform, "linux");
        #[cfg(windows)]
        {
            assert!(!status.available);
            assert!(status.detail.contains("Linux"));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sandbox_blocks_tcp_to_a_local_listener() {
        let status = status();
        if !status.network_block {
            eprintln!("skip: no network block on this host ({})", status.detail);
            return;
        }
        if Command::new("python3")
            .arg("-c")
            .arg("print(1)")
            .output()
            .is_err()
        {
            eprintln!("skip: python3 missing");
            return;
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let server = thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while std::time::Instant::now() < deadline {
                if listener.accept().is_ok() {
                    return true;
                }
                thread::sleep(Duration::from_millis(20));
            }
            false
        });

        let dir = tempdir().unwrap();
        let mut blocked = Command::new("python3");
        blocked.args([
            "-c",
            &format!("import socket; socket.create_connection(('127.0.0.1', {port}), 1)"),
        ]);
        blocked.current_dir(dir.path());
        constrain(
            &mut blocked,
            SandboxExec {
                workspace: dir.path().to_path_buf(),
                allow_network: false,
            },
        );
        let blocked_out = blocked.output().expect("spawn sandboxed python");
        assert!(
            !blocked_out.status.success(),
            "sandboxed connect must fail: {}",
            String::from_utf8_lossy(&blocked_out.stderr)
        );
        assert!(
            !server.join().unwrap(),
            "the listener must not have accepted a sandboxed connection"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sandbox_allows_tcp_when_network_is_explicitly_released() {
        let status = status();
        if !status.network_block {
            eprintln!("skip: no network block on this host ({})", status.detail);
            return;
        }
        if Command::new("python3")
            .arg("-c")
            .arg("print(1)")
            .output()
            .is_err()
        {
            eprintln!("skip: python3 missing");
            return;
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            listener.set_nonblocking(true).ok();
            while std::time::Instant::now() < deadline {
                if listener.accept().is_ok() {
                    return true;
                }
                thread::sleep(Duration::from_millis(20));
            }
            false
        });

        let dir = tempdir().unwrap();
        let mut allowed = Command::new("python3");
        allowed.args([
            "-c",
            &format!("import socket; socket.create_connection(('127.0.0.1', {port}), 2)"),
        ]);
        allowed.current_dir(dir.path());
        constrain(
            &mut allowed,
            SandboxExec {
                workspace: dir.path().to_path_buf(),
                allow_network: true,
            },
        );
        let allowed_out = allowed.output().expect("spawn python with network");
        assert!(
            allowed_out.status.success(),
            "approved network must connect: stderr={}",
            String::from_utf8_lossy(&allowed_out.stderr)
        );
        assert!(server.join().unwrap(), "listener should have accepted");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sandbox_allows_writes_in_the_workspace_and_denies_home() {
        let status = status();
        if !status.filesystem {
            eprintln!("skip: no landlock on this host ({})", status.detail);
            return;
        }

        let workspace = tempdir().unwrap();
        let inside = workspace.path().join("ok.txt");
        let mut ok = Command::new("touch");
        ok.arg(&inside);
        ok.current_dir(workspace.path());
        constrain(
            &mut ok,
            SandboxExec {
                workspace: workspace.path().to_path_buf(),
                allow_network: false,
            },
        );
        let ok_out = ok.output().expect("touch inside workspace");
        assert!(
            ok_out.status.success(),
            "workspace write must work: {}",
            String::from_utf8_lossy(&ok_out.stderr)
        );
        assert!(inside.exists());

        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let forbid = PathBuf::from(home).join(format!("cd-ai-fase7-forbid-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&forbid);
        let target = forbid.join("nope.txt");
        let mut denied = Command::new("touch");
        denied.arg(&target);
        denied.current_dir(workspace.path());
        constrain(
            &mut denied,
            SandboxExec {
                workspace: workspace.path().to_path_buf(),
                allow_network: false,
            },
        );
        let denied_out = denied.output().expect("touch outside workspace");
        let _ = std::fs::remove_dir_all(&forbid);
        assert!(
            !denied_out.status.success(),
            "write outside workspace/tmp/cache must fail"
        );
        assert!(!target.exists());
    }
}
