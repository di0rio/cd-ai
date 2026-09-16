# Handoff — estado do cd-ai

Estado em 2026-09-16, sobre `main` (Fases 0–12, PR #12) mais o trabalho da **Fase 13**
(plano 023, esta PR). Serve pra uma sessão/agente novo continuar sem depender da conversa anterior.

- **Fontes da verdade:** [`AGENTS.md`](../AGENTS.md) (regras + comandos), [`SPEC.md`](../SPEC.md) (§15.3, §16.4, §27 e §34), [`plans/README.md`](../plans/README.md), [`docs/decisions/`](decisions/) (ADRs), [`plans/023-fase-13-otimizacao.md`](../plans/023-fase-13-otimizacao.md).
- **Gate:** qualquer mudança só está pronta com `bun run verify` passando (exit 0).

## O que já está pronto (Fases 0–13)

Tudo em `main` até a Fase 12; Fase 13 nesta PR. Gate: `bun run verify`.

- **Núcleo Rust** ([`crates/agent-core/`](../crates/agent-core/)): workspace, Ollama, Tool Engine, loop, eval, permissões/sandbox, Verifier, checkpoints, Context Manager, Skills, Model Router, e agora as otimizações medidas da Fase 13:
  - Redactor de entropia linear (histogram deslizante; skip se < 26 bytes distintos).
  - Um walk por `assemble_new`; router 8k reusa o mapa; refresh em memória até edit/write/comando.
  - Cache JSON do repo map não regrava se nada mudou; parse tree-sitter reusado / paralelo em misses.
  - `read_file` independentes do mesmo turno em paralelo (eventos na ordem; secret/escrita sequenciais).
  - Métricas `contextMs` / `cacheHits` / `cacheMisses`.
- **UI:** tokens coalescidos num frame; `Markdown` memorizado.
- **Eval:** scripted 3/3. JSON traz as métricas novas. Evidência em [`docs/audit/fase-13-otimizacao.md`](audit/fase-13-otimizacao.md).

## Fase 13 — aceite

- Mediu primeiro: o hotspot era Shannon O(n·W) no redactor (~2,14 s em 16 × 64 KiB), não o disco; `assemble_new` andava o workspace duas vezes (e de novo no cap 8k).
- Entropia: mesmos testes de detecção; 64 KiB de baixa cardinalidade < 50 ms.
- Contexto: um índice no start; `cacheHits` > 0 no eval scripted.
- Leituras: duas `read_file` no mesmo turno, ordem estável.
- Eval scripted: `cargo run -q -p cd-ai-cli -- eval --scripted` → 3/3.
- Detalhe: [`docs/audit/fase-13-otimizacao.md`](audit/fase-13-otimizacao.md).

## O que ainda falta (Fase 14)

| Fase | O que falta |
|---|---|
| 14 — Release | Empacotamento `.deb`/AppImage e demais formatos. Sandbox Windows/macOS se algum dia for alvo de release. |

Detalhes no `SPEC.md` §34. Commits automáticos no git do **usuário** continuam desligados (SPEC §19). Status `verifying`/`reviewing`/`fixing` (§23) ficaram de fora da Fase 8 de propósito. Marketplace / importação de skills do usuário ficaram de fora da Fase 11 de propósito. UI completa de categorias FAST/CODER/REASONER ficou de fora da Fase 12. Tokenizer por modelo e watcher de filesystem ficaram de fora da Fase 13 de propósito.

## Próximo passo recomendado

**Fase 14 (Release).** Empacotar `.deb` e AppImage. A taxa ao vivo com `qwen3-coder:30b` ainda pode ser registrada numa máquina com Ollama (comando no audit da Fase 13); não bloqueia a 14.

## Pontos de atenção pro próximo executor

- **Código e comentários em inglês; UI e documentação em pt-BR.**
- Fronteira de confiança é o Rust: path, permissões, sandbox, execução, secrets, Verifier, rollback, o que entra no prompt, **quais skills carregam** e **qual modelo a tarefa usa** nunca caem no frontend. I/O paralelo de leitura **não** fura isso: authorize continua no engine, redação no Rust.
- Tool results continuam marcados como não confiáveis. O resumo de compactação usa só estado do engine (`files_changed`, `commands`). Memória não é extraída do modelo.
- FULL ACCESS **não** é um interruptor na webview: `set_permission_mode` recusa se `sandbox::status().available` for falso.
- Quem gerar tipos novos no core roda `cargo test -p agent-core` (o `ts-rs` exporta em `apps/desktop/src/lib/bindings/`); campo `u64` vira `bigint` a menos que tenha `#[ts(type = "number")]`.
- Um `ToolEngine` por tarefa. `run_tool` / `runTool` / `task_id: "root"` não existem mais (D4).
- O gate depende de `cargo` (edition 2024, Rust 1.85+) e `bun`. Testes de sandbox Linux precisam de Landlock (ABI ≥ 1) e, para bloqueio total de rede, user namespace. O Verifier dispara `bun`/`cargo` via PATH. Checkpoints precisam de `git` no PATH.
- `keep_alive: -1` está no corpo do `/api/chat`.
- Compostos continuam recusados (`CompoundCommand`) — o sandbox não liga shell.
- Trajetórias e memória ficam no diretório de dados do app (`CD_AI_DATA_DIR`); nada sobe à rede (decisão 0004).
- O índice em memória do mapa **não** vê edição do usuário no meio da tarefa até o agente escrever ou rodar um comando. Sem watcher (020 D1 / 023).
