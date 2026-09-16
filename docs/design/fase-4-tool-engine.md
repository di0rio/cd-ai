# Design: Fase 4 — Tool Engine, permissões, Secret Redactor e eventos

Status: **proposta de design validada por spikes** (2026-09-11). Fonte da verdade da Fase 4. A implementar pelo plano 014, citando as decisões abaixo.

## 1. Objetivo e fronteira da fatia

A Fase 4 entrega **as 6 tools mínimas, o pipeline de permissão/aprovação, classificação de comandos, Secret Redactor básico, eventos estruturados e o checkpoint mínimo** — tudo dentro de `agent-core` (decisão 0003), exposto à UI pela ponte IPC existente. É a porta da frente do agente: todo pedido do modelo atravessa `Workspace::resolve` (§20.1), permissões (§20.4) e redator (§20.6).

### O que entra na Fase 4

| Peça | Referência SPEC |
|---|---|
| `read_file`, `search`, `edit_file`, `write_file`, `list_directory`, `run_command` | §15.1 |
| Formato de edição search/replace exato + fallback | §14 |
| Classificação determinística de comandos (função pura + tabela) | §20.2 |
| Pipeline de permissão/aprovação em modo **ASK** (sem sandbox) | §20.4, decisão 0001 |
| Secret Redactor por path e por conteúdo | §20.6 |
| Eventos `ToolEvent` roteados por `Channel<T>` | §22, padrão do plano 005 |
| Checkpoint mínimo: hash de cada arquivo antes de editar | §21 (minimal) |

### O que fica para a Fase 5 e além

- Loop `Task → modelo → tools → resultado` com retry, timeout por fase, detecção de loop e estado persistente (§11.1; §34 Fase 5).
- Orquestrador como máquina de estados e classificação de tarefa (§11.2).
- Verifier e ciclo de correção (§13; Fase 8).
- Sandbox de shell e modos AUTO/FULL ACCESS (Fase 7, decisão 0001).
- Shadow repo e rollback (Fase 9).
- Streaming de terminal aberto na UI (Fase 5, quando a UI tiver a aba de terminal).

**Critério de saída (SPEC §34 Fase 4):** testes das tools e do formato de edição passando. Não se exige loop.

## 2. As 6 tools e seus schemas

Toda tool recebe um `ToolRequest` tipado e devolve um `ToolResult` estruturado. Tipos centrais em `agent-core`:

```rust
pub enum ToolRequest {
    ReadFile(ReadFileArgs),
    Search(SearchArgs),
    EditFile(EditFileArgs),
    WriteFile(WriteFileArgs),
    ListDirectory(ListDirectoryArgs),
    RunCommand(RunCommandArgs),
}

pub struct ToolResult<T> {
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<ToolError>,
    pub truncated: Option<Truncation>,
}

pub struct Truncation {
    pub shown: usize,   // o que foi entregue
    pub total: usize,   // o que existia
    pub how_to_get_more: String, // instrução textual para o modelo
}

pub enum ToolError {
    OutsideWorkspace,
    NotFound,
    NotAFile, NotADirectory,
    AlreadyExists,
    ParseFailed { detail: String, line: usize },
    AmbiguousEdit, EditNotFound,
    CompoundCommand, UnknownCommand,
    PermissionDenied { reason: String },
    SecretDenied,
    Io(String),
}
```

**Regra transversal:** nenhuma tool recebe path cru. Todo path vai a `Workspace::resolve`; o resultado canônico é o que a tool usa. Caminho inválido → `OutsideWorkspace` sem tocar o disco. `Workspace` é instanciado pela tarefa/workspace atual (plano 002).

### 2.1 read_file

```json
{ "path": "src/main.rs", "startLine": 1, "endLine": 200 }
```

- Linhas 1-indexadas; sem `startLine`/`endLine` lê até o limite de truncamento.
- **Truncamento:** `MAX_READ_LINES = 2000` e `MAX_READ_BYTES = 200 KiB`. Se estourar qualquer um, devolve a fatia lida, `truncated` com `total` (total de linhas) e `howToGetMore: "read_file com startLine/endLine"`.
- Resultado: `{ path (canônico), text, startLine, endLine, totalLines, isTruncated }`.
- Conteúdo passa pelo Secret Redactor antes de ir ao resultado (§6).
- Arquivo que é secret é barrado na permissão, não na leitura (§5).

