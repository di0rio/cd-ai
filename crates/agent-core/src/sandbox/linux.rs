use super::*;
use std::ffi::CString;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::Path;

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
    constrain_with(command, exec, super::secret_home_paths());
}

/// [`constrain`] with the credential paths to hide spelled out, so a test can aim it at a
/// directory of its own.
fn constrain_with(command: &mut Command, exec: SandboxExec, secrets: Vec<PathBuf>) {
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
    // Landlock only ever adds access beneath a path: it cannot take `.git/hooks` back from a
    // writable workspace, nor hide `~/.ssh` from a readable `/`. A mount namespace can, and the
    // user namespace the network isolation already creates is what allows one. Without it
    // (Landlock-only hosts, approved `network` commands, which need the user's credentials) these
    // paths stay as Landlock leaves them, and the approval for a command that names `.git` is
    // what protects the hooks.
    let mounts = if use_netns {
        MountPlan::new(&exec.workspace, &secrets)
    } else {
        MountPlan::default()
    };

    // pre_exec runs after fork, before exec. Not strictly async-signal-safe (opens files),
    // which is how every practical Landlock wrapper does it.
    unsafe {
        command.pre_exec(move || {
            if use_netns {
                enter_netns()?;
                // Best effort: a kernel that refuses a mount leaves that path as it was, it
                // does not stop the command.
                mounts.apply();
            }
            if use_landlock_fs || use_landlock_net {
                apply_landlock(&write_roots, use_landlock_net)?;
            }
            Ok(())
        });
    }
}

/// What to remount in the child's own mount namespace, as C strings built before the fork.
#[derive(Default)]
struct MountPlan {
    /// Existing paths that stay visible but read-only: `.git/hooks`, `.git/config`.
    read_only: Vec<CString>,
    /// Credential directories, replaced by an empty read-only filesystem.
    hidden_dirs: Vec<CString>,
    /// Credential files, replaced by `/dev/null`.
    hidden_files: Vec<CString>,
}

impl MountPlan {
    fn new(workspace: &Path, secrets: &[PathBuf]) -> Self {
        use std::os::unix::ffi::OsStrExt;
        let c_path = |path: &Path| CString::new(path.as_os_str().as_bytes()).ok();
        let mut plan = Self::default();
        for path in super::git_protected_paths(workspace) {
            // A directory or file that is not there yet cannot be mounted over.
            if path.exists()
                && let Some(c_path) = c_path(&path)
            {
                plan.read_only.push(c_path);
            }
        }
        for path in secrets {
            let Some(c_path) = c_path(path) else {
                continue;
            };
            // `symlink_metadata`: a link to somewhere else is not mounted over, the link itself
            // is a file that names nothing the command could not already name.
            match std::fs::symlink_metadata(path) {
                Ok(meta) if meta.is_dir() => plan.hidden_dirs.push(c_path),
                Ok(meta) if meta.is_file() => plan.hidden_files.push(c_path),
                _ => {}
            }
        }
        plan
    }

    fn is_empty(&self) -> bool {
        self.read_only.is_empty() && self.hidden_dirs.is_empty() && self.hidden_files.is_empty()
    }

    /// Runs in the child between `unshare(CLONE_NEWUSER)` and `exec`. Every failure is ignored.
    fn apply(&self) {
        if self.is_empty() || unsafe { libc::unshare(libc::CLONE_NEWNS) } != 0 {
            return;
        }
        // Nothing mounted below may propagate back to the host's namespace.
        let none: *const libc::c_char = std::ptr::null();
        let no_data: *const libc::c_void = std::ptr::null();
        unsafe {
            libc::mount(
                none,
                c"/".as_ptr(),
                none,
                libc::MS_REC | libc::MS_PRIVATE,
                no_data,
            );
        }
        for path in &self.read_only {
            bind_read_only(path.as_c_str(), path.as_c_str());
        }
        for path in &self.hidden_files {
            bind_read_only(c"/dev/null", path.as_c_str());
        }
        for path in &self.hidden_dirs {
            let flags = libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC;
            unsafe {
                libc::mount(
                    c"tmpfs".as_ptr(),
                    path.as_ptr(),
                    c"tmpfs".as_ptr(),
                    flags,
                    c"size=4k,mode=0555".as_ptr().cast(),
                );
            }
        }
    }
}

