# Plano 018: Fase 8 — Verifier e ciclo de correção

> **Instruções ao executor:** rode os passos nesta ordem e confirme cada verificação antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Uma parte só está pronta com `bun run verify` saindo com exit 0 (AGENTS.md).
>
> **Drift check (rode primeiro):** `git log -1 --oneline` deve estar em `main` com as Fases 0–7 (PR #4). Confira `crates/agent-core/src/agent/runner.rs` (`status_for` / D11), `crates/agent-core/src/permissions.rs` (`CommandClass::Validate`), `crates/agent-core/src/agent/profile.rs` e SPEC §13 / §23 / §34 Fase 8. Se o loop ou a classificação tiverem sumido, trate como STOP.

## Status

- **Prioridade:** P1
- **Esforço:** M
- **Risco:** MEDIUM (status `completed` errado mascara falta de evidência; o ciclo de correção pode inflar iterações)
- **Depende de:** 017 (Fase 7 no `main`)
- **Categoria:** feature (SPEC §13, §23, §34 Fase 8)
- **Planejado em:** commit `643171a` (`main`), 2026-09-16

## Por que isso importa

Até a Fase 7 o loop **nunca** produz `completed` (plano 015 D11): qualquer turno sem tools vira `completed_unvalidated` com `validated: false`, mesmo quando o modelo rodou `bun test` e passou. A afirmação "terminei" do modelo não é evidência (SPEC §2). Sem o Verifier, o eval mede só o check *depois* do agente; o agente não se corrige quando os testes falham.

A Fase 8 fecha SPEC §13: o sistema executa a verificação, devolve falha ao modelo até um limite, e só então marca `completed` quando há evidência real.

## Estado atual (fatos)

- `classify` já marca `cargo test` / `bun test` / scripts `test|lint|typecheck|check|build|verify` como `validate` (auto em ASK/AUTO/FULL ACCESS).
- O perfil do workspace lista comandos de validação (strings), mas o loop **não os executa** ao encerrar.
- `parse_check` (tree-sitter TS/Rust) já roda em `edit_file`/`write_file`; o Verifier não relê o disco no fim.
- `TaskStatus` não tem `completed`. `StopReason::Finished` mapeia só para `completed_unvalidated`.
- Eval scripted 3/3; sucesso = exit do check, não o status do agente (016 D3). Os três scripts já editam e rodam `bun test`.

## Decisões deste plano (não reabrir sem ADR)

| # | Decisão | Motivo |
|---|---|---|
| D1 | **Verifier é módulo em `agent-core`** (`agent/verify.rs`), chamado pelo loop. Sem crate novo, sem role separado no runtime. | Decisão 0003. SPEC §12.3: o Verifier julga por evidência; o Coder continua sendo o único modelo com tools. |
| D2 | **`completed` só com evidência:** arquivos alterados **e** (parse+diff ok) **e** pelo menos um comando classe `validate` com exit 0 **depois da última edição**. Caso contrário `completed_unvalidated`. | SPEC §2 e §23. Nunca mascarar falta de evidência. Q&A / leitura → unvalidated. |
| D3 | **O Verifier executa.** Não confia na frase do modelo. Reusa um `validate` já gravado em `state.commands` se foi exit 0 *depois* da última `FileChanged`; senão dispara o argv detectado via `ToolEngine` (mesmo sandbox/permissões). | SPEC §13.1 "executa e checa". Evita `bun test` duas vezes no caminho feliz. |
| D4 | **Detecção = perfil do workspace em argv**, barato primeiro (typecheck/lint → test → build/verify). Sem lockfile, se o corpo do script começa com `bun`, o runner é `bun` (não `npm`). | SPEC §13.1. Os fixtures de eval não têm lockfile e o script é `bun test`; `npm run test` seria um falso Fail. |
| D5 | **Parse tree-sitter no fim** (reusa `parse_check` de `edit.rs`). Extensão fora de TS/JS/Rust: pula, não é erro. | Já é dependência da Fase 4 (D3 do 014). Sem crate novo. |
| D6 | **Diff checks determinísticos, sem git:** secrets tocados (`detect_path_secret`); lockfile/gerado alterado; `TODO`/`FIXME`/`debugger` em linhas adicionadas do diff; diff > 4000 linhas. Sem shadow repo (Fase 9) não há "fora do plano". | SPEC §13.1 item 2, o que dá para fazer sem git. |
| D7 | **Ciclo de correção:** Fail → mensagem de usuário com as razões (até `max_correction_retries`, default 3) → o loop continua. No limite: para em `completed_unvalidated` e reporta o que falhou. **Não** vira `failed`. | SPEC §13.3: parar e reportar o estado real. |
| D8 | **Review por LLM opcional, depois do PASS determinístico.** Conversa isolada (pedido + diff + checks; sem tools, sem transcript do Coder). `PASS` ou `CHANGES_REQUIRED`. Falha determinística sempre ganha. Resposta vazia/ilegível = skip (trata como PASS). | SPEC §13.2. `ScriptedModel::once` esgota com texto vazio: o eval scripted não quebra. |
| D9 | **`StopReason::Verified`** → `TaskStatus::Completed` e `report.validated = true`. `Finished` continua unvalidated. Sem status `verifying`/`reviewing`/`fixing` nesta fase: o loop permanece `running`; a UI já mostra a fase `validate` no `commandStarted`. | Menos superfície de IPC. §23 lista esses estados; o critério de saída da Fase 8 não os exige no enum. |
| D10 | **Eval não cresce a suíte.** Os 3 scripts passam a terminar `completed` (validado). Check 3/3 não pode regressar. Testes novos no runner cobrem `completed` vs `completed_unvalidated` vs correção. | 016 D3/D4. Melhora mensurável no *outcome* do agente; a taxa de check fica estável. |
| D11 | **tree-sitter no Verifier, não no Context Manager.** Repo map continua Fase 10. | Escopo. |

## Escopo

**Dentro:** módulo `verify`; `TaskStatus::Completed` + `StopReason::Verified`; detecção argv; parse+diff+comandos; ciclo de correção; review LLM opcional; prompt curto; CLI/UI labels; testes; docs/handoff; eval scripted sem regressão.

**Fora:** Fase 9+ (shadow git, context manager, skills, router, release). Status `verifying`/`reviewing`/`fixing`. Allowlist. `--yes` no `task`. Suíte de eval maior. Payloads ofensivos.

## Passos

1. **Estado:** `TaskStatus::Completed`; `StopReason::Verified`; `AgentLimits.max_correction_retries` (3) e `llm_review` (true). Bindings via `cargo test`.
2. **`profile.rs`:** `validation_argv(workspace) -> Vec<Vec<String>>` (split + ordem de custo). Runner bun sem lockfile quando o script invoca bun.
3. **`edit.rs`:** `parse_check` vira `pub(crate)`.
4. **`agent/verify.rs`:** veredito `Pass` / `Fail` / `Unvalidated`; checks de arquivo; `parse_review`; texto de correção e prompt de review. Testes unitários sem spawn de modelo.
5. **`runner.rs`:** no turno sem tools, chamar o Verifier. Reusar validate pós-edição ou executar via engine (eventos entram na evidência). Fail → user message até o limite. Pass → review opcional isolada (sem tokens na conversa). `status_for` / `TaskReport.validated`.
6. **Prompt:** uma frase: o sistema verifica por evidência depois do turno sem tools.
7. **CLI/eval/UI:** label `concluída`; exit 0 em `Completed`; `status_slug`; `describeStopReason` para `verified` (silencioso, como `finished`); teste de atividade com `validated: true`.
8. **Docs:** handoff (Fase 8 feita, próxima 9), README dos planos, `docs/audit/fase-8-verifier.md` com taxa scripted e o comando ao vivo.
9. **Gate:** `bun run verify` exit 0. `cargo run -q -p cd-ai-cli -- eval --scripted` 3/3, com `agentStatus: completed` nas três.

## Contratos

Veredito (interno, não IPC):

```text
Pass                         → Completed + Verified + validated true
Fail + retries remaining     → user message, loop continua
Fail + retries exhausted     → CompletedUnvalidated + Finished + validated false
Unvalidated { why }          → CompletedUnvalidated + Finished + validated false
```

Review LLM (texto isolado): primeira linha `PASS` ou `CHANGES_REQUIRED`. Qualquer outra coisa = skip.

## Critérios de pronto

- [x] `plans/018-fase-8-verifier-e-ciclo-de-correcao.md` existe; linha 018 em `plans/README.md` = DONE
- [x] Loop produz `completed` quando parse+diff passam e há `validate` exit 0 após a última edição
- [x] Sem arquivos alterados, ou sem comando de validação no workspace → `completed_unvalidated`
- [x] Falha de verificação reentra no loop até o limite; no limite, unvalidated com as razões
- [x] Review LLM não bloqueia o eval scripted (skip em resposta vazia)
- [x] Falha determinística prevalece sobre `PASS` do LLM
- [x] `cd-ai eval --scripted` 3/3 e as tarefas scripted saem `completed`
- [x] `bun run verify` exit 0
- [x] PR aberto

## STOP conditions

- `run_command` deixou de ser argv-only → reporte, não ligue shell para "facilitar" o Verifier.
- Eval scripted caiu de 3/3 porque o Verifier rodou `npm run test` sem npm → ajuste a detecção (D4), não desligue o Verifier.
- Pedido de PoC/exploit para "provar" checks → recuse.
- Ollama ausente → não é STOP: cubra com `ScriptedModel` e documente o comando ao vivo.

## Notas de manutenção

- Código e comentários em inglês; UI e docs em pt-BR.
- O frontend não decide se validou: só mostra `report.validated` e o status que o Rust emitiu.
- Comandos que o Verifier dispara passam pelo `ToolEngine` (sandbox, classificação, redator).
- `max_correction_retries` conta falhas de verificação, não retries de transporte do modelo (`state.retries`).
