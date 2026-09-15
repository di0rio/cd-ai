# Plano 015: Fase 5 — o agente resolve uma tarefa de ponta a ponta (core, bridge, CLI e UI)

> **Instruções ao executor:** este plano é dividido em **partes com dono** (seção "Partes"). Execute **só a sua parte**, na ordem dos passos, e confirme cada verificação antes de avançar. Se alguma STOP condition acontecer, pare e reporte ao lead; não improvise. Uma parte só está pronta com `bun run verify` saindo com exit 0 (AGENTS.md). **Proibido para todos:** `git commit`, `git push`, force push, `git reset --hard`, `git checkout -- <arquivo>`, `git clean` e qualquer outra operação destrutiva, a menos que o usuário peça explicitamente.
>
> **Drift check (rode primeiro, em cada parte):** `git diff --stat 040369a -- <arquivos do escopo da sua parte>`. Se algum arquivo do seu escopo mudou desde `040369a` por algo que **não** seja uma parte anterior deste plano (ver "Ondas"), compare com os trechos de "Estado atual". Se não baterem, trate como STOP.

## Status

- **Prioridade:** P1
- **Esforço:** L (8 partes, de 0 a G, em 3 ondas)
- **Risco:** HIGH (concorrência, cancelamento e o primeiro código que executa ações do modelo sem um humano digitando cada chamada)
- **Depende de:** `plans/014-fase-4-tool-engine.md` e, por transitividade, 002, 005, 006, 009, 010 e 012
- **Categoria:** feature (SPEC §34, Fase 5)
- **Planejado em:** commit `040369a` (`main` local), 2026-09-11
- **Estado de partida:** `docs/handoff.md`
- **Implementação das Partes 0–G:** em `main` a partir de `193d198` (core), `7f4fe0d` (bridge), `fa50221` (CLI) e `57c522b` (UI G1+G2)
- **Fechamento:** 2026-09-15 — ver seção abaixo. Linha em `plans/README.md`: DONE (o aceite ao vivo com Ollama ficou registrado como não executável neste ambiente).

## Fechamento (2026-09-15)

A investigação sobre `main` em `024cc8e` mostrou que o plano foi executado: o `ToolEngine` tem um dono por tarefa, a UI chama `respondApproval` / `startTask` / `steerTask` / `cancelTask` / `resumeTask`, a CLI tem `cd-ai task`, e o loop cobre limites, timeout (sem contar a espera humana), cancelamento, detecção de loop e estado persistente.

O que **não** estava fechado:

1. **Documentação defasada.** `plans/README.md`, `docs/handoff.md` e `docs/fase-5-o-que-falta.md` ainda descreviam G2 como não começada e a árvore como não commitada — falso desde `57c522b` / `d14c5d4`.
2. **Aceite ao vivo reprovado (2026-09-12).** O agente corrigiu `evals/fixtures/soma/`, mas morreu em `taskTimeout` porque a espera de aprovação contava no deadline. O relógio já foi corrigido no runner (`blocked_ms`); faltava um teste de ponta a ponta **determinístico** no mesmo fixture (sem depender do Ollama) e registrar o que este ambiente consegue e o que não consegue verificar.
3. **`keep_alive` (decisão 0002, revisão).** O cliente Ollama mandava só `num_ctx`. Com o padrão de 5 minutos, uma tarefa parada na aprovação descarrega o CODER (~20 GB) e paga o reload. O fechamento manda `keep_alive: -1` em toda virada do `/api/chat` — um modelo grande residente, como a regra 1 da 0002.

Fora de escopo, como o pedido original: evals (Fase 6), sandbox (Fase 7), Verifier (Fase 8), shadow git, context manager, skills, model router, empacotamento.

## Drift check feito ao escrever este plano

- **HEAD** é `040369a` ("fix(core): normalize display_path to forward slashes on Windows").
- **`1d26ce2`**, citado no pedido como base, **não** é ancestral do HEAD e não está em nenhuma branch. Porém `git diff 1d26ce2 040369a` sai vazio: é a mesma árvore, num commit reescrito. A base deste plano é `040369a`.
- **`docs/handoff.md`** descreve o estado em `ce19df9`. De `ce19df9` até o HEAD mudaram só `README.md`, `docs/handoff.md` e `display_path` em `crates/agent-core/src/tools/mod.rs` (junta com `/` no Windows). Nada disso muda a Fase 5.
- **`origin/main`** está em `fa37c8c`. O `main` local está 1 commit à frente (`040369a` ainda não foi publicado).
- **O gate na base está vermelho no Windows**, ao contrário do que diz o handoff. O `bun run verify` rodado em `040369a` na máquina de desenvolvimento (Windows 11) deu:
  - Biome, typecheck e testes TS (3/3) passam;
  - o `cargo test -p agent-core` passa em 135 testes, **falha em 3** e ignora 2.

  Os que falham são `tools::command::tests::runs_echo_and_reports_exit`, `timeout_kills_and_marks_timed_out` e `running_command_runs_inside_workspace_cwd`, todos com `Io("não foi possível executar: program not found")`. Eles chamam `echo`, `sleep` e `ls` como executáveis, e nenhum deles está no PATH do Windows. O handoff ("141 passando") foi medido num ambiente Unix. **A Parte 0 corrige isso antes de qualquer outra parte.**

## Por que isso importa

Até a Fase 4, o cd-ai tem um `ToolEngine` seguro (path validado, permissões, redator, eventos), mas **nada o chama**. O `run_tool` do Tauri existe e nenhuma tela o usa. A Fase 5 fecha o circuito:

```text
tarefa → contexto básico → modelo → tools → resultado → modelo → … → relatório
```

O loop precisa ter limites, timeout, cancelamento, detecção de loop e estado persistente, na UI e na CLI.

É a fase que transforma o projeto numa ferramenta que resolve tarefas e libera o dogfooding no próprio repositório. **Saída (SPEC §34):** o agente resolve uma tarefa simples real num repositório de teste.

## Estado atual (fatos que o executor precisa)

### Core (`crates/agent-core`)

- `lib.rs` declara `events`, `ollama`, `permissions`, `redactor`, `tool_call`, `tools` e `workspace`. Ainda não existe módulo de agente.
- `tools/mod.rs` define o `ToolEngine`, que é **síncrono**:

  ```rust
  pub type EventSink<'e> = &'e mut dyn FnMut(ToolEventMessage);
  pub type Responder<'r> = &'r mut dyn FnMut(ApprovalRequest) -> ApprovalResponse;

  pub struct ToolEngine { pub workspace: Workspace, task_id: String, permissions: PermissionManager, sequence: u64, request_id: u64, command_next_id: u64 }

  impl ToolEngine {
      pub fn new(workspace: Workspace, task_id: impl Into<String>) -> Self;
      pub fn run_tool(&mut self, request: ToolRequest, events: EventSink, responder: Responder) -> ToolOutcome;
      pub fn permissions(&self) -> &PermissionManager;
      pub fn task_id(&self) -> &str;
  }
  ```

  `ToolRequest` tem as variantes `ReadFile`, `Search`, `EditFile`, `WriteFile`, `ListDirectory` e `RunCommand`, com os structs de args de `tools/mod.rs` (por exemplo `ReadFileArgs { path, start_line, end_line }` e `RunCommandArgs { argv, cwd, timeout_ms }`). Todo path passa por `Workspace::resolve` dentro das tools.
- `tools/command.rs`:
  - o `run_command` bloqueia no `wait_with_timeout`, que faz polling a cada 10 ms até o deadline e depois chama `kill_process_tree` (`kill -KILL -- -pgid` no Unix, `taskkill /T /F` no Windows);
  - **não existe cancelamento externo:** só o timeout interrompe;
  - os testes do módulo usam `echo`, `sleep` e `ls` como argv (linhas 254, 304, 348 e 369), o que quebra no Windows (ver drift check).
