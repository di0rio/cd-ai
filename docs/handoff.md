# Handoff — estado do cd-ai

Estado em 2026-09-16, sobre `main` (Fases 0–10, PR #10) mais o trabalho da **Fase 11**
(plano 021, esta PR). Serve pra uma sessão/agente novo continuar sem depender da conversa anterior.

- **Fontes da verdade:** [`AGENTS.md`](../AGENTS.md) (regras + comandos), [`SPEC.md`](../SPEC.md) (§17, §12.4 e §34), [`plans/README.md`](../plans/README.md), [`docs/decisions/`](decisions/) (ADRs), [`plans/021-fase-11-skills.md`](../plans/021-fase-11-skills.md).
- **Gate:** qualquer mudança só está pronta com `bun run verify` passando (exit 0).

## O que já está pronto (Fases 0–11)

Tudo em `main` até a Fase 10; Fase 11 nesta PR. Gate: `bun run verify`.

- **Núcleo Rust** ([`crates/agent-core/`](../crates/agent-core/)): workspace, Ollama, Tool Engine, loop, eval, permissões/sandbox, Verifier, checkpoints, Context Manager, e agora Skills:
  - `skills/` — registry com licença (allowlist SPDX), catálogo condensado original, Skill Router determinístico (perfil, paths, keywords, deps, conflitos, orçamento 10%).
  - `agent/profile.rs` — `frameworks` (react/nextjs/tailwind) a partir de manifests.
  - `agent/context.rs` — seção `Skills:` no system prompt, cortada no orçamento.
  - Eventos `skillsDetected` / `skillLoaded` / `skillSkipped`; `TaskState.selectedSkills`.
- **Eval:** scripted 3/3. JSON traz `selectedSkills`. Neutralidade documentada em [`docs/audit/fase-11-skills.md`](audit/fase-11-skills.md).

## Fase 11 — aceite

- Registry recusa skill sem licença de redistribuição; nomes de terceiro do catálogo conceitual **não** estão no binário.
- Versão condensada respeita o orçamento declarado (`chars/4`).
- Router é puro: mesmas entradas → mesmos nomes. `nextjs` puxa `react` → `typescript`. Conflito fica com o maior score.
- Skills **não** alteram path, sandbox ou `PermissionMode`.
- Eval scripted: `cargo run -q -p cd-ai-cli -- eval --scripted` → 3/3, com `typescript`+`testing`+`debugging` nas fixtures.
- Detalhe: [`docs/audit/fase-11-skills.md`](audit/fase-11-skills.md).

## O que ainda falta (Fases 12–14)

| Fase | O que falta |
|---|---|
| 12 — Model Router/memória | Roteamento FAST/CODER/REASONER, memória, trajetórias. |
| 13 — Otimização | Medir primeiro, otimizar com base em métrica. |
| 14 — Release | Empacotamento `.deb`/AppImage e demais formatos. Sandbox Windows/macOS se algum dia for alvo de release. |

Detalhes no `SPEC.md` §34. Commits automáticos no git do **usuário** continuam desligados (SPEC §19). Status `verifying`/`reviewing`/`fixing` (§23) ficaram de fora da Fase 8 de propósito. Marketplace / importação de skills do usuário ficaram de fora da Fase 11 de propósito.

## Próximo passo recomendado

**Fase 12 (Model Router, memória e trajetórias).** A taxa ao vivo com `qwen3-coder:30b` ainda pode ser registrada numa máquina com Ollama (comando no audit da Fase 11); não bloqueia a 12.

## Pontos de atenção pro próximo executor

- **Código e comentários em inglês; UI e documentação em pt-BR.**
- Fronteira de confiança é o Rust: path, permissões, sandbox, execução, secrets, Verifier, rollback, o que entra no prompt e **quais skills carregam** nunca caem no frontend.
- Tool results continuam marcados como não confiáveis. O resumo de compactação usa só estado do engine (`files_changed`, `commands`).
- FULL ACCESS **não** é um interruptor na webview: `set_permission_mode` recusa se `sandbox::status().available` for falso.
- Quem gerar tipos novos no core roda `cargo test -p agent-core` (o `ts-rs` exporta em `apps/desktop/src/lib/bindings/`); campo `u64` vira `bigint` a menos que tenha `#[ts(type = "number")]`.
- Um `ToolEngine` por tarefa. `run_tool` / `runTool` / `task_id: "root"` não existem mais (D4).
- O gate depende de `cargo` (edition 2024, Rust 1.85+) e `bun`. Testes de sandbox Linux precisam de Landlock (ABI ≥ 1) e, para bloqueio total de rede, user namespace. O Verifier dispara `bun`/`cargo` via PATH. Checkpoints precisam de `git` no PATH.
- `keep_alive: -1` está no corpo do `/api/chat`.
- Compostos continuam recusados (`CompoundCommand`) — o sandbox não liga shell.
