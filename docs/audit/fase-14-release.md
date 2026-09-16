# Fase 14 — Release Linux (auditoria)

Medido em 2026-09-16. Plano: [`plans/024-fase-14-release.md`](../../plans/024-fase-14-release.md). Ambiente desta sessão: Ubuntu 24.04 x86_64, rustc 1.98.1, bun 1.4.2, WebKitGTK 4.1. **Sem Ollama.**

## O que o pacote deveria provar

Saída da SPEC §34 Fase 14: Linux x86_64 `.deb` + AppImage; app instalado do zero resolve uma tarefa do eval.

Caminho escolhido (CLI no pacote + suíte em `/usr/share/cd-ai/evals`):

```bash
cd-ai eval --scripted --suite /usr/share/cd-ai/evals --out /tmp/cd-ai-eval.json
```

## Evidência (preencher após o gate e o bundle)

| Prova | Resultado | Onde |
|---|---|---|
| `bun run verify` | *pendente* | esta VM |
| `cd-ai eval --scripted` (árvore de dev) | *pendente* | esta VM |
| `bun scripts/build-linux-release.ts` | *pendente* | esta VM (glibc 2.39) |
| `cd-ai --version` no binário do `.deb` | *pendente* | esta VM |
| eval 3/3 com CLI + `--suite` do pacote | *pendente* | esta VM; precisa de Bun no PATH para `bun test` |
| GUI abre | *pendente* | precisa de display |
| eval ao vivo com Ollama | **não corrido** | sem daemon nesta VM |
| artefato glibc 2.35 (Ubuntu 22.04) | CI [`.github/workflows/linux-release.yml`](../../.github/workflows/linux-release.yml) | não é esta VM |

## Caminho ao vivo (Ollama + CODER)

```bash
cd-ai eval --model qwen3-coder:30b --suite /usr/share/cd-ai/evals --out /tmp/cd-ai-eval-live.json
```

## Fora desta fase

Flatpak, RPM, Windows, macOS, assinatura, updater, LICENSE, `fix-path-env` na GUI. Ver [`docs/release.md`](../release.md).
