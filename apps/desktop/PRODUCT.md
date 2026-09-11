# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Cauã, desenvolvedor, em uso pessoal no próprio desktop. Sessões longas de programação, com o cd-ai aberto ao lado do editor e do terminal. O trabalho é delegar tarefas de código a um agente local e acompanhar, aprovar e revisar o que ele faz.

## Product Purpose

O cd-ai é um coding agent desktop, local-first. Ele abre qualquer workspace do computador, entende o codebase, planeja, edita arquivos, executa comandos, testa e revisa.

Uma tarefa só conta como concluída quando existe evidência: comandos executados, exit codes, testes, typecheck, diff. Quando não dá para validar, o resultado diz "não validado".

## Positioning

- Roda inteiro na máquina, com modelos locais via Ollama. Sem cloud, sem telemetria, e o código nunca sai do computador.
- "Pronto" exige evidência. A diferença entre "validado" e "não validado" é explícita, não uma impressão.

## Operating Context

- Fluxo: escolher workspace, descrever a tarefa, acompanhar a atividade do agente, aprovar ações sensíveis, revisar o diff, fazer rollback se preciso.
- Modelos locais são lentos (dezenas de tokens/s nesta máquina). A espera é parte normal do uso, e a interface precisa comunicar progresso e estado sem ansiedade.
- Desenvolvimento no Windows 11; primeiro release em Linux x86_64. App Tauri 2 com frontend Next.js em static export.

## Capabilities and Constraints

- Estado atual: esqueleto desktop (Fase 1). O agente, a seleção de workspace e a conexão com o Ollama ainda não estão ligados à UI.
- Sem nenhuma requisição de rede do frontend: sem fontes, imagens ou scripts remotos.
- Interface em pt-BR.
- Frontend só apresenta dados. Toda lógica e segurança vivem no core Rust (ver `docs/decisions/`).

## Brand Commitments

- Nome: **cd-ai**.
- Referência de interface: o app desktop do Claude Code (aba Code). Conversa no centro, sidebar de tarefas, composer embaixo, painéis sob demanda.
- Melhorias obrigatórias sobre a referência:
  - **Atividade agrupada:** tool calls de leitura e busca não poluem a conversa.
  - **Estado sempre visível:** fase, modelo carregado, uso de contexto, validado ou não.
- Não usar logo, nome ou marca da Anthropic ou do Claude.

## Evidence on Hand

Ainda não existem sessões reais. Qualquer conversa de exemplo na UI é demonstração, marcada como tal, e só aparece em desenvolvimento.

## Product Principles

1. Evidência antes de afirmação: a interface nunca apresenta como sucesso o que não foi validado.
2. Estado visível: o usuário sempre sabe o que o agente está fazendo, com qual modelo e quanto contexto resta.
3. Conversa primeiro, detalhe sob demanda: atividade rotineira fica resumida; erros e decisões ficam à vista.
4. O usuário no controle: cancelar, aprovar, negar e reverter sempre a um gesto de distância.
5. Local e privado por padrão.

## Accessibility & Inclusion

- Uso completo por teclado.
- Respeitar `prefers-reduced-motion` e `prefers-contrast`.
- Contraste legível nos temas claro e escuro.
