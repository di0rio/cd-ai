# Fase 5 — o que falta

Situação em 2026-09-12. Base: commit `37f8b28` mais a árvore de trabalho **não commitada**.
Plano: `plans/015-fase-5-loop-ponta-a-ponta.md`.

## O que já está na árvore

| Parte | O que entregou | Revisão do lead |
|---|---|---|
| 0 | Testes de `run_command` portáveis (Windows e Unix), via `test_argv` | aprovada |
| A | Tool calling no cliente Ollama: `tools` no request, `tool_calls` na resposta, `ChatEvent::ToolCalls` | aprovada |
| B | `CancelToken`, cancelamento no `ToolEngine` e morte da árvore de processos | aprovada |
| C | `agent/`: `TaskState`, `AgentEvent`, `TaskStore` (em `%APPDATA%\cd-ai`) e perfil do workspace | aprovada |
| D | O loop: `ChatModel`, `OllamaModel`, mapeamento de tool calls, prompt, orçamento de contexto e `run_task` | aprovada |
| E | Bridge Tauri: `start_task`, `resume_task`, `cancel_task`, `steer_task`, `respond_approval`, `list_tasks`, `task_events`; `run_tool` removido; ACL atualizada; wrappers no `ipc.ts` | gate verde, **falta revisão** |
| F | CLI `cd-ai task` (com `--resume`), aprovação pelo terminal, testes novos e o fixture `evals/fixtures/soma/` | gate verde, **falta revisão** |
| G1 | Barra de aprovação e `applyToolEvent` na UI | aprovada |

O gate `bun run verify` foi rodado pelo lead em 2026-09-12 sobre esta árvore e **saiu com exit 0**, já com as Partes E e F dentro (a CLI passou a ter 11 testes).

## O que falta, em ordem

### 1. Fechar as Partes E e F

Os dois teammates foram interrompidos pelo limite de uso **no momento de rodar o gate**. O gate já foi rodado depois disso e passou; falta a revisão do lead e o build do app.

- Rodar `bun tauri build --no-bundle`.
- Revisar o diff de `src-tauri/src/lib.rs`, `src-tauri/build.rs`, `src-tauri/capabilities/default.json`, `apps/desktop/src/lib/ipc.ts`, `apps/cli/src/main.rs` e `apps/cli/tests/cli.rs`.
- Conferir os pontos que o plano exige: as três listas de ACL batendo, o cancelamento respondendo `Denied` às aprovações pendentes, o `resume` validado com `load_state` antes, e nenhum `run_tool` sobrando.

### 2. Parte G2 — ligar a UI ao loop

Ainda não começou. `app-shell.tsx`, `composer.tsx` e `sidebar.tsx` estão intocados.

Falta:

- `applyAgentEvent` em `apps/desktop/src/lib/activity.ts`, com os testes;
- carregar as tarefas do workspace (`listTasks`) e fazer replay de uma tarefa antiga (`taskEvents`);
- composer que começa tarefa, corrige o rumo com a tarefa rodando, cancela e escolhe o modelo pela lista do Ollama;
- "Nova tarefa" habilitado na sidebar;
- a barra de aprovação chamando `respondApproval(taskId, id, granted, reason)`;
- botão "Retomar" numa tarefa interrompida.

### 3. Aceite da Fase 5

O fixture `evals/fixtures/soma/` já existe (teste que falha de propósito). Falta:

1. copiar o fixture para uma pasta temporária fora do repositório;
2. rodar pela CLI, com o Ollama ligado e o modelo CODER da decisão 0002;
3. repetir pela UI;
4. registrar modelo, tempo e resultado em `docs/audit/fase-5-aceite.md` (arquivo ainda não existe).

### 4. Fechamento

- `plans/README.md`: a linha do 015 ainda está `TODO`.
- `docs/handoff.md`: descreve o estado da Fase 4 e precisa refletir a Fase 5. Ele também afirma "gate verde", o que era falso no Windows antes da Parte 0.
- Commit: **nada foi commitado**. São dezenas de arquivos na árvore. O commit e o push dependem de pedido explícito do usuário.

## Riscos e pendências conhecidas

- O gate depende de `cargo`, `bun` e, para o aceite, do Ollama com o modelo baixado.
- A UI ainda não consegue iniciar tarefa nenhuma: sem a G2, o loop só roda pela CLI.
- "Editar" o comando na hora da aprovação e o modo plano (read-only) do SPEC §29 ficaram fora da Fase 5 de propósito.
- A compactação automática de contexto é da Fase 10 (decisão 0008). Hoje, contexto estourado termina a tarefa com `ContextExhausted`.

## Como retomar

```bash
bun run verify
```

```bash
bun tauri build --no-bundle
```

```bash
cargo run -q -p cd-ai-cli -- task --model <modelo> --workspace <pasta> "<pedido>"
```
