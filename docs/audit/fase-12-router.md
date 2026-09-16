# Fase 12 — Model Router, memória e trajetórias (auditoria)

Medido em 2026-09-16. Plano: [`plans/022-fase-12-model-router-memoria-trajetorias.md`](../../plans/022-fase-12-model-router-memoria-trajetorias.md).

## O que o core cobre de verdade

| Garantia | Como | Teste |
|---|---|---|
| Nomes são config | `Settings.fast/coder/reasoner`; nenhum tag no `route` | `a_single_model_serves_every_category` |
| Um só nome → as três categorias | FAST=CODER=REASONER usa o modelo do pedido | o mesmo |
| FAST não edita se CODER distinto | `trivial`/`normal`/`complex` → CODER | `fast_does_not_edit_when_coder_is_distinct` |
| Pergunta → FAST quando distinto e livre | e CODER não está carregado | `a_question_uses_fast_when_it_is_distinct_and_free` |
| Preferir o residente | pergunta com CODER carregado fica no CODER | `a_loaded_coder_is_kept_for_a_question` |
| Fallback se ausente | nome configurado fora da listagem → pedido | `a_missing_configured_model_falls_back_to_the_task_model` |
| RAM | `/proc/meminfo` MemAvailable; `None` fora do Linux; não cabe → residente | `mem_available_parses_proc_meminfo`, `a_model_that_does_not_fit_keeps_the_resident` |
| Janela 8k em pergunta/trivial | `num_ctx = min(pedido, 8192)` | `a_single_model_serves_every_category`; loop `a_trivial_task_emits_a_route_and_caps_the_window` |
| Prefill menor em trivial | `expected_prefill_cost = num_ctx × {fast:1, coder:3, reasoner:8}` | `trivial_and_question_cost_less_than_normal` |
| Rota estável | mesmas entradas → mesma decisão | `routing_is_stable` |
| Escalonamento após 2 falhas | Verifier/tool inválida; REASONER distinto | `two_coder_failures_escalate_to_reasoner`; loop `two_quality_failures_escalate_to_reasoner` |
| Sem escalonamento se o nome é o mesmo | 5 falhas no único modelo → continua CODER | `same_reasoner_name_does_not_escalate` |
| Memória estruturada | JSON local; kinds §24.2; CLI `memory` | `add_list_forget_and_stale_round_trip` |
| Stale fora; overlap; orçamento 5% | constraints/rules primeiro | `stale_entries_are_not_selected`, `learned_facts_need_overlap_rules_do_not`, `budget_drops_the_lowest_score`; `assemble_injects_relevant_memory_and_skips_stale` |
| Secrets no disco | redactor antes de gravar | `secrets_are_redacted_before_reaching_disk` |
| Trajetórias off por padrão | nenhum JSONL sem flag/settings | `trajectories_are_off_by_default_and_opt_in_writes_jsonl` |
| Trajetória opt-in redigida | JSONL local; tokens ignorados | `opt_in_writes_redacted_jsonl`, `observe_ignores_tokens` |
| Eventos + estado | `modelRouted` / `memoryLoaded`; `TaskState.modelCategory` | `a_trivial_task_emits_a_route_and_caps_the_window` |
| Eval JSON | `modelCategory` + `routedNumCtx` + `routeReason` | `eval.rs`: `soma.model_category == "coder"`, `routed_num_ctx == 8192` |

Nada sobe à rede (decisão 0004). O frontend não escolhe a categoria: recebe `modelRouted`.

## Latência em tarefas triviais (prova determinística)

O exit da Fase 12 pede latência média menor em tarefas triviais. Nesta VM **não há Ollama**, então a prova é o custo de prefill (ADR 0002: o prefill domina, não a geração):

```text
expected_prefill_cost = num_ctx × { fast: 1, coder: 3, reasoner: 8 }
```

Pedido de 16k, inventário com FAST/CODER/REASONER distintos e nada carregado (`trivial_and_question_cost_less_than_normal`):

| kind | categoria | `num_ctx` | custo |
|---|---|---|---|
| question | FAST | 8 192 | **8 192** |
| trivial | CODER | 8 192 | **24 576** |
| normal | CODER | 16 384 | **49 152** |

`question < trivial < normal`. A suíte de eval é toda `trivial` (pedido com verbo de edição + repo pequeno), então cada fixture cai em **8k** em vez de 16k — metade da janela, mesmo peso CODER: custo 24 576 vs 49 152 (−50%). Skills condensadas das fixtures cabem em 10% de 8k.

Com um só modelo (caso scripted / hardware que só tem o CODER): a categoria fica `coder`, mas a janela 8k **ainda aplica**. É o que o eval scripted exercita.

## Eval

A suíte **não cresceu** (016 D3). `ScriptedModel` não fala com o Ollama — a taxa scripted só pode mostrar **neutralidade**; a latência real só se mede ao vivo.

```bash
cargo run -q -p cd-ai-cli -- eval --scripted --out evals/results/scripted-fase12.json
```

| evidência | resultado |
|---|---|
| Taxa scripted | **100% (3/3)** — mesma da Fase 11. Gate `bun run verify` exit 0. |
| Rota nas fixtures | `coder` / `8192` (trivial + um só modelo) |
| Campo no JSON | `modelCategory`, `routedNumCtx`, `routeReason` |
| Prefill vs Fase 11 | janela 8k em vez de 16k; tokens estimados continuam `chars/4` (o prompt cabe; o ganho é prefill, não contagem de chars) |

Números por tarefa (`evals/results/scripted-fase12.json`, 2026-09-16):

| tarefa | iterações | estimatedPromptTokens | modelCategory | routedNumCtx | agentStatus |
|---|---|---|---|---|---|
| dobro | 4 | 3307 | coder | 8192 | completed |
| greet | 4 | 3138 | coder | 8192 | completed |
| soma | 4 | 3100 | coder | 8192 | completed |

Taxa **100% (3/3)**. Tokens estimados iguais à Fase 11 (o prompt das fixtures cabe em 8k; o ganho é prefill, não `chars/4`). Rota `coder/8192` em todas.

## Caminho ao vivo (Ollama + CODER, opcionalmente FAST)

Nesta VM **não há Ollama**. Comparar wall-clock de tarefas triviais vs. a Fase 11 fica para uma máquina com Ollama 0.34+ e os modelos da decisão 0002:

```bash
# só o CODER (janela 8k nas fixtures triviais; mesmo nome nas três categorias)
cargo run -q -p cd-ai-cli -- eval --model qwen3-coder:30b --out evals/results/qwen3-coder_30b-fase12.json

# com FAST distinto: perguntas iriam para qwen3:4b; as fixtures de eval continuam CODER
# (edição). Configure settings.fast / settings.coder no data dir, ou rode uma pergunta
# avulsa: cd-ai task --model qwen3-coder:30b "o que este repo faz?"
```

Expectativa: check ≥ 3/3 (neutralidade); wall-clock por fixture trivial menor que a Fase 11 no mesmo hardware, porque o prefill lê 8k em vez de 16k. Se só um modelo está puxado, a categoria não muda — só a janela.

## Fora desta fase

Otimização ampla (Fase 13), empacotamento (Fase 14), UI de categorias, RAG/embeddings, extração automática de memória, treino/fine-tune, suíte de eval maior, provider cloud.
