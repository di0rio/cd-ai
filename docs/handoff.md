# Handoff — estado do cd-ai

Estado em 2026-09-16, sobre `main` (Fases 0–7, PR #4) mais o trabalho da **Fase 8**
(plano 018, esta PR). Serve pra uma sessão/agente novo continuar sem depender da conversa anterior.

- **Fontes da verdade:** [`AGENTS.md`](../AGENTS.md) (regras + comandos), [`SPEC.md`](../SPEC.md) (§13, §23 e §34), [`plans/README.md`](../plans/README.md), [`docs/decisions/`](decisions/) (ADRs), [`plans/018-fase-8-verifier-e-ciclo-de-correcao.md`](../plans/018-fase-8-verifier-e-ciclo-de-correcao.md).
- **Gate:** qualquer mudança só está pronta com `bun run verify` passando (exit 0).

## O que já está pronto (Fases 0–8)

Tudo em `main` até a Fase 7; Fase 8 nesta PR. Gate: `bun run verify`.

- **Núcleo Rust** ([`crates/agent-core/`](../crates/agent-core/)): workspace, Ollama, Tool Engine, loop, eval, permissões/sandbox, e agora o Verifier:
  - `agent/verify.rs` — parse tree-sitter, checks de diff, detecção de argv de validação, review LLM opcional.
  - O loop, no turno sem tools, **executa** a verificação. `completed` só com evidência (comando `validate` exit 0 depois da última edição). Sem evidência: `completed_unvalidated`.
  - Ciclo de correção: até 3 voltas (`AgentLimits.max_correction_retries`). No limite, reporta o estado real — não mascara como `failed`.
  - Review por LLM depois do PASS determinístico, conversa isolada; resposta vazia/ilegível = skip (eval scripted não quebra).
- **Sandbox / modos** da Fase 7 continuam: ASK/AUTO/FULL ACCESS, Landlock+netns no Linux.
- **Eval:** scripted 3/3; as três tarefas passam a terminar `completed` (validado). Sucesso do harness continua sendo o exit do check (016 D3).

## Fase 8 — aceite

- `completed` quando parse+diff passam e há `validate` exit 0 após a última edição.
- Sem arquivos alterados, ou workspace sem comando de validação → `completed_unvalidated`.
- Falha de check reentra no loop; no limite, unvalidated com as razões.
- Falha determinística prevalece sobre `PASS` do LLM.
- Eval scripted: `cargo run -q -p cd-ai-cli -- eval --scripted` → 3/3, `agentStatus: completed`.
- Detalhe: [`docs/audit/fase-8-verifier.md`](audit/fase-8-verifier.md).

## O que ainda falta (Fases 9–14)

| Fase | O que falta |
|---|---|
| 9 — Checkpoints/Git/histórico | Shadow repo, rollback, tools de git, histórico por workspace. |
| 10 — Context Manager | Repo map (tree-sitter), orçamento por seção, cache, Explorer role. Hoje: perfil + corte explícito (`ContextExhausted`) + marcação de não confiável + Verifier. |
| 11 — Skills | Registry, licenças, Skill Router. |
| 12 — Model Router/memória | Roteamento FAST/CODER/REASONER, memória, trajetórias. |
| 13 — Otimização | Medir primeiro, otimizar com base em métrica. |
| 14 — Release | Empacotamento `.deb`/AppImage e demais formatos. Sandbox Windows/macOS se algum dia for alvo de release. |

Detalhes no `SPEC.md` §34. Status `verifying`/`reviewing`/`fixing` (§23) ficaram de fora da Fase 8 de propósito (D9 do 018): o loop permanece `running` durante o ciclo.

## Próximo passo recomendado

**Fase 9 (checkpoints, git e histórico).** A taxa ao vivo com `qwen3-coder:30b` ainda pode ser registrada numa máquina com Ollama; não bloqueia a 9.

## Pontos de atenção pro próximo executor

- **Código e comentários em inglês; UI e documentação em pt-BR.**
- Fronteira de confiança é o Rust: path, permissões, sandbox, execução, secrets e o Verifier nunca caem no frontend.
- FULL ACCESS **não** é um interruptor na webview: `set_permission_mode` recusa se `sandbox::status().available` for falso.
- Quem gerar tipos novos no core roda `cargo test -p agent-core` (o `ts-rs` exporta em `apps/desktop/src/lib/bindings/`); campo `u64` vira `bigint` a menos que tenha `#[ts(type = "number")]`.
- Um `ToolEngine` por tarefa. `run_tool` / `runTool` / `task_id: "root"` não existem mais (D4).
- O gate depende de `cargo` (edition 2024, Rust 1.85+) e `bun`. Testes de sandbox Linux precisam de Landlock (ABI ≥ 1) e, para bloqueio total de rede, user namespace. O Verifier dispara `bun`/`cargo` via PATH.
- `keep_alive: -1` está no corpo do `/api/chat`.
- Compostos continuam recusados (`CompoundCommand`) — o sandbox não liga shell.