### 2.2 search

```json
{ "query": "TODO", "regex": false, "path": "src", "maxResults": 200 }
```

- Motor **ripgrep** (crates `grep` + `ignore`), respeitando `.gitignore` (§15.1). `path` default é a raiz do workspace; resolve pelo `Workspace::resolve` e exige diretório.
- `regex: false` → parâmetro literal do `grep` (sem regex, sem worry de ReDoS do modelo).
- **Truncamento:** `maxResults` (default 200) e `MAX_TOTAL_BYTES = 64 KiB`. Devolve `matches: [{ path, lineNumber, line }]` + total de matches encontrados + amostras. Se estourou, `truncated` com `howToGetMore`.
- Cada linha de match passa pelo redator.
- **Custo/dependência aceito:** `grep`+`ignore` são as únicas formas em Rust puro de respeitar `.gitignore`. Sem elas, teríamos de reimplementar gitignore (não-objetivo). A régua é a regra de ouro (§35): o SPEC exige explicitamente "ripgrep respeitando .gitignore"; não existe std para isso. Aceito.

### 2.3 edit_file

```json
{ "path": "src/foo.ts", "oldText": "const x = 1", "newText": "const x = 2" }
```

Regras da seção 3. Resultado: `{ changedLines, removed, added, hashBefore, hashAfter }`.

### 2.4 write_file

```json
{ "path": "src/novo.ts", "content": "…", "ifExists": "error" }
```

- `ifExists` default `"error"` (arquivo novo, §14) com valores `"error" | "overwrite"`.
- `"overwrite"` permitido **só** se o arquivo atual ≤ `MAX_REWRITE_BYTES` (default 8 KiB). Acima disso, `AlreadyExists` com explicação para usar `edit_file`.
- Gravação atômica: escreve em temp no mesmo diretório → parse sintático no conteúdo → rename. Falha de parse não persiste (§3.4).

### 2.5 list_directory

```json
{ "path": "." }
```

- Sem recursão. Retorna `entries: [{ name, isDir, isFile, size }]`, ordenadas por nome (determinístico), **sem seguir symlinks** e sem atravessar para fora do workspace (`resolve` + `symlink_metadata`).
- **Truncamento:** `MAX_LIST_ENTRIES = 500`. Acima, `truncated` com total real.

### 2.6 run_command

```json
{ "argv": ["cargo", "test", "--workspace"], "cwd": "." }
```

- **`argv` (Vec<String>), nunca shell string** — minimiza bugs de quoting e evita injeção por construção. `std::process::Command` direto, sem `sh -c` (§ plano 012: decisão D6).
- **Compostos não suportados na Fase 4** (§ plano 012: D2): token contendo metacaractere de shell (`| & ; < > $ \` \n`) → rejeitado com `CompoundCommand`. Consequência natural: sem shell não existe pipe/`&&`/subshell. O modelo usa chamadas sequenciais. A classificação de compostos (§20.2) fica definida e testada agora, para quando a Fase 7 executar shells.
- `cwd` default = raiz do workspace (resolvido via `Workspace`).
- Timout por tool: default 30 s (configurável; Fase 5 terá timeout por fase). Cancelamento = matar a árvore de processos (Spike A).
- stdout e stderr capturados separados internamente (§18), juntos até `MAX_OUTPUT_BYTES = 64 KiB`; acima, `truncated` (`howToGetMore`: reexecutar com escopo menor). Ambos passam pelo redator.
- **Cliente não recebe a saída antes da decisão de permissão.** Se a permissão nega, o comando não é executado (§5).

### Ordem de implementação sugerida (dependência entre tools)

1. `list_directory` + `read_file` (só leitura, sem aprovação) — habilita Explorer.
2. `search` — depende do motor de grep.
3. `edit_file` + `write_file` (formato de edição, aprovação, parse) — o cerne de segurança.
4. `run_command` (classificação + aprovação + tree-kill).
5. Redator aplicado sobre os pontos de saída (1–4).

