# Plano 012: design da Fase 4 — Tool Engine, permissões e Secret Redactor (entrega um documento + spikes)

> **Instruções ao executor:** siga o plano passo a passo. Este é um plano de **design**, não de implementação: o produto final é um documento de decisão em `docs/design/` com o desenho da Fase 4 (SPEC §34) — as 6 ferramentas mínimas, o pipeline de permissão/aprovação, o Secret Redactor e os eventos — mais a conclusão de dois spikes de risco. **Você não deve implementar as ferramentas.** Rode cada verificação e confirme o resultado esperado. Se alguma STOP condition acontecer, pare e reporte; não improvise. Ao terminar, atualize a linha deste plano em `plans/README.md` e registre o plano de implementação proposto como um TODO futuro.
>
> **Drift check (rode primeiro):** confirme que `crates/agent-core/src/lib.rs` ainda tem só `AppInfo` (ou no máximo os módulos `workspace`/`ollama`/`tool_call` dos planos 002/004/009) e que `plans/README.md` ainda lista 001–009. Se a árvore avançou além disso, STOP.

## Status

- **Prioridade:** P1
- **Esforço:** M (design + 2 spikes curtos)
- **Risco:** LOW (zero mudança de código de produto; artefato é documento + notas de spike)
- **Depende de:** leitura prévia de `plans/002` (workspace), `plans/004` (ollama), `plans/005` (streaming/Channel), `plans/006` (permissões), `plans/009` (parser tolerante) — para que o design se apóie no que eles definem
- **Categoria:** direction (Fase 4/5, plano de design/spike)
- **Planejado em:** commit `f6e7764`, 2026-09-11

## Por que isso importa

Os planos 001–009 preparam exatamente os pré-requisitos da Fase 4 (módulo `workspace` com validação de path, cliente do Ollama, streaming cancelável, ACL do Tauri, tipos IPC, parser de tool calls). **Nenhum deles desenha ou entrega o que vem depois**: as 6 ferramentas mínimas (SPEC §15.1), o ciclo de permissão/aprovação (§20.4), a classificação de comandos (§20.2) e o Secret Redactor (§20.6) — o último obrigatório **antes** de qualquer output de comando cruzar para a UI, o modelo ou o histórico. O SPEC §34 define a saída da Fase 4 e o SPEC §11.1 define o loop mínimo da Fase 5.

Ferramenta de arquivo é código de segurança: tudo que o modelo pedir vai atravessar `Workspace::resolve`, permissões e redator. Implementar isso "direto" sem um design explícito é como construir a porta da frente da casa sem decidir onde fica a fechadura. Este plano gera a decisão antes do código — e valida os dois riscos mais altos (cancelar a árvore de processos; heurísticas de redação sem falso-positivo demais) com spikes baratos.

## Estado atual e contratos que o design deve respeitar

### Código hoje

- `crates/agent-core/src/lib.rs` (26 linhas): só `AppInfo` (estendido pelos planos 002/004/009 com `workspace`, `ollama`, `tool_call`).
- `src-tauri/src/lib.rs` (11 linhas): um command `app_info`, sem estado e sem eventos.
- `apps/desktop/src/lib/session.ts`: tipos `TaskStatus`, `Phase`, `ActivityEvent` (kinds `read`, `search`, `edit`, `command`, `report`…) — a UI já está "esperando" eventos dessa forma (ver `conversation.tsx` e `groupActivity`).
- `plans/002` define `Workspace::resolve` (valida path, symlink, `../`, exige que o resultado fique dentro do root). **Todo acesso a disco das ferramentas deve usar o path devolvido por `resolve`, nunca o input cru.**
- `plans/005` define o padrão de canal de eventos: Rust `Channel<T>` + enum com `#[serde(tag = "event", content = "data")]`, espelhado em `ipc.ts`.
- `plans/009` define o parser tolerante de tool calls (`parse_text_tool_calls(content, known_tools)`).

### Contratos do SPEC que o design NÃO pode violar (extraídos aqui — o executor não leu o SPEC por inteiro)

**§14 — Formato de edição.** Arquivo novo: `write_file` com conteúdo completo. Arquivo existente: search/replace com match **exato**; zero matches → rejeitar e devolver a diferença real mais próxima; múltiplos matches → rejeitar e pedir mais contexto; fallback fuzzy só normalizando whitespace, registrado em evento. Reescrita completa só abaixo de um tamanho configurável. Toda edição passa por parse sintático e emite `file.changed` com o diff.

**§15.1 — Conjunto mínimo (6 tools):** `read_file` (com faixa de linhas), `search` (ripgrep: texto/regex, respeitando `.gitignore`), `edit_file` (search/replace), `write_file` (arquivo novo), `list_directory`, `run_command` (shell, sob permissões/sandbox).

