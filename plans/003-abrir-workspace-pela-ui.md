# Plano 003: o usuário abre um workspace pela UI, e o core valida e guarda

> **Instruções ao executor:** siga o plano passo a passo. Rode cada comando de verificação e confirme o resultado esperado antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Ao terminar, atualize a linha deste plano em `plans/README.md`.
>
> **Drift check (rode primeiro):** compare os trechos de "Estado atual" com os arquivos vivos. Se não baterem, STOP. Confirme também que `crates/agent-core/src/workspace.rs` existe (plano 002). Se não existir, STOP.

## Status

- **Prioridade:** P1
- **Esforço:** M
- **Risco:** LOW
- **Depende de:** `plans/002-workspace-path-validation.md`
- **Categoria:** direction (Fase 2)
- **Planejado em:** base `158f609` + árvore de trabalho de 2026-09-11

## Por que isso importa

O primeiro botão da interface ("Abrir workspace") está desabilitado. Sem workspace aberto, nenhuma funcionalidade seguinte (ferramentas, agente) tem onde operar.

O diálogo de pasta deve ser aberto **pelo Rust**, e a validação (`Workspace::open`) acontece no Rust. Assim a webview nunca escolhe paths sozinha: a fronteira de confiança continua no Rust (decisão 0003).

## Estado atual

- `src-tauri/src/lib.rs` (inteiro):

  ```rust
  #[tauri::command]
  fn app_info() -> agent_core::AppInfo {
      agent_core::app_info()
  }

  pub fn run() {
      tauri::Builder::default()
          .invoke_handler(tauri::generate_handler![app_info])
          .run(tauri::generate_context!())
          .expect("error while running tauri application");
  }
  ```

- `src-tauri/Cargo.toml`, dependências:

  ```toml
  agent-core.workspace = true
  tauri = { version = "2.11.3", features = [] }
  ```

- `apps/desktop/src/lib/ipc.ts` (padrão de wrapper IPC a seguir):

  ```ts
  import { invoke } from "@tauri-apps/api/core";

  // Mirrors agent_core::AppInfo. The Rust side is the source of truth.
  export type AppInfo = {
    name: string;
    version: string;
  };

  export function getAppInfo(): Promise<AppInfo> {
    return invoke<AppInfo>("app_info");
  }
  ```

- `apps/desktop/src/components/sidebar.tsx`, linhas 53–64: botão de workspace desabilitado.

  ```tsx
  <button
    type="button"
    disabled
    title="A seleção de pasta chega na próxima fase"
    className={`${rowButton} text-ink hover:bg-canvas`}
  >
    <Icon name="folder" className="size-4 text-ink-faint" />
    <span className="truncate">{workspace ?? "Abrir workspace"}</span>
  </button>
  ```

- `apps/desktop/src/components/empty-workspace.tsx`, linhas 27–36: o botão primário "Abrir workspace" está desabilitado, com o mesmo `title`.
- `apps/desktop/src/components/app-shell.tsx`, linha 48: `workspace={task?.workspace ?? null}` é passado para a `Sidebar`. A linha 70 mostra `Nenhum workspace aberto` no header quando não há tarefa.
- Padrão para chamar o core com fallback fora do Tauri: `apps/desktop/src/components/core-status.tsx`. Ele chama o IPC dentro de `useEffect` e trata a rejeição com um estado `unavailable`.
- **Design** (ver `apps/desktop/DESIGN.md`):
  - O botão primário usa `bg-accent text-accent-ink`, `h-9`, `rounded-lg`.
  - Texto de erro usa `text-bad`.
  - Não crie componentes visuais novos além do necessário.
- A API do plugin de diálogo (tauri-plugin-dialog v2, pelo lado Rust):
  - O plugin é registrado com `.plugin(tauri_plugin_dialog::init())`.
  - Com `use tauri_plugin_dialog::DialogExt;`, `app.dialog().file().blocking_pick_folder()` retorna `Option<FilePath>`.
  - `FilePath::into_path()` retorna `Result<PathBuf, _>`.
  - `blocking_pick_folder` **não** pode rodar na thread principal. Por isso o command precisa ser `async`.

## Comandos que você vai precisar

