# Plano 014: Implementar a Fase 4 — Tool Engine, permissões, Secret Redactor, eventos e checkpoint

> **Instruções ao executor:** rode os passos nesta ordem e confirme cada verificação antes de avançar. O design (fonte da verdade) é `docs/design/fase-4-tool-engine.md` — as decisões D1–D9 e as seções 2–7 são obrigação. Rode `bun run verify` antes de atualizar a linha em `plans/README.md`.
>
> **Drift check:** todas as dependências do plano estão DONE (002 workspace, 005 streaming/Channel, 006 ACL, 009 parser, 010 eventos na UI, 012 design). Escrevendo em `main` (pedido do usuário; planos anteriores foram mergeados via PR #1).

## Status

- **Prioridade:** P1 · **Esforço:** L · **Risco:** MEDIUM
- **Depende de:** 002, 005, 006, 009, 010, 012
- **Categoria:** feature (Fase 4 do SPEC §34)
- **Fonte da verdade:** `docs/design/fase-4-tool-engine.md`

## Por que isso importa

A Fase 4 é a porta da frente do agente: toda tool que o modelo vai usar (SPEC §15) atravessa path validation (`Workspace::resolve`), pipeline de permissão/aprovação (§20.4), Secret Redactor (§20.6) e emite eventos estruturados (§22). É o coração de segurança do produto.

## Escopo

**Dentro:** módulos `tools`, `permissions`, `redactor`, `events` novos em `agent-core`; 6 tools; formato de edição exacto+fallback; classificação de comandos; aprovação em modo ASK; redator path+conteúdo; `ToolEvent`; checkpoint SHA-256; bridge `run_tool`/`respond_approval` no Tauri; ACL; bindings TS e `ipc.ts`. **Fora:** loop de tarefa (Fase 5), sandbox/FULL ACCESS (F7), shadow repo (F9), streaming de terminal na UI, consumo dos eventos na CLI.

## Decisões do design que NÃO são reabertas

| # | Decisão | Implementação |
|---|---|---|
| D1 | diff `file.changed` | crate `similar` (unified diff line-level) |
| D2 | sem compostos | `argv` only; token com metacaractere → `CompoundCommand` |
| D3 | parse imediato | `tree-sitter` + `tree-sitter-typescript` (ts/tsx/js/jsx) e `tree-sitter-rust` (`rs`) |
| D4 | search | `grep-regex` + `grep-searcher` (engines do ripgrep) + `ignore` (`.gitignore`); `regex:false` → literal escapado |
| D5 | checkpoint | SHA-256 (`sha2`) de cada arquivo antes de editar |
| D6 | execução | `std::process::Command` argv+cwd, sem shell |
| D7 | entropia | janela 32 B, limite 4,7 bits/char, 2 janelas consecutivas |
| D8 | rede | classe `network` sempre pede aprovação |
| D9 | edição | `edit_file`/`write_file` pedem aprovação com diff exato; `read`/`list`/`search`/comandos `read`/`validate` automáticos |

Limites (design §2): `MAX_READ_LINES=2000`, `MAX_READ_BYTES=200KiB`, `MAX_SEARCH_RESULTS=200`, `MAX_SEARCH_BYTES=64KiB`, `MAX_OUTPUT_BYTES=64KiB`, `MAX_LIST_ENTRIES=500`, `MAX_REWRITE_BYTES=8KiB`, timeout de comando 30s.

## Passos

1. **Deps:** `cargo add` em `agent-core`: `similar`, `sha2`, `ignore`, `grep-regex`, `grep-searcher`, `tree-sitter`, `tree-sitter-typescript`, `tree-sitter-rust`. Verificar: `cargo build -p agent-core` exit 0.
2. **redactor.rs:** `SecretKind` (DotEnv|PrivateKey|Credentials), `SecretSpan`, `recognize(text) -> Vec<SecretSpan>` (prefixos, PEM, JWT, URI userinfo, entropia janela 32/4.7/run2), `redact(text) -> RedactedText`, `detect_path_secret(Path)`. Sem regex, sem unsafe, sem deps novas. Testes: fixtures do Spike B (tabela §9), regressão "janela 16 nunca usada" (`log2(16)=4.0 < 4.7`).
3. **permissions.rs:** `CommandClass` (Read|Validate|Write|Network|Destructive|Unknown), `PermissionDecision` (denied|auto|granted), `ApprovalAction` (tagged `type`), `ApprovalResponse`, `ApprovalRequest {id: aprv_NNNN, task_id, at, action}` idempotente (`PermissionManager`), `respond(id, ...)`.
4. **events.rs:** `ToolEvent` no padrão do plano 005 (tag `event`, `data`) + `ToolEventMessage { task_id, sequence, at, event }`. Variantes exatamente como design §7.
5. **tools/read.rs:** `read_file` (linhas 1-indexed, truncamento, redação, view de secret aprovado §6.4) e `list_directory` (ordenado, sem seguir symlink, truncamento).
6. **tools/search.rs:** walker `ignore` + `Searcher` com literal escapado, `matches`, total, truncamento de resultados e bytes, redação por linha.
7. **tools/edit.rs:** `apply_edit` exacto (1→aplica, 0→fallback whitespace/normalização se único, senão `EditNotFound`+snippet mais próximo, ≥2→`AmbiguousEdit`), `write_file` (ifExists error|overwrite com limite 8KiB), parse tree-sitter (extensões fora pulam, registrado), gravação atômica (temp→rename), checkpoint sha2 antes/depois, diff `similar`.
8. **tools/command.rs:** `classify(argv)` pura com a tabela §4 (basename+flags); `run_command` com cwd canonizado, timeout kill de grupo (spike A: `process_group(0)` + `kill -KILL -- -pgid` no unix; fallback documentado no Windows), stdout/stderr separados, junção cap `MAX_OUTPUT_BYTES`, redação.
9. **tools/mod.rs:** limitações,tipos `ToolRequest` (tag `tool`), `ToolOutcome`, `ToolError` (tag `kind`); **ToolEngine**: resolve→permissão→redact→eventos (auto emite decisão implícita), aprovação por callback responder.
10. **Testes** (critério de saída do design): cada tool com casos de sucesso/erro; formato de edição (exacto, fallback, ambíguo, snippet); classificação (tabela §4 inteira); redator (Spike B); permissões (auto vs aprovar vs negado); checkpoint.
11. **Tauri:** commands `run_tool(request, task_channel: Channel<ToolEventMessage>)` (spawn-blocking; responder bloqueia em oneshot resolvido por `respond_approval`) e `respond_approval(id, granted, reason)`; estado do engine por workspace; ACL: `allow-run-tool`, `allow-respond-approval`.
12. **TS:** regenerar bindings (`cargo test -p agent-core export_bindings`), `ipc.ts` com `runTool`/`respondApproval` e reexports.
13. **Gate/README/commit:** `bun run verify` exit 0; linha 014 → DONE; commit em `main` no formato `feat(core): tool engine fase 4`.

## Testes do formato de edição (obrigatórios)

- 1 match → aplica, diff não vazio, hash before/after mudam.
- 0 matches → `EditNotFound` com `context` (janela mais próxima normalizada).
- ≥2 matches → `AmbiguousEdit`.
- 0 matches + fallback whitespace único → aplica com `fuzzy: true`.
- Parse falha (TS inválido) → `ParseFailed { line }` e arquivo original intacto.
- Extensão sem gramática → aplica sem parse (registrado no evento).
- `write_file` with overwrite em arquivo > 8KiB → recusado (usar `edit_file`).

## Critérios de pronto

- [ ] `grep -rn "TODO\|a decidir\|unimplemented" crates/agent-core/src/tools crates/agent-core/src/permissions.rs crates/agent-core/src/redactor.rs crates/agent-core/src/events.rs` vazio
- [ ] `bun run verify` exit 0
- [ ] testes das tools e do formato de edição passando (critério de saída do design)
- [ ] linha 014 em `plans/README.md` = DONE
- [ ] commit em `main` (sem push)

## STOP conditions

- `ignore` e `grep-searcher` incompatíveis de versão (panic/binary detection estranho) → reporte sem improvisar outro motor.
- `tree-sitter-typescript`/`rust` com ABI incompatível com o `tree-sitter` resolvido → reporte as versões.
- Bindings TS regenerados com shape divergente do esperado (ex.: `u64` vira `bigint`) → reporte (regra do plano 008).

## Notas de manutenção

- Todo tipo IPC novo: `#[derive(TS)] #[ts(export)]` + `u64 → #[ts(type = "number")]` (regra do plano 008).
- Nenhuma tool recebe path cru: SEMPRE `Workspace::resolve` primeiro (design §2 regra transversal).
- Redação é obrigatória na fronteira de saída (design §6.3): output de comando, read_file, search, diffs.