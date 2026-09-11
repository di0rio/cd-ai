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
| 006 | Permissões mínimas no Tauri (ACL para os commands do app) | P2 | S | 003, 004, 005 | TODO |
| 007 | Remover o `style=` inline gerado pelo `next/image` | P3 | S | 001 | TODO |
| 008 | Gerar os tipos TypeScript do IPC a partir do Rust | P2 | M | 003, 004 | TODO |
| 009 | Parser tolerante de tool calls no formato `<function=…>` | P2 | S | 001 | TODO |

### Geração 2

| Plano | Título | Prioridade | Esforço | Depende de | Status |
|------|--------|-----------|---------|------------|--------|
| 010 | Conversa segue eventos vivos (auto-scroll + comando que falha abre) | P1 | S | — | TODO |
| 011 | Testes de caracterização da CLI (antes do 005 reescrever `main.rs`) | P1 | S | — | DONE |
| 012 | Design/spike da Fase 4: Tool Engine, permissões, redator, eventos | P1 | M | leitura de 001–009 | TODO |
| 013 | Tooltip volta a funcionar em botões de ícone desabilitados | P3 | S | — | TODO |

O plano 009 foi adicionado depois do benchmark de modelos (`docs/audit/benchmark-2026-09-11.md`). O modelo CODER escolhido escreve tool calls num formato que o Ollama 0.34 não converte.

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
- **012 produz o design da Fase 4.** Qualquer implementação das ferramentas/redator/permissões será o plano 014, reservado, dependente de 002, 005, 006, 009, 010 e 012.
- **010 é pré-requisito de UI** para ligar o stream de eventos do Tool Engine à conversa (a conversa hoje assume lista estática).

## Achados considerados e descartados

- **`.env` na raiz do repositório:** está no `.gitignore` e não é rastreado pelo git. Não é problema. Não foi lido.
- **`@types/node` 22 com Node 24:** sem efeito, porque o frontend roda na webview e não no Node.
- **Testes de componentes React agora:** a UI ainda é estática, com dados de demonstração. Escrever esses testes quando ela for ligada a dados reais.
- **Troca de modelo do Ollama durante o benchmark:** o script já foi corrigido na mesma sessão para não recarregar o modelo entre as fases.

### Geração 2

- **`tasks === demoTasks` para o badge de demonstração** (`app-shell.tsx:18`): frágil por construção, mas dev-only, funciona hoje e desaparece naturalmente quando os dados reais chegarem. Não vale plano.
- **Bugs de produção nos componentes estáticos** (ex.: `title` em `empty-workspace.tsx`/`sidebar.tsx`): verificados um a um; só `icon-button.tsx` tinha o problema real de `pointer-events-none` → coberto pelo plano 013.
- **Rust sem `cargo` neste ambiente de auditoria:** não foi possível rodar `cargo test`/`clippy` aqui; a auditoria do Rust foi por leitura. Os planos 001–011 exigem `bun run verify` numa máquina com cargo (Windows de dev).
