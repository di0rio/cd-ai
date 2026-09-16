# Plano 022: Fase 12 — Model Router, memória e trajetórias

> **Instruções ao executor:** rode os passos nesta ordem e confirme cada verificação antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Uma parte só está pronta com `bun run verify` saindo com exit 0 (AGENTS.md).
>
> **Drift check (rode primeiro):** `git log -1 --oneline` deve estar em `main` com as Fases 0–11 (PR #11). Confira `crates/agent-core/src/skills/` (registry + router), `crates/agent-core/src/agent/role.rs` (`TaskKind` / Explorer), `crates/agent-core/src/agent/settings.rs` (só `model` + `permissionMode`), `crates/agent-core/src/ollama.rs` (`status` com `loaded`) e SPEC §10 / §24.2 / §25 / §34 Fase 12. Se o Skill Router ou o Context Manager tiverem sumido, trate como STOP.

## Status

- **Prioridade:** P1
- **Esforço:** L
- **Risco:** MEDIUM (trocar de modelo no meio da tarefa custa dezenas de segundos; FAST editando código estoura o orçamento — decisão 0002 regra 4; trajetória com secret é vazamento local)
- **Depende de:** 021 (Fase 11 no `main`)
- **Categoria:** feature (SPEC §10, §24.2, §25, §28, §30, §32, §34 Fase 12)
- **Planejado em:** commit `3c7f428` (`main`), 2026-09-16

## Por que isso importa

Até a Fase 11 o loop usa **um** modelo: o `--model` / o último escolhido na UI. Não há categorias FAST/CODER/REASONER, não há preferência pelo residente, não há escalonamento depois de falhas do Coder, não há memória estruturada no prompt, e nada grava trajetórias. Em hardware de 6 GB de VRAM (decisão 0002) cada troca descarrega ~20 GB; um FAST de 4B cabe 100% na GPU e **não edita**. A Fase 12 fecha SPEC §10 / §24.2 / §25: router determinístico, memória pequena e local, trajetórias opt-in.

## Estado atual (fatos)

- `Settings` tem `model` e `permissionMode`. Sem `fast` / `coder` / `reasoner` / `trajectories`.
- `OllamaModel` fixa nome + `num_ctx` no construtor. `ChatModel` não troca de modelo.
- `TaskKind` já existe (Fase 10): `question` / `trivial` / `normal` / `complex`.
- Fixtures de eval são `trivial` (pedido com verbo de edição, repo pequeno) e usam um único nome (`scripted`).
- Eval scripted 3/3. A suíte não cresce (016 D3 / 021 D9). `ScriptedModel` não fala com o Ollama — latência real só se mede ao vivo.
- Sem módulo de memória. Sem JSONL de trajetória.

## Decisões deste plano (não reabrir sem ADR)

| # | Decisão | Motivo |
|---|---|---|
| D1 | **Router é módulo em `agent-core`** (`agent/router.rs`). Função pura: `TaskKind` + assignment + inventário (disponíveis, carregados, RAM) + falhas do Coder → categoria, nome, `num_ctx`, motivo. Sem crate novo, sem LLM de desempate. | Decisão 0003. SPEC §4 / §10: determinístico antes do LLM. |
| D2 | **Nomes são configuração.** `Settings.fast` / `coder` / `reasoner` (opcionais) + o modelo do pedido (`TaskStart.model`). Nenhum identificador de pesos no código (decisão 0002 regra 5). Ausente ou indisponível → o modelo do pedido. Um só nome resolvido para as três categorias → todas usam ele (SPEC §10, caso normal). | Hardware de 32 GB cabe um grande residente. |
| D3 | **FAST não edita, a menos que seja o único modelo.** `question` → FAST quando o nome do FAST é distinto e disponível. `trivial` / `normal` / `complex` → CODER. FAST já carregado numa tarefa de edição **troca** para CODER (ganho justifica: 0002 regra 4). | `qwen3:4b` estoura o orçamento em diffs. Role Coder ≠ categoria CODER. |
| D4 | **Preferir o residente.** Trocar custa prefill + carga. Pergunta com CODER já carregado **fica** no CODER. Edição com CODER/REASONER carregado **fica**. Escalonamento (D6) é a exceção. Inventário vazio (scripted / Ollama down) = tudo “disponível”. | SPEC §10; ADR 0002 regra 1. |
| D5 | **Janela menor em pergunta e trivial.** `num_ctx = min(pedido, 8192)` nesses kinds. Normal/complexa e REASONER após escalonamento usam o pedido. Re-montar o system prompt com o `num_ctx` roteado, para o runtime não truncar em silêncio (SPEC §4). | Prefill domina a latência (0002 revisão). 8k é o ponto do FAST no bench. |
| D6 | **Escalonar para REASONER após 2 falhas de qualidade do Coder** (correção do Verifier ou tool call inválida). Não conta timeout/rede. Só se REASONER for um nome distinto e disponível. `ChatModel::apply_route` atualiza o adapter; `ScriptedModel` só registra. | SPEC §10: “escalar para REASONER após N falhas do CODER”. |
| D7 | **RAM: `/proc/meminfo` MemAvailable no Linux; `None` no resto.** Cabe se já está carregado, ou se `size + loaded` ≤ disponível + tamanhos carregados (descarregar libera). Sem crate novo. Sem número → não bloqueia. | SPEC §10 “memória disponível”. Sem `sysinfo`. |
| D8 | **Memória estruturada** em `<data_dir>/memory/<sha256 do root>/memory.json`. Kinds: `rule`, `architecture`, `preference`, `constraint`, `learned`. Editável (CLI `memory`), invalidável (`stale`). Sem RAG, sem extração automática do modelo (saída não confiável). Só entradas relevantes (score de tokens do pedido; constraints/rules primeiro) cabem em **5%** de `num_ctx`. Redactor antes do disco. | SPEC §24.2. Mesmo id de pasta que cache/shadow. |
| D9 | **Trajetórias opt-in, desligadas por padrão.** `Settings.trajectories` + flag `--trajectories` na CLI. JSONL versionado em `<data_dir>/trajectories/<sha256>/<task_id>.jsonl`: pedido, kind, rota, turnos (sem tokens de conteúdo), tools (sem output), validação, intervenção, resultado. Diffs = lista de paths. Tudo redigido. Nada sobe à rede. | SPEC §25 / decisão 0004. Dataset futuro, sem treino agora. |
| D10 | **Eval não cresce.** Scripted 3/3 não pode regressar. Evidência de latência: (a) `expected_prefill_cost` (num_ctx × peso da categoria) menor em `question`/`trivial` que em `normal`; (b) JSON com `modelCategory` + `routedNumCtx` + `routeReason`; (c) comando ao vivo documentado. Scripted usa um só modelo → D2, janela D5 ainda aplica. | 016 D3. Sem Ollama nesta VM a taxa ao vivo não bloqueia. |

## Escopo

**Dentro:** router puro + `apply_route`; settings de categorias e trajetórias; memória + CLI; JSONL opt-in; eventos `modelRouted` / `memoryLoaded`; eval JSON; testes determinísticos de rota/escalonamento/memória/trajetória; docs/handoff/audit.

**Fora:** Fase 13–14 (otimização ampla, empacotamento). Provider cloud. RAG/embeddings. Treino/fine-tune. Suíte de eval maior. UI completa de categorias (IPC + settings bastam; o picker atual continua sendo o default). Payloads ofensivos.

## Passos

1. **Plano e tipos:** este arquivo; `ModelCategory`; eventos `modelRouted` / `memoryLoaded`; `Settings` ganha `fast`/`coder`/`reasoner`/`trajectories`; `TaskState` ganha categoria + motivo + `coderFailures` (`#[serde(default)]`). Bindings via `cargo test`.
2. **Router:** `route` + inventário + RAM D7 + testes da tabela de contratos (único modelo, FAST pergunta, FAST não edita, residente, indisponível, memória, escalonamento, custo de prefill).
3. **`ChatModel::apply_route`:** Ollama troca nome/`num_ctx`; Scripted registra; `FormatCounter` encaminha.
4. **Memória:** store, relevância, orçamento 5%, seção `Memory:` no `assemble`, CLI `memory list|add|forget|stale`, testes (stale fora, secret redigido, corte).
5. **Trajetórias:** writer JSONL; ligado só com flag/settings; teste off = nenhum arquivo; on = linhas redigidas.
6. **Loop:** classificar → rotar → montar prompt no `num_ctx` roteado → emitir eventos; falhas D6 re-rotam; `--trajectories` / settings.
7. **Eval/CLI/Tauri:** campos novos no JSON; CLI imprime rota; `set_router_settings` no Tauri (ACL); eventos novos são no-op na conversa.
8. **Docs:** handoff (Fase 12 feita, próxima 13), README dos planos, `docs/audit/fase-12-router.md`.
9. **Gate:** `bun run verify` exit 0. `cargo run -q -p cd-ai-cli -- eval --scripted` 3/3.

## Contratos

Categorias:

```text
question                         → FAST (se distinto e disponível)
trivial / normal / complex       → CODER
FAST distinto + tarefa de edição → nunca FAST
só um nome resolvido             → as três categorias usam ele
CODER já carregado + pergunta    → fica no CODER (não descarrega)
FAST carregado + edição          → troca para CODER
2 falhas de qualidade            → REASONER se distinto e disponível
modelo configurado ausente       → fallback para o modelo do pedido
```

Janela:

```text
question | trivial   → min(pedido, 8192)
normal | complex     → pedido
após escalonamento   → pedido (não encolher)
```

Custo (teste, não billing):

```text
expected_prefill_cost = num_ctx × { fast: 1, coder: 3, reasoner: 8 }
question/trivial  <  normal   (mesmo pedido 16k)
```

Memória:

```text
stale                         → fora
constraint/rule               → prioridade
overlap com o pedido          → entra
orçamento 5% de num_ctx       → corta a de menor score
modelo nunca grava sozinho
```

Trajetória:

```text
trajectories = false (default) → nenhum arquivo
trajectories = true            → JSONL local, redigido, sem output de tool
```

## Critérios de pronto

- [ ] `plans/022-fase-12-model-router-memoria-trajetorias.md` existe; linha 022 em `plans/README.md` = DONE
- [ ] Router determinístico: mesmas entradas → mesma categoria, nome, `num_ctx`, motivo (teste)
- [ ] FAST não edita quando há CODER distinto (teste)
- [ ] Preferência pelo carregado (teste)
- [ ] Fallback se o configurado não está na listagem (teste)
- [ ] Escalonamento após 2 falhas (teste no loop com `apply_route`)
- [ ] Memória relevante no prompt, stale fora, secrets redigidos (teste)
- [ ] Trajetórias off por padrão; on grava JSONL local redigido (teste)
- [ ] Eval scripted 3/3; JSON com rota; audit com evidência de custo/latência
- [ ] `bun run verify` exit 0
- [ ] PR aberto

## STOP conditions

- Pedido de mandar memória/trajetórias para a nuvem ou de treinar nesta fase → recuse (SPEC §3 / §25 / decisão 0004).
- Pedido de PoC/exploit “para testar a skill/memória de security” → recuse.
- Eval scripted caiu de 3/3 porque a janela 8k derrubou skills das fixtures → o condensado das três skills cabe em 10% de 8k; se falhou, a causa é outra. Não inchhe o script.
- Ollama ausente → não é STOP. Prove rota/escalonamento com testes; documente o comando ao vivo.

## Notas de manutenção

- Código e comentários em inglês; UI e docs em pt-BR.
- O frontend não escolhe a categoria por tarefa: recebe `modelRouted`. O Rust decide.
- `chars/4` continua sendo a estimativa. Tokenizer por modelo é Fase 13.
- Troca de modelo no Ollama descarrega o anterior; por isso D4 existe.