| Propósito | Comando | Esperado |
|-----------|---------|----------|
| Adicionar o plugin | `cargo add tauri-plugin-dialog -p cd-ai-desktop` | exit 0 |
| Testes do core | `cargo test -p agent-core` | `0 failed` |
| Gate | `bun run verify` | exit 0 |
| Build do app | `bun tauri build --no-bundle` | `Built application at:` |

## Escopo

**Dentro do escopo:**

- `crates/agent-core/src/workspace.rs` (adicionar `WorkspaceInfo` e `Workspace::info`)
- `src-tauri/Cargo.toml` e `src-tauri/src/lib.rs`
- `apps/desktop/src/lib/ipc.ts`
- `apps/desktop/src/components/app-shell.tsx`, `sidebar.tsx` e `empty-workspace.tsx`

**Fora do escopo:**

- `src-tauri/capabilities/default.json`: o JS não chama o plugin de diálogo, então não precisa de permissão. As permissões são o plano 006.
- A lógica de `Workspace::resolve` (plano 002).
- Persistir workspaces recentes em disco. Fica para depois.
- `demo-session.ts` e o modo `?demo`.

## Git

- Branch: `advisor/003-open-workspace`.
- Mensagem no estilo `feat: open workspace from the UI`.
- Não faça push.

## Passos

### Passo 1: `WorkspaceInfo` no core

Em `crates/agent-core/src/workspace.rs`, adicione:

```rust
use serde::Serialize;

/// What the UI needs to show an open workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceInfo {
    pub name: String,
    pub root: String,
}

impl Workspace {
    pub fn info(&self) -> WorkspaceInfo {
        let root = self.root.to_string_lossy();
        WorkspaceInfo {
            name: self.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.to_string()),
            // Strip the Windows verbatim prefix for display only; comparisons keep the canonical form.
            root: root.trim_start_matches(r"\\?\").to_string(),
        }
    }
}
```

Junte o `impl` com o existente se preferir. Adicione o teste `info_uses_folder_name`: abrir um `tempdir()` que contém a subpasta `meu-projeto`; `info().name == "meu-projeto"` e `info().root` não começa com `\\?\`.

**Verificar:** `cargo test -p agent-core workspace` roda com `0 failed`.

### Passo 2: plugin e estado no `src-tauri`

1. Rode `cargo add tauri-plugin-dialog -p cd-ai-desktop`.
2. Em `src-tauri/src/lib.rs`:
   - Crie `struct AppState { workspace: std::sync::Mutex<Option<agent_core::workspace::Workspace>> }`.
   - Registre `.manage(AppState { workspace: Default::default() })`.
   - Registre `.plugin(tauri_plugin_dialog::init())`.
3. Adicione os commands abaixo e registre-os no `generate_handler![app_info, open_workspace, current_workspace]`:

```rust
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
async fn open_workspace(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<agent_core::workspace::WorkspaceInfo>, String> {
    // Runs off the main thread (async command), where the blocking dialog is allowed.
    let Some(folder) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None); // user cancelled
    };
    let path = folder.into_path().map_err(|error| error.to_string())?;
    let workspace = agent_core::workspace::Workspace::open(path).map_err(|error| error.to_string())?;
    let info = workspace.info();
    *state.workspace.lock().map_err(|_| "estado do workspace corrompido".to_string())? = Some(workspace);
    Ok(Some(info))
}

#[tauri::command]
fn current_workspace(state: tauri::State<'_, AppState>) -> Option<agent_core::workspace::WorkspaceInfo> {
    state.workspace.lock().ok()?.as_ref().map(|workspace| workspace.info())
}
```

**Verificar:** `cargo clippy --workspace --all-targets -- -D warnings` sai com exit 0.

### Passo 3: wrappers IPC

Em `apps/desktop/src/lib/ipc.ts`, adicione, seguindo o padrão existente:

```ts
// Mirrors agent_core::workspace::WorkspaceInfo.
export type WorkspaceInfo = { name: string; root: string };

export function openWorkspace(): Promise<WorkspaceInfo | null> {
  return invoke<WorkspaceInfo | null>("open_workspace");
}

