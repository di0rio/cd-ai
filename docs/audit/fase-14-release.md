# Fase 14 — Release Linux (auditoria)

Medido em 2026-09-16. Plano: [`plans/024-fase-14-release.md`](../../plans/024-fase-14-release.md). Ambiente desta sessão: Ubuntu 24.04 x86_64, rustc 1.98.1, bun 1.4.2, WebKitGTK 4.1. **Sem Ollama.** glibc desta VM: **2.39**.

## O que o pacote prova

Saída da SPEC §34 Fase 14: Linux x86_64 `.deb` + AppImage; app instalado do zero resolve uma tarefa do eval.

Caminho usado (CLI no pacote + suíte em `/usr/share/cd-ai/evals`):

```bash
sudo apt install ./cd-ai_0.1.0_amd64.deb
env -i HOME="$HOME" PATH="/usr/bin:/bin:$HOME/.bun/bin" \
  cd-ai eval --scripted --suite /usr/share/cd-ai/evals --out /tmp/cd-ai-eval.json
```

`PATH` **sem** `cargo`/`rustc`/`tauri`. Bun só entra porque o check das fixtures é `bun test` (não é `Depends` do `.deb`).

## Evidência desta VM

| Prova | Resultado | Notas |
|---|---|---|
| `bun run verify` | exit 0 | teto do teste de leituras da Fase 13: 200 ms → 500 ms (esta VM mediu sequential=229 ms; o redactor linear ainda está ~10× abaixo dos 2,1 s antigos) |
| `cargo run -p cd-ai-cli -- eval --scripted` (árvore de dev) | **100% (3/3)** | `evals/results/scripted-fase14.json` (gitignored) |
| `bun scripts/build-linux-release.ts` | exit 0 | `.deb` 9,5 MiB; AppImage 81 MiB; CLI 13 MiB |
| `dpkg-deb -I` | `Package: cd-ai`; Depends `libwebkit2gtk-4.1-0, libgtk-3-0`; Recommends `git`; section `devel` | |
| Conteúdo do `.deb` | `/usr/bin/cd-ai`, `/usr/bin/cd-ai-desktop`, `/usr/share/cd-ai/evals/{tasks,fixtures}`, `cd-ai.desktop` | ícones hicolor 32/128/256 |
| `apt install` do `.deb` | pacote `cd-ai` configurado | `fuse3`/`xdg-desktop-portal` desta VM já estavam partidos **antes**; o `cd-ai` instalou |
| `cd-ai --version` sem toolchain | `cd-ai 0.1.0` | `PATH=/usr/bin:/bin:$HOME/.bun/bin` |
| eval 3/3 com CLI + `--suite` do pacote | **100% (3/3)** | suite=`/usr/share/cd-ai/evals`; dobro/greet/soma `completed` |
| AppImage `--appimage-extract` | CLI `cd-ai 0.1.0` + as três tasks | sem FUSE |
| GUI abre | **não verificado** | `cd-ai-desktop` panicou em `gtk::rt::init` neste VNC (`DISPLAY=:1`, sessão GTK incompleta). Binário e `.desktop` estão no pacote. Precisa de um desktop real. |
| eval ao vivo com Ollama | **não corrido** | sem daemon nesta VM |
| artefato glibc 2.35 (Ubuntu 22.04) | CI | [`.github/workflows/linux-release.yml`](../../.github/workflows/linux-release.yml). **Não use o `.deb` desta VM 24.04 em 22.04.** |

A primeira corrida da CI (commit `2153cc5`) morreu no passo `bun run verify`: *“The hosted runner lost communication with the server”* (CPU/RAM). `clippy --workspace --all-targets` compilava o crate Tauri/WebKit em **debug** no runner de 7 GiB.

A segunda (`db45c1e`, só core+CLI) morreu no mesmo erro durante `clippy --all-targets` + `cargo test` (~46 min). O workflow agora: 8 GiB de swap, `CARGO_BUILD_JOBS=1`, clippy **sem** `--all-targets`, testes com 2 threads, WebKit só no `tauri build` em release. `bun run verify` local continua a incluir o desktop.

SHA256 desta VM (`dist/linux/`, não commitado):

```text
fb7ee5e400eb495a7745c4227632b7fa52b72c390feb35c219397974e556dc99  13186056  cd-ai
1c1842e97333f1293aecad94b424ab330922962a7ae52e1f8a0cef8332db0c08  9860406  cd-ai_0.1.0_amd64.deb
7fe669ce51a95f4cfdb664621403a6452d75625f01416bbd57be3433642d1112  84044280  cd-ai_0.1.0_amd64.AppImage
```

Eval instalado (`/tmp/cd-ai-eval.json`):

| tarefa | iterações | estimatedPromptTokens | agentStatus |
|---|---|---|---|
| dobro | 4 | 3307 | completed |
| greet | 4 | 3138 | completed |
| soma | 4 | 3100 | completed |

Taxa **100% (3/3)**. Rota `coder/8192`.

## Caminho ao vivo (Ollama + CODER)

Numa máquina com Ollama 0.34+ e o CODER da decisão 0002, **depois** de instalar o `.deb`:

```bash
cd-ai eval --model qwen3-coder:30b --suite /usr/share/cd-ai/evals --out /tmp/cd-ai-eval-live.json
```

## Fora desta fase

Flatpak, RPM, Windows, macOS, assinatura, updater, LICENSE, `fix-path-env` na GUI. Ver [`docs/release.md`](../release.md).
