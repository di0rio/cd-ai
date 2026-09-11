# Plano 010: a conversa acompanha eventos vivos (auto-scroll e colapso de comando derivado do estado)

> **Instruções ao executor:** siga o plano passo a passo. Rode cada comando de verificação e confirme o resultado esperado antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Ao terminar, atualize a linha deste plano em `plans/README.md`.
>
> **Drift check (rode primeiro):** compare os trechos de "Estado atual" com `apps/desktop/src/components/conversation.tsx`. Se não baterem, trate como STOP condition.

## Status

- **Prioridade:** P1
- **Esforço:** S
- **Risco:** LOW
- **Depende de:** nenhum
- **Categoria:** bug (correção preventiva, pré-requisito da Fase 4/5)
- **Planejado em:** commit `f6e7764`, 2026-09-11

## Por que isso importa

Hoje a conversa recebe uma lista de eventos estática (dados de demonstração) e assume que ela não cresce depois do mount. Quando a Fase 4/5 ligar o stream real de eventos do core Rust à UI — os planos 001–009 constroem exatamente esse pipeline — duas coisas vão quebrar:

1. **A conversa deixa de seguir a atividade nova.** O auto-scroll roda uma única vez, no mount (`conversation.tsx:23-26`). Um comando ou edit novo chegando com a tarefa aberta não rola a tela até ele, quebrando a promessa do `DESIGN.md` ("Abre rolada até a atividade mais recente") e do fluxo "acompanhar o que o agente está fazendo".
2. **Comando que falha no meio fica colapsado.** `CommandRow` decide o estado `open` uma vez, no mount (`conversation.tsx:171`). No stream real, um comando entra como `running` (recolhido) e depois termina com `exitCode != 0`; o `open` continua `false` e a saída da falha fica escondida — contrariando a regra do `DESIGN.md` ("A saída fica recolhida, **exceto quando o comando falhou**") e do `PRODUCT.md` ("Erros e decisões ficam à vista").

Este plano deixa a conversa pronta para receber eventos incrementais antes de qualquer ferramenta entregar isso de verdade.

## Estado atual

- `apps/desktop/src/components/conversation.tsx`, linhas 20–26 — auto-scroll só no mount:

  ```tsx
  export function Conversation({ task }: { task: Task }) {
    const scroller = useRef<HTMLDivElement>(null);

    useEffect(() => {
      const el = scroller.current;
      if (el) el.scrollTop = el.scrollHeight;
    }, []);
  ```

- `apps/desktop/src/components/conversation.tsx`, linhas 167–171 — colapso decidido no mount:

  ```tsx
  function CommandRow({ event }: { event: CommandEvent }) {
    const running = event.exitCode === null;
    const failed = !running && event.exitCode !== 0;
    // Errors never collapse by default.
    const [open, setOpen] = useState(failed);
  ```

- `apps/desktop/src/components/app-shell.tsx`, linha 96 — a conversa é montada por tarefa (key `task.id`), **não** por evento: ` <Conversation key={task.id} task={task} />`. Quando a lista `task.events` crescer, o componente permanece montado e recebe a mesma `task` com `events` maior.

- Convenções do frontend (exemplar: `apps/desktop/src/lib/session.test.ts`):
  - React com Hooks (`useEffect`, `useRef`, `useState` — já importados em `conversation.tsx:3`).
  - Biome (`bun run check`) e `tsc --noEmit` como gate.
  - **Não há runner de testes de componentes** no repo (só `bun test` para TS puro). A decisão de não criar um agora já está registrada em `plans/README.md` → "Achados considerados e descartados". **Não adicione** uma dependência de teste (jsdom/testing-library).

## Comandos que você vai precisar

| Propósito | Comando | Esperado |
|-----------|---------|----------|
| Lint/format | `bun run check` | exit 0 |
| Typecheck | `bun run --cwd apps/desktop typecheck` | exit 0 |
| Testes existentes | `bun run --cwd apps/desktop test` | `0 fail` |

## Escopo

**Dentro do escopo:**

- `apps/desktop/src/components/conversation.tsx` (os dois trechos abaixo)

