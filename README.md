# cd-ai

Coding agent local-first para desenvolvimento de software. Ele roda no seu próprio computador, usa modelos locais via Ollama e executa o ciclo completo de entendimento, planejamento, implementação, execução, validação e correção sem depender de uma API de IA externa.

App desktop em Tauri, com interface em Next.js e núcleo em Rust.

Tokens não possuem custo financeiro para o cd-ai: a inferência acontece localmente no hardware do usuário. Os limites existentes são exclusivamente operacionais e de segurança, como memória disponível, tempo de execução, número de iterações e prevenção de loops.

A especificação completa está em [SPEC.md](SPEC.md) e as decisões de arquitetura estão em [docs/decisions/](docs/decisions/).

## Pré-requisitos

- [Bun](https://bun.sh)
- [Rust](https://rustup.rs) (toolchain estável, com suporte à edition 2024 — rustc 1.85+)
- Os [pré-requisitos do Tauri](https://tauri.app/start/prerequisites/) pro seu sistema. No Windows são o WebView2 e o Microsoft C++ Build Tools. No Linux, WebKitGTK 4.1 + GTK 3 (detalhe em [docs/release.md](docs/release.md)).
- [Ollama](https://ollama.com) para tarefas ao vivo (o eval `--scripted` não precisa).

Confira se `cargo --version` funciona no terminal. Se o Rust estiver instalado mas o comando não for encontrado, adicione `%USERPROFILE%\.cargo\bin` (Windows) ou `~/.cargo/bin` (Linux/macOS) ao PATH e abra o terminal de novo.

## Instalar (Linux x86_64)

Pacotes da [página de releases](https://github.com/di0rio/cd-ai/releases) ou gerados com `bun scripts/build-linux-release.ts`. Sem Rust nem Tauri na máquina de uso.

### .deb (Debian / Ubuntu)

```bash
sudo apt install ./cd-ai_0.1.0_amd64.deb
cd-ai --version
cd-ai-desktop &
```

A CLI (`cd-ai`) e a GUI (`cd-ai-desktop`) vêm no mesmo pacote. A suíte de eval fica em `/usr/share/cd-ai/evals`.

### AppImage

```bash
chmod +x cd-ai_0.1.0_amd64.AppImage
./cd-ai_0.1.0_amd64.AppImage
```

Passo a passo, glibc, eval numa máquina limpa e o que ficou para depois (Flatpak, RPM, Windows, macOS): [docs/release.md](docs/release.md).

## Desenvolvimento

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

Com o Ollama rodando, o `task` resolve uma tarefa de ponta a ponta no terminal:

```bash
cargo run -p cd-ai-cli -- task --model <modelo> [--workspace <pasta>] [--mode ask|auto|full-access] "<pedido>"
```

Cada escrita e cada comando que não seja leitura pedem aprovação no terminal (`Aprovar? [s/N]`),
no modo padrão ASK. Sem terminal interativo a ação é negada — não existe `--yes`.
`--mode auto` edita sozinho e, com sandbox Linux, também corre comandos de escrita.
`--mode full-access` só existe com sandbox ativo (Linux). Rede, destrutivo e secrets sempre
pedem aprovação. Uma tarefa interrompida volta com `--resume <id>`, e o Ctrl+C cancela a
tarefa e os processos filhos.

A suíte de eval (Fase 6) é outro comando, headless, sobre cópias das fixtures:

```bash
cargo run -p cd-ai-cli -- eval --scripted
cargo run -p cd-ai-cli -- eval --model <modelo>
```

`--scripted` não fala com o Ollama (usa o campo `script` de cada tarefa). O eval aprova sozinho
porque só edita a cópia descartável. Detalhes em [evals/README.md](evals/README.md).

### Pacotes de release (Linux x86_64)

```bash
bun scripts/build-linux-release.ts
```

Gera `.deb`, AppImage e a CLI em `dist/linux/`. Equivale a `bun run build` (`tauri build`) mais cópia e hashes. O `beforeBuildCommand` do Tauri também compila a CLI.

### Binário de release (sem instalador)

```bash
bun tauri build --no-bundle
```

Gera o executável da GUI sem criar instalador — útil pra testar o app de produção em qualquer SO, inclusive Windows/macOS, onde este release ainda não empacota.

### Benchmark de modelos (com o Ollama rodando)

```bash
bun scripts/bench-models.ts <modelo> [modelo...]
```

## Outros comandos

| Comando | O que faz |
| --- | --- |
| `bun run verify` | Roda todos os checks: Biome, typecheck, testes do frontend, metadados de release, `cargo fmt`, clippy e testes do Rust |
| `bun run build` | Gera o app desktop de produção (`tauri build`: `.deb` + AppImage no Linux) |
| `bun scripts/build-linux-release.ts` | Idem, e copia artefatos + SHA256 para `dist/linux/` |
| `bun tauri build --no-bundle` | Gera só o binário da GUI de produção, sem instalador |
| `bun run check` | Lint e formatação com Biome |
| `bun run format` | Formata o código com Biome |
| `bun run --cwd apps/desktop typecheck` | Checagem de tipos do TypeScript |
| `bun run --cwd apps/desktop test` | Testes do frontend |
| `cargo test` | Testes do Rust |
| `cargo run -p cd-ai-cli -- eval --scripted` | Roda a suíte de eval sem Ollama (Fase 6) |

Os formatos de instalador ficam em `bundle.targets`, no [src-tauri/tauri.conf.json](src-tauri/tauri.conf.json). O primeiro release só empacota `deb` e `appimage` (Linux x86_64). Flatpak, RPM, Windows (`msi`/`nsis`) e macOS (`.dmg`) são follow-ups — ver [docs/release.md](docs/release.md).

## Estrutura

```text
apps/desktop/       interface (Next.js + Tailwind)
apps/cli/           binário de linha de comando `cd-ai`
crates/agent-core/  núcleo do agente em Rust
src-tauri/          shell desktop Tauri
evals/              suíte de eval (Fase 6): tasks, fixtures, results
docs/decisions/     registros de decisão (ADRs)
docs/release.md     instalação Linux (.deb / AppImage)
plans/              planos de implementação
scripts/            scripts auxiliares (benchmark, release Linux)
.github/workflows/  CI de pacotes Linux (Ubuntu 22.04)
```

## Referências e inspiração

- **Paseo** ([paseo.sh](https://paseo.sh/) · [github.com/getpaseo/paseo](https://github.com/getpaseo/paseo) · Apache 2.0) — control plane open source para coding agents. Anotado como referência em 2026-09-11:
  - **Pegar:**
    - *Worktree/git isolado por tarefa* — o agente roda num `git worktree` + branch próprios, sem tocar o diretório de trabalho. Casa com a **Fase 9** (shadow repo/rollback) do SPEC §34.
    - *Superfície de UI de tarefas* — workspace, fase da tarefa, status (working/passed/ready to review), mudanças +/- e "resume in-progress". Concretiza a UI do SPEC §33 nas **Fases 5 e 9**.
    - *Resume in-progress* — persistência/histórico por workspace pra retomar tarefas interrompidas (**Fases 5 e 9**).
  - **Não pegar:** acesso remoto/mobile/web (o cd-ai é local-first e não faz rede, decisão 0005); multi-provedor/MCP/SDK (nosso core é Ollama local); voice.
  - **Licença:** pra copiar código, manter a atribuição do Apache 2.0; pra pegar a ideia, não precisa nada.
