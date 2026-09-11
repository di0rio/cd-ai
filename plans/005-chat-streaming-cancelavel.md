# Plano 005: chat com o Ollama em streaming e cancelável, no core, no IPC e na CLI

> **Instruções ao executor:** siga o plano passo a passo. Rode cada comando de verificação e confirme o resultado esperado antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Ao terminar, atualize a linha deste plano em `plans/README.md`.
>
> **Drift check (rode primeiro):** confirme que `crates/agent-core/src/ollama.rs` existe, com `OllamaClient`, `DEFAULT_BASE_URL` e a validação de loopback (plano 004). Se não existir, STOP.

## Status

- **Prioridade:** P2
- **Esforço:** L
- **Risco:** MED (concorrência e cancelamento)
- **Depende de:** `plans/004-ollama-provider-e-status.md`
- **Categoria:** direction (Fase 3)
- **Planejado em:** base `158f609` + árvore de trabalho de 2026-09-11

## Por que isso importa

A saída da Fase 3 (SPEC §34) é um chat em streaming, cancelável, na UI e na CLI, com o modelo escolhido. Modelos locais são lentos (dezenas de tokens por segundo nesta máquina), então:

- **o streaming é obrigatório:** o usuário precisa ver o texto chegando;
- **o cancelamento é obrigatório:** o usuário precisa poder parar uma geração longa.

O SPEC §4 também exige que o tamanho de contexto (`num_ctx`) seja **sempre definido pelo cd-ai**, nunca pelo padrão do Ollama, que trunca o prompt em silêncio.

Este plano entrega o core, os commands IPC com `tauri::ipc::Channel` e o subcomando de CLI. A tela de chat na UI é uma task de design separada.

## Estado atual

- `crates/agent-core/src/ollama.rs` (plano 004) tem `OllamaClient::new(base_url) -> Result<Self, String>`, que só aceita loopback e usa um `reqwest::Client` com timeout de 5 s.
- `apps/cli/src/main.rs` (inteiro, hoje):

  ```rust
  use std::process::ExitCode;

  const USAGE: &str = "uso: cd-ai [--version | --help]";

  fn main() -> ExitCode {
      let arg = std::env::args().nth(1);

      match arg.as_deref() {
          Some("--version" | "-V") => { /* prints "cd-ai 0.1.0" */ }
          None | Some("--help" | "-h") => { println!("{USAGE}"); ExitCode::SUCCESS }
          Some(other) => { eprintln!("argumento desconhecido: {other}\n{USAGE}"); ExitCode::from(2) }
      }
  }
  ```

- Formato do streaming de `POST /api/chat` com `"stream": true` (documentação do Ollama): uma resposta NDJSON, com um objeto JSON por linha.
  - Os fragmentos intermediários trazem `{"message":{"role":"assistant","content":"..."},"done":false}` (modelos com raciocínio também trazem `message.thinking`).
  - A última linha traz `"done":true` e as métricas `prompt_eval_count`, `prompt_eval_duration`, `eval_count` e `eval_duration` (durações em nanossegundos).
  - Um erro chega como `{"error":"..."}`.
- Channel do Tauri 2 (documentação oficial):
  - No Rust, o command recebe `on_event: tauri::ipc::Channel<T>` com `T: Serialize` e chama `on_event.send(value)`.
  - No TS, `import { Channel, invoke } from "@tauri-apps/api/core"`, com `const channel = new Channel<T>(); channel.onmessage = (m) => ...; await invoke("cmd", { onEvent: channel })`.
  - O argumento Rust `on_event` vira `onEvent` no JS.
  - Exemplo oficial de enum de eventos:

    ```rust
    #[derive(Clone, Serialize)]
    #[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "event", content = "data")]
    enum DownloadEvent<'a> { Started { url: &'a str, download_id: usize }, Finished { download_id: usize } }
    ```

## Comandos que você vai precisar

| Propósito | Comando | Esperado |
|-----------|---------|----------|
| Testes do core | `cargo test -p agent-core chat` | `0 failed` |
| Tokio na CLI | `cargo add tokio -p cd-ai-cli --features rt-multi-thread,macros,signal` | exit 0 |
| Tokio no desktop (JoinHandle/abort) | `cargo add tokio -p cd-ai-desktop --features sync` | exit 0 |
| Gate | `bun run verify` | exit 0 |
| Teste manual da CLI | `cargo run -q -p cd-ai-cli -- chat --model qwen3:4b --ctx 8192 "diga oi em uma palavra"` | texto aparecendo aos poucos, exit 0 |

## Escopo

**Dentro do escopo:**

- `crates/agent-core/src/ollama.rs` (adicionar o chat)
- `apps/cli/src/main.rs` e `apps/cli/Cargo.toml`
- `src-tauri/src/lib.rs` e `src-tauri/Cargo.toml`
- `apps/desktop/src/lib/ipc.ts` (só tipos e wrappers)

**Fora do escopo:**

- Qualquer componente visual. A tela de chat é uma task separada de UI, com o `DESIGN.md`.
- Tool calling (Fase 4/5).
- Histórico persistido.
- Troca automática de modelo.