- `ollama.rs`:
  - `ChatMessage { role, content }` e `ChatRequest { model, messages, num_ctx }`.
  - `chat_stream(&self, &ChatRequest, on_event)` monta o corpo **sem `tools`**, e o `NdjsonChatParser` ignora `message.tool_calls`.
  - `ChatEvent` tem as variantes `Token`, `Thinking`, `Done` e `Error`.
  - O cancelamento é feito dropando a future.
- `tool_call.rs`: `parse_text_tool_calls(content, known_tools) -> Vec<ParsedToolCall { name, arguments: BTreeMap<String, String> }>` entende o formato `<function=…><parameter=…>` do qwen3-coder. É obrigatório como fallback (decisão 0002, regra 2).
- `permissions.rs`:
  - `PermissionManager::pending_for(task_id, action)` gera ids `aprv_NNNN` **por instância**, então dois engines podem gerar o mesmo id.
  - Existem `resolve(id)` e `clear()`.
- `Cargo.toml`: `tokio` existe só em `[dev-dependencies]`. Ele já está na árvore de dependências via `reqwest`, `src-tauri` e `apps/cli`.

### Bridge (`src-tauri/src/lib.rs`)

- `AppState { engine: Mutex<Option<ToolEngine>> }`, criado em `open_workspace` com `ToolEngine::new(workspace, "root")`. Esse `task_id` é o provisório citado no handoff.
- `run_tool`: tira o engine do estado, roda num `spawn_blocking` e devolve. O responder bloqueia num `mpsc` guardado em `PendingApprovals`, chaveado **só pelo id `aprv_*`**.
- `respond_approval(id, granted, reason)`.
- `chat` e `cancel_chat` (plano 005), com `JoinHandle` e `abort()`.
- A ACL em `build.rs` e em `capabilities/default.json` lista os 8 commands atuais.

### CLI (`apps/cli`)

- `main.rs` tem `--version`, `--help` e `chat --model <nome> [--ctx n] <prompt>`, que usa `tokio::select!` com `ctrl_c` (código de saída 130).
- `tests/cli.rs` trava o texto de uso e o código de saída 2 para argumento desconhecido.

### UI (`apps/desktop`)

- `lib/session.ts`:
  - `TaskStatus = running | waiting_approval | completed | completed_unvalidated | failed | cancelled`;
  - `ActivityEvent` (user, assistant, read, search, edit, command, report);
  - `groupActivity`.
- `components/app-shell.tsx`: as tarefas vêm **só** de `demoTasks` (`?demo` em desenvolvimento), e o botão "Nova tarefa" está desabilitado.
- `components/composer.tsx`: o botão de enviar está `disabled`, com `title="O agente ainda não está conectado"`. Existe o seletor de modo de permissão, com "Acesso total" desabilitado.
- `lib/ipc.ts`: os wrappers `runTool` e `respondApproval(id, granted, reason)` existem e **ninguém os chama**. Os tipos vêm de `lib/bindings/` (ts-rs).
- Design obrigatório: `apps/desktop/DESIGN.md` ("A Sala de Controle") e `apps/desktop/PRODUCT.md`.

### API de tool calling do Ollama (documentação oficial)

- **Request:** `"tools": [{ "type": "function", "function": { "name", "description", "parameters": <JSON Schema> } }]`.
- **Resposta:** `message.tool_calls: [{ "function": { "name", "arguments": { … } } }]`. Os `arguments` vêm como objeto JSON, e com streaming as chamadas chegam num dos chunks.
- **Resultado de volta ao modelo:** uma mensagem `{ "role": "tool", "tool_name": "<nome>", "content": "<texto>" }`.

## Decisões deste plano (não reabrir sem registrar em `docs/decisions/`)

| # | Decisão | Motivo |
|---|---|---|
| D1 | **O loop é síncrono e roda numa thread dedicada.** O modelo entra pelo trait síncrono `ChatModel`, e o adaptador do Ollama usa `tokio::runtime::Handle::block_on`. `tokio` vira dependência direta do `agent-core`, com as features `rt`, `sync`, `time` e `macros`. | O `ToolEngine` e o responder de aprovação são síncronos e bloqueiam. Reescrever o engine como async não traz ganho na v1. O `tokio` já está na árvore. |
| D2 | **Tool calling nativo primeiro.** Se `tool_calls` vier vazio, o loop aplica `parse_text_tool_calls` sobre o texto. Só nomes oferecidos na requisição são aceitos. | Decisão 0002, regra 2 (formato do qwen3-coder). |
| D3 | **As tools são expostas ao modelo com nomes snake_case e o schema da tabela "Tools para o modelo".** O mapeamento para `ToolRequest` é código nosso, nunca desserialização direta do texto do modelo. | Mesmos nomes do benchmark e do parser. Os args são validados um a um. |
| D4 | **Um `ToolEngine` por tarefa**, com `ToolEngine::new(workspace.clone(), task_id)`. O command `run_tool` e o wrapper `runTool` são **removidos** (deletion first; ninguém os usa). As aprovações passam a ser chaveadas por `(task_id, approval_id)`. | Handoff. Remove o `"root"` provisório e a colisão de ids `aprv_*` entre engines. |
| D5 | **Uma tarefa rodando por workspace.** Um segundo `start` recebe o erro `"já existe uma tarefa em andamento"`. | SPEC §8.2: concorrência fica para depois da v1. |
| D6 | **Um `CancelToken` compartilhado** (`Arc<AtomicBool>` + `tokio::sync::Notify`) cancela tudo: o loop checa entre passos, o stream do modelo aborta via `select!`, o `run_command` mata a árvore de processos, e a aprovação pendente da tarefa é respondida com `Denied { reason: "tarefa cancelada" }`. | SPEC §11.1: o cancelamento propaga para o modelo e para os processos filhos. |
| D7 | **Limites padrão** (`AgentLimits`): `max_iterations = 30`; `max_model_retries = 2`; `max_invalid_tool_calls = 3` consecutivas; `model_turn_timeout = 300 s`; `task_timeout = 45 min`. O timeout de comando continua sendo o da Fase 4 (30 s). **Detecção de loop:** a mesma assinatura (tool + args normalizados) 3 vezes na tarefa, **ou** a mesma mensagem de erro 3 vezes seguidas, para a tarefa com `LoopDetected`. | SPEC §11.1. O timeout de 300 s cobre prefill de 16k tokens a cerca de 100 tok/s mais geração a cerca de 20 tok/s (benchmark de 2026-09-11). |
| D8 | **O estado persistente fica no diretório de dados do app, nunca no workspace.** A função `data_dir()` usa só std: `%APPDATA%\cd-ai` no Windows, `~/Library/Application Support/cd-ai` no macOS, e `$XDG_DATA_HOME/cd-ai` ou `~/.local/share/cd-ai` no Linux. O override é `CD_AI_DATA_DIR`, usado nos testes. Layout: `tasks/<id>/state.json` (escrita atômica, temp + rename), `tasks/<id>/transcript.jsonl` e `tasks/<id>/events.jsonl`. **Tudo passa pelo redator antes de ir para o disco.** | SPEC §23, §24, §30 e §20.6. A CLI e a UI veem as mesmas tarefas. |
| D9 | **Retomada:** ao abrir o store, uma tarefa em `running` ou `waiting_approval` vira `cancelled` com `stop_reason = Interrupted`. O `resume` reconstrói as mensagens a partir do `transcript.jsonl`, descarta a última mensagem do assistente que tenha tool calls sem resultado e acrescenta a mensagem de usuário "A tarefa foi interrompida. Continue de onde parou." | SPEC §11.1 ("retomável após crash") sem inventar status fora do §23. |
| D10 | **Contexto básico** = system prompt curto (seção "System prompt") + perfil determinístico do workspace (Parte C) + listagem da raiz. A busca fica com a tool `search`. **Orçamento:** estimativa de `chars / 4`. Acima de 75% do `num_ctx`, o conteúdo dos resultados de tool mais antigos é substituído por `[resultado antigo omitido; chame a tool de novo se precisar]` (os 2 últimos ficam) e o loop emite `ContextTrimmed`. Se ainda passar de 90%, a tarefa para com `ContextExhausted`. | SPEC §16.2: cortar de forma explícita e registrar. A compactação automática é da Fase 10 (decisão 0008). |
| D11 | **Sem Verifier nesta fase:** toda tarefa que termina vira `completed_unvalidated`, com evidências (comandos com exit code e arquivos alterados). **O loop nunca produz `completed`.** | SPEC §2 e §23. O Verifier é a Fase 8. |
| D12 | **Eventos:** `AgentEvent` (Parte C) dentro de `AgentEventMessage { taskId, sequence, at, …flatten }`, no mesmo formato `{ event, data }` do plano 005. Os eventos do `ToolEngine` viajam como `AgentEvent::Tool(ToolEvent)`. | SPEC §22. Um único canal por tarefa. |
| D13 | **Na CLI, a aprovação é pedida no stdin** (terminal interativo), mostrando o argv ou o diff exato. Sem terminal interativo, o pedido é **negado**. Não existe flag de auto-aprovação. | Decisão 0001: sem sandbox, toda escrita e todo comando não-read pedem aprovação. |
| D14 | **`num_ctx` padrão de 16384** (decisão 0002, regra 3). O modelo é sempre escolhido por quem chama: a UI escolhe a partir da lista do Ollama, e a CLI exige `--model`. Nenhum nome de modelo fica no código. | Decisão 0002 e seu esclarecimento. |
| D15 | **Testes que executam processos usam argv por plataforma**, com helpers de teste: `cmd /C echo` e `cmd /C dir /B` no Windows, `echo` e `ls` no Unix; e `powershell -NoProfile -Command "Start-Sleep -Seconds N"` no Windows, `sleep N` no Unix. Nenhum teste chama um binário Unix sem `cfg`. | Decisão 0001: as Fases 1–6 rodam no Windows 11. O gate precisa ficar verde nas duas plataformas. |

