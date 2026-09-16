---
name: cd-ai
description: Coding agent desktop local — a sala de controle de um agente que trabalha no seu código.
colors:
  accent: "oklch(0.52 0.1 195)"
  accent-ink: "oklch(0.99 0.01 195)"
  canvas: "oklch(0.99 0.002 250)"
  sidebar: "oklch(0.965 0.004 250)"
  raised: "oklch(1 0 0)"
  line: "oklch(0.9 0.005 250)"
  ink: "oklch(0.23 0.012 250)"
  ink-muted: "oklch(0.45 0.012 250)"
  ink-faint: "oklch(0.55 0.01 250)"
  ok: "oklch(0.52 0.13 150)"
  warn: "oklch(0.6 0.13 70)"
  bad: "oklch(0.55 0.19 25)"
typography:
  title:
    fontFamily: "system-ui, \"Segoe UI\", -apple-system, Ubuntu, sans-serif"
    fontSize: "1.375rem"
    fontWeight: 600
    lineHeight: 1.25
    letterSpacing: "-0.02em"
  conversation:
    fontFamily: "system-ui, \"Segoe UI\", -apple-system, Ubuntu, sans-serif"
    fontSize: "0.9375rem"
    fontWeight: 400
    lineHeight: 1.625
  body:
    fontFamily: "system-ui, \"Segoe UI\", -apple-system, Ubuntu, sans-serif"
    fontSize: "0.875rem"
    fontWeight: 400
    lineHeight: 1.5
  label:
    fontFamily: "system-ui, \"Segoe UI\", -apple-system, Ubuntu, sans-serif"
    fontSize: "0.75rem"
    fontWeight: 400
    lineHeight: 1.33
  code:
    fontFamily: "ui-monospace, \"Cascadia Code\", \"JetBrains Mono\", Consolas, monospace"
    fontSize: "0.92em"
rounded:
  sm: "6px"
  md: "8px"
  lg: "12px"
  xl: "16px"
  full: "9999px"
spacing:
  row: "32px"
  topbar: "48px"
  sidebar: "256px"
  column: "736px"
  gutter: "24px"
components:
  button-primary:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-ink}"
    rounded: "{rounded.md}"
    height: "36px"
    padding: "0 14px"
  icon-button:
    textColor: "{colors.ink-muted}"
    rounded: "{rounded.md}"
    size: "32px"
  icon-button-pressed:
    backgroundColor: "{colors.sidebar}"
    textColor: "{colors.ink}"
    rounded: "{rounded.md}"
    size: "32px"
  activity-row:
    textColor: "{colors.ink-muted}"
    rounded: "{rounded.md}"
    height: "{spacing.row}"
    padding: "0 8px"
  user-message:
    backgroundColor: "{colors.sidebar}"
    textColor: "{colors.ink}"
    typography: "{typography.conversation}"
    rounded: "{rounded.xl}"
    padding: "10px 16px"
  composer:
    backgroundColor: "{colors.raised}"
    textColor: "{colors.ink}"
    typography: "{typography.conversation}"
    rounded: "{rounded.xl}"
    padding: "14px 16px 10px"
  report:
    textColor: "{colors.ink}"
    rounded: "{rounded.lg}"
    padding: "14px 16px"
---

# Design System: cd-ai

## Overview

**Creative North Star: "A Sala de Controle"**

O cd-ai é a sala de controle de um agente que trabalha sozinho no código do usuário. O papel da interface é o de um painel de operação: mostrar com precisão o que está acontecendo agora (fase, modelo, contexto) e o que já aconteceu (o que foi lido, editado, executado e com qual resultado). Não é uma IDE; é o lugar de onde se acompanha, aprova e confere.

A gramática vem do app desktop do Claude Code (aba Code): sidebar de tarefas, conversa no centro, composer embaixo, painéis de diff e terminal sob demanda. Sobre ela entram duas regras próprias: a atividade de rotina fica recolhida, e o estado da tarefa nunca some da tela.

A densidade é de ferramenta de trabalho, não de página de marketing: texto do sistema, linhas finas, nada decorativo. A cor aparece só onde carrega informação: ação primária, seleção, estado ao vivo e veredito.

**Key Characteristics:**

- Duas camadas neutras frias (canvas e sidebar) e um único acento verde-água.
- Superfícies planas; profundidade por tom e linha de 1px, nunca por sombra.
- Fonte do sistema para tudo; monoespaçada só para código, paths e comandos.
- Estado da tarefa sempre visível acima do composer.
- Rotina recolhida, erro sempre aberto.

## Colors

Paleta contida: neutros de matiz fria (250) e um acento verde-água (195). Verde, âmbar e vermelho existem só como semântica. O tema escuro redefine os mesmos papéis em `apps/desktop/src/app/globals.css`, e o tema segue o sistema operacional. Os componentes do shadcn/ui leem nomes próprios (`--primary`, `--accent`, `--muted`…); no mesmo arquivo eles são apenas apelidos destes tokens e não carregam cor alguma. Atenção ao par trocado: o `--primary` do shadcn é o `signal` do projeto, e o `--accent` dele é o neutro `sidebar`.