## 3. Formato de edição

### 3.1 Assinatura e regra de matches

`edit_file { path, oldText, newText }` aplica search/replace **exato** (§14):

| Caso | Comportamento |
|---|---|
| 1 match | aplica; evento `file.changed` com diff |
| 0 matches | falha `EditNotFound` + **trecho real mais próximo** (para o modelo não chutar) |
| ≥ 2 matches | falha `AmbiguousEdit` — pede mais contexto no bloco de busca (mesmo se os matches forem idênticos) |

### 3.2 Fallback de whitespace

Em 0 matches, tenta candidato único por **normalização** (trim de linha + colapso de espaços dentro da linha). Se o bloco normalizado aparece **uma única vez**, aplica e marca o evento `file.changed` com `fuzzy: true`. Se ainda ambíguo, mantém `EditNotFound`.

### 3.3 Trecho mais próximo (para o modelo)

Para calcular o trecho real "mais próximo" de `oldText`: normaliza as linhas do arquivo e as de `oldText`, desliza uma janela do tamanho de `oldText` sobre o arquivo e escolhe a janela com mais linhas normalizadas coincidentes. Devolve `{ startLine, endLine, context }` (a janela do arquivo). Implementável em ~40 linhas, sem dependência.

### 3.4 Parse + gravação atômica

- **Parse sintático imediato (§14, §13.1):** `tree-sitter` + gramáticas de **typescript e rust** (stack de facto do produto — decisão 0002). Linguagem detectada por extensão (`ts/tsx/js/jsx` → TS; `rs` → Rust); extensão fora da lista → parse é pulado e registrado no evento (não é erro).
- **Custo/dependência aceito:** `tree-sitter` é a forma portável de parse sem shell out. As gramáticas de TS e Rust cobrem o workspace atual do produto (dogfooding).
- **Atomicidade:** escreve em temp no mesmo diretório → parse do conteúdo → `rename`. Parse falhou → o arquivo original permanece intacto e a tool falha com `ParseFailed { line }`.

### 3.5 Diff do evento `file.changed`

**Decisão D1:** crate `similar` (diff line-level → unified diff texto). Motivo: §20.4 exige que o diálogo de aprovação e o histórico mostrem o **diff completo**, e §14 exige o diff no evento. Contagens de linhas sozinhas não produzem o diff. Custo de `similar`: crate puro, pequeno, sem deps pesadas. Alternativa ("contagens + match") seria barata mas insuficiente para a exigência de mostrar o diff.

## 4. Classificação de comandos

Função pura `classify(argv: &[String]) -> CommandClass` baseada no **basename de `argv[0]` + flags**, nunca na string inteira (para `grep | xargs rm` não virar `read`). Classes: `Read | Validate | Write | Network | Destructive | Unknown`.

### Tabela inicial