## Contratos (tipos que as partes criam)

Todo tipo que cruza o IPC leva `#[derive(TS)] #[ts(export)]`, e todo campo `u64` leva `#[ts(type = "number")]` (regra do plano 008). Para `serde_json::Value`, use `#[ts(type = "Record<string, unknown>")]`.

```rust
// agent/state.rs (Parte C)
#[serde(rename_all = "snake_case")]
pub enum TaskStatus { Running, WaitingApproval, CompletedUnvalidated, Failed, Cancelled }

#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum StopReason {
    Finished,                 // o modelo respondeu sem tool calls
    MaxIterations,
    TaskTimeout,
    LoopDetected { detail: String },
    InvalidToolCalls,
    ModelError { message: String },
    ContextExhausted,
    Cancelled,
    Interrupted,
}

pub struct AgentLimits { max_iterations: u32, max_model_retries: u32, max_invalid_tool_calls: u32, model_turn_timeout_ms: u64, task_timeout_ms: u64 }
// impl Default com os valores de D7

pub struct TaskState {
    id: String, workspace: String /* raiz para exibição */, request: String, model: String, num_ctx: u32,
    status: TaskStatus, stop_reason: Option<StopReason>, created_at: String, updated_at: String,
    iterations: u32, files_read: Vec<String>, files_changed: Vec<FileChange /* { path, hash_after } */>,
    commands: Vec<CommandRecord /* { argv, exit_code: Option<i32>, duration_ms } */>,
    errors: Vec<String> /* redigidos */, retries: u32, metrics: TaskMetrics /* { prompt_tokens, gen_tokens, model_ms, tool_ms } */,
}

pub struct TaskSummary { id: String, title: String /* primeiros 60 chars do pedido */, status: TaskStatus, updated_at: String, model: String }

pub struct TaskReport { summary: String /* texto final do modelo, redigido */, validated: bool /* sempre false na F5 */, evidence: Vec<CommandRecord>, files_changed: Vec<String> }
```

```rust
// agent/events.rs (Parte C)
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "event", content = "data")]
pub enum AgentEvent {
    TaskStarted { summary: TaskSummary },
    StatusChanged { status: TaskStatus, reason: Option<String> },
    UserMessage { text: String },                 // pedido, correção de rumo, nota de retomada
    ModelTurnStarted { iteration: u32, model: String },
    Token { content: String },
    Thinking { content: String },
    ModelTurnCompleted { prompt_tokens: u64, gen_tokens: u64, prompt_ms: u64, gen_ms: u64 },
    AssistantMessage { content: String },         // texto completo do turno, redigido
    ToolCallRequested { tool: String, input: serde_json::Value },   // strings redigidas
    ToolCallFinished { tool: String, ok: bool, detail: String, output: Option<String> },
    Tool(ToolEvent),                              // tudo que o ToolEngine emite
    ContextTrimmed { removed_messages: u32, estimated_tokens: u64 },
    Retrying { attempt: u32, reason: String },
    TaskFinished { status: TaskStatus, stop_reason: StopReason, report: TaskReport },
}

pub struct AgentEventMessage { task_id: String, sequence: u64, at: String, #[serde(flatten)] #[ts(flatten)] event: AgentEvent }
```

```rust
// tools/cancel.rs (Parte B)
#[derive(Clone, Default)]
pub struct CancelToken(/* Arc<{ flag: AtomicBool, notify: tokio::sync::Notify }> */);
impl CancelToken {
    pub fn cancel(&self);
    pub fn is_cancelled(&self) -> bool;
    pub async fn cancelled(&self);   // resolve imediatamente se já cancelado
}
```

```rust
// agent/model.rs (Parte D)
pub struct ModelReply { content: String, thinking: String, tool_calls: Vec<ModelToolCall>, prompt_tokens: u64, gen_tokens: u64, prompt_ms: u64, gen_ms: u64 }
pub enum ModelError { Timeout, Cancelled, Failed(String) }
pub trait ChatModel {
    fn turn(&mut self, messages: &[ChatMessage], tools: &[ToolSpec], on_event: &mut dyn FnMut(ChatEvent), cancel: &CancelToken) -> Result<ModelReply, ModelError>;
}
pub struct OllamaModel { client: OllamaClient, runtime: tokio::runtime::Handle, model: String, num_ctx: u32, turn_timeout: Duration }
```

```rust
// tools/command.rs, dentro de #[cfg(test)] (Parte 0) — usado também nos testes das Partes B e D
#[cfg(test)]
pub(crate) mod test_argv {
    pub fn echo(text: &str) -> Vec<String>;   // Windows: ["cmd","/C","echo",text]      Unix: ["echo",text]
    pub fn list_cwd() -> Vec<String>;         // Windows: ["cmd","/C","dir","/B"]       Unix: ["ls"]
    pub fn sleep_secs(n: u64) -> Vec<String>; // Windows: ["powershell","-NoProfile","-Command","Start-Sleep -Seconds N"]  Unix: ["sleep","N"]
}
```

### Tools para o modelo (D3)

