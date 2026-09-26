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
    let mut command = Command::new(SANDBOX_EXEC);
    command
        .arg("-p")
        .arg(profile(roots.len(), exec.allow_network));
    // Paths travel as parameters, never spliced into the profile text.
    for (index, root) in roots.iter().enumerate() {
        let mut define = std::ffi::OsString::from(format!("-DROOT_{index}="));
        define.push(root);
        command.arg(define);
    }
    command.arg("--").arg(program);
    command
}

fn profile(roots: usize, allow_network: bool) -> String {
    let mut profile =
        String::from("(version 1)\n(allow default)\n(deny file-write*)\n(allow file-write*\n");
    for index in 0..roots {
        profile.push_str(&format!("  (subpath (param \"ROOT_{index}\"))\n"));
    }
    profile.push_str(
        "  (literal \"/dev/null\")\n  (literal \"/dev/zero\")\n  (literal \"/dev/dtracehelper\")\n  (regex #\"^/dev/tty\")\n  (regex #\"^/dev/fd/\"))\n",
    );
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
}
