# Fase 0 — Relatório de ambiente

Data: 2026-09-10. Coletado com comandos reais na máquina de desenvolvimento.

## Hardware

| Item | Valor |
|---|---|
| CPU | AMD Ryzen 5 5600X — 6 cores / 12 threads |
| RAM | 31,9 GB total (17,1 GB livres no momento da medição) |
| GPU | NVIDIA GeForce GTX 1660 — 6 GB VRAM (1,8 GB já em uso), driver 610.62 |
| Disco | C: 228 GB livres, D: 82 GB livres |
| OS | Windows 11 Pro 10.0.26200, x64 |

## Ferramentas

| Ferramenta | Estado |
|---|---|
| Bun | 1.3.13 |
| Node.js | v24.11.1 (npm 11.19.0) |
| Git | 2.51.0.windows.2 |
| MSVC Build Tools | Visual Studio 2022 Build Tools presente |
| WebView2 | 152.0.4191.66 |
| Rust (rustup/rustc/cargo) | **ausente** |
| Tauri CLI | **ausente** (depende do Rust) |
| ripgrep | **ausente** |
| Ollama | **ausente** (binário não encontrado, API em `localhost:11434` offline) |
| WSL | **não instalado** (`wsl --version` retorna "Class not registered") |

## Consequências

1. **A Fase 1 está bloqueada até o Rust ser instalado.** Os pré-requisitos Windows do Tauri (MSVC Build Tools e WebView2) já estão presentes.
2. **A Fase 3 está bloqueada até o Ollama ser instalado.** Não há modelos locais ainda, então a escolha de modelos da decisão 0002 é provisória até o benchmark.
3. **O Linux ainda não está disponível** (sem WSL). Sandbox de shell (Fase 7) e empacotamento `.deb`/AppImage (Fase 14) exigem Linux. Ver decisão 0001.
4. **A memória é a restrição dominante para modelos.** 6 GB de VRAM não comportam nenhum dos modelos candidatos de 14B+ inteiros na GPU. Eles rodarão com offload parcial, majoritariamente em CPU/RAM. Com ~17 GB de RAM livre, um modelo de ~19 GB mais o KV cache do contexto fica apertado. Ver decisão 0002.

## Pendências (Fase 0b)

- [x] Instalar Rust via rustup — rustup 1.29.1, rustc/cargo 1.98.1, `stable-x86_64-pc-windows-msvc`.
- [x] Instalar Tauri CLI — `@tauri-apps/cli` 2.11.4 como devDependency (sem `cargo install`).
- [x] Instalar Ollama — 0.34.0, API online em `localhost:11434`, nenhum modelo baixado.
- [x] Instalar ripgrep — 15.2.0.
- [ ] Baixar os modelos candidatos (ação explícita do usuário) e rodar o benchmark da decisão 0002.
- [ ] Definir o ambiente Linux (decisão 0001).

## Riscos

| Risco | Impacto | Mitigação |
|---|---|---|
| Modelos candidatos lentos neste hardware | Agente pouco utilizável | Benchmark antes de fixar; MoE com poucos parâmetros ativos; contexto modesto |
| RAM livre insuficiente para modelo + contexto + app + builds | Swap, travamentos | Um único modelo residente; medir com o app e o build abertos |
| Sem Linux | Sandbox e pacotes Linux atrasam | Decisão 0001 |
| Next.js dentro do Tauri (static export) | Atrito com recursos server do Next | Decisão 0005 lista o que é proibido |