| Nome | Parâmetros (JSON Schema) | `ToolRequest` |
|---|---|---|
| `read_file` | `path` (string, obrigatório), `start_line` (integer), `end_line` (integer) | `ReadFile` |
| `list_directory` | `path` (string, obrigatório; `"."` = raiz) | `ListDirectory` |
| `search` | `query` (string, obrigatório), `regex` (boolean), `path` (string), `max_results` (integer) | `Search` |
| `edit_file` | `path`, `old_text`, `new_text` (strings, obrigatórios) | `EditFile` |
| `write_file` | `path`, `content` (obrigatórios), `if_exists` (`"error"` \| `"overwrite"`) | `WriteFile` |
| `run_command` | `argv` (array de strings, obrigatório), `cwd` (string), `timeout_ms` (integer) | `RunCommand` |

**Args vindos do fallback de texto** chegam todos como string:

- inteiros e booleanos são convertidos com `parse`;
- `argv` só é aceito como **array JSON** (`["cargo","test"]`), nunca como string separada por espaço.

Um arg ausente, com tipo errado ou uma tool desconhecida vira uma mensagem de erro para o modelo (loop de reparo, SPEC §4.3), e isso conta para `max_invalid_tool_calls`.

### System prompt (inglês, curto; `agent/prompt.rs`)

```text
You are cd-ai, a coding agent working inside one local project folder (the workspace).
Work in small steps and look before you change anything.
Rules:
- Paths are relative to the workspace root. Never try to leave it.
- run_command takes argv as an array of strings and runs without a shell: no pipes, &&, redirects or globs. One command per call.
- For existing files prefer edit_file (exact old_text -> new_text) over write_file.
- File changes and most commands need the user's approval. If something is denied, adapt; do not repeat the same call.
- When the task is done, or you cannot continue, answer WITHOUT tool calls: a short summary in Brazilian Portuguese of what changed and how it was checked.

Workspace profile:
{profile}
```

### Resultado de tool para o modelo (`agent/tool_calls.rs`, `render_outcome`)

Texto simples, nunca o JSON do `ToolOutcome`, com no máximo `MAX_TOOL_RESULT_CHARS = 12_000` e uma nota de corte:

| Tool | Formato |
|---|---|
| `read_file` | `"{path} (linhas {a}-{b} de {n})\n{text}"` |
| `list_directory` | uma linha por entrada: `dir/` ou `arquivo (N bytes)` |
| `search` | `"{total} resultados"` e depois uma linha `path:linha: texto` por match |
| `edit_file` | `"ok: {path} (+{added} −{removed})"` |
| `write_file` | `"ok: {path} ({size} bytes)"` |
| `run_command` | `"exit {code}"` (ou `"timeout"`, ou `"cancelado"`), seguido do output |
| qualquer erro | `"erro: {ToolError}"`, com a mensagem pt-BR que já existe no `Display` |

## Partes

Cada parte lista escopo, passos, testes e verificação. **Arquivos fora do escopo da sua parte não são tocados.** Se precisar tocar, é STOP.

### Parte 0 — Gate verde no Windows (dono: `core`; primeira de todas)

**Escopo:** `crates/agent-core/src/tools/command.rs`, **só** o `mod tests`.

**Passos:**

1. Crie o módulo `test_argv` do contrato (D15) dentro do arquivo, com `#[cfg(test)]` e `pub(crate)`, para as Partes B e D o usarem.
2. Troque os argv literais dos testes:
   - `echo` → `test_argv::echo`;
   - `sleep` → `test_argv::sleep_secs`;
   - `ls` → `test_argv::list_cwd`.
3. Mantenha as asserções. No teste de timeout, o `timeout_ms: Some(200)` continua valendo: se o `powershell` ainda não tiver iniciado quando o timeout vencer, o resultado continua `timed_out = true`.

**Verificar:** `cargo test -p agent-core tools::command` sai com `0 failed`, e `bun run verify` sai com exit 0 no Windows (pela primeira vez desde a Fase 4).

**STOP:** a correção exigiria mudar o código de produção do `run_command` (não só os testes).

### Parte A — Tool calling no cliente Ollama (dono: `core`; depois da Parte 0)

**Escopo:**

- `crates/agent-core/src/ollama.rs`
- `apps/cli/src/main.rs` (**só** os literais de `ChatMessage` e `ChatRequest` do `chat`)
- `apps/desktop/src/lib/bindings/` (regenerado)

**Passos:**

1. Crie `pub struct ModelToolCall { pub function: ModelFunctionCall }` e `pub struct ModelFunctionCall { pub name: String, pub arguments: serde_json::Value }`, que espelham o JSON do Ollama. Também `ToolSpec { r#type: "function", function: ToolFunctionSpec { name, description, parameters: Value } }`, com o construtor `ToolSpec::function(name, description, parameters)`.
2. Em `ChatMessage`, adicione `Default` e os campos:
   - `#[serde(default, skip_serializing_if = "Vec::is_empty")] tool_calls: Vec<ModelToolCall>`;
   - `#[serde(default, skip_serializing_if = "Option::is_none")] tool_name: Option<String>`.

   Em `ChatRequest`, adicione `#[serde(default)] #[ts(optional)] tools: Option<Vec<ToolSpec>>`.
3. Extraia a montagem do corpo para `fn request_body(request: &ChatRequest) -> serde_json::Value`, que inclui `"tools"` só quando houver tools, e use-a no `chat_stream`.
4. No parser NDJSON, leia `message.tool_calls` e emita a nova variante `ChatEvent::ToolCalls { calls: Vec<ModelToolCall> }` (antes do `Done` da mesma linha).
5. Na CLI, troque os literais por `ChatMessage { role, content, ..Default::default() }` e `ChatRequest { …, tools: None }`.
6. Regenere os bindings com `cargo test -p agent-core export_bindings`.

**Testes (em `ollama.rs`):**

- `request_body_includes_tools_only_when_present`;
- `parser_emits_tool_calls` (linha com `"tool_calls":[{"function":{"name":"read_file","arguments":{"path":"a.rs"}}}]`);
- `tool_message_serializes_role_and_name`;
- os testes existentes continuam verdes.

**Verificar:** `cargo test -p agent-core ollama` sai com `0 failed`, e `bun run verify` sai com exit 0.

### Parte B — Cancelamento no ToolEngine (dono: `core`; depois da Parte A)

**Escopo:**

- `crates/agent-core/src/tools/cancel.rs` (novo)
- `crates/agent-core/src/tools/mod.rs`
- `crates/agent-core/src/tools/command.rs`
- `crates/agent-core/Cargo.toml` (só o `tokio`)
- bindings (regenerados)

**Passos:**

1. Em `Cargo.toml`, em `[dependencies]`: `tokio = { version = "1.53.1", features = ["rt", "sync", "time", "macros"] }`. Mantenha a entrada de dev.
2. Crie `tools/cancel.rs` com o `CancelToken` do contrato e declare `pub mod cancel;` em `tools/mod.rs`.
3. No `ToolEngine`:
   - adicione o campo `cancel: CancelToken`, com `pub fn set_cancel(&mut self, token: CancelToken)` e `pub fn cancel_token(&self) -> &CancelToken`;
   - no início de `run_tool`, se já estiver cancelado, devolva `ToolOutcome::err(ToolError::Cancelled)` sem executar nada;
   - adicione `ToolError::Cancelled`, com o `Display` "tarefa cancelada".
4. Em `command.rs`, o `wait_with_timeout` passa a receber `&CancelToken`. O loop de polling checa `is_cancelled()`: se cancelado, chama `kill_process_tree`, marca `cancelled: true` no `RunStatus`, e o `run_command` emite `CommandCompleted` (com `exit_code: None`) e devolve `Err(ToolError::Cancelled)`.
5. **Silencie o `kill_process_tree`** (decisão do lead na revisão da Parte 0): hoje o `taskkill` do Windows escreve `ERROR: The process "NNN" not found.` no stderr do processo de teste quando o filho já saiu sozinho. Passe `Stdio::null()` em stdout e stderr do `kill`/`taskkill` e ignore o status: um processo que já morreu não é erro. O cancelamento vai usar esse mesmo caminho com frequência.

