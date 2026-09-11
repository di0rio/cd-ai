use std::process::{Command, Output};

fn run_cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cd-ai"))
        .args(args)
        .output()
        .expect("falha ao executar o binário cd-ai")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout não é UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr não é UTF-8")
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
fn unknown_argument_exits_with_code_2_and_message_on_stderr() {
    let output = run_cli(&["--bogus"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("argumento desconhecido: --bogus"));
    assert!(stderr(&output).contains("uso: cd-ai"));
}
