# Fase 5 — fechamento

Situação em 2026-09-15. Plano: `plans/015-fase-5-loop-ponta-a-ponta.md`.

## O que já estava em `main` (Partes 0–G)

| Parte | O que entregou |
|---|---|
| 0 | Testes de `run_command` portáveis (Windows e Unix), via `test_argv` |
| A | Tool calling no cliente Ollama: `tools` no request, `tool_calls` na resposta, `ChatEvent::ToolCalls` |
| B | `CancelToken`, cancelamento no `ToolEngine` e morte da árvore de processos |
| C | `agent/`: `TaskState`, `AgentEvent`, `TaskStore` e perfil do workspace |
| D | O loop: `ChatModel`, `OllamaModel`, mapeamento de tool calls, prompt, orçamento e `run_task` |
| E | Bridge Tauri: `start_task`, `resume_task`, `cancel_task`, `steer_task`, `respond_approval`, `list_tasks`, `task_events`; `run_tool` removido |
| F | CLI `cd-ai task` (com `--resume`) e o fixture `evals/fixtures/soma/` |
| G1+G2 | Barra de aprovação, `applyToolEvent` / `applyAgentEvent`, composer, sidebar, retomar |

A revisão de 2026-09-12 deste arquivo dizia que a G2 não tinha começado e que nada estava commitado. Isso ficou defasado no mesmo dia: `57c522b` ligou a UI e os commits da Fase 5 já estavam em `main`.

## O que o fechamento de 2026-09-15 entregou

- `keep_alive: -1` em toda virada do `/api/chat` (o modelo não descarrega nos 5 minutos padrão do Ollama enquanto a UI/CLI espera aprovação).
- Teste `soma_fixture_is_fixed_and_its_tests_run`: o loop real, com `ScriptedModel`, resolve o fixture de aceite (lê, edita, `bun test`, `completed_unvalidated`).
- Handoff, README dos planos e este arquivo alinhados com o git.

## O que este ambiente não verificou

O aceite **ao vivo** (CLI e UI, com Ollama e o CODER da decisão 0002) não rodou aqui: não há daemon em `127.0.0.1:11434`. A tentativa de 2026-09-12 pela CLI reprovou por timeout de espera humana — causa já corrigida no runner. Repetir o procedimento de `docs/audit/fase-5-aceite.md` numa máquina com o modelo carregado.

## Riscos que continuam

- "Editar" o comando na hora da aprovação e o modo plano (read-only) do SPEC §29 ficaram fora da Fase 5 de propósito.
- A compactação automática de contexto é da Fase 10 (decisão 0008). Hoje, contexto estourado termina a tarefa com `ContextExhausted`.
- Sem Verifier (Fase 8) o loop **nunca** produz `completed`.
