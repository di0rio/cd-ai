# Eval local (Fase 6)

Suíte headless do cd-ai (SPEC §26). Cada tarefa copia uma fixture, roda o agente, depois um
**check automático** (`bun test` nestas primeiras tarefas). O sucesso é o exit do check, não o
status do agente.

A pasta `fixtures/` é o repositório de teste (o SPEC chama isso de `repos/`; o nome no disco
ficou o da Fase 5). `tasks/` descreve o pedido, o timeout e o script de dry-run. `results/`
guarda os JSON de cada execução.

A partir da Fase 8 o agente pode terminar `completed` quando o Verifier tem evidência (comando
`validate` exit 0 depois da última edição). O **sucesso da suíte** continua sendo o exit do
check automático, não esse status.

## Como rodar

Na raiz do repositório, com o binário da CLI:

```bash
# Dry-run determinístico (sem Ollama). Usa o campo `script` de cada tarefa.
cargo run -q -p cd-ai-cli -- eval --scripted

# Uma tarefa só
cargo run -q -p cd-ai-cli -- eval --scripted --task soma

# Ao vivo, com o CODER da decisão 0002 (Ollama rodando, modelo baixado)
cargo run -q -p cd-ai-cli -- eval --model qwen3-coder:30b
```

`--suite` aponta para a pasta `evals/` (padrão: `evals` no cwd). `--out` escolhe o JSON;
o padrão é `evals/results/<data>-<modelo>.json`.

O eval **aprova sozinho** as edições e os comandos, porque trabalha numa cópia descartável da
fixture. `cd-ai task` continua pedindo aprovação no terminal — não existe `--yes` lá.

## Tarefas desta baseline

| id | Fixture | O que o agente precisa fazer |
|---|---|---|
| `soma` | `fixtures/soma` | Trocar `a - b` por `a + b` |
| `greet` | `fixtures/greet` | Cumprimentar (`Olá, {nome}`) |
| `dobro` | `fixtures/dobro` | Criar `src/dobro.ts` (`n * 2`) |

A suíte é propositalmente pequena. 20–50 tarefas é o alvo do SPEC §26, não desta fase.

## Relatório

Cada JSON traz taxa de sucesso, e por tarefa: iterações, tokens, tempo, retries, falhas de
formato de tool call, edições rejeitadas, turnos nativos vs. texto, **skills selecionadas**
(Fase 11), **rota do modelo** (`modelCategory`, `routedNumCtx`, `routeReason` — Fase 12) e
**custo de contexto** (`contextMs`, `cacheHits`, `cacheMisses` — Fase 13).
A taxa oficial da baseline está em [`docs/audit/fase-6-baseline.md`](../docs/audit/fase-6-baseline.md).
A prova de latência trivial da Fase 12 está em [`docs/audit/fase-12-router.md`](../docs/audit/fase-12-router.md).
A prova de otimização da Fase 13 está em [`docs/audit/fase-13-otimizacao.md`](../docs/audit/fase-13-otimizacao.md).
