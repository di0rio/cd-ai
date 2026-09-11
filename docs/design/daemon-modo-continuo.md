# Design: Daemon — modo contínuo de validação e tarefas automáticas

Status: **proposta de design** (2026-09-11). A validar antes de virar plano de implementação.

## 1. Objetivo

O cd-ai observa os workspaces do usuário enquanto está aberto e, quando detecta que algo quebrou (build, typecheck, lint, testes), cria uma tarefa automaticamente, com contexto completo, e a abre na sidebar. O agente "fica por perto" em vez de só responder a comandos.

Esse é o primeiro passo do modo contínuo ("pocando 100% do tempo") sem custo financeiro ou quota artificial de tokens — ele só pausa por segurança.

## 2. Contexto

- O app é local-first e exclusivamente local (decisão 0001/0004). Não existe custo financeiro nem quota artificial de tokens; ele pausa por segurança, não por billing.
- O Core (agent-core) já terá `Workspace::resolve` (plano 002), `run_command` (Fase 4/plano 004/012) e o TaskManager para tarefas existentes.
- O Verifier (SPEC §13.1) define validação em estágios progressivos: parse → diff checks → typecheck → lint → testes direcionados → build/suíte completa.
- A janela de contexto é restrição de hardware (32k no qwen3-coder:30b), não de política (decisão 0008).
- Config de dados do app vive no diretório de dados do app, **não** no repositório (SPEC §30).

## 3. Arquitetura

O daemon roda dentro do processo Tauri (não é serviço separado na v1). Ele observa **todos os workspaces abertos** simultaneamente.

```
┌──────────────────────────────────────────────────┐
│  Tauri App                                       │
│  ┌───────────────┐    ┌───────────────────────┐  │
│  │ Frontend      │◄───│ IPC Channel<DaemonEvent>│ │
│  │ Next.js       │    └──────────┬────────────┘  │
│  └───────────────┘               │               │
│  ┌───────────────────────────────▼─────────────┐ │
│  │ agent-core (Rust)                           │ │
│  │  ┌───────────────────────────────────────┐  │ │
│  │  │ DaemonManager                         │  │ │
│  │  │  workspace_1 ── WorkspaceWatcher      │  │ │
│  │  │  workspace_2 ── WorkspaceWatcher      │  │ │
│  │  │  workspace_N ── WorkspaceWatcher      │  │ │
│  │  └───────────────┬───────────────────────┘  │ │
│  │                  │                          │ │
│  │            ├─────────────────┐              │ │
│  │            ▼                 ▼              │ │
│  │  ValidationExecutor  TaskFactory             │ │
│  │  (§13.1 stages)      (cria Task)            │ │
│  │            │                 │              │ │
│  │            ▼                 ▼              │ │
│  │       run_command    TaskManager            │ │
│  │       (Fase 4)       (existente)            │ │
│  └─────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────┘
```

Regras:

- Um `WorkspaceWatcher` por workspace; cada um observa **múltiplos paths** dentro dele (`src/`, `tests/`, `lib/`, etc.).
- Um comando de validação por vez por workspace (sem fila de concorrência entre workspaces na v1). Entre workspaces, a validação roda em paralelo.
- Config carregada de `daemon.json` no app data dir (persistência entre sessões).
- `run_command` vem da Fase 4 (plano 012); o daemon é orquestração sobre ele, não código novo de execução.

## 4. Data flow

```
App abre → DaemonManager lê daemon.json → cria watchers por workspace
Arquivo muda (em qualquer watch_path) → debounce (tuning depois, não fixar agora)
  → ValidationExecutor roda estágios (§13.1), parando no primeiro erro:
      parse → diff checks → typecheck → lint → testes direcionados → build/suíte
  → Todos passam → nada (log silencioso)
  → Algum estágio falha →
      TaskFactory cria Task com contexto completo +
      classificação (trivial/normal/complexa) +
      contexto Explorer (arquivos relevantes, riscos) →
      IPC event TaskCreated → frontend auto-abre tarefa na sidebar
App fecha → DaemonManager para watchers, persiste estado
App reabre → DaemonManager carrega e retoma
```

