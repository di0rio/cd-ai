# Planos de implementação

Gerados pela skill `improve` em 2026-09-11.

**Geração 1** (001–009): a partir da árvore de trabalho sobre o commit `158f609`.
**Geração 2** (010–013): auditado em cima do commit `f6e7764` (que já contém 001–009).

**Antes de executar qualquer plano:** commite o estado atual e use esse commit como base do drift check de cada plano.

Cada executor deve:

1. Ler o plano inteiro antes de começar.
2. Respeitar as STOP conditions.
3. Atualizar a sua linha na tabela quando terminar.

## Ordem de execução e status

| Plano | Título | Prioridade | Esforço | Depende de | Status |
|------|--------|-----------|---------|------------|--------|
| 001 | Comando único de verificação e guia de agentes na raiz | P1 | S | — | DONE |
| 002 | Módulo `workspace` no agent-core com validação de path | P1 | M | 001 | DONE |
| 003 | Abrir workspace pela UI (diálogo nativo + IPC) | P1 | M | 002 | DONE |
| 004 | Provider do Ollama no Rust + status real na sidebar | P1 | M | 001 | DONE |
| 005 | Chat em streaming cancelável com o Ollama (core, IPC, CLI) | P2 | L | 004 | DONE |
| 006 | Permissões mínimas no Tauri (ACL para os commands do app) | P2 | S | 003, 004, 005 | DONE |
| 007 | Remover o `style=` inline gerado pelo `next/image` | P3 | S | 001 | DONE |
| 008 | Gerar os tipos TypeScript do IPC a partir do Rust | P2 | M | 003, 004 | DONE |
| 009 | Parser tolerante de tool calls no formato `<function=…>` | P2 | S | 001 | DONE |

### Geração 2

| Plano | Título | Prioridade | Esforço | Depende de | Status |
|------|--------|-----------|---------|------------|--------|
| 010 | Conversa segue eventos vivos (auto-scroll + comando que falha abre) | P1 | S | — | DONE |
| 011 | Testes de caracterização da CLI (antes do 005 reescrever `main.rs`) | P1 | S | — | DONE |
| 012 | Design/spike da Fase 4: Tool Engine, permissões, redator, eventos | P1 | M | leitura de 001–009 | DONE |
| 013 | Tooltip volta a funcionar em botões de ícone desabilitados | P3 | S | — | DONE |
| 014 | Implementar a Fase 4 (tools, permissões, redator, eventos) | P1 | L | 002, 005, 006, 009, 010, 012 | DONE |

### Geração 3

Escrita pelo lead em 2026-09-11 sobre o commit `040369a`. Ponto de partida: `docs/handoff.md`.

| Plano | Título | Prioridade | Esforço | Depende de | Status |
|------|--------|-----------|---------|------------|--------|
| 015 | Fase 5 — loop de ponta a ponta (core, bridge, CLI e UI) | P1 | L | 014 | DONE |

O plano 009 foi adicionado depois do benchmark de modelos (`docs/audit/benchmark-2026-09-11.md`). O modelo CODER escolhido escreve tool calls num formato que o Ollama 0.34 não converte.

