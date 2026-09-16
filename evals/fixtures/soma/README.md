# Fixture de aceite: soma

Projeto mínimo com um teste que falha **de propósito**: `src/soma.ts` devolve `a - b` em vez de
`a + b`. Serve para o aceite da Fase 5 (`plans/015-fase-5-loop-ponta-a-ponta.md`), na CLI e na
interface.

Este fixture **não** entra no `bun run verify`, que roda os testes só em `apps/desktop`.

## Como usar

1. Copie esta pasta para um lugar **fora do repositório** (o agente vai editar arquivos):

   ```bash
   cp -r evals/fixtures/soma /tmp/soma
   ```

   No PowerShell:

   ```powershell
   Copy-Item -Recurse evals\fixtures\soma $env:TEMP\soma
   ```

2. Com o Ollama rodando e um modelo CODER disponível (decisão 0002), rode:

   ```bash
   cd-ai task --workspace <cópia> --model <modelo> "O teste de soma falha. Corrija e rode os testes."
   ```

3. Aprove as ações pelo terminal (`Aprovar? [s/N]`). Não existe aprovação automática no `task`.
   Para a suíte headless (Fase 6), use `cd-ai eval --scripted` ou `cd-ai eval --model <modelo>` —
   aí a cópia da fixture é aprovada sozinha. Ver [evals/README.md](../../evals/README.md).

## Pedido de aceite

> O teste de soma falha. Corrija e rode os testes.

## Esperado

O agente lê `src/soma.ts`, troca `a - b` por `a + b`, roda `bun test` (classe `validate`, aprovada
automaticamente). O Verifier confirma o exit 0 depois da última edição e termina com status
`completed` (`validated: true`) e código de saída 0.
