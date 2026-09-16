# Handoff — estado do cd-ai

Estado em 2026-09-16, sobre `main` (Fases 0–8, PR #5) mais o trabalho da **Fase 9**
(plano 019, esta PR). Serve pra uma sessão/agente novo continuar sem depender da conversa anterior.

- **Fontes da verdade:** [`AGENTS.md`](../AGENTS.md) (regras + comandos), [`SPEC.md`](../SPEC.md) (§19, §21, §24.1 e §34), [`plans/README.md`](../plans/README.md), [`docs/decisions/`](decisions/) (ADRs), [`plans/019-fase-9-checkpoints-git-historico.md`](../plans/019-fase-9-checkpoints-git-historico.md).
- **Gate:** qualquer mudança só está pronta com `bun run verify` passando (exit 0).

## O que já está pronto (Fases 0–9)

Tudo em `main` até a Fase 8; Fase 9 nesta PR. Gate: `bun run verify`.

- **Núcleo Rust** ([`crates/agent-core/`](../crates/agent-core/)): workspace, Ollama, Tool Engine, loop, eval, permissões/sandbox, Verifier, e agora checkpoints:
  - `checkpoint.rs` — shadow git em `<data_dir>/shadow/<sha256 do root>/`, work tree = workspace. Nunca toca no `.git` do usuário.
  - Baseline no início da tarefa; `git add -f` depois de cada edição; snapshot antes de `run_command` destructive.
  - Rollback só dos paths em `files_changed`. Hash atual == `hash_after` → restaura o baseline. Outra coisa → conflito + diff. `--force` sobrescreve conflitos, nunca arquivos que o agente não escreveu.
  - Tools `git_status` / `git_diff` / `git_log` / `git_branch` (leitura, argv fixo, git do usuário).
- **Sandbox / modos** da Fase 7 e **Verifier** da Fase 8 continuam.
- **Eval:** scripted 3/3. Sucesso do harness continua sendo o exit do check (016 D3).

## Fase 9 — aceite

- Shadow repo fora do workspace; o `.git` do usuário não muda num rollback.
- Rollback restaura só o que o agente escreveu; mudanças do usuário no mesmo arquivo e em arquivos não relacionados sobrevivem (testes em `checkpoint.rs`).
- `cd-ai history` lista o workspace; `cd-ai rollback <id>` reverte.
- UI: botão no relatório; segundo gesto (“Reverter mesmo assim”) para conflitos.
- Eval scripted: `cargo run -q -p cd-ai-cli -- eval --scripted` → 3/3.
- Detalhe: [`docs/audit/fase-9-checkpoints.md`](audit/fase-9-checkpoints.md).

## O que ainda falta (Fases 10–14)

| Fase | O que falta |
|---|---|
| 10 — Context Manager | Repo map (tree-sitter), orçamento por seção, cache, Explorer role. Hoje: perfil + corte explícito (`ContextExhausted`) + marcação de não confiável + Verifier. |
| 11 — Skills | Registry, licenças, Skill Router. |
| 12 — Model Router/memória | Roteamento FAST/CODER/REASONER, memória, trajetórias. |
| 13 — Otimização | Medir primeiro, otimizar com base em métrica. |
| 14 — Release | Empacotamento `.deb`/AppImage e demais formatos. Sandbox Windows/macOS se algum dia for alvo de release. |

Detalhes no `SPEC.md` §34. Commits automáticos no git do **usuário** continuam desligados (SPEC §19). Status `verifying`/`reviewing`/`fixing` (§23) ficaram de fora da Fase 8 de propósito.

## Próximo passo recomendado

**Fase 10 (Context Manager completo).** A taxa ao vivo com `qwen3-coder:30b` ainda pode ser registrada numa máquina com Ollama; não bloqueia a 10.

## Pontos de atenção pro próximo executor

- **Código e comentários em inglês; UI e documentação em pt-BR.**
- Fronteira de confiança é o Rust: path, permissões, sandbox, execução, secrets, Verifier e rollback nunca caem no frontend.
- O snapshot do shadow **não** passa pelo sandbox do `run_command` (o git dir está fora do workspace). As tools `git_*` **passam**.
- FULL ACCESS **não** é um interruptor na webview: `set_permission_mode` recusa se `sandbox::status().available` for falso.
- Quem gerar tipos novos no core roda `cargo test -p agent-core` (o `ts-rs` exporta em `apps/desktop/src/lib/bindings/`); campo `u64` vira `bigint` a menos que tenha `#[ts(type = "number")]`.
- Um `ToolEngine` por tarefa. `run_tool` / `runTool` / `task_id: "root"` não existem mais (D4).
- O gate depende de `cargo` (edition 2024, Rust 1.85+) e `bun`. Testes de sandbox Linux precisam de Landlock (ABI ≥ 1) e, para bloqueio total de rede, user namespace. O Verifier dispara `bun`/`cargo` via PATH. Checkpoints precisam de `git` no PATH.
- `keep_alive: -1` está no corpo do `/api/chat`.
- Compostos continuam recusados (`CompoundCommand`) — o sandbox não liga shell.
