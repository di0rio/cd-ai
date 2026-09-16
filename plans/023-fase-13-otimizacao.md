# Plano 023: Fase 13 — Otimização (medir, depois mudar)

> **Instruções ao executor:** rode os passos nesta ordem e confirme cada verificação antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Uma parte só está pronta com `bun run verify` saindo com exit 0 (AGENTS.md).
>
> **Drift check (rode primeiro):** `git log -1 --oneline` deve estar em `main` com as Fases 0–12 (PR #12). Confira `crates/agent-core/src/agent/repo_map.rs` (cache mtime), `crates/agent-core/src/agent/context.rs` (`assemble` / `assemble_new`), `crates/agent-core/src/agent/runner.rs` (tools em série), `crates/agent-core/src/redactor.rs` (`entropy_spans`), `crates/agent-core/src/agent/router.rs` e SPEC §15.3 / §16.4 / §27 / §34 Fase 13. Se o Context Manager, o Model Router ou o eval scripted tiverem sumido, trate como STOP.

## Status

- **Prioridade:** P1
- **Esforço:** M
- **Risco:** MEDIUM (redactor mais rápido que deixa passar secret é regressão de segurança; cache em memória que ignora edit do agente serve mapa podre; leituras paralelas que reordenam resultados confundem o modelo)
- **Depende de:** 022 (Fase 12 no `main`)
- **Categoria:** optimization (SPEC §15.3, §16.4, §27, §34 Fase 13)
- **Planejado em:** commit `be3be77` (`main`), 2026-09-16

## Por que isso importa

As Fases 10–12 já cortam tokens, cacheiam o mapa em disco e encolhem a janela em trivial. O loop ainda **mede pouco e gasta à toa**: `assemble_new` anda o workspace duas vezes, o router em 8k remonta o prompt com **outro** walk, o redactor calcula Shannon do zero em cada janela de 32 bytes, e leituras independentes do mesmo turno correm em série (SPEC §15.3). A Fase 13 fecha a saída da §34: ganhos comprovados por métrica e eval, sem otimizar por especulação.

## O que foi medido (antes de mudar)

Ambiente: debug `cargo test -p agent-core --lib`, Rust 1.98, um núcleo. Sem Ollama.

| Hipótese | Como mediu | Resultado | Veredito |
|---|---|---|---|
| Walk do mapa é o gargalo | 400 arquivos `.ts` de uma linha; `load_repo_map` frio vs quente | Frio < 2 s (tree-sitter); quente **menor** que o frio (`from_cache`) | Cache em disco **já funciona**. Ainda há walk+stat. Não reescrever o índice. |
| `assemble_new` anda duas vezes | Código: `probe = load_repo_map` + `assemble` → segundo `load_repo_map`. No start, se o router capar `num_ctx` (trivial 8k), **terceiro e quarto** walks | Confirmado por leitura; fixtures de eval são todas `trivial` | **Justificado.** Um índice, re-corte de orçamento. |
| Truncar o mapa é O(n²) | `render(800)` em 400 arquivos | < 200 ms no debug | Melhoria barata (prefix-sum); não é o hotspot. Fazer. |
| Leituras em paralelo | 16 × 64 KiB via `read_file` vs `thread::scope` + `fs::read` | Sequencial **2,14 s**; paralelo cru **2,1 ms** | Comparação injusta: o custo é o **redactor**, não o disco. Paralelo de `read_file` continua no SPEC — implementar **depois** do redactor, com teste de ordem. |
| Redactor / entropia | `entropy_spans` aloca `Vec<f64>` de n−31 e chama `shannon_entropy` (32+256 ops) **por byte** | 16 × 64 KiB ≈ 2,14 s em debug | **Hotspot nº 1.** Sliding histogram + pular janela com < 26 bytes distintos (log₂(25) < 4,7). Mesmos testes de detecção. |
| Tokenizer por modelo | Nota do plano 022 | Sem Ollama nesta VM; `chars/4` já corta o prompt | **Descartado.** Dependência nova sem prova (015 D10). |
| Watcher de filesystem | SPEC §16.4 | Plano 020 D1 recusou `notify` | **Descartado.** Invalidar por mtime + dirty-flag do próprio agente. |
| UI a cada token | `handleEvent` → `setTasks` a cada `token`; `Markdown` reparseia o bloco | Código: AppShell inteiro re-renderiza | **Justificado.** Coalescer num `requestAnimationFrame`; `memo` no Markdown. |

## Decisões deste plano (não reabrir sem ADR)

| # | Decisão | Motivo |
|---|---|---|
| D1 | **Medir, depois mudar.** Só entra o que a tabela acima justificou. Sem tokenizer, sem watcher, sem crate novo, sem reescrever o Tool Engine. | SPEC §34 Fase 13. |
| D2 | **Redactor: mesma detecção, algoritmo linear.** Janela 32 / limite 4,7 / run 2 **não mudam**. Histogram deslizante; se `unique < 26` a entropia não pode passar de 4,7. Testes de prefixo/PEM/JWT/prosa/código/high-cardinality continuam a passar. | Segurança não é o lugar de “aproximar”. |
| D3 | **Um walk por montagem.** `assemble_new` classifica com o mapa que vai para o prompt. Se o router só muda `num_ctx`, re-renderiza orçamento/skills **reusando o mapa**. Refresh do loop reusa o mapa em memória até edit/write/comando sujar o índice. | SPEC §16.4. Comando e edição marcam dirty; leitura não. |
| D4 | **Não regravar o JSON do cache se nada reparseou e nada nasceu/sumiu.** | Write de 400 entradas a cada turno é lixo. |
| D5 | **Leituras independentes do mesmo turno em paralelo** (`read_file` não-secret). Eventos e resultados **na ordem do pedido**. Escritas e `run_command` sequenciais. Secret (Ask) sequencial. | SPEC §15.3. Auto em leitura comum (§20). |
| D6 | **Parse tree-sitter reusa o `Parser` e, com ≥ 8 misses, reparte em até 4 threads.** Walk continua `ignore` (gitignore). Sem Rayon. | Frio é parse, não walk. |
| D7 | **Métricas novas com `#[serde(default)]`:** `contextMs`, `cacheHits`, `cacheMisses`. O JSON do eval as copia. Estados antigos carregam. | SPEC §27. Prova no audit, não billing. |
| D8 | **UI: coalescer tokens num frame; `React.memo` no Markdown.** Sem lib nova. | Token stream não pode clonar a lista de tarefas 50×/s. |
| D9 | **Eval não cresce.** Scripted 3/3 não pode regressar. Evidência: (a) taxa 3/3; (b) teste de entropia 64 KiB < teto; (c) `assemble_new` não 2× `assemble`; (d) duas `read_file` no mesmo turno, ordem estável; (e) `contextMs`/`cacheHits` no JSON. | 016 D3. |

## Escopo

**Dentro:** redactor linear; um walk por assemble; reuse de mapa no router/refresh; skip de write do cache; render por prefix-sum; parse reusado/paralelo; `read_file` paralelo no turno; métricas; rAF + memo na UI; testes; handoff/audit; plano.

**Fora:** Fase 14 (empacotamento). Tokenizer por modelo. Watcher. Suíte de eval maior. Provider cloud. Payloads ofensivos. Paralelizar escrita.

## Passos

1. **Plano:** este arquivo; linha 023 em `plans/README.md`.
2. **Redactor D2** + teste de 64 KiB em teto de tempo + os testes de detecção já existentes.
3. **Mapa D3/D4/D6:** `assemble` aceita mapa pronto; `assemble_new` uma vez; skip write; `Parser` reusado; render O(n).
4. **Loop:** dirty-flag; refresh reusa mapa; router 8k não reindexa.
5. **Tools D5:** lote de `read_file` não-secret no mesmo turno.
6. **Métricas D7** + JSON do eval + bindings (`cargo test`).
7. **UI D8:** rAF no `handleEvent`; `memo(Markdown)`.
8. **Docs:** handoff (Fase 13 feita, próxima 14), `docs/audit/fase-13-otimizacao.md`.
9. **Gate:** `bun run verify` exit 0. `cargo run -q -p cd-ai-cli -- eval --scripted` 3/3.

## Contratos

Entropia:

```text
janela 32, limite 4.7, run 2          → iguais à Fase 4
unique < 26                           → skip (impossível H > 4.7)
high-cardinality 200 bytes            → ainda redige
prosa PT-BR / código Rust             → ainda intactos
64 KiB de 'x'                         → < 50 ms (debug)
```

Mapa:

```text
assemble_new                          → 1 walk
router só muda num_ctx                → 0 walks a mais
refresh sem edit/write/comando        → 0 walks
cache hit sem mudança                 → não regrava repo-map.json
```

Leituras:

```text
2+ read_file não-secret no mesmo turno → I/O concorrente
ordem de files_read e tool messages    → a do pedido
secret / edit / write / command        → sequencial
```

## Critérios de pronto

- [x] `plans/023-fase-13-otimizacao.md` existe; linha 023 em `plans/README.md` = DONE
- [x] Entropia linear, detecção inalterada, 64 KiB abaixo do teto (teste)
- [x] `assemble_new` não custa ~2× `assemble` (teste)
- [x] Duas `read_file` no mesmo turno: ordem estável (teste já existente + lote)
- [x] Eval JSON com `contextMs` / `cacheHits` / `cacheMisses`
- [x] Eval scripted 3/3; audit com before/after
- [x] `bun run verify` exit 0
- [ ] PR aberto

## STOP conditions

- Pedido de PoC/exploit “para testar o redactor” → recuse.
- Entropia nova deixa passar o caso high-cardinality → pare e reverta D2.
- Eval scripted caiu de 3/3 → a otimização não vale; reverta o pedaço culpado.
- Ollama ausente → não é STOP. Prove com testes + scripted.

## Notas de manutenção

- Código e comentários em inglês; UI e docs em pt-BR.
- `chars/4` continua sendo a estimativa. Tokenizer por modelo ficou de fora de propósito.
- A fronteira de confiança não muda: path, permissão e redação continuam no Rust, mesmo com I/O paralelo.
