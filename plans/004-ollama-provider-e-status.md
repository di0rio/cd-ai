# Plano 004: o core fala com o Ollama local, e a sidebar mostra o status real

> **Instruções ao executor:** siga o plano passo a passo. Rode cada comando de verificação e confirme o resultado esperado antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Ao terminar, atualize a linha deste plano em `plans/README.md`.
>
> **Drift check (rode primeiro):** compare os trechos de "Estado atual" com os arquivos vivos. Se não baterem, STOP.

## Status

- **Prioridade:** P1
- **Esforço:** M
- **Risco:** LOW
- **Depende de:** `plans/001-verify-e-guia-de-agentes.md`
- **Categoria:** direction (Fase 3)
- **Planejado em:** base `158f609` + árvore de trabalho de 2026-09-11

## Por que isso importa

A sidebar mostra um texto fixo, "Ollama · não verificado". O app não sabe se o Ollama está rodando, quais modelos existem nem qual está carregado. Tudo o que vem depois (chat, agente, roteamento de modelo) precisa desse cliente.

Pela decisão 0004 (sem cloud), o cliente só pode falar com o **loopback**: um `OLLAMA_HOST` remoto tem que ser recusado.

## Estado atual

- `apps/desktop/src/components/sidebar.tsx`, linhas 97–103:

  ```tsx
  <div className="space-y-1 border-t border-line px-4 py-3 text-xs text-ink-faint">
    <p className="flex items-center gap-2">
      <span className="size-1.5 rounded-full bg-ink-faint" />
      Ollama · não verificado
    </p>
    <CoreStatus />
  </div>
  ```

- `apps/desktop/src/components/core-status.tsx` é o padrão de componente que chama o IPC com fallback:
  - um estado discriminado `loading | ready | unavailable`;
  - a chamada ao IPC dentro de `useEffect`;
  - `<output aria-live="polite">`.
- `crates/agent-core/Cargo.toml` tem só `serde.workspace = true` como dependência. A edição é 2024.
- Formato real da API do Ollama (documentação oficial e `scripts/bench-models.ts`):
  - `GET /api/version` → `{"version":"0.34.0"}`
  - `GET /api/tags` → `{"models":[{"name":"qwen3:4b","size":2500000000,"details":{"parameter_size":"4.0B","quantization_level":"Q4_K_M","family":"qwen3"}}]}`
  - `GET /api/ps` → `{"models":[{"name":"qwen3:4b","size":3900000000,"size_vram":3900000000}]}`
- O `src-tauri/src/lib.rs` registra commands com `generate_handler!`. O plano 003 pode já ter adicionado `AppState`, `open_workspace` e `current_workspace`: preserve tudo o que existir.

## Comandos que você vai precisar

| Propósito | Comando | Esperado |
|-----------|---------|----------|
| Dependência HTTP | `cargo add reqwest -p agent-core --no-default-features --features json` | exit 0 |
| Serde JSON (testes) | `cargo add --dev serde_json -p agent-core` | exit 0 |
| Tokio (testes) | `cargo add --dev tokio -p agent-core --features macros,rt-multi-thread` | exit 0 |
| Testes | `cargo test -p agent-core ollama` | `0 failed` |
| Integração real (opcional, com o Ollama rodando) | `cargo test -p agent-core ollama -- --ignored` | `0 failed` |
| Gate | `bun run verify` | exit 0 |

## Escopo

**Dentro do escopo:**

- `crates/agent-core/src/ollama.rs` (criar)
- `crates/agent-core/src/lib.rs` (só `pub mod ollama;`)
- `crates/agent-core/Cargo.toml`
- `src-tauri/src/lib.rs` (command `ollama_status`)
- `apps/desktop/src/lib/ipc.ts`
- `apps/desktop/src/components/ollama-status.tsx` (criar)
- `apps/desktop/src/components/sidebar.tsx` (só trocar o `<p>` fixo pelo componente)

**Fora do escopo:**

- O chat e o streaming (plano 005).
- Baixar modelos (proibido fazer automaticamente, SPEC §31).
- Features TLS no reqwest: o cliente é só HTTP local.
- A tela de configurações.

## Git

- Branch: `advisor/004-ollama-status`.
- Mensagem no estilo `feat: ollama status in core and sidebar`.
- Não faça push.