| Classe | argv[0] + regra de flags |
|---|---|
| `read` | ls, cat, head, tail, less, grep, rg, find, wc, file, stat, git status, git diff, git log, git branch, git show, git blame |
| `validate` | comandos de validação detectados no workspace (cacheados, §13.1): npm/yarn/pnpm/bun test, build, check, typecheck; cargo test/check/build; biome check |
| `write` | touch, mkdir, cp, mv, sed -i, tee, git add, git commit, git checkout -b, git branch -M, biome format --write/*rustfmt* com escrita |
| `network` | curl, wget, npm/pnpm/bun/yarn install/add/update/remove, pip install, cargo add/install, git fetch, git pull, git push, git clone, ssh, ping |
| `destructive` | `rm` com `-r`/`-rf`/`-fr`/`-R`; `git reset --hard`; `git clean -f[d]`; `git checkout --` (descarta mudanças); mkfs, shred, dd (em device), kill -9 |
| `unknown` | qualquer coisa não listada; token com metacaractere de shell (tabela §2.6) |

Nuances determinísticas:

- `rm` sem flag recursiva → `write` (arquivo único). `rm -r*` → `destructive`.
- `git reset` sem `--hard` → `write` (mexe no index); com `--hard` → `destructive`.
- `git checkout` com `--` → `destructive`; `git checkout -b` → `write`.
- Flags são lidas nos tokens que começam com `-`; uma flag composta (`-rf`) conta para ambas.
- Ordem de avaliação: `unknown` é o resultado do match; nada cai em "default permissivo".
- `read` só vale enquanto o comando só lê: `find -delete` → `destructive`; `find -exec/-execdir/-ok/-okdir` e `rg --pre` → `unknown`; `find -fprint*/-fls` e `git diff/log/show --output` → `write`; `git branch` só é `read` quando lista (renomear, copiar, criar ou apagar → `write`; `-D`/`--delete --force` → `destructive`); `git remote` só é `read` sem subcomando ou com `get-url` (`show`/`update`/`prune` → `network`).
- Flags longas contam como as curtas: `rm --recursive` e `git clean --force` → `destructive`; `git checkout -f/--force` → `destructive`.
- Validadores que reescrevem arquivos são `write`: `cargo fmt` sem `--check`, `cargo clippy --fix`, `biome check/lint --write/--fix`. Python só valida por `-m pytest`, `-m unittest` e `manage.py test`.
- `read`/`validate` exigem `argv[0]` sem separador de caminho (`./ls`, `target/debug/cargo` → `unknown`).
- Em `run_command` (não em `classify`, que não conhece o workspace): um comando `read`/`validate`/`write` cujo argumento (ou valor de `--flag=valor`, ou o caminho depois de `:` como em `HEAD:.env`) resolve fora do workspace ou num arquivo de secret vira `unknown` e pede aprovação — o sandbox só restringe escrita.
- Compostos: se qualquer token é metacaractere → classe da parte mais perigosa que o mesmo argv sugere; na dúvida, `unknown` (mantém a regra do SPEC §20.2 para quando a Fase 7 executar shells — e na Fase 4 `run_command` os recusa com `CompoundCommand` de qualquer forma).

Uso: a classe entra na tabela de permissão (§5) e no evento `command.started`/`completed`. A detecção de comandos `validate` do workspace (§13.1) é cache por workspace e fica pronta na Fase 5; na Fase 4, `validate` cobre a lista de prefixos de test/lint/typecheck/build acima.

## 5. Pipeline de permissão e aprovação

### 5.1 Quem decide

Módulo `permissions` em `agent-core` (decisão 0003). Nunca a webview, nunca alegações do modelo (§20.5). Input: `(Action, Path canônico, CommandClass, Modo)`.

### 5.2 Tabela da v1 (modo ASK; sem sandbox)

| Operação | Decisão |
|---|---|
| Ler arquivo no workspace (não secret) | **auto** (executa; nenhum prompt) |
| Editar/criar arquivo no workspace | **aprovar** |
| Deletar arquivo | **aprovar** |
| Comando `read`/`validate` | **auto** |
| Comando `write` | **aprovar** (seria auto só com sandbox) |
| Comando `network`/`destructive`/`unknown` | **aprovar** |
| Arquivo de secret (qualquer acesso) | **aprovar** — e mesmo aprovado, volta só a visão anônima (§6.4) |
| Fora do workspace | **negado** (sem aprovação; `PermissionDenied`) |

FULL ACCESS fica indisponível até a Fase 7 (decisão 0001). AUTO não existe ainda como modo configurável; a lista "auto" acima é o comportamento do modo ASK para leituras/comandos read/validate.

### 5.3 Contrato de aprovação com a UI

`approval.required` carrega o **payload exato** (§20.4): comando completo (argv) ou diff completo. Nunca resumo gerado pelo modelo.

Protótipo do payload:

```json
{
  "id": "aprv_0007",
  "taskId": "task_0042",
  "at": "2026-09-11T10:00:00.000Z",
  "action": {
    "type": "run_command",
    "argv": ["rm", "-rf", "dist"],
    "class": "destructive",
    "cwd": "/home/user/projeto"
  }
}
```

- Idempotente: pedidos idênticos em aberto reutilizam o mesmo `id` (mesma `aprv_*`); aprovar/negar responde a todas as réplicas pendentes.
- A UI responde `approval.granted { id }` ou `approval.denied { id }`; `approval.denied` pode carregar `reason` (opcional, para o histórico).
- Aprovações pendentes são limpas quando a tarefa cancela ou termina.
- Decisões auto também emitem eventos (`tool.completed` carrega a decisão de permissão implícita; nenhum prompt extra).

### 5.4 Estado

`PermissionManager` guarda `pending: HashMap<ApprovalId, PendingApproval>` e o modo atual. V1: um modo global `Ask`, por workspace depois (Fase 7). Sem allowlist por workspace ainda (lista §30 fica para Fase 7).

## 6. Secret Redactor

Módulo `redactor` em `agent-core`. Duas camadas complementares (path E conteúdo, §20.6).

### 6.1 Detecção por path

Filename/extension em lista: `.env`, `.env.*`, `*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.cer`, `*.crt`, `id_rsa`, `id_ed25519`, `id_ecdsa`, `*credentials*`, `*secret*`, `*.kdbx`. Retorna o tipo (`SecretKind::DotEnv`, `PrivateKey`, `Credentials`). A presença de secret no path **bloqueia a leitura** no pipeline de permissão antes de qualquer conteúdo ir para o contexto.

### 6.2 Detecção por conteúdo (heurísticas iniciais)

1. **Prefixo de token conhecido:** `sk-`, `sk_live_`, `ghp_`, `gho_`, `ghu_`, `xoxb-`, `xoxp-`, `AKIA[0-9A-Z]{16}`, `gsk_`, `rk_live_`, `api_`, `key-` (seguido de base64).
2. **Bloco de chave privada:** `-----BEGIN …PRIVATE KEY-----` (RSA/EC/OPENSSH/privado) até o `-----END`.
3. **JWT:** três segmentos base64url `eyJ…` (dois `.`) com payload decodificável.
4. **URL com userinfo:** `scheme://user:pass@host`.
5. **Entropia (Spike B):** média móvel de entropia de Shannon em **janela de 32 bytes** acima de **4,7 bits/char**, com **2 janelas consecutivas** (≈64 bytes contínuos) para redigir. Observação do spike: janela de 16 bytes tem teto teórico `log2(16) = 4,0 bits/char`, então qualquer limite ≥ 4,5 em janela 16 é inalcançável — o spike detectou isso e recalibrou para janela 32 (teto ~5,0).