Anti-duplicação: se o workspace já tem uma tarefa pendente criada pelo daemon para o mesmo arquivo (mesmo erro), não cria outra.

## 5. Componentes

### 5.1 DaemonManager

```rust
pub struct DaemonManager {
    watchers: HashMap<WorkspaceId, WorkspaceWatcher>,
    config: DaemonConfig,
    event_sender: Channel<DaemonEvent>,
}

impl DaemonManager {
    pub fn start(&mut self) -> Result<()>;
    pub fn stop(&mut self) -> Result<()>;
    pub fn add_workspace(&mut self, path: &Path) -> Result<WorkspaceId>;
    pub fn remove_workspace(&mut self, id: WorkspaceId) -> Result<()>;
    pub fn cancel_task(&mut self, task_id: TaskId) -> Result<()>;
}
```

### 5.2 WorkspaceWatcher

Observa múltiplos paths de um workspace (correção funcional: mudança em `tests/` passaria batido se observasse só a raiz).

```rust
pub struct WorkspaceWatcher {
    workspace_id: WorkspaceId,
    path: PathBuf,
    watch_paths: Vec<PathBuf>,
    watcher: RecommendedWatcher,        // crate notify
    validation_stages: Vec<ValidationStage>,
    pending_task: Option<TaskId>,       // anti-duplicação
}

impl WorkspaceWatcher {
    fn detect_watch_paths(root: &Path) -> Result<Vec<PathBuf>>;
    pub fn on_file_changed(&mut self, event: FileEvent) -> Result<()>;
}
```

Detecção automática de paths:
- se tem `src/` → observa `src/`
- se tem `tests/` → observa `tests/`
- se tem `lib/`, `test/`, `__tests__/` → idem
- se não tem nenhum → observa a raiz do workspace
- override manual via `watch_paths` no `daemon.json` (se definido, usa o manual)

### 5.3 ValidationExecutor

Executa os estágios do Verifier (§13.1), parando no primeiro erro.

```rust
pub enum ValidationStage {
    ParseSyntax,
    DiffChecks,
    Typecheck,
    Lint,
    TargetedTests,
    FullBuildSuite,
}

pub struct ValidationResult {
    pub stage_failed: ValidationStage,
    pub exit_code: i32,
    pub output: String,
    pub duration: Duration,
}
```

Detecção dos comandos (determinística, sem LLM):
- `package.json` → `npm run build` (yarn/pnpm/bun conforme lockfile)
- `Makefile` → `make` (ou `make build` se existir)
- `Cargo.toml` → `cargo check` (mais rápido que `cargo build` pra validar)
- Nenhum comando detectado → workspace é só observado, não validado

### 5.4 TaskFactory

Cria a tarefa com contexto completo + melhorias integradas.

```rust
impl TaskFactory {
    pub fn create_from_failure(
        workspace: &Workspace,
        validation: &ValidationResult,
        changed_files: &[PathBuf],
        diff: &str,
    ) -> Task;

    fn classify(validation: &ValidationResult) -> TaskClassification {
        match validation.stage_failed {
            ParseSyntax        => Trivial,
            DiffChecks         => Normal,
            Typecheck | Lint   => Normal,
            TargetedTests      => Complexa,
            FullBuildSuite     => Complexa,
        }
    }
}
```

Campos da Task criada: `workspace_id`, `title`, `prompt` ("Corrija o erro"), `classification` (§11.2), `explorer_context` (arquivos relevantes, comandos de validação detectados, riscos), `context` (comando, exit_code, output, changed_files, diff), `status: Pending`, `source: Daemon`.

### 5.5 IPC events

```rust
#[derive(Serialize)]
#[serde(tag = "event", content = "data")]
pub enum DaemonEvent {
    TaskCreated    { task: Task },
    TaskCancelled  { task_id: TaskId },
    StatusChanged  { workspace_id: WorkspaceId, status: DaemonStatus },
}
```

Frontend consome via `Channel<DaemonEvent>` (padrão do plano 005). `DaemonStatus`: `Watching | Validating | Idle | Error`.

## 6. Persistência

`daemon.json` no diretório de dados do app (nunca no repo — SPEC §30):

