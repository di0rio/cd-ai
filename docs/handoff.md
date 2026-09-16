# Handoff — estado do cd-ai

Estado em 2026-09-16, sobre `main` (Fases 0–13, PR #13) mais o trabalho da **Fase 14**
(plano 024, esta PR). Serve pra uma sessão/agente novo continuar sem depender da conversa anterior.

- **Fontes da verdade:** [`AGENTS.md`](../AGENTS.md) (regras + comandos), [`SPEC.md`](../SPEC.md) (§34), [`plans/README.md`](../plans/README.md), [`docs/decisions/`](decisions/) (ADRs), [`plans/024-fase-14-release.md`](../plans/024-fase-14-release.md), [`docs/release.md`](release.md).
- **Gate:** qualquer mudança só está pronta com `bun run verify` passando (exit 0).

## O que já está pronto (Fases 0–14)

Tudo em `main` até a Fase 13; Fase 14 nesta PR. Gate: `bun run verify`. **O roadmap da SPEC §34 está completo para o primeiro release Linux.**

- **Núcleo Rust** ([`crates/agent-core/`](../crates/agent-core/)): workspace, Ollama, Tool Engine, loop, eval, permissões/sandbox Linux, Verifier, checkpoints, Context Manager, Skills, Model Router, otimizações da Fase 13.
- **CLI** `cd-ai` e **GUI** Tauri (`cd-ai-desktop`), mesmo core.
- **Eval:** scripted 3/3.
- **Release Linux x86_64:** `.deb` + AppImage. GUI em `/usr/bin/cd-ai-desktop`, CLI em `/usr/bin/cd-ai`, suíte em `/usr/share/cd-ai/evals`. Script `bun scripts/build-linux-release.ts`; CI Ubuntu 22.04 em [`.github/workflows/linux-release.yml`](../.github/workflows/linux-release.yml). Detalhe em [`docs/release.md`](release.md) e [`docs/audit/fase-14-release.md`](audit/fase-14-release.md).

## Fase 14 — aceite

- Pacote Linux x86_64: `.deb` e AppImage, metadados (categoria, descrição, homepage, ícones, desktop).
- CLI e suíte de eval viajam no pacote; `cd-ai eval --scripted --suite /usr/share/cd-ai/evals --out …` é o caminho de instalação limpa.
- Sem chave de assinatura no git. Sem updater. Sem Windows/macOS/Flatpak/RPM neste release.
- Eval scripted: `cargo run -q -p cd-ai-cli -- eval --scripted` → 3/3 (não regressou).
- Detalhe: [`docs/audit/fase-14-release.md`](audit/fase-14-release.md).

## O que ficou de fora (não bloqueia o roadmap Linux)

| Item | Por quê |
|---|---|
| Flatpak / RPM | SPEC §34 “depois” |
| Windows `.msi`/NSIS, macOS `.dmg` | Alvo do primeiro release é Linux (decisão 0001) |
| Sandbox de OS em Windows/macOS | FULL ACCESS continua recusado fora do Linux; não fingir sandbox |
| Assinatura GPG / minisign | Sem secret no repositório; procedimento em `docs/release.md` |
| Auto-update Tauri | v1 local-first, sem rede (decisão 0004) |
| `fix-path-env` na GUI | Menu do desktop não herda `~/.bashrc`; abrir `cd-ai-desktop` dum terminal ou pôr `git`/`bun` em `/usr/bin` |
| LICENSE na raiz | Não inventar; o repo ainda não declara uma |
| Tokenizer por modelo, watcher, suíte de eval maior | Fora das Fases 13–14 de propósito |

Commits automáticos no git do **usuário** continuam desligados (SPEC §19). Status `verifying`/`reviewing`/`fixing` (§23) ficaram de fora da Fase 8. Marketplace / importação de skills do usuário ficaram de fora da Fase 11. UI completa de categorias FAST/CODER/REASONER ficou de fora da Fase 12.

## Próximo passo recomendado

O roadmap das 15 fases está fechado no Linux. Próximos trabalhos são **opcionais**: assinar os pacotes, Flatpak/RPM, Windows/macOS, PATH da GUI, taxa ao vivo com `qwen3-coder:30b` numa máquina com Ollama (comando no audit da Fase 14).

## Pontos de atenção pro próximo executor

- **Código e comentários em inglês; UI e documentação em pt-BR.**
- Fronteira de confiança é o Rust: path, permissões, sandbox, execução, secrets, Verifier, rollback, o que entra no prompt, **quais skills carregam** e **qual modelo a tarefa usa** nunca caem no frontend.
- Tool results continuam marcados como não confiáveis. O resumo de compactação usa só estado do engine (`files_changed`, `commands`). Memória não é extraída do modelo.
- FULL ACCESS **não** é um interruptor na webview: `set_permission_mode` recusa se `sandbox::status().available` for falso.
- Quem gerar tipos novos no core roda `cargo test -p agent-core` (o `ts-rs` exporta em `apps/desktop/src/lib/bindings/`); campo `u64` vira `bigint` a menos que tenha `#[ts(type = "number")]`.
- Um `ToolEngine` por tarefa. `run_tool` / `runTool` / `task_id: "root"` não existem mais (D4).
- O gate depende de `cargo` (edition 2024, Rust 1.85+) e `bun`. Testes de sandbox Linux precisam de Landlock (ABI ≥ 1) e, para bloqueio total de rede, user namespace. O Verifier dispara `bun`/`cargo` via PATH. Checkpoints precisam de `git` no PATH.
- `keep_alive: -1` está no corpo do `/api/chat`.
- Compostos continuam recusados (`CompoundCommand`) — o sandbox não liga shell.
- Trajetórias e memória ficam no diretório de dados do app (`CD_AI_DATA_DIR`); nada sobe à rede (decisão 0004).
- O índice em memória do mapa **não** vê edição do usuário no meio da tarefa até o agente escrever ou rodar um comando. Sem watcher (020 D1 / 023).
- Versão: bump **ao mesmo tempo** em `Cargo.toml` (`workspace.package.version`) e `src-tauri/tauri.conf.json`. A CI de release corre em Ubuntu 22.04; build em 24.04 sobe o piso de glibc.