Todas as funções são puras (`recognize(text) -> Vec<SecretSpan>`). Nenhuma dependência nova (sem regex; prefixos por `starts_with` e escaneamento por fatias).

### 6.3 Pontos de aplicação

`redact(text) -> RedactedText` (com `spans` e contagem) é **obrigatória** na fronteira de saída de: output de comando, conteúdo de `read_file`, linhas de `search`, diffs de `file.changed`, e futuramente histórico/trajetórias (§20.6, §24). Substitui por `[REDIGIDO:sk-…]` com fragmento curto do tipo detectado.

### 6.4 Arquivo de secret aprovado

Mesmo com aprovação, `read_file` em `.env` devolve **só as chaves** (`[{ key, set: bool }]`), nunca os valores (§20.6). Para chaves privadas, devolve metadados (tipo, fingerprint do PEM) sem o corpo.

## 7. Eventos

Enum Rust no estilo do plano 005 (`#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "event", content = "data")]`):

```rust
pub enum ToolEvent {
    ToolStarted     { tool: String, request_id: u64 },
    ToolCompleted   { tool: String, request_id: u64, duration_ms: u64, truncated: bool, decision: PermissionDecision },
    ToolFailed      { tool: String, request_id: u64, message: String },   // já redigido
    FileRead        { path: String, start_line: u64, line_count: u64, total_lines: u64, truncated: bool, redacted: usize },
    FileChanged     { path: String, diff: String, fuzzy: bool, hash_before: String, hash_after: String },
    CommandStarted  { id: u64, argv: Vec<String>, class: CommandClass },
    CommandCompleted{ id: u64, exit_code: Option<i32>, duration_ms: u64, truncated: bool, output_len: usize },
    ApprovalRequired{ id: String, task_id: String, action: ApprovalAction },
    ApprovalGranted { id: String },
    ApprovalDenied  { id: String, reason: Option<String> },
    CheckpointCreated { hash: String },
}
```

