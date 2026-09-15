# Handoff — estado do cd-ai

Estado em 2026-09-15, sobre `main` (Fases 0–5 fechadas, PR #2) mais o trabalho da **Fase 6**
(plano 016, esta PR). Serve pra uma sessão/agente novo continuar sem depender da conversa anterior.

- **Fontes da verdade:** [`AGENTS.md`](../AGENTS.md) (regras + comandos), [`SPEC.md`](../SPEC.md) (§26 e §34), [`plans/README.md`](../plans/README.md), [`docs/decisions/`](decisions/) (ADRs), [`docs/design/fase-4-tool-engine.md`](design/fase-4-tool-engine.md), [`plans/015-fase-5-loop-ponta-a-ponta.md`](../plans/015-fase-5-loop-ponta-a-ponta.md), [`plans/016-fase-6-eval-baseline.md`](../plans/016-fase-6-eval-baseline.md).
- **Gate:** qualquer mudança só está pronta com `bun run verify` passando (exit 0).

**Correção sobre versões anteriores deste documento:** a handoff da Fase 4 afirmava "141 passando, gate verde", medido num ambiente Unix. **No Windows 11, na base `040369a` (2026-09-11), o gate estava vermelho:** `cargo test -p agent-core` passava em 135 testes e **falhava em 3** (`echo`/`sleep`/`ls` ausentes do PATH). A Parte 0 do plano 015 corrigiu com `test_argv`. Uma revisão de 2026-09-12 ainda descrevia a Fase 5 como "não commitada" e a Parte G2 como não começada — isso era falso: as Partes 0–G já estavam em `main` (`193d198` … `57c522b`).

## O que já está pronto (Fases 0–5)

Tudo em `main` (fechamento do 015 na PR #2). Gate: `bun run verify`.

- **Núcleo Rust** ([`crates/agent-core/`](../crates/agent-core/)): workspace com validação de path/symlink, cliente Ollama com streaming cancelável, parser tolerante de tool calls, Tool Engine da Fase 4, e o loop da Fase 5:
  - `tools/` — `read_file`, `list_directory`, `search`, `edit_file`, `write_file`, `run_command`; `ToolEngine` (um por tarefa) com resolve → permissão → redata → evento; `CancelToken`; checkpoint SHA-256; busca literal via `fixed_strings(true)`. `kill_process_tree` tenta o grupo e, se falhar, `Child::kill`.
  - `agent/` — `run_task`, `TaskStore` (diretório de dados do app, override `CD_AI_DATA_DIR`), perfil determinístico do workspace, limites, detecção de loop, timeout que **não** conta a espera de aprovação.
  - `permissions.rs` / `redactor.rs` / `events.rs` — como na Fase 4.
  - Ollama: `tools` no request, `tool_calls` na resposta, `keep_alive: -1` (modelo residente durante aprovação; decisão 0002).
- **Bridge do Tauri** ([`src-tauri/src/lib.rs`](../src-tauri/src/lib.rs)): `start_task`, `resume_task`, `cancel_task`, `steer_task`, `respond_approval` (chaveado por tarefa), `list_tasks`, `task_events`. **Não existe** `run_tool` nem `task_id: "root"`.
- **TS/IPC** ([`apps/desktop/src/lib/ipc.ts`](../apps/desktop/src/lib/ipc.ts) + `bindings/`): tipos gerados do Rust (`u64` → `number`, regra do plano 008). Wrappers `startTask` / `respondApproval` / etc.
- **CLI** ([`apps/cli/`](../apps/cli/)): `cd-ai task --model … [--workspace …] "<pedido>"`; `--resume <id>`; aprovação no terminal (sem TTY, nega).
- **UI:** composer inicia / corrige rumo / cancela; barra de aprovação chama `respondApproval`; conversa consome `applyAgentEvent`; "Nova tarefa" e "Retomar" (só `Interrupted`).

## Fase 5 — aceite

Procedimento e registro: [`docs/audit/fase-5-aceite.md`](audit/fase-5-aceite.md).

- **2026-09-12, CLI + `qwen3-coder:30b`:** REPROVOU (`taskTimeout` enquanto o operador estava no prompt). O código da soma foi corrigido; os testes não chegaram a rodar. Causa (espera humana no deadline) e métrica `tool_ms` já foram corrigidas no runner.
- **2026-09-15, loop determinístico:** PASSOU. `agent::runner::tests::soma_fixture_is_fixed_and_its_tests_run` lê o fixture `evals/fixtures/soma/`, aplica a edição, roda `bun test` (classe `validate`, sem aprovação) e termina `completed_unvalidated`. Sem Ollama — o modelo é `ScriptedModel`.
- **Ao vivo (CLI/UI + CODER):** não executável neste ambiente (nenhum daemon Ollama em `127.0.0.1:11434`). Repetir numa máquina com o modelo da decisão 0002.

## Fase 6 — Eval baseline (plano 016, esta PR)

- Suíte em `evals/tasks/` + fixtures `soma`, `greet`, `dobro`.
- Runner headless: `cd-ai eval --model <nome>` (Ollama) e `cd-ai eval --scripted` (dry-run).
- Cada tarefa tem check (`bun test`) e timeout. Relatório em `evals/results/` (taxa, iterações, tokens, tempo, retries, falhas de formato, edições rejeitadas).
- O eval auto-aprova **só** na cópia da fixture. `cd-ai task` continua sem `--yes`.
- Baseline scripted **100% (3/3)** e o comando da taxa ao vivo: [`docs/audit/fase-6-baseline.md`](audit/fase-6-baseline.md).
- O `bun run verify` **não** dispara o eval ao vivo. Os testes scripted do harness entram via `cargo test --workspace`.

## O que ainda falta (Fases 7–14)

| Fase | O que falta |
|---|---|
| 7 — Permissões e sandbox | Sandbox de shell, rede bloqueada de fato, modos ASK/AUTO/FULL ACCESS. Hoje só ASK; `read`/`validate` são automáticos. |
| 8 — Verifier e ciclo de correção | Validação determinística + review por LLM. O loop **nunca** produz `completed` (D11). |
| 9 — Checkpoints/Git/histórico | Shadow repo, rollback, tools de git, histórico por workspace. |
| 10 — Context Manager | Repo map (tree-sitter), orçamento por seção, cache, Explorer role. Hoje: perfil + corte explícito (`ContextExhausted`). |
| 11 — Skills | Registry, licenças, Skill Router. |
| 12 — Model Router/memória | Roteamento FAST/CODER/REASONER, memória, trajetórias. |
| 13 — Otimização | Medir primeiro, otimizar com base em métrica. |
| 14 — Release | Empacotamento `.deb`/AppImage e demais formatos. |

Detalhes no `SPEC.md` §34.

## Próximo passo recomendado

**Fase 7 (permissões e sandbox).** A taxa ao vivo com `qwen3-coder:30b` ainda pode ser registrada numa máquina com Ollama (comando em `docs/audit/fase-6-baseline.md`); não bloqueia a 7.

## Pontos de atenção pro próximo executor

- **Código e comentários em inglês; UI e documentação em pt-BR.**
- Fronteira de confiança é o Rust: path, permissões, execução e secrets nunca caem no frontend.
- Quem gerar tipos novos no core roda `cargo test -p agent-core` (o `ts-rs` exporta em `apps/desktop/src/lib/bindings/`); campo `u64` vira `bigint` a menos que tenha `#[ts(type = "number")]`.
- Um `ToolEngine` por tarefa. `run_tool` / `runTool` / `task_id: "root"` não existem mais (D4).
- O gate depende de `cargo` (edition 2024, Rust 1.85+) e `bun`. O aceite ao vivo e o eval ao vivo dependem do Ollama com o modelo CODER da decisão 0002.
- `keep_alive: -1` está no corpo do `/api/chat`. `num_ctx` 8k e `OLLAMA_KV_CACHE_TYPE=q8_0` continuam abertos na revisão da 0002.
- Ampliar a suíte de eval é um JSON em `evals/tasks/` + uma pasta em `evals/fixtures/`. O runner não muda.
- Processos de sessão anterior (ex.: `bun run dev`) não sobrevivem — suba de novo.
