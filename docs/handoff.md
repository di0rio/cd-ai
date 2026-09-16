# Handoff — estado do cd-ai

Estado em 2026-09-16, sobre `main` (Fases 0–9, PR #7) mais o trabalho da **Fase 10**
(plano 020, esta PR). Serve pra uma sessão/agente novo continuar sem depender da conversa anterior.

- **Fontes da verdade:** [`AGENTS.md`](../AGENTS.md) (regras + comandos), [`SPEC.md`](../SPEC.md) (§12.1, §16 e §34), [`plans/README.md`](../plans/README.md), [`docs/decisions/`](decisions/) (ADRs), [`plans/020-fase-10-context-manager.md`](../plans/020-fase-10-context-manager.md).
- **Gate:** qualquer mudança só está pronta com `bun run verify` passando (exit 0).

## O que já está pronto (Fases 0–10)

Tudo em `main` até a Fase 9; Fase 10 nesta PR. Gate: `bun run verify`.

- **Núcleo Rust** ([`crates/agent-core/`](../crates/agent-core/)): workspace, Ollama, Tool Engine, loop, eval, permissões/sandbox, Verifier, checkpoints, e agora o Context Manager:
  - `agent/repo_map.rs` — walk `ignore` + tree-sitter (TS/JS/Rust); cache em `<data_dir>/cache/<sha256 do root>/` invalidado por mtime+tamanho.
  - `agent/context.rs` — orçamento por seção, higiene (resultado obsoleto, log enorme, duplicata), compactação determinística antes de `ContextExhausted`.
  - `agent/role.rs` — Explorer (prompt + tools só de leitura) vs Coder; classificação léxica. Nas tarefas de código o Explorer é um briefing injetado, não um turno extra de LLM.
- **Sandbox / modos** da Fase 7, **Verifier** da Fase 8 e **checkpoints** da Fase 9 continuam.
- **Eval:** scripted 3/3. Sucesso do harness continua sendo o exit do check (016 D3). `promptTokens` no scripted passa a ser `chars/4` quando o modelo reporta 0.

## Fase 10 — aceite

- Repo map no system prompt, ranqueado para o pedido; secrets e gitignore fora.
- Orçamento por seção com corte explícito (`[truncated]` / `[repo map truncated]`).
- Cache invalidado por mudança de arquivo, não por TTL.
- Higiene: versão antiga do mesmo path some; log enorme de comando reduz às linhas decisivas.
- Explorer é role nas perguntas; Coder recebe o briefing nas tarefas de edição.
- Eval scripted: `cargo run -q -p cd-ai-cli -- eval --scripted` → 3/3.
- Detalhe: [`docs/audit/fase-10-context-manager.md`](audit/fase-10-context-manager.md).

## O que ainda falta (Fases 11–14)

| Fase | O que falta |
|---|---|
| 11 — Skills | Registry, licenças, Skill Router. |
| 12 — Model Router/memória | Roteamento FAST/CODER/REASONER, memória, trajetórias. |
| 13 — Otimização | Medir primeiro, otimizar com base em métrica. |
| 14 — Release | Empacotamento `.deb`/AppImage e demais formatos. Sandbox Windows/macOS se algum dia for alvo de release. |

Detalhes no `SPEC.md` §34. Skills no orçamento de contexto ficam em 0% até a Fase 11.

## Próximo passo recomendado

**Fase 11 (Skills).** A taxa ao vivo com `qwen3-coder:30b` ainda pode ser registrada numa máquina com Ollama; não bloqueia a 11.

## Pontos de atenção pro próximo executor

- **Código e comentários em inglês; UI e documentação em pt-BR.**
- Fronteira de confiança é o Rust: path, permissões, sandbox, execução, secrets, Verifier, rollback e **o que entra no prompt** nunca caem no frontend.
- O snapshot do shadow **não** passa pelo sandbox do `run_command` (o git dir está fora do workspace). As tools `git_*` **passam**.
- FULL ACCESS **não** é um interruptor na webview: `set_permission_mode` recusa se `sandbox::status().available` for falso.
- Quem gerar tipos novos no core roda `cargo test -p agent-core` (o `ts-rs` exporta em `apps/desktop/src/lib/bindings/`); campo `u64` vira `bigint` a menos que tenha `#[ts(type = "number")]`.
- Um `ToolEngine` por tarefa. `run_tool` / `runTool` / `task_id: "root"` não existem mais (D4).
- O gate depende de `cargo` (edition 2024, Rust 1.85+) e `bun`. Testes de sandbox Linux precisam de Landlock (ABI ≥ 1) e, para bloqueio total de rede, user namespace. O Verifier dispara `bun`/`cargo` via PATH. Checkpoints precisam de `git` no PATH.
- `keep_alive: -1` está no corpo do `/api/chat`.
- Compostos continuam recusados (`CompoundCommand`) — o sandbox não liga shell.
