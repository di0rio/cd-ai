# 0003 — Agent Core em Rust

Status: **aceita**.

## Decisão

O Agent Core é escrito em Rust, num Cargo workspace, e usado por dois binários:

```text
                 ┌── Desktop (src-tauri)
Agent Core ──────┤
                 └── CLI headless (apps/cli)
```

- `src-tauri` é **só o adaptador desktop**: commands e events do Tauri que chamam o core. Não contém lógica de agente.
- A CLI (`caua-ai task "..."`) usa o mesmo core, sem UI. É ela que roda o eval.
- Toda validação de path, permissão, classificação de comando, sandbox e redação de secrets vive no Rust. A webview nunca é fronteira de confiança.

## Motivos

- Uma única fronteira de confiança.
- CLI headless natural para eval e automação.
- Sem sidecar e sem segundo runtime no backend.

## Consequências

- O frontend não tem backend próprio: sem servidor Node, sem API routes.
- Crates são criados sob demanda. Na Fase 1 existem apenas `crates/agent-core`, `src-tauri` e `apps/cli`. Novos crates (`model-provider`, `tool-engine`, `permissions`, `sandbox`, `workspace`, `verifier`, `checkpoints`, …) só nascem quando o vertical slice precisar de uma fronteira real. Até lá, são módulos dentro de `agent-core`.
