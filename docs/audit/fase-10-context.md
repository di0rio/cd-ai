# Fase 10 — Context Manager completo (auditoria)

Medido em 2026-09-16. Plano: [`plans/020-fase-10-context-manager.md`](../../plans/020-fase-10-context-manager.md).

## O que o core cobre de verdade

| Garantia | Como | Teste |
|---|---|---|
| Repo map tree-sitter | Walk `ignore` + grammar TS/Rust (`syntax.rs`); path + assinatura | `typescript_function_lands_in_the_map_and_scores_against_the_request`, `rust_struct_and_fn_are_indexed` |
| Prioridade pelo pedido | Score de tokens do request em path/símbolo | o mesmo; `alpha` cabe num orçamento apertado |
| Cache com invalidação | `<data_dir>/cache/<sha256(root)>/repo-map.json`; mtime+tamanho | `cache_reuses_unchanged_files_and_reparses_after_an_edit` |
| Secrets / gitignore | `detect_path_secret` + filtros padrão do `ignore` | `gitignored_and_secret_files_are_not_opened` |
| Orçamento por seção | Role 12% / regras 8% / map 15% / resposta 20% de `num_ctx` | `section_budget_cuts_an_oversized_map` |
| Higiene | `read_file` obsoleto após edit; log de comando compactado | `hygiene_drops_a_stale_read_after_an_edit`, `hygiene_compacts_a_huge_command_log` |
| Compactação (0008) | Resumo com fatos do engine; tools grandes viram stub | `compaction_replaces_the_middle_and_keeps_the_request`, `compaction_lets_a_bloated_conversation_continue` |
| `ContextExhausted` | Só se o pedido sozinho não cabe (nunca cortado) | `an_oversized_user_request_still_exhausts_the_window` |
| Explorer read-only | Tools filtradas + recusa no loop | `explorer_refuses_to_edit_and_a_question_never_gets_write_tools` |
| Fixtures de eval = Coder | Classificação `trivial` (≤ 12 fontes + verbo de edição) | `eval_style_requests_are_trivial_on_a_small_repo` |

## Tokens

O `ScriptedModel` continua reportando `promptTokens: 0`. A métrica honesta desta fase é `estimatedPromptTokens`: soma de `chars/4` em cada turno, gravada em `TaskMetrics` e no JSON do eval.

A redução frente ao corte cego da Fase 5 está nos testes unitários:

- higiene de log e de leitura obsoleta encolhe o que o modelo vê;
- orçamento por seção impede o map de comer a janela;
- compactação substitui o meio da conversa por um resumo curto e deixa a tarefa **continuar** (`Finished`) onde antes ela parava em `ContextExhausted`.

Numa tarefa scripted mínima (3–4 turnos, fixture de 2 arquivos) o map **adiciona** umas centenas de caracteres ao system prompt — o ganho aparece em repos maiores e em conversas longas, que é o caso da §16.

## Eval

```bash
cargo run -q -p cd-ai-cli -- eval --scripted
```

A suíte não cresceu (planos 016 D3 / 018 D10 / 019 D10 / 020 D9). Taxa scripted medida nesta fase: **100% (3/3)** — sem regressão. Cada linha do relatório CLI mostra `~N tok` (estimado).

## Caminho ao vivo (Ollama + CODER)

Nesta VM de cloud **não há Ollama**. A taxa ao vivo e a comparação de `promptTokens` reais ficam bloqueadas até rodar, numa máquina com o Ollama 0.34+ e `qwen3-coder:30b` puxado (decisão 0002):

```bash
ollama show qwen3-coder:30b
cargo run -q -p cd-ai-cli -- eval --model qwen3-coder:30b --out evals/results/qwen3-coder_30b-fase10.json
```

Comparar `promptTokens` / `estimatedPromptTokens` com o JSON da Fase 9 (`evals/results/`). A expectativa: menos tokens por tarefa em média, taxa de check ≥ 3/3.

## Fora desta fase

Skills (Fase 11), Model Router (Fase 12), watcher de filesystem, tool `search_symbols`, suíte de eval maior.
