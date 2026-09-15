# Aceite da Fase 5 — registro

> **Duas tentativas.** A CLI ao vivo, em 2026-09-12, REPROVOU (`taskTimeout` na espera de aprovação). Em 2026-09-15 o loop foi reexecutado de forma determinística sobre o mesmo fixture (`ScriptedModel` + `bun test`) e PASSOU. CLI/UI com um modelo real **não** rodaram em 2026-09-15: este ambiente não tem Ollama.

Pré-requisito: as Partes 0 a G do plano 015 com o gate (`bun run verify`) verde — ver `docs/fase-5-o-que-falta.md` para o estado atual.

## Procedimento (do plano 015, seção "Aceite da Fase 5")

1. Copie `evals/fixtures/soma/` para uma pasta temporária **fora do repositório**.
2. Com o Ollama rodando e o modelo CODER da decisão 0002 disponível, rode pela CLI:

   ```bash
   cd-ai task --workspace <cópia> --model <modelo> "O teste de soma falha. Corrija e rode os testes."
   ```

   Aprove as ações pelo terminal.
3. **Esperado:** o agente lê o arquivo, edita a função, roda `bun test` (classe `validate`), e o comando sai com código 0 e status `completed_unvalidated`, com evidências (comandos com exit code e arquivos alterados).
4. Repita o mesmo pedido, sobre uma cópia nova do fixture, pela UI (abrir o workspace, iniciar a tarefa pelo composer, aprovar as ações pela barra de aprovação).
5. Registre o resultado, o modelo e o tempo na tabela abaixo.

## Registro dos casos

| Cenário (CLI / UI) | Modelo usado | Tempo | Status final esperado | Resultado | Evidências |
|---|---|---|---|---|---|
| CLI | `qwen3-coder:30b` | 67 min | `completed_unvalidated` | **FALHOU** — terminou em `failed` com `stopReason: taskTimeout`. A correção do código estava certa (`src/soma.ts` passou a `return a + b`), mas a tarefa morreu antes de rodar os testes. | `task_1789209104359_14900`: 5 iterações, `commands: []`, `filesChanged: ["src/soma.ts"]`, `metrics.modelMs: 97_952`, `metrics.toolMs: 3_532_469`. Estado em `%APPDATA%\cd-ai\tasks\task_1789209104359_14900\`. |
| UI | — | — | `completed_unvalidated` | _(não executado — 2026-09-12 nem 2026-09-15)_ | |
| Loop determinístico (sem Ollama) | `ScriptedModel` (não é um LLM) | < 1 s | `completed_unvalidated` | **PASSOU** (2026-09-15). O loop real leu o fixture, trocou `a - b` por `a + b`, rodou `bun test` (exit 0, classe `validate`) e terminou `completed_unvalidated` com `validated: false`. | `cargo test -p agent-core soma_fixture_is_fixed_and_its_tests_run`. Cópia do fixture montada em `tempdir`, nunca a pasta do repositório. |
| CLI ao vivo (2026-09-15) | — | — | `completed_unvalidated` | **Não executado.** `curl http://127.0.0.1:11434/api/tags` falhou com conexão recusada. | Ambiente do agente em nuvem, sem daemon Ollama. |
| UI ao vivo (2026-09-15) | — | — | `completed_unvalidated` | **Não executado** (mesmo motivo). | |

- **Cenário:** `CLI` ou `UI`.
- **Modelo usado:** nome exato como carregado no Ollama (ex.: `qwen3-coder:30b`).
- **Tempo:** duração total da tarefa, do início ao `TaskFinished`.
- **Status final esperado:** sempre `completed_unvalidated` (decisão D11 do plano 015 — a Fase 5 nunca produz `completed`; o Verifier é a Fase 8). Se o resultado real for outro status (`failed`, `cancelled` etc.), registre-o na coluna "Resultado" e trate como reprovação do aceite, não ajuste esta coluna.
- **Resultado:** PASSOU / FALHOU, com o status final observado e uma frase do porquê.
- **Evidências:** exit code do `bun test`, arquivos alterados (`src/soma.ts`), e onde ficou o log/transcript da tarefa (`task_events` ou `%APPDATA%\cd-ai\tasks\<id>\`).

## Análise da reprovação de 2026-09-12 (CLI)

O agente fez a parte dele: em 5 iterações leu o arquivo e corrigiu o bug proposital. O que matou a tarefa foi o orçamento de tempo.

O `modelMs` foi de 98 segundos — o modelo trabalhou pouco mais de um minuto e meio. O `toolMs` foi de 3.532.469 ms, quase 59 minutos, e esse número não é tempo de ferramenta: é o tempo em que a tarefa ficou **parada num prompt de aprovação**, esperando a resposta do operador no terminal. Como o deadline da tarefa (`AgentLimits::task_timeout_ms`, padrão 45 min) era medido com `Instant::elapsed()` em `agent/runner.rs`, ou seja, relógio de parede puro, a espera humana consumiu o orçamento e o loop parou com `TaskTimeout` antes de chegar ao `bun test`.

Duas consequências, ambas corrigidas depois desta tentativa:

1. **Qualquer tarefa em que o operador se ausenta morre de timeout**, por melhor que o agente esteja indo. Com humano no loop, o aceite era praticamente impossível de passar.
2. **A métrica mentia.** O `tool_ms` mostrado no relatório media o tempo de decisão do operador como se fosse execução de ferramenta.

Uma terceira observação, de desempenho e não de corretude: com o prompt parado por 59 minutos, o Ollama descarregou o modelo da memória (o `keep_alive` padrão é de 5 minutos e o cliente não manda um valor próprio), então a retomada depois da aprovação pagou o recarregamento do modelo do disco.

Antes de repetir o aceite **ao vivo**, confirme que o binário em uso já contém as correções desta data — em especial a aprovação automática para comandos das classes `read` e `validate` (SPEC §20.4), sem a qual o `bun test` do fixture também pararia para pedir aprovação — e o `keep_alive: -1` no cliente Ollama, senão o CODER descarrega nos 5 minutos padrão.

## Fechamento de 2026-09-15

O que este ambiente **pôde** verificar, com evidência:

- O loop (`run_task`) resolve o fixture `evals/fixtures/soma/` de ponta a ponta quando as tool calls vêm certas: arquivo corrigido, `bun test` exit 0, status `completed_unvalidated`, relatório sem `validated`.
- `keep_alive: -1` entra no corpo de `POST /api/chat` (`request_body_keeps_the_model_resident`).
- A UI em `main` já chama `startTask` / `respondApproval` / `steerTask` / `cancelTask` / `resumeTask`; `run_tool` e `task_id: "root"` não existem.

O que **não** pôde ser verificado aqui: um modelo local real (CODER) resolvendo o mesmo pedido pela CLI ou pela UI. Isso continua o último passo humano do aceite da Fase 5, numa máquina com Ollama.
