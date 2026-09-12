# Handoff — estado do cd-ai

Estado em 2026-09-12, sobre o commit `37f8b28` mais a árvore de trabalho local — **ainda não commitada** (dezenas de arquivos modificados e novos; ver `git status`). Serve pra uma sessão/agente novo continuar o trabalho sem depender da conversa anterior.

- **Fontes da verdade:** [`AGENTS.md`](../AGENTS.md) (regras + comandos), [`SPEC.md`](../SPEC.md) (§34 define o roteiro), [`plans/README.md`](../plans/README.md) (status dos planos), [`docs/decisions/`](decisions/) (ADRs), [`docs/design/fase-4-tool-engine.md`](design/fase-4-tool-engine.md) (design da Fase 4, já implementada), [`plans/015-fase-5-loop-ponta-a-ponta.md`](../plans/015-fase-5-loop-ponta-a-ponta.md) (plano da Fase 5, em andamento) e [`docs/fase-5-o-que-falta.md`](fase-5-o-que-falta.md) (o que falta dele, atualizado a cada sessão).
- **Gate:** qualquer mudança só está pronta com `bun run verify` passando (exit 0).

**Correção sobre a versão anterior deste documento:** ela afirmava "141 passando, gate verde", medido num ambiente Unix. **No Windows 11, na base `040369a` (2026-09-11), o gate estava vermelho:** Biome, typecheck e os testes TS (3/3) passavam, mas `cargo test -p agent-core` passava em 135 testes, **falhava em 3** (`tools::command::tests::runs_echo_and_reports_exit`, `timeout_kills_and_marks_timed_out` e `running_command_runs_inside_workspace_cwd`, todos com `Io("não foi possível executar: program not found")` porque chamavam `echo`, `sleep` e `ls` como executáveis — ausentes do PATH no Windows) e ignorava 2. A **Parte 0 do plano 015** corrigiu isso: os testes de `tools/command.rs` passaram a usar o módulo `test_argv`, com o argv certo por plataforma (`cmd /C echo`/`powershell -NoProfile -Command "Start-Sleep..."` no Windows, `echo`/`sleep` no Unix). Hoje, na árvore de trabalho local (não commitada), o gate volta a sair com exit 0.

## O que já está pronto e commitado (Fases 0–4)

Tudo em `main`, no commit `37f8b28`, gate verde na origem (Unix) e, com a Parte 0 do plano 015 já aplicada na árvore local, também no Windows.

- **Núcleo Rust** ([`crates/agent-core/`](../crates/agent-core/)): workspace com validação de path/symlink, cliente Ollama com streaming cancelável, parser tolerante de tool calls, e a Fase 4:
  - `tools/` — `read_file`, `list_directory`, `search`, `edit_file`, `write_file`, `run_command`; `ToolEngine` com sequência resolve → permissão → redata → evento; checkpoint SHA-256 antes de editar; diff por `similar`; parse por `tree-sitter`; busca literal via `fixed_strings(true)`.
  - `permissions.rs` — `CommandClass`, `ApprovalAction` (tagged `type`), `PermissionManager` com requests idempotentes `aprv_NNNN`.
  - `redactor.rs` — redação de secrets (prefixos, PEM, JWT, userinfo, entropia janela 32/4.7/run2), sem regex.
  - `events.rs` — `ToolEvent`/`ToolEventMessage` achatado como `{ event, data, taskId, sequence, at }`.
- **Bridge do Tauri** ([`src-tauri/src/lib.rs`](../src-tauri/src/lib.rs)): na base commitada, `run_tool` (Channel de eventos, responder bloqueia em `mpsc`) e `respond_approval`; engine por workspace com `task_id: "root"` provisório. **Isso mudou na Fase 5 (ver abaixo): `run_tool` foi removido.**
- **TS/IPC** ([`apps/desktop/src/lib/ipc.ts`](../apps/desktop/src/lib/ipc.ts) + `bindings/`): tipos gerados do Rust (`u64` → `number`, regra do plano 008).
- **CLI** ([`apps/cli/`](../apps/cli/)): binário `cd-ai` compartilha o mesmo core; na base commitada só tinha `--version`, `--help` e `chat`.

## Fase 5 — loop de ponta a ponta (em andamento, não commitada)

Plano: [`plans/015-fase-5-loop-ponta-a-ponta.md`](../plans/015-fase-5-loop-ponta-a-ponta.md). O que falta, em detalhe e atualizado por sessão: [`docs/fase-5-o-que-falta.md`](fase-5-o-que-falta.md) — leia-o antes de continuar, ele tem mais frescor que esta seção.

Estado em 2026-09-12: as Partes a seguir estão **na árvore de trabalho local**, com `bun run verify` saindo com exit 0.

| Parte | O que entrega |
|---|---|
| 0 | Testes de `run_command` portáveis (Windows e Unix), via `test_argv` — corrige o gate vermelho descrito acima |
| A | Tool calling no cliente Ollama: `tools` no request, `tool_calls` na resposta, `ChatEvent::ToolCalls` |
| B | `CancelToken`, cancelamento no `ToolEngine` e morte da árvore de processos |
| C | `agent/`: `TaskState`, `AgentEvent`, `TaskStore` (persistência em `%APPDATA%\cd-ai`) e perfil determinístico do workspace |
| D | O loop `run_task`: `ChatModel`/`OllamaModel`, mapeamento de tool calls (nativo + fallback de texto), prompt, orçamento de contexto, limites e detecção de loop |
| E | Bridge Tauri: `start_task`, `resume_task`, `cancel_task`, `steer_task`, `respond_approval`, `list_tasks`, `task_events`; `run_tool` removido; ACL atualizada; wrappers em `ipc.ts` |
| F | CLI `cd-ai task` (com `--resume`), aprovação pelo terminal, e o fixture `evals/fixtures/soma/` para o aceite |
| G1 | Barra de aprovação (`approval-bar.tsx`) e `applyToolEvent` na conversa da UI |

As Partes E e F ainda não tiveram a revisão final do lead (o gate passou, mas a revisão de diff — ACL batendo, `resume` validado, nenhum `run_tool` sobrando — está pendente).

### O que ainda falta

1. **Parte G2 — ligar a UI ao loop.** Ainda não começou; `app-shell.tsx`, `composer.tsx` e `sidebar.tsx` estão intocados. Falta `applyAgentEvent` em `lib/activity.ts`, carregar tarefas do workspace (`listTasks`/`taskEvents`), o composer iniciar/corrigir/cancelar tarefa e escolher modelo, "Nova tarefa" habilitado na sidebar, a barra de aprovação chamando `respondApproval`, e um botão "Retomar" numa tarefa interrompida.
2. **Aceite da Fase 5.** Ainda não foi executado. Procedimento e registro (a preencher) em [`docs/audit/fase-5-aceite.md`](audit/fase-5-aceite.md).
3. **Commit.** Nada foi commitado — dezenas de arquivos na árvore. Commit e push dependem de pedido explícito do usuário (AGENTS.md).

## Próximo passo recomendado

Terminar a Parte G2 do plano 015 (ligar a UI ao loop), depois rodar o aceite da Fase 5 (CLI e UI, com o fixture `evals/fixtures/soma/`) e registrar o resultado em `docs/audit/fase-5-aceite.md`. Só depois disso a Fase 5 está pronta para ser commitada — com pedido explícito do usuário.

## Pontos de atenção pro próximo executor

- **Código e comentários em inglês; UI e documentação em pt-BR.**
- Fronteira de confiança é o Rust: path, permissões, execução e secrets nunca caem no frontend.
- Quem gerar tipos novos no core roda `cargo test -p agent-core export_bindings` pra regenerar os TS; campo `u64` vira `bigint` a menos que tenha `#[ts(type = "number")]`.
- **O `task_id: "root"` provisório do Tauri foi removido na Parte E** (decisão D4 do plano 015): agora é um `ToolEngine` por tarefa, e o command `run_tool`/wrapper `runTool` não existem mais.
- O gate depende de `cargo`, `bun` e, para o aceite, do Ollama rodando com o modelo CODER da decisão 0002 baixado.
- Contexto de sessão anterior (ex.: escolha do `fixed_strings(true)`, flatten de `ToolEventMessage`, as 15 decisões D1–D15 do plano 015) está registrado no código, comentários, testes e no próprio plano — não na conversa.
- Processos de sessão anterior (ex.: `bun run dev`) não sobrevivem — suba de novo.
