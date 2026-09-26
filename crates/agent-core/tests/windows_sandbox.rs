//! End to end check of the Windows AppContainer sandbox. `harness = false` because this binary is
//! also the launcher the sandbox re-executes: `main` has to call `sandbox::init()` first, exactly
//! like the CLI and the desktop app do. A no-op on other platforms.

fn main() {
    #[cfg(windows)]
    windows::run();
}

#[cfg(windows)]
mod windows {
    use std::net::TcpListener;
    use std::path::Path;
    use std::process::{Command, Output, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    use agent_core::sandbox::{self, SandboxExec};

    pub fn run() {
        sandbox::init();
        let status = sandbox::status();
        writes_only_inside_the_workspace();
        network_is_blocked_even_on_loopback();
        approved_network_runs_outside_the_container();
        if status.available {
            toolchains_on_path_still_run();
        } else {
            // git/cargo/node need the one-time `cd-ai sandbox-setup`; the probe must say so.
            assert!(status.detail.contains("sandbox-setup"), "{}", status.detail);
            println!("skip: toolchains ({})", status.detail);
        }
        println!("windows_sandbox: ok");
    }

    fn workspace() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        // The engine hands the sandbox the canonical (verbatim) root.
        let root = dir.path().canonicalize().unwrap();
        (dir, root)
    }

    fn run_in(workspace: &Path, argv: &[&str], allow_network: bool) -> Output {
        let exec = SandboxExec {
            workspace: workspace.to_path_buf(),
            allow_network,
        };
        let command = if allow_network {
            sandbox::command(argv[0], &exec)
        } else {
            sandbox::contained_command(argv[0], &exec)
        };
        command
            .unwrap()
            .args(&argv[1..])
            .current_dir(workspace)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn writes_only_inside_the_workspace() {
        let (_dir, root) = workspace();
        let inside = run_in(&root, &["cmd", "/c", "echo dentro> dentro.txt"], false);
        assert!(inside.status.success(), "{inside:?}");
        assert!(root.join("dentro.txt").exists());

        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("fora.txt");
        let script = format!("echo fora> \"{}\"", target.display());
        let denied = run_in(&root, &["cmd", "/c", &script], false);
        assert!(!denied.status.success(), "{denied:?}");
        assert!(
            !target.exists(),
            "a write outside the workspace went through"
        );
        println!("ok: escrita só no workspace");
    }

    fn listener() -> (u16, thread::JoinHandle<bool>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(4);
            while Instant::now() < deadline {
                if listener.accept().is_ok() {
                    return true;
                }
                thread::sleep(Duration::from_millis(20));
            }
            false
        });
        (port, server)
    }

    fn network_is_blocked_even_on_loopback() {
        let (_dir, root) = workspace();
        let (port, server) = listener();
        let url = format!("http://127.0.0.1:{port}/");
        let _ = run_in(&root, &["curl", "-s", "--max-time", "2", &url], false);
        assert!(
            !server.join().unwrap(),
            "a contained process reached the network"
        );
        println!("ok: rede bloqueada");
    }

    fn approved_network_runs_outside_the_container() {
        let (_dir, root) = workspace();
        let (port, server) = listener();
        let url = format!("http://127.0.0.1:{port}/");
        let _ = run_in(&root, &["curl", "-s", "--max-time", "2", &url], true);
        assert!(
            server.join().unwrap(),
            "an approved network command must connect"
        );
        println!("ok: comando de rede aprovado conecta");
    }

    fn on_path(program: &str) -> bool {
        Command::new(program)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn toolchains_on_path_still_run() {
        let (_dir, root) = workspace();
        if on_path("git") {
            let init = Command::new("git")
                .args(["init", "-q"])
                .current_dir(&root)
                .status()
                .unwrap();
            assert!(init.success());
            std::fs::write(root.join("a.txt"), "a\n").unwrap();
            let status = run_in(&root, &["git", "status", "--porcelain"], false);
            assert!(status.status.success(), "{status:?}");
            assert!(String::from_utf8_lossy(&status.stdout).contains("a.txt"));
            println!("ok: git dentro do sandbox");
        }
        if on_path("cargo") {
            let version = run_in(&root, &["cargo", "--version"], false);
            assert!(version.status.success(), "{version:?}");
            println!("ok: cargo dentro do sandbox");
        }
    }
}