**Testes:**

- `cancelled_engine_runs_nothing` (em `mod.rs`);
- `cancel_kills_running_command`: um `test_argv::sleep_secs(5)` com cancelamento disparado depois de 200 ms por outra thread volta em menos de 3 s com `Cancelled`;
- `#[tokio::test] cancelled_future_resolves` (em `cancel.rs`).

**Verificar:** `cargo test -p agent-core tools` sai com `0 failed`, e `bun run verify` sai com exit 0.

### Parte C — Tipos, persistência e perfil do workspace (dono: `core`; depois da Parte B)

**Escopo:**

- `crates/agent-core/src/lib.rs` (só `pub mod agent;`)
- `crates/agent-core/src/agent/mod.rs`, `state.rs`, `events.rs`, `storage.rs` e `profile.rs` (novos)
- bindings (regenerados)

**Passos:**

1. **`state.rs` e `events.rs`:** exatamente os contratos acima, com `impl Default for AgentLimits` usando os valores de D7.
2. **`storage.rs`:**
   - `pub fn data_dir() -> Result<PathBuf, StorageError>`, conforme D8, só com std e o override `CD_AI_DATA_DIR`;
   - `pub struct TaskStore { root: PathBuf }`, com:

     ```rust
     open(data_dir) -> Result<Self>
     new_task_id() -> String            // task_<unix_ms>_<pid hex><contador>
     save_state(&TaskState)             // temp no mesmo diretório + rename
     load_state(id)
     append_transcript(id, &ChatMessage)
     load_transcript(id) -> Vec<ChatMessage>
     append_event(&AgentEventMessage)
     load_events(id) -> Vec<AgentEventMessage>
     list(workspace: &str) -> Vec<TaskSummary>   // mais recentes primeiro
     recover_interrupted() -> Vec<String>        // D9
     ```

   - O `id` é validado (só `[a-z0-9_]`) antes de montar qualquer path. **Não existe path cru vindo de fora:** o store monta seus próprios paths.
   - Antes de gravar, todo texto livre (conteúdo de `ChatMessage`, `errors`, `summary`) passa por `redactor::redact`.
3. **`profile.rs`:** `pub fn workspace_profile(workspace: &Workspace) -> WorkspaceProfile { languages, package_manager, validation_commands, rules_excerpt, root_entries }` e `render(&self) -> String` (no máximo 3 KiB).
   - **Detecção por arquivo na raiz:**

     | Arquivo | Detecta |
     |---|---|
     | `Cargo.toml` | Rust; `cargo test` e `cargo clippy` |
     | `package.json` | JS/TS; os scripts `test`, `typecheck`, `lint`, `check`, `build` e `verify` que existirem, como `<pm> run <script>` |
     | `tsconfig.json` | TS |
     | `pyproject.toml` ou `requirements.txt` | Python |
     | `go.mod` | Go |

   - **Package manager pelo lockfile:** `bun.lock`/`bun.lockb` → bun; `pnpm-lock.yaml` → pnpm; `yarn.lock` → yarn; `package-lock.json` → npm.
   - **Regras do projeto:** os primeiros 4 KiB de `AGENTS.md` ou, na falta dele, de `CLAUDE.md`.
   - **Listagem da raiz:** até 60 nomes, ordenados.
   - **Toda leitura** usa `workspace.resolve(...)` e é pulada se `redactor::detect_path_secret` acusar secret. O conteúdo passa por `redact`.

**Testes:**

- **storage:** round-trip de state, transcript e events; escrita atômica (nenhum `.tmp` sobra); `recover_interrupted` muda `running` para `cancelled`/`Interrupted`; `list` filtra por workspace; id inválido (`../x`) é recusado; um secret no conteúdo (token `ghp_…` do fixture do redator) sai `[REDIGIDO…]` no arquivo.
- **profile:** `tempdir` com `Cargo.toml` e `package.json` + `bun.lock` gera os comandos certos; `.env` na raiz nunca é lido; `AGENTS.md` é cortado em 4 KiB.
- Todos os testes usam `CD_AI_DATA_DIR` num `tempdir`, **nunca** o diretório real do usuário.

**Verificar:** `cargo test -p agent-core agent` sai com `0 failed`, e `bun run verify` sai com exit 0.

### Parte D — O loop (dono: `core`; depois da Parte C)

**Escopo:** `crates/agent-core/src/agent/mod.rs` (declarar os módulos novos), `model.rs`, `tool_calls.rs`, `prompt.rs` e `runner.rs` (novos). Mais o passo 0 abaixo, que toca `events.rs` (dos dois módulos), `permissions.rs` e `agent/storage.rs`.

**Passo 0 — replay tipado** (decisão do lead na revisão da Parte C): hoje `TaskStore::load_events` devolve `Vec<serde_json::Value>`, porque `AgentEvent` não consegue derivar `Deserialize` enquanto `ToolEvent` e `PermissionDecision` forem serialize-only. Acrescente `Deserialize` a `ToolEvent` (`crates/agent-core/src/events.rs`), a `PermissionDecision` (`permissions.rs`) e ao que mais faltar em `agent/state.rs` e `agent/events.rs`, e troque a assinatura para `load_events(id) -> Result<Vec<AgentEventMessage>, StorageError>`, pulando linha malformada como o `load_transcript` já faz. O contrato da Parte E promete o tipo, não `Value`.

**Passos:**

1. **`model.rs`:** o trait `ChatModel` e o `OllamaModel`.
   - O `turn` faz `Handle::block_on` de `tokio::select!` entre três ramos: `tokio::time::timeout(turn_timeout, client.chat_stream(...))`, `cancel.cancelled()` → `ModelError::Cancelled`, e o fim do stream.
   - Ele acumula `Token` em `content`, `Thinking` em `thinking`, `ToolCalls` em `tool_calls` e `Done` nas métricas, e repassa cada `ChatEvent` ao `on_event`.
   - Crie `#[cfg(test)] pub(crate) struct ScriptedModel`, que devolve respostas pré-programadas em ordem e registra as mensagens que recebeu.
2. **`tool_calls.rs`:**
   - `tool_specs() -> Vec<ToolSpec>` (a tabela D3);
   - `to_request(name, args: &serde_json::Value) -> Result<ToolRequest, String>`;
   - `to_request_from_text(ParsedToolCall) -> Result<ToolRequest, String>`;
   - `signature(&ToolRequest) -> String` (JSON canônico, para a detecção de loop);
   - `render_outcome(&ToolOutcome) -> String` (tabela acima).
3. **`prompt.rs`:**
   - `system_prompt(profile: &str) -> String`;
   - `estimate_tokens(&[ChatMessage]) -> u64` (`chars / 4`);
   - `trim_for_budget(&mut Vec<ChatMessage>, num_ctx) -> Option<(u32, u64)>` (D10: omite os resultados de tool mais antigos e preserva o system, o primeiro user e os 2 últimos resultados).
