# Plano 016: Fase 6 — Eval baseline (suite, runner headless, número registrado)

> **Instruções ao executor:** rode os passos nesta ordem e confirme cada verificação antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Uma parte só está pronta com `bun run verify` saindo com exit 0 (AGENTS.md).
>
> **Drift check (rode primeiro):** `git log -1 --oneline` deve estar em `main` (Fases 0–5 mergeadas). Confira `evals/fixtures/soma/`, `apps/cli/src/main.rs` (`cd-ai task`) e `crates/agent-core/src/agent/runner.rs` (`run_task`, `ScriptedModel`). Se o comando `task` ou o fixture tiverem sumido, trate como STOP.

## Status

- **Prioridade:** P1
- **Esforço:** M
- **Risco:** MEDIUM (o número baseline ao vivo depende do Ollama + CODER; o harness não)
- **Depende de:** 015 (Fase 5 no `main`)
- **Categoria:** feature (SPEC §34 Fase 6, §26)
- **Planejado em:** commit `024cc8e` (`main`), 2026-09-15

## Por que isso importa

A Fase 5 fecha o loop, mas a única prova de que o agente “resolve uma tarefa” é um aceite manual (e o de 2026-09-12 reprovou por timeout de aprovação). Sem eval, mudanças de prompt, parser ou tools não têm um número para comparar.

A Fase 6 não é a suíte de 20–50 tarefas do SPEC §26. É o **harness** e um **baseline pequeno**: cada tarefa com check automático e limite de tempo, runner na CLI sem UI, relatório com as métricas do §26, resultados gravados.

## Estado atual (fatos)

- `evals/` só tem `fixtures/soma/` (aceite da Fase 5). Não há `tasks/`, `results/` nem runner.
- `cd-ai task` é interativo: sem TTY a aprovação é **negada** (plano 015 D13). Não serve como runner de eval.
- O loop (`run_task`) já persiste iterações, retries, tokens, `model_ms`/`tool_ms`. Falhas de formato e edições rejeitadas existem como eventos, não como totais no relatório de eval.
- `ScriptedModel` é `pub(crate)` e `#[cfg(test)]`. O teste determinístico da soma no runner já passa.
- Ollama + `qwen3-coder:30b` (decisão 0002) **pode não existir** neste ambiente. O harness precisa de um caminho seco.

## Decisões deste plano (não reabrir sem ADR)

| # | Decisão | Motivo |
|---|---|---|
| D1 | **`cd-ai eval` auto-aprova** no workspace que ele mesmo copiou. `cd-ai task` continua sem `--yes`. | SPEC §26 é headless. O fixture copiado é descartável, nunca o projeto do usuário. |
| D2 | **`evals/fixtures/` permanece** (não renomear para `repos/`). `evals/tasks/` são as tarefas; `evals/results/` os relatórios. | Fase 5 já publicou `fixtures/soma/`. A árvore do SPEC §7.1 (`repos/`) é o papel, não o nome no disco. |
| D3 | **Sucesso = exit do check**, não o status do agente. | O modelo pode encerrar em `completed_unvalidated` com o bug intacto. |
| D4 | **Suite enxuta (3 tarefas).** 20–50 vem depois, quando o harness existir. | SPEC §34 pede baseline, não a suíte final. |
| D5 | **`--scripted` usa o campo `script` da tarefa** via `ScriptedModel` (passa a ser API pública do core). | Dry-run sem Ollama; os testes do harness não dependem de rede. |
| D6 | **Módulo `eval` no `agent-core`.** A CLI só adapta args, imprime e grava. | Fronteira de confiança é o Rust. Sem crate novo. |
| D7 | **Eval usa `TaskStore` temporário**, nunca o diretório de dados do usuário. | Isolamento. `CD_AI_DATA_DIR` nos testes. |
| D8 | **Tarefas em série.** | Decisão 0002: um modelo grande residente. |
| D9 | **Timeout padrão da tarefa no eval: 10 min** (`timeoutMs` no JSON). Check: 60 s. | 45 min do loop é orçamento de produto, não de eval. |
| D10 | **Relatórios em `evals/results/`.** Epêmeros no gitignore; `*-baseline.json` pode ser commitado. O número oficial vive em `docs/audit/fase-6-baseline.md`. | SPEC §26: comparar versões. |

## Escopo

**Dentro:** `evals/tasks`, duas fixtures além de `soma`, módulo `eval`, `cd-ai eval`, relatório §26, baseline scripted medido, nota de auditoria com o comando ao vivo, docs/planos.

**Fora:** Fase 7+ (sandbox, verifier, shadow git, context manager, skills, router, release). Sem `--yes` no `task`. Sem suíte grande. Sem UI de eval.

## Passos

