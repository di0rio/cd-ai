# Plano 021: Fase 11 — Skills

> **Instruções ao executor:** rode os passos nesta ordem e confirme cada verificação antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Uma parte só está pronta com `bun run verify` saindo com exit 0 (AGENTS.md).
>
> **Drift check (rode primeiro):** `git log -1 --oneline` deve estar em `main` com as Fases 0–10 (PR #10). Confira `crates/agent-core/src/agent/context.rs` (`assemble` / orçamento por seção), `crates/agent-core/src/agent/role.rs` (Explorer/Coder; comentário “Skills are Fase 11”), `crates/agent-core/src/agent/events.rs` (`contextBudgetCut` / `roleChanged`) e SPEC §17 / §12.4 / §34 Fase 11. Se o Context Manager ou o Explorer tiverem sumido, trate como STOP.

## Status

- **Prioridade:** P1
- **Esforço:** M
- **Risco:** MEDIUM (skill sem licença embutida é violação; skill que “libera” path/sandbox fura a fronteira; texto demais estoura o orçamento local)
- **Depende de:** 020 (Fase 10 no `main`)
- **Categoria:** feature (SPEC §17, §12.4, §22, §23, §34 Fase 11)
- **Planejado em:** commit `72ff1ac` (`main`), 2026-09-16

## Por que isso importa

Até a Fase 10 o prompt leva role + perfil + repo map. Não há catálogo, não há licença, não há roteamento: o Coder não recebe conhecimento operacional extra (testing, TypeScript, security) além das regras genéricas. A Fase 11 fecha SPEC §17: registry com versões condensadas, verificação de licença **antes** de embutir, Skill Router determinístico, dependências e conflitos, eventos `skills.detected` / `skill.loaded` / `skill.skipped`.

## Estado atual (fatos)

- `assemble` monta role 12% / regras 8% / map 15% / resposta 20%. Não há seção de skills.
- `WorkspaceProfile` detecta linguagens e package manager; **não** detecta framework (react/next/tailwind).
- `TaskState` tem `role` e `task_kind`; não tem `selectedSkills`.
- Eventos de skill não existem. `role.rs` ainda diz “Skills are Fase 11”.
- Eval scripted 3/3. A suíte não cresce (016 D3 / 020 D9). ScriptedModel ignora o texto do system prompt — a taxa scripted só pode mostrar **neutralidade**.
- Catálogo conceitual da §17.1 inclui nomes de skills de terceiros (`coss`, `impeccable`, `caveman`, …). Não há cópia licenciada no repo.

## Decisões deste plano (não reabrir sem ADR)

| # | Decisão | Motivo |
|---|---|---|
| D1 | **Skills são módulo em `agent-core`** (`skills/`: registry, catálogo, router). Sem crate novo, sem `packages/skills`, sem marketplace, sem importação local (SPEC: “mecanismo futuro”). | Decisão 0003 / 0006. Fronteira Skills da §7 fica no Rust. |
| D2 | **Só entra no registry o que tiver licença de redistribuição.** Allowlist: MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, CC0-1.0, Unlicense. Skill original do cd-ai usa `Apache-2.0` + `origin: cd-ai`. Nomes do catálogo conceitual **sem** texto licenciado (`coss`, `coss-particles`, `emil-design-eng`, `apple-design`, `impeccable`, `ponytail`, `caveman`) **não são embutidos** — não reconstruir conteúdo alheio (SPEC §17.4). | Bloqueador da fase. Inventar o texto de uma skill de terceiro é o mesmo que copiar sem licença. |
| D3 | **Conjunto mínimo original (condensado):** `typescript`, `rust`, `react`, `nextjs`, `tailwind`, `testing`, `debugging`, `security`, `code-review`, `performance`, `git`, `accessibility`, `frontend-design`. Cada uma declara orçamento em tokens (`chars/4`); o router só injeta a versão condensada. Sem versão “full” no binário. | SPEC §17.1 é catálogo conceitual; `rust` cobre o que o perfil já detecta. Qualidade da condensação = eval + testes de orçamento. |
| D4 | **Router é função pura, sem LLM.** Sinais: perfil (linguagens + frameworks), extensões/paths do pedido e do repo map, palavras-chave + `TaskKind`, tags/deps/conflitos da skill. Empate: maior score, depois nome lexicográfico. Sem desempate por modelo nesta fase. | SPEC §4 / §17.3: determinístico antes do LLM. LLM de desempate é YAGNI enquanto o empate tem ordem total. |
| D5 | **Dependências puxam; conflitos descartam o de menor score.** `nextjs` → `react` → `typescript`. Skill `permissions` é **declarativa** (documenta o que a skill assume: read/edit). Nunca eleva `PermissionMode`, nunca fura path/sandbox/secrets. | SPEC §12.4 e §20. A fronteira de confiança continua no Tool Engine. |
| D6 | **Seleção no start da tarefa, persistida em `TaskState.selectedSkills`.** Refresh do system prompt re-renderiza os mesmos nomes a partir do registry (catálogo é a fonte). Eventos: um `skillsDetected` (quem pontuou > 0) + `skillLoaded`/`skillSkipped` só para carregadas ou puladas por conflito/orçamento/dependência/licença, não para as irrelevantes (score 0). | SPEC §22 / §23. Evita 13 skips ruidosos por tarefa. |
| D7 | **Orçamento da seção skills = 10% de `num_ctx`.** Se não cabe, cai a de menor score até caber (skip `over_budget`). Corte explícito + `contextBudgetCut` se ainda estourar. | SPEC §16.2. Não pode comer a resposta nem o pedido. |
| D8 | **Estilo comprimido só em handoffs internos** (Explorer → Coder), texto original curto. Não é a skill `caveman` de terceiro. | SPEC §17.2 “no espírito”; eval scripted não pode piorar. |
| D9 | **Eval não cresce.** Scripted 3/3 não pode regressar. O JSON ganha `selectedSkills`. Evidência de ganho/neutralidade: (a) taxa 3/3 = neutralidade; (b) teste do router nas fixtures (`typescript`+`testing`+`debugging`); (c) condensado ≤ orçamento declarado; (d) `estimatedPromptTokens` documentado vs Fase 10 — o acréscimo é a seção, limitado a 10%. | 016 D3. Scripted não “ganha” qualidade de modelo. |

## Escopo

**Dentro:** `skills/` (registry, licença, catálogo condensado, router); frameworks no perfil; seção skills no Context Manager; `selectedSkills` + eventos; CLI imprime skills; eval JSON; testes; docs/handoff/audit.

**Fora:** Fase 12+ (model router, memória, trajetórias, release). Marketplace, importação de skills do usuário, versões full de terceiros, desempate LLM, suíte de eval maior, payloads ofensivos. Skill não executa código nem muda permissões.

## Passos

1. **Plano e tipos:** este arquivo; `SelectedSkill`; eventos `skillsDetected` / `skillLoaded` / `skillSkipped`; `TaskState.selected_skills` com `#[serde(default)]`. Bindings via `cargo test`.
2. **Registry + licença:** allowlist; catálogo D3; nomes D2 ausentes do registry (teste). Metadados da §17.4. Conteúdo condensado original, inglês, ≤ orçamento.
3. **Perfil:** `frameworks` a partir de `package.json` (deps/scripts) e marcadores (`next.config.*`, `tailwind.config.*`). Render `Frameworks:`.
4. **Router:** score, limiar, deps, conflitos, orçamento D7; testes com catálogo injetado (dep + conflito) e com o catálogo real (fixture soma).
5. **Context + loop:** `assemble` injeta a seção; start emite eventos e grava `selectedSkills`; refresh reusa os nomes. Handoff D8. Skill `permissions` não é lida pelo Permission Manager (teste negativo: o router não chama permissões).
6. **Eval/CLI/UI:** `selectedSkills` no JSON; CLI lista nomes no resumo; eventos novos são no-op na conversa (como `roleChanged`).
7. **Docs:** handoff (Fase 11 feita, próxima 12), README dos planos, `docs/audit/fase-11-skills.md`.
8. **Gate:** `bun run verify` exit 0. `cargo run -q -p cd-ai-cli -- eval --scripted` 3/3.

## Contratos

Licença:

```text
license ∈ allowlist  → pode entrar no registry
license ausente/outra → não embutir, não reconstruir o texto
origin  = "cd-ai" para o conjunto D3
```

Router:

```text
score  = matches de linguagem/framework/extensão/keywords/kind
score ≥ 2          → candidata
dependências       → puxar (mesmo com score 0)
conflito           → fica a de maior score (empate: nome)
orçamento 10%      → derruba a de menor score
saída              → loaded + skipped (com motivo) + detected
```

Prompt:

```text
Skills:
### typescript
<body condensado>
### testing
…
```

Explorer:

```text
skills entram no system prompt do Explorer também (são conhecimento, não tools)
escrita continua recusada no loop
```

## Critérios de pronto

- [x] `plans/021-fase-11-skills.md` existe; linha 021 em `plans/README.md` = DONE
- [x] Registry só contém skills com licença de redistribuição (teste)
- [x] Nomes de terceiro sem licença **não** estão no binário
- [x] Versão condensada respeita o orçamento declarado (teste)
- [x] Router determinístico: mesmas entradas → mesmos nomes, nesta ordem (teste)
- [x] Dependências puxam; conflitos descartam (teste)
- [x] Skills **não** alteram path/sandbox/PermissionMode
- [x] Eventos `skillsDetected` / `skillLoaded` / `skillSkipped` com motivo
- [x] Eval scripted 3/3; `selectedSkills` no JSON; audit com evidência de ganho ou neutralidade
- [x] `bun run verify` exit 0
- [x] PR aberto

## STOP conditions

- Pedido para copiar/reconstruir skill de terceiro sem licença de redistribuição → recuse (SPEC §17.4).
- Pedido de skill que ignore sandbox, path ou permissões → recuse.
- Pedido de PoC/exploit “para demonstrar a skill security” → recuse.
- Eval scripted caiu de 3/3 porque o system prompt mudou o ScriptedModel → o script **não** lê o prompt; se falhou, a causa é outra (classificação/Explorer). Ajuste o router, não o script.
- Ollama ausente → não é STOP.

## Notas de manutenção

- Código e comentários em inglês; UI e docs em pt-BR.
- O frontend não escolhe skills: recebe eventos. O Rust decide.
- `chars/4` continua sendo a estimativa. Tokenizer por modelo é Fase 13.
- Conteúdo das skills é operacional (como pensar), não uma tool nova.