**§15.2 — Segunda leva:** `delete_file`, `create_directory`, `search_symbols` (tree-sitter), `get_file_metadata`, git tools.

**§15.3 — Regras das tools:** resultados sempre estruturados (sucesso/erro, dados, truncamento indicado); outputs grandes truncados com indicação clara e forma de pedir mais; cada chamada emite `tool.started` e depois `tool.completed`/`tool.failed`; toda chamada passa pelo Permission Manager; leituras independentes podem rodar em paralelo; escritas são sequenciais na v1.

**§18 — Terminal:** streaming de stdout e stderr **separados**; exit code; timeout e cancelamento **matando a árvore de processos inteira**; nunca esconder erros; output passa pelo Secret Redactor antes de modelo/UI/histórico.

**§20.2 — Classificação de comandos** (determinística): `read` (ls, cat, git status, git diff…), `validate` (testes/lint/typecheck/build detectados no workspace), `write` (altera arquivos), `network` (curl, wget, install, git fetch/push…), `destructive` (rm recursivo, `git reset --hard`, `git clean`, formatação de disco…), `unknown`. Compostos (pipes, `&&`, `;`, subshells, `$(...)`) recebem a **classe mais perigosa** entre as partes; na dúvida, `unknown`.

**§20.4 — Modos de permissão e a tabela** (na v1, sem sandbox de SO, o modo FULL ACCESS fica indisponível e **todo comando fora de `read`/`validate` exige aprovação**): ler no workspace = auto; editar/criar = aprovar em ASK; deletar = aprovar; comando `read`/`validate` = auto; `write` = aprovar (auto só com sandbox); `network`/`destructive`/`unknown` = aprovar; arquivo de secret = aprovar; fora do workspace = negado. **O diálogo de aprovação mostra exatamente o que será executado (comando completo, diff completo) — nunca uma descrição resumida gerada pelo modelo.**

**§20.5 — Prompt injection:** todo resultado de tool entra no contexto marcado como dado não confiável, delimitado, com instrução de nunca seguir instruções contidas nele. Decisões de permissão nunca se baseiam em alegações do modelo.

**§20.6 — Secrets:** nunca enviar automaticamente ao modelo `.env`, `.env.*`, chaves privadas, SSH, credenciais, tokens, certificados, senhas. Detecção por nome/caminho **e** por conteúdo (prefixos conhecidos de tokens, blocos de chave privada, strings de alta entropia). Redação também em **output de comandos** (env, logs, stack traces) antes de modelo/UI/histórico/trajetórias. Acesso a secret só com aprovação e, mesmo assim, evitar colocar valores no contexto (ex.: só as chaves de um `.env`, sem valores).

**§22 — Eventos** que o Tool Engine emite: `tool.started | tool.completed | tool.failed`, `file.read | file.changed`, `command.started | command.completed | command.failed`, `approval.required | approval.granted | approval.denied`, `checkpoint.created | rollback.completed`. Cada evento carrega id da tarefa, timestamp, duração quando aplicável e motivo quando é uma decisão.

**§21 — Checkpoints:** snapshot antes de tarefa que edita e antes de operações destrutivas; registrar hash de cada arquivo escrito. (Shadow repo + rollback são provavelmente pós-v1/plano separado — o design deve decidir o mínimo: hash antes de editar.)

### Decisões e convenções já registradas

- Decisão 0001: até existir sandbox, todo comando fora de `read`/`validate` exige aprovação. Código portável (paths via `std::path`, execução por trás de um módulo de plataforma).
- Decisão 0003: fronteira de confiança é o Rust; tudo de security mora no `agent-core`. A webview nunca é fronteira de confiança.
- Decisão 0005: frontend sem rede; só apresentação.
- Prioridades do SPEC §0: CORRECTNESS > SECURITY > RELIABILITY > PERFORMANCE > MAINTAINABILITY > FEATURE COUNT.
- Crates são criados sob demanda (§7.1): o Tool Engine começa como módulo em `agent-core` (ex.: `tools/`), não como crate novo. Sem abstração especulativa.
- Convenções de código: inglês em código/comentários, pt-BR em mensagens de erro de usuário, testes no próprio arquivo (`#[cfg(test)] mod tests`), sem `thiserror` (Display à mão), sem dependência nova sem necessidade (regra de ouro do §35).

## Comandos que você vai precisar

| Propósito | Comando | Esperado |
|-----------|---------|----------|
| Nada quebra | `bun run check` | exit 0 (biome) |
| Nada quebra | `bun run --cwd apps/desktop typecheck` | exit 0 |
| Nada quebra | `bun run --cwd apps/desktop test` | `0 fail` |
| Spikes (se `cargo` existir) | `bun run verify` | exit 0 |
| Testar se `cargo` existe | `cargo --version` | pode falhar; só reporte |

