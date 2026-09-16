# Handoff — estado do cd-ai

Estado em 2026-09-16, sobre `main` (Fases 0–9, PR #7) mais o trabalho da **Fase 10**
(plano 020, esta PR). Serve pra uma sessão/agente novo continuar sem depender da conversa anterior.

- **Fontes da verdade:** [`AGENTS.md`](../AGENTS.md) (regras + comandos), [`SPEC.md`](../SPEC.md) (§12.1, §16 e §34), [`plans/README.md`](../plans/README.md), [`docs/decisions/`](decisions/) (ADRs), [`plans/020-fase-10-context-manager.md`](../plans/020-fase-10-context-manager.md).
- **Gate:** qualquer mudança só está pronta com `bun run verify` passando (exit 0).

## O que já está pronto (Fases 0–10)

Tudo em `main` até a Fase 9; Fase 10 nesta PR. Gate: `bun run verify`.

- **Núcleo Rust** ([`crates/agent-core/`](../crates/agent-core/)): workspace, Ollama, Tool Engine, loop, eval, permissões/sandbox, Verifier, checkpoints, e agora o Context Manager:
  - `agent/repo_map.rs` — mapa de arquivos + símbolos (tree-sitter TS/JS/Rust), cache em `<data_dir>/context/<sha256(root)>/` invalidado por mtime/size.
  - `agent/context.rs` — orçamento por seção, higiene, classificação, Explorer findings determinísticos.
  - Explorer é **role** (prompt + tools de leitura) só em pergunta clara; demais tarefas são Coder com o mapa no system prompt. Sem turno extra de LLM.
  - Compactação determinística do miolo quando higiene+trim não bastam (ADR 0008 aceita). `ContextExhausted` continua no teto de 90%.
- **Sandbox / modos** da Fase 7, **Verifier** da Fase 8 e **checkpoints** da Fase 9 continuam.
- **Eval:** scripted 3/3. O JSON agora traz `estimatedTokens` / `peakEstimatedTokens` (`chars/4`). Sucesso do harness continua sendo o exit do check (016 D3).

## Fase 10 — aceite

- Repo map tree-sitter priorizado pela tarefa; cache reparse só o arquivo mudado.
- Seções respeitam o orçamento; corte explícito `[section truncated]`.
- Higiene omite leitura obsoleta e colapsa log grande.
- Pergunta clara → Explorer read-only; Coder no restante.
- Eval scripted: `cargo run -q -p cd-ai-cli -- eval --scripted` → 3/3.
- Detalhe: [`docs/audit/fase-10-context.md`](audit/fase-10-context.md).

## O que ainda falta (Fases 11–14)

| Fase | O que falta |
|---|---|
| 11 — Skills | Registry, licenças, Skill Router. |
| 12 — Model Router/memória | Roteamento FAST/CODER/REASONER, memória, trajetórias. |
| 13 — Otimização | Medir primeiro, otimizar com base em métrica. Watcher de filesystem se a métrica pedir. |
| 14 — Release | Empacotamento `.deb`/AppImage e demais formatos. Sandbox Windows/macOS se algum dia for alvo de release. |

Detalhes no `SPEC.md` §34. Commits automáticos no git do **usuário** continuam desligados (SPEC §19).

## Próximo passo recomendado

**Fase 11 (Skills).** A taxa ao vivo com `qwen3-coder:30b` ainda pode ser registrada numa máquina com Ollama; não bloqueia a 11. O ganho de tokens **ao vivo** também fica nessa máquina (scripted tem `promptTokens` = 0).

## Pontos de atenção pro próximo executor

- **Código e comentários em inglês; UI e documentação em pt-BR.**
- Fronteira de confiança é o Rust: path, permissões, sandbox, execução, secrets, Verifier, rollback e o Context Manager nunca caem no frontend.
- Classificação do Explorer é **conservadora**: na dúvida, Coder. Bloquear `edit_file` num script existente é pior do que um mapa a mais.
- O snapshot do shadow **não** passa pelo sandbox do `run_command`. As tools `git_*` **passam**.
- FULL ACCESS **não** é um interruptor na webview: `set_permission_mode` recusa se `sandbox::status().available` for falso.
- Quem gerar tipos novos no core roda `cargo test -p agent-core` (o `ts-rs` exporta em `apps/desktop/src/lib/bindings/`); campo `u64` vira `bigint` a menos que tenha `#[ts(type = "number")]`.
- Um `ToolEngine` por tarefa. `run_tool` / `runTool` / `task_id: "root"` não existem mais (D4).
- O gate depende de `cargo` (edition 2024, Rust 1.85+) e `bun`. Testes de sandbox Linux precisam de Landlock (ABI ≥ 1) e, para bloqueio total de rede, user namespace. O Verifier dispara `bun`/`cargo` via PATH. Checkpoints precisam de `git` no PATH.
- `keep_alive: -1` está no corpo do `/api/chat`.
- Compostos continuam recusados (`CompoundCommand`) — o sandbox não liga shell.
