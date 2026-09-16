# Plano 020: Fase 10 — Context Manager completo

> **Instruções ao executor:** rode os passos nesta ordem e confirme cada verificação antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Uma parte só está pronta com `bun run verify` saindo com exit 0 (AGENTS.md).
>
> **Drift check (rode primeiro):** `git log -1 --oneline` deve estar em `main` com as Fases 0–9 (PR #7). Confira `crates/agent-core/src/agent/prompt.rs` (perfil + corte explícito + marcadores de não confiável), `crates/agent-core/src/agent/profile.rs`, `crates/agent-core/src/tools/edit.rs` (`parse_check` tree-sitter) e SPEC §12.1 / §16 / §34 Fase 10. Se o loop ou o perfil tiverem sumido, trate como STOP.

## Status

- **Prioridade:** P1
- **Esforço:** L
- **Risco:** MEDIUM (prompt maior no system pode inflar tokens; Explorer como turno extra de LLM quebraria o eval scripted)
- **Depende de:** 019 (Fase 9 no `main`)
- **Categoria:** feature (SPEC §12.1, §16, §34 Fase 10)
- **Planejado em:** commit `81fe812` (`main`), 2026-09-16

## Por que isso importa

Até a Fase 9 o contexto é perfil determinístico + corte visível (`ContextTrimmed` / `ContextExhausted`) + marcação de tool result não confiável. Não há mapa do repositório, orçamento por seção, cache invalidável nem o role Explorer. O modelo descobre o projeto à custa de `list_directory` / `read_file`, e a janela enche com resultados obsoletos.

A Fase 10 fecha SPEC §16: o Core monta o contexto (nunca o frontend), prioriza o que a tarefa precisa, e o Explorer existe como role (prompt + tools).

## Estado atual (fatos)

- System prompt curto + `WorkspaceProfile::render()` (raiz, linguagens, validação, regras). Sem repo map.
- Orçamento: `chars/4`; acima de 75% omitir tool results antigos; acima de 90% `ContextExhausted`. Sem frações por seção.
- Sem cache de perfil/mapa. Sem higiene além do omitir resultados antigos.
- Um único prompt e as 10 tools em todo turno. Verifier é código, não role de LLM (018 D1).
- Eval scripted 3/3; `promptTokens` no relatório scripted é 0 (`ScriptedModel` não estima). A suíte não cresce (016 D3).

## Decisões deste plano (não reabrir sem ADR)

| # | Decisão | Motivo |
|---|---|---|
| D1 | **Context Manager é módulo em `agent-core`** (`agent/context.rs`, `agent/repo_map.rs`, `agent/role.rs`). Sem crate novo, sem embeddings/RAG. | Decisão 0003. SPEC §16: “Sem embeddings/RAG na v1”. |
| D2 | **Repo map = walk `ignore` + tree-sitter** (reusa as linguagens de `parse_check`: TS/JS/TSX/Rust). Por arquivo: path + assinaturas de topo (primeira linha do nó, cap). Prioridade: sobreposição de tokens do pedido com path/símbolos; testes ao lado sobem junto. | SPEC §16.1. Tree-sitter já é dependência da Fase 4. |
| D3 | **Cache em `<data_dir>/cache/<sha256 do root>/repo-map.json`.** Invalidar por mtime+tamanho do arquivo, nunca por TTL. Reparse só o que mudou; entradas de arquivos apagados saem. Perfil cacheado com fingerprint dos marcadores da raiz. Sem watcher (SPEC permite hash/mtime). | SPEC §16.4. O mesmo `data_dir` do store. |
| D4 | **Orçamento por seção como fração de `num_ctx`:** role 8%, perfil/regras 8%, repo map 12%, notas do Explorer 8%, histórico 44%, resposta reservada 20%. Corte explícito (`[truncated]`) e registrado. Skills = 0% (Fase 11). Estimativa continua `chars/4`. | SPEC §16.2. |
| D5 | **Higiene antes do trim de 75%:** resultado obsoleto do mesmo path (read/edit/write mais novo ganha); log enorme de `run_command` reduzido às linhas decisivas (erro/arquivo/linha + cauda). Duplicata idêntica consecutiva some. Skills não usadas não existem ainda. | SPEC §16.3. |
| D6 | **Compactação determinística antes de `ContextExhausted` (ADR 0008).** Se depois da higiene+trim a janela ainda passa de 80%, o miolo vira um resumo estruturado do `TaskState` (pedido, ficheiros, comandos, próximo passo) e o loop segue. `ContextExhausted` só se o system+pedido sozinhos não cabem. Sem LLM para resumir (não gasta tokens de geração). | ADR 0008. Eval não pode ganhar um turno extra. |
| D7 | **Explorer é role (prompt + tools), não um turno extra de LLM nas tarefas de código.** Classificação determinística: `pergunta` (leitura/pergunta, sem verbo de edição/execução) → prompt Explorer e tools só de leitura (`read_file`, `list_directory`, `search`, `git_*`). `trivial` / `normal` / `complexa` → Coder com todas as tools, **briefing do Explorer já injetado** (mapa ranqueado + testes relacionados + comandos de validação). Default = Coder. Sem turno LLM de exploração no eval. | SPEC §11.2 / §12.1. Um turno extra quebraria os scripts de 4 turnos. |
| D8 | **Tokens no eval scripted:** quando o modelo devolve `prompt_tokens = 0`, o loop grava `estimate_tokens(messages)`. A prova de “menos tokens por tarefa” é teste no Core (workspace gordo: mapa ranqueado vs dump da árvore) mais o número no relatório scripted. A suíte não cresce. | Saída da Fase 10. Ollama ausente não é STOP. |
| D9 | **Raiz deixa de ir no prompt quando há repo map.** `Root entries:` era orientação; o mapa a substitui. Perfil no prompt = linguagens, package manager, validação, regras. | Menos tokens no caminho feliz. |
| D10 | **Conteúdo de tool continua marcado como não confiável.** Higiene e compactação não removem os marcadores dos resultados que ficam. | SPEC §20.5. |

## Escopo

**Dentro:** repo map tree-sitter; cache mtime; orçamento por seção; higiene; compactação determinística; role Explorer (classificação + prompt + tools); briefing injetado; métrica de tokens no scripted; testes; docs/handoff.

**Fora:** Fase 11+ (skills, router, release). Watcher de filesystem. Embeddings. Turno LLM de Explorer em tarefa `normal`. Suíte de eval maior. Payloads ofensivos. Mudar o Verifier.

## Passos

1. **`role.rs`:** `Role::{Explorer, Coder}`; `classify`; prompts; `tool_names`. Testes da classificação (eval soma/greet/dobro → Coder; “explique X” → Explorer).
2. **`edit.rs`:** `language_for_path` compartilhado com o mapa.
3. **`repo_map.rs`:** walk, símbolos, rank, cache em disco, testes (símbolos TS/Rust, gitignore, secret não aberto, invalidação mtime, rank).
4. **`context.rs`:** seções + orçamento; `assemble`; higiene; compactação; teste de menos tokens vs dump.
5. **`prompt.rs` / `runner.rs`:** montar o system a partir do assemble; role escolhe tools; estimar tokens se o modelo reporta 0; compactar antes de exhausted.
6. **Docs:** handoff (Fase 10 feita, próxima 11), README dos planos, `docs/audit/fase-10-context-manager.md`, ADR 0008 aceito com a leitura da compactação determinística.
7. **Gate:** `bun run verify` exit 0. `cargo run -q -p cd-ai-cli -- eval --scripted` 3/3.

## Contratos

Classificação (puramente léxica):

```text
verbo de edição/execução (corrija, crie, escreva, rode, faça, …)
  + path no pedido     → trivial  → Coder
  + senão              → normal   → Coder  (complexa: pedido longo / vários paths / “refator”)
leitura/pergunta sem verbo de edição → pergunta → Explorer
qualquer outra coisa                 → normal   → Coder
```

Repo map (texto no prompt):

```text
Repo map (ranked for this task; not the whole project):
src/soma.ts: export function soma(a: number, b: number)
src/soma.test.ts: test("soma devolve a soma dos dois números"
[repo map truncated]
```

Explorer (tools): `read_file`, `list_directory`, `search`, `git_status`, `git_diff`, `git_log`, `git_branch`.
Coder: as 10 tools atuais. Uma chamada de escrita no Explorer volta como erro ao modelo, não executa.

## Critérios de pronto

- [x] `plans/020-fase-10-context-manager.md` existe; linha 020 em `plans/README.md` = DONE
- [x] Repo map tree-sitter no system prompt, ranqueado, cache invalidado por mtime
- [x] Orçamento por seção com corte explícito
- [x] Higiene: obsoleto / log enorme / duplicata
- [x] Explorer como role (prompt + tools) nas perguntas; briefing nas tarefas de código
- [ ] Eval scripted 3/3 sem regressão; tokens documentados (teste de redução no Core)
- [ ] `bun run verify` exit 0
- [ ] PR aberto

## STOP conditions

- Pedido de embeddings/RAG “só nesta fase” → recuse; SPEC §16.1.
- Pedido de PoC/exploit para “provar” o mapa → recuse.
- Eval scripted caiu de 3/3 porque o Explorer consumiu o primeiro turno do script → o briefing é determinístico (D7), não desligue o mapa.
- Ollama ausente → não é STOP: cubra com `ScriptedModel` e documente o comando ao vivo.

## Notas de manutenção

- Código e comentários em inglês; UI e docs em pt-BR.
- O frontend não monta contexto: só mostra `promptTokens` / `contextUsed` que o Rust já emitiu.
- Walk e parse passam por `Workspace` + redator; arquivo de secret não é aberto para o mapa.
- Compactação e trim continuam só em memória, como o `ContextTrimmed` da Fase 5 (o transcript em disco é append-only).