## Git

- Branch: `advisor/005-chat-streaming`.
- Mensagem no estilo `feat: streaming chat with cancellation`.
- Não faça push.

## Passos

### Passo 1: tipos e parser no core

Em `ollama.rs`, adicione:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatMessage { pub role: String, pub content: String }

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    /// Always explicit: the runtime default can silently truncate the prompt (SPEC §4).
    pub num_ctx: u32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "event", content = "data")]
pub enum ChatEvent {
    Token { content: String },
    Thinking { content: String },
    Done { prompt_tokens: u64, gen_tokens: u64, prompt_ms: u64, gen_ms: u64 },
    Error { message: String },
}
```

Crie também um parser de linhas **puro**, que acumula bytes e emite um evento por linha completa:

```rust
/// Splits an NDJSON byte stream into ChatEvents. Keeps partial lines between chunks.
#[derive(Default)]
pub struct NdjsonChatParser { buffer: Vec<u8> }

impl NdjsonChatParser {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<ChatEvent>;
}
```

Regras do parser:

- Divida o buffer em `\n`. Ignore linhas vazias.
- Cada linha é desserializada num struct privado `StreamLine { message: Option<Msg { content: String (default), thinking: Option<String> }>, done: bool (default), prompt_eval_count, prompt_eval_duration, eval_count, eval_duration (todos u64 com #[serde(default)]), error: Option<String> }`.
- Com `error`, emita `ChatEvent::Error`.
- Com `thinking` não vazio, emita `Thinking`.
- Com `content` não vazio, emita `Token`.
- Com `done`, emita `Done`, com as durações convertidas de ns para ms.
- Uma linha inválida emite `Error { message: "resposta inválida do Ollama" }`.

**Verificar:** `cargo build -p agent-core` sai com exit 0.

### Passo 2: testes do parser

Crie os testes:

1. `parser_emits_tokens_in_order`: duas linhas com `content` `"Ol"` e `"á"` num único chunk emitem `[Token("Ol"), Token("á")]`.
2. `parser_keeps_partial_line`: um chunk com a linha cortada no meio não emite nada; o chunk seguinte com o resto emite o `Token`.
3. `parser_handles_split_utf8`: corte um chunk no meio dos bytes de `"á"`; o resultado final é `Token("á")`.
4. `parser_emits_done_with_metrics`: `{"done":true,"prompt_eval_count":10,"prompt_eval_duration":2000000,"eval_count":5,"eval_duration":1000000}` emite `Done { prompt_tokens: 10, gen_tokens: 5, prompt_ms: 2, gen_ms: 1 }`.
5. `parser_emits_error_line`: `{"error":"model not found"}` emite `Error { message: "model not found" }`.
6. `parser_separates_thinking`: `{"message":{"content":"","thinking":"hmm"},"done":false}` emite `Thinking("hmm")`.

Garanta o caso 3 fazendo o parse **só em linhas completas**. Como o JSON só é convertido depois do `\n`, o UTF-8 cortado nunca é decodificado pela metade.

**Verificar:** `cargo test -p agent-core chat` roda com 6 testes passando.

### Passo 3: `chat_stream` no cliente

Adicione ao `OllamaClient`:

```rust
pub async fn chat_stream(&self, request: &ChatRequest, mut on_event: impl FnMut(ChatEvent) + Send) -> Result<(), String>
```

- Faça `POST {base}/api/chat` com o corpo `{"model", "messages", "stream": true, "options": {"num_ctx": request.num_ctx}}`, montado com `serde_json::json!`.
  - Para isso, mova `serde_json` de dev-dependency para dependency: `cargo add serde_json -p agent-core`.
- **Não use o timeout global de 5 s** do cliente de status: uma geração pode levar minutos. Crie `reqwest::Client::builder().connect_timeout(Duration::from_secs(5)).build()` só para o chat.
- Faça o loop `while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? { for event in parser.push(&chunk) { on_event(event) } }`.
- Um status HTTP diferente de 2xx vira `Err` com o corpo da resposta.
- **O cancelamento é feito pelo chamador, abortando a task.** Dropar a future fecha a conexão, e o Ollama para de gerar. Não crie um token de cancelamento próprio.

**Verificar:** `cargo clippy --workspace --all-targets -- -D warnings` sai com exit 0.

### Passo 4: subcomando na CLI

1. Rode `cargo add tokio -p cd-ai-cli --features rt-multi-thread,macros,signal`.
2. Troque `fn main` por `#[tokio::main] async fn main() -> ExitCode`.
3. Mantenha `--version` e `--help` iguais.
4. Adicione `chat`, com a sintaxe `cd-ai chat --model <nome> [--ctx <n>] <prompt...>`:
   - `--ctx` tem padrão `8192`.
   - O prompt é o resto dos argumentos, unidos por espaço.
   - Faça o parse à mão, no estilo do código atual. Não adicione `clap`.
5. Faça o streaming dos `Token` para o stdout, com `print!` seguido de `std::io::stdout().flush()`. Ignore os `Thinking`. No `Done`, imprima uma linha em branco e depois, no stderr, `"{gen_tokens} tokens · {tok/s:.1} tok/s"`.
6. Use `tokio::select!` entre o `chat_stream` e `tokio::signal::ctrl_c()`. No Ctrl+C, imprima `\ncancelado` no stderr e retorne `ExitCode::from(130)`.
7. Um erro vira `eprintln!` com retorno `ExitCode::from(1)`. Um modelo ausente vira o erro do Ollama, repassado como está.
8. Atualize `USAGE` para `"uso: cd-ai [--version | --help | chat --model <nome> [--ctx <n>] <prompt>]"`.

**Verificar:** `cargo run -q -p cd-ai-cli -- --help` imprime o novo `USAGE`. Com o Ollama rodando, o teste manual da tabela mostra texto em streaming.

### Passo 5: commands IPC

1. Rode `cargo add tokio -p cd-ai-desktop --features sync`.
2. Em `src-tauri/src/lib.rs`:
   - Crie o estado `struct ChatTasks(std::sync::Mutex<std::collections::HashMap<u64, tauri::async_runtime::JoinHandle<()>>>)` e um `AtomicU64` para gerar ids. Ambos ficam com `.manage(...)`.
   - O command `chat(request: ChatRequest, on_event: Channel<ChatEvent>, tasks: State<ChatTasks>, ...) -> Result<u64, String>`:
     1. Monta o `OllamaClient` como o `ollama_status` do plano 004 (mesmo `OLLAMA_HOST` e mesma validação).
     2. Gera um id.
     3. Faz `tauri::async_runtime::spawn` de uma task que roda o `chat_stream`, enviando cada evento com `on_event.send(event)`. Em `Err`, envia um `ChatEvent::Error`. No fim, remove o próprio id do mapa.
     4. Guarda o `JoinHandle` e retorna o id imediatamente.
   - O command `cancel_chat(id: u64, tasks: State<ChatTasks>) -> bool`: remove o handle e chama `.abort()`. Retorna `true` se existia.
   - Registre `chat` e `cancel_chat` no `generate_handler!`.

**Verificar:** `cargo clippy --workspace --all-targets -- -D warnings` sai com exit 0.

### Passo 6: wrappers TS

Em `ipc.ts`, adicione:

```ts
import { Channel, invoke } from "@tauri-apps/api/core";

// Mirrors agent_core::ollama::{ChatRequest, ChatEvent}.
export type ChatMessage = { role: "system" | "user" | "assistant"; content: string };
export type ChatRequest = { model: string; messages: ChatMessage[]; numCtx: number };
export type ChatEvent =
  | { event: "token"; data: { content: string } }
  | { event: "thinking"; data: { content: string } }
  | { event: "done"; data: { promptTokens: number; genTokens: number; promptMs: number; genMs: number } }
  | { event: "error"; data: { message: string } };

export async function startChat(request: ChatRequest, onEvent: (event: ChatEvent) => void): Promise<number> {
  const channel = new Channel<ChatEvent>();
  channel.onmessage = onEvent;
  return invoke<number>("chat", { request, onEvent: channel });
}

export function cancelChat(id: number): Promise<boolean> {
  return invoke<boolean>("cancel_chat", { id });
}
```

**Verificar:** `bun run verify` sai com exit 0.

## Plano de testes

- Os 6 testes de parser do Passo 2, no padrão do `mod tests` já existente em `ollama.rs`.
- Um teste de integração `#[tokio::test] #[ignore] live_chat_streams_tokens`: com `qwen3:4b`, `num_ctx` 2048 e a mensagem "responda só: ok", ele recebe pelo menos um `Token` e um `Done`.

## Critérios de pronto

- [ ] `cargo test -p agent-core` roda com `0 failed`, incluindo os 6 testes de parser
- [ ] `bun run verify` sai com exit 0
- [ ] `cargo run -q -p cd-ai-cli -- --help` mostra o subcomando `chat`
- [ ] Verificação humana: o chat na CLI faz streaming, e o Ctrl+C sai com o código 130 no meio da geração
- [ ] `git status` mostra apenas os arquivos do escopo, mais `Cargo.lock`
- [ ] A linha deste plano em `plans/README.md` está atualizada

## STOP conditions

- `reqwest::Response::chunk()` não existe na versão instalada.
- O abort da task não fecha a conexão: o Ollama continua gerando depois do `cancel_chat` (confira com `ollama ps` ou pelo uso da GPU).
- O `Channel` exige alguma permissão em `src-tauri/capabilities/default.json` para funcionar. Reporte; isso é tratado no plano 006.

## Notas de manutenção

- A futura tela de chat da UI deve:
  - chamar `cancelChat` ao desmontar e no botão de parar;
  - mostrar `Thinking` recolhido por padrão (princípio "detalhe sob demanda" do `PRODUCT.md`).
- O plano 006 precisa incluir `chat` e `cancel_chat` na allowlist.
- O plano 008 vai gerar `ChatEvent` e `ChatRequest` a partir do Rust. Atenção ao `rename_all = "camelCase"`: os campos de `ChatRequest` chegam do JS como `numCtx`.
