# 0004 — Exclusivamente local

Status: **aceita** (esclarecida em 2026-09-11).

## Decisão

A v1 é **exclusivamente local**. Não existe API externa, cloud inference, telemetria, sincronização, login ou billing.

O Core usa uma interface abstrata de **Model Provider** para permitir providers futuros sem acoplamento, mas **nenhum provider remoto faz parte da v1**.

## Esclarecimento (2026-09-11)

- Anteriormente esta decisão dizia que um "provider compatível com OpenAI mantém a porta possível no futuro". A intenção agora é mais forte: a abstração do Model Provider existe **apenas** para evitar acoplamento de código com um provider específico — não é uma promessa de porta remota, nem um produto SaaS em potencial.
- O produto é um agente local de verdade, não uma plataforma com planos Free/Paid, entitlement ou quota de tokens.