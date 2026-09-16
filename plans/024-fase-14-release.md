# Plano 024: Fase 14 — Release Linux x86_64 (`.deb` e AppImage)

> **Instruções ao executor:** rode os passos nesta ordem e confirme cada verificação antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Uma parte só está pronta com `bun run verify` saindo com exit 0 (AGENTS.md).
>
> **Drift check (rode primeiro):** `git log -1 --oneline` deve estar em `main` com as Fases 0–13 (PR #13). Confira `src-tauri/tauri.conf.json` (`bundle.targets` = `deb` + `appimage`), `apps/desktop/public/icon.svg`, `src-tauri/icons/`, `Cargo.toml` `version = "0.1.0"`, e SPEC §34 Fase 14. Se o eval scripted, o CLI `cd-ai` ou os alvos Linux tiverem sumido, trate como STOP.

## Status

- **Prioridade:** P1
- **Esforço:** M
- **Risco:** MEDIUM (glibc do host de build trava a distro mínima; nome do binário desktop colide com a CLI `cd-ai`; assinatura/segredo no repositório)
- **Depende de:** 023 (Fase 13 no `main`)
- **Categoria:** release (SPEC §34 Fase 14; decisão 0001)
- **Planejado em:** commit `83ba7a1` (`main`), 2026-09-16

## Por que isso importa

As Fases 0–13 fecham o produto no repositório: o loop, o sandbox Linux, o eval 3/3 e as otimizações medidas. Ainda não há **caminho de instalação** — `bundle.targets` já pede `.deb` e AppImage, mas faltam metadados, o binário da CLI no pacote, instruções reproduzíveis e prova de máquina limpa. A Fase 14 é a última do roadmap da §34: Linux x86_64 instalado do zero resolve uma tarefa do eval (ou a prova mais próxima, com lacunas explícitas).

## Estado atual (fatos)

- `tauri.conf.json`: `productName` `cd-ai`, `identifier` `dev.cdai.desktop`, `version` `0.1.0`, `targets` `["deb", "appimage"]`. Sem `mainBinaryName`, sem `linux.deb`/`linux.appimage`, sem descrição/categoria/homepage.
- O crate desktop chama-se `cd-ai-desktop`; a CLI publica o binário `cd-ai`. Sem `mainBinaryName`, o bundler do Tauri pode gravar a GUI em `/usr/bin/cd-ai` e esmagar a CLI.
- Ícones já existem em `src-tauri/icons/` (gerados de `apps/desktop/public/icon.svg`, decisão 0007).
- `bun run build` = `tauri build`. `beforeBuildCommand` só gera o frontend. Não há script de release, nem CI, nem `dist/`.
- README descreve o app como árvore de desenvolvimento (`bun install` / `cargo run`). Não há instalação de pacote.
- Sem `.github/workflows`. Sem assinatura. Sem LICENSE na raiz.
- Eval scripted 3/3; o check das fixtures chama `bun test`. Sem Ollama nesta VM a taxa ao vivo não bloqueia (mesmo padrão das fases 6–13).

## Decisões deste plano (não reabrir sem ADR)

| # | Decisão | Motivo |
|---|---|---|
| D1 | **GUI = `cd-ai-desktop`; CLI = `cd-ai`.** `mainBinaryName` no Tauri. O `.deb` e o AppImage incluem a CLI em `/usr/bin/cd-ai` via `bundle.linux.*.files`. O menu do desktop abre a janela; o terminal fala com a CLI. | Evita colisão. O eval é CLI (decisão 0003). |
| D2 | **Primeiro release = Linux x86_64, sem assinatura.** `.deb` + AppImage. Sem Flatpak, RPM, Windows, macOS. Sem updater Tauri. Sem chave/GPG no git. Como assinar depois fica em `docs/release.md` (segredo fora do repo). | SPEC §34 “depois”; decisão 0001. |
| D3 | **Versão única `0.1.0`.** `Cargo.toml` workspace e `tauri.conf.json` iguais. Script barato no gate recusa drift. Bump manual nos dois quando houver tag `v*`. | Tauri recomenda a versão no conf; o CLI imprime `CARGO_PKG_VERSION`. |
| D4 | **A suíte de eval viaja no pacote** (`/usr/share/cd-ai/evals/{tasks,fixtures}`). Relatório com `--out` num path gravável — `/usr/share` não é. | Saída da §34: app instalado do zero + uma tarefa de eval, sem clonar o monorepo só para achar fixtures. |
| D5 | **Host de CI = Ubuntu 22.04** (glibc mais velho que ainda tem WebKitGTK 4.1). Build nesta VM 24.04 é evidência local, não o artefato portátil oficial. | Docs do Tauri: glibc do builder é o piso. |
| D6 | **Ollama e Bun não são `Depends` do `.deb`.** Ollama é runtime das tarefas ao vivo; Bun só entra no *check* das fixtures atuais. `git` fica em `Recommends` (checkpoints). | Máquina limpa corre o binário sem toolchain Rust/Tauri. O check `bun test` é propriedade da suíte, não do app. |
| D7 | **Eval não cresce.** Scripted 3/3 não pode regressar. Prova de release: (a) pacotes existem; (b) `cd-ai --version` no binário empacotado; (c) `eval --scripted` com a CLI empacotada + `--suite` empacotada, se Bun estiver no PATH; (d) lacunas (GUI/display, Ollama ao vivo, glibc) no audit. | 016 D3. |

## Escopo

**Dentro:** plano 024; metadados/ícones/desktop do Tauri; CLI + eval no pacote; script `build-linux-release`; check de metadados no `verify`; workflow GitHub (22.04); README/AGENTS/handoff/`docs/release.md`/audit; tentativa de bundle nesta VM.

**Fora:** Windows/macOS/Flatpak/RPM como bloqueadores. Assinar pacotes. Auto-update. LICENSE nova. `fix-path-env` na GUI (PATH do menu). Tokenizer, watcher, suíte maior. Payloads ofensivos.

## Passos

1. **Plano:** este arquivo; linha 024 em `plans/README.md`.
2. **Tauri D1–D4:** `mainBinaryName`, categoria, descrições, homepage, `linux.deb` (section `devel`, recommends `git`, files CLI + evals), `linux.appimage.files` iguais. `beforeBuildCommand` também faz `cargo build --release -p cd-ai-cli`.
3. **Scripts:** `scripts/check-release-metadata.ts` (gate); `scripts/build-linux-release.ts` (CLI + `tauri build` + cópia para `dist/linux/` + hashes).
4. **CI D5:** `.github/workflows/linux-release.yml` em Ubuntu 22.04: deps WebKitGTK, `bun run verify`, eval scripted, script de release, upload de artefatos; GitHub Release só em tag `v*`.
5. **Docs:** `docs/release.md` (build, instalação limpa, prova de eval, glibc, o que não entra); README/AGENTS; handoff marca roadmap Linux completo; `docs/audit/fase-14-release.md`.
6. **Gate:** `bun run verify` exit 0. `cargo run -q -p cd-ai-cli -- eval --scripted` 3/3. Bundle nesta VM se as deps deixarem; senão o audit registra o comando e o erro.

## Contratos

Binários:

```text
/usr/bin/cd-ai-desktop     → GUI Tauri
/usr/bin/cd-ai             → CLI (mesmo agent-core)
pacote Debian              → nome cd-ai (productName)
```

Eval instalado:

```text
cd-ai eval --scripted --suite /usr/share/cd-ai/evals --out /tmp/cd-ai-eval.json
check das fixtures         → bun test (Bun no PATH; não é dependência do .deb)
```

Versão:

```text
tauri.conf.json version  ==  workspace.package.version  ==  cd-ai --version
```

Fora do primeiro release:

```text
Flatpak / RPM / .msi / .exe / .dmg / sandbox Win+mac  → depois (SPEC §34)
assinatura                                             → docs, sem segredo no git
```

## Critérios de pronto

- [x] `plans/024-fase-14-release.md` existe; linha 024 em `plans/README.md` = DONE
- [x] `tauri.conf.json` tem metadados Linux + `mainBinaryName` + CLI/evals nos `files`
- [x] `bun scripts/build-linux-release.ts` documentado; CI 22.04 existe
- [x] README/handoff/`docs/release.md` descrevem instalação limpa e plataformas “depois”
- [x] Audit com o que esta VM/CI comprovou e o que falta numa máquina nua
- [x] `bun run verify` exit 0
- [x] Eval scripted 3/3
- [x] PR aberto

## STOP conditions

- Pedido de colocar chave de assinatura, token ou secret no repositório → recuse.
- Pedido de PoC/exploit “para testar o pacote” → recuse.
- Eval scripted caiu de 3/3 → o empacotamento não vale uma regressão do agente; pare.
- Bundle Linux impossível nesta VM (WebKitGTK, linuxdeploy, etc.) → **não é STOP**. Deixe config + docs + CI; registre o erro no audit.
- Ollama ausente → não é STOP. Prove com CLI empacotada + `--scripted`.

## Notas de manutenção

- Código e comentários em inglês; UI e docs em pt-BR.
- Regenerar ícones: `bunx tauri icon apps/desktop/public/icon.svg` (fonte: decisão 0007).
- A fronteira de confiança não muda: o pacote só distribui o que o Rust já valida.
- Commits automáticos no git do **usuário** continuam desligados (SPEC §19).