## Passos

### Passo 1: dependências

Rode os três `cargo add` da tabela.

**Verificar:** `cargo build -p agent-core` sai com exit 0.

### Passo 2: criar `crates/agent-core/src/ollama.rs`

A API pública deve ter exatamente esta forma:

```rust
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ModelInfo { pub name: String, pub size_bytes: u64, pub parameter_size: String, pub quantization: String }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LoadedModel { pub name: String, pub size_bytes: u64, pub vram_bytes: u64 }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OllamaStatus {
    pub reachable: bool,
    pub version: Option<String>,
    pub models: Vec<ModelInfo>,
    pub loaded: Vec<LoadedModel>,
    pub error: Option<String>,
}

pub struct OllamaClient { base_url: String, http: reqwest::Client }

impl OllamaClient {
    /// Refuses non-loopback hosts: the app never sends prompts off the machine (decision 0004).
    pub fn new(base_url: &str) -> Result<Self, String>;
    pub async fn status(&self) -> OllamaStatus; // never fails; unreachable => reachable: false + error
}
```

Regras de implementação:

- **`new`:**
  - Faz o parse com `reqwest::Url::parse`.
  - Aceita apenas o esquema `http` e os hosts `127.0.0.1`, `localhost` ou `::1`. Qualquer outro host retorna `Err("o Ollama precisa estar nesta máquina (loopback)")`.
  - Monta o `reqwest::Client` com `.timeout(Duration::from_secs(5))`.
  - Remove a `/` final de `base_url`.
- **Structs privadas de parse,** com `#[derive(serde::Deserialize)]`, espelhando o JSON do "Estado atual":
  - `VersionResponse { version }`;
  - `TagsResponse { models: Vec<Tag> }`, com `Tag { name, size, details: TagDetails { parameter_size, quantization_level } }`;
  - `PsResponse { models: Vec<Ps> }`, com `Ps { name, size, size_vram }`.
  - Use `#[serde(default)]` nos campos de `details` (alguns modelos não os trazem).
- **Funções puras e testáveis:**
  - `fn parse_models(json: &str) -> Result<Vec<ModelInfo>, serde_json::Error>`
  - `fn parse_loaded(json: &str) -> Result<Vec<LoadedModel>, serde_json::Error>`

  Como `serde_json` é só dev-dependency, implemente a conversão como `impl From<Tag> for ModelInfo` e `impl From<Ps> for LoadedModel`, e deixe os helpers `parse_*` dentro do `mod tests`.
- **`status()`:**
  - Faz `GET {base}/api/version`. Se falhar, retorna `OllamaStatus { reachable: false, error: Some(<mensagem>), .. }`.
  - Depois faz `GET /api/tags` e `GET /api/ps` com `.json::<TagsResponse>()` e `.json::<PsResponse>()`.
  - Se alguma dessas falhar, retorna `reachable: true` com `error: Some(...)` e as listas vazias.

### Passo 3: testes em `ollama.rs`

Crie o `mod tests` com:

1. `new_accepts_loopback`: `OllamaClient::new("http://127.0.0.1:11434")` e `OllamaClient::new("http://localhost:11434/")` são `Ok`.
2. `new_refuses_remote_host`: `OllamaClient::new("http://192.168.0.10:11434")` e `OllamaClient::new("https://example.com")` são `Err`.
3. `parses_tags`: o JSON de `/api/tags` do "Estado atual" vira um `ModelInfo` com `name == "qwen3:4b"` e `quantization == "Q4_K_M"`.
4. `parses_tags_without_details`: `{"models":[{"name":"x","size":1}]}` faz parse, com `parameter_size == ""`.
5. `parses_ps`: vira um `LoadedModel` com `vram_bytes == 3900000000`.
6. `#[tokio::test] #[ignore] live_status`: `OllamaClient::new(DEFAULT_BASE_URL).unwrap().status().await.reachable == true`.

**Verificar:** `cargo test -p agent-core ollama` roda com 5 testes passando e 1 ignorado.

### Passo 4: registrar o módulo e o command

1. Em `crates/agent-core/src/lib.rs`, adicione `pub mod ollama;`.
2. Em `src-tauri/src/lib.rs`, adicione e registre em `generate_handler!`:

```rust
#[tauri::command]
async fn ollama_status() -> agent_core::ollama::OllamaStatus {
    let base = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| agent_core::ollama::DEFAULT_BASE_URL.to_string());
    match agent_core::ollama::OllamaClient::new(&base) {
        Ok(client) => client.status().await,
        Err(error) => agent_core::ollama::OllamaStatus {
            reachable: false, version: None, models: vec![], loaded: vec![], error: Some(error),
        },
    }
}
```

Atenção: `OLLAMA_HOST` às vezes vem sem esquema (ex.: `127.0.0.1:11434`). Se não começar com `http`, prefixe `http://` antes de chamar `new`.

**Verificar:** `cargo clippy --workspace --all-targets -- -D warnings` sai com exit 0.

### Passo 5: IPC e componente

1. Em `ipc.ts`, adicione os tipos `ModelInfo`, `LoadedModel` e `OllamaStatus`.
   - Os nomes dos campos precisam bater com o Rust: `snake_case` como no struct (`size_bytes`, `vram_bytes`, `parameter_size`), porque o serde serializa os nomes dos campos como estão.
   - Adicione `export function getOllamaStatus(): Promise<OllamaStatus> { return invoke<OllamaStatus>("ollama_status"); }`.
2. Crie `apps/desktop/src/components/ollama-status.tsx` (`"use client"`), no padrão do `core-status.tsx`:
   - Busca o status ao montar, depois a cada 15 s (`setInterval`) e no evento `focus` da janela. Limpe os dois no cleanup do `useEffect`.
   - Os estados de exibição, sempre dentro de um `<p className="flex items-center gap-2">` com um ponto `size-1.5 rounded-full`:

     | Estado | Ponto | Texto |
     |---|---|---|
     | fora do Tauri (rejeição) | `bg-ink-faint` | `Ollama · indisponível fora do Tauri` |
     | `reachable: false` | `bg-bad` | `Ollama offline` (com `title={status.error}`) |
     | alcançável, nada carregado | `bg-ok` | `Ollama {version} · {models.length} modelos` |
     | com modelo carregado | `bg-ok` | `{loaded[0].name} carregado` |

3. Em `sidebar.tsx`, troque o `<p>...Ollama · não verificado</p>` (linhas 98–101) por `<OllamaStatus />`, importado de `./ollama-status`.

**Verificar:** `bun run verify` sai com exit 0.

### Passo 6: build

**Verificar:** `bun tauri build --no-bundle` sai com exit 0. Verificação humana: com o Ollama rodando, a sidebar mostra a versão e o número de modelos; com o Ollama parado, mostra "Ollama offline" em até 15 s.

## Plano de testes

Os 5 testes de unidade do Passo 3, mais 1 teste de integração `#[ignore]`. O padrão estrutural é o `mod tests` de `crates/agent-core/src/lib.rs`.

## Critérios de pronto

- [ ] `cargo test -p agent-core ollama` mostra 5 testes passando e 1 ignorado
- [ ] `bun run verify` sai com exit 0
- [ ] `bun tauri build --no-bundle` sai com exit 0
- [ ] `grep -rn "não verificado" apps/desktop/src` não retorna nenhuma linha
- [ ] `git status` mostra apenas os arquivos do escopo, mais `Cargo.lock`
- [ ] A linha deste plano em `plans/README.md` está atualizada

## STOP conditions

- O `reqwest` sem as features padrão não compila no Windows sem TLS. Reporte o erro; não adicione `rustls` ou `native-tls` sem aprovação.
- A resposta real de `/api/tags` ou `/api/ps` (teste `--ignored`) tem campos diferentes dos documentados aqui.
- A mudança exige tocar em `src-tauri/capabilities/default.json`.

## Notas de manutenção

- O plano 005 reutiliza o `OllamaClient` (base URL, timeout, validação de loopback). O streaming vai precisar de um cliente **sem** o timeout global de 5 s: use um `reqwest::Client` separado ou um timeout por request.
- O plano 008 vai gerar os tipos TS a partir destes structs. Quando isso acontecer, apague os tipos escritos à mão em `ipc.ts`.
- O polling de 15 s é barato (localhost). Quando existir um event bus (SPEC §22), prefira eventos.
