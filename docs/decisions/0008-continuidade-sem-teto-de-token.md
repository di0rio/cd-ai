# 0008 — Continuidade sem limite financeiro ou quota artificial de tokens

Status: **aceita** (2026-09-16, Fase 10).

## Contexto

O projeto é local-first e exclusivamente local (decisão 0001/0004). **Não existe custo financeiro nem quota artificial de tokens**: a inferência acontece no hardware do usuário. O agente não deve parar por causa de plano, billing ou teto de tokens.

Isso **não** significa ausência de limites: o Core continua tendo limites operacionais e de segurança para evitar loops, travamentos e comportamento sem progresso (memória disponível, janela de contexto, tempo de execução, número de iterações, timeout, detecção de loop — SPEC §11.1).

A janela de contexto (32k no qwen3-coder:30b) é restrição de hardware, não de política — mas deve ser gerenciada para que a operação contínua seja possível.

## Decisão

O Context Manager (SPEC §16) e o sistema de checkpoints (SPEC §21) suportam continuidade automática:

1. **Compactação, não extensão.** Quando o contexto passa de 80% de `num_ctx` depois da higiene e do trim de resultados, o miolo da conversa vira um resumo **determinístico** extraído do `TaskState` (pedido, ficheiros lidos/alterados, comandos, iterações) e o loop segue. Não há um turno de LLM só para resumir: isso gastaria os tokens que a compactação deveria poupar. `ContextExhausted` só se o system prompt e o pedido já não cabem.
2. **Estado no disco, não na memória do modelo.** O progresso real da tarefa vive nos arquivos, no git e nos checkpoints. O que precisa ficar no contexto é o resumo e referências. Detalhes são re-lidos via tools (`read_file`, `search`) sob demanda.
3. **Só a segurança pausa.** O agente não para por custo ou quota de tokens. Interrupção só por approval gate (decisão 0001) ou cancellation explícita do usuário.

A compactação, como o `ContextTrimmed` da Fase 5, é em memória: o transcript em disco continua append-only.

## Conexões

- SPEC §16.3 — higiene de contexto
- SPEC §21 — checkpoints e rollback
- SPEC §11.1 — orquestrador com persistência de estado
- Plano 020 — implementação da Fase 10

## Revisitar quando

- O eval ao vivo medir qualidade de compaction/continuação (deve mostrar sem regressão);
- Houver um resumidor por LLM que prove ganho de taxa sem inflar tokens;
- O hardware mudar (mais RAM/VRAM ou modelos com janela maior que 32k).
