# Fase 11 — Skills (auditoria)

Medido em 2026-09-16. Plano: [`plans/021-fase-11-skills.md`](../../plans/021-fase-11-skills.md).

## O que o core cobre de verdade

| Garantia | Como | Teste |
|---|---|---|
| Registry só com licença redistribuível | Allowlist SPDX; origin `cd-ai` + Apache-2.0 | `every_catalog_entry_is_licensed_original_and_within_budget`, `registry_drops_an_unlicensed_skill` |
| Terceiros sem licença **não** embarcam | Nomes conceituais da §17.1 fora do `CATALOG` | `third_party_conceptual_names_are_not_embedded` |
| Condensado ≤ orçamento | `chars/4` ≤ `token_budget` (160 tok típico) | o mesmo `every_catalog_entry_…` |
| Router determinístico | Score perfil/path/keywords; empate por nome | `eval_style_typescript_test_loads_testing_and_typescript` (roda duas vezes) |
| Dependências | `nextjs` → `react` → `typescript` | `nextjs_pulls_react_and_typescript` |
| Conflitos | fica o maior score | `conflicts_keep_the_higher_score` |
| Orçamento da seção (10%) | derruba a de menor score | `budget_drops_the_lowest_score` |
| Sem bypass de confiança | `permissions` é declaração; security diz para não enfraquecer path/sandbox | `security_skill_does_not_claim_a_bypass`; o Permission Manager não lê o catálogo |
| Eventos + estado | `skillsDetected` / `skillLoaded` / `skillSkipped`; `TaskState.selectedSkills` | `a_typescript_fix_task_emits_skill_events_and_keeps_the_trust_boundary` |

Catálogo original embarcado: `typescript`, `rust`, `react`, `nextjs`, `tailwind`, `testing`, `debugging`, `security`, `code-review`, `performance`, `git`, `accessibility`, `frontend-design`.

Não embarcados (sem licença de redistribuição no repo): `coss`, `coss-particles`, `emil-design-eng`, `apple-design`, `impeccable`, `ponytail`, `caveman`. Handoff Explorer→Coder usa texto curto original (D8), não a skill de terceiro.

## Eval por skill (ganho ou neutralidade)

A suíte **não cresceu** (016 D3). `ScriptedModel` não lê o system prompt, então a taxa scripted só pode mostrar **neutralidade**.

```bash
cargo run -q -p cd-ai-cli -- eval --scripted --out evals/results/scripted-fase11.json
```

| evidência | resultado |
|---|---|
| Taxa scripted | **100% (3/3)** — mesma da Fase 10 |
| Skills nas fixtures | `soma` / `greet` / `dobro` carregam `typescript` + `testing` + `debugging` (pedido fala em teste/falha; perfil JS/TS) |
| Campo no JSON | `selectedSkills` por tarefa |
| Condensado vs “full” de terceiro | não há full no binário; o condensado cabe em 10% de `num_ctx` |
| Tokens | a seção skills **aumenta** `estimatedPromptTokens` de forma limitada (orçamento 10%). Não é regressão de taxa. Ganho de qualidade de modelo só é mensurável ao vivo. |

Fase 10 (audit) `soma` ≈ 2240 tok estimados em 4 turnos. Nesta fase (JSON `evals/results/scripted-fase11.json`):

| tarefa | iterações | estimatedPromptTokens | selectedSkills | agentStatus |
|---|---|---|---|---|
| dobro | 4 | 3307 | typescript, testing, debugging | completed |
| greet | 4 | 3138 | typescript, testing, debugging | completed |
| soma | 4 | 3100 | typescript, testing, debugging | completed |

O acréscimo (~860 tok vs Fase 10) é a seção skills somada nos 4 turnos (~215 tok/turno), dentro do orçamento de 10% de `num_ctx`. Taxa inalterada: **neutralidade**. Ganho de qualidade de modelo só é mensurável ao vivo.

## Caminho ao vivo (Ollama + CODER)

Nesta VM **não há Ollama**. Comparar taxa e `promptTokens` reais fica para uma máquina com Ollama 0.34+ e `qwen3-coder:30b`:

```bash
cargo run -q -p cd-ai-cli -- eval --model qwen3-coder:30b --out evals/results/qwen3-coder_30b-fase11.json
```

Expectativa: check ≥ 3/3 (neutralidade); se a taxa subir, isso é o ganho da fase.

## Fora desta fase

Model Router / memória / trajetórias (Fase 12), marketplace, importação de skills do usuário, desempate LLM, suíte maior.