Regras comuns (§22): todo evento carrega `task_id` e `timestamp`; duração quando aplicável; motivo quando é decisão. `CommandClass` e `PermissionDecision` são enums Serialize (`denied`/`auto`/`granted`).

**Roteamento:** um command Tauri `run_tool(request, task_channel: Channel<ToolEvent>)` por tarefa, no padrão do `chat`/`Channel` do plano 005. A CLI consome os mesmos eventos (fila em memória).

**Reconciliação com a UI (session.ts):** o frontend mapeia `ToolEvent` → `ActivityEvent` num adaptador (só apresentação):

| ToolEvent | ActivityEvent |
|---|---|
| `FileRead` | `{ kind: "read", path }` |
| `FileChanged` | `{ kind: "edit", path, added, removed }` (derivado do diff) |
| `CommandStarted/Completed` | `{ kind: "command", command, exitCode, durationMs, output }` |
| `ToolFailed` | `{ kind: "command"/"read"/"edit", …, error }` conforme o tool |
| `ApprovalRequired` | status `waiting_approval` + prompt com payload exato |
| `ToolStarted` | fase atual (agrupamento `explore` já existe para `read`/`search`) |

O `groupActivity` existente continua servindo: leituras/buscas bem-sucedidas colapsam em bloco `explore`.

## 8. Perguntas que o design deve responder (decisões fechadas)

| # | Pergunta (do plano 012) | Decisão |
|---|---|---|
| D1 | Diff do `file.changed`: crate `similar` ou contagens? | `similar` (line-level, unified diff) — necessário para §14/§20.4 |
| D2 | `run_command` suporta compostos na v1? | **Não.** argv-only, sem shell; token com metacaractere → `CompoundCommand`. Classificação de compostos fica pronta (função pura) para a Fase 7 |
| D3 | Parse sintático imediato? | Sim, `tree-sitter` + gramáticas TS e Rust; extensão fora da lista pula com registro no evento |
| D4 | Motor do `search` | Crates `grep` + `ignore` (ripgrep), respeitando `.gitignore` |
| D5 | Checkpoint mínimo | Hash SHA-256 (`sha2`) de cada arquivo antes de editar, no evento `CheckpointCreated`; shadow repo fica para a Fase 9 |
| D6 | Forma de executar | `std::process::Command` com `argv`, `cwd` canonizado, sem shell |
| D7 | Limite de entropia do redator | Janela 32 bytes, limite 4,7 bits/char, 2 janelas consecutivas — calibrado no Spike B (seção 9) |
| D8 | Rede por comando | Classe `network` sempre aprova; nenhum comando de rede roda sem prompt na v1 |
| D9 | Aprovação por edição | `edit_file`/`write_file`/`delete` pedem aprovação com o diff exato; `read`/`validate` automáticos |

### Riscos abertos (aceitos, mitigados)

| Risco | Mitigação |
|---|---|
| Peso de dependência (tree-sitter + grep + similar + sha2) | Justificadas na régua §35; mantidas em `agent-core`; gramáticas só TS/Rust na v1 |
| ReDoS/linhas gigantes no `search` | `regex: false` por padrão; límites de bytes e resultados |
| Árvore de processos não morre | Spike A mediu na prática (recomendação abaixo) |
| Falso-positivo do redator | Spike B calibrou; perfil conservador; eventos registram `redacted` |
| Fadiga de aprovação | Agrupamento idempotente (mesmo `id` para pedido idêntico); `read`/`validate` automáticos |

## 9. Spikes

### Spike A — cancelar a árvore de processos

Medido em Linux (2026-09-11), scratch em `/tmp/opencode/spike-fase4`, **fora do workspace**.

**Contexto:** no Linux, `Command::process_group(0)` (API std, estável) faz o filho líder de um novo process group. Matar o grupo inteiro com `kill -- -PGID`.

**Receita testada:**
1. `bash -c 'sleep 1000 & sleep 1000 & wait'` lançado com `process_group(0)`.
2. Verificado: dois processos `sleep` filhos.
3. `kill -KILL -- -<pgid>` onde `pgid == pid do bash`.
4. Verificado: `ps` passa a mostrar zero `sleep` com o pid original — **filhos órfãos não sobreviveram** (o SIGKILL no grupo atingiu todos).

