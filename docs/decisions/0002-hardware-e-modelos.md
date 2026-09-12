# 0002 — Hardware e modelos locais

Status: **aceita** (2026-09-11, com base em `docs/audit/benchmark-2026-09-11.md`), **com revisão em 2026-09-12** — ver "Revisão" no fim.

## Contexto

Máquina de desenvolvimento: Ryzen 5 5600X, 32 GB de RAM, GTX 1660 com 6 GB de VRAM (ver `docs/audit/fase-0-ambiente.md`). Nenhum modelo de 14B ou mais cabe inteiro na VRAM.

Cinco candidatos foram medidos: `qwen3:4b`, `qwen3:14b`, `qwen3-coder:30b`, `gemma4:26b` e `devstral-small-2`.

## Decisão

| Categoria | Modelo | Contexto | Motivo |
|---|---|---|---|
| FAST | `qwen3:4b` | 8k | Único que cabe 100% na VRAM (39 tok/s). Em 32k cai para 9 tok/s. |
| CODER | `qwen3-coder:30b` | 16k | MoE mais rápido (20 tok/s); 8/10 tool calls **com parser tolerante**. |
| Alternativa ao CODER | `gemma4:26b` | 16k | 15 tok/s, 7/10 tool calls no formato nativo do Ollama. |
| Escalonamento de formato | `devstral-small-2` | 8k | 10/10 tool calls, mas a 2 tok/s. Só quando o CODER falha repetidamente. |
| Descartado como padrão | `qwen3:14b` | — | 3,2 tok/s: denso demais para esta máquina. |

## Regras que decorrem disso

1. **Um único modelo grande residente.** CODER e alternativa (cerca de 19 GB cada) não cabem juntos na RAM. O Model Router prefere o modelo já carregado.
2. **O parser tolerante de tool calls é obrigatório antes do agente (Fase 4/5).** O `qwen3-coder` emite tool calls no formato próprio `<function=…><parameter=…>`. **Qualificada em 2026-09-12** (ver Revisão): o Ollama 0.34 converte esse formato num pedido com uma tool só, mas volta ao texto com as seis tools que o agente usa. A regra continua valendo. Ver `plans/009-parser-tolerante-tool-calls.md`.
3. **Contexto máximo de 16k no CODER.** Em 32k sobraram 0,6 GB de RAM livre.
4. **O FAST não edita código.** O `qwen3:4b` raciocina mesmo com `think: false` e estoura o orçamento de tokens em edições.
5. Nomes de modelos continuam sendo **configuração**. Nenhum código depende deles.

## Esclarecimento (2026-09-11)

Os modelos são **recursos locais descobertos dinamicamente pelo provider** (listagem do Ollama), não um catálogo embutido no código. As escolhas desta decisão são **defaults e recomendações para o hardware atual** (a máquina de desenvolvimento), não dependências hardcoded do produto. Em outro hardware, o provider lista o que existe e o cd-ai escolhe conforme a disponibilidade e o roteamento.

## Metadados de modelo vistos pelo Core

```text
Model
├── id
├── provider
├── capabilities (tools, structured output, ...)
├── context_length
├── parameter_count
├── quantization
├── memory_estimate
├── loaded
├── available
└── benchmark
```

## Revisitar quando

- houver uma versão do Ollama que converta o formato do `qwen3-coder`;
- houver mais RAM ou VRAM;
- o eval da Fase 6 mostrar taxas de sucesso diferentes das deste benchmark de tool calls isoladas.

## Revisão (2026-09-12)

Dois achados durante a Fase 5 mexem com esta decisão. Ambos foram medidos na mesma máquina, com o Ollama 0.34.0.

### 1. O Ollama 0.34 converte o formato do `qwen3-coder` — às vezes

