# Plano 008: os tipos TypeScript do IPC são gerados a partir do Rust

> **Instruções ao executor:** siga o plano passo a passo. Rode cada comando de verificação e confirme o resultado esperado antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Ao terminar, atualize a linha deste plano em `plans/README.md`.
>
> **Drift check (rode primeiro):** confirme que existem em `crates/agent-core/src`:
>
> - `workspace.rs` com `WorkspaceInfo` (plano 003);
> - `ollama.rs` com `OllamaStatus`, `ModelInfo` e `LoadedModel` (plano 004).
>
> Se o plano 005 também foi executado, existem `ChatRequest`, `ChatMessage` e `ChatEvent`. Anote quais tipos existem; eles são a lista do Passo 2.

## Status

- **Prioridade:** P2
- **Esforço:** M
- **Risco:** LOW
- **Depende de:** `plans/003-abrir-workspace-pela-ui.md` e `plans/004-ollama-provider-e-status.md`
- **Categoria:** tech-debt
- **Planejado em:** base `158f609` + árvore de trabalho de 2026-09-11

## Por que isso importa

Hoje cada tipo trafegado pelo IPC é escrito duas vezes: no Rust e em `apps/desktop/src/lib/ipc.ts`, com comentários `// Mirrors ...`. Um campo renomeado no Rust quebra a UI em tempo de execução, sem nenhum erro de compilação.

Cada command novo aumenta esse risco. Gerar os tipos a partir do Rust (a fonte da verdade, pela decisão 0003) transforma a divergência em erro de typecheck.

## Estado atual

- `apps/desktop/src/lib/ipc.ts`: tipos escritos à mão, cada um com `// Mirrors agent_core::...`, e wrappers `invoke<T>("command")`.
- Os structs Rust derivam `serde::Serialize` (e alguns `Deserialize`). Alguns usam `#[serde(rename_all = "camelCase", ...)]`, que o gerador precisa respeitar.
- Os campos `u64` (tamanhos em bytes, contagens de tokens) são serializados pelo serde como número JSON. **O `ts-rs` mapeia `u64` para `bigint` por padrão, o que está errado aqui.**
- O `biome.json` checa todo o repositório. Arquivos gerados não devem ser checados.
- `apps/desktop/tsconfig.json` usa o alias `@/*` → `./src/*`.

## Comandos que você vai precisar

| Propósito | Comando | Esperado |
|-----------|---------|----------|
| Dependência | `cargo add ts-rs -p agent-core` | exit 0 |
| Gerar os bindings | `cargo test -p agent-core export_bindings` | arquivos `.ts` criados em `apps/desktop/src/lib/bindings/` |
| Gate | `bun run verify` | exit 0 |

## Escopo

**Dentro do escopo:**

- `crates/agent-core/Cargo.toml`
- `crates/agent-core/src/lib.rs`, `workspace.rs` e `ollama.rs` (só derives e atributos `#[ts(...)]`)
- `.cargo/config.toml` (criar)
- `apps/desktop/src/lib/bindings/` (gerado)
- `apps/desktop/src/lib/ipc.ts`
- `biome.json`

**Fora do escopo:**

- Mudar nomes de campos ou o formato do JSON: o contrato atual não muda, só a origem dos tipos.
- `src-tauri/`.

## Git

- Branch: `advisor/008-ts-bindings`.
- Mensagem no estilo `refactor: generate IPC types from Rust`.
- Commite os bindings gerados junto.
- Não faça push.

## Passos

### Passo 1: configurar o destino

1. Crie `.cargo/config.toml` na raiz com:

   ```toml
   [env]
   TS_RS_EXPORT_DIR = { value = "apps/desktop/src/lib/bindings", relative = true }
   ```

2. Rode `cargo add ts-rs -p agent-core`.

**Verificar:** `cargo build -p agent-core` sai com exit 0.

### Passo 2: derivar os tipos

