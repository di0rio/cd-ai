# Plano 013: botões de ícone desabilitados voltam a mostrar o tooltip

> **Instruções ao executor:** siga o plano passo a passo. Rode cada comando de verificação e confirme o resultado esperado antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Ao terminar, atualize a linha deste plano em `plans/README.md`.
>
> **Drift check (rode primeiro):** compare os trechos de "Estado atual" com `apps/desktop/src/components/icon-button.tsx`. Se não baterem, trate como STOP condition.

## Status

- **Prioridade:** P3
- **Esforço:** S
- **Risco:** LOW
- **Depende de:** nenhum
- **Categoria:** UX / a11y
- **Planejado em:** commit `f6e7764`, 2026-09-11

## Por que isso importa

`IconButton` aplica `disabled:pointer-events-none` no botão desabilitado (`icon-button.tsx:22`). No WebKitGTK (e nos demais browsers), um elemento com `pointer-events: none` não recebe hover, então o `title` — que o componente sempre seta — nunca aparece num `IconButton` desabilitado. Hoje isso afeta os toggles "Alterações" e "Terminal" quando nenhuma tarefa está aberta: o usuário vê o botão apagado sem nenhuma dica do porquê. O `DESIGN.md` manda que todo botão de ícone tenha `aria-label` **e** `title`; o `title` precisa funcionar em todos os estados.

O resto dos botões desabilitados do app (`empty-workspace.tsx:27`, `sidebar.tsx:44/57`, `composer.tsx:66`) **não** tem `pointer-events-none` e seus `title` já aparecem — não devem ser alterados.

## Estado atual

`apps/desktop/src/components/icon-button.tsx`, arquivo inteiro:

```tsx
import { Icon, type IconName } from "./icons";

type IconButtonProps = {
  label: string;
  icon: IconName;
  onClick: () => void;
  pressed?: boolean;
  disabled?: boolean;
};

export function IconButton({ label, icon, onClick, pressed, disabled }: IconButtonProps) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      aria-pressed={pressed}
      disabled={disabled}
      onClick={onClick}
      className={`grid size-8 place-items-center rounded-lg transition-[background-color,color,scale] duration-150 active:scale-95 disabled:pointer-events-none disabled:opacity-35 ${
        pressed ? "bg-sidebar text-ink" : "text-ink-muted hover:bg-sidebar hover:text-ink"
      }`}
    >
      <Icon name={icon} />
    </button>
  );
}
```

O único ponto problemático é `disabled:pointer-events-none` (linha 20) combinado com `hover:bg-sidebar hover:text-ink` (linha 21) que, sem o `pointer-events`, voltariam a pintar o fundo no botão desabilitado no hover.

Convenção de estilo: variantes `enabled:`/`disabled:` são usadas no resto do app (ex.: `empty-workspace.tsx:31` usa `disabled:opacity-40`; `icon-button` usa `disabled:opacity-35`).

## Comandos que você vai precisar

| Propósito | Comando | Esperado |
|-----------|---------|----------|
| Lint/format | `bun run check` | exit 0 |
| Typecheck | `bun run --cwd apps/desktop typecheck` | exit 0 |
| Testes existentes | `bun run --cwd apps/desktop test` | `0 fail` |

## Escopo

**Dentro do escopo:**

- `apps/desktop/src/components/icon-button.tsx` (só a linha do `className`)

**Fora do escopo:**

- Qualquer outro componente. `empty-workspace.tsx`, `sidebar.tsx` e `composer.tsx` já exibem seus `title` em botões desabilitados e não entram neste plano.
- Mudar o rótulo ou adicionar novos textos.
- Qualquer lógica de estado (`app-shell.tsx`).

## Git

- Branch: `advisor/013-icon-button-tooltips`.
- Um commit, mensagem no estilo `fix(desktop): show tooltip on disabled icon buttons`.
- Não faça push.

## Passos

### Passo 1: trocar as classes do botão

Em `icon-button.tsx`, no `className` do `<button>` (linhas 20–22):

- remova `disabled:pointer-events-none`;
- restrinja o hover ao estado habilitado trocando `hover:bg-sidebar hover:text-ink` por `enabled:hover:bg-sidebar enabled:hover:text-ink`.

Resultado:

```tsx
className={`grid size-8 place-items-center rounded-lg transition-[background-color,color,scale] duration-150 active:scale-95 disabled:opacity-35 ${
  pressed ? "bg-sidebar text-ink" : "text-ink-muted enabled:hover:bg-sidebar enabled:hover:text-ink"
}`}
```

O atributo `disabled` nativo continua impedindo qualquer ativação; a mudança só devolve o hover (e, com ele, o `title`) ao estado desabilitado.

**Verificar:** `bun run --cwd apps/desktop typecheck` → exit 0.

### Passo 2: gate completo

**Verificar:** `bun run check`, typecheck e `bun run --cwd apps/desktop test` → todos exit 0 / `0 fail`.

## Plano de testes

Não há infraestrutura de teste de componentes no repo (decisão registrada em `plans/README.md`). O comportamento é visual e deve ser conferido manualmente pelo usuário quando a UI estiver no ar:

- abrir o app **sem** tarefa (estado vazio) e passar o mouse sobre os botões de ícone "Alterações" e "Terminal" → o `title` (o mesmo do `aria-label`) aparece; o fundo não muda no hover do estado desabilitado;
- com uma tarefa selecionada, os botões habilitados continuam com hover e tooltip normais.

Gates automáticos: `bun run check`, typecheck e os 3 testes existentes.

## Critérios de pronto

- [ ] `bun run check` exit 0
- [ ] `bun run --cwd apps/desktop typecheck` exit 0
- [ ] `bun run --cwd apps/desktop test` → `0 fail`
- [ ] `grep -n "pointer-events-none" apps/desktop/src/components/icon-button.tsx` não retorna nada
- [ ] `grep -rn "enabled:hover:bg-sidebar" apps/desktop/src/components/icon-button.tsx` retorna a linha nova
- [ ] `git status` mostra modificado apenas `apps/desktop/src/components/icon-button.tsx`
- [ ] A linha deste plano em `plans/README.md` está atualizada

## STOP conditions

- O trecho de "Estado atual" não bate com o arquivo vivo (drift).
- O gate do TypeScript acusa algo fora do `className` (não deve; o componente não muda de contrato).
- O usuário relatar que, **habilitado**, o botão perdeu o hover ou o tooltip (regressão do estado normal).

## Notas de manutenção

- Se um dia o app ganhar botões de ícone desabilitados com **motivo** próprio ("chega com o core conectado"), o `label` atual é a ação, não a razão; o `title` precisará de um campo extra. Fora de escopo hoje — os dois únicos casos desabilitados são toggles de painel sem tarefa.
- `pointer-events-none` continua correto em **ícones decorativos** (ex.: a seta do select em `composer.tsx:159`, que precisa deixar os cliques passarem para o `<select>`). Não remova de lá.