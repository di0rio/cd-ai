# Plano 020: Fase 10 — Context Manager completo

> **Instruções ao executor:** rode os passos nesta ordem e confirme cada verificação antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Uma parte só está pronta com `bun run verify` saindo com exit 0 (AGENTS.md).
>
> **Drift check (rode primeiro):** `git log -1 --oneline` deve estar em `main` com as Fases 0–9 (PR #7). Confira `crates/agent-core/src/agent/prompt.rs` (`trim_for_budget` / `ContextExhausted`), `crates/agent-core/src/agent/profile.rs`, `crates/agent-core/src/tools/edit.rs` (`parse_check` tree-sitter), `docs/decisions/0008-continuidade-sem-teto-de-token.md` e SPEC §12.1 / §16 / §34 Fase 10. Se o perfil, o corte explícito ou o parse tiverem sumido, trate como STOP.

## Status

- **Prioridade:** P1
- **Esforço:** L
- **Risco:** MEDIUM (contexto mal cortado faz o modelo perder o pedido; Explorer com tools de escrita fura o role; cache podre serve mapa velho)
- **Depende de:** 019 (Fase 9 no `main`)
- **Categoria:** feature (SPEC §12.1, §16, §34 Fase 10)
- **Planejado em:** commit `81fe812` (`main`), 2026-09-16

## Por que isso importa

Até a Fase 9 o contexto é o perfil determinístico da raiz + a conversa inteira. O corte é bruto: omitir resultados de tool antigos e, se ainda passar de 90% de `num_ctx`, **parar** (`ContextExhausted`). Não há mapa de símbolos, não há orçamento por seção, o perfil não é cacheado, e o Explorer da §12.1 não existe — o Coder explora com as dez tools.

A Fase 10 fecha SPEC §16 e a saída da §34: repo map tree-sitter, orçamento por seção, cache com invalidação por mtime, higiene, Explorer como role, compactação automática (decisão 0008) no lugar do teto cego.

## Estado atual (fatos)

- `workspace_profile` lê marcadores da raiz (sem cache). `render` ≤ 3 KiB, inclui `root_entries`.
- `system_prompt` é um bloco único. `trim_for_budget` zera os resultados de tool mais antigos (os 2 recentes ficam). Acima de 90% → `StopReason::ContextExhausted` → `failed`.
- `parse_check` em `edit.rs` já escolhe a grammar TS/Rust por extensão. O Verifier reusa isso. Nenhum extrator de símbolos, nenhum walk do workspace além do `search`.
- O loop oferece `tool_specs()` completo em todo turno. Não há `TaskKind` nem role.
- Resultados de tool vão delimitados (`UNTRUSTED_TOOL_BEGIN/END`). Isso permanece.
- Eval scripted 3/3. `ScriptedModel` reporta `prompt_tokens: 0`. A suíte não cresce (016 D3).
- Decisão 0008 está **proposta**, a validar nesta fase.

## Decisões deste plano (não reabrir sem ADR)

| # | Decisão | Motivo |
|---|---|---|
| D1 | **Context Manager é módulo em `agent-core`** (`syntax.rs`, `repo_map.rs`, `context.rs`, `role.rs`). Sem crate novo, sem watcher, sem embeddings. | Decisão 0003. SPEC §16.4: invalidar por hash/mtime, não por tempo. `notify` seria dependência nova sem ganho nesta fase. |
| D2 | **Repo map = walk `ignore` + tree-sitter TS/Rust** (as mesmas extensões de `parse_check`). Por arquivo: path + assinaturas (campo `name` / primeira linha do nó). Extensão fora da lista: path só, se o arquivo pontuar. Nunca o projeto inteiro. | SPEC §16.1. Reusa a grammar já no Cargo.toml (018 D11 reservou o map para agora). |
| D3 | **Cache em disco** `<data_dir>/cache/<sha256 do root>/repo-map.json`. Entrada por arquivo com mtime+tamanho. Reindexar só o que mudou ou nasceu; apagar o que sumiu. Perfil cacheado pelos mtimes dos marcadores da raiz. | SPEC §16.4. Mesmo id de pasta que o shadow (019). Sem o data_dir o map ainda funciona, só não persiste. |
| D4 | **Orçamento por seção, fração de `num_ctx`.** Sempre reservar 20% para a resposta. Role 12%, regras/perfil 8%, repo map 15%; o resto é histórico. Seção que estoura é cortada com marcador e evento. O pedido do usuário **não** é cortado. | SPEC §16.2. Pedido truncado silenciosamente é pior que parar. |
| D5 | **Higiene, depois omitir tools, depois compactar.** (1) resultado de `read_file` cujo path foi `edit_file`/`write_file` depois → omitir; (2) output grande de `run_command` → só linhas de erro/arquivo:linha + cabeça/cauda; (3) `trim_for_budget` atual; (4) se ainda > 75%, condensar o meio num resumo **só com fatos do engine** (pedido, files_changed, commands) e manter system + pedido + os 2 resultados recentes. `ContextExhausted` só se, depois disso, ainda passar de 90% — em geral porque o pedido sozinho não cabe. | SPEC §16.3 e decisão 0008. Resumo gerado pelo core, não pelo modelo (tool results são não confiáveis). |
| D6 | **Explorer = o mesmo modelo, prompt + tools diferentes.** Tools: `read_file`, `list_directory`, `search`, `git_*`. Sem `edit_file` / `write_file` / `run_command`. A recusa é no loop (fronteira de confiança), não só na lista oferecida. | SPEC §12.1. O modelo pode emitir a call mesmo assim; o Rust não executa. |
| D7 | **LLM Explorer só em `pergunta` e `complexa`.** `trivial` e `normal` começam Coder, com o map já no system prompt. Classificação determinística, sem LLM. `pergunta` termina no Explorer (sem edição). `complexa`: no turno sem tools, ou após 8 iterações de exploração, vira Coder com os achados na conversa. | SPEC §4: poucas trocas de role. SPEC §11.2: trivial → Coder. Os fixtures de eval são pequenos e pedem correção → `trivial`, scripts atuais continuam válidos. |
| D8 | **Classificar assim:** `pergunta` se há `?` ou interrogativa e nenhum verbo de edição; `trivial` se ≤ 12 arquivos-fonte **ou** o pedido cita um path que existe; `complexa` se o pedido > 800 caracteres ou fala em arquitetura/refatoração/migração; senão `normal`. | Barato, testável, português e inglês. Sem classificador LLM. |
| D9 | **Eval não cresce.** Scripted 3/3 não pode regressar. `TaskMetrics.estimated_prompt_tokens` soma `chars/4` por turno (o ScriptedModel não preenche `prompt_tokens`). A redução de tokens é evidência de teste (higiene/orçamento/compactação) + o campo no JSON do eval; o comando ao vivo fica documentado. | 016 D3. Saída da Fase 10: eval sem regressão **e** menos tokens — o segundo não dá para mentir com `prompt_tokens: 0`. |
| D10 | **Decisão 0008 passa a aceita.** Compactação automática no loop. Sem teto financeiro; o teto de janela continua sendo hardware. | O próprio ADR pedia validação nesta fase. |

## Escopo

**Dentro:** `syntax` (grammar por path); repo map + cache mtime; orçamento por seção; higiene + compactação; roles Explorer/Coder; classificação; eventos de corte/compactação/role; métrica estimada; testes; docs/handoff; ADR 0008 aceita.

**Fora:** Fase 11+ (skills, router, memória, release). Watcher de filesystem. `search_symbols` como tool. Status `verifying`/`reviewing`/`fixing`. Suíte de eval maior. Payloads ofensivos.

## Passos

1. **Plano e tipos:** este arquivo; `AgentRole`, `TaskKind`; `estimated_prompt_tokens`; eventos `contextBudgetCut` / `contextCompacted` / `roleChanged`. Bindings via `cargo test`.
2. **`syntax.rs`:** `language_for_path`. `parse_check` passa a usar.
3. **`repo_map.rs`:** walk, símbolos, score, render, cache D3, testes (fixture TS/Rust, gitignore, invalidação, secrets).
4. **`context.rs`:** orçamentos, montagem do system prompt, higiene, compactação, testes de corte e de tokens.
5. **`role.rs`:** classificar, tools, prompt de Explorer, `permits`.
6. **Loop:** montar contexto no start e a cada turno (map fresco); tools conforme o role; recusar escrita no Explorer; troca de role; métrica estimada; `ContextExhausted` só depois da compactação.
7. **Eval/CLI/UI:** campo estimado no JSON; CLI imprime tokens estimados; eventos novos são no-op na conversa (como `contextTrimmed`).
8. **Docs:** handoff (Fase 10 feita, próxima 11), README dos planos, `docs/audit/fase-10-context.md`, ADR 0008 aceita.
9. **Gate:** `bun run verify` exit 0. `cargo run -q -p cd-ai-cli -- eval --scripted` 3/3.

## Contratos

Classificação:

```text
? ou interrogativa, sem verbo de edição     → pergunta  → Explorer até o fim
≤ 12 fontes ou path existente no pedido     → trivial   → Coder + map
pedido longo / arquitetura / refator        → complexa  → Explorer → Coder
resto                                       → normal    → Coder + map
```

Explorer:

```text
tools oferecidas  = leitura + git
edit/write/run    = erro para o modelo, disco intacto
turno sem tools
  pergunta        → Verifier (unvalidated, nada foi editado)
  complexa        → RoleChanged Coder, loop continua
8 iterações       → mesma troca, se ainda for Explorer
```

Cache:

```text
GIT-style id = sha256(root canônico)
arquivo      = <data_dir>/cache/<id>/repo-map.json
invalidação  = mtime+len por path; sem watcher
```

## Critérios de pronto

- [x] `plans/020-fase-10-context-manager.md` existe; linha 020 em `plans/README.md` = DONE
- [x] Repo map tree-sitter (TS/Rust) entra no system prompt, priorizado pelo pedido
- [x] Cache invalida quando o arquivo muda (teste)
- [x] Orçamento por seção corta e registra
- [x] Higiene omite leitura obsoleta e logs enormes (teste)
- [x] Compactação continua a tarefa no lugar de `ContextExhausted`, salvo pedido que sozinho não cabe (teste)
- [x] Explorer é role read-only; escrita não executa (teste)
- [ ] Eval scripted 3/3; `estimated_prompt_tokens` no JSON
- [ ] `bun run verify` exit 0
- [ ] PR aberto

## STOP conditions

- Pedido de embeddings/RAG ou de mandar o repo para um índice externo → recuse (SPEC §16.1 / decisão 0004).
- Pedido de PoC/exploit para "provar" o corte de contexto → recuse.
- Eval scripted caiu de 3/3 porque o Explorer comeu o primeiro turno do script → a classificação `trivial` (D7/D8) tem que pegar os fixtures; ajuste a regra, não o script.
- Ollama ausente → não é STOP.

## Notas de manutenção

- Código e comentários em inglês; UI e docs em pt-BR.
- Tool results continuam marcados como não confiáveis. O resumo de compactação usa só estado do engine.
- O frontend não monta contexto: recebe eventos. O Rust decide o que entra no prompt.
- `chars/4` continua sendo a estimativa (plano 015 D10); tokenizer por modelo é Fase 13.