Em cada struct ou enum que passa pelo IPC:

1. Adicione `#[derive(ts_rs::TS)]` e `#[ts(export)]`.
2. Em cada campo `u64`, adicione `#[ts(type = "number")]`.

A lista é: `AppInfo`, `WorkspaceInfo`, `OllamaStatus`, `ModelInfo`, `LoadedModel` e, se existirem, `ChatMessage`, `ChatRequest` e `ChatEvent`.

Atenção a dois casos:

- `AppInfo` tem campos `&'static str`. Se o `ts-rs` não aceitar, adicione `#[ts(type = "string")]` nesses campos. Não mude o tipo Rust.
- Os atributos `serde(rename_all, tag, content)` devem ser respeitados pelo `ts-rs`, que os lê por padrão.

**Verificar:** `cargo test -p agent-core` sai com `0 failed`, e `apps/desktop/src/lib/bindings/` contém um `.ts` por tipo, como `AppInfo.ts` e `OllamaStatus.ts`.

### Passo 3: excluir os bindings do Biome

No `biome.json`, adicione na raiz do objeto (ou dentro de `files`, se já existir):

```json
"files": { "includes": ["**", "!!apps/desktop/src/lib/bindings"] }
```

**Verificar:** `bun run check` sai com exit 0.

### Passo 4: usar os bindings no `ipc.ts`

1. Apague os tipos escritos à mão e os comentários `// Mirrors`.
2. Importe os tipos gerados, por exemplo: `import type { AppInfo } from "./bindings/AppInfo";`.
3. Reexporte-os (`export type { AppInfo }`) para não mudar os imports dos componentes.
4. Se `ChatEvent` foi gerado, confira que o formato é idêntico ao union escrito à mão (`{ event: "token"; data: {...} } | ...`). Se for diferente, veja a STOP condition.

**Verificar:** `bun run --cwd apps/desktop typecheck` sai com exit 0.

### Passo 5: prova de que a divergência agora quebra

1. Temporariamente, renomeie no Rust o campo `WorkspaceInfo.name` para `title`.
2. Rode `cargo test -p agent-core` e depois `bun run --cwd apps/desktop typecheck`.
3. **O typecheck precisa falhar.**
4. Desfaça o rename e rode `cargo test -p agent-core` de novo, para regenerar os bindings.

**Verificar:** depois de desfazer, `bun run verify` sai com exit 0, e `git diff apps/desktop/src/lib/bindings` fica vazio em relação ao Passo 2.

## Plano de testes

O Passo 5 é o teste. Os testes de export do `ts-rs` (`export_bindings_*`) rodam no `cargo test`.

## Critérios de pronto

- [ ] `grep -rn "Mirrors agent_core" apps/desktop/src` não retorna nenhuma linha
- [ ] `apps/desktop/src/lib/bindings/` está commitado, com um arquivo por tipo
- [ ] `bun run verify` sai com exit 0
- [ ] Rodar `cargo test -p agent-core` numa árvore limpa não gera diff em `apps/desktop/src/lib/bindings`
- [ ] A linha deste plano em `plans/README.md` está atualizada

## STOP conditions

- O `ts-rs` gera `bigint` mesmo com `#[ts(type = "number")]`.
- O tipo gerado para um enum com `tag`/`content` diverge do JSON real que o serde produz.
- O `TS_RS_EXPORT_DIR` é ignorado e os arquivos vão para `crates/agent-core/bindings`. Reporte a versão do `ts-rs`.

## Notas de manutenção

- Todo struct novo que passa pelo IPC deve ganhar `#[derive(TS)] #[ts(export)]` no mesmo commit em que é criado.
- Todo `u64` precisa de `#[ts(type = "number")]`. Valores acima de 2^53 perderiam precisão no JS, o que é aceitável para bytes e tokens.
- Os bindings são arquivos gerados: nunca os edite à mão.
