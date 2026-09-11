# Plano 006: a webview só pode chamar o que o app libera explicitamente

> **Instruções ao executor:** siga o plano passo a passo. Rode cada comando de verificação e confirme o resultado esperado antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Ao terminar, atualize a linha deste plano em `plans/README.md`.
>
> **Drift check (rode primeiro):** leia `src-tauri/src/lib.rs` e anote **todos** os nomes dentro de `tauri::generate_handler![...]`. Essa lista é a entrada do Passo 1. Compare também `src-tauri/build.rs` e `src-tauri/capabilities/default.json` com os trechos abaixo. Se não baterem, STOP.

## Status

- **Prioridade:** P2
- **Esforço:** S
- **Risco:** MED (pode bloquear uma funcionalidade legítima até achar a permissão certa)
- **Depende de:** `plans/003-abrir-workspace-pela-ui.md`, `plans/004-ollama-provider-e-status.md` e `plans/005-chat-streaming-cancelavel.md`
- **Categoria:** security
- **Planejado em:** base `158f609` + árvore de trabalho de 2026-09-11

## Por que isso importa

A webview vai renderizar conteúdo vindo de repositórios de terceiros: nomes de arquivo, saídas de comando, texto gerado pelo modelo. Se algum dia um XSS escapar, o estrago fica limitado ao que a webview tem permissão de chamar.

Hoje ela tem duas liberações amplas:

1. **`core:default`:** o conjunto padrão de **todos** os plugins do core (janela, menu, tray, path, recursos, imagem, eventos etc.). O app não usa quase nada disso.
2. **Commands do app sem ACL:** pela documentação do Tauri 2, todo command registrado com `invoke_handler` fica liberado para todas as janelas, a menos que `build.rs` use `AppManifest::commands`.

Aplicar menor privilégio (SPEC §20 e decisão 0003) antes de existirem ferramentas que tocam o disco é barato. Depois fica caro.

## Estado atual

- `src-tauri/build.rs`:

  ```rust
  fn main() {
      tauri_build::build()
  }
  ```

- `src-tauri/capabilities/default.json`:

  ```json
  {
    "$schema": "../gen/schemas/desktop-schema.json",
    "identifier": "default",
    "description": "enables the default permissions",
    "windows": ["main"],
    "permissions": ["core:default"]
  }
  ```

- Commands esperados depois dos planos 003–005 (confirme no drift check):
  - `app_info`
  - `open_workspace`
  - `current_workspace`
  - `ollama_status`
  - `chat`
  - `cancel_chat`
- A documentação oficial (Tauri 2, "Capabilities") mostra como colocar os commands do app sob ACL:

  ```rust
  fn main() {
      tauri_build::try_build(
          tauri_build::Attributes::new()
              .app_manifest(tauri_build::AppManifest::new().commands(&["your_command"])),
      )
      .unwrap();
  }
  ```

  Com isso, o Tauri gera uma permissão `allow-<command-em-kebab-case>` para cada command (por exemplo `allow-app-info`), e só os commands listados na capability ficam acessíveis.
- O frontend chama apenas `invoke(...)` e `new Channel()`, ambos de `@tauri-apps/api/core`. Nenhum plugin é chamado pelo JS (o diálogo de pasta é aberto pelo Rust no plano 003).

## Comandos que você vai precisar

| Propósito | Comando | Esperado |
|-----------|---------|----------|
| Gate | `bun run verify` | exit 0 |
| Build do app | `bun tauri build --no-bundle` | `Built application at:` |
| Nomes gerados | `Select-String -Path src-tauri/gen/schemas/desktop-schema.json -Pattern 'allow-app-info'` | pelo menos uma linha |

## Escopo

**Dentro do escopo:**

- `src-tauri/build.rs`
- `src-tauri/capabilities/default.json`

**Fora do escopo:**

- A CSP em `src-tauri/tauri.conf.json` (tema separado; ver plano 007).
- A lógica dos commands.
- Qualquer arquivo do frontend.

## Git

- Branch: `advisor/006-least-privilege`.
- Mensagem no estilo `fix(security): least-privilege capabilities`.
- Não faça push.

