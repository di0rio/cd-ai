# 0002 — Hardware e modelos locais

Status: **provisória** (aguarda instalação do Ollama e benchmark).

## Contexto

Máquina de desenvolvimento: Ryzen 5 5600X, 32 GB RAM, GTX 1660 com 6 GB VRAM (ver `docs/audit/fase-0-ambiente.md`).

Configuração inicial sugerida:

| Categoria | Candidato |
|---|---|
| FAST | Qwen3 14B |
| CODER | Qwen3-Coder 30B-A3B |
| REASONER | Gemma 4 26B-A4B |
| ALTERNATIVE | Devstral Small 2 24B |

## Análise para este hardware

Tamanhos aproximados em quantização Q4 (confirmar com `ollama show` após o download):

- **Qwen3-Coder 30B-A3B** (MoE, ~3B parâmetros ativos por token): ~19 GB. Não cabe na VRAM; roda com offload parcial. Por ter poucos parâmetros ativos, é o candidato com mais chance de velocidade utilizável rodando majoritariamente em CPU.
- **Gemma 4 26B-A4B** (MoE, ~4B ativos): mesma classe de memória do CODER. **Não coexiste** com ele na RAM; usar só como escalonamento, aceitando o custo de recarga.
- **Qwen3 14B** (denso): ~9 GB. Não cabe nos 6 GB de VRAM; denso rodando parcialmente em CPU tende a ser lento. **Provavelmente não é "fast" neste hardware.** Candidato melhor para FAST: um modelo pequeno que caiba inteiro na VRAM (classe 4B–8B quantizado), a definir no benchmark.
- **Devstral Small 2 24B** (denso): ~14 GB. Espera-se lento aqui. Fica como alternativa, não como default.

## Decisão provisória

1. **Um único modelo residente é o caso normal.** O Model Router prefere o modelo carregado.
2. **CODER padrão:** Qwen3-Coder 30B-A3B, se o benchmark confirmar velocidade utilizável.
3. **REASONER:** só por escalonamento após falhas repetidas do CODER, nunca por padrão.
4. **FAST:** modelo pequeno que caiba inteiro na VRAM, escolhido no benchmark.
5. **Contexto inicial modesto** (começar em 16k e testar 32k). O KV cache compete com a RAM.
6. Nomes de modelos são **configuração**. Nenhum código depende deles.

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

## Benchmark (Fase 0b)

Para cada modelo candidato, com o app e um editor abertos:

- tempo de carga;
- tokens/s de prompt (prefill) e de geração, com contexto de 4k, 16k e 32k;
- pico de uso de RAM e VRAM;
- taxa de tool calls válidas em um conjunto fixo de prompts com tools e JSON schema.

Esses resultados fecham esta decisão e viram o primeiro dado do eval.
