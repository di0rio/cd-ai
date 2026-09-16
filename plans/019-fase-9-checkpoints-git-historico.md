# Plano 019: Fase 9 — Checkpoints, Git e histórico

> **Instruções ao executor:** rode os passos nesta ordem e confirme cada verificação antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Uma parte só está pronta com `bun run verify` saindo com exit 0 (AGENTS.md).
>
> **Drift check (rode primeiro):** `git log -1 --oneline` deve estar em `main` com as Fases 0–8 (PR #5). Confira `crates/agent-core/src/tools/edit.rs` (hash SHA-256 + `CheckpointCreated` por arquivo), `crates/agent-core/src/agent/storage.rs` (`TaskStore` em `data_dir/tasks`), `crates/agent-core/src/permissions.rs` (`classify_git`) e SPEC §19 / §21 / §24.1 / §34 Fase 9. Se o store ou o hash de edição tiverem sumido, trate como STOP.

## Status

- **Prioridade:** P1
- **Esforço:** L
- **Risco:** HIGH (rollback que apaga trabalho do usuário é irrecuperável; o shadow git não pode tocar no `.git` do projeto)
- **Depende de:** 018 (Fase 8 no `main`)
- **Categoria:** feature (SPEC §19, §21, §24.1, §34 Fase 9)
- **Planejado em:** commit `a2efe4b` (`main`), 2026-09-16

## Por que isso importa

Até a Fase 8 o agente registra o SHA-256 de cada arquivo que escreve e a UI lista tarefas do workspace. Não há snapshot do disco, não há como desfazer uma tarefa sem arriscar o que o usuário mexeu no meio, e o modelo só vê git se inventar `run_command` — inclusive variantes destrutivas/remotos.

A Fase 9 fecha SPEC §21 e a saída da §34: shadow repo fora do projeto, rollback que protege mudanças do usuário, tools de git só de leitura, histórico por workspace.

## Estado atual (fatos)

- `edit_file`/`write_file` emitem `FileChanged` + `CheckpointCreated { hash }` com SHA-256 do conteúdo. `TaskState.files_changed` guarda `path` + `hash_after`. Não há commit, não há git dir, não há restore.
- `TaskStore` persiste `state.json` / `transcript.jsonl` / `events.jsonl` em `<data_dir>/tasks/<id>`. `list(workspace)` já filtra por workspace. Não há entrada de histórico rica nem `cd-ai history`.
- `classify_git` existe; `git status|diff|log|branch` são `read`. Não há tools `git_*`. `run_command` ainda pode receber `git reset --hard` (destructive, pergunta).
- Evento `rollback.completed` não existe. A UI não tem botão de reverter.
- Eval scripted 3/3; a suíte não cresce (016 D3 / 018 D10).

## Decisões deste plano (não reabrir sem ADR)

| # | Decisão | Motivo |
|---|---|---|
| D1 | **Shadow repo é módulo em `agent-core`** (`checkpoint.rs`). Git dir em `<data_dir>/shadow/<sha256 do root canônico>/`. Work tree = workspace. Sem crate novo, sem `git2`. | Decisão 0003. SPEC §21: git dir fora do projeto. Binário `git` já é o que o usuário tem; sem dependência nova. |
| D2 | **Nunca `GIT_DIR` / `--separate-git-dir` no workspace.** Toda invocação passa `--git-dir` + `--work-tree`. `info/exclude` contém `.git/` (e dirs gordos típicos). O `.git` do usuário não é lido nem escrito. | SPEC §21. `--separate-git-dir` criaria um ficheiro `.git` no projeto. |
| D3 | **Checkpoint de baseline no início de cada tarefa** (`git add -A` + commit `--allow-empty`). Depois de cada `edit_file`/`write_file` bem-sucedido: `git add -f -- <path>` + commit. Antes de `run_command` classe `destructive`: snapshot completo. | SPEC: "antes de cada tarefa que edita arquivos e antes de operações destrutivas". `-f` garante que um ficheiro gitignored que o agente tocou entra no snapshot. |
| D4 | **Rollback de uma tarefa = só os paths em `files_changed`.** Comparar o conteúdo atual com `hash_after` (ainda é a escrita do agente) vs o blob do baseline. Igual ao baseline → já limpo. Igual a `hash_after` → restaurar. Qualquer outra coisa → conflito: não tocar, devolver diff. `--force` / segundo gesto na UI sobrescreve conflitos, nunca ficheiros que o agente não escreveu. | SPEC §21. Trabalho não relacionado (o utilizador editou B enquanto o agente editava A) fica. |
| D5 | **Rollback não é tool do modelo.** É comando do utilizador (CLI `cd-ai rollback`, IPC `rollback_task`). Recusa tarefa `running`/`waiting_approval`. Grava `RollbackCompleted` no `events.jsonl` da tarefa. | O modelo não desfaz o próprio trabalho. A fronteira de confiança continua no Rust. |
| D6 | **Tools `git_status` / `git_diff` / `git_log` / `git_branch` são só leitura no git do utilizador**, argv fixo (`--no-ext-diff`, `--porcelain=v1`, `log -n` limitado). Sem flags livres do modelo. Auto em todos os modos (classe `read`). Sem git no workspace → erro claro, não `run_command`. | SPEC §15.2 e §19. Nunca force push, reset, remote. |
| D7 | **Commits automáticos no git do utilizador continuam desligados e fora desta fase.** | SPEC §19: "desligados por padrão". Shadow ≠ git do projeto. |
| D8 | **Histórico = o que o store já guarda, agora com superfície.** `TaskHistoryEntry` (resumo + ficheiros + comandos + checkpoint baseline + `rolled_back`). CLI `cd-ai history`; IPC `workspace_history`. A sidebar já lista por workspace — ganha o rollback no relatório. | SPEC §24.1. Sem RAG, sem memória estruturada (Fase 12). |
| D9 | **Identidade git do shadow é sempre `cd-ai` / `cd-ai@local`, `commit.gpgsign=false`, sem config global.** Falha de `git` no PATH: a tarefa continua (erro em `state.errors`); rollback sem baseline recusa com mensagem. | Um GPG do utilizador não pode bloquear snapshot. Falta de git não mata o loop. |
| D10 | **Eval não cresce.** Scripted 3/3 não pode regressar. Testes novos cobrem rollback com mudanças do utilizador e o git dir do utilizador intacto. | 016 D3. Saída da Fase 9 é teste, não taxa de eval. |

## Escopo

**Dentro:** `checkpoint.rs` (shadow + rollback); `TaskState.checkpoints` / `rolled_back`; tools git de leitura; eventos `checkpointCreated` (commit) e `rollbackCompleted`; CLI `history`/`rollback`; IPC + ACL + botão na UI; testes; docs/handoff.

**Fora:** Fase 10+ (context manager, skills, router, release). `delete_file`. Commits no git do utilizador. Status `verifying`/`reviewing`/`fixing`. Suíte de eval maior. Payloads ofensivos.

## Passos

1. **Estado:** `Checkpoint`, `CheckpointKind`, `RollbackResult` / `RollbackSkip`; campos novos em `TaskState` com `#[serde(default)]`. Bindings via `cargo test`.
2. **`TaskStore`:** guardar `data_dir` (pai de `tasks/`) para o shadow viver ao lado, nunca no workspace.
3. **`checkpoint.rs`:** init do git dir, exclude, snapshot completo e por path, `show` de blob, rollback D4, testes (workspace sem git, workspace com git do utilizador, conflito, não-relacionado, ficheiro criado pelo agente, `--force`).
4. **`tools/git.rs`:** quatro tools, argv fixo, sandbox como `read`, output estruturado + `render_outcome`.
5. **Loop:** baseline ao criar/retomar; `add -f` após edição; snapshot antes de destructive; emitir eventos.
6. **CLI:** `history`, `rollback [--force]`; USAGE e testes de parse.
7. **Tauri/UI:** `workspace_history`, `rollback_task`; ACL; botão no relatório; conflitos visíveis; segundo gesto = force. Demo (`?demo`) não chama IPC.
8. **Docs:** handoff (Fase 9 feita, próxima 10), README dos planos, `docs/audit/fase-9-checkpoints.md`.
9. **Gate:** `bun run verify` exit 0. `cargo run -q -p cd-ai-cli -- eval --scripted` 3/3.

## Contratos

Rollback (interno e IPC):

```text
já igual ao baseline          → already_clean (não escreve)
hash atual == hash_after      → restore do blob do baseline (ou apaga se o baseline não tem o path)
qualquer outro conteúdo       → skip + diff  (force → restore)
tarefa running / sem baseline → erro, disco intacto
paths fora de files_changed   → nunca tocados
```

Shadow:

```text
GIT_DIR  = <data_dir>/shadow/<sha256(root)>/
worktree = workspace canônico
exclude  = .git/ + node_modules/ target/ dist/ .next/ …
```

## Critérios de pronto

- [x] `plans/019-fase-9-checkpoints-git-historico.md` existe; linha 019 em `plans/README.md` = DONE
- [x] Shadow repo fora do workspace; o `.git` do utilizador não muda num rollback
- [x] Rollback restaura só o que o agente escreveu; mudanças do utilizador no mesmo ficheiro são protegidas (teste)
- [x] Mudanças do utilizador em ficheiros não relacionados sobrevivem (teste)
- [x] Tools `git_status` / `git_diff` / `git_log` / `git_branch` existem e recusam workspace sem git
- [x] `cd-ai history` lista tarefas do workspace; `cd-ai rollback <id>` reverte
- [x] UI: reverter no relatório, com segundo gesto para conflitos
- [x] `cd-ai eval --scripted` 3/3
- [x] `bun run verify` exit 0
- [x] PR aberto

## STOP conditions

- Pedido para implementar rollback com `git reset --hard` no repositório do utilizador → recuse; o shadow é o único restore.
- Pedido de PoC/exploit para "provar" o rollback → recuse.
- `git` ausente no PATH dos testes → não é STOP se os testes pularem com mensagem clara; documente. Neste ambiente `git` 2.43 está presente.
- Eval scripted caiu de 3/3 porque o snapshot atrasou o loop → o baseline é best-effort e não pode falhar a tarefa; ajuste, não desligue o Verifier.
- Ollama ausente → não é STOP.

## Notas de manutenção

- Código e comentários em inglês; UI e docs em pt-BR.
- O frontend não decide o que restaurar: manda `taskId` + `force`; o Rust compara hashes.
- Snapshot do shadow **não** passa pelo sandbox do `run_command` (o git dir está fora do workspace; o Landlock bloquearia o commit). É código confiável do core.
- Tools `git_*` **passam** pelo sandbox (leitura no workspace, rede bloqueada).
