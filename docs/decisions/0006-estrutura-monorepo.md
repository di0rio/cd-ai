# 0006 — Estrutura do monorepo

Status: **aceita**.

## Estrutura alvo

```text
caua-ai/
├── apps/
│   ├── desktop/          # Next.js (static export)
│   └── cli/              # binário Rust headless
├── crates/
│   └── agent-core/       # demais crates só quando necessário (decisão 0003)
├── src-tauri/            # adaptador desktop: commands/events → agent-core
├── packages/             # criados sob demanda: ui, contracts, config, skills
├── evals/
│   ├── tasks/
│   ├── repos/
│   └── results/
├── docs/
│   ├── audit/
│   └── decisions/
├── scripts/
├── Cargo.toml            # Cargo workspace
├── package.json          # Bun workspaces
├── bun.lock
├── biome.json
└── SPEC.md
```

## Regras

- Não criar diretórios, crates ou packages vazios "para depois". Cada um nasce quando a primeira funcionalidade real precisar dele.
- `packages/contracts`: tipos compartilhados entre o frontend e os eventos/commands do Rust. Nasce quando os tipos IPC deixarem de caber num único arquivo do frontend (`apps/desktop/src/lib/ipc.ts`). A fonte de verdade é o Rust; o TypeScript espelha.
- `evals/` nasce na Fase 6.

## Distribuição

1. Linux x86_64: `.deb` e AppImage (Flatpak e RPM depois).
2. Windows: `.msi` / `.exe`.
3. macOS: `.dmg`.
