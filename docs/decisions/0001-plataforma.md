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

## Esclarecimento (2026-09-11)

- **Linux é o alvo de release; Windows é o ambiente de desenvolvimento completo** (não "fases 1–6 e depois abandona").
- **Segurança básica de execução existe desde o Tool Engine** (Fase 4): classificação de comando e aprovação. A Fase 7 adiciona o **sandbox de OS** (isolamento de processos), que é uma camada a mais sobre essa base, não o começo da segurança.

## Consequências

- Testes de sandbox Linux rodam na Fase 7 (`crates/agent-core/src/sandbox.rs`). No Windows/macOS o probe reporta indisponível e a policy rebaixa FULL ACCESS para ASK.
- CI local precisa rodar os testes de path em ambas as plataformas quando o Linux estiver disponível.
