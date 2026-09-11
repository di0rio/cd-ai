# 0007 — Nome do projeto: cd-ai

Status: **aceita** (2026-09-11).

## Decisão

O projeto passa a se chamar **cd-ai** (antes: Cauã AI). O nome vale para:

- nome exibido na interface e na janela;
- `productName` do Tauri e identifier `dev.cdai.desktop`;
- binário da CLI: `cd-ai`;
- crates `cd-ai-cli` e `cd-ai-desktop`, e pacote `@cd-ai/desktop`.

## O que não muda

- A pasta local do repositório continua `caua-ai`. Renomear a pasta fica a cargo do usuário, quando não houver sessões abertas nela.
- O crate `agent-core` mantém o nome.

## Ícone

O ícone é uma versão minimalista e vetorial de uma foto do autor: cabelo cacheado escuro e fone de ouvido azul, em fundo claro.

- Fonte única: `apps/desktop/public/icon.svg`.
- Os ícones do Tauri são gerados a partir dele com `tauri icon`.