### Primary

- **Verde-água de sinal** (`signal`): botão primário, anel de foco, ponto da fase ativa, spinner de comando em execução, seleção de texto. Nunca é usado como decoração nem em estado inativo.
- **Tinta sobre sinal** (`signal-ink`): texto e ícones sobre o acento.

### Neutral

- **Canvas** (`canvas`): fundo da conversa e da área principal.
- **Painel lateral** (`sidebar`): segunda camada, usada na sidebar, no balão de mensagem do usuário, nas saídas de comando e no hover das linhas.
- **Superfície elevada** (`raised`): só o campo do composer, que é o único ponto de entrada de texto.
- **Linha** (`line`): divisórias de 1px, bordas do composer e do relatório, trilho do medidor de contexto.
- **Tinta** (`ink`): texto principal.
- **Tinta secundária** (`ink-muted`): texto de apoio e linhas de atividade.
- **Tinta discreta** (`ink-faint`): rótulos, metadados, placeholders, ícones em repouso.

### Semânticas

- **Validado** (`ok`): veredito validado, check de comando que passou, linhas adicionadas.
- **Atenção** (`warn`): "não validado", tarefa aguardando aprovação, contexto acima de 80%.
- **Falha** (`bad`): comando que falhou, leitura que falhou, linhas removidas, "Ollama offline".

### Named Rules

**A Regra da Voz Única.** O verde-água só aparece em ação primária, foco, seleção ou estado ao vivo. Se um elemento está verde-água e não é nenhum desses, está errado.

**A Regra do Veredito.** Verde, âmbar e vermelho significam validado, não validado e falhou. Não são usados para mais nada.

## Typography

**Body Font:** fonte do sistema (Segoe UI no Windows, Ubuntu/Cantarell no Linux).
**Mono Font:** ui-monospace (Cascadia Code no Windows).

**Character:** uma única família de trabalho, com hierarquia feita por peso e tamanho em passos curtos. Nenhuma fonte baixada da rede.

### Hierarchy

- **Title** (600, 1.375rem, 1.25, -0.02em): título de estado vazio. Aparece uma vez por tela, no máximo.
- **Conversation** (400, 0.9375rem, 1.625): mensagens do usuário e do agente, em coluna de até 46rem.
- **Body** (400, 0.875rem, 1.5): linhas de atividade, sidebar, cabeçalho.
- **Label** (400, 0.75rem): rótulos de grupo da sidebar, metadados, linha de estado. Em caixa normal, nunca em caixa alta com tracking.
- **Code** (mono, 0.92em do texto ao redor): paths, comandos, nomes de modelo, código inline (este com fundo `sidebar` e raio 6px).

### Named Rules

**A Regra do Mono Honesto.** Monoespaçada só para o que é literalmente código, path, comando ou identificador. Nunca como fantasia de "técnico".

## Layout

- **Janela:** mínimo de 960×600, padrão de 1280×800.
- **Sidebar:** 16rem, recolhível com Ctrl+B. A largura anima em 200ms com ease-out exponencial, e o conteúdo interno tem largura fixa, para não refluir durante a animação.
- **Cabeçalho:** 48px de altura, translúcido, sobreposto à conversa; o conteúdo rola por baixo dele.
- **Conversa:** coluna central de até 46rem, com respiro lateral de 24px e intervalo de 16px entre blocos. Abre rolada até a atividade mais recente.
- **Composer:** fixo embaixo, na mesma largura da coluna, com um fade de 32px do canvas acima dele no lugar de uma divisória.
- **Painéis de diff e terminal:** 22rem à direita, abertos sob demanda (Esc fecha). Entram deslizando 12px com fade.
- **Linhas de atividade:** 32px de altura mínima. O hover estende a área 8px para cada lado, sem deslocar o texto.

## Elevation & Depth

O sistema é plano. A profundidade vem de três recursos, nesta ordem:

1. **Troca de tom:** canvas → sidebar → raised.
2. **Linhas de 1px** em `line`.
3. **Translucidez**, só no cabeçalho: fundo `canvas` a 80% com blur de 16px e saturação de 160%.

Não há sombras. Com `prefers-reduced-transparency`, o cabeçalho fica sólido.

### Named Rules

**A Regra da Camada Dupla.** No máximo dois neutros por região (canvas e sidebar); `raised` é reservado ao campo do composer. Uma superfície nova escolhe um desses, nunca um tom intermediário.

## Shapes

Cantos suaves e consistentes, crescendo com o tamanho da superfície:

| Raio | Onde |
|---|---|
| 6px | código inline e logo |
| 8px | linhas, botões e botões de ícone |
| 12px | relatório de resultado |
| 16px | composer e balão do usuário |
| total | só pontos de status e o botão de enviar |