```json
{
  "workspaces": [
    {
      "path": "/home/user/project-a",
      "watch_paths": ["src", "tests", "config"],
      "enabled": true
    }
  ]
}
```

- `watch_paths`: opcional. Se ausente, usa detecção automática (5.2).
- Sem histórico de builds, sem diffs persistidos — só a lista de workspaces e preferências.
- Round-trip simples: salvar → carregar → mesmo estado.

## 7. Melhorias integradas

| # | Melhoria | Base no SPEC | Onde |
|---|----------|--------------|------|
| 1 | Classificação automática da tarefa (trivial/normal/complexa) | §11.2 | `TaskFactory::classify` |
| 2 | Contexto do Explorer na tarefa (arquivos, riscos, comandos) | §12.1 | `TaskFactory` |
| 3 | Cancelamento: descartar tarefa para o watcher | §11.1 (estado, cancelamento) | `DaemonManager::cancel_task` |
| 4 | Anti-duplicação: mesmo arquivo + mesmo erro = 1 tarefa | §11.1 (evitar trabalho redundante) | `WorkspaceWatcher::pending_task` |

## 8. Tratamento de erro

| Cenário | Comportamento |
|---------|---------------|
| Watch falha (notify) | Retry com backoff exponencial (3 tentativas), depois `StatusChanged { Error }` |
| Comando de validação não roda | `StatusChanged { Error }` + log; não cria tarefa |
| Processo do comando trava | Timeout (5 min padrão) + tree kill (padrão do plano 012) |
| App fecha no meio da validação | Persiste `daemon.json` no close (§11.1 estado persistente); retoma no start |
| Workspace removido durante execução | Cancela watcher, mata validação em andamento |
| `.env`/secret alterado | Nunca valida secret (§20.6) — filtra paths de secret do watch |

## 9. Testes

- Unit: `detect_watch_paths` (src+tests; só raiz quando vazio), `detect_validation_command` (package.json/Makefile/Cargo.toml), `TaskFactory::classify`, `TaskFactory::create_from_failure` (contexto completo), round-trip de `daemon.json`.
- Integration (com `tempdir` real, sem mock): arquivo em `tests/` gera evento (cobre a correção de múltiplos paths); debounce não dispara dentro da janela; anti-duplicação (2 mudanças, 1 tarefa); cancellation para watcher.

## 10. Limites e não-objetivos (v1)

- Sem serviço em background quando o app está fechado (tray/daemon SO fica fora).
- Sem fila de prioridades de comandos.
- Sem limite de recursos (é otimização — Fase 13, §35 "medir primeiro").
- Sem debounce fixado por número mágico (tuning depois).
- Sem config no repo (`.cd-ai.json`) — contar §30.
- Sem histórico/comparação de builds anteriores — validação sempre fresca (§13).
- O daemon **não executa correção sozinho**: cria a tarefa e deixa o fluxo normal do agente resolver.

## 11. Conexões com o SPEC

- §13.1 — Verifier: estágios de validação reutilizados pelo ValidationExecutor.
- §11.1 — Loop: estado persistente e cancelamento.
- §11.2 — Orchestrator: classificação da tarefa criada.
- §12.1 — Explorer: contexto incluído na tarefa.
- §20.6 — Secrets: filtra arquivos de secret do watch.
- §22 — Eventos: `DaemonEvent` segue o formato de eventos.
- §30 — Configuração: dados no app data dir, não no repo.
- Fase 6 (eval) e Fase 9 (checkpoints): pré-requisitos do agente completo; o daemon depende do TaskManager (Fase 5) e `run_command` (Fase 4).

## 12. Riscos

| Risco | Mitigação |
|-------|-----------|
| Falso-positivo (daemon cria tarefa pra algo que não é quebra) | Validação usa comandos reais do workspace (build/lint/test), não heurística de LLM |
| Estourar CPU/RAM com muitos watch_paths | Na v1 só observa paths detectados; medir antes de otimizar (Fase 13) |
| Criação de tarefa duplicada | Anti-duplicação por arquivo+estágio (5.2) |
| Data de secret vazando no contexto da tarefa | Filtro de secret no watch (§20.6) |