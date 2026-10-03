# 0001 — Plataforma de desenvolvimento e alvo

Status: **aceita**.

## Contexto

A máquina de desenvolvimento roda Windows 11 sem WSL. O alvo do primeiro release é Linux x86_64. O sandbox de shell (bubblewrap/landlock/namespaces) e o empacotamento `.deb`/AppImage exigem Linux.

## Decisão

- **Fases 1–6:** desenvolvimento no Windows 11 nativo. Tauri, Rust e Ollama (com GPU) funcionam nativamente.
- **Alvo do primeiro release:** Linux x86_64 (`.deb` e AppImage).
- **Linux entra obrigatoriamente** (WSL2 ou VM) na Fase 7 (sandbox de shell) e na Fase 14 (empacotamento e instalação limpa).

## Regras de portabilidade desde o início

- Paths sempre via APIs de path (`std::path`), sem assumir separador nem letra de drive.
- Validação de workspace cobre os casos de Windows (letras de drive, UNC, junctions) e de Linux (symlinks).
- Execução de comandos passa por um módulo de plataforma: o core não chama `cmd`, `powershell` ou `bash` diretamente.
- **Fase 7 (2026-09-16):** no Linux, `run_command` entra em sandbox (Landlock + user/net namespace). Rede bloqueada por padrão; só um comando da classe `network` **aprovado** ganha rede. Windows/macOS continuam sem sandbox de SO: FULL ACCESS indisponível, e comando `write` em AUTO pergunta.
- **Sandbox nos três sistemas (2026-09-26):**
  - Linux: as pastas graváveis são o workspace, os temporários e só os caches de pacote (`~/.cargo/registry`, `~/.cargo/git`, `~/.bun/install/cache`, `~/.npm/_cacache`, subpastas de `~/.cache`). Nada que rode depois fora do sandbox (`~/.cargo/bin`, `~/.rustup`, `~/.bun/bin`, `~/.local/share`, `~/.cache/pre-commit`, corepack).
  - macOS: `sandbox-exec` (Seatbelt) com o mesmo conjunto gravável e `deny network*`.
  - Windows: AppContainer, iniciado pelo próprio executável em modo launcher (`agent_core::sandbox::init()` no `main`). O container recebe por ACL o workspace (modificar), os caches (modificar) e, só leitura, as toolchains do PATH dentro do perfil. Sem rede, nem loopback. Um comando `network` aprovado roda fora do container (lá dentro ele perderia loopback, gerenciador de credenciais e `~/.ssh`).
  - Windows exige um passo único de administrador, `cd-ai sandbox-setup`: git, Rust e Node resolvem o nome real de cada caminho listando as pastas-pai, e `C:\` e `C:\Users` não deixam nenhum AppContainer listá-las. O setup dá só a listagem dessas pastas (nunca o conteúdo). Sem ele o probe reporta indisponível e tudo continua como ASK.

## Esclarecimento (2026-09-11)

- **Linux é o alvo de release; Windows é o ambiente de desenvolvimento completo** (não "fases 1–6 e depois abandona").
- **Segurança básica de execução existe desde o Tool Engine** (Fase 4): classificação de comando e aprovação. A Fase 7 adiciona o **sandbox de OS** (isolamento de processos), que é uma camada a mais sobre essa base, não o começo da segurança.

## Consequências

- Testes de sandbox: Linux em `crates/agent-core/src/sandbox/linux.rs`, macOS em `sandbox/macos.rs` (pulam quando `sandbox-exec` falta), Windows ponta a ponta em `crates/agent-core/tests/windows_sandbox.rs` (binário próprio, porque é também o launcher). Sem sandbox pronto, o probe reporta indisponível e a policy rebaixa FULL ACCESS para ASK.
- CI local precisa rodar os testes de path em ambas as plataformas quando o Linux estiver disponível.

## Limites do sandbox (auditoria de segurança, 2026-10-03)

O sandbox é uma camada a mais; o que ele **não** promete:

- **Sem sandbox pronto** (Windows sem `cd-ai sandbox-setup`, macOS sem `sandbox-exec`, outros sistemas): `run_command` roda como o usuário, sem isolamento algum. FULL ACCESS fica indisponível e comandos `write` perguntam, mas comandos `read` continuam automáticos, mas `validate` (`npm test`, `cargo test`, `make test`) agora **pergunta em todos os modos**: ele executa código do próprio repositório (`build.rs`, scripts de pacote), e sem sandbox isso roda como o usuário. `cd-ai sandbox-setup` (Windows) restaura a execução automática. No CLI sem terminal interativo a aprovação é negada e a tarefa termina como não validada, com a razão dita; `cd-ai eval` continua aprovando sozinho. Confira `sandbox_status` (UI) ou `cd-ai sandbox-setup` antes de abrir um repositório que você não escreveu.
- **Leitura:** Linux (Landlock) e macOS (Seatbelt) deixam o sistema inteiro legível (`~/.ssh`, `~/.aws`); só a escrita e a rede são limitadas. O AppContainer do Windows é mais restrito: lê só o que recebeu por ACL, além do que o Windows abre a qualquer AppContainer (pastas do sistema).
- **Escrita no workspace inclui o que roda depois, fora do sandbox:** `.git/hooks`, `.git/config`, `package.json`, `Makefile`, `.vscode/tasks.json`. As ferramentas `edit_file`/`write_file` recusam `.git`, mas um comando sandboxed pode escrevê-lo. Revise o diff antes de rodar esses arquivos por conta própria.
- **Comando `network` aprovado** perde o isolamento de rede e, no Windows, também o do sistema de arquivos (roda fora do container). Aprove só o que você leu.
- **Corrida entre a checagem do path e a abertura do arquivo** (TOCTOU): `Workspace::resolve` valida e as ferramentas abrem em seguida. Um processo que sobreviva a um comando (daemonizado) poderia trocar um diretório por um symlink nesse intervalo.
