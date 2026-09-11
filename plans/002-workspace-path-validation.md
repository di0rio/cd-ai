# Plano 002: o agent-core valida qualquer path contra o workspace aberto

> **Instruções ao executor:** siga o plano passo a passo. Rode cada comando de verificação e confirme o resultado esperado antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Ao terminar, atualize a linha deste plano em `plans/README.md`.
>
> **Drift check (rode primeiro):** compare os trechos de "Estado atual" com `crates/agent-core/src/lib.rs` e `crates/agent-core/Cargo.toml`. Se não baterem, trate como STOP condition.

## Status

- **Prioridade:** P1
- **Esforço:** M
- **Risco:** MED (código de segurança)
- **Depende de:** `plans/001-verify-e-guia-de-agentes.md`
- **Categoria:** direction (Fase 2) / security
- **Planejado em:** base `158f609` + árvore de trabalho de 2026-09-11

## Por que isso importa

Todas as ferramentas futuras do agente (ler, escrever, buscar, executar) recebem paths que vêm do modelo. O workspace escolhido pelo usuário é a fronteira: nenhum path pode escapar dele, nem por `../`, nem por path absoluto, nem por symlink.

Isso precisa existir, com testes, **antes** de qualquer ferramenta que toque o disco. Este plano entrega só o módulo e os testes, sem UI.

## Estado atual

- `crates/agent-core/src/lib.rs` hoje (26 linhas) contém só `AppInfo`:

  ```rust
  use serde::Serialize;

  #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
  pub struct AppInfo {
      pub name: &'static str,
      pub version: &'static str,
  }

  pub fn app_info() -> AppInfo { ... }

  #[cfg(test)]
  mod tests { ... }
  ```

- `crates/agent-core/Cargo.toml`:

  ```toml
  [package]
  name = "agent-core"
  version.workspace = true
  edition.workspace = true

  [dependencies]
  serde.workspace = true
  ```

- A edição Rust é 2024 e o workspace usa `resolver = "3"`.
- **Convenções:**
  - Testes ficam no próprio arquivo, em `#[cfg(test)] mod tests`, como em `lib.rs`.
  - Identificadores e comentários em inglês.
  - Mensagens de erro vistas pelo usuário em pt-BR.
  - Sem `thiserror`: implemente `Display` à mão.
- **Regras que este código deve cumprir** (SPEC §20.1 e decisão 0001):
  - Validar path absoluto e canônico.
  - Resolver symlinks e checar se o destino continua dentro do workspace.
  - Bloquear `../`.
  - Usar `std::path` para tudo, sem assumir separador nem letra de drive.

## Comandos que você vai precisar

| Propósito | Comando | Esperado |
|-----------|---------|----------|
| Testes do módulo | `cargo test -p agent-core workspace` | `test result: ok`, com os testes novos listados |
| Gate completo | `bun run verify` | exit 0 |

No Windows, se o `cargo` não for encontrado: `$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"`.

## Escopo

**Dentro do escopo:**

- `crates/agent-core/src/workspace.rs` (criar)
- `crates/agent-core/src/lib.rs` (só adicionar `pub mod workspace;`)
- `crates/agent-core/Cargo.toml` (só a dev-dependency `tempfile`)

**Fora do escopo:**

- `src-tauri/` e `apps/`: a integração com a UI é o plano 003.
- Qualquer nova dependência de runtime (como `dunce` ou `path-clean`).

## Git

- Branch: `advisor/002-workspace`.
- Mensagem no estilo `feat(core): add workspace path validation`.
- Não faça push.

## Passos

### Passo 1: adicionar `tempfile` como dev-dependency

Rode `cargo add --dev tempfile -p agent-core`.

**Verificar:** `crates/agent-core/Cargo.toml` passa a ter uma seção `[dev-dependencies]` com `tempfile`.

### Passo 2: criar `crates/agent-core/src/workspace.rs`

Use exatamente esta implementação. O algoritmo é a parte de segurança, então **não o "simplifique"**:

```rust
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// A folder the user opened. Every path the agent touches must resolve inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    root: PathBuf,
}

#[derive(Debug)]
pub enum WorkspaceError {
    NotFound(PathBuf),
    NotADirectory(PathBuf),
    OutsideWorkspace(PathBuf),
    Io(io::Error),
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(path) => write!(f, "pasta não encontrada: {}", path.display()),
            Self::NotADirectory(path) => write!(f, "não é uma pasta: {}", path.display()),
            Self::OutsideWorkspace(path) => write!(f, "caminho fora do workspace: {}", path.display()),
            Self::Io(error) => write!(f, "erro de E/S: {error}"),
        }
    }
}

impl std::error::Error for WorkspaceError {}

impl Workspace {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        let path = path.as_ref();
        let root = path.canonicalize().map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => WorkspaceError::NotFound(path.to_path_buf()),
            _ => WorkspaceError::Io(error),
        })?;
        if !root.is_dir() {
            return Err(WorkspaceError::NotADirectory(path.to_path_buf()));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves a workspace-relative (or absolute) path to a canonical path inside the workspace.
    /// Paths that do not exist yet are allowed, so callers can create files.
    pub fn resolve(&self, path: impl AsRef<Path>) -> Result<PathBuf, WorkspaceError> {
        let requested = path.as_ref();
        let outside = || WorkspaceError::OutsideWorkspace(requested.to_path_buf());

        // 1. Lexical normalization. An absolute input starts from itself and must still land inside the root.
        let mut lexical = if requested.is_absolute() { PathBuf::new() } else { self.root.clone() };
        for component in requested.components() {
            match component {
                Component::Prefix(_) | Component::RootDir | Component::Normal(_) => lexical.push(component),
                Component::CurDir => {}
                Component::ParentDir => {
                    if !lexical.pop() {
                        return Err(outside());
                    }
                }
            }
        }
        if !lexical.starts_with(&self.root) {
            return Err(outside());
        }

        // 2. Let the OS resolve symlinks on the deepest part that exists; the rest are plain names to be created.
        let mut existing = lexical;
        let mut missing = Vec::new();
        while fs::symlink_metadata(&existing).is_err() {
            match existing.file_name() {
                Some(name) => missing.push(name.to_os_string()),
                None => return Err(outside()),
            }
            existing.pop();
        }
        // A dangling symlink fails to canonicalize and is refused: writing through it could land anywhere.
        let canonical = existing.canonicalize().map_err(|_| outside())?;
        if !canonical.starts_with(&self.root) {
            return Err(outside());
        }
        Ok(missing.into_iter().rev().fold(canonical, |path, name| path.join(name)))
    }
}
```