4. **`runner.rs`:**

   ```rust
   pub struct TaskContext<'a> { store: &'a TaskStore, workspace: Workspace, limits: AgentLimits, cancel: CancelToken, steer: Arc<Mutex<VecDeque<String>>> }
   pub enum TaskStart { New { request: String, model: String, num_ctx: u32 }, Resume { task_id: String } }
   pub fn run_task(ctx: TaskContext, model: &mut dyn ChatModel, start: TaskStart, on_event: &mut dyn FnMut(AgentEventMessage), responder: Responder) -> TaskState
   ```

   O algoritmo, em cada iteração:
   1. Cheque o cancelamento e o `task_timeout`.
   2. Esvazie o `steer` em mensagens `user` e emita `UserMessage`.
   3. Rode o `trim_for_budget`.
   4. Emita `ModelTurnStarted`.
   5. Chame `model.turn`. Em erro, faça retry até `max_model_retries`, emitindo `Retrying`; depois `ModelError`.
   6. Colete as tool calls: primeiro as nativas, depois o fallback de texto (D2).
   7. **Sem tool calls:** `completed_unvalidated` com `StopReason::Finished` e `TaskReport`, e fim.
   8. **Para cada chamada:**
      - mapeie; se o mapeamento falhar, a mensagem de erro vai como resultado da tool (loop de reparo);
      - emita `ToolCallRequested`;
      - rode a detecção de loop (D7);
      - rode `engine.run_tool`, com os eventos repassados como `AgentEvent::Tool`, `StatusChanged(WaitingApproval)` antes de pedir aprovação e `Running` depois;
      - emita `ToolCallFinished`;
      - acrescente a mensagem `{ role: "tool", tool_name, content: render_outcome }`;
      - atualize `files_read`, `files_changed` e `commands` do `TaskState`.
   9. Persista o `state`, o transcript e os eventos.

   Ao passar de `max_iterations`, a tarefa para com `MaxIterations`. O engine é `ToolEngine::new(workspace.clone(), task_id)` com `set_cancel(ctx.cancel.clone())` (D4 e D6). O `Resume` segue D9.

**Testes (com `ScriptedModel` e `tempdir`, sem Ollama):**

1. Uma resposta sem tools termina `completed_unvalidated`/`Finished`, e o `TaskReport.validated == false`.
2. Um `read_file` nativo roda, e o resultado volta ao modelo como mensagem `tool` com o texto do arquivo.
3. O fallback de texto `<function=read_file>…` roda.
4. Uma tool desconhecida vira mensagem de erro; 3 inválidas seguidas terminam `failed`/`InvalidToolCalls`.
5. A mesma chamada 3 vezes termina `LoopDetected`.
6. `max_iterations = 2` com um modelo que sempre chama tool termina `MaxIterations`.
7. Uma aprovação negada volta ao modelo como `"erro: permissão negada…"` e o loop continua.
8. Um cancelamento disparado durante um `run_command` com `test_argv::sleep_secs(5)` termina `cancelled`/`Cancelled` em menos de 3 s.
9. A persistência grava `state.json` a cada iteração, e o `Resume` de uma tarefa interrompida reconstrói as mensagens e acrescenta a nota de D9.
10. Com `num_ctx` pequeno, `trim_for_budget` emite `ContextTrimmed`.
11. Um erro de modelo com retry termina `ModelError` depois de `1 + max_model_retries` tentativas.
12. Um secret no pedido do usuário aparece redigido em `transcript.jsonl`.

**Verificar:** `cargo test -p agent-core agent` sai com `0 failed`, e `bun run verify` sai com exit 0.

### Parte E — Bridge do Tauri (dono: `bridge`; onda 2)

**Escopo:**

- `src-tauri/src/lib.rs`
- `src-tauri/build.rs`
- `src-tauri/capabilities/default.json`
- `apps/desktop/src/lib/ipc.ts`

**Passos:**

1. **Estado:** `AppState { workspace: Mutex<Option<Workspace>> }` e `RunningTasks(Arc<Mutex<HashMap<String, RunningTask { cancel, steer, pending: HashMap<String, mpsc::Sender<ApprovalResponse>> }>>>)`. O `open_workspace` e o `current_workspace` passam a usar `workspace`. **Remova o engine `"root"` e o command `run_tool`** (D4).
2. **Commands:**

   | Command | O que faz |
   |---|---|
   | `start_task(request, model, num_ctx, on_event: Channel<AgentEventMessage>) -> Result<String, String>` | Recusa sem workspace ou com uma tarefa rodando (D5). Cria o id, faz `std::thread::spawn` do `run_task` com um `OllamaModel` montado com `tauri::async_runtime::handle()` e o `OLLAMA_HOST` validado como no `chat`. O responder registra o `Sender` em `pending` e bloqueia no `Receiver`. Remove a entrada ao terminar. |
   | `resume_task(task_id, model, num_ctx, on_event)` | Mesmo fluxo, com `TaskStart::Resume`. |
   | `cancel_task(task_id) -> bool` | Chama `cancel()` e responde `Denied { reason: "tarefa cancelada" }` a todo `pending` da tarefa. |
   | `steer_task(task_id, text) -> bool` | Acrescenta o texto ao `steer`. |
   | `respond_approval(task_id, id, granted, reason) -> bool` | Nova assinatura, chaveada por tarefa. |
   | `list_tasks() -> Vec<TaskSummary>` | Tarefas do workspace atual. |
   | `task_events(task_id) -> Vec<AgentEventMessage>` | Replay de uma tarefa já existente. |

   Na inicialização do app, chame `TaskStore::recover_interrupted()`.

   **Antes de um `resume`, valide com `store.load_state(id)`** (decisão do lead na revisão da Parte D): o `run_task` aceita um id inexistente ou de outro workspace, mas devolve `failed` com `StopReason::ModelError`, que é impreciso. Valide antes e devolva `Err(String)` com a mensagem certa; o caminho do `run_task` fica só como rede de segurança.
3. **ACL:** atualize a lista do `build.rs` e as permissões `allow-*` da capability. Tire `run_tool` e ponha os 7 commands novos ou alterados. As três listas precisam bater (plano 006).
4. **`ipc.ts`:**
   - remova `runTool`;
   - adicione `startTask`, `resumeTask`, `cancelTask`, `steerTask`, `listTasks` e `taskEvents`;
   - `respondApproval(taskId, id, granted, reason?)`;
   - reexporte os tipos novos dos bindings (`AgentEvent`, `AgentEventMessage`, `TaskSummary`, `TaskStatus`, `StopReason`, `TaskReport`).

   O `startTask` recebe `(request, model, numCtx, onEvent)` e cria o `Channel<AgentEventMessage>` como o `startChat`.
5. **Limpe os warnings do próprio arquivo** (decisão do lead na revisão da G1): o `bun run check` aponta 26 `noUnusedImports` em `ipc.ts`, por causa dos `import type` seguidos de `export type` para o mesmo tipo. Como a Parte E é dona do arquivo, resolva de vez (reexporte sem importar duas vezes). Não são erros, mas poluem a saída do gate de todo mundo.

**Verificar:** `bun run verify` sai com exit 0, e `bun tauri build --no-bundle` sai com exit 0.

**Verificação manual (lead):** com o app aberto, iniciar uma tarefa de leitura ("liste os arquivos da raiz") produz eventos na conversa depois da Parte G. Antes disso, dá para conferir pelo DevTools.

### Parte F — CLI `cd-ai task` (dono: `cli`; onda 2, em paralelo com E)

**Escopo:**

- `apps/cli/src/main.rs`
- `apps/cli/tests/cli.rs`
- `apps/cli/Cargo.toml` (se precisar da feature `io-std` do tokio)
- `evals/fixtures/soma/` (novo)
- `AGENTS.md` e `README.md` (só a linha do comando nas listas de comandos)

**Passos:**

1. **Subcomando:** `cd-ai task --model <nome> [--ctx <n>] [--workspace <pasta>] [--max-iterations <n>] <pedido…>` e `cd-ai task --resume <id> --model <nome> [--ctx <n>] [--workspace <pasta>]`.
   - O `--ctx` tem padrão 16384 (D14).
   - O `--workspace` tem padrão na pasta atual, sempre via `Workspace::open`.
   - Atualize o `USAGE`.
2. **Execução:** capture o `Handle` do runtime e rode `run_task` num `tokio::task::spawn_blocking`. Faça `select!` com `ctrl_c`, que cancela o `CancelToken` e espera a thread terminar.
3. **Saída compacta no terminal:**
   - `Token` direto no stdout;
   - `ToolCallRequested` e `ToolCallFinished` numa linha `→ read_file src/x.rs · ok`, no stderr;
   - `TaskFinished` imprime o status, o motivo e as evidências.
