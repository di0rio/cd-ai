# 0004 — Sem cloud na v1

Status: **aceita**.

## Decisão

A v1 não tem nenhuma porta para LLM externo: sem API paga, sem telemetria, sem sincronização e sem backend remoto.

O Model Provider baseado na API compatível com OpenAI mantém a porta arquiteturalmente possível no futuro, sem mudanças no Agent Core. Reabrir esta decisão exige um novo registro em `docs/decisions/`.
