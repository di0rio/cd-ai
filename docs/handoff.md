# Handoff — estado do cd-ai

Estado em 2026-09-16, sobre `main` (Fases 0–9, PR #7) mais o trabalho da **Fase 10**
(plano 020, esta PR). Serve pra uma sessão/agente novo continuar sem depender da conversa anterior.

- **Fontes da verdade:** [`AGENTS.md`](../AGENTS.md) (regras + comandos), [`SPEC.md`](../SPEC.md) (§12.1, §16 e §34), [`plans/README.md`](../plans/README.md), [`docs/decisions/`](decisions/) (ADRs, inclusive [`0008`](decisions/0008-continuidade-sem-teto-de-token.md) agora aceita), [`plans/020-fase-10-context-manager.md`](../plans/020-fase-10-context-manager.md).
- **Gate:** qualquer mudança só está pronta com `bun run verify` passando (exit 0).

## O que já está pronto (Fases 0–10)

Tudo em `main` até a Fase 9; Fase 10 nesta PR. Gate: `bun run verify`.

- **Núcleo Rust** ([`crates/agent-core/`](../crates/agent-core/)): workspace, Ollama, Tool Engine, loop, eval, permissões/sandbox, Verifier, checkpoints, e agora o Context Manager:
  - `syntax.rs` — grammar tree-sitter por extensão (reusado por `edit` / Verifier / repo map).
  - `agent/repo_map.rs` — walk `ignore` + assinaturas TS/Rust, score pelo pedido, cache em `<data_dir>/cache/<sha256(root)>/repo-map.json` (mtime+tamanho).
  - `agent/context.rs` — orçamento por seção, higiene (leitura obsoleta, logs), compactação automática (decisão 0008).
  - `agent/role.rs` — classificação determinística; Explorer (só leitura) em `pergunta`/`complexa`; Coder no resto, sempre com o map no prompt.
- **Eval:** scripted 3/3. Sucesso do harness continua sendo o exit do check (016 D3). O JSON passa a trazer `estimatedPromptTokens` (`chars/4` por turno).

## Fase 10 — aceite

- Repo map tree-sitter entra no system prompt, priorizado pelo pedido.
- Cache reindexa só o arquivo que mudou (teste em `repo_map.rs`).
- Seção que estoura o orçamento é cortada e emite `contextBudgetCut`.
- Higiene omite `read_file` de path já editado e compacta logs enormes.
- Janela perto do teto: compacta o histórico (fatos do engine) em vez de parar; `ContextExhausted` só se o **pedido** sozinho não cabe.
- Explorer recusa `edit_file` / `write_file` / `run_command` no loop, não só na lista de tools.
- Eval scripted: `cargo run -q -p cd-ai-cli -- eval --scripted` → 3/3.
- Detalhe: [`docs/audit/fase-10-context.md`](audit/fase-10-context.md).

## O que ainda falta (Fases 11–14)

| Fase | O que falta |
|---|---|
| 11 — Skills | Registry, licenças, Skill Router. |
| 12 — Model Router/memória | Roteamento FAST/CODER/REASONER, memória, trajetórias. |
| 13 — Otimização | Medir primeiro, otimizar com base em métrica. |
| 14 — Release | Empacotamento `.deb`/AppImage e demais formatos. Sandbox Windows/macOS se algum dia for alvo de release. |

Detalhes no `SPEC.md` §34. Commits automáticos no git do **usuário** continuam desligados (SPEC §19). Status `verifying`/`reviewing`/`fixing` (§23) ficaram de fora da Fase 8 de propósito.

## Próximo passo recomendado

**Fase 11 (Skills).** A taxa ao vivo com `qwen3-coder:30b` ainda pode ser registrada numa máquina com Ollama (comando no audit da Fase 10); não bloqueia a 11.

## Pontos de atenção pro próximo executor

- **Código e comentários em inglês; UI e documentação em pt-BR.**
- Fronteira de confiança é o Rust: path, permissões, sandbox, execução, secrets, Verifier, rollback e o que entra no prompt nunca caem no frontend.
- Tool results continuam marcados como não confiáveis. O resumo de compactação usa só estado do engine (`files_changed`, `commands`).
- FULL ACCESS **não** é um interruptor na webview: `set_permission_mode` recusa se `sandbox::status().available` for falso.
- Quem gerar tipos novos no core roda `cargo test -p agent-core` (o `ts-rs` exporta em `apps/desktop/src/lib/bindings/`); campo `u64` vira `bigint` a menos que tenha `#[ts(type = "number")]`.
- Um `ToolEngine` por tarefa. `run_tool` / `runTool` / `task_id: "root"` não existem mais (D4).
- O gate depende de `cargo` (edition 2024, Rust 1.85+) e `bun`. Testes de sandbox Linux precisam de Landlock (ABI ≥ 1) e, para bloqueio total de rede, user namespace. O Verifier dispara `bun`/`cargo` via PATH. Checkpoints precisam de `git` no PATH.
- `keep_alive: -1` está no corpo do `/api/chat`.
- Compostos continuam recusados (`CompoundCommand`) — o sandbox não liga shell.