**Estado do 015 em 2026-09-15:** Partes 0 e A–G em `main` (PR #2); fechamento com `keep_alive: -1`, `Child::kill` se o kill do grupo falhar, teste determinístico no fixture `soma/` e documentação alinhada. O aceite ao vivo com Ollama+CODER **não** rodou neste ambiente (sem daemon); o loop no mesmo fixture, com `ScriptedModel`, passou. Detalhe em `docs/fase-5-o-que-falta.md` e `docs/audit/fase-5-aceite.md`.

### Geração 4

Escrita em 2026-09-15 sobre o `main` com as Fases 0–5 fechadas.

| Plano | Título | Prioridade | Esforço | Depende de | Status |
|------|--------|-----------|---------|------------|--------|
| 016 | Fase 6 — Eval baseline (suite, runner headless, número registrado) | P1 | M | 015 | DONE |

### Geração 5

Escrita em 2026-09-16 sobre o `main` com as Fases 0–6 fechadas (PR #3).

| Plano | Título | Prioridade | Esforço | Depende de | Status |
|------|--------|-----------|---------|------------|--------|
| 017 | Fase 7 — Permissões e sandbox (modos ASK/AUTO/FULL ACCESS, Landlock+netns, conteúdo não confiável) | P1 | L | 016 | DONE |

### Geração 6

Escrita em 2026-09-16 sobre o `main` com as Fases 0–7 fechadas (PR #4).

| Plano | Título | Prioridade | Esforço | Depende de | Status |
|------|--------|-----------|---------|------------|--------|
| 018 | Fase 8 — Verifier e ciclo de correção (checks determinísticos, review LLM opcional, `completed`) | P1 | M | 017 | DONE |

### Geração 7

Escrita em 2026-09-16 sobre o `main` com as Fases 0–8 fechadas (PR #5).

| Plano | Título | Prioridade | Esforço | Depende de | Status |
|------|--------|-----------|---------|------------|--------|
| 019 | Fase 9 — Checkpoints, Git e histórico (shadow repo, rollback seguro, tools de git, histórico por workspace) | P1 | L | 018 | DONE |

### Geração 8

Escrita em 2026-09-16 sobre o `main` com as Fases 0–9 fechadas (PR #7).

| Plano | Título | Prioridade | Esforço | Depende de | Status |
|------|--------|-----------|---------|------------|--------|
| 020 | Fase 10 — Context Manager completo (repo map, orçamento por seção, cache, higiene, Explorer) | P1 | L | 019 | DONE |

Valores de status: TODO | IN PROGRESS | DONE | BLOCKED (com motivo de uma linha) | REJECTED (com justificativa de uma linha).

## Notas de dependência

- **001 vem primeiro.** Ele cria o `bun run verify`, que todos os outros usam como gate.
- **003 depende de 002:** usa `Workspace` e `WorkspaceError`.
- **005 depende de 004:** usa o `OllamaClient` e os tipos de modelo.
- **006 vem depois de 003, 004 e 005:** precisa listar todos os commands do app que existirem.
- **008 vem depois de 003 e 004:** gera os tipos que eles criam.
- **Não rode 002, 004 e 007 em paralelo sem coordenar.** 002 e 004 editam `crates/agent-core/src/lib.rs` e `crates/agent-core/Cargo.toml`. 004 e 007 editam `apps/desktop/src/components/sidebar.tsx`.

### Geração 2

- **011 antes de 005.** O 005 reescreve `apps/cli/src/main.rs`; os testes do 011 travam o comportamento atual antes disso.
- **012 produz o design da Fase 4** (`docs/design/fase-4-tool-engine.md`). A implementação das ferramentas/redator/permissões é o plano **014**, reservado, dependente de 002, 005, 006, 009, 010 e 012; o executor de 014 deve citar o documento de design (fonte da verdade da Fase 4).
- **010 é pré-requisito de UI** para ligar o stream de eventos do Tool Engine à conversa (a conversa hoje assume lista estática).

### Geração 3

- **O 015 é dividido em partes com dono** (0 e A a G) e executado em 3 ondas (ver "Ondas e notas de dependência" no próprio plano).
  - **Onda 1:** `core` faz 0 → A → B → C → D em sequência, porque todas editam o crate `agent-core`. A Parte 0 conserta os testes de `tools::command`, que falham no Windows na base `040369a`, então o gate está vermelho hoje. Em paralelo, `ui` faz a G1, só com os bindings que já existem.
  - **Onda 2:** `bridge` (E) e `cli` (F), em crates diferentes.
  - **Onda 3:** `ui` faz a G2.
- **Arquivos disputados têm um dono por vez:**
  - `bindings/` e `lib.rs` do core ficam só com o `core`;
  - `apps/cli/src/main.rs`: A, depois F;
  - `conversation.tsx`: G1, depois G2;
  - `ipc.ts` fica só com a E.
- **Sem worktrees:** todos usam o mesmo working tree. Um gate vermelho causado por arquivo fora do escopo de quem roda espera o aviso do lead e roda de novo; ninguém reporta pronto com o gate vermelho.

### Geração 4

- **016 depois do 015.** O runner chama `run_task`; não reimplementa o loop. `--yes` no `task` continua proibido (D13 do 015); só o `eval` auto-aprova, e só na cópia da fixture.

### Geração 5

- **017 depois do 016.** O sandbox envolve `run_command`; a policy é função pura. FULL ACCESS exige sandbox Linux completo. Allowlist por workspace (§30) fica de fora. O eval scripted não cresce — só não pode regressar.

### Geração 6

- **018 depois do 017.** O Verifier chama `run_command` já classificado e sandboxed. Não liga shell. `completed` só com evidência (validate exit 0 após a última edição). A suíte de eval não cresce.

### Geração 7

- **019 depois do 018.** O shadow git vive no diretório de dados do app, não no workspace. Rollback compara `hash_after` com o disco e restaura do baseline; não usa o `.git` do utilizador. A suíte de eval não cresce.

### Geração 8

- **020 depois do 019.** O Context Manager vive no `agent-core`. Explorer nas tarefas de código é briefing determinístico (mapa + notas), não um turno extra de LLM — senão o eval scripted quebra. A suíte não cresce.

## Achados considerados e descartados

- **`.env` na raiz do repositório:** está no `.gitignore` e não é rastreado pelo git. Não é problema. Não foi lido.
- **`@types/node` 22 com Node 24:** sem efeito, porque o frontend roda na webview e não no Node.
- **Testes de componentes React agora:** a UI ainda é estática, com dados de demonstração. Escrever esses testes quando ela for ligada a dados reais.
- **Troca de modelo do Ollama durante o benchmark:** o script já foi corrigido na mesma sessão para não recarregar o modelo entre as fases.

### Geração 2

- **`tasks === demoTasks` para o badge de demonstração** (`app-shell.tsx:18`): frágil por construção, mas dev-only, funciona hoje e desaparece naturalmente quando os dados reais chegarem. Não vale plano.
- **Bugs de produção nos componentes estáticos** (ex.: `title` em `empty-workspace.tsx`/`sidebar.tsx`): verificados um a um; só `icon-button.tsx` tinha o problema real de `pointer-events-none` → coberto pelo plano 013.
- **Rust sem `cargo` neste ambiente de auditoria:** não foi possível rodar `cargo test`/`clippy` aqui; a auditoria do Rust foi por leitura. Os planos 001–011 exigem `bun run verify` numa máquina com cargo (Windows de dev).
