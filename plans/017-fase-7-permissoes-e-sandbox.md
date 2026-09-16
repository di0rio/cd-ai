# Plano 017: Fase 7 — Permissões e sandbox

> **Instruções ao executor:** rode os passos nesta ordem e confirme cada verificação antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Uma parte só está pronta com `bun run verify` saindo com exit 0 (AGENTS.md).
>
> **Drift check (rode primeiro):** `git log -1 --oneline` deve estar em `main` com as Fases 0–6 (PR #3). Confira `crates/agent-core/src/permissions.rs`, `crates/agent-core/src/tools/command.rs`, `docs/design/fase-4-tool-engine.md` §5 e SPEC §20 / §34 Fase 7. Se a classificação de comandos ou o `ToolEngine` tiverem sumido, trate como STOP.

## Status

- **Prioridade:** P1
- **Esforço:** L
- **Risco:** HIGH (isolamento de processo; um sandbox frouxo finge segurança)
- **Depende de:** 016 (Fase 6 no `main`)
- **Categoria:** feature (SPEC §20, §34 Fase 7)
- **Planejado em:** commit `0ea7032` (`main`), 2026-09-16

## Por que isso importa

Até a Fase 6 o modo é efetivamente ASK: `read`/`validate` passam sozinhos e o resto pergunta. A classificação existe, mas **não há sandbox de SO** e a rede de um comando aprovado (ou de um `unknown` que o usuário liberou) continua disponível. Sem isolamento, a promessa "não sair do workspace" não vale para `run_command`.

A Fase 7 fecha SPEC §20.3–§20.5: sandbox do shell, rede bloqueada de fato, modos ASK/AUTO/FULL ACCESS, e todo resultado de tool marcado como dado não confiável.

## Estado atual (fatos)

- `permissions.rs` classifica argv (função pura + testes). `PermissionManager` só guarda aprovações pendentes. Não existe `PermissionMode`.
- `run_command` auto-aprova `read`/`validate` e pergunta o resto. Depois de aprovado, o filho nasce com `std::process::Command` sem namespace, sem landlock, com a rede do usuário.
- `edit_file`/`write_file` sempre perguntam. Arquivo de secret sempre pergunta. Fora do workspace já é negado em `Workspace::resolve`.
- FULL ACCESS na UI está visível como rótulo inerte ("Perguntar antes"); o composer diz que o core só implementa ASK (decisão 0001).
- Conteúdo não confiável: o relatório herdado de outra tarefa já vai delimitado (`prompt.rs`). Resultados de tool **não**.
- Linux neste ambiente: kernel 6.12, Landlock ABI 6, `unshare --user --net` funciona. Não há `bwrap` instalado — e não é necessário.

## Decisões deste plano (não reabrir sem ADR)

| # | Decisão | Motivo |
|---|---|---|
| D1 | **Sandbox é módulo em `agent-core`**, não crate novo. | Decisão 0003: crates só com fronteira real. |
| D2 | **Linux primeiro, com Landlock + user/net namespace.** Sem bubblewrap (binário extra). Windows/macOS: sandbox indisponível, documentado, sem fingir isolamento. | SPEC §20.3 + decisão 0001. Landlock ABI 6 e `unshare` já existem aqui. |
| D3 | **Rede bloqueada no OS para qualquer classe que não seja `network` aprovada.** Mesmo um `unknown` liberado pelo usuário nasce sem rede. `network` aprovado pula o netns (e o deny TCP do Landlock). | SPEC: "rede bloqueada por padrão"; "a rede é liberada por comando, com aprovação explícita". Perguntar não é bloquear. |
| D4 | **Escrita no sandbox: workspace + tmp/cache necessários.** `/` fica leitura+execução. Write em workspace, `TMPDIR`/`/tmp`/`/var/tmp`, e caches do usuário (`~/.cache`, `~/.bun`, `~/.cargo`, `~/.npm`, `~/.local/share`). Não liberar `$HOME` inteiro. | SPEC §20.3. `bun test`/`cargo test` precisam do cache; `$HOME` solto furaria o workspace. |
| D5 | **FULL ACCESS só com sandbox completo** (filesystem **e** bloqueio de rede). Sem isso o modo é recusado (CLI/UI) e, se chegar no engine, rebaixa para ASK. | SPEC §20.3–§20.4. |
| D6 | **AUTO sem sandbox de FS:** editar/criar arquivo continua automático (path já é validado); comando `write` **pergunta** (a tabela diz "auto (sandbox)"). | Não fingir isolamento de shell no Windows. |
| D7 | **A tabela §20.4 é uma função pura** `policy(mode, kind, caps) -> Auto \| Ask \| Deny`. Tools só perguntam quando a policy diz `Ask`. | Fronteira de confiança no Rust; testável sem spawn. |
| D8 | **Modo vive em `settings.json`** (`permissionMode`), default ASK. CLI: `--mode ask\|auto\|full-access`. Eval continua ASK + responder que concede — o sandbox ainda envolve os comandos. Sem `--yes` no `task`. | Settings já é o lugar da preferência (plano 015). Eval não pode regressar. |
| D9 | **Todo resultado de tool no contexto vai entre marcadores**, com regra no system prompt de nunca seguir instruções ali. Decisão de permissão nunca lê o conteúdo. | SPEC §20.5. O relatório herdado já faz o equivalente. |
| D10 | **Allowlist por workspace (§30) fica de fora.** A saída da Fase 7 não pede isso; modos + sandbox + marcação cobrem o critério. | Mais simples e correto. |
| D11 | **Dependência nova: `libc`.** Só bindings de syscall (unshare, landlock, prctl). Sem crate `landlock`/`nix`. | Régua §35: API nativa; `libc` é o FFI mínimo. |

### Tabela efetiva (o que o código implementa)

| Operação | ASK | AUTO | FULL ACCESS |
|---|---|---|---|
| Ler arquivo (não secret) | auto | auto | auto |
| Editar/criar arquivo | aprovar | auto | auto |
| Arquivo de secret | aprovar | aprovar | aprovar |
| Comando `read`/`validate` | auto | auto | auto |
| Comando `write` | aprovar | auto se FS sandbox; senão aprovar | auto (exige sandbox) |
| Comando `network`/`destructive`/`unknown` | aprovar | aprovar | aprovar |
| Fora do workspace | negado | negado | negado |

Não há tool `delete_file` na v1. "Deletar arquivo" da tabela cai em comando (`rm` sem recursão = `write`; `rm -r` = `destructive`, sempre pergunta). Checkpoint de hash antes de editar já existe (Fase 4); shadow repo é Fase 9.

## Escopo

**Dentro:** `PermissionMode` + `policy`; sandbox Linux (Landlock + netns) aplicado em `run_command`; bloqueio real de rede; modos na CLI/settings/UI; marcação de tool results; testes de policy e de sandbox; docs/handoff; eval scripted sem regressão.

**Fora:** Fase 8+ (verifier, shadow git, context manager, skills, router, release). Allowlist. Sandbox Windows/macOS. Ampliar a suíte de eval. `--yes` no `task`. Payloads ofensivos nos testes (só conectividade local e escrita negada).

## Passos

1. **Policy (`permissions.rs`):** `PermissionMode { Ask, Auto, FullAccess }`, `PermissionKind`, `policy(...)` com a tabela, testes de cada célula. `effective_mode` rebaixa FullAccess sem sandbox completo para Ask.
2. **Sandbox (`sandbox.rs`):** `probe()` em `OnceLock`. Linux: Landlock ABI (FS; TCP deny se ABI ≥ 4 e o netns falhar) + `CLONE_NEWUSER|CLONE_NEWNET` no `pre_exec` quando a rede não foi liberada. Não-Linux: `available: false`. `apply(command, opts)` falha o spawn se o sandbox deveria estar ativo e não aplicou — nunca executa "pela metade" como se estivesse isolado.
3. **`ToolEngine`:** guarda modo + aplica sandbox em `run_command` (rede só se classe `network` **e** decisão `granted`). `authorize()` consulta `policy` antes de `ask_approval`. Default do engine continua ASK.
4. **Conteúdo não confiável (`prompt.rs` + `runner.rs`):** marcadores em todo `tool` message; frase no system prompt; testes de wrapping. Trim continua reconhecendo `OMITTED_RESULT` (texto nosso, sem wrap).
5. **Settings + loop:** `Settings.permission_mode`; `TaskContext.permission_mode`; `run_task` cria o engine no modo pedido.
6. **CLI:** `--mode`; FULL ACCESS recusado se `probe` não estiver completo; USAGE/testes.
7. **Tauri/UI:** `sandbox_status`, `set_permission_mode`; ACL; seletor no composer (Acesso total desabilitado sem sandbox); persistência via settings. Frontend não decide.
8. **Docs:** handoff (Fase 7 feita, próxima 8), README dos planos, nota de auditoria com o que o Linux cobriu e o buraco Windows/macOS, esclarecimento na 0001. `AGENTS.md`/`README.md` se o comando `task` ganhar `--mode`.
9. **Gate:** `bun run verify` exit 0. `cargo run -q -p cd-ai-cli -- eval --scripted` continua 3/3.

## Contratos

`SandboxStatus` (IPC, gerado do Rust):

```json
{
  "available": true,
  "filesystem": true,
  "networkBlock": true,
  "platform": "linux",
  "detail": "landlock fs + user/net namespace"
}
```

`available == filesystem && networkBlock`. FULL ACCESS consulta só esse booleano no Rust.

Modo na CLI: `--mode ask|auto|full-access`. Default `ask`. Valor inválido → exit 2.

## Critérios de pronto

- [ ] `plans/017-fase-7-permissoes-e-sandbox.md` existe; linha 017 em `plans/README.md` = DONE
- [ ] Testes da tabela ASK/AUTO/FULL ACCESS passando
- [ ] No Linux: comando sem classe `network` não alcança um TCP listener local; escrita fora do workspace/tmp/cache falha
- [ ] Windows/macOS: gap documentado; FULL ACCESS indisponível; testes de policy ainda passam
- [ ] Tool results delimitados como não confiáveis
- [ ] `cd-ai eval --scripted` 3/3
- [ ] `bun run verify` exit 0
- [ ] PR aberto

## STOP conditions

- `run_command` deixou de ser argv-only (voltou shell string) → reporte, não "consertar" com `sh -c` dentro do sandbox.
- Landlock/user ns indisponíveis neste Linux e não há fallback honesto de bloqueio de rede → não marque FULL ACCESS como disponível; documente e ainda entregue modos + marcação + policy.
- Eval scripted caiu de 3/3 porque o sandbox quebrou `bun test` → ajuste a allowlist de write (cache/tmp), não desligue o sandbox no eval.
- Pedido de PoC/exploit para "provar" o sandbox → recuse; use só listener local e escrita negada.

## Notas de manutenção

- Código e comentários em inglês; UI e docs em pt-BR.
- A UI nunca valida path, modo ou sandbox: só mostra o que o Rust exportou.
- Probe é cacheado no processo (`OnceLock`). Testes da policy não chamam o probe; passam `SandboxCapabilities` explícitas.
- Compostos continuam recusados (`CompoundCommand`) antes da classe — o sandbox não é desculpa para ligar shell.