**Fora do escopo:**

- Qualquer outro arquivo, incluindo `app-shell.tsx` (a chave por `task.id` está correta e não deve mudar).
- Adicionar testes de componente, jsdom ou testing-library.
- Qualquer lógica do core Rust.

## Git

- Branch: `advisor/010-conversation-live`.
- Um commit, mensagem no estilo `fix(desktop): follow live task events in conversation`.
- Não faça push.

## Passos

### Passo 1: auto-scroll em eventos novos

Em `conversation.tsx`, troque o `useEffect` do scroll (linhas 23–26) para reagir ao crescimento da lista de eventos:

```tsx
useEffect(() => {
  const el = scroller.current;
  if (el) el.scrollTop = el.scrollHeight;
}, [task.events.length]);
```

Mantenha o comentário da linha 19 (`// Mounted per task (keyed by id), so it opens scrolled to the latest activity.`). O efeito continua rolando para o fim no mount **e** em toda nova leva de eventos na mesma tarefa.

**Verificar:** `bun run --cwd apps/desktop typecheck` → exit 0 (sem mudar mais nada).

### Passo 2: comando que falha depois abre a saída

Em `conversation.tsx`, mantenha o estado inicial linha 171 e adicione um efeito logo depois do `useState(failed)`, dentro de `CommandRow`:

```tsx
const [open, setOpen] = useState(failed);
// A command that starts running (collapsed) and later fails must surface its output.
useEffect(() => {
  if (failed) setOpen(true);
}, [failed]);
```

Comportamento que este código deve preservar:

- comandos que já nascem falhos abrem no mount (estado inicial `failed` = `true`);
- um comando `running` (recolhido) que termina com erro abre sozinho;
- o usuário continua podendo recolher/expandir manualmente depois (o efeito só força `true` quando `failed` passa a ser verdadeiro);
- comandos bem-sucedidos e em execução preservam o estado manual do usuário.

**Verificar:** `bun run check` → exit 0 (o Biome não deve acusar o efeito; se alguma regra exigir outra coisa, reporte sem mudar a lógica).

### Passo 3: gate completo

Rode os três comandos da tabela, em ordem.

**Verificar:** todos saem com exit 0.

## Plano de testes

Não há teste novo neste plano — o repo não tem runner de testes de componentes (decisão registrada em `plans/README.md`). O que existe e deve continuar passando:

- `bun run --cwd apps/desktop test` (3 testes de `groupActivity` em `session.test.ts`);
- `typecheck` e `biome` como gates.

A validação "humana" deste plano (uma lista de eventos crescendo numa tarefa aberta) só é possível quando a Fase 4/5 entregar o stream real; até lá, o guard é o typecheck e a preservação dos testes existentes.

## Critérios de pronto

- [ ] `bun run check` exit 0
- [ ] `bun run --cwd apps/desktop typecheck` exit 0
- [ ] `bun run --cwd apps/desktop test` → `0 fail`
- [ ] `git status` mostra modificado apenas `apps/desktop/src/components/conversation.tsx`
- [ ] A linha deste plano em `plans/README.md` está atualizada

## STOP conditions

- Os trechos de "Estado atual" não batem com o arquivo vivo (drift).
- O efeito de `CommandRow` faz um comando **bem-sucedido** abrir sozinho, ou faz um comando falho ficar colapsado depois da transição.
- Algum gate pede uma mudança de dependência (não deve: nem `jsdom` nem `testing-library` são permitidos).

## Notas de manutenção

- **Quando a Fase 4/5 ligar eventos vivos, teste estes dois comportamentos com dados reais**: rolagem seguindo a atividade e comando que falha abrindo a saída. Eles foram preparados aqui de propósito.
- Se um dia houver "rolagem fixada" (usuário leu o histórico e não quer ser puxado), o efeito do Passo 1 é o ponto único a mudar — hoje o comportamento intencional é sempre seguir o fim.
- Não troque `key={task.id}` por `key={task.id}-{task.events.length}`: isso desmontaria a conversa a cada evento e perderia o estado de colapso que este plano justamente preserva.