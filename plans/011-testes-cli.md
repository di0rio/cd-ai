# Plano 011: a CLI ganha testes de caracterização antes do próximo comando

> **Instruções ao executor:** siga o plano passo a passo. Rode cada comando de verificação e confirme o resultado esperado antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Ao terminar, atualize a linha deste plano em `plans/README.md`.
>
> **Drift check (rode primeiro):** compare o trecho de "Estado atual" com `apps/cli/src/main.rs`. Se não bater, trate como STOP condition.

## Status

- **Prioridade:** P1
- **Esforço:** S
- **Risco:** LOW
- **Depende de:** nenhum (mas deve vir **antes** do `plans/005-chat-streaming-cancelavel.md`, que reescreve `main.rs`)
- **Categoria:** tests
- **Planejado em:** commit `f6e7764`, 2026-09-11

## Por que isso importa

`apps/cli/src/main.rs` (23 linhas) parseia argumentos com três branches (`--version`/`-V`, ninguém/`--help`/`-h`, desconhecido → `ExitCode::from(2)`) e **não tem um único teste**. O plano 005 vai reescrever esse arquivo (async, subcomando `chat`, `clap` fica proibido, parse à mão). Sem uma rede de segurança, uma regressão silenciosa no `--version` ou num exit code erra na cara de um usuário — e a CLI é também o futuro runner do eval (SPEC §26 e Fase 6). Estes testes de caracterização travam o comportamento atual antes dessa reescrita.

## Estado atual

`apps/cli/src/main.rs`, arquivo inteiro:

```rust
use std::process::ExitCode;

const USAGE: &str = "uso: cd-ai [--version | --help]";

fn main() -> ExitCode {
    let arg = std::env::args().nth(1);

    match arg.as_deref() {
        Some("--version" | "-V") => {
            let info = agent_core::app_info();
            println!("{} {}", info.name, info.version);
            ExitCode::SUCCESS
        }
        None | Some("--help" | "-h") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("argumento desconhecido: {other}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
```

- `apps/cli/Cargo.toml`: binário `cd-ai` (nome com hífen), dependência só `agent-core.workspace = true`.
- Convenções (de `crates/agent-core/src/lib.rs` e da decisão 0003): identificadores e comentários em inglês, mensagens de **saída e erro em pt-BR** (o USAGE e o `eprintln!` acima são pt-BR de propósito). Testes ficam em `#[cfg(test)]` ou, para binário, em `tests/`.
- Não há runner de testes de componente no frontend; isso **não** afeta este plano, que testa o binário Rust.

## Comandos que você vai precisar

| Propósito | Comando | Esperado |
|-----------|---------|----------|
| Testes da CLI | `cargo test -p cd-ai-cli` | `test result: ok`, com os testes novos listados |
| Gate completo | `bun run verify` | exit 0 |

Se o `cargo` não for encontrado no PATH: `export PATH="$HOME/.cargo/bin:$PATH"` (Linux/macOS) ou `$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"` (Windows).

> **Atenção:** se `cargo` não estiver instalado na sua máquina, este plano não tem como ser executado aqui. O ambiente de desenvolvimento é Windows com Rust (docs/audit/fase-0-ambiente.md). Nesse caso, reporte; não improvise um outro runner.

## Escopo

**Dentro do escopo:**

- `apps/cli/tests/cli.rs` (criar)

**Fora do escopo:**

- `apps/cli/src/main.rs` — **não modificar** (a extração de função para testar internamente é possível, mas desnecessária e conflita com o plano 005, que reescreve o arquivo).
- Qualquer outra crate, incluindo `agent-core`.
- Adicionar `clap`, `assert_cmd` ou qualquer dependência nova.

## Git

- Branch: `advisor/011-cli-tests`.
- Um commit, mensagem no estilo `test(cli): characterize argument handling`.
- Não faça push.

## Passos

### Passo 1: criar `apps/cli/tests/cli.rs`

Crie o arquivo com exatamente esta estrutura (teste de integração que executa o binário compilado; o Cargo expõe o caminho em `CARGO_BIN_EXE_cd-ai` automaticamente para testes em `tests/` com binário `cd-ai`):

```rust
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
    assert_eq!(stdout(&output), format!("cd-ai {}\n", env!("CARGO_PKG_VERSION")));
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
```

Regras:

- Não use `env::args` no teste nem chame `main` diretamente. O `CARGO_BIN_EXE_...` é o contrato.
- `assert_eq!(output.status.code(), Some(2))` — o `ExitCode::from(2)` vira isso no processo filho.
- Nomes de teste em inglês (convenção do repo), mensagens de asserção em pt-BR.

**Verificar:** `cargo test -p cd-ai-cli` → `4 passed`, `0 failed`.

### Passo 2: gate completo

Rode `bun run verify` (que inclui `cargo test --workspace` e `cargo fmt --all --check`).

**Verificar:** exit 0. `cargo fmt --all --check` precisa aceitar o arquivo novo (formatação padrão `rustfmt`).

## Plano de testes

Os 4 testes do Passo 1:

1. `version_prints_name_and_crate_version` — happy path + regressão da flag `--version`.
2. `no_args_prints_usage_and_succeeds` — sem argumentos (default).
3. `help_flags_print_usage_and_succeed` — `--help` e `-h` (duas flags equivalentes).
4. `unknown_argument_exits_with_code_2_and_message_on_stderr` — o caso do exit 2 + USAGE no stderr.

Nenhuma dependência nova (sem `assert_cmd`). O binário real é exercitado, então nenhum mock.

## Critérios de pronto

- [ ] `cargo test -p cd-ai-cli` → 4 testes passando
- [ ] `bun run verify` exit 0
- [ ] `git status` mostra apenas `apps/cli/tests/cli.rs` e `git diff` do `Cargo.lock` vazio (não deve haver dependência nova)
- [ ] A linha deste plano em `plans/README.md` está atualizada

## STOP conditions

- O trecho de "Estado atual" não bate com `apps/cli/src/main.rs` (drift).
- `env!("CARGO_BIN_EXE_cd-ai")` não compila (nome com hífen com comportamento inesperado em alguma versão do Cargo). Reporte a mensagem exata; **não** troque por uma extração de função em `main.rs` sem reportar.
- `cargo test -p cd-ai-cli` falha **antes** de você criar o arquivo novo (base quebrada, ex.: binário não compila).
- O teste 4 exige abrir um shell interativo (não deve: o `exit code 2` do processo filho vem do `ExitCode`, sem shell).

## Notas de manutenção

- **O plano 005 reescreve `main.rs`** (async, subcomando `chat`, novo `USAGE`). Ao fazê-lo, mantenha verdes estes 4 testes: o `USAGE` novo deve continuar contendo "uso: cd-ai", `--version` deve continuar imprimindo `cd-ai X.Y.Z`, e argumento desconhecido deve continuar saindo com código 2 e mensagem no stderr. Se o 005 adicionar subcomando `chat`, os testes de `--help`/`--bogus` continuam válidos (`--bogus` vira "argumento desconhecido").
- Quando existir subcomando (`chat`), estenda este arquivo com casos por subcomando no mesmo padrão `Command::new(env!("CARGO_BIN_EXE_cd-ai"))`.
- Código de saída 2 para "uso incorreto" é convenção de CLI (0 sucesso, 1 erro de execução, 2 uso): não o troque silenciosamente.