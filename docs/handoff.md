# Handoff — estado do cd-ai

Estado em 2026-09-16, sobre `main` (Fases 0–11, PR #11) mais o trabalho da **Fase 12**
(plano 022, esta PR). Serve pra uma sessão/agente novo continuar sem depender da conversa anterior.

- **Fontes da verdade:** [`AGENTS.md`](../AGENTS.md) (regras + comandos), [`SPEC.md`](../SPEC.md) (§10, §24.2, §25 e §34), [`plans/README.md`](../plans/README.md), [`docs/decisions/`](decisions/) (ADRs), [`plans/022-fase-12-model-router-memoria-trajetorias.md`](../plans/022-fase-12-model-router-memoria-trajetorias.md).
- **Gate:** qualquer mudança só está pronta com `bun run verify` passando (exit 0).

## O que já está pronto (Fases 0–12)

Tudo em `main` até a Fase 11; Fase 12 nesta PR. Gate: `bun run verify`.

- **Núcleo Rust** ([`crates/agent-core/`](../crates/agent-core/)): workspace, Ollama, Tool Engine, loop, eval, permissões/sandbox, Verifier, checkpoints, Context Manager, Skills, e agora o Model Router:
  - `agent/router.rs` — função pura FAST / CODER / REASONER (nomes são config; preferência pelo residente; FAST não edita se houver CODER distinto; janela 8k em pergunta/trivial; escalonamento após 2 falhas de qualidade; RAM via `/proc/meminfo`).
  - `agent/memory.rs` — JSON local por workspace, kinds da §24.2, orçamento 5% do `num_ctx`, redactor no disco.
  - `agent/trajectory.rs` — JSONL opt-in, desligado por padrão, redigido, sem output de tool.
  - Eventos `modelRouted` / `memoryLoaded`; `TaskState` com categoria, motivo e `coderFailures`.
  - Settings: `fast` / `coder` / `reasoner` / `trajectories`. CLI `memory` e `--trajectories`.
- **Eval:** scripted 3/3. JSON traz `modelCategory`, `routedNumCtx`, `routeReason`. Evidência de latência em [`docs/audit/fase-12-router.md`](audit/fase-12-router.md) (`expected_prefill_cost`).

## Fase 12 — aceite

- Router puro: mesmas entradas → mesma categoria, nome, `num_ctx`, motivo.
- FAST distinto não edita; pergunta com CODER já carregado fica no CODER.
- Janela `min(pedido, 8192)` em `question`/`trivial`; custo de prefill menor que `normal` no mesmo pedido 16k.
- Duas falhas de qualidade (Verifier ou tool call inválida) escalam para REASONER se o nome for distinto.
- Memória: stale fora; constraints/rules primeiro; secrets redigidos; modelo não grava sozinho.
- Trajetórias nascem desligadas; com flag/settings gravam JSONL local.
- Eval scripted: `cargo run -q -p cd-ai-cli -- eval --scripted` → 3/3, fixtures triviais em 8k (`coder/8192`).
- Detalhe: [`docs/audit/fase-12-router.md`](audit/fase-12-router.md).

## O que ainda falta (Fases 13–14)

| Fase | O que falta |
|---|---|
| 13 — Otimização | Medir primeiro, otimizar com base em métrica (contexto, cache, tool calls, paralelismo de leituras, roteamento, filesystem, UI). |
| 14 — Release | Empacotamento `.deb`/AppImage e demais formatos. Sandbox Windows/macOS se algum dia for alvo de release. |

Detalhes no `SPEC.md` §34. Commits automáticos no git do **usuário** continuam desligados (SPEC §19). Status `verifying`/`reviewing`/`fixing` (§23) ficaram de fora da Fase 8 de propósito. Marketplace / importação de skills do usuário ficaram de fora da Fase 11 de propósito. UI completa de categorias FAST/CODER/REASONER ficou de fora da Fase 12 (IPC + settings bastam; o picker continua sendo o default).

## Próximo passo recomendado

**Fase 13 (Otimização).** Medir primeiro. A taxa ao vivo com `qwen3-coder:30b` ainda pode ser registrada numa máquina com Ollama (comando no audit da Fase 12); não bloqueia a 13.

## Pontos de atenção pro próximo executor

- **Código e comentários em inglês; UI e documentação em pt-BR.**
- Fronteira de confiança é o Rust: path, permissões, sandbox, execução, secrets, Verifier, rollback, o que entra no prompt, **quais skills carregam** e **qual modelo a tarefa usa** nunca caem no frontend.
- Tool results continuam marcados como não confiáveis. O resumo de compactação usa só estado do engine (`files_changed`, `commands`). Memória não é extraída do modelo.
- FULL ACCESS **não** é um interruptor na webview: `set_permission_mode` recusa se `sandbox::status().available` for falso.
- Quem gerar tipos novos no core roda `cargo test -p agent-core` (o `ts-rs` exporta em `apps/desktop/src/lib/bindings/`); campo `u64` vira `bigint` a menos que tenha `#[ts(type = "number")]`.
- Um `ToolEngine` por tarefa. `run_tool` / `runTool` / `task_id: "root"` não existem mais (D4).
- O gate depende de `cargo` (edition 2024, Rust 1.85+) e `bun`. Testes de sandbox Linux precisam de Landlock (ABI ≥ 1) e, para bloqueio total de rede, user namespace. O Verifier dispara `bun`/`cargo` via PATH. Checkpoints precisam de `git` no PATH.
- `keep_alive: -1` está no corpo do `/api/chat`.
- Compostos continuam recusados (`CompoundCommand`) — o sandbox não liga shell.
- Trajetórias e memória ficam no diretório de dados do app (`CD_AI_DATA_DIR`); nada sobe à rede (decisão 0004).