A regra 2 dizia, sem qualificar, que o Ollama 0.34 não converte o formato próprio `<function=…><parameter=…>`. A tag instalada declara `PARSER qwen3-coder` (`ollama show --modelfile qwen3-coder:30b`), e num pedido isolado, com **uma** tool no request, a resposta volta com `tool_calls` nativos:

```json
[{"id": "call_jg7w1fx4", "function": {"name": "read_file", "arguments": {"path": "src/soma.ts"}}}]
```

Mas com as seis tools do agente no request, o mesmo modelo volta ao formato de texto. Isso foi observado duas vezes no mesmo dia: numa tarefa real do usuário pela UI e num teste de fumaça pela CLI, os dois com o markup `<function=…>` aparecendo no conteúdo da mensagem (inclusive um `</tool_call>` órfão, sem abertura).

Então a regra 2 não está superada, está **qualificada**: a conversão existe, mas não é confiável na configuração que o agente realmente usa. O parser tolerante de `crates/agent-core/src/tool_call.rs` continua carregando o peso, e não é candidato a remoção. O eval da Fase 6 deve medir a taxa de cada formato com o número de tools que o agente usa de verdade, não com uma tool isolada — foi exatamente essa diferença que produziu a leitura errada aqui.

Efeito colateral que a descoberta revelou, já corrigido: o conteúdo emitido como mensagem do assistente incluía o markup consumido pelo parser, então a conversa mostrava a chamada crua e a linha da ferramenta executada, em duplicidade.

### 2. A métrica que a tabela usou não é a que o agente sente

A tabela acima classifica os modelos por tokens de **geração** por segundo. Num agente, cada iteração do loop reenvia a conversa inteira, então o que domina o tempo de resposta é o **prefill** (processamento do prompt), não a geração. Medido com um prompt de 6,4k tokens, tamanho realista de uma iteração:

| Modelo | Prefill | Geração | Fração na GPU |
|---|---|---|---|
| `qwen3-coder:30b` | 6440 tok em 61,6 s (105 tok/s) | 17,2 tok/s | 4,2 GB de 20,4 GB (21%) |
| `qwen3:4b` | 6442 tok em 38,4 s (168 tok/s) | 12,8 tok/s | 79% |

Dos 63 s daquela chamada, 61,6 s foram prefill e 1,4 s foram geração. O `scripts/bench-models.ts` já coleta `prefillTokensPerSecond`; foi a leitura da tabela que privilegiou a geração.

O gargalo é a VRAM, não o modelo: com 20,4 GB de pesos e 6 GB de placa, 79% do CODER roda na CPU. Note que nem o `qwen3:4b` chega a 100% de GPU, porque o KV cache de 16k disputa a mesma memória.

Decorre disso, e ainda **não** está implementado:

- O cliente não manda `keep_alive` (`crates/agent-core/src/ollama.rs`, que envia só `num_ctx` nas options). Com o padrão de 5 minutos do Ollama, uma tarefa parada num prompt de aprovação perde o modelo da memória e paga o recarregamento de 20 GB do disco ao retomar. Aconteceu no aceite registrado em `docs/audit/fase-5-aceite.md`.
- Baixar o `num_ctx` do CODER de 16k para 8k libera VRAM do KV cache para mais camadas na GPU. A regra 3 fixou 16k por RAM livre; o critério de VRAM sugere revisitar o número.
- `OLLAMA_KV_CACHE_TYPE=q8_0` corta o KV cache pela metade pelo mesmo motivo. Não foi medido.

### 3. `cd-ai-coder`

O CODER passa a ter um Modelfile versionado em `models/cd-ai-coder.Modelfile`, derivado do `qwen3-coder:30b`, com `temperature` 0.15 e `top_p` 0.7 no lugar dos padrões do modelo (0.7 e 0.8), que são calibrados para conversa. Isso não muda nenhuma das medições acima — os pesos são os mesmos — e não contradiz a regra 5: o nome continua sendo configuração, descoberta pela listagem do Ollama, e nenhum código depende dele.