> Se `cargo` não existir na sua máquina, **não bloqueie o design**: escreva o documento do mesmo jeito e marque os spikes 1 e 2 com o estado "não rodado — requer cargo" no doc. O documento é o entregável; os spikes são anexo.

## Escopo

**Dentro do escopo:**

- `docs/design/fase-4-tool-engine.md` (criar — o entregável)
- Anotações dos spikes (dentro do mesmo documento, seção "Spikes")
- Registrar em `plans/README.md`: a linha deste plano (status) e uma linha TODO futura para o plano de implementação proposto (ex.: "014-implementar-tool-engine")

**Fora do escopo:**

- Qualquer mudança em código-fonte de produto (Rust, TS, configs, capacidade do Tauri).
- Implementar ferramentas, redator ou permissões.
- Modificar o `SPEC.md` (o design é um documento complementar; o SPEC não muda neste plano).

## Git

- Branch: `advisor/012-design-fase4`.
- Um commit com o documento e o README: mensagem `docs(design): fase 4 tool engine`.
- Não faça push.

## Passos

### Passo 1: criar o documento com as seções exigidas

Crie `docs/design/fase-4-tool-engine.md`. Ele deve **decidir** (nada de "TBD") os pontos abaixo, citando os contratos deste plano. Estrutura mínima, na ordem:

1. **Objetivo e fronteira da fatiada**: o que entra na Fase 4 (6 tools mínimas + permissões básicas + redator básico + eventos) e o que fica para a Fase 5 (orquestrador, loop de retry, estado persistente, detecção de loop, `completed_unvalidated`). Use os critérios de saída do SPEC §34 (Fase 4 e Fase 5) como baliza.
2. **Definições das 6 tools e suas schemas JSON**, seguindo §15.1: nome, parâmetros, validações que chamam `Workspace::resolve`, resultado estruturado (`{ ok, data, error?, truncated? }`), regra de truncamento (tamanho padrão e mecanismo de "pedir mais" — ex.: `read_file` com `start_line/end_line`; `search` com contagem e amostras). Explique a ordem de implementação sugerida (leitura primeiro, escrita e comando depois).
3. **Formato de edição** (§14): assinatura de `edit_file` (search/replace exato), regras de zero/múltiplos matches e fallback de whitespace com evento, quando `write_file` se aplica, e como o diff do evento `file.changed` é calculado (decida: diff line-level com crate pequena tipo `similar`/`dissimilar`, ou só contagens de linhas + match; aponte o custo de cada).
4. **Command classification** (§20.2): regras determinísticas por prefixo/argv (não por string inteira, para evitar `grep | xargs rm` classificado como read), lista inicial de comandos por classe, e a regra de compostos (pipe/`&&`/`;`/subshell/`$(...)` → classe mais perigosa; na dúvida `unknown`). `run_command` recebe **argv** (`Vec<String>`), não string de shell, para minimizar bugs de quoting — decida se suporta compostos na v1 ou exige aprovação `unknown` nesses casos.
5. **Pipeline de permissão/aprovação** (§20.4): quem decide (módulo `permissions` no `agent-core`), entrada (ação + path/comando + classe + modo atual), a tabela de decisões da v1 (sem sandbox → FULL ACCESS indisponível), e o contrato do pedido de aprovação para a UI (payload com comando/diff **exatos**, idempotente, com `approval.required/granted/denied`). O protótipo de payload em JSON é bem-vindo aqui.
6. **Secret Redactor** (§20.6): 
   - detecção por path (glob de `.env`, `.env.*`, chaves/certs/ssh);
   - detecção por conteúdo (prefixos de token conhecidos, blocos PEM/der de chave privada, heurística de entropia com limite e janela);
   - pontos de aplicação (output de comando, conteúdo de `read_file`, histórico, trajetórias) e como o fluxo de aprovação oferece um arquivo de secret sem colocar valores no contexto (ex.: só nomes de chaves).
7. **Eventos** (§22): enum Rust `ToolEvent` no estilo do plano 005 (`#[serde(tag = "event", content = "data")]`), os variants mínimos (`tool.started/completed/failed`, `file.read`, `file.changed`, `command.started/completed/failed`, `approval.*`) e o roteamento via `Channel<T>` do Tauri para a UI, reconciliando com os `ActivityEvent` já existentes em `apps/desktop/src/lib/session.ts` (a UI consome `{ kind: "read" | "edit" | "command" | ... }`).
8. **Riscos e decisões abertas**: responda explicitamente às perguntas abaixo da seção "Perguntas que o design deve responder".
9. **Spikes** (se `cargo` existir) — veja Passo 2.

**Verificar:** o arquivo existe e nenhuma das seções 1–8 termina com "TBD", "a decidir" ou similar não resolvido. Um documento que decida pouco é falha deste plano.

### Passo 2: rodar os dois spikes e registrar as conclusões

