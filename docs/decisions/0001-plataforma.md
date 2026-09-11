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
- Até a Fase 7 não existe sandbox: **todo comando fora das classes `read`/`validate` exige aprovação**, em qualquer modo de permissão.

## Consequências

- Testes de sandbox e pacotes Linux só rodam a partir da Fase 7.
- CI local precisa rodar os testes de path em ambas as plataformas quando o Linux estiver disponível.
