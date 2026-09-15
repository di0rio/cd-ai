# Handoff — estado do cd-ai

Estado em 2026-09-15, sobre `main` em `024cc8e` (Fases 0–5 mergeadas, PR #2) mais o trabalho da
**Fase 6** (plano 016). Fontes da verdade: [`AGENTS.md`](../AGENTS.md), [`SPEC.md`](../SPEC.md)
(§26 e §34), [`plans/README.md`](../plans/README.md), [`docs/decisions/`](decisions/).

**Gate:** qualquer mudança só está pronta com `bun run verify` passando (exit 0).

## O que já está em `main` (Fases 0–5)

Loop de ponta a ponta no core, Tauri (`start_task` / `resume_task` / `cancel_task` / `steer_task`)
e CLI `cd-ai task`. Fixture `evals/fixtures/soma/`. O aceite ao vivo da Fase 5 pela CLI em
2026-09-12 **reprovou** (timeout de aprovação); registro em
[`docs/audit/fase-5-aceite.md`](audit/fase-5-aceite.md). O teste determinístico com
`ScriptedModel` passa.

## Fase 6 — Eval baseline (plano 016)

- Suíte em `evals/tasks/` + fixtures `soma`, `greet`, `dobro`.
- Runner headless: `cd-ai eval --model <nome>` (Ollama) e `cd-ai eval --scripted` (dry-run).
- Cada tarefa tem check (`bun test`) e timeout. Relatório em `evals/results/` (taxa, iterações,
  tokens, tempo, retries, falhas de formato, edições rejeitadas).
- O eval auto-aprova **só** na cópia da fixture. `cd-ai task` continua sem `--yes`.
- Baseline scripted e o comando da taxa ao vivo: [`docs/audit/fase-6-baseline.md`](audit/fase-6-baseline.md).

## Próximo passo

Fase 7 (permissões/sandbox, SPEC §34), **depois** de registrar a taxa ao vivo com
`qwen3-coder:30b` se ainda não estiver na tabela do audit.

Fora de escopo enquanto a 016 não fecha: verifier, shadow git, context manager, skills, router,
release.

## Pontos de atenção

- Código e comentários em inglês; UI e documentação em pt-BR.
- Fronteira de confiança é o Rust.
- Ampliar a suíte é um JSON em `evals/tasks/` + uma pasta em `evals/fixtures/`. O runner não muda.
- O `bun run verify` **não** dispara o eval ao vivo. Os testes scripted do harness entram via
  `cargo test --workspace`.