## Passos

### Passo 1: colocar os commands do app sob ACL

Troque `src-tauri/build.rs` por:

```rust
fn main() {
    // Every app command must be listed here and allowed in capabilities/default.json;
    // anything else is unreachable from the webview.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "app_info",
            "open_workspace",
            "current_workspace",
            "ollama_status",
            "chat",
            "cancel_chat",
        ]),
    ))
    .expect("failed to run tauri-build");
}
```

Use exatamente a lista de commands anotada no drift check.

**Verificar:** `cargo build -p cd-ai-desktop` sai com exit 0, e o `Select-String` da tabela encontra `allow-app-info`.

### Passo 2: capability mínima

Troque `src-tauri/capabilities/default.json` por:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Main window: only the app's own commands",
  "windows": ["main"],
  "permissions": [
    "allow-app-info",
    "allow-open-workspace",
    "allow-current-workspace",
    "allow-ollama-status",
    "allow-chat",
    "allow-cancel-chat"
  ]
}
```

**Verificar:** `bun run verify` e `bun tauri build --no-bundle` saem com exit 0.

### Passo 3: verificar o app de verdade (humano ou executor com acesso à tela)

Abra `target/release/cd-ai-desktop.exe` e confira, uma coisa por vez:

1. O rodapé da sidebar mostra `core v…` (comando `app_info`).
2. O status do Ollama aparece (`ollama_status`).
3. "Abrir workspace" abre o diálogo e mostra o nome da pasta (`open_workspace`).
4. **Canal de streaming:** o `startChat` do `ipc.ts` só pode ser testado no app. Se ainda não existir UI de chat, rode `bun run dev` e, no console do DevTools da janela, execute:

   ```js
   const { Channel, invoke } = window.__TAURI__.core;
   const c = new Channel();
   c.onmessage = console.log;
   await invoke("chat", { request: { model: "qwen3:4b", messages: [{ role: "user", content: "oi" }], numCtx: 2048 }, onEvent: c });
   ```

   Se `window.__TAURI__` não existir (o `withGlobalTauri` está desligado), pule este item e anote isso no status do plano.

Se algum item falhar com erro de permissão no console (mensagens com `not allowed` ou `permission`), o próprio erro traz o identificador que falta, por exemplo `core:event:allow-listen`. Adicione **só esse identificador** na lista de permissões, reconstrua e repita. Anote cada permissão adicionada e o motivo em "Notas de manutenção", no fim deste arquivo.

**Verificar:** os quatro itens funcionam, e a lista final de permissões não contém `core:default`.

## Plano de testes

Não há teste automatizado possível para a ACL sem E2E. A verificação é o Passo 3. Registre no status do plano quais itens foram conferidos manualmente.

> **Status da execução (2026-09-11):** build validou os identificadores, app abre sem erro de permissão no log. Itens 1 a 3 do Passo 3 (visual) pendentes de conferência humana; item 4 pulado conforme definido (`withGlobalTauri` desligado, sem `window.__TAURI__`).

## Critérios de pronto

- [ ] `src-tauri/capabilities/default.json` não contém `core:default`
- [ ] `src-tauri/build.rs` lista exatamente os commands de `generate_handler!`
- [ ] `bun run verify` e `bun tauri build --no-bundle` saem com exit 0
- [ ] Os itens 1 a 3 do Passo 3 foram conferidos
- [ ] A linha deste plano em `plans/README.md` está atualizada

## STOP conditions

- O schema gerado não contém identificadores `allow-<command>` depois do Passo 1 (a API do `AppManifest` mudou). Reporte o conteúdo relevante do schema.
- Um item do Passo 3 continua falhando depois de você adicionar três permissões seguidas. Reporte os erros; não volte para `core:default`.
- O app nem abre a janela.

## Notas de manutenção

- **Todo command novo precisa entrar em três lugares:** `generate_handler!`, a lista do `build.rs` e a capability. Um reviewer deve recusar um PR que adicione um command em só um deles.
- Plugins futuros chamados pelo JS exigem a permissão específica do plugin (por exemplo `dialog:allow-open`), nunca `*:default` sem justificativa.