4. **Aprovação (D13):**
   - com stdin num TTY (`std::io::IsTerminal`), imprime o argv ou o diff exato e pergunta `Aprovar? [s/N]`;
   - sem TTY, imprime `sem terminal interativo: negado` e nega.
5. **Códigos de saída:**

   | Código | Situação |
   |---|---|
   | 0 | `completed_unvalidated` |
   | 1 | `failed` ou erro de modelo |
   | 130 | cancelada |
   | 2 | erro de uso |
6. **Fixture de aceite em `evals/fixtures/soma/`:**
   - `package.json` com `"scripts": { "test": "bun test" }`;
   - `src/soma.ts` com um bug proposital (`return a - b`);
   - `src/soma.test.ts`, que espera `soma(2, 3) === 5`;
   - `README.md` com o pedido de aceite.

   O fixture fica fora do `bun test` do `verify`, que roda só em `apps/desktop`.
7. **Documentação:** acrescente o comando `cd-ai task` às listas de comandos do `AGENTS.md` e do `README.md`.

**Testes em `tests/cli.rs`** (os existentes continuam; nenhum precisa do Ollama):

- `task_without_model_exits_2`;
- `task_with_missing_workspace_exits_1_with_message`;
- `task_with_unreachable_ollama_fails_cleanly`: com `OLLAMA_HOST=http://127.0.0.1:9` e `CD_AI_DATA_DIR` num `tempdir`, sai com 1 e a mensagem de erro no stderr;
- `help_mentions_task`.

**Verificar:** `cargo test -p cd-ai-cli` sai com `0 failed`, e `bun run verify` sai com exit 0.

### Parte G — UI (dono: `ui`; G1 na onda 1, G2 na onda 3)

**Skills:** `impeccable` e `apple-design`. Siga o `apps/desktop/DESIGN.md`: nada de modal para a aprovação, erros nunca recolhidos, verde-água só em ação primária, foco ou estado ao vivo.

**G1 — em paralelo com o core, usando só os bindings que já existem**

**Escopo:**

- `apps/desktop/src/lib/activity.ts` e `activity.test.ts` (novos)
- `apps/desktop/src/lib/session.ts`
- `apps/desktop/src/lib/demo-session.ts`
- `apps/desktop/src/components/approval-bar.tsx` (novo)
- `apps/desktop/src/components/conversation.tsx`
- `apps/desktop/src/components/icons.tsx`

**Passos:**

1. **`session.ts`:**
   - `Task` ganha `pendingApproval?: { id: string; action: ApprovalAction }` e `stopReason?: string`;
   - o evento `search` ganha `summary?: string` (a contagem vira opcional: `matches?: number`).
2. **`activity.ts`:** a função pura `applyToolEvent(task: Task, message: ToolEventMessage): Task`, que segue a tabela de reconciliação do design da Fase 4 (§7):

   | Evento | Efeito |
   |---|---|
   | `FileRead` | `read` |
   | `FileChanged` | `edit`, com `+`/`−` contados das linhas do diff |
   | `CommandStarted` | `command` com `exitCode: null` |
   | `CommandCompleted` | atualiza o comando pelo `id` |
   | `ApprovalRequired` | `status = "waiting_approval"` e `pendingApproval` |
   | `ApprovalGranted` / `ApprovalDenied` | limpa o `pendingApproval` e volta para `running` |
   | `ToolFailed` | evento com `error` (nunca recolhido) |

3. **`approval-bar.tsx`:** uma barra **inline** no fim da conversa, sem modal.
   - Mostra a ação **exata**: argv em mono (com a classe `read`/`write`/`network`/`destructive`/`unknown` e o cwd) ou o diff completo, com linhas `+` e `−` coloridas.
   - Tem os botões "Aprovar" (primário) e "Negar", e um campo opcional de motivo da negação.
   - O foco vai para a barra quando ela aparece. **Nenhum atalho global aprova por Enter.**
   - O `onDecision(granted, reason)` vem por prop; a G1 não chama IPC.
4. **`conversation.tsx`:** renderize a `ApprovalBar` quando `task.pendingApproval` existir e mostre o `stopReason` quando houver.
5. **`demo-session.ts`:** acrescente uma tarefa de demonstração com `pendingApproval` (comando `cargo test`) para inspecionar a tela em `?demo`.
6. **`icons.tsx`:** acrescente o ícone `stop`.

**Testes em `activity.test.ts`:** cada linha da tabela do passo 2, incluindo que `groupActivity` continua recolhendo leituras e que um comando que falha abre.

**Verificar:** `bun run --cwd apps/desktop test` e `bun run --cwd apps/desktop typecheck` passam. **Inspeção visual:** `bun run --cwd apps/desktop dev` e `http://localhost:1420/?demo`, nos temas claro e escuro. Por fim, `bun run verify` (ver "Gate compartilhado").

**G2 — depois da Parte E**

**Escopo:**

- `apps/desktop/src/lib/activity.ts` e `activity.test.ts`
- `apps/desktop/src/components/app-shell.tsx`, `composer.tsx`, `sidebar.tsx` e `conversation.tsx`

**Passos:**

1. **`activity.ts`:** `applyAgentEvent(task, message: AgentEventMessage): Task`, que delega `Tool(…)` ao `applyToolEvent`.
   - `UserMessage` vira `user`; `Token` acumula num `assistant` em andamento; `AssistantMessage` fecha o texto do turno.
   - `ModelTurnCompleted` atualiza `contextUsed` (`prompt_tokens`).
   - `StatusChanged` e `TaskFinished` atualizam `status` e `stopReason`; o `TaskFinished` também acrescenta um `report` com `validated: false` e as evidências.
   - **Fase exibida (heurística):**

     | Atividade | Fase |
     |---|---|
     | leitura ou busca | `explore` |
     | edição ou escrita | `implement` |
     | comando da classe `validate` | `validate` |

2. **`app-shell.tsx`:**
   - com um workspace aberto, carrega `listTasks()`;
   - ao selecionar uma tarefa antiga, faz replay com `taskEvents(id)` e `applyAgentEvent`;
   - mantém o `?demo` como está (só em desenvolvimento).
3. **`composer.tsx`:**
   - o botão de enviar é habilitado com um workspace aberto e um modelo disponível;
   - sem tarefa rodando, envia `startTask(texto, modelo, 16384, onEvent)`; com tarefa rodando, envia `steerTask`;
   - um botão **Cancelar** (ícone `stop`) fica sempre visível enquanto a tarefa roda;
   - o seletor de modelo usa `getOllamaStatus().models`, com padrão no modelo carregado e, na falta dele, o primeiro da lista;
   - remova o `title="O agente ainda não está conectado"`.
4. **`sidebar.tsx`:** "Nova tarefa" é habilitado com um workspace aberto (limpa a seleção e foca o composer).
5. **`conversation.tsx`:** a barra de aprovação chama `respondApproval(task.id, id, granted, reason)`. Numa tarefa `cancelled`/`Interrupted`, mostre o botão "Retomar", que chama `resumeTask`.

**Decisões do lead na revisão da G1 (valem para a G2):**

- A variante `failure` e o campo `id` do evento `command` estão aprovados; construa em cima deles.
- **A negação não precisa de linha própria.** Quando a aprovação é negada, a tool devolve `PermissionDenied`, o engine emite `ToolFailed` e a G1 já renderiza isso como linha de falha, sempre aberta. Não duplique.
- A demo usa um comando de classe `write` de propósito: comando `validate` é automático pela tabela de permissões e nunca pediria aprovação.

