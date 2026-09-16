# Fase 10 — Context Manager completo (auditoria)

Medido em 2026-09-16. Plano: [`plans/020-fase-10-context-manager.md`](../../plans/020-fase-10-context-manager.md).

## O que o core cobre de verdade

| Garantia | Como | Teste |
|---|---|---|
| Repo map tree-sitter | Walk `ignore` + AST TS/JS/Rust; assinatura = 1ª linha; pula `node_modules`/`target`/secrets | `extracts_rust_and_typescript_symbols`, `skips_node_modules_and_secrets` |
| Prioridade pela tarefa | Score por overlap de path/símbolo com o pedido | `ranks_the_file_named_in_the_request` |
| Mapa ≪ dump das fontes | Render só com símbolos, não o corpo | `the_map_is_much_smaller_than_dumping_sources` |
| Orçamento por seção | Frações de `num_ctx`; corte `[section truncated]`; role (regras) não encolhe | `assemble_keeps_sections_inside_budget`, `cut_section_marks_the_overflow` |
| Cache mtime/size | `<data_dir>/context/<sha256(root)>/map.json`; reparse só o que mudou | `cache_reparses_only_the_file_that_changed` |
| Higiene | Leitura obsoleta após `edit_file`; log de comando colapsado; duplicata omitida | `hygiene_marks_a_stale_read_after_an_edit`, `hygiene_collapses_a_huge_command_log` |
| Compactação (ADR 0008) | Miolo da conversa → recado estruturado depois de higiene+trim | `compact_folds_the_middle_and_keeps_the_task` |
| Explorer como role | Pergunta clara (`?` / onde / o que é) e sem verbo de edição → tools de leitura; `edit_file` recusado | `a_question_uses_the_explorer_role_and_refuses_edits` |
| Classificação conservadora | Na dúvida, Coder (scripts/eval não perdem `edit_file`) | `a_clear_question_is_explorer_and_an_edit_request_is_coder` |

O Context Manager **não** faz rede e **não** monta prompt no frontend. Resultados de tool continuam delimitados como não confiáveis (SPEC §20.5).

## Tokens

O modelo scripted reporta `promptTokens` do Ollama = 0. A evidência desta fase é a estimativa `chars/4` gravada em `TaskMetrics.estimatedTokens` / `peakEstimatedTokens` e no JSON do eval (`estimatedTokens`).

O mapa de um fixture pequeno (soma/greet/dobro) cabe em dezenas de tokens e é menor do que concatenar os `.ts`. Numa tarefa viva, o ganho real é o Coder **não** listar a árvore inteira e a higiene encolher logs/leituras velhas nos turnos seguintes.

Comparar `estimatedTokens` entre relatórios (Fase 9 vs Fase 10) no mesmo `--scripted` mede o custo do mapa no system prompt; o script ainda faz as mesmas tool calls, então a queda de tokens **por tarefa ao vivo** só aparece com Ollama.

## Eval

```bash
cargo run -q -p cd-ai-cli -- eval --scripted
```

A suíte não cresceu (planos 016 D3 / 018 D10 / 019 D10 / 020 D9). Esperado: **100% (3/3)** sem regressão, `agentStatus: completed`.

## Caminho ao vivo (Ollama + CODER)

Nesta VM de cloud **não há Ollama**. A taxa ao vivo e a comparação de tokens reais ficam bloqueadas até rodar, numa máquina com o Ollama 0.34+ e `qwen3-coder:30b` puxado (decisão 0002):

```bash
ollama show qwen3-coder:30b
cargo run -q -p cd-ai-cli -- eval --model qwen3-coder:30b --out evals/results/qwen3-coder_30b-fase10.json
```

Compare `promptTokens` (Ollama) e `estimatedTokens` com o JSON da Fase 9. A CLI imprime `~N tok` por tarefa e uma linha `contexto: coder/trivial mapa …`.

## Fora desta fase

Skills / Skill Router (Fase 11), model router e memória (Fase 12), watcher de filesystem, `search_symbols` como tool do modelo, embeddings/RAG, suíte de eval maior.
