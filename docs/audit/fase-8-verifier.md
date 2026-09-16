# Fase 8 — Verifier e ciclo de correção (auditoria)

Medido em 2026-09-16. Plano: [`plans/018-fase-8-verifier-e-ciclo-de-correcao.md`](../../plans/018-fase-8-verifier-e-ciclo-de-correcao.md).

## O que o core cobre de verdade

| Garantia | Como | Teste |
|---|---|---|
| `completed` só com evidência | Arquivos alterados + parse/diff ok + `validate` exit 0 depois da última edição | `soma_fixture_is_fixed_and_its_tests_run` |
| Sem evidência → `completed_unvalidated` | Nada alterado, ou workspace sem comando de validação, ou retries esgotados | `a_reply_without_tools_finishes_unvalidated`, `exhausted_corrections_finish_unvalidated` |
| O Verifier **executa** | Se o modelo esquece o teste, o loop dispara o argv do perfil (`bun run test` nos fixtures) | `a_correct_edit_is_completed_even_when_the_model_skips_tests` |
| Ciclo de correção | Fail → user message até `max_correction_retries` (default 3) | `a_failed_check_is_sent_back_until_the_model_fixes_it` |
| Review LLM opcional | Conversa isolada; `CHANGES_REQUIRED` reentra; vazio/ilegível = skip | `llm_review_can_send_the_model_back`, `parse_review_reads_pass_and_changes` |
| Falha determinística ganha | Parse/diff/comando falho nunca vira `completed` por um `PASS` do LLM | `broken_typescript_fails_parse`, `todo_introduced_in_the_diff_fails` |
| Detecção de argv | Perfil do workspace, barato primeiro; sem lockfile, script `bun …` usa bun | `validation_argv_is_cheapest_first`, `without_a_lockfile_a_bun_script_uses_bun` |

Comandos que o Verifier dispara passam pelo `ToolEngine` (classificação, sandbox, redator). Sem shell.

## Eval

```bash
cargo run -q -p cd-ai-cli -- eval --scripted
```

As três tarefas scripted (soma, greet, dobro) editam, rodam `bun test` e agora terminam `completed` / `stopReason.kind = verified`. O sucesso do harness continua sendo o **exit do check** (plano 016 D3), não o status do agente. Taxa scripted: **100% (3/3)** — sem regressão, com completions validadas. Medido em `b1ef74a`.

Gate `bun run verify`: exit 0 no mesmo commit (biome, typecheck, testes da UI, `cargo fmt`, clippy `-D warnings` no workspace, `cargo test --workspace` incluindo `cd-ai-desktop`).

## Caminho ao vivo (Ollama + CODER)

Nesta VM de cloud **não há Ollama**. A taxa ao vivo fica bloqueada até rodar, numa máquina com o Ollama 0.34+ e `qwen3-coder:30b` puxado (decisão 0002):

```bash
ollama show qwen3-coder:30b
cargo run -q -p cd-ai-cli -- eval --model qwen3-coder:30b --out evals/results/qwen3-coder_30b-fase8.json
```

O eval copia cada fixture para um tempdir e **aprova sozinho**. Não use `cd-ai task` para isto. Timeout padrão por tarefa: 10 min.

O ganho esperado ao vivo, se houver, vem do ciclo de correção: um modelo que esquece o teste ou erra a primeira edição recebe a falha de volta em vez de encerrar `completed_unvalidated` com o bug intacto.

## Fora desta fase

Shadow git (Fase 9), repo map tree-sitter (Fase 10), status `verifying`/`reviewing`/`fixing` no enum (D9), suíte de eval maior.
