# Plano 009: o core reconhece tool calls escritas no texto no formato `<function=…>`

> **Instruções ao executor:** siga o plano passo a passo. Rode cada comando de verificação e confirme o resultado esperado antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Ao terminar, atualize a linha deste plano em `plans/README.md`.
>
> **Drift check (rode primeiro):** confirme que `crates/agent-core/src/lib.rs` ainda não declara um módulo `tool_call`. Se já declarar, STOP.

## Status

- **Prioridade:** P2
- **Esforço:** S
- **Risco:** LOW (função pura, sem I/O)
- **Depende de:** `plans/001-verify-e-guia-de-agentes.md`
- **Categoria:** direction (pré-requisito da Fase 4/5)
- **Planejado em:** base `158f609` + árvore de trabalho de 2026-09-11

## Por que isso importa

O benchmark de 2026-09-11 (`docs/audit/benchmark-2026-09-11.md`) escolheu o `qwen3-coder:30b` como modelo CODER (decisão 0002). Em 5 de 10 casos, ele escreveu tool calls **corretas** no formato nativo dele, dentro de `message.content`, e o Ollama 0.34 não converteu esse formato em `message.tool_calls`. Sem um parser tolerante, o agente trataria essas respostas como "sem ação" e perderia metade das jogadas certas.

O SPEC §4, item 3, já exige um parser tolerante. Este plano entrega a peça pura e testada; o loop do agente vai usá-la depois.

## Estado atual

- Não existe nada parecido no repositório. `crates/agent-core/src/lib.rs` declara no máximo os módulos `workspace` e `ollama` (planos 002 e 004).
- Exemplos **reais** do benchmark (`docs/audit/benchmark-2026-09-11.json`, campo `toolCases[].content` do modelo `qwen3-coder:30b`). Use-os como fixtures, byte a byte:
  1. `"<function=read_file>\n<parameter=path>\nsrc/app/page.tsx\n</parameter>\n</function>\n</tool_call>"`
  2. `"I'll run the test suite using the specified command.\n\n<function=run_command>\n<parameter=command>\ncargo test --workspace\n</parameter>\n</function>\n</tool_call>"`
  3. `"I'll search for all TODO comments in the codebase to help identify pending tasks or areas that need attention.\n\n<function=search>\n<parameter=query>\nTODO\n</parameter>\n</function>\n</tool_call>"`
  4. `"I need to find the file `apps/cli/src/main.rs` and locate the constant `USAGE` that needs to be renamed to `HELP_TEXT`.\n\nFirst, let me search for the USAGE constant in the file:"` (prosa pura, **sem** chamada)
- **Convenções:** sem dependências novas (nada de `regex`), testes no próprio arquivo e identificadores em inglês.

## Comandos que você vai precisar

| Propósito | Comando | Esperado |
|-----------|---------|----------|
| Testes | `cargo test -p agent-core tool_call` | `0 failed` |
| Gate | `bun run verify` | exit 0 |

## Escopo

**Dentro do escopo:**

- `crates/agent-core/src/tool_call.rs` (criar)
- `crates/agent-core/src/lib.rs` (só `pub mod tool_call;`)

**Fora do escopo:**

- Integrar o parser ao chat (plano 005) ou a um loop de agente (ainda não existe).
- Outros formatos (JSON solto no texto, `<tool_call>{json}</tool_call>`). Anote-os em "Notas de manutenção" se aparecerem nos testes manuais.

## Git

- Branch: `advisor/009-tolerant-tool-calls`.
- Mensagem no estilo `feat(core): parse text-embedded tool calls`.
- Não faça push.

## Passos

### Passo 1: API

Crie `crates/agent-core/src/tool_call.rs`:

```rust
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedToolCall {
    pub name: String,
    pub arguments: BTreeMap<String, String>,
}

/// Extracts tool calls a model wrote as text in the `<function=name><parameter=key>value</parameter></function>`
/// format (qwen3-coder), accepting only tool names in `known_tools`. Prose around the calls is ignored.
pub fn parse_text_tool_calls(content: &str, known_tools: &[&str]) -> Vec<ParsedToolCall>;
```

Regras:

- Percorra o texto procurando `<function=`. O nome vai até o próximo `>`. Descarte o bloco se o nome, depois do `trim()`, não estiver em `known_tools`.
- Dentro do bloco, até `</function>`, cada `<parameter=KEY>` vai até o `>`. O valor vai até o próximo `</parameter>`.
- Remova **um** `\n` inicial e **um** `\n` final do valor, se existirem. Não use `trim()` no valor inteiro: espaços dentro de código importam.
- Um bloco sem `</function>` (resposta cortada) é descartado.
- As tags `</tool_call>` e `<tool_call>` soltas são ignoradas.
- Implemente com `str::find` e fatiamento. Sem regex e sem `unwrap()` fora dos testes.

**Verificar:** `cargo build -p agent-core` sai com exit 0 (depois de adicionar `pub mod tool_call;` no `lib.rs`).

### Passo 2: testes

Com `const TOOLS: &[&str] = &["read_file", "search", "edit_file", "run_command"];`, crie:

1. `parses_bare_call`: a fixture 1 vira um `ParsedToolCall` com `name == "read_file"` e `arguments["path"] == "src/app/page.tsx"`.
2. `ignores_leading_prose`: a fixture 2 vira `run_command` com `command == "cargo test --workspace"`.
3. `parses_search`: a fixture 3 vira `search` com `query == "TODO"`.
4. `prose_only_returns_nothing`: a fixture 4 vira um `Vec` vazio.
5. `rejects_unknown_tool`: `"<function=delete_everything>\n<parameter=path>\n/\n</parameter>\n</function>"` vira um `Vec` vazio.
6. `drops_truncated_call`: `"<function=read_file>\n<parameter=path>\nsrc/"` (sem fechamento) vira um `Vec` vazio.
7. `keeps_inner_whitespace`: um `edit_file` com `<parameter=replace>\n    indented line\n</parameter>` preserva `"    indented line"`.
8. `parses_multiple_calls_in_order`: duas chamadas seguidas viram dois itens, na ordem.

**Verificar:** `cargo test -p agent-core tool_call` roda com 8 testes passando.

### Passo 3: gate

**Verificar:** `bun run verify` sai com exit 0.

## Plano de testes

Os 8 testes do Passo 2. As fixtures 1 a 4 são cópias literais da saída real do modelo.

## Critérios de pronto

- [ ] `cargo test -p agent-core tool_call` passa com 8 testes
- [ ] `bun run verify` sai com exit 0
- [ ] `grep -n "regex" crates/agent-core/Cargo.toml` não retorna nada
- [ ] A linha deste plano em `plans/README.md` está atualizada

## STOP conditions

- As fixtures do `docs/audit/benchmark-2026-09-11.json` não batem com as citadas aqui.
- O `clippy -D warnings` exige `unsafe` ou dependência nova.

## Notas de manutenção

- **O loop do agente deve usar este parser só quando `message.tool_calls` vier vazio.** O formato nativo do Ollama tem prioridade.
- A lista `known_tools` sempre vem das ferramentas oferecidas na requisição. Nunca aceite um nome que não foi oferecido: é a primeira linha de defesa contra conteúdo injetado no texto.
- Se uma versão futura do Ollama passar a converter esse formato, o parser continua inofensivo: ele só atua quando `tool_calls` está vazio.
