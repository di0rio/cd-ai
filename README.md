# cd-ai

Coding agent local, pessoal e standalone. App desktop em Tauri, com interface em Next.js e núcleo em Rust.

A especificação completa está em [SPEC.md](SPEC.md) e as decisões de arquitetura estão em [docs/decisions/](docs/decisions/).

## Pré-requisitos

- [Bun](https://bun.sh)
- [Rust](https://rustup.rs) (toolchain estável, com suporte à edition 2024)
- Os [pré-requisitos do Tauri](https://tauri.app/start/prerequisites/) pro seu sistema. No Windows são o WebView2 e o Microsoft C++ Build Tools.

Confira se `cargo --version` funciona no terminal. Se o Rust estiver instalado mas o comando não for encontrado, adicione `%USERPROFILE%\.cargo\bin` (Windows) ou `~/.cargo/bin` (Linux/macOS) ao PATH e abra o terminal de novo.

## Instalação

```bash
bun install
```

## Rodando

### App desktop completo

```bash
bun run dev
```

Esse comando roda `tauri dev`, que sobe o Next.js em `http://localhost:1420`, compila o `src-tauri` e abre a janela. A primeira compilação do Rust demora alguns minutos.

### Só a interface, no navegador

```bash
bun run --cwd apps/desktop dev
```

Abra `http://localhost:1420`. Esse modo é mais rápido pra mexer no visual, mas o que depende do núcleo em Rust (via Tauri) não funciona aqui.

Pra ver a interface com sessões de exemplo, abra `http://localhost:1420/?demo`. Isso só funciona em desenvolvimento.

### CLI

```bash
cargo run -p cd-ai-cli -- --version
```

## Outros comandos

| Comando | O que faz |
| --- | --- |
| `bun run build` | Gera o app desktop de produção (`tauri build`) |
| `bun run check` | Lint e formatação com Biome |
| `bun run format` | Formata o código com Biome |
| `bun run --cwd apps/desktop typecheck` | Checagem de tipos do TypeScript |
| `bun run --cwd apps/desktop test` | Testes do frontend |
| `cargo test` | Testes do Rust |

Os formatos de instalador ficam em `bundle.targets`, no [src-tauri/tauri.conf.json](src-tauri/tauri.conf.json). Hoje só `deb` e `appimage` (Linux) estão configurados. Pra gerar instalador no Windows, acrescente `msi` ou `nsis`.

## Estrutura

```text
apps/desktop/       interface (Next.js + Tailwind)
apps/cli/           binário de linha de comando `cd-ai`
crates/agent-core/  núcleo do agente em Rust
src-tauri/          shell desktop Tauri
docs/decisions/     registros de decisão (ADRs)
plans/              planos de implementação
scripts/            scripts auxiliares (ex.: benchmark de modelos no Ollama)
```