/// `mount --bind source target` and then read-only. The second step has to repeat the flags the
/// source mount already carries (`nosuid`, `nodev`, ...): inside a user namespace those are
/// locked, and a remount that drops one fails with EPERM.
fn bind_read_only(source: &std::ffi::CStr, target: &std::ffi::CStr) {
    let none: *const libc::c_char = std::ptr::null();
    let no_data: *const libc::c_void = std::ptr::null();
    let bound = unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            none,
            libc::MS_BIND,
            no_data,
        )
    };
    if bound != 0 {
        return;
    }
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let kept = if unsafe { libc::statvfs(target.as_ptr(), &mut stat) } == 0 {
        mount_flags_of(stat.f_flag as u64)
    } else {
        0
    };
    unsafe {
        libc::mount(
            none,
            target.as_ptr(),
            none,
            libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY | kept,
            no_data,
        );
    }
}

/// `statvfs` flags (`ST_*`) as the `MS_*` flags a remount has to pass to keep them.
fn mount_flags_of(st_flags: u64) -> libc::c_ulong {
    const ST_NOSUID: u64 = 2;
    const ST_NODEV: u64 = 4;
    const ST_NOEXEC: u64 = 8;
    const ST_NOATIME: u64 = 1024;
    const ST_NODIRATIME: u64 = 2048;
    const ST_RELATIME: u64 = 4096;
    let mut flags = 0;
    for (st, ms) in [
        (ST_NOSUID, libc::MS_NOSUID),
        (ST_NODEV, libc::MS_NODEV),
        (ST_NOEXEC, libc::MS_NOEXEC),
        (ST_NOATIME, libc::MS_NOATIME),
        (ST_NODIRATIME, libc::MS_NODIRATIME),
        (ST_RELATIME, libc::MS_RELATIME),
    ] {
        if st_flags & st != 0 {
            flags |= ms;
        }
    }
    flags
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
            // Device files reject directory/execute bits (EINVAL on ABI 6). Git opens
            // `/dev/null` O_RDWR, so the rule has to be file read/write (+ ioctl).
            let _ = add_path(
                fd,
                Path::new(device),
                (FS_READ_FILE | FS_WRITE_FILE | FS_IOCTL_DEV) & handled_fs,
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
    let mut allowed = allowed & handled;
    // A rule on a regular file may only carry file rights (EINVAL otherwise): the cargo locks.
    if !path.is_dir() {
        allowed &= FS_EXECUTE | FS_WRITE_FILE | FS_READ_FILE | FS_TRUNCATE | FS_IOCTL_DEV;
    }
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
    for cache in super::cache_roots() {
        push_unique(&mut roots, cache);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

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

    #[test]
    fn sandbox_allows_opening_dev_null_read_write() {
        let status = status();
        if !status.filesystem {
            eprintln!("skip: no landlock on this host ({})", status.detail);
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

        let workspace = tempdir().unwrap();
        let mut command = Command::new("python3");
        command.args(["-c", "open('/dev/null', 'r+').close()"]);
        command.current_dir(workspace.path());
        constrain(
            &mut command,
            SandboxExec {
                workspace: workspace.path().to_path_buf(),
                allow_network: false,
            },
        );
        let output = command.output().expect("open /dev/null in sandbox");
        assert!(
            output.status.success(),
            "sandboxed /dev/null O_RDWR must work: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn mount_plan_picks_only_what_exists_and_sorts_files_from_directories() {
        let workspace = tempdir().unwrap();
        // No `.git` yet: nothing to protect.
        assert!(MountPlan::new(workspace.path(), &[]).is_empty());

        std::fs::create_dir_all(workspace.path().join(".git").join("hooks")).unwrap();
        std::fs::write(workspace.path().join(".git").join("config"), "[core]\n").unwrap();
        let home = tempdir().unwrap();
        std::fs::create_dir(home.path().join(".ssh")).unwrap();
        std::fs::write(home.path().join(".npmrc"), "//x:_authToken=y\n").unwrap();
        let plan = MountPlan::new(
            workspace.path(),
            &[
                home.path().join(".ssh"),
                home.path().join(".npmrc"),
                home.path().join(".aws"),
            ],
        );
        assert_eq!(plan.read_only.len(), 2);
        assert_eq!(plan.hidden_dirs.len(), 1);
        assert_eq!(plan.hidden_files.len(), 1);
    }

    #[test]
    fn statvfs_flags_translate_to_the_mount_flags_a_remount_must_keep() {
        assert_eq!(mount_flags_of(0), 0);
        assert_eq!(
            mount_flags_of(2 | 4 | 8),
            libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC
        );
        assert_eq!(mount_flags_of(4096), libc::MS_RELATIME);
    }

    #[test]
    fn sandbox_keeps_git_hooks_and_config_read_only_and_hides_credentials() {
        let status = status();
        if !status.filesystem || !net_namespace_available() {
            eprintln!(
                "skip: needs Landlock and a user namespace ({})",
                status.detail
            );
            return;
        }
        let workspace = tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join(".git").join("hooks")).unwrap();
        std::fs::write(workspace.path().join(".git").join("config"), "[core]\n").unwrap();
        let home = tempdir().unwrap();
        std::fs::create_dir(home.path().join(".ssh")).unwrap();
        std::fs::write(home.path().join(".ssh").join("id_test"), "CHAVE-PRIVADA\n").unwrap();
        std::fs::write(home.path().join(".npmrc"), "TOKEN-NPM\n").unwrap();
        let secrets = vec![home.path().join(".ssh"), home.path().join(".npmrc")];

        let run = |script: &str| {
            let mut command = Command::new("sh");
            command.args(["-c", script]).current_dir(workspace.path());
            constrain_with(
                &mut command,
                SandboxExec {
                    workspace: workspace.path().to_path_buf(),
                    allow_network: false,
                },
                secrets.clone(),
            );
            command.output().expect("spawn sandboxed sh")
        };

        let hook = run("echo '#!/bin/sh' > .git/hooks/pre-commit");
        assert!(!hook.status.success(), "writing a hook must fail");
        assert!(!workspace.path().join(".git/hooks/pre-commit").exists());
        let config = run("echo '[core] fsmonitor = evil' >> .git/config");
        assert!(
            !config.status.success(),
            "appending to .git/config must fail"
        );
        assert_eq!(
            std::fs::read_to_string(workspace.path().join(".git/config")).unwrap(),
            "[core]\n"
        );
        // Moving the hooks folder away to plant another is a write to its mount point.
        let renamed = run("mv .git/hooks .git/hooks-old");
        assert!(!renamed.status.success(), "renaming .git/hooks must fail");

        // The rest of the workspace is still writable, and `.git/config` still readable.
        let ok = run("echo ok > a.txt && cat .git/config");
        assert!(
            ok.status.success(),
            "{}",
            String::from_utf8_lossy(&ok.stderr)
        );
        assert!(workspace.path().join("a.txt").exists());

        let ssh = run(&format!(
            "cat {}",
            home.path().join(".ssh/id_test").display()
        ));
        assert!(!String::from_utf8_lossy(&ssh.stdout).contains("CHAVE-PRIVADA"));
        let npmrc = run(&format!("cat {}", home.path().join(".npmrc").display()));
        assert!(!String::from_utf8_lossy(&npmrc.stdout).contains("TOKEN-NPM"));
        // Outside the sandbox nothing changed.
        assert!(home.path().join(".ssh/id_test").exists());
        assert_eq!(
            std::fs::read_to_string(home.path().join(".npmrc")).unwrap(),
            "TOKEN-NPM\n"
        );
    }
}