**Conclusão para `run_command`:** no Unix, `Command` com `process_group(0)` + `kill -- -<pid>` no cancelamento/timeout. No Windows, o equivalente é `taskkill /PID <pid> /T /F` (documentado; a validação fica na máquina de dev Windows — o design não muda). O `Job Object` da API do Windows cobre a mesma garantia e será adotado se o `taskkill` se mostrar lento demais nos testes de integração do 014.

### Spike B — heurísticas de redação (calibração da entropia)

Medido em Linux (2026-09-11), scratch rust em `/tmp/opencode/spike-fase4`, **fora do workspace**.

**Método:** função pura `shannon(window)` testada sobre 10 trechos realistas (`.env` real, token `sk-`, AWS key, log com token, chave privada PEM, JWT, comentário pt-BR, código Rust, `git clone` com userinfo, markdown pt-BR). Para cada trecho mediu-se a entropia máxima por janela deslizante e o maior run de janelas consecutivas acima do limite.

**Descoberta crítica (correção ao design):** uma janela de 16 bytes tem teto teórico `log2(16) = 4,0 bits/char`. A entropia de Shannon de uma janela de tamanho *n* com símbolos distintos nunca passa de `log2(n)`. Portanto o limite "4,5 em janela 16" era **inalcançável** — os primeiros testes rodaram com 0 disparos em amostras claramente secretas. Recalibrado para janela de 32 bytes (teto ~5,0).

**Resultados (janela 32, limite 4,7, run mínimo 2):**

| Amostra | max entropia | run | disparou? | esperado | veredito |
|---|---|---|---|---|---|
| 1. `.env` DATABASE_URL (senha curta) | 4,56 | 0 | não | não (regra URI) | OK |
| 2. token `sk-` de 24 chars em prosa | 4,94 | 6 | sim | — | redige (benéfico) |
| 3. AWS key `AKIA…` 20 chars | 4,03 | 0 | não | não (prefixo) | OK |
| 4. log com `ghp_` + URL | 4,88 | 18 | sim | — | redige (benéfico) |
| 5. chave privada PEM (base64 longo) | 5,00 | 60 | sim | sim | OK |
| 6. comentário pt-BR | 4,08 | 0 | não | não | OK |
| 7. código Rust comum | 4,08 | 0 | não | não | OK |
| 8. `git clone` com userinfo | 4,12 | 0 | não | não (regra URI) | OK |
| 9. JWT em log | 5,00 | 642 | sim | sim | OK |
| 10. markdown pt-BR longo | 3,87 | 0 | não | não | OK |

Textos limpos (6, 7, 8, 10): **0 falsos-positivos**. As amostras 2 e 4 dispararam entropia além das regras de prefixo — inofensivo (são segredos; a redação é o comportamento desejado) e útil como redundância. Texto natural pt-BR/código ficou entre 3,9 e 4,2 bits/char, folga confortável até o limite 4,7.

**Recomendação final:** implementar em ordem (1) path/kind, (2) prefixos conhecidos, (3) bloco PEM, (4) URI userinfo, (5) JWT, (6) entropia (janela 32 / limite 4,7 / run ≥ 2). Os trechos desta tabela viram fixtures dos testes do `redactor` no plano 014, incluindo um teste de regressão que garante que **janela 16 nunca vai ser usada** (tetório `log2(16)`).

---

## Conexões com o SPEC

- §14 / §15 — formato de edição e as 6 tools (seções 2–3).
- §13.1 — parse sintático imediato e comandos de validação (seções 3.4–4).
- §18 — terminal: stdout/stderr separados, tree-kill (seções 2.6, Spike A).
- §20.2 / §20.4 / §20.5 / §20.6 — classificação, permissão, injection, secrets (seções 4–6).
- §21 — checkpoint mínimo de hash (seção D5).
- §22 — eventos (seção 7) e compatibilidade com `session.ts`.
- Decisões 0001 (sem sandbox até F7; aprovação obrigatória), 0003 (tudo no Rust), 0005 (UI só apresenta).
- §34 Fase 4 — critério de saída = testes das tools e do formato de edição.