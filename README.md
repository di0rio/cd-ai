# cd-ai

Coding agent local-first para desenvolvimento de software. Ele roda no seu próprio computador, usa modelos locais via Ollama e executa o ciclo completo de entendimento, planejamento, implementação, execução, validação e correção sem depender de uma API de IA externa.

App desktop em Tauri, com interface em Next.js e núcleo em Rust.

Tokens não possuem custo financeiro para o cd-ai: a inferência acontece localmente no hardware do usuário. Os limites existentes são exclusivamente operacionais e de segurança, como memória disponível, tempo de execução, número de iterações e prevenção de loops.

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

### App desktop completo (Tauri + Next.js)

```bash
bun run dev
```

Esse é o comando do app de verdade. Ele:

1. Sobe o servidor Next.js (dev) em `http://localhost:1420` (via `beforeDevCommand` do Tauri).
2. Compila o `src-tauri` em Rust — a primeira compilação demora alguns minutos.
3. Abre a janela nativa do Tauri apontando pra `http://localhost:1420`.

A janela do Tauri é o lugar onde o app funciona por inteiro: é por ela que os *commands* do núcleo em Rust (workspace, Ollama, tools) são acessíveis. O servidor do Next.js sozinho não tem acesso a eles.

### Só a interface no navegador (sem o Tauri)

```bash
bun run --cwd apps/desktop dev
```

Abra `http://localhost:1420`. Esse modo é mais rápido pra mexer no visual, mas o que depende do núcleo em Rust (via Tauri) não funciona aqui.

Pra ver a interface com sessões de exemplo, abra `http://localhost:1420/?demo`. Isso só funciona em desenvolvimento.

### CLI

```bash
cargo run -p cd-ai-cli -- --version
```

### Binário de release (sem instalador)

```bash
bun tauri build --no-bundle
```

Gera o binário final do app em `src-tauri/target/release/` sem criar instalador — útil pra testar o app de produção antes do empacotamento.

### Benchmark de modelos (com o Ollama rodando)

```bash
bun scripts/bench-models.ts <modelo> [modelo...]
```

## Outros comandos

| Comando | O que faz |
| --- | --- |
| `bun run verify` | Roda todos os checks: Biome, typecheck, testes do frontend, `cargo fmt`, clippy e testes do Rust |
| `bun run build` | Gera o app desktop de produção (`tauri build`, com instalador) |
| `bun tauri build --no-bundle` | Gera só o binário do app de produção, sem instalador |
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