Ícones desenhados à mão num grid de 16px, traço de 1.5 e pontas arredondadas. Nada de emoji ou glifo no lugar de ícone.

## Components

### Buttons

- **Primário:** fundo verde-água, texto `signal-ink`, altura de 36px, padding horizontal de 14px, raio de 8px. Ao pressionar, escala 0.97 em cerca de 100ms. Desabilitado fica com opacidade de 40%.
- **Ícone:** 32×32, raio de 8px, tinta secundária. No hover ganha fundo `sidebar` e tinta principal. Quando é um toggle ligado (`aria-pressed`), mantém o fundo `sidebar`. Tem sempre `aria-label` e um tooltip próprio — nunca o `title` nativo, que chega com meio segundo de atraso e com a aparência do sistema.
- **Enviar:** círculo verde-água de 32px com seta; fica desabilitado enquanto o agente não está conectado.

### Inputs / Fields

- **Composer:** campo único com borda `line`, fundo `raised` e raio de 16px. No foco, a borda vira verde-água a 60%, sem anel externo.
  - O textarea cresce com o conteúdo até 12.5rem (via JS, porque o WebKitGTK não tem `field-sizing`).
  - Embaixo ficam o seletor de modo de permissão (Perguntar antes / Automático / Acesso total; Acesso total desabilitado sem sandbox Linux), o modelo com o ponto de carregado e o botão de enviar.

### Navigation

- **Sidebar:** logo e nome, "Nova tarefa", grupo Workspace, grupo Tarefas e rodapé de status (Ollama e versão do core).
- **Itens de tarefa:** 32px de altura, com ponto de status de 6px:
  - verde-água pulsando: em andamento;
  - verde: validada;
  - anel âmbar: não validada;
  - vermelho: falhou.

  O item ativo ganha fundo `canvas`. Todo status tem um texto `sr-only` para leitores de tela.

### Activity Rows (componente-assinatura)

- **Exploração agrupada:** leituras e buscas bem-sucedidas em sequência viram uma linha "Explorou N arquivos · M buscas" com chevron. Ela expande em 200ms (grid-rows 0fr→1fr, ease-out exponencial) numa lista com guia vertical de 1px.
- **Edição:** lápis, "Editou", path em mono e contagens `+n −m` tabulares.
- **Comando:** ícone de terminal, comando em mono e status (spinner "rodando", check com duração, ou "saiu com N" em vermelho). A saída fica recolhida, **exceto quando o comando falhou**, e aparece num bloco `sidebar` com raio de 8px.
- **Falha de leitura ou busca:** nunca recolhida; alerta vermelho e mensagem de erro à direita.

### Status Line (componente-assinatura)

A linha acima do composer, sempre presente numa tarefa:

- **Com a tarefa em andamento:** as fases Explorar › Implementar › Validar › Revisar, com a atual em peso médio e ponto verde-água pulsando.
- **Com a tarefa concluída:** o veredito (Validado, Não validado, Falhou).
- **À direita, sempre:** um `<meter>` nativo de contexto usado, que fica âmbar acima de 80%, e o texto "11,4 mil de 32,8 mil tokens".

### Report

A tarefa termina num relatório com borda `line` e raio de 12px:

- ícone de veredito num círculo tingido a 15%;
- título "Validado" ou "Não validado";
- resumo;
- lista de checks, cada um com ícone, rótulo, comando em mono e resultado ("passou", "falhou" ou "não executado").

## Do's and Don'ts

### Do:

- **Do** manter a linha de estado (fase, modelo, contexto) visível em qualquer tarefa aberta.
- **Do** recolher a atividade de rotina (leituras e buscas bem-sucedidas) e deixar edições, comandos e erros à vista.
- **Do** usar `<meter>`, `<select>` e `inert` nativos antes de reinventar controles.
- **Do** manter todo valor dinâmico fora de atributos `style` no HTML estático. A CSP do Tauri bloqueia estilos inline.
- **Do** respeitar `prefers-reduced-motion` (as transições viram instantâneas) e `prefers-contrast` (as linhas ficam mais fortes).
- **Do** manter transições de estado entre 150 e 250ms, com ease-out exponencial `cubic-bezier(0.16, 1, 0.3, 1)`.

### Don't:

- **Don't** transformar a interface numa IDE: nada de árvore de arquivos fixa, painéis permanentes ou terminal sempre aberto.
- **Don't** usar logo, nome ou cores da Anthropic ou do Claude. A referência é de estrutura, não de marca.
- **Don't** usar sombras, gradientes decorativos, texto em gradiente ou bordas coloridas grossas.
- **Don't** mostrar "validado" sem evidência, nem esconder um "não validado" atrás de um tom neutro.
- **Don't** carregar fontes, imagens ou scripts da rede.
- **Don't** usar rótulos em caixa alta com tracking acima de títulos.
