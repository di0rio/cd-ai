# Baseline de eval — Fase 6

Registro da primeira taxa de sucesso da suíte em `evals/` (SPEC §26 / §34). O harness é
`cd-ai eval`. Sucesso = exit do **check automático**, não o status `completed_unvalidated` do
agente.

Modelo CODER da decisão 0002: `qwen3-coder:30b`, `num_ctx` 16384.

## Suite

Três tarefas, de propósito pequenas:

| id | O que mede |
|---|---|
| `soma` | `edit_file` num bug óbvio + `bun test` |
| `greet` | `edit_file` numa string + `bun test` |
| `dobro` | `write_file` de um módulo que o teste importa + `bun test` |

## Caminho scripted (harness)

Não fala com o Ollama. Cada tarefa traz um `script` que reproduz as tool calls certas. Serve
para provar o runner, o check e o relatório — **não** é a taxa do modelo.

```bash
cargo run -q -p cd-ai-cli -- eval --scripted --out evals/results/scripted-baseline.json
```

| Data | Commit | Modelo | Taxa | Passed / scored | Notas |
|---|---|---|---|---|---|
| 2026-09-15 | desta branch (ver JSON) | `scripted` | **100%** | 3/3 | Dry-run do harness. Cada tarefa: 4 iterações, check `bun test` exit 0. JSON em `evals/results/scripted-baseline.json`. |

O JSON correspondente, quando gerado, fica em `evals/results/scripted-baseline.json` (exceção do
gitignore para `*-baseline.json`).

Nesta VM, `bun run verify` corre Biome, typecheck, testes TS, rustfmt, clippy e o `cargo test`
do workspace. Dois testes **já existentes** de cancelamento de processo
(`cancel_kills_running_command` e `cancelling_during_a_command_stops_the_task_quickly`) falham
aqui: `kill -KILL -- -<pgid>` não derruba o `sleep` (o comando segue até os 5 s). Não fazem
parte desta mudança; o resto do gate passou, inclusive a suíte scripted.

## Caminho ao vivo (Ollama + CODER)

Nesta VM de cloud **não havia Ollama nem o modelo CODER**. A taxa ao vivo fica bloqueada até
rodar, numa máquina com o Ollama 0.34+ e `qwen3-coder:30b` puxado (decisão 0002):

```bash
# na raiz do repositório; o Ollama precisa responder em 127.0.0.1:11434 (ou OLLAMA_HOST)
ollama show qwen3-coder:30b
cargo run -q -p cd-ai-cli -- eval --model qwen3-coder:30b --out evals/results/qwen3-coder_30b-baseline.json
```

O eval copia cada fixture para um tempdir e **aprova sozinho**. Não use `cd-ai task` para
isto: sem TTY as edições são negadas (plano 015 D13). Timeout padrão por tarefa: 10 min.

| Data | Commit | Modelo | Taxa | Passed / scored | Notas |
|---|---|---|---|---|---|
| — | — | `qwen3-coder:30b` | **não medida** | — | Bloqueado: Ollama ausente neste ambiente |

Quando a corrida ao vivo existir, acrescente a linha e trate esse número como o baseline de
produto. Mudança de prompt, parser, tools ou formato de edição só fica se o eval não piorar
(SPEC §26).
