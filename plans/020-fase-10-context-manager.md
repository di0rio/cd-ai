# Plano 020: Fase 10 — Context Manager completo

> **Instruções ao executor:** rode os passos nesta ordem e confirme cada verificação antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Uma parte só está pronta com `bun run verify` saindo com exit 0 (AGENTS.md).
>
> **Drift check (rode primeiro):** `git log -1 --oneline` deve estar em `main` com as Fases 0–9 (PR #7). Confira `crates/agent-core/src/agent/prompt.rs` (perfil + `trim_for_budget` + `ContextExhausted`), `crates/agent-core/src/agent/profile.rs`, `crates/agent-core/src/tools/edit.rs` (`parse_check` tree-sitter), e SPEC §12.1 / §16 / §34 Fase 10. Se o corte explícito ou o tree-sitter tiverem sumido, trate como STOP.

## Status

- **Prioridade:** P1
- **Esforço:** L
- **Risco:** MEDIUM (contexto demais estoura a janela; Explorer read-only demais bloqueia o Coder; cache podre serve mapa velho)
- **Depende de:** 019 (Fase 9 no `main`)
- **Categoria:** feature (SPEC §12.1, §16, §34 Fase 10)
- **Planejado em:** commit `81fe812` (`main`), 2026-09-16

## Por que isso importa

Até a Fase 9 o contexto é o perfil determinístico da raiz, o transcript e um corte visível quando a janela enche (`ContextExhausted`). Não há mapa de símbolos, não há orçamento por seção, o cache não existe, a higiene é só omitir resultados antigos, e o Explorer ainda não é um role. O modelo ou lista o projeto à toa, ou estoura a janela de 32k.

A Fase 10 fecha SPEC §16: nunca enviar o projeto inteiro; montar o prompt por seções com orçamento; cachear perfil e repo map com invalidação por mtime/tamanho; higienizar o transcript; Explorer como role read-only quando a tarefa é pergunta.

## Estado atual (fatos)

- `workspace_profile` lê marcadores na raiz (linguagens, scripts, AGENTS.md) e cabe em 3 KiB. Sem walk do código, sem símbolos.
- `parse_check` já usa tree-sitter TS/JS/Rust em `edit.rs` / Verifier. Sem extração de símbolos. Sem `search_symbols`.
- Orçamento: `chars/4`, corte dos resultados de tool mais antigos a 75% de `num_ctx`, stop a 90%. Sem frações por seção.
- Sem cache. O perfil é recomputado em toda tarefa.
- Higiene: omitir tool results antigos. Sem colapso de log, sem leitura obsoleta depois de `edit_file`.
- Um único prompt de Coder e `tool_specs()` com as 10 tools. Roles do SPEC §12 ainda não existem no runtime (Verifier é módulo, plano 018 D1).
- Eval scripted 3/3. A suíte não cresce (016 D3 / 018 D10 / 019 D10).
- ADR 0008 (compactação automática) está em **proposta**, para validar nesta fase.

## Decisões deste plano (não reabrir sem ADR)

| # | Decisão | Motivo |
|---|---|---|
| D1 | **Context Manager é módulo em `agent-core`** (`agent/repo_map.rs` + `agent/context.rs`). Sem crate novo, sem embeddings/RAG. | Decisão 0003. SPEC §16: v1 sem RAG. |
| D2 | **Repo map = walk `ignore` + tree-sitter** nos `.rs`/`.ts`/`.tsx`/`.js`/`.jsx`. Símbolos = walk da AST (fn/struct/class/type/…), assinatura = primeira linha. Priorizar por overlap do pedido com path/símbolo. Pula `node_modules`/`target`/secrets/`detect_path_secret`. | Já há tree-sitter. Query files seriam dependência nova. SPEC §16.1. |
| D3 | **Orçamento por seção = fração de `num_ctx`**, com 25% reservado à resposta. Corte explícito (`[section truncated]`) e evento `ContextReady` listando seções cortadas. Estimativa continua `chars/4`. | SPEC §16.2. Sem tokenizer por modelo. |
| D4 | **Cache em `<data_dir>/context/<sha256(root)>/map.json`.** Invalidar por `(mtime, size)` por arquivo, não por tempo. Reparse só o que mudou; perfil invalidado pelos marcadores da raiz. Cache corrompido = rebuild. | SPEC §16.4. Mesmo `data_dir` do shadow (019). Sem watcher nesta fase: o walk no início da tarefa é o check. |
| D5 | **Higiene antes do trim:** (1) leitura de arquivo obsoleta depois de `edit_file`/`write_file` no mesmo path; (2) log enorme de `run_command` vira só linhas decisivas (erro, FAIL, path:linha, exit); (3) resultado de tool idêntico duplicado. Trim a 75% e `ContextExhausted` a 90% continuam. | SPEC §16.3. Skills fora (Fase 11). |
| D6 | **Compactação determinística (ADR 0008):** se depois da higiene+trim ainda passar de 75%, o miolo da conversa (depois do system + primeiro user, antes da cauda) vira um recado estruturado (pedido, arquivos, comandos). Sem LLM extra — um round-trip aumentaria tokens. | Continuidade sem teto financeiro. Transcript em disco permanece; só a janela enviada ao modelo é compactada. |
| D7 | **Explorer é role (prompt + tools), não um turno extra de LLM.** Classificação determinística: pergunta clara (`?` / onde / o que é / …) **e** sem verbo de edição → Explorer (read-only). Demais → Coder, já com o mapa e os achados no system prompt. Achados do Explorer são determinísticos (rank + testes irmãos), sem `read_file` extra no core. | SPEC §11.2 / §12.1. Um turno extra quebraria o eval scripted e **aumentaria** tokens (saída da Fase 10 pede o contrário). Classificação conservadora: na dúvida, Coder — bloquear `edit_file` num teste/script existente é pior. |
| D8 | **Tools do Explorer:** `read_file`, `list_directory`, `search`, `git_*`. `edit_file` / `write_file` / `run_command` recusados com erro claro (pt-BR). `search_symbols` como tool do modelo fica fora (SPEC §15.2, segunda leva). | Fronteira de confiança no Rust: o modelo não escolhe o conjunto. |
| D9 | **Eval não cresce.** Scripted 3/3 não pode regressar. Tokens: o scripted tem `prompt_tokens` do modelo = 0; medir `estimated_tokens` / pico no relatório. Provar que o mapa cabe no orçamento e é menor que concatenar as fontes. Comando ao vivo documentado. | 016 D3. Saída da Fase 10: sem regressão **e** menos tokens por tarefa — no scripted, evidência via estimativa + higiene; o ganho ao vivo depende do Ollama. |
| D10 | **Sem watcher de filesystem nesta fase.** Invalidação é o walk + fingerprint no início (e no resume, só em memória). | Mais simples e correto. Watcher é Fase 13 se a métrica pedir. |

## Escopo

**Dentro:** repo map tree-sitter; orçamento por seção; cache mtime/size; higiene; compactação D6; Explorer como role; evento `ContextReady`; métricas de estimativa no eval; testes; docs/handoff; ADR 0008 aceita.

**Fora:** Fase 11+ (skills, model router, otimização, release). `search_symbols` como tool. Watcher. Embeddings. Suíte de eval maior. Payloads ofensivos. Status `verifying`/`reviewing`/`fixing`.

## Passos

1. **`repo_map.rs`:** walk, parse, símbolos, score, render com orçamento, cache em disco, testes (TS/Rust, skip de dir gordo, invalidação, mapa ≪ dump).
2. **`context.rs`:** classificar; montar seções; Explorer findings; higiene; `assemble` no início da tarefa.
3. **`prompt.rs`:** prompts por role; `estimate_text_tokens`; compactação D6; stubs de higiene não reembrulhados.
4. **Loop:** `assemble` no `New` e refresh em memória no `Resume`; `tool_specs_for(role)`; recusar tools fora do role; higiene+trim+compact por iteração; gravar métricas; emitir `ContextReady`.
5. **Eval/CLI:** `estimatedTokens` / `peakEstimatedTokens` no JSON; a CLI mostra a estimativa.
6. **Docs:** plano 020, README, handoff (próxima = Fase 11), `docs/audit/fase-10-context.md`, ADR 0008 → aceita.
7. **Gate:** `bun run verify` exit 0. `cargo run -q -p cd-ai-cli -- eval --scripted` 3/3.

## Contratos

Seções (fração de `num_ctx`):

```text
role          8%
profile+rules 10%
repo map      12%
explorer      10%
inherited      8%
histórico     (o que couber até 75%)
reservado     25%  (nunca preenchido pelo prompt)
```

Roles:

```text
question → Explorer, tools de leitura
trivial / normal / complex → Coder, todas as tools
                           + mapa + achados no system prompt
```

Cache:

```text
GIT-like key = sha256(canonical root)
arquivo      = <data_dir>/context/<key>/map.json
hit          = mesmo (mtime, size) por path relativo
```

## Critérios de pronto

- [x] `plans/020-fase-10-context-manager.md` existe; linha 020 em `plans/README.md` = DONE
- [x] Repo map tree-sitter priorizado pela tarefa; teste de símbolos TS e Rust
- [x] Cada seção respeita o orçamento; corte explícito
- [x] Cache invalida por mtime/size e reparse só o arquivo mudado
- [x] Higiene omite leitura obsoleta e colapsa log grande
- [x] Pergunta clara usa o prompt/tools do Explorer; Coder nas demais
- [ ] `cd-ai eval --scripted` 3/3, com estimativa de tokens no relatório
- [ ] `bun run verify` exit 0
- [ ] PR aberto

## STOP conditions

- Pedido de embeddings/RAG nesta fase → recuse; SPEC §16 v1 sem isso.
- Pedido de PoC/exploit contra o cache ou o workspace → recuse.
- Eval scripted caiu de 3/3 porque o Explorer bloqueou `edit_file` → a classificação está agressiva demais; volte a Coder na dúvida, não desligue o mapa.
- Ollama ausente → não é STOP; documente o comando ao vivo.

## Notas de manutenção

- Código e comentários em inglês; UI e docs em pt-BR.
- Resultados de tool continuam marcados untrusted (SPEC §20.5).
- O frontend não monta contexto: só mostra `ContextReady` se a UI quiser; o default já ignora eventos sem linha.
- `u64` novo em evento/métrica leva `#[ts(type = "number")]`.