export function currentWorkspace(): Promise<WorkspaceInfo | null> {
  return invoke<WorkspaceInfo | null>("current_workspace");
}
```

**Verificar:** `bun run --cwd apps/desktop typecheck` sai com exit 0.

### Passo 4: ligar a UI

1. **`app-shell.tsx`:**
   - Adicione o estado `const [workspace, setWorkspace] = useState<WorkspaceInfo | null>(null)`.
   - Adicione o estado `const [workspaceError, setWorkspaceError] = useState<string | null>(null)`.
   - Num `useEffect` de montagem, chame `currentWorkspace().then(setWorkspace, () => {})`. Fora do Tauri a chamada rejeita e isso é esperado.
   - Crie uma função `handleOpenWorkspace`:
     - Chama `openWorkspace()`.
     - Em sucesso com valor, faz `setWorkspace(info)` e `setWorkspaceError(null)`.
     - Em `null` (usuário cancelou), não faz nada.
     - Em rejeição, faz `setWorkspaceError(String(error))`.
   - Passe `workspace={workspace?.name ?? task?.workspace ?? null}` e `onOpenWorkspace={handleOpenWorkspace}` para a `Sidebar`.
   - No header, troque `Nenhum workspace aberto` por `{workspace ? workspace.name : "Nenhum workspace aberto"}`.
   - Passe `workspace`, `error={workspaceError}` e `onOpen={handleOpenWorkspace}` para o `EmptyWorkspace`.
2. **`sidebar.tsx`:**
   - Adicione a prop `onOpenWorkspace: () => void`.
   - No botão de workspace, remova `disabled` e o `title`, e adicione `onClick={onOpenWorkspace}`.
3. **`empty-workspace.tsx`:**
   - Receba as props `{ workspace: WorkspaceInfo | null; error: string | null; onOpen: () => void }`.
   - O botão perde `disabled` e `title` e ganha `onClick={onOpen}`.
   - Abaixo do botão, se houver `error`, renderize `<p role="alert" className="mt-3 text-sm text-bad">{error}</p>`.
   - Se `workspace` existir, troque:
     - o título por `workspace.name`;
     - o parágrafo por `<code className="text-ink-muted">{workspace.root}</code>`;
     - o rótulo do botão por `Trocar workspace`.
   - A lista de três fatos permanece.

**Verificar:** `bun run verify` sai com exit 0.

### Passo 5: build do app

**Verificar:** `bun tauri build --no-bundle` imprime `Built application at:` e sai com exit 0.

### Passo 6: verificação manual (humana)

Abra `target/release/cd-ai-desktop.exe`, clique em "Abrir workspace" e escolha uma pasta. O resultado esperado:

- a sidebar e o header mostram o nome da pasta;
- o estado vazio mostra o path sem `\\?\`.

Depois cancele o diálogo uma vez: nada deve mudar e nenhum erro deve aparecer.

## Plano de testes

- **Rust:** `info_uses_folder_name` (Passo 1).
- **Frontend:** sem teste automatizado, porque a UI depende do IPC. A verificação é o Passo 6.

## Critérios de pronto

- [ ] `bun run verify` sai com exit 0
- [ ] `bun tauri build --no-bundle` sai com exit 0
- [ ] `grep -rn "chega na próxima fase" apps/desktop/src` não retorna nenhuma linha
- [ ] `git status` mostra modificados apenas os arquivos do escopo, mais `Cargo.lock`
- [ ] A linha deste plano em `plans/README.md` está atualizada

## STOP conditions

- `blocking_pick_folder` ou `FilePath::into_path` não existem com essa assinatura na versão instalada do `tauri-plugin-dialog`. Reporte a versão e a assinatura real.
- O app trava ao abrir o diálogo (sinal de que o command rodou na thread principal).
- A mudança exige alterar `src-tauri/capabilities/default.json`.

## Notas de manutenção

- O plano 006 vai colocar `open_workspace` e `current_workspace` sob ACL. Se você renomear algum dos dois, atualize esse plano.
- O workspace fica só na memória. Reiniciar o app fecha o workspace. Persistir workspaces recentes é uma task futura, e deve guardar o path no diretório de dados do app, nunca no projeto.
- Toda ferramenta futura deve pegar o `Workspace` do `AppState` e usar `resolve`, nunca montar paths a partir do `WorkspaceInfo.root`, que existe só para exibição.
