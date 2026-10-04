use super::*;
use std::path::Path;
use std::process::Stdio;

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

pub fn probe() -> SandboxStatus {
    // Deprecated by Apple but still the only way to confine a child process on macOS; the probe
    // runs a real (empty) profile so a removed or broken `sandbox-exec` reads as unavailable.
    let works = Command::new(SANDBOX_EXEC)
        .args([
            "-p",
            "(version 1)(allow default)(deny network*)",
            "--",
            "/usr/bin/true",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    SandboxStatus {
        available: works,
        filesystem: works,
        network_block: works,
        platform: "macos".to_string(),
        detail: if works {
            "Seatbelt (sandbox-exec)".to_string()
        } else {
            "sandbox-exec indisponível neste macOS".to_string()
        },
    }
}

pub fn command(program: &str, exec: &SandboxExec) -> Command {
    if !super::status().available {
        return Command::new(program);
    }
    let roots = write_roots(&exec.workspace);
    // Hooks and config name programs git runs on its own. Every command keeps them read-only.
    let read_only = real_paths(super::git_protected_paths(&exec.workspace));
    // Credentials are hidden from every command that has no network: an approved `network`
    // command (`git push`, `npm publish`) runs as the user and needs them.
    let hidden = if exec.allow_network {
        Vec::new()
    } else {
        real_paths(super::secret_home_paths())
    };
    let mut command = Command::new(SANDBOX_EXEC);
    command.arg("-p").arg(profile(
        roots.len(),
        read_only.len(),
        hidden.len(),
        exec.allow_network,
    ));
    // Paths travel as parameters, never spliced into the profile text.
    for (prefix, paths) in [("ROOT", &roots), ("RO", &read_only), ("HIDE", &hidden)] {
        for (index, path) in paths.iter().enumerate() {
            let mut define = std::ffi::OsString::from(format!("-D{prefix}_{index}="));
            define.push(path);
            command.arg(define);
        }
    }
    command.arg("--").arg(program);
    command
}

/// The paths that exist, in the real form Seatbelt matches (`/tmp` is `/private/tmp`).
fn real_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter_map(|path| path.canonicalize().ok())
        .collect()
}

/// Seatbelt takes the last rule that matches, so the denials come after the allows they carve
/// out of: the workspace is writable except `read_only` paths, and everything is readable except
/// `hidden` ones.
fn profile(roots: usize, read_only: usize, hidden: usize, allow_network: bool) -> String {
    let mut profile =
        String::from("(version 1)\n(allow default)\n(deny file-write*)\n(allow file-write*\n");
    for index in 0..roots {
        profile.push_str(&format!("  (subpath (param \"ROOT_{index}\"))\n"));
    }
    profile.push_str(
        "  (literal \"/dev/null\")\n  (literal \"/dev/zero\")\n  (literal \"/dev/dtracehelper\")\n  (regex #\"^/dev/tty\")\n  (regex #\"^/dev/fd/\"))\n",
    );
    for index in 0..read_only {
        profile.push_str(&format!(
            "(deny file-write* (subpath (param \"RO_{index}\")))\n"
        ));
    }
    for index in 0..hidden {
        profile.push_str(&format!(
            "(deny file-read* file-write* (subpath (param \"HIDE_{index}\")))\n"
        ));
    }
    if !allow_network {
        profile.push_str("(deny network*)\n");
    }
    profile
}

/// Seatbelt matches real paths (`/tmp` is `/private/tmp`), so every root is canonical.
fn write_roots(workspace: &Path) -> Vec<PathBuf> {
    let mut roots = vec![workspace.to_path_buf()];
    let temps = [
        Some(std::env::temp_dir()),
        std::env::var_os("TMPDIR").map(PathBuf::from),
        Some(PathBuf::from("/tmp")),
        Some(PathBuf::from("/var/tmp")),
    ];
    for root in temps.into_iter().flatten().chain(super::cache_roots()) {
        if let Ok(real) = root.canonicalize()
            && !roots.contains(&real)
        {
            roots.push(real);
        }
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sandboxed(workspace: &Path, argv: &[&str], allow_network: bool) -> std::process::Output {
        let mut command = super::super::command(
            argv[0],
            &SandboxExec {
                workspace: workspace.to_path_buf(),
                allow_network,
            },
        )
        .unwrap();
        command.args(&argv[1..]).current_dir(workspace);
        command.output().unwrap()
    }

    #[test]
    fn writes_only_inside_the_workspace() {
        if !status().available {
            eprintln!("skip: {}", status().detail);
            return;
        }
        let workspace = tempdir().unwrap();
        let workspace_path = workspace.path().canonicalize().unwrap();
        let ok = sandboxed(&workspace_path, &["/usr/bin/touch", "dentro.txt"], false);
        assert!(
            ok.status.success(),
            "{}",
            String::from_utf8_lossy(&ok.stderr)
        );
        assert!(workspace_path.join("dentro.txt").exists());

        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let outside = home.join(format!("cd-ai-sandbox-{}.txt", std::process::id()));
        let denied = sandboxed(
            &workspace_path,
            &["/usr/bin/touch", outside.to_str().unwrap()],
            false,
        );
        let _ = std::fs::remove_file(&outside);
        assert!(
            !denied.status.success(),
            "write outside the workspace must fail"
        );
    }

    #[test]
    fn network_is_blocked_unless_released() {
        if !status().available {
            eprintln!("skip: {}", status().detail);
            return;
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while std::time::Instant::now() < deadline {
                if listener.accept().is_ok() {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            false
        });
        let workspace = tempdir().unwrap();
        let url = format!("http://127.0.0.1:{port}/");
        let _ = sandboxed(
            workspace.path(),
            &["/usr/bin/curl", "-s", "--max-time", "1", &url],
            false,
        );
        assert!(
            !server.join().unwrap(),
            "the listener must not see a sandboxed connection"
        );
    }

    #[test]
    fn profile_carves_hooks_and_credentials_out_after_the_allows() {
        let text = profile(2, 2, 1, false);
        let allow = text.find("(allow file-write*").unwrap();
        for denial in [
            "(deny file-write* (subpath (param \"RO_0\")))",
            "(deny file-write* (subpath (param \"RO_1\")))",
            "(deny file-read* file-write* (subpath (param \"HIDE_0\")))",
        ] {
            let at = text
                .find(denial)
                .unwrap_or_else(|| panic!("{denial} em {text}"));
            assert!(at > allow, "a negação tem de vir depois do allow: {text}");
        }
        assert!(text.contains("(deny network*)"));
        assert!(!text.contains("RO_2") && !text.contains("HIDE_1"));
        // An approved `network` command keeps the network but not a different file policy.
        assert!(!profile(1, 2, 0, true).contains("(deny network*)"));
        assert!(!profile(1, 0, 0, false).contains("HIDE_"));
    }

    #[test]
    fn git_hooks_and_config_are_read_only_inside_the_workspace() {
        if !status().available {
            eprintln!("skip: {}", status().detail);
            return;
        }
        let workspace = tempdir().unwrap();
        let workspace_path = workspace.path().canonicalize().unwrap();
        std::fs::create_dir_all(workspace_path.join(".git").join("hooks")).unwrap();
        std::fs::write(workspace_path.join(".git").join("config"), "[core]\n").unwrap();

        let hook = sandboxed(
            &workspace_path,
            &["/bin/sh", "-c", "echo x > .git/hooks/pre-commit"],
            false,
        );
        assert!(!hook.status.success(), "writing a hook must fail");
        assert!(!workspace_path.join(".git/hooks/pre-commit").exists());
        let config = sandboxed(
            &workspace_path,
            &["/bin/sh", "-c", "echo x >> .git/config"],
            false,
        );
        assert!(
            !config.status.success(),
            "appending to .git/config must fail"
        );
        let moved = sandboxed(
            &workspace_path,
            &["/bin/sh", "-c", "mv .git/hooks .git/hooks-old"],
            false,
        );
        assert!(!moved.status.success(), "renaming .git/hooks must fail");
        let ok = sandboxed(
            &workspace_path,
            &["/bin/sh", "-c", "echo ok > a.txt && cat .git/config"],
            false,
        );
        assert!(
            ok.status.success(),
            "{}",
            String::from_utf8_lossy(&ok.stderr)
        );
    }
}