**Verificar:** o arquivo existe (o build é verificado no passo 4).

### Passo 3: registrar o módulo

Em `crates/agent-core/src/lib.rs`, adicione `pub mod workspace;` na primeira linha, antes do `use serde::Serialize;`.

**Verificar:** `cargo build -p agent-core` sai com exit 0.

### Passo 4: escrever os testes

No fim de `workspace.rs`, adicione `#[cfg(test)] mod tests` com `use super::*;` e `use tempfile::tempdir;`, cobrindo **cada** caso abaixo, um `#[test]` por caso:

1. `open_missing_folder_is_not_found`: `Workspace::open(dir.path().join("nope"))` retorna `Err(WorkspaceError::NotFound(_))`.
2. `open_file_is_not_a_directory`: crie um arquivo e abra-o; retorna `Err(WorkspaceError::NotADirectory(_))`.
3. `resolves_existing_file_inside`: crie `src/main.rs`; `resolve("src/main.rs")` é `Ok(p)` com `p.starts_with(ws.root())` e `p.ends_with("main.rs")`.
4. `resolves_root_for_dot`: `resolve(".")` é `Ok(p)` com `p == ws.root()`.
5. `allows_parent_that_stays_inside`: crie `src/`; `resolve("src/../README.md")` é `Ok(p)` com `p == ws.root().join("README.md")`.
6. `rejects_parent_escape`: `resolve("../outside.txt")` retorna `Err(WorkspaceError::OutsideWorkspace(_))`.
7. `rejects_nested_parent_escape`: `resolve("a/../../x")` retorna `OutsideWorkspace`.
8. `rejects_absolute_path_outside`: com um segundo `tempdir()` chamado `other`, `resolve(other.path().join("x"))` retorna `OutsideWorkspace`.
9. `allows_new_nested_file`: `resolve("new/dir/file.txt")` (nada disso existe) é `Ok(p)` com `p == ws.root().join("new").join("dir").join("file.txt")`.
10. `rejects_escape_through_missing_dirs`: `resolve("new/../../x")` retorna `OutsideWorkspace`.
11. `#[cfg(unix)] rejects_symlink_that_escapes`: crie `std::os::unix::fs::symlink(other.path(), root.join("link"))`; `resolve("link/secret.txt")` retorna `OutsideWorkspace`.
12. `#[cfg(unix)] rejects_dangling_symlink`: crie um symlink `root/dangling` apontando para `other.path().join("missing")`; `resolve("dangling")` retorna `OutsideWorkspace`.

Use `assert!(matches!(result, Err(WorkspaceError::OutsideWorkspace(_))))` para os casos de erro.

**Verificar:** `cargo test -p agent-core workspace` roda com `0 failed`. No Windows rodam 10 testes; no Linux, 12.

### Passo 5: gate completo

**Verificar:** `bun run verify` sai com exit 0 (o `cargo fmt --all --check` inclui o arquivo novo).

## Plano de testes

Os 12 casos do Passo 4, com o padrão estrutural de `crates/agent-core/src/lib.rs` (módulo `tests` no mesmo arquivo).

## Critérios de pronto

- [ ] `cargo test -p agent-core workspace` passa, com pelo menos 10 testes novos
- [ ] `bun run verify` sai com exit 0
- [ ] `git status` mostra apenas `crates/agent-core/src/workspace.rs`, `crates/agent-core/src/lib.rs`, `crates/agent-core/Cargo.toml` e `Cargo.lock`
- [ ] A linha deste plano em `plans/README.md` está atualizada

## STOP conditions

- O caso 3 ou o 9 falha no Windows por causa do prefixo `\\?\`. Reporte a saída exata; não troque por outro algoritmo.
- Algum teste de escape (6, 7, 8, 10, 11, 12) **passa** num `Ok(...)`. Isso é uma falha de segurança: pare.
- O `clippy -D warnings` exige mudar a lógica (não só o estilo) do `resolve`.

## Notas de manutenção

- **Todo acesso a disco das ferramentas futuras deve passar por `Workspace::resolve` e usar o path que ele retorna, nunca o input cru.** A garantia só vale para o path devolvido.
- No Windows, o `root` fica no formato canônico `\\?\C:\...`. Um input absoluto sem esse prefixo é recusado mesmo estando dentro do workspace. Isso é seguro de propósito: o agente deve usar paths relativos. Para exibir o path na UI, remova o prefixo só na exibição (plano 003).
- Os testes de symlink só rodam no Unix. No Windows, criar symlink exige privilégio. Adicionar testes com junction quando houver CI no Linux ou Windows (Fase 7).
- Existe uma corrida entre resolver e usar o path (TOCTOU). Ela fica para a Fase 7 (sandbox), que é a defesa real contra processos concorrentes.
