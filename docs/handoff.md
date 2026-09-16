# Handoff — estado do cd-ai

Estado em 2026-09-16, sobre `main` (Fases 0–6, PR #3) mais o trabalho da **Fase 7**
(plano 017, esta PR). Serve pra uma sessão/agente novo continuar sem depender da conversa anterior.

- **Fontes da verdade:** [`AGENTS.md`](../AGENTS.md) (regras + comandos), [`SPEC.md`](../SPEC.md) (§20 e §34), [`plans/README.md`](../plans/README.md), [`docs/decisions/`](decisions/) (ADRs), [`docs/design/fase-4-tool-engine.md`](design/fase-4-tool-engine.md), [`plans/017-fase-7-permissoes-e-sandbox.md`](../plans/017-fase-7-permissoes-e-sandbox.md).
- **Gate:** qualquer mudança só está pronta com `bun run verify` passando (exit 0).

## O que já está pronto (Fases 0–7)

Tudo em `main` até a Fase 6; Fase 7 nesta PR. Gate: `bun run verify`.

- **Núcleo Rust** ([`crates/agent-core/`](../crates/agent-core/)): workspace, Ollama, Tool Engine, loop, eval, e agora permissões/sandbox:
  - `permissions.rs` — classificação + **tabela ASK/AUTO/FULL ACCESS** (`policy`, função pura). FULL ACCESS sem sandbox completo rebaixa para ASK.
  - `sandbox.rs` — **Linux:** Landlock (FS) + user/net namespace (rede). Rede só é liberada para comando `network` **aprovado**. **Windows/macOS:** indisponível, documentado, sem fingir isolamento.
  - `tools/command.rs` aplica o sandbox em todo spawn. `edit_file`/`write_file` seguem a policy (AUTO/FULL ACCESS não perguntam edição).
  - Todo resultado de tool no contexto vai entre `--- begin/end untrusted tool result ---` (SPEC §20.5).
- **Settings:** `permissionMode` em `settings.json` (default ASK).
- **CLI:** `cd-ai task --mode ask|auto|full-access`. FULL ACCESS recusado se o probe não estiver completo.
- **UI:** seletor de modo no composer; Acesso total desabilitado sem sandbox. IPC `sandbox_status` / `set_permission_mode`.
- **Eval:** continua ASK + responder que concede, sobre a cópia da fixture. Os comandos **ainda passam pelo sandbox**. Baseline scripted 3/3.

## Fase 7 — aceite

- Policy: testes da tabela em `permissions.rs`.
- Sandbox Linux: TCP a um listener local falha sem `allow_network` e passa com; escrita no workspace passa; escrita em `$HOME` (fora de cache) falha.
- Eval scripted: `cargo run -q -p cd-ai-cli -- eval --scripted` → 3/3.
- Detalhe e gap Windows/macOS: [`docs/audit/fase-7-sandbox.md`](audit/fase-7-sandbox.md).

## O que ainda falta (Fases 8–14)

| Fase | O que falta |
|---|---|
| 8 — Verifier e ciclo de correção | Validação determinística + review por LLM. O loop **nunca** produz `completed` (D11). |
| 9 — Checkpoints/Git/histórico | Shadow repo, rollback, tools de git, histórico por workspace. |
| 10 — Context Manager | Repo map (tree-sitter), orçamento por seção, cache, Explorer role. Hoje: perfil + corte explícito (`ContextExhausted`) + marcação de não confiável. |
| 11 — Skills | Registry, licenças, Skill Router. |
| 12 — Model Router/memória | Roteamento FAST/CODER/REASONER, memória, trajetórias. |
| 13 — Otimização | Medir primeiro, otimizar com base em métrica. |
| 14 — Release | Empacotamento `.deb`/AppImage e demais formatos. Sandbox Windows/macOS se algum dia for alvo de release. |

Detalhes no `SPEC.md` §34. Allowlist de comandos por workspace (SPEC §30) ficou de fora da Fase 7 de propósito (D10 do 017).

## Próximo passo recomendado

**Fase 8 (verifier e ciclo de correção).** A taxa ao vivo com `qwen3-coder:30b` ainda pode ser registrada numa máquina com Ollama; não bloqueia a 8.

## Pontos de atenção pro próximo executor

- **Código e comentários em inglês; UI e documentação em pt-BR.**
- Fronteira de confiança é o Rust: path, permissões, sandbox, execução e secrets nunca caem no frontend.
- FULL ACCESS **não** é um interruptor na webview: `set_permission_mode` recusa se `sandbox::status().available` for falso.
- Quem gerar tipos novos no core roda `cargo test -p agent-core` (o `ts-rs` exporta em `apps/desktop/src/lib/bindings/`); campo `u64` vira `bigint` a menos que tenha `#[ts(type = "number")]`.
- Um `ToolEngine` por tarefa. `run_tool` / `runTool` / `task_id: "root"` não existem mais (D4).
- O gate depende de `cargo` (edition 2024, Rust 1.85+) e `bun`. Testes de sandbox Linux precisam de Landlock (ABI ≥ 1) e, para bloqueio total de rede, user namespace.
- `keep_alive: -1` está no corpo do `/api/chat`.
- Compostos continuam recusados (`CompoundCommand`) — o sandbox não liga shell.
