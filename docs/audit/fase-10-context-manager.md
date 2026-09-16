# Fase 10 — Context Manager completo (auditoria)

Medido em 2026-09-16. Plano: [`plans/020-fase-10-context-manager.md`](../../plans/020-fase-10-context-manager.md).

## O que o core cobre de verdade

| Garantia | Como | Teste |
|---|---|---|
| Repo map tree-sitter | Walk `ignore` + assinaturas TS/JS/Rust; secrets e gitignore fora | `extracts_typescript_and_rust_signatures`, `gitignore_and_secrets_are_skipped` |
| Rank por tarefa | Tokens do pedido × path/símbolos; testes ao lado sobem nas notas | `ranks_the_file_named_in_the_request` |
| Cache com invalidação | `<data_dir>/cache/<sha256(root)>/repo-map.json`; mtime+tamanho, sem TTL | `cache_reuses_unchanged_files_and_reparses_after_mtime` |
| Orçamento por seção | Perfil 8%, mapa 12%, notas 8%, histórico ~44%, resposta 20%; corte `[truncated]` | `section_budget_truncates_a_huge_profile_rule`, `render_respects_the_char_budget` |
| Higiene | Mesmo path: versão antiga some; log enorme de comando reduz a erro/arquivo/linha | `stale_reads_of_the_same_file_are_dropped`, `huge_command_logs_keep_error_lines` |
| Compactação (ADR 0008) | Acima de 80% o miolo vira resumo do `TaskState`; `ContextExhausted` só se system+pedido não cabem | `compact_replaces_the_middle_with_a_record`, `an_exhausted_context_stops_the_task` |
| Explorer como role | Pergunta → prompt + tools só de leitura; edição → Coder com briefing | `questions_are_explorer`, `explorer_rejects_an_edit_and_coder_gets_every_tool` |
| Menos tokens que o dump | Workspace gordo: mapa ranqueado < role+conteúdo de todos os ficheiros | `managed_context_uses_fewer_tokens_than_dumping_the_tree` |

O frontend **não** monta contexto. Marcadores de tool result não confiável (SPEC §20.5) continuam em tudo o que fica na conversa.

## Eval

```bash
cargo run -q -p cd-ai-cli -- eval --scripted
```

A suíte não cresceu (planos 016 D3 / 018 D10 / 019 D10 / 020 D8). Taxa scripted medida em 2026-09-16: **100% (3/3)** — sem regressão. As três tarefas terminam `completed` / `verified` (o Verifier da Fase 8 continua a valer). Iterações: **4** em cada uma (o Explorer não consome um turno extra; D7).

`promptTokens` deixa de ser 0: o loop estima `chars/4` quando o `ScriptedModel` não reporta a contagem do Ollama.

| Tarefa | promptTokens (estimado) | genTokens | iterações |
|---|---|---|---|
| dobro | 2470 | 20 | 4 |
| greet | 2402 | 20 | 4 |
| soma | 2369 | 20 | 4 |
| **total** | **7241** | **60** | |

A baseline scripted da Fase 6 gravava `promptTokens: 0` porque o modelo scripted não estimava. Estes números não são comparáveis com essa coluna a zero — são a primeira medição honesta no dry-run.

A prova de **menos tokens por tarefa** vs mandar a árvore inteira está no teste `managed_context_uses_fewer_tokens_than_dumping_the_tree` (30 módulos gordos + `soma.ts`: o mapa ranqueado cabe no orçamento; o dump dos corpos não). Nas fixtures minúsculas da suíte o mapa e o perfil antigo têm tamanho parecido; o ganho aparece em workspace real.

`bun run verify` exit 0 nesta branch (2026-09-16).

## Como testar à mão

```bash
# mapa + role no system prompt (precisa de Ollama):
cargo run -q -p cd-ai-cli -- task --model <modelo> --workspace <pasta> "explique o que soma faz"
# só leitura: edit_file / run_command devem voltar erro de role
cargo run -q -p cd-ai-cli -- task --model <modelo> --workspace <pasta> "corrija src/soma.ts"
# Coder, com repo map de soma.ts no system
```

## Caminho ao vivo (Ollama + CODER)

Nesta VM de cloud **não há Ollama**. A taxa ao vivo fica bloqueada até rodar, numa máquina com o Ollama 0.34+ e `qwen3-coder:30b` puxado (decisão 0002):

```bash
ollama show qwen3-coder:30b
cargo run -q -p cd-ai-cli -- eval --model qwen3-coder:30b --out evals/results/qwen3-coder_30b-fase10.json
```

O ganho esperado ao vivo é menos `list_directory` / leituras cegas: o mapa já aponta símbolos e testes relacionados.

## Fora desta fase

Skills / Skill Router (Fase 11), embeddings/RAG, watcher de filesystem, turno LLM de Explorer em tarefa `normal`, suíte de eval maior.
