# Fase 13 — Otimização (auditoria)

Medido em 2026-09-16. Plano: [`plans/023-fase-13-otimizacao.md`](../../plans/023-fase-13-otimizacao.md). Ambiente: debug `cargo test`, Rust 1.98, sem Ollama. **Medir primeiro**; só entrou o que a tabela justificou.

## O que foi medido (antes)

| Hipótese | Resultado | Veredito |
|---|---|---|
| Walk do mapa (400 `.ts`) | Frio < 2 s; quente menor (`from_cache`) | Cache em disco da Fase 10 **já funciona**. Não reescrever o índice. |
| `assemble_new` anda duas vezes | Código: `probe` + `assemble`. No start, router 8k chamava `assemble_new` de novo → até **4 walks** nas fixtures triviais | **Justificado.** Um índice; re-corte de orçamento. |
| Truncar o mapa O(n²) | `render(800)` em 400 arquivos < 200 ms | Barato; prefix-sum. Não era o hotspot. |
| Leituras em paralelo | 16 × 64 KiB `read_file` = **2,14 s**; `fs::read` paralelo = **2,1 ms** | O disco não era o gargalo. O custo era o **redactor**. |
| Entropia Shannon | `shannon_entropy` do zero em cada janela de 32 bytes (n × 32 × 256) | **Hotspot nº 1.** Sliding histogram + skip se `unique < 26`. |
| Tokenizer por modelo | Sem Ollama; `chars/4` já corta | **Descartado** (015 D10). |
| Watcher | 020 D1 | **Descartado.** Dirty-flag do agente. |
| UI a cada token | `setTasks` por evento | Coalescer num frame; `memo` no Markdown. |

## O que mudou

| Área | Mudança | Prova |
|---|---|---|
| Redactor | Histogram deslizante; janela com < 26 bytes distintos não calcula H (log₂(25) < 4,7). Limite 4,7 / janela 32 / run 2 **iguais**. | `entropy_catches_high_cardinality_run`, prosa, código; `entropy_scan_of_low_cardinality_source_is_bounded` (64 KiB de `x` < 50 ms, era ~134 ms **por arquivo**). |
| Contexto | `assemble_new` um walk. Router 8k reusa o mapa. Refresh reusa em memória até edit/write/comando. Cache JSON não regrava se nada mudou. | `assemble_new_is_not_twice_as_expensive_as_assemble`; eval `cacheHits`/`cacheMisses`. |
| Parse | `Parser` reusado; ≥ 8 misses em até 4 threads. Render por prefix-sum. | `a_midsize_tree_indexes_and_renders_in_bounded_time`. |
| Tools | 2+ `read_file` não-secret no mesmo turno: I/O paralelo, eventos na ordem. | `two_calls_in_one_turn_both_run_in_order`; `measure_sequential_vs_parallel_independent_reads` agora < 200 ms nos dois lados. |
| Métricas | `contextMs`, `cacheHits`, `cacheMisses` (`serde default`). JSON do eval copia. | `scripted_soma_passes_its_check`. |
| UI | Tokens coalescidos num `requestAnimationFrame`; `React.memo(Markdown)`; `useMemo` no agrupamento. | Testes de `applyAgentEvent` inalterados (o reducer continua puro). |

## Eval

A suíte **não cresceu** (016 D3). `ScriptedModel` não fala com o Ollama — a taxa scripted só pode mostrar **neutralidade**; o ganho de wall-clock local é contexto/tools, não o modelo.

```bash
cargo run -q -p cd-ai-cli -- eval --scripted --out evals/results/scripted-fase13.json
```

Números desta sessão (`evals/results/scripted-fase13.json`, 2026-09-16):

| tarefa | iterações | estimatedPromptTokens | contextMs | cacheHits | cacheMisses | agentStatus |
|---|---|---|---|---|---|---|
| dobro | 4 | 3310 | 2 | 4 | 2 | completed |
| greet | 4 | 3140 | 1 | 4 | 2 | completed |
| soma | 4 | 3100 | 1 | 4 | 2 | completed |

Taxa **100% (3/3)**. Tokens estimados iguais à Fase 12 (±3 tok no dobro/greet; soma idêntico). Rota `coder/8192`. Cada tarefa: **2 misses** (start + reindex após edit) e **4 hits** (refresh e recorte 8k reusam o mapa).

Comparado à Fase 12:

| evidência | Fase 12 | Fase 13 |
|---|---|---|
| Taxa scripted | 100% (3/3) | **100% (3/3)** |
| Rota | coder / 8192 | coder / 8192 |
| `estimatedPromptTokens` (soma) | 3100 | **3100** |
| `cacheHits` / `cacheMisses` (soma) | — | **4 / 2** |
| Entropia 16 × 64 KiB | 2,14 s | **< 200 ms** (teste de lote) |
| Walks no start trivial | até 4 | **1** |

Tokens estimados continuam `chars/4`. O prompt das fixtures cabe em 8k; o ganho desta fase **não** é cortar chars — é não reindexar, não recalcular Shannon à toa, e não serializar o cache a cada turno.

## Caminho ao vivo (Ollama + CODER)

Nesta VM **não há Ollama**. Wall-clock de prefill/geração fica para uma máquina com Ollama 0.34+ e o CODER da decisão 0002:

```bash
cargo run -q -p cd-ai-cli -- eval --model qwen3-coder:30b --out evals/results/qwen3-coder_30b-fase13.json
```

Expectativa: check ≥ 3/3; `toolMs` menor em turnos com várias leituras; `cacheHits` > 0; janela 8k nas triviais (Fase 12).

## Fora desta fase

Empacotamento (Fase 14), tokenizer por modelo, watcher, suíte de eval maior, provider cloud.