**Testes:** o `applyAgentEvent` cobre token → texto, uma tarefa completa (`TaskFinished` com relatório não validado) e o replay idempotente (aplicar a mesma lista de eventos duas vezes gera o mesmo `Task`).

**Verificar:** `bun run verify` sai com exit 0. A verificação manual no app é do lead (ver "Aceite").

## Ondas e notas de dependência

| Onda | Em paralelo | Por quê |
|---|---|---|
| 1 | **`core`** faz 0 → A → B → C → D, em sequência | Todas editam o crate `agent-core`. No mesmo working tree, uma parte com erro de compilação quebraria o `cargo test` das outras. A Parte 0 vem primeiro porque sem ela ninguém consegue um gate verde no Windows. |
| 1 | **`ui`** faz a G1 | Só toca TS em `apps/desktop` e usa bindings que já existem (`ToolEventMessage`, `ApprovalAction`). Não toca `ipc.ts` nem `bindings/`. |
| 2 | **`bridge`** (E) e **`cli`** (F) | Crates diferentes (`src-tauri`, `apps/cli`). Os dois só **leem** o `agent-core`. Só o `bridge` edita `ipc.ts`. |
| 3 | **`ui`** faz a G2 | Depende dos wrappers de `ipc.ts` da E e dos bindings da C. |

**Arquivos disputados (um dono por vez):**

| Arquivo | Donos, na ordem |
|---|---|
| `apps/desktop/src/lib/bindings/` | regenerado pela Parte A, depois B, C e D (só o `core`) |
| `crates/agent-core/src/tools/command.rs` | Parte 0 (só testes), depois Parte B |
| `apps/cli/src/main.rs` | Parte A (dois literais), depois Parte F |
| `crates/agent-core/src/lib.rs` | só a Parte C |
| `apps/desktop/src/components/conversation.tsx` | G1, depois G2 |
| `AGENTS.md` e `README.md` | só a Parte F |

**Gate compartilhado:** todos trabalham no mesmo working tree. Não há worktrees, porque sem commits não haveria como levar o trabalho de uma onda para a seguinte. O `bun run verify` compila o workspace inteiro, então um teammate pode ver o gate falhar por causa de um arquivo **fora do seu escopo** que outra parte está editando naquele momento. Nesse caso:

1. Não mexa no arquivo alheio.
2. Rode os checks do seu escopo (`cargo test -p <crate> <módulo>` ou `bun run --cwd apps/desktop test`/`typecheck`).
3. Avise o lead e rode o `verify` de novo quando ele liberar.

**Uma parte nunca é reportada como pronta com o gate vermelho.**

## Comandos

| Propósito | Comando | Esperado |
|---|---|---|
| Gate | `bun run verify` | exit 0 |
| Core por módulo | `cargo test -p agent-core <ollama\|tools\|agent>` | `0 failed` |
| CLI | `cargo test -p cd-ai-cli` | `0 failed` |
| Bindings | `cargo test -p agent-core export_bindings` | arquivos em `apps/desktop/src/lib/bindings/` atualizados |
| UI | `bun run --cwd apps/desktop test` e `bun run --cwd apps/desktop typecheck` | `0 fail` e exit 0 |
| App | `bun tauri build --no-bundle` | `Built application at:` |

No Windows, se o `cargo` não for encontrado: `$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"`.

## Aceite da Fase 5 (lead, depois da onda 3)

1. Copie `evals/fixtures/soma/` para uma pasta temporária **fora do repositório**.
2. Com o Ollama rodando e o modelo CODER da decisão 0002 disponível, rode:

   ```bash
   cd-ai task --workspace <cópia> --model <modelo> "O teste de soma falha. Corrija e rode os testes."
   ```

   Aprove as ações pelo terminal.
3. **Esperado:** o agente lê o arquivo, edita a função, roda `bun test` (classe `validate`), e o comando sai com código 0 e status `completed_unvalidated`, com evidências.
4. Repita pela UI.
5. Registre o resultado, o modelo e o tempo em `docs/audit/fase-5-aceite.md`.

## Critérios de pronto (plano inteiro)

- [x] Partes 0 e A a G com o gate verde (em `main`: `193d198` … `57c522b`)
- [x] `grep -rn "\"root\"" src-tauri/src` não retorna nada (D4)
- [x] `grep -rn "runTool\|run_tool" apps/desktop/src src-tauri` não retorna nada (D4)
- [x] `grep -rn "O agente ainda não está conectado" apps/desktop/src` não retorna nada
- [x] As três listas de ACL (`generate_handler!`, `build.rs` e a capability) batem
- [x] `bun run verify` — a rodar no fechamento (Linux); o Windows 11 da máquina de dev não está neste ambiente
- [x] O aceite está registrado em `docs/audit/fase-5-aceite.md` (CLI ao vivo 2026-09-12 reprovou; loop determinístico 2026-09-15 passou; CLI/UI ao vivo 2026-09-15 sem Ollama)
- [x] A linha 015 em `plans/README.md` está atualizada, e o `docs/handoff.md` reflete a Fase 5 (incluindo a correção do "gate verde" da base)
- [ ] **Nenhum commit, push ou reset feito por executor** — vale para as ondas originais; o fechamento de 2026-09-15 commitou a pedido do usuário (PR da Fase 5)

## STOP conditions

- O Ollama 0.34 não devolve `tool_calls` com `stream: true` para o modelo testado. Reporte o JSON real; não troque para `stream: false` sem registrar a decisão.
- `Handle::block_on` entra em pânico ("Cannot start a runtime from within a runtime") no ponto de uso. Isso significa que o loop foi chamado de dentro de uma task async. Reporte; não troque D1 por conta própria.
- O cancelamento de um `run_command` não mata os processos filhos no Windows (o `taskkill /T` falha). Reporte os PIDs que sobreviveram.
- Alguma parte precisaria que o frontend decidisse permissão, montasse um path ou executasse algo. Isso é violação da fronteira de confiança.
- Uma dependência nova além do `tokio` no `agent-core` parece necessária.
- O gate falha por causa de um arquivo do seu escopo depois de duas tentativas de correção.

## Notas de manutenção

- **Todo command novo entra nas três listas de ACL** (plano 006).
- **Todo tipo IPC novo** leva `#[derive(TS)] #[ts(export)]` e `u64 → #[ts(type = "number")]` (plano 008).
- **Continuidade (decisão 0008):** o `trim_for_budget` desta fase é um corte explícito. A compactação automática com resumo é da Fase 10 e deve substituir o `ContextExhausted`.
- **Verifier (Fase 8):** vai trocar `completed_unvalidated` por `completed` quando houver evidência de validação depois da última edição. Até lá, o loop **nunca** produz `completed` (D11).
- **Pendente do SPEC §29:** "editar" o comando na aprovação e o modo plano (read-only) ficam para depois. A barra de aprovação tem só aprovar e negar.
- O `recover_interrupted` roda na inicialização do app. A CLI chama o mesmo método antes de um `--resume`.
- **Comando cancelado não devolve output parcial** (decisão do lead na revisão da Parte B): o `run_command` devolve `Err(ToolError::Cancelled)` sem montar o `CommandResult`. O que o comando chegou a fazer já aparece na conversa pelos eventos `CommandStarted`/`CommandCompleted`, e no relatório ele entra com `exit_code` vazio. Se algum dia o parcial for necessário, o lugar é o `RunStatus`, não o `TaskReport`.
- **Gate na base (`040369a`, Windows 11, 2026-09-11):** vermelho.
  - Biome, typecheck e testes TS (3/3) passam.
  - O `cargo test -p agent-core` passa em 135, falha em 3 (`tools::command`: `echo`, `sleep` e `ls` ausentes do PATH) e ignora 2.
  - A Parte 0 corrige.
