# 0008 — Continuidade sem limite financeiro ou quota artificial de tokens

Status: **aceita** (2026-09-16, Fase 10).

## Contexto

O projeto é local-first e exclusivamente local (decisão 0001/0004). **Não existe custo financeiro nem quota artificial de tokens**: a inferência acontece no hardware do usuário. O agente não deve parar por causa de plano, billing ou teto de tokens.

Isso **não** significa ausência de limites: o Core continua tendo limites operacionais e de segurança para evitar loops, travamentos e comportamento sem progresso (memória disponível, janela de contexto, tempo de execução, número de iterações, timeout, detecção de loop — SPEC §11.1).

A janela de contexto (32k no qwen3-coder:30b) é restrição de hardware, não de política — mas deve ser gerenciada para que a operação contínua seja possível.

## Decisão

O Context Manager (SPEC §16) e o sistema de checkpoints (SPEC §21) devem suportar continuidade automática:

1. **Compactação, não extensão.** Quando o contexto estiver perto de estourar, condensar o histórico num resumo estruturado (objetivo, decisões tomadas, arquivos tocados, próximo passo) e seguir num turno novo com contexto limpo.
2. **Estado no disco, não na memória do modelo.** O progresso real da tarefa vive nos arquivos, no git e nos checkpoints. O que precisa ficar no contexto é o resumo e referências. Detalhes são re-lidos via tools (`read_file`, `search`) sob demanda.
3. **Só a segurança pausa.** O agente não para por custo ou quota de tokens. Interrupção só por approval gate (decisão 0001) ou cancellation explícita do usuário.

Na Fase 10 isso virou: higiene (leituras obsoletas, logs colapsados) → omitir resultados de tool antigos (75%) → compactação **determinística** do miolo da conversa (sem round-trip extra de LLM) → `ContextExhausted` só acima de 90% quando não há mais o que cortar com honestidade. A compactação por LLM (um resumo gerado) fica para se o eval ao vivo mostrar que o recado determinístico não basta.

## Conexões

- SPEC §16.3 — higiene de contexto
- SPEC §21 — checkpoints e rollback
- SPEC §11.1 — orquestrador com persistência de estado
- `plans/020-fase-10-context-manager.md` — implementação
- `plans/012-design-fase4-tool-engine.md` — design da Fase 4, que define o que o Tool Engine entrega antes do orquestrador da Fase 5
- `docs/design/daemon-modo-continuo.md` — modo contínuo (validação automática e tarefas)

## Revisitar quando

- O eval medir qualidade de compaction/continuação (deve mostrar sem regressão);
- A compactação determinística perder sinal demais e um resumo por LLM se justificar com métrica;
- O hardware mudar (mais RAM/VRAM ou modelos com janela maior que 32k).