1. **`ScriptedModel` público** em `agent/model.rs`: tira `#[cfg(test)]`/`pub(crate)`. `new` continua repetindo a última resposta (testes do runner). `once` esgota o script e devolve texto sem tools, para o eval não entrar em loop.
2. **Módulo `crates/agent-core/src/eval.rs`:**
   - Lê `evals/tasks/*.json` (camelCase, std + serde_json).
   - Copia a fixture para um tempdir (pula `node_modules`, `target`, `.git`, symlinks).
   - Roda `run_task` com responder que concede tudo.
   - Roda o check (`ToolEngine` + `run_command`, timeout do JSON) **depois** do agente.
   - Conta, a partir dos eventos: falhas de formato (pedido inválido, sem `ToolStarted`) e edições rejeitadas (`ApprovalDenied` em edit/write).
   - Conta turnos nativos vs. texto (wrapper do `ChatModel`; decisão 0002).
   - Monta `EvalReport` (taxa, por tarefa: iterações, tokens, tempo, retries, falhas de formato, edições rejeitadas).
   - Grava JSON.
3. **Suite:**
   - `soma` — bug `a - b` (já existe).
   - `greet` — função devolve o nome; o teste espera `Olá, {nome}`.
   - `dobro` — o teste importa um arquivo que não existe; o agente precisa criar.
   Cada JSON tem `prompt`, `fixture`, `timeoutMs`, `check.argv`, `script`.
4. **CLI:** `cd-ai eval --model <nome> [--suite <pasta>] [--task <id>] [--out <arquivo>] [--ctx <n>]` e `cd-ai eval --scripted […]`. Sem `--model` e sem `--scripted` → exit 2. Ctrl+C cancela. Códigos: 0 = 100% no check, 1 = alguma falhou ou erro de execução, 2 = uso, 130 = cancelado.
5. **Testes (sem Ollama):** parse da suite; soma scripted passa o check; soma scripted que não edita falha o check; `--scripted --task soma` na CLI; help cita `eval`.
6. **Docs:** `evals/README.md`, `docs/audit/fase-6-baseline.md`, `AGENTS.md`, `README.md`, `docs/handoff.md`, gitignore, `plans/README.md`.
7. **Baseline:** `cargo run -q -p cd-ai-cli -- eval --scripted --out evals/results/scripted-baseline.json`. Se Ollama+CODER existir, rode também o vivo e registre a taxa. Senão, deixe o comando exato na nota de auditoria.
8. **Gate:** `bun run verify` exit 0. Linha 016 → DONE.

## Contratos

Tarefa (`evals/tasks/<id>.json`):

```json
{
  "id": "soma",
  "fixture": "soma",
  "prompt": "O teste de soma falha. Corrija e rode os testes.",
  "timeoutMs": 600000,
  "maxIterations": 15,
  "check": { "argv": ["bun", "test"], "expectExit": 0, "timeoutMs": 60000 },
  "script": [
    { "calls": [{ "name": "read_file", "arguments": { "path": "src/soma.ts" } }] },
    { "calls": [{ "name": "edit_file", "arguments": { "path": "src/soma.ts", "old_text": "a - b", "new_text": "a + b" } }] },
    { "calls": [{ "name": "run_command", "arguments": { "argv": ["bun", "test"] } }] },
    { "text": "Corrigido." }
  ]
}
```

Relatório (campos do SPEC §26): `successRate`, por tarefa `success`, `iterations`, `promptTokens`, `genTokens`, `durationMs`, `retries`, `toolCallFormatFailures`, `rejectedEdits`; extras úteis: `nativeToolCallTurns`, `textToolCallTurns`, `stopReason`, `checkExitCode`.

## Critérios de pronto

- [x] `plans/016-fase-6-eval-baseline.md` existe; linha 016 em `plans/README.md` = DONE
- [x] `cd-ai eval --scripted` roda a suite e grava JSON
- [x] cada tarefa tem check automático e timeout
- [x] relatório traz as métricas do §26
- [x] taxa baseline **scripted** registrada; taxa **ao vivo** registrada **ou** bloqueada com o comando exato (CODER da decisão 0002)
- [x] `bun run verify` exit 0
- [x] PR aberto

## STOP conditions

- `run_task` deixou de ser a API do loop → reporte, não reimplemente o agente no eval.
- Fixture `soma` mudou de forma que o script quebraria o aceite da Fase 5 → não “consertar” o fixture para o eval; ajuste só o `script`/check.
- Check precisa de rede ou de um binário que o `verify` não tem → escolha outro check.

## Notas de manutenção

- Código e comentários em inglês; UI e docs em pt-BR.
- O eval **não** entra no `bun run verify` como suíte ao vivo. Os testes do harness (scripted) sim, via `cargo test --workspace`.
- Ampliar a suíte é acrescentar JSON + fixture. O runner não muda.
