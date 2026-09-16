use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

fn run_cli(args: &[&str]) -> Output {
    run_cli_env(args, &[])
}

/// Runs the binary with extra environment. Every task test sets `CD_AI_DATA_DIR` and `OLLAMA_HOST`
/// so it never writes to the real data directory and never reaches a real Ollama.
fn run_cli_env(args: &[&str], vars: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cd-ai"));
    command.args(args);
    for (key, value) in vars {
        command.env(key, value);
    }
    command.output().expect("falha ao executar o binário cd-ai")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout não é UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr não é UTF-8")
}

/// A directory under the system temp dir, removed on drop. Std only: the CLI crate has no
/// dev-dependencies and one temp directory is not worth adding any.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("cd-ai-cli-{label}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).expect("criar diretório temporário");
        Self { path }
    }

    fn as_str(&self) -> &str {
        self.path.to_str().expect("caminho temporário em UTF-8")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn version_prints_name_and_crate_version() {
    let output = run_cli(&["--version"]);
    assert!(output.status.success());
    assert_eq!(
        stdout(&output),
        format!("cd-ai {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn no_args_prints_usage_and_succeeds() {
    let output = run_cli(&[]);
    assert!(output.status.success());
    assert!(stdout(&output).contains("uso: cd-ai"));
}

#[test]
fn help_flags_print_usage_and_succeed() {
    for flag in ["--help", "-h"] {
        let output = run_cli(&[flag]);
        assert!(output.status.success(), "flag {flag} deve sair com sucesso");
        assert!(stdout(&output).contains("uso: cd-ai"));
    }
}

#[test]
fn help_mentions_task() {
    for flag in ["--help", "-h"] {
        let text = stdout(&run_cli(&[flag]));
        assert!(text.contains("cd-ai task"), "{flag} não cita task: {text}");
        assert!(
            text.contains("--resume"),
            "{flag} não cita --resume: {text}"
        );
        assert!(
            text.contains("--workspace"),
            "{flag} não cita --workspace: {text}"
        );
        assert!(text.contains("--mode"), "{flag} não cita --mode: {text}");
    }
}

#[test]
fn unknown_argument_exits_with_code_2_and_message_on_stderr() {
    let output = run_cli(&["--bogus"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("argumento desconhecido: --bogus"));
    assert!(stderr(&output).contains("uso: cd-ai"));
}

#[test]
fn task_without_model_exits_2() {
    let output = run_cli(&["task", "arrume", "a", "soma"]);
    assert_eq!(output.status.code(), Some(2));
    let text = stderr(&output);
    assert!(text.contains("faltou --model"), "{text}");
    assert!(text.contains("uso: cd-ai"), "{text}");
}

#[test]
fn task_without_request_exits_2() {
    let output = run_cli(&["task", "--model", "modelo-de-teste"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("faltou o pedido da tarefa"));
}

#[test]
fn task_with_bad_number_exits_2() {
    let output = run_cli(&["task", "--model", "m", "--ctx", "muito", "oi"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--ctx exige um número: muito"));
}

#[test]
fn task_with_unknown_flag_exits_2() {
    let output = run_cli(&["task", "--model", "m", "--modl", "qwen3", "arrume a soma"]);
    assert_eq!(output.status.code(), Some(2));
    let text = stderr(&output);
    assert!(text.contains("flag desconhecida: --modl"), "{text}");
    assert!(text.contains("uso: cd-ai"), "{text}");
}

/// After `--` the request is text again, even when it starts with a hyphen: reaching the missing
/// workspace (exit 1) proves the parser accepted it instead of calling it a flag (exit 2).
#[test]
fn task_accepts_a_request_after_a_double_dash() {
    let output = run_cli(&[
        "task",
        "--model",
        "modelo-de-teste",
        "--workspace",
        "pasta-que-nao-existe-cd-ai",
        "--",
        "--arrume a soma",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let text = stderr(&output);
    assert!(text.contains("pasta não encontrada"), "{text}");
}

#[test]
fn task_with_missing_workspace_exits_1_with_message() {
    let output = run_cli(&[
        "task",
        "--model",
        "modelo-de-teste",
        "--workspace",
        "pasta-que-nao-existe-cd-ai",
        "arrume a soma",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let text = stderr(&output);
    assert!(text.contains("pasta não encontrada"), "{text}");
}

#[test]
fn task_with_unknown_resume_id_exits_1_with_message() {
    let data = TempDir::new("data");
    let workspace = TempDir::new("ws");
    let output = run_cli_env(
        &[
            "task",
            "--model",
            "modelo-de-teste",
            "--workspace",
            workspace.as_str(),
            "--resume",
            "task_nao_existe",
        ],
        &[
            ("OLLAMA_HOST", "http://127.0.0.1:9"),
            ("CD_AI_DATA_DIR", data.as_str()),
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    let text = stderr(&output);
    assert!(
        text.contains("não foi possível retomar task_nao_existe"),
        "{text}"
    );
    assert!(text.contains("tarefa não encontrada"), "{text}");
}

#[test]
fn task_with_unreachable_ollama_fails_cleanly() {
    let data = TempDir::new("data");
    let workspace = TempDir::new("ws");
    let output = run_cli_env(
        &[
            "task",
            "--model",
            "modelo-de-teste",
            "--workspace",
            workspace.as_str(),
            "--max-iterations",
            "1",
            "liste a raiz",
        ],
        &[
            ("OLLAMA_HOST", "http://127.0.0.1:9"),
            ("CD_AI_DATA_DIR", data.as_str()),
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    let text = stderr(&output);
    assert!(text.contains("falhou"), "{text}");
    assert!(text.contains("erro do modelo"), "{text}");
    // The task was persisted under CD_AI_DATA_DIR, never in the user's real data directory.
    let tasks = data.path.join("tasks");
    let saved = fs::read_dir(&tasks)
        .expect("diretório de tarefas")
        .filter_map(Result::ok)
        .count();
    assert_eq!(saved, 1, "esperava uma tarefa em {}", tasks.display());
}

#[test]
fn continue_and_resume_together_exit_2() {
    let output = run_cli(&["task", "--model", "m", "--resume", "task_1", "--continue"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("escolha um"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn continue_without_a_previous_task_exits_1_with_message() {
    let data = TempDir::new("data-cont");
    let workspace = TempDir::new("ws-cont");
    let output = run_cli_env(
        &[
            "task",
            "--model",
            "modelo-de-teste",
            "--workspace",
            workspace.as_str(),
            "--continue",
            "continue o que você fazia",
        ],
        &[
            ("OLLAMA_HOST", "http://127.0.0.1:9"),
            ("CD_AI_DATA_DIR", data.as_str()),
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("não há tarefa anterior"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn help_mentions_continue() {
    let output = run_cli(&["--help"]);
    assert!(stdout(&output).contains("--continue"));
}

#[test]
fn help_mentions_permission_mode() {
    let text = stdout(&run_cli(&["--help"]));
    assert!(text.contains("full-access"), "{text}");
}

#[test]
fn help_mentions_history_and_rollback() {
    let text = stdout(&run_cli(&["--help"]));
    assert!(text.contains("cd-ai history"), "{text}");
    assert!(text.contains("cd-ai rollback"), "{text}");
    assert!(text.contains("--force"), "{text}");
}

#[test]
fn help_mentions_memory_and_trajectories() {
    let text = stdout(&run_cli(&["--help"]));
    assert!(text.contains("cd-ai memory"), "{text}");
    assert!(text.contains("memory list"), "{text}");
    assert!(text.contains("--trajectories"), "{text}");
}

#[test]
fn memory_without_subcommand_exits_2() {
    let output = run_cli(&["memory"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("faltou o subcomando de memory"));
}

#[test]
fn history_unknown_flag_exits_2() {
    let output = run_cli(&["history", "--bogus"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn rollback_without_id_exits_2() {
    let output = run_cli(&["rollback"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("faltou o id da tarefa"));
}

#[test]
fn help_mentions_eval() {
    for flag in ["--help", "-h"] {
        let text = stdout(&run_cli(&[flag]));
        assert!(text.contains("cd-ai eval"), "{flag} não cita eval: {text}");
        assert!(
            text.contains("--scripted"),
            "{flag} não cita --scripted: {text}"
        );
    }
}

#[test]
fn eval_without_model_or_scripted_exits_2() {
    let output = run_cli(&["eval"]);
    assert_eq!(output.status.code(), Some(2));
    let text = stderr(&output);
    assert!(text.contains("faltou --model"), "{text}");
}

#[test]
fn eval_scripted_and_model_together_exit_2() {
    let output = run_cli(&["eval", "--scripted", "--model", "qwen3"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("--scripted não aceita --model"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn eval_scripted_with_missing_suite_exits_1() {
    let data = TempDir::new("eval-missing");
    let output = run_cli_env(
        &[
            "eval",
            "--scripted",
            "--suite",
            "pasta-de-eval-que-nao-existe-cd-ai",
        ],
        &[("CD_AI_DATA_DIR", data.as_str())],
    );
    assert_eq!(output.status.code(), Some(1));
    let text = stderr(&output);
    assert!(text.contains("pasta de tarefas não encontrada"), "{text}");
}

/// Runs the real `soma` fixture through the CLI harness. Needs `bun` on PATH (the verify gate
/// already does). Does not talk to Ollama.
#[test]
fn eval_scripted_soma_exits_0() {
    let suite = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../evals");
    let suite = suite.canonicalize().expect("evals na raiz do repo");
    let data = TempDir::new("eval-soma");
    let out = data.path.join("soma.json");
    let output = run_cli_env(
        &[
            "eval",
            "--scripted",
            "--suite",
            suite.to_str().expect("utf-8"),
            "--task",
            "soma",
            "--out",
            out.to_str().expect("utf-8"),
        ],
        &[("CD_AI_DATA_DIR", data.as_str())],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}\nstdout: {}",
        stderr(&output),
        stdout(&output)
    );
    let text = stderr(&output);
    assert!(text.contains("taxa de sucesso"), "{text}");
    assert!(out.is_file(), "relatório em {}", out.display());
}