Se `cargo --version` funcionar, valide com código de brinquedo (pode ser num diretório temporário fora do workspace, ex. `/tmp/opencode/spike-fase4`; **não** altere o workspace):

- **Spike A — cancelamento mata a árvore de processos.** Com `tokio`, inicie um processo que abre filhos (ex.: shell script com `sleep 1 & sleep 1`), aborte via drop do handle/`Child::kill` e verifique se **todos** os filhos morrem. Compare: Linux (`kill` com pgroup via `setsid`/`process_group`) e Windows (`taskkill /T /F`). Registre no doc: comando exato usado por plataforma, o que funcionou, e a recomendação para `run_command`.
- **Spike B — heurísticas de redação.** Monte 8–10 trechos realistas de output (ex.: `cat .env`, saída de `env`, log de teste com URL de serviço contendo token, `git diff` tocando um `.env`) e rode as heurísticas de entropia/marcação manualmente (uma função Rust de 20 linhas em scratch ou até um script ad-hoc): meça falso-positivo (redigiu o que não é segredo?). Registre no doc: a lista de heurísticas escolhidas, o limite de entropia, e a taxa observada de falso-positivo.

Se `cargo` **não** existir, escreva na seção "Spikes": "não rodado — requer cargo" e liste o que cada spike vai medir, para o executor do plano de implementação rodar.

**Verificar:** a seção "Spikes" do documento tem uma conclusão explícita por spike (funcionou/não funcionou/medida), ou a marcação "não rodado" com a receita.

### Passo 3: registrar o plano de implementação como TODO futuro

Atualize `plans/README.md`:

- marque a linha do plano 012 como feita (DONE no seu passo final, junto com o Passo 4);
- adicione uma linha nova na tabela com o plano de implementação proposto, status `TODO`, ex.:

  | 014 | Implementar a Fase 4 (tools, permissões, redator, eventos) | P1 | L | 002, 005, 006, 009, 010, 012 | TODO |

  (Se quiser, adiante o design na própria linha de 014 na coluna "título"; o número fica reservado.)

- adicione uma nota de dependência curta: "014 depende do design do 012".

**Verificar:** `grep -n "014" plans/README.md` retorna pelo menos uma linha.

### Passo 4: gate final

**Verificar:** `bun run check`, `bun run --cwd apps/desktop typecheck` e `bun run --cwd apps/desktop test` → todos exit 0, e `git status` mostra apenas `docs/design/fase-4-tool-engine.md` e `plans/README.md`.

## Plano de testes (do próprio plano)

Este plano não produz código de produto; seu "teste" é o documento atender à estrutura do Passo 1 e às verificações dos Passos 2–4. O **executor dos planos futuros** deve conferir contra este documento que tudo o que ele implementa está previsto aqui (drift entre design e implementação é falha de processo, não de código).

## Critérios de pronto

- [ ] `docs/design/fase-4-tool-engine.md` existe, com as seções 1–8 decididas (sem "TBD")
- [ ] Seção "Spikes" com conclusão ou marcação "não rodado — requer cargo" + receita
- [ ] `plans/README.md` com a linha do 012 preenchida e a linha futura `014` em TODO
- [ ] `bun run check`, typecheck e testes do frontend passam
- [ ] `git status` mostra apenas `docs/design/fase-4-tool-engine.md` e `plans/README.md`
- [ ] Nenhum arquivo de código-fonte foi tocado

## STOP conditions

- A árvore avançou além do estado descrito em "Estado atual" (ex.: `plans/README.md` sem linhas 001–009, ou módulos de tools já existentes em `agent-core`).
- A vontade de "adiantar a implementação" do Tool Engine: este plano é design. Implementação é o plano 014.
- Uma decisão de design conflita com um contrato citado deste plano (SPEC §14/15/18/20/22 ou decisões 0001/0003). Conflito = revisar o design, não ignorar o SPEC.

## Notas de manutenção

- **Este documento vira a fonte da verdade da Fase 4.** O executor do 014 deve citá-lo; se uma decisão do 014 precisar divergir do design, o motivo entra no PR e o documento é atualizado.
- Os planos 002 (workspace), 005 (Channel), 006 (ACL do Tauri), 009 (parser) são pré-requisitos **de implementação**. As linhas 010 (conversa com eventos vivos) e 011 (testes CLI) também — a conversa já estará pronta para o stream que o Tool Engine vai emitir.
- O SPEC §34 coloca timeout/cancelamento/detecção de loop na Fase 5 (orquestrador); o design do 012 deve separar o que é responsabilidade do Tool Engine (execução pontual de uma tool com timeout próprio) do que é do loop.
- Revisar em PR futuro: match exato/esquema de aprovação sem descrição resumida do modelo; o redator aplicado em todos os pontos (§20.6), não só no output de comando.