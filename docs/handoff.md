# Handoff — estado do cd-ai

Resumo do estado do projeto no commit `ce19df9` (2026-09-11). Serve pra uma sessão/agente novo continuar o trabalho sem depender da conversa anterior.

- **Fontes da verdade:** [`AGENTS.md`](../AGENTS.md) (regras + comandos), [`SPEC.md`](../SPEC.md) (§34 define o roteiro), [`plans/README.md`](../plans/README.md) (status dos planos), [`docs/decisions/`](decisions/) (ADRs), [`docs/design/fase-4-tool-engine.md`](design/fase-4-tool-engine.md) (design da Fase 4 já implementada).
- **Gate:** qualquer mudança só está pronta com `bun run verify` passando (exit 0).

## O que já está pronto (Fases 0–4)

Tudo commitado em `main`, gate verde. Testes: 141 passando em `agent-core`, clippy limpo com `-D warnings`.

- **Núcleo Rust** ([`crates/agent-core/`](../crates/agent-core/)): workspace com validação de path/symlink, cliente Ollama com streaming cancelável, parser tolerante de tool calls, e a Fase 4:
  - `tools/` — `read_file`, `list_directory`, `search`, `edit_file`, `write_file`, `run_command`; `ToolEngine` com sequência resolve → permissão → redata → evento; checkpoint SHA-256 antes de editar; diff por `similar`; parse por `tree-sitter`; busca literal via `fixed_strings(true)`.
  - `permissions.rs` — `CommandClass`, `ApprovalAction` (tagged `type`), `PermissionManager` com requests idempotentes `aprv_NNNN`.
  - `redactor.rs` — redação de secrets (prefixos, PEM, JWT, userinfo, entropia janela 32/4.7/run2), sem regex.
  - `events.rs` — `ToolEvent`/`ToolEventMessage` achatado como `{ event, data, taskId, sequence, at }`.
- **Bridge do Tauri** ([`src-tauri/src/lib.rs`](../src-tauri/src/lib.rs)): `run_tool` (Channel de eventos, responder bloqueia em `mpsc`) e `respond_approval`; engine por workspace; commands declarados no manifest do `build.rs` e nas permissões ([`capabilities/default.json`](../src-tauri/capabilities/default.json)).
- **TS/IPC** ([`apps/desktop/src/lib/ipc.ts`](../apps/desktop/src/lib/ipc.ts) + `bindings/`): tipos gerados do Rust (`u64` → `number`, regra do plano 008); wrappers `runTool`/`respondApproval` prontos.
- **CLI** ([`apps/cli/`](../apps/cli/)): binário `cd-ai` compartilha o mesmo core (a Fase 4 é "fora de escopo" para a CLI).

## O que ainda falta (Fases 5–14)

| Fase | O que falta |
|---|---|
| **5 — Loop de ponta a ponta** | O essencial: agente recebe a tarefa → contexto básico → chama as tools → volta ao modelo, com limites/timeout/cancelamento/detecção de loop/estado persistente, na UI e na CLI. Envolve: tela de aprovação na UI (o `respondApproval` existe, falta quem chama), consumir os eventos na conversa e dirigir o `ToolEngine` (hoje só alcançável via IPC, nada o chama). |
| 6 — Eval baseline | Suíte `evals/` + runner na CLI; registrar taxa de sucesso inicial. |
| 7 — Permissões e sandbox | Sandbox de shell, rede bloqueada de fato, modos ASK/AUTO/FULL ACCESS. |
| 8 — Verifier e ciclo de correção | Validação determinística + review por LLM, correção com limite. |
| 9 — Checkpoints/Git/histórico | Shadow repo, rollback, tools de git, histórico por workspace. |
| 10 — Context Manager | Repo map (tree-sitter), orçamento por seção, cache, Explorer role. |
| 11 — Skills | Registry, licenças, Skill Router. |
| 12 — Model Router/memória | Roteamento FAST/CODER/REASONER, memória, trajetórias. |
| 13 — Otimização | Medir primeiro, otimizar com base em métrica. |
| 14 — Release | Empacotamento `.deb`/AppImage (já configurado em `bundle.targets`) e demais formatos. |

Detalhes no `SPEC.md` §34.

## Próximo passo recomendado

Criar o **plano 015 (Fase 5)** seguindo a convenção dos `plans/` (base: `SPEC.md` §34 Fase 5; drift check sobre o commit `ce19df9`). A Fase 5 é a que transforma o projeto de "UI com um engine de tools atrás" num agente que resolve tarefas de verdade — e libera o dogfooding no próprio repo.

## Pontos de atenção pro próximo executor

- **Código e comentários em inglês; UI e documentação em pt-BR.**
- Fronteira de confiança é o Rust: path, permissões, execução e secrets nunca caem no frontend.
- Quem gerar tipos novos no core roda `cargo test -p agent-core export_bindings` pra regenerar os TS; campo `u64` vira `bigint` a menos que tenha `#[ts(type = "number")]`.
- O engine em `src-tauri` usa `task_id: "root"` provisório — a Fase 5 deve criar um engine por tarefa.
- Contexto de sessão anterior (ex.: escolha do `fixed_strings(true)`, flatten de `ToolEventMessage`) está registrado no código, comentários e testes — não na conversa.
- Processos de sessão anterior (ex.: `bun run dev`) não sobrevivem — suba de novo.