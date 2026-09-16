# Fase 9 — Checkpoints, Git e histórico (auditoria)

Medido em 2026-09-16. Plano: [`plans/019-fase-9-checkpoints-git-historico.md`](../../plans/019-fase-9-checkpoints-git-historico.md).

## O que o core cobre de verdade

| Garantia | Como | Teste |
|---|---|---|
| Shadow git fora do projeto | `<data_dir>/shadow/<sha256(root)>/`; `--git-dir` + `--work-tree`; `info/exclude` tem `.git/` | `snapshot_does_not_create_a_dot_git_in_the_workspace` |
| Rollback só do agente | Restaura paths de `files_changed` cujo hash atual ainda é `hash_after` | `rollback_restores_agent_file_and_keeps_unrelated_user_edits` |
| Proteção de mudança do usuário | Hash ≠ `hash_after` → skip + diff; `--force` sobrescreve | `rollback_skips_a_file_the_user_changed_after_the_agent` |
| Arquivo criado pelo agente | Baseline sem o path → delete, a menos que o usuário tenha editado | `rollback_deletes_a_file_the_agent_created_unless_the_user_edited_it` |
| Git do usuário intacto | HEAD e `git log` iguais antes/depois do rollback | `rollback_does_not_touch_the_users_git` |
| Tools de git só leitura | argv fixo; workspace sem git → erro claro | `status_on_a_repo_and_error_without_one` |
| Histórico por workspace | `TaskStore::history`; CLI `cd-ai history`; IPC `workspace_history` | `history_includes_files_and_checkpoint` |

O snapshot **não** passa pelo sandbox do `run_command` (o git dir está fora do workspace; o Landlock bloquearia o commit). É código confiável do core. As tools `git_*` **passam** pelo sandbox, com rede bloqueada.

## Eval

```bash
cargo run -q -p cd-ai-cli -- eval --scripted
```

A suíte não cresceu (planos 016 D3 / 018 D10 / 019 D10). Taxa scripted medida em 2026-09-16: **100% (3/3)** — sem regressão. O baseline do shadow é best-effort: falha de `git` entra em `state.errors` e a tarefa continua.

## Como testar rollback à mão

```bash
# num workspace de teste, depois de uma tarefa que editou arquivos:
cargo run -q -p cd-ai-cli -- history --workspace <pasta>
cargo run -q -p cd-ai-cli -- rollback <id> --workspace <pasta>
# conflito (você editou o mesmo arquivo depois do agente):
cargo run -q -p cd-ai-cli -- rollback <id> --workspace <pasta> --force
```

Na UI: relatório da tarefa → “Reverter alterações desta tarefa”. Se houver conflitos, o diff aparece e “Reverter mesmo assim” chama `force`.

## Caminho ao vivo (Ollama + CODER)

Nesta VM de cloud **não há Ollama**. A taxa ao vivo fica bloqueada até rodar, numa máquina com o Ollama 0.34+ e `qwen3-coder:30b` puxado (decisão 0002):

```bash
ollama show qwen3-coder:30b
cargo run -q -p cd-ai-cli -- eval --model qwen3-coder:30b --out evals/results/qwen3-coder_30b-fase9.json
```

## Fora desta fase

Context Manager / repo map (Fase 10), commits automáticos no git do **usuário** (SPEC §19, desligados por padrão), `delete_file`, status `verifying`/`reviewing`/`fixing`, suíte de eval maior.
