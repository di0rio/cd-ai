# Release Linux x86_64 (Fase 14)

Primeiro release do cd-ai: **Linux x86_64**, pacotes **`.deb`** e **AppImage**. Windows, macOS, Flatpak e RPM ficam para depois (SPEC §34, decisão 0001). Não há auto-update e os artefatos **não são assinados**.

Versão atual: **0.1.0** (`Cargo.toml` workspace e `src-tauri/tauri.conf.json` — o gate recusa drift).

## O que o pacote instala

| Path | Conteúdo |
|---|---|
| `/usr/bin/cd-ai-desktop` | App Tauri (janela) |
| `/usr/bin/cd-ai` | CLI (`task`, `eval`, `history`, …) |
| `/usr/share/cd-ai/evals/tasks` | Tarefas da suíte de eval |
| `/usr/share/cd-ai/evals/fixtures` | Fixtures (cópia descartável em runtime) |
| menu de aplicações | atalho **cd-ai** → a GUI |

O Ollama **não** vai no `.deb`: instale-o à parte e deixe o daemon no ar para tarefas ao vivo. `git` é `Recommends` (checkpoints). O check das fixtures atuais chama `bun test` — o Bun também não é dependência do pacote.

## Gerar os artefatos

Na raiz, em Linux x86_64, com [pré-requisitos do Tauri](https://v2.tauri.app/start/prerequisites/) (WebKitGTK 4.1, GTK 3, `patchelf`, Rust 1.85+, Bun):

```bash
bun install
bun scripts/build-linux-release.ts
```

Saída em `dist/linux/`:

- `cd-ai_<versão>_amd64.deb`
- `cd-ai_<versão>_amd64.AppImage`
- `cd-ai` (CLI solta, o mesmo binário que o `.deb` instala)
- `SHA256SUMS`

`bun run build` (`tauri build`) também gera `.deb` e AppImage; o script acima copia, hasheia e recusa metadados incoerentes. `bun tauri build --no-bundle` continua sendo só o executável da GUI, sem instalador.

**glibc:** o binário exige a glibc da máquina que compilou. A CI oficial usa **Ubuntu 22.04**. Um build em Ubuntu 24.04 (glibc 2.39) **não** corre em 22.04. Prefira a CI ou um container 22.04 para artefatos que pretende distribuir.

Workflow: [`.github/workflows/linux-release.yml`](../.github/workflows/linux-release.yml) (PR, `workflow_dispatch`, tag `v*`). Tag `v0.1.0` publica os arquivos num GitHub Release.

A CI **não** corre `bun run verify` nem `cargo test`: o runner de 7 GiB morreu ao compilar o harness de testes / sandbox. Lá: frontend + clippy da lib + `tauri build` em release + eval com a CLI empacotada. Na máquina de desenvolvimento o gate continua a ser `bun run verify`.

## Instalação limpa (sem Rust/Tauri)

### Debian / Ubuntu (`.deb`)

```bash
sudo apt install ./cd-ai_0.1.0_amd64.deb
cd-ai --version          # cd-ai 0.1.0
cd-ai-desktop &          # janela; ou o ícone do menu
```

Dependências de runtime da GUI (WebKitGTK 4.1, GTK 3) vêm no `Depends` do pacote.

### AppImage

```bash
chmod +x cd-ai_0.1.0_amd64.AppImage
./cd-ai_0.1.0_amd64.AppImage
```

A CLI e a suíte de eval ficam **dentro** do AppImage. Para usá-las sem instalar o `.deb`:

```bash
./cd-ai_0.1.0_amd64.AppImage --appimage-extract
./squashfs-root/usr/bin/cd-ai --version
./squashfs-root/usr/bin/cd-ai eval --scripted \
  --suite ./squashfs-root/usr/share/cd-ai/evals \
  --out /tmp/cd-ai-eval.json
```

(`--appimage-extract` não precisa de FUSE.)

## Prova de eval (saída da Fase 14)

Com a CLI instalada, Bun no `PATH` (o check das fixtures é `bun test`) e **sem** toolchain de compilação:

```bash
cd-ai eval --scripted --suite /usr/share/cd-ai/evals --out /tmp/cd-ai-eval.json
```

Esperado: taxa 3/3. `/usr/share` não é gravável — `--out` é obrigatório nesse path. Tarefa ao vivo (Ollama + CODER):

```bash
cd-ai eval --model qwen3-coder:30b --suite /usr/share/cd-ai/evals --out /tmp/cd-ai-eval-live.json
```

A GUI resolve a mesma classe de tarefa (abrir workspace, descrever o pedido). Lançada pelo menu, ela **não** herda o `PATH` do `~/.bashrc`; `git`/`bun`/`cargo` precisam estar em `/usr/bin` ou a GUI deve ser aberta dum terminal. Isso é lacuna conhecida, não um bloqueio do `.deb`.

## Assinatura (depois, sem secret no git)

Os pacotes desta fase são **unsigned**. Quem for assinar:

- **`.deb`:** `debsign` / GPG. A chave privada fica no secret store da CI (`GPG_PRIVATE_KEY` + passphrase), nunca no repositório.
- **AppImage:** digest em `SHA256SUMS` já sai do script; assinatura minisign/GPG é opcional e usa o mesmo tipo de secret.

Não ligue `bundle.createUpdaterArtifacts` sem um fluxo de chave separado. A v1 não faz rede para atualizar (decisão 0004).

## Regenerar ícones

Fonte: `apps/desktop/public/icon.svg` (decisão 0007).

```bash
bunx tauri icon apps/desktop/public/icon.svg
```

## Plataformas que não entram neste release

| Formato | Estado |
|---|---|
| Flatpak | depois |
| RPM | depois (`bundle.targets` não inclui) |
| Windows `.msi` / NSIS | depois; no Windows use `bun tauri build --no-bundle` para o executável |
| macOS `.dmg` | depois |
| Sandbox de OS em Windows/macOS | indisponível; FULL ACCESS recusado (decisão 0001, Fase 7) |

Evidência do que foi corrido nesta fase: [`docs/audit/fase-14-release.md`](audit/fase-14-release.md).
