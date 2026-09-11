# 0006 — Estrutura do monorepo

Status: **aceita** (fixada em 2026-09-11).

## Estrutura fixa

```text
cd-ai/
├── apps/
│   ├── desktop/
│   │   ├── src/               # Next.js (static export): componentes e lib
│   │   ├── public/
│   │   ├── next.config.ts     # output: "export"
│   │   └── package.json
│   └── cli/                   # binário Rust headless (cd-ai)
├── crates/
│   └── agent-core/            # demais crates só quando uma fronteira real surgir
├── src-tauri/                 # adaptador desktop: commands/events → agent-core
├── evals/                     # Fase 6: tasks/, repos/, results/
├── docs/
│   ├── audit/
│   ├── decisions/
│   └── design/
├── plans/                     # planos de implementação
├── scripts/
├── Cargo.toml                 # Cargo workspace
├── package.json               # Bun workspaces
├── bun.lock
├── biome.json
└── SPEC.md
```

## Esclarecimento (2026-09-11)

- A estrutura é **fixa** e reflete o repositório real atual: `src-tauri/` é o adaptador desktop do Tauri na **raiz** do repo (não dentro de `apps/desktop/`). O frontend Next.js vive em `apps/desktop/` (`src/` + `public/`), e a CLI em `apps/cli/`.
- Nenhum diretório, crate ou package vazio "para depois". Cada um nasce quando a primeira funcionalidade real precisar dele.

## Regras

- Não criar diretórios, crates ou packages vazios "para depois". Cada um nasce quando a primeira funcionalidade real precisar dele.
- `packages/contracts`: tipos compartilhados entre o frontend e os eventos/commands do Rust. Nasce quando os tipos IPC deixarem de caber num único arquivo do frontend (`apps/desktop/src/lib/ipc.ts`). A fonte de verdade é o Rust; o TypeScript espelha.
- `evals/` nasce na Fase 6.

## Distribuição

1. Linux x86_64: `.deb` e AppImage (Flatpak e RPM depois).
2. Windows: `.msi` / `.exe`.
3. macOS: `.dmg`.