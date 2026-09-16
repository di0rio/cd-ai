# Fase 7 — sandbox e permissões (auditoria)

Medido em 2026-09-16, Linux kernel 6.12, Landlock ABI 6, `unshare --user --net` disponível.
Plano: [`plans/017-fase-7-permissoes-e-sandbox.md`](../../plans/017-fase-7-permissoes-e-sandbox.md).

## O que o Linux cobre de verdade

| Garantia | Como | Teste |
|---|---|---|
| Escrita só no workspace + tmp/cache | Landlock: `/` leitura+execução; write em workspace, `/tmp`, `/var/tmp`, `TMPDIR`, `~/.cache`, `~/.bun`, `~/.cargo`, `~/.npm`, `~/.local/share` | `sandbox_allows_writes_in_the_workspace_and_denies_home` |
| Rede bloqueada por padrão | User+net namespace (sem rota). Fallback: Landlock TCP deny (ABI ≥ 4) se o namespace falhar | `sandbox_blocks_tcp_to_a_local_listener` |
| Rede liberada por comando | Classe `network` **e** aprovação `granted` → o filho não entra no netns | `sandbox_allows_tcp_when_network_is_explicitly_released` |
| ASK / AUTO / FULL ACCESS | Função pura `policy` | testes em `permissions.rs` |
| FULL ACCESS exige sandbox completo | `effective_mode` rebaixa para ASK se faltar FS ou bloqueio de rede | `full_access_without_a_complete_sandbox_behaves_like_ask` |
| Conteúdo não confiável | Marcadores em todo resultado de tool + regra no system prompt | `wrap_untrusted_tool_result_is_delimited_and_idempotent` |

Os testes de rede usam um `TcpListener` em `127.0.0.1` no próprio processo de teste — conectividade local, não payload ofensivo.

## O que Windows e macOS **não** têm

Não há Landlock nem user namespace equivalentes aplicados nesta fase.

- `sandbox::status().available == false`
- FULL ACCESS recusado na CLI (`--mode full-access`) e na UI (`set_permission_mode`)
- AUTO ainda auto-aprova **editar/criar arquivo** (path já validado); comando `write` **pergunta**
- `run_command` roda com a rede do usuário; a defesa restante é classificação + aprovação

Isso é o comportamento do SPEC §20.3 para plataforma sem sandbox, não um sandbox fingido.

## Eval

```bash
cargo run -q -p cd-ai-cli -- eval --scripted
```

Medido em 2026-09-16: **100% (3/3)**. Os `run_command` da suíte (`bun test`) passam pelo sandbox Linux. Sem regressão.

O eval continua no modo ASK com responder que concede (cópia descartável).

## Fora desta fase

Allowlist por workspace (SPEC §30), verifier (Fase 8), shadow git (Fase 9), sandbox Windows/macOS.
