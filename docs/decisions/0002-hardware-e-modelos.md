# 0002 — Hardware e modelos locais

Status: **aceita** (2026-09-11, com base em `docs/audit/benchmark-2026-09-11.md`).

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
2. **O parser tolerante de tool calls é obrigatório antes do agente (Fase 4/5).** O `qwen3-coder` emite tool calls no formato próprio `<function=…><parameter=…>`, que o Ollama 0.34 não converte. Ver `plans/009-parser-tolerante-tool-calls.md`.
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
