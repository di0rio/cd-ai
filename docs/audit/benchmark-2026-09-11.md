# Benchmark de modelos locais — 2026-09-11

Script: `scripts/bench-models.ts`. Dados brutos: `docs/audit/benchmark-2026-09-11.json`.

**Máquina:** Ryzen 5 5600X, 32 GB de RAM, GTX 1660 com 6 GB de VRAM, Windows 11 e Ollama 0.34.0. O app e o editor estavam abertos durante a medição.

**Método, por modelo:**

- carga a frio seguida de um aquecimento;
- prompt de cerca de 4k tokens (código repetido);
- geração de 128 tokens com temperatura 0;
- medição com `num_ctx` de 8192 e de 32768;
- 10 casos de tool call com 4 ferramentas (`read_file`, `search`, `edit_file`, `run_command`), rodados com o modelo já carregado em 8k.

## Resultados

| Modelo | Carga (s) | Prefill tok/s (8k) | Geração tok/s (8k) | Geração tok/s (32k) | GPU (8k) | RAM livre após 32k | Tool calls válidas | Tempo dos 10 casos |
|---|---|---|---|---|---|---|---|---|
| qwen3:4b | 7 | 189 | 39,4 | 8,7 | 100% | 12,7 GB | 8/10 | 114 s |
| qwen3:14b | 32 | 53 | 3,2 | 3,2 | 39% | 5,5 GB | 9/10 | 74 s |
| qwen3-coder:30b | 29 | 102 | 20,1 | 15,2 | 21% | **0,6 GB** | 3/10 (8/10 com parser tolerante) | 29 s |
| gemma4:26b | 51 | 111 | 15,3 | 13,4 | n/d¹ | 0,8 GB | 7/10 | 42 s |
| devstral-small-2 | 18 | 42 | 2,1 | 2,0 | n/d² | 1,2 GB | **10/10** | 99 s |

¹ O `/api/ps` informou só 0,8 GB alocados para o gemma4. O valor não é confiável e precisa ser medido de novo.

² O script procurava o nome sem a tag (`devstral-small-2`), mas o `/api/ps` lista `devstral-small-2:latest`. Isso já foi corrigido no script; a memória desse modelo precisa ser medida de novo.

## O que os números dizem

- **qwen3:4b.** É o único que cabe inteiro na VRAM, e só em 8k: em 32k a geração cai de 39 para 9 tok/s. Ele raciocina mesmo com `think: false` (gasta de 200 a 700 tokens por tool call) e estourou o limite de 1024 tokens nos dois casos de edição. Serve como FAST para tarefas curtas; não serve para editar código.
- **qwen3-coder:30b.** É o modelo grande mais rápido (20 tok/s, graças à arquitetura MoE com 3B de parâmetros ativos). Os 5 casos "no_call" são chamadas **corretas**, escritas no formato nativo do modelo, com prosa opcional antes:

  ```text
  <function=read_file>
  <parameter=path>
  Cargo.toml
  </parameter>
  </function>
  </tool_call>
  ```

  O Ollama 0.34 não converteu esse formato em `tool_calls`. Com um parser tolerante (SPEC §4, item 3), fica em 8/10. Os 2 erros restantes foram `search` no lugar de `edit_file`, ou seja, ler antes de editar, o que é um comportamento razoável para um agente.

  O custo é memória: 18 GB alocados em 8k, e **só 0,6 GB de RAM livre em 32k**, com risco de swap.
- **gemma4:26b.** Chega a 15 tok/s e acertou 7/10. Os 3 erros foram de escolha de ferramenta (ler em vez de editar; `run_command` em vez de `search`). É a alternativa ao CODER.
- **devstral-small-2.** É o melhor em tool calling (10/10, com respostas curtas), mas gera a 2 tok/s. Lento demais como padrão.
- **qwen3:14b.** Acertou 9/10, mas gera a 3,2 tok/s. Como é um modelo denso rodando quase todo na CPU, não compensa diante dos MoE.

## Limitações desta medição

- O benchmark espera **uma** tool call por prompt. Um agente que lê o arquivo antes de editar é penalizado como `wrong_tool`, embora esteja certo.
- Foi uma única rodada, com temperatura 0 e sem repetições. Diferenças de ±1 caso não são significativas.
- A RAM livre depende do que estava aberto. Com o navegador e o editor fechados, os modelos de cerca de 19 GB ganham folga.
