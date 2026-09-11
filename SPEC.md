# CAUÃ AI — LOCAL CODING AGENT (Especificação v2)

## 0. Como usar este documento

Este documento é a especificação para **construir** o Cauã AI. Ele é lido pelo agente/desenvolvedor que implementa o produto.

Ele **não** é o system prompt do agente em runtime. Os prompts de runtime do Cauã AI devem ser curtos, específicos por role e caber no orçamento de contexto de um modelo local (ver seção 17).

Prioridades absolutas, em ordem:

```text
CORRECTNESS > SECURITY > RELIABILITY > PERFORMANCE > MAINTAINABILITY > FEATURE COUNT
```

---

## 1. Objetivo do produto

Construir um aplicativo desktop chamado **Cauã AI**: um coding agent local, pessoal, standalone e extensível, focado exclusivamente em desenvolvimento de software.

O mesmo aplicativo trabalha em qualquer workspace permitido pelo usuário. Nada é instalado dentro dos projetos.

```text
Cauã AI
   ↓
Select Workspace
   ↓
/home/user/projeto-a
/home/user/projeto-b
...
```

Capacidades alvo:

- abrir e entender um workspace;
- conversar sobre o projeto;
- planejar tarefas;
- pesquisar, ler, criar, editar e remover arquivos;
- executar comandos, testes, typecheck, lint e build;
- trabalhar com Git;
- diagnosticar e corrigir erros;
- revisar alterações com base em evidência;
- aplicar skills sob demanda;
- escolher o modelo local adequado;
- manter contexto eficiente, histórico e checkpoints;
- recuperar-se de falhas;
- medir a própria taxa de sucesso.

---

## 2. Princípio fundamental: runtime de agente, não chatbot

O Cauã AI é um **Coding Agent Runtime**. Seu ciclo central:

```text
UNDERSTAND → PLAN → IMPLEMENT → EXECUTE → TEST → REVIEW → FIX → VALIDATE
```

**Uma tarefa só está concluída quando há evidência.** A resposta "terminei" do modelo não é evidência. Evidência é: comando executado, exit code, teste que passou, typecheck limpo, diff inspecionado.

Quando a validação não for tecnicamente possível (projeto sem testes, sem typecheck etc.), o relatório final deve dizer explicitamente **"não validado"** e listar o que não pôde ser verificado. Nunca declarar sucesso sem evidência.

---

## 3. Local-first e zero cloud por padrão

Por padrão, o Cauã AI não faz:

- chamadas a APIs de LLM externas;
- telemetria ou analytics;
- sincronização cloud;
- coleta ou upload de código;
- execução remota.

Qualquer operação de rede deve ser iniciada explicitamente pelo usuário. O código do projeto nunca sai da máquina por padrão.

**Porta cloud opt-in (fora da v1, decisão da Fase 0):** a arquitetura do provider (seção 10) permite, sem mudanças no Agent Core, que o usuário configure futuramente um endpoint externo com a própria API key, desligado por padrão e habilitado por tarefa. Não implementar na v1 salvo decisão explícita na Fase 0.

---

## 4. A realidade do modelo local (restrição que guia tudo)

A qualidade do agente é limitada pela qualidade do modelo local. Modelos open-weight pequenos e médios:

- erram o formato de tool calls e JSON com frequência;
- erram diffs e edições em arquivos grandes;
- têm janela de contexto efetiva pequena;
- são lentos para gerar output, e trocar de modelo custa segundos (carregar/descarregar da VRAM).

Consequências obrigatórias de arquitetura:

1. **Determinístico antes do LLM.** Tudo que pode ser resolvido por código (detectar stack, escolher skills, classificar comando, rotear modelo, revisar diff) é resolvido por código. O LLM entra só onde há ambiguidade real.
2. **Saída estruturada forçada.** Usar structured output / JSON schema do runtime quando disponível para tool calls.
3. **Parser tolerante + loop de reparo.** Resposta malformada gera uma mensagem de erro curta para o modelo ("formato inválido: esperado X") e nova tentativa, com limite configurável.
4. **Contexto configurado explicitamente.** O tamanho de contexto enviado ao runtime é sempre definido pelo Cauã AI com base nos metadados do modelo e na memória disponível. Nunca confiar no default do runtime, que pode ser menor que o máximo do modelo e truncar o prompt silenciosamente.
5. **Prompts curtos.** Prompts de runtime por role, skills condensadas e orçamento de tokens por seção.
6. **Poucas trocas de modelo e de role.** Cada troca custa prefill ou carga de modelo.

---

## 5. Decisões da Fase 0

Registradas em `docs/decisions/`. O relatório de ambiente está em `docs/audit/fase-0-ambiente.md`. Mudar uma decisão exige um novo registro.

### 5.1 Plataforma — `0001` (aceita)

- Fases 1–6 desenvolvidas no **Windows 11 nativo**.
- Alvo do primeiro release: **Linux x86_64** (`.deb`, AppImage).
- Linux (WSL2 ou VM) entra obrigatoriamente na Fase 7 (sandbox) e na Fase 14 (empacotamento).
- Código portável desde o início: paths via APIs de path, execução de comandos atrás de um módulo de plataforma.
- Até existir sandbox, todo comando fora das classes `read`/`validate` exige aprovação.

### 5.2 Hardware e modelos — `0002` (provisória até o benchmark)

Máquina: Ryzen 5 5600X, 32 GB RAM, GTX 1660 com 6 GB VRAM. Nenhum modelo de 14B+ cabe inteiro na VRAM.

- **Um único modelo residente é o caso normal.**
- CODER candidato: Qwen3-Coder 30B-A3B (MoE, poucos parâmetros ativos).
- REASONER: Gemma 4 26B-A4B, só por escalonamento (não coexiste com o CODER na RAM).
- FAST: modelo pequeno que caiba inteiro na VRAM, escolhido no benchmark.
- Devstral Small 2 24B: alternativa.
- Contexto inicial modesto (16k, testar 32k).
- Nomes de modelos são configuração; nenhum código depende deles.

### 5.3 Agent Core em Rust — `0003` (aceita)

Cargo workspace. `src-tauri` é só o adaptador desktop; a CLI headless (`apps/cli`) usa o mesmo core. Crates são criados sob demanda, não antecipadamente. Toda validação de path, permissão, classificação de comando, sandbox e redação de secrets vive no Rust. A webview nunca é fronteira de confiança.

### 5.4 Sem cloud na v1 — `0004` (aceita)

Sem API paga, sem telemetria, sem backend remoto. O provider compatível com OpenAI mantém a porta possível no futuro.

### 5.5 Frontend Next.js com static export — `0005` (aceita)

Next.js em `output: "export"`, servido pelo Tauri a partir de `out/`. Sem SSR, API routes, Server Actions, middleware ou runtime Node.

### 5.6 Estrutura do monorepo — `0006` (aceita)

Ver seção 7.1.

---

## 6. Stack

Versões estáveis atuais, verificadas na instalação:

```text
Desktop:          Tauri 2 (CLI via pacote npm @tauri-apps/cli)
Frontend:         Next.js (static export) + React + TypeScript
Componentes:      Base UI / coss (adicionados quando o primeiro componente real precisar)
Styling:          Tailwind CSS
Package manager:  Bun (workspaces)
Lint/format:      Biome (frontend), rustfmt + clippy (Rust)
Native/core:      Rust (Cargo workspace)
Busca de código:  ripgrep + tree-sitter
Local LLM:        Ollama
```

Antes de adicionar qualquer dependência:

1. existe solução no projeto?
2. é realmente necessária?
3. uma API nativa resolve?
4. mantém bundle e arquitetura simples?

---

## 7. Arquitetura

```text
┌─────────────────────────────┐   ┌───────────────────┐
│        Desktop UI           │   │   CLI headless    │
│   (React, só apresentação)  │   │ (eval, automação) │
└──────────────┬──────────────┘   └─────────┬─────────┘
               │ IPC (Tauri)                │
               └──────────────┬─────────────┘
                              ▼
┌───────────────────────────────────────────────────────┐
│                 CORE (Rust) — fronteira de confiança  │
│                                                       │
│  Orchestrator (máquina de estados, determinística)    │
│        │                                              │
│        ├── Context Manager ── Repo Map / Cache        │
│        ├── Skill Router (regras + tags)               │
│        ├── Model Router (heurística + VRAM)           │
│        │        │                                     │
│        │   Model Provider ── Ollama                   │
│        │                                              │
│        ├── Roles: Explorer / Coder / Verifier         │
│        │                                              │
│        └── Tool Engine                                │
│               │                                       │
│        Permission Manager + Secret Redactor           │
│               │                                       │
│        ┌──────┼──────────┬─────────────┐              │
│      Files  Shell      Git       Checkpoints          │
│             (sandbox)          (shadow repo)          │
│                                                       │
│  Eventos → UI / Histórico / Métricas / Trajetórias    │
└───────────────────────────────────────────────────────┘
```

Boundaries fundamentais que devem ser preservadas:

```text
Agent Core | Model Provider | Tools | Permissions | Skills | Context | Workspace
```

### 7.1 Estrutura do repositório (decisão `0006`)

```text
caua-ai/
├── apps/
│   ├── desktop/          # Next.js (static export) — só apresentação
│   └── cli/              # binário Rust headless (caua-ai)
├── crates/
│   └── agent-core/       # demais crates só quando uma fronteira real surgir
├── src-tauri/            # adaptador desktop: commands/events → agent-core
├── packages/             # sob demanda: ui, contracts, config, skills
├── evals/                # Fase 6: tasks/, repos/, results/
├── docs/
│   ├── audit/
│   └── decisions/
├── scripts/
├── Cargo.toml            # Cargo workspace
├── package.json          # Bun workspaces
├── biome.json
└── SPEC.md
```

Não criar diretórios, crates ou packages vazios "para depois". Até uma fronteira justificar um crate próprio, ela é um módulo dentro de `agent-core`.

---

## 8. Escopo

### 8.1 Nunca construir

- framework genérico de agentes;
- plugin marketplace;
- multi-tenant, SaaS, cloud orchestration;
- agentes distribuídos;
- workflow builder / no-code;
- abstrações sem uso real.

### 8.2 Adiado para depois da v1

- controle de concorrência e locks de arquivo (v1 é sequencial);
- execução paralela de roles;
- roles especializados Security/Performance/UI (na v1 são skills);
- métricas detalhadas por sub-etapa;
- RAG/embeddings;
- porta cloud (salvo decisão na Fase 0);
- treinamento/fine-tuning.

---

## 9. Model Provider

Interface simples para o Agent Core não depender do Ollama diretamente.

Responsabilidades:

- listar modelos;
- verificar disponibilidade e saúde;
- obter metadados (janela de contexto máxima, tamanho, quantização quando disponível);
- listar modelos atualmente carregados;
- enviar mensagens com tools e/ou JSON schema;
- receber streaming;
- cancelar geração;
- controlar tamanho de contexto e `keep_alive`.

Implementação:

- **Um provider baseado na API compatível com OpenAI** (chat completions com streaming e tools), que Ollama, llama.cpp, LM Studio e vLLM expõem.
- **Extras específicos do Ollama** (listar modelos, modelos carregados, metadados, keep_alive, opções de contexto) isolados num módulo pequeno.

Isso torna providers futuros quase gratuitos sem criar abstração especulativa. Não implementar outros providers agora.

Verificar na documentação atual do Ollama, durante a Fase 3, o suporte a tool calling, structured outputs e opções de contexto da versão instalada.

---

## 10. Model Router

Categorias: `FAST`, `CODER`, `REASONER`. Os modelos de cada categoria são configurados pelo usuário. Nunca hardcodar nomes de modelos.

Seleção por **heurística determinística**, considerando:

- tipo de tarefa (classificação da seção 11.2);
- tamanho de contexto necessário;
- **qual modelo já está carregado** (trocar custa segundos; preferir o residente);
- memória disponível;
- disponibilidade do modelo;
- histórico de sucesso no eval (seção 26), quando existir.

Regras:

- Se só um modelo está configurado ou cabe na memória, todas as categorias usam ele. Esse é o caso normal.
- Trocar de modelo no meio de uma tarefa só se o ganho esperado justificar (ex.: escalar para REASONER após N falhas do CODER).
- Toda decisão de roteamento é registrada com o motivo (seção 28).

---

## 11. Agent Core

### 11.1 Loop

```text
User Task
  → Classificação (determinística; LLM só se ambígua)
  → Context Discovery (Explorer, se necessário)
  → Skill Selection (regras)
  → Plano (proporcional à complexidade)
  → Model ⇄ Tool Calls ⇄ Tool Results
  → Verificação determinística (Verifier)
  → Correção (se falhou, com limite)
  → Relatório final com evidências
```

O loop possui:

- limite de iterações por tarefa e por fase;
- timeout por chamada de modelo, por tool e por tarefa;
- cancelamento a qualquer momento (propaga para modelo e processos filhos);
- retry controlado com limite configurável;
- detecção de loop (mesma tool call com mesmos argumentos repetida, mesmo erro repetido) que interrompe e pede intervenção;
- estado persistente (retomável após crash);
- eventos e métricas.

O usuário pode **intervir no meio da tarefa** (mensagem de correção de rumo) sem cancelar tudo.

### 11.2 Orchestrator

Na v1, o Orchestrator é **código** (máquina de estados), não um role de LLM.

Responsabilidades:

- classificar a tarefa: `trivial` (arquivo e mudança explícitos), `normal`, `complexa`, `pergunta` (sem edição);
- decidir quais roles executar;
- selecionar skills e modelo;
- controlar fases, estado, limites e retries;
- evitar trabalho redundante;
- consolidar o relatório final.

Exemplos de fluxo:

```text
pergunta:  Explorer → resposta (sem edição)
trivial:   Coder → Verifier
normal:    Explorer → plano curto → Coder → Verifier
complexa:  Explorer → plano → aprovação do plano (em ASK) → Coder → Verifier → correção
```

### 11.3 Modo plano (read-only)

Modo em que o agente só pode ler e pesquisar e entrega um plano. Útil para tarefas grandes e para revisão antes de editar.

---

## 12. Roles da v1

Na v1, roles são prompts e conjuntos de tools diferentes sobre o mesmo modelo. São só três.

### 12.1 Explorer (read-only)

Entende o codebase o suficiente para a tarefa. Nunca edita.

- Começa do **repo map** e da detecção determinística de stack (seção 16).
- Busca incremental: tarefa → símbolos relevantes → referências → imports → arquivos → trechos.
- Nunca lê o projeto inteiro.
- Entrega: arquivos relevantes, trechos, testes relacionados, comandos de validação detectados, riscos.

### 12.2 Coder

Implementa, e também planeja quando a tarefa é normal/complexa (o plano é uma fase do Coder, não um role separado).

Plano, quando aplicável: objetivo, arquivos, etapas, riscos, como validar, definição de pronto. Proporcional à tarefa.

Regras de implementação:

- ler → entender → modificar;
- editar só o necessário, seguindo os padrões do projeto;
- sem refactors não relacionados, sem abstrações desnecessárias;
- tipagem forte, sem código temporário, sem TODO no lugar de implementação;
- remover código morto causado pela própria alteração;
- comentários só quando agregam.

Debugging (quando há erro concreto) segue o fluxo:

```text
Erro → Reproduzir → Inspecionar → Causa raiz → Correção mínima → Reproduzir → Validar
```

Diferenciar: causa raiz, sintoma, problema ambiental, problema preexistente, regressão. Sem tentativas aleatórias: cada tentativa declara a hipótese que está testando.

### 12.3 Verifier

Julga o resultado **por evidência**, não por opinião. Detalhes na seção 13.

### 12.4 Especialistas como skills

Security, Performance, UI, Git e Accessibility são **skills** carregadas no Coder/Verifier quando relevantes (seção 17), não agentes separados.

Promover um especialista a role próprio só quando o eval (seção 26) mostrar ganho mensurável.

---

## 13. Verificação e review

### 13.1 Verificação determinística (primeiro, sempre)

O Verifier executa e checa, em ordem de custo:

1. **Parse sintático** dos arquivos alterados (tree-sitter): arquivo quebrado é detectado sem rodar nada.
2. **Checks de diff:**
   - arquivos alterados fora do plano;
   - TODO/FIXME/código de debug introduzidos;
   - arquivos de secrets tocados;
   - arquivos gerados/lockfiles alterados inesperadamente;
   - tamanho do diff desproporcional à tarefa.
3. **Typecheck** direcionado quando possível.
4. **Lint** nos arquivos alterados.
5. **Testes direcionados** (testes do arquivo/módulo alterado), depois relacionados.
6. **Build/suíte completa** só quando a tarefa justificar ou o usuário pedir.

Os comandos de validação são detectados deterministicamente (scripts do `package.json`, `Cargo.toml`, configs de test runner) e cacheados por workspace.

### 13.2 Review por LLM (segundo, opcional)

O mesmo modelo tende a aprovar o próprio trabalho. Por isso o review por LLM:

- recebe só o diff, o requisito e os resultados da verificação, não a conversa do Coder;
- verifica requisito, edge cases, tratamento de erros e regressões óbvias;
- não bloqueia por estética;
- resultado: `PASS` ou `CHANGES_REQUIRED` com motivos concretos.

Uma falha determinística sempre prevalece sobre um `PASS` do LLM.

### 13.3 Ciclo de correção

```text
implement → verify → (falhou) → diagnosticar → corrigir → verify → ... (até o limite)
```

Ao atingir o limite de retries, parar e reportar o estado real: o que passou, o que falhou, a última hipótese.

---

## 14. Formato de edição

A forma como o modelo edita arquivos é um dos maiores fatores de sucesso. Regras:

- **Arquivo novo:** `write_file` com o conteúdo completo.
- **Arquivo existente:** blocos **search/replace**. O trecho de busca precisa bater exatamente com o arquivo.
  - Zero matches: rejeitar e devolver ao modelo o erro e o trecho real mais próximo.
  - Múltiplos matches: rejeitar e pedir mais contexto no bloco de busca.
  - Fallback fuzzy só normalizando whitespace, e registrado no evento.
- Reescrita completa de arquivo existente só abaixo de um tamanho configurável.
- Toda edição passa pelo parse sintático (seção 13.1) imediatamente.
- Toda edição gera evento `file.changed` com o diff.

Estudar o formato de edição usado por ferramentas existentes (ex.: Aider) antes de definir o formato final, e validar no eval.

---

## 15. Tools

### 15.1 Conjunto mínimo (primeiro loop funcional)

```text
read_file        (com faixa de linhas)
search           (ripgrep: texto/regex, respeitando .gitignore)
edit_file        (search/replace, seção 14)
write_file       (arquivo novo)
list_directory
run_command      (shell, sob permissões e sandbox)
```

### 15.2 Adicionadas depois

```text
delete_file
create_directory
search_symbols   (tree-sitter)
get_file_metadata
git_status / git_diff / git_log / git_branch
```

### 15.3 Regras

- Resultados sempre estruturados (sucesso/erro, dados, truncamento indicado).
- Outputs grandes são truncados com indicação clara e forma de pedir mais (ex.: faixa de linhas).
- Cada chamada emite `tool.started` e depois `tool.completed` ou `tool.failed`.
- Toda chamada passa pelo Permission Manager antes de executar.
- Leituras independentes podem rodar em paralelo. Escritas são sequenciais na v1.

---

## 16. Context Manager

Um dos componentes mais importantes. Nunca enviar o projeto inteiro.

### 16.1 Fontes de contexto

- **Perfil do workspace (determinístico, cacheado):** linguagens, package manager, framework, configs de TS/lint/test, comandos de validação, regras do projeto (ex.: arquivos de instruções do repo).
- **Repo map:** lista compacta de arquivos com símbolos e assinaturas principais (tree-sitter), priorizada por relevância para a tarefa.
- **Trechos relevantes** encontrados pelo Explorer.
- **Skills** selecionadas (versões condensadas).
- **Memória** relevante (seção 24).
- **Histórico** da tarefa atual, compactado.

Sem embeddings/RAG na v1.

### 16.2 Orçamento de tokens

O prompt é montado por seções, cada uma com orçamento definido como fração da janela de contexto efetiva do modelo em uso. Exemplo de seções: instruções do role, regras do projeto, skills, repo map, arquivos/trechos, histórico, resposta reservada.

Sempre reservar espaço para a resposta do modelo. Quando uma seção excede o orçamento, cortar de forma explícita (priorizando o mais relevante) e registrar o corte.

### 16.3 Higiene

Remover do contexto: duplicações, logs enormes (manter só as linhas decisivas: erro, arquivo, linha), arquivos irrelevantes, resultados obsoletos (versões antigas de um arquivo já editado), skills não usadas.

### 16.4 Cache

Cachear o perfil do workspace e o repo map. Invalidar por mudança de arquivo (watcher de filesystem ou hash/mtime), não por tempo. Reindexar só o que mudou.

---

## 17. Skills

### 17.1 Conceito

```text
Skill = como pensar/trabalhar (conhecimento operacional)
Tool  = o que o agente consegue executar
```

As skills vêm embutidas no aplicativo. O usuário não copia nada para os projetos.

Catálogo inicial conceitual:

```text
typescript, react, nextjs, tailwind, coss, coss-particles, emil-design-eng,
apple-design, impeccable, ponytail, caveman, frontend-design, testing,
debugging, security, code-review, performance, git, accessibility
```

### 17.2 Condensação para modelo local

Skills escritas para modelos frontier com contexto grande estouram o contexto de um modelo local. Cada skill embutida deve ter:

- **versão condensada** (a usada por padrão), com orçamento de tokens máximo definido;
- referência para a versão completa, quando existir e a licença permitir.

A qualidade das versões condensadas é validada no eval.

Uso interno sugerido: um estilo de comunicação comprimido (no espírito da skill `caveman`) nos handoffs internos entre roles e nos resumos de contexto, para reduzir tokens de saída, que são a parte mais lenta no local. Validar no eval que isso não piora a taxa de sucesso.

### 17.3 Skill Router

Seleção **determinística** por:

- perfil do workspace (framework e linguagens detectados);
- extensões e caminhos dos arquivos envolvidos;
- palavras-chave e tipo da tarefa;
- tags, dependências e conflitos declarados pela skill.

LLM só para desempate quando ambíguo. Sempre registrar por que cada skill foi carregada ou pulada (`skills.detected`, `skill.loaded`, `skill.skipped`).

### 17.4 Governança

Cada skill possui metadados: nome, descrição, versão, origem, licença, dependências, conflitos, tags, permissões, status, orçamento de tokens, conteúdo.

Para skills externas:

- **verificar a licença antes de embutir; skill sem licença que permita redistribuição não é embutida** (bloqueador da fase de skills);
- preservar atribuição;
- registrar origem;
- não inventar conteúdo ausente.

Mecanismo futuro de importação/atualização local. Sem marketplace.

---

## 18. Terminal

- streaming de stdout e stderr separados;
- exit code;
- timeout e cancelamento (matando a árvore de processos inteira);
- visualização de processos em execução;
- nunca esconder erros;
- output passa pelo Secret Redactor antes de ir para o modelo, a UI ou o histórico.

---

## 19. Git

- `status`, `diff`, `log`, `branch`;
- `git status` antes de alterações grandes, `git diff` depois;
- commits automáticos configuráveis e **desligados por padrão**.

Nunca, sem autorização explícita: force push, reset destrutivo, reescrever/apagar histórico, sobrescrever mudanças do usuário, operações que falem com remote.

---

## 20. Segurança

### 20.1 Fronteira do workspace

O workspace escolhido é a fronteira padrão para as tools de arquivo. Validar:

- path absoluto e canônico;
- symlinks (resolver e checar se o destino continua dentro do workspace);
- path traversal (`../`);
- no Windows, se aplicável: letras de drive, UNC, junctions.

O agente não acessa o filesystem arbitrariamente só porque o modelo pediu.

### 20.2 Classificação de comandos

Antes de executar, todo comando é classificado deterministicamente:

```text
read         (ls, cat, git status, git diff...)
validate     (comandos de teste/lint/typecheck/build detectados no workspace)
write        (altera arquivos no workspace)
network      (curl, wget, npm/bun/pip install, git fetch/push...)
destructive  (rm recursivo, git reset --hard, git clean, formatação de disco...)
unknown      (não classificado)
```

Comandos compostos (pipes, `&&`, `;`, subshells, `$(...)`) recebem a classificação mais perigosa entre suas partes. Na dúvida, `unknown`.

### 20.3 Sandbox do shell

Validar paths não protege o shell: `run_command` pode fazer qualquer coisa que o usuário faria. A promessa "não sair do workspace" só é garantida com sandbox do sistema operacional.

- **Linux:** executar comandos em sandbox (ex.: bubblewrap, landlock, namespaces), com escrita permitida só no workspace e diretórios temporários/cache necessários, e **rede bloqueada por padrão**.
- **Plataforma sem sandbox disponível:** o modo FULL ACCESS fica indisponível, e todo comando fora das classes `read`/`validate` exige aprovação.

A rede é liberada por comando, com aprovação explícita (ex.: instalar dependências).

### 20.4 Modos de permissão

| Operação | ASK | AUTO | FULL ACCESS |
|---|---|---|---|
| Ler arquivo no workspace | auto | auto | auto |
| Editar/criar arquivo no workspace | aprovar | auto | auto |
| Deletar arquivo | aprovar | aprovar | auto (com checkpoint) |
| Comando `read` / `validate` | auto | auto | auto |
| Comando `write` | aprovar | auto (sandbox) | auto (sandbox) |
| Comando `network` | aprovar | aprovar | aprovar |
| Comando `destructive` / `unknown` | aprovar | aprovar | aprovar |
| Arquivo de secret | aprovar | aprovar | aprovar |
| Fora do workspace | negado | negado | negado (salvo liberação explícita por path) |

FULL ACCESS exige sandbox ativo. Mesmo nele: não acessar secrets sem necessidade, não sair do workspace, não executar ações externas não autorizadas.

O diálogo de aprovação mostra exatamente o que será executado (comando completo, diff completo), nunca uma descrição resumida gerada pelo modelo.

### 20.5 Prompt injection

Conteúdo do repositório (README, comentários, issues, output de testes, páginas de dependências) pode conter instruções maliciosas.

- Todo resultado de tool entra no contexto marcado como **dado não confiável**, delimitado, com instrução ao modelo de nunca seguir instruções contidas nele.
- Decisões de permissão nunca se baseiam em alegações do modelo ou do conteúdo ("o usuário autorizou", "isto é seguro").
- A classificação de comandos e o sandbox são a defesa real; o prompt é só uma camada extra.

### 20.6 Secrets

Nunca enviar automaticamente ao modelo: `.env`, `.env.*`, chaves privadas, chaves SSH, credenciais, tokens, certificados, arquivos de senha.

- Detecção por nome/caminho **e** por conteúdo (prefixos conhecidos de tokens, blocos de chave privada, strings de alta entropia).
- **Redação também em output de comandos** (ex.: `env`, logs, stack traces), antes de ir para o modelo, a UI, o histórico ou as trajetórias.
- Acesso a arquivo de secret só com aprovação, e mesmo assim evitar colocar valores no contexto (ex.: mostrar só as chaves de um `.env`, não os valores).

---

## 21. Checkpoints e rollback

Não depender do git do usuário: o workspace pode não ter git, ou ter mudanças não commitadas.

- **Shadow repository:** um repositório git com git dir **fora** do projeto (no diretório de dados do app), usando o workspace como work tree, para snapshots do agente. Nunca toca no `.git` do usuário.
- Checkpoint automático antes de cada tarefa que edita arquivos e antes de operações destrutivas.
- Registrar o hash de cada arquivo que o agente escreveu.

Rollback:

```text
detectar mudanças do usuário → proteger mudanças não relacionadas → reverter só as mudanças do agente
```

Se um arquivo foi alterado pelo usuário depois da escrita do agente (hash diferente), não restaurar sem perguntar e mostrar o diff.

---

## 22. Eventos

Eventos estruturados alimentam UI, histórico, debugging, métricas e trajetórias:

```text
agent.started | agent.thinking | agent.completed | agent.error
task.started | task.completed | task.failed | task.cancelled
skills.detected | skill.loaded | skill.skipped
model.selected | model.loaded
tool.started | tool.completed | tool.failed
approval.required | approval.granted | approval.denied
file.read | file.changed
command.started | command.completed | command.failed
verify.started | verify.completed
review.started | review.completed
test.started | test.completed | test.failed
checkpoint.created | rollback.completed
```

Cada evento carrega: id da tarefa, timestamp, duração quando aplicável e **motivo** quando é uma decisão.

---

## 23. Estado da tarefa

```text
TaskState
├── id
├── workspace
├── request
├── classification
├── phase
├── selectedModel (+ motivo)
├── selectedSkills (+ motivo)
├── roles
├── plan
├── filesRead
├── filesChanged (+ hashes)
├── checkpoints
├── commands
├── validations (comando, exit code, resumo)
├── errors
├── retries
├── metrics
└── status
```

Status:

```text
queued | running | waiting_approval | verifying | reviewing | fixing |
completed | completed_unvalidated | failed | cancelled
```

`completed_unvalidated` existe para que a falta de evidência nunca seja mascarada como sucesso.

O estado é persistido e a tarefa pode ser retomada após crash ou reinício do app.

---

## 24. Histórico e memória

### 24.1 Histórico

Salvar localmente, associado ao workspace: tarefas, mensagens, arquivos modificados, comandos, validações, reviews, métricas, decisões. Sem secrets (passa pelo redactor).

### 24.2 Memória estruturada

Sem RAG na v1. Memória pequena, local, editável pelo usuário e invalidável:

- regras do projeto;
- decisões de arquitetura;
- decisões e preferências do usuário;
- restrições importantes;
- fatos aprendidos (ex.: "testes deste projeto precisam de X").

Só a memória relevante para a tarefa entra no prompt, dentro do orçamento (seção 16.2).

---

## 25. Trajetórias (base para o futuro modelo próprio)

Opt-in, local, desligado por padrão.

Salvar cada tarefa em formato estável e versionado (ex.: JSONL):

- tarefa e classificação;
- contexto enviado (ou referências a ele);
- sequência de mensagens e tool calls;
- diffs;
- resultados de validação (pass/fail, exit codes);
- resultado final e intervenção humana, se houve.

Isso não exige nenhuma infraestrutura de ML e forma o dataset para o futuro:

```text
Trajetórias validadas → filtro de qualidade → deduplicação → validação → fine-tuning/distillation → Cauã Coder
```

Não implementar treinamento agora.

---

## 26. Eval local (medir se o agente melhora)

Métricas de latência não dizem se o agente ficou melhor. O eval diz.

- Diretório `evals/` com 20–50 tarefas fixas sobre repositórios de teste (fixtures).
- Cada tarefa tem um **check automático** (ex.: um teste que só passa se a tarefa foi bem resolvida) e um limite de tempo.
- Rodado pela **CLI headless**, sem UI.
- Reporta por execução: taxa de sucesso, iterações, tokens, tempo, retries, falhas de formato de tool call, edições rejeitadas.
- Resultados salvos para comparar entre versões de prompt, modelo, skill e formato de edição.

Regra: mudança em prompt, formato de edição, context manager, skill ou router só é mantida se o eval não piorar.

---

## 27. Métricas

Locais, nunca enviadas a servidores.

v1 (por tarefa): duração, sucesso/validado, tokens de entrada/saída, latência do modelo, tool calls, comandos, retries, arquivos lidos/alterados.

Depois (quando o eval indicar gargalo): latência por tool, cache hits/misses, tempo de carga de skills, breakdown por fase:

```text
Task
├── Context discovery: 320ms
├── Model: 4.1s
├── Tools: 120ms
├── Validação: 3.7s
└── Review: 1.8s
```

---

## 28. Observabilidade

Logs estruturados locais que respondam:

- Por que esta skill foi carregada?
- Por que este modelo foi escolhido?
- Por que este arquivo foi lido?
- Por que o contexto foi cortado?
- Por que o agente tentou de novo?
- Por que a tarefa falhou?
- Quanto tempo cada etapa levou?

Sem conteúdo sensível desnecessário.

---

## 29. UI

Interface desktop profissional, com informação progressiva, sem virar dashboard.

```text
┌──────────────────────────────────────────────┐
│ Cauã AI                        Model / Status│
├────────────┬─────────────────────┬───────────┤
│ Workspace  │                     │ Activity  │
│ Files      │       Chat          │ Tools     │
│ Git        │                     │ Skills    │
├────────────┴─────────────────────┴───────────┤
│ Terminal / Diff / Validação / Output         │
└──────────────────────────────────────────────┘
```

Mostrar: workspace, modelo (e se está carregado), tarefa, fase, arquivos alterados, tool calls, comandos, validações com evidência, diff, skills carregadas, approvals, erros.

Essencial:

- aprovação com diff/comando completo e opções aprovar / negar / editar;
- botão de cancelar sempre visível;
- campo para corrigir o rumo no meio da tarefa;
- relatório final com evidências e status honesto (validado / não validado);
- rollback de uma tarefa pela UI;
- modo plano (read-only) acessível.

Para construir a UI, usar quando relevante as skills `coss`, `emil-design-eng`, `apple-design`, `impeccable`, `ponytail`, `tailwind`, `frontend-design`, `accessibility`. Não criar design system paralelo. Priorizar hierarquia, espaçamento, tipografia, teclado, estados (loading, erro, vazio) e acessibilidade.

---

## 30. Configuração

```text
modelos: default, fast, coder, reasoner
ollama: endpoint, keep_alive, tamanho de contexto por modelo (override)
permissões: modo, allowlist de comandos por workspace
workspace(s)
limites: max retries, max iterações, timeouts
contexto: orçamentos por seção
skills: habilitadas/desabilitadas
trajetórias: on/off
tema
```

Config global + override por workspace (armazenado no diretório de dados do app, não no projeto).

---

## 31. Gerenciamento de modelos

A UI deve:

- detectar o Ollama e verificar a conexão;
- listar modelos locais e os carregados;
- mostrar janela de contexto e tamanho quando disponível;
- estimar se o modelo cabe na memória com o contexto configurado;
- permitir selecionar modelos por categoria;
- identificar modelos configurados mas ausentes.

Nunca baixar modelos sem ação explícita do usuário.

---

## 32. Testes do próprio Cauã AI

- **Agent Core:** transições de estado, retries, detecção de loop, cancelamento, conclusão, falhas, retomada após crash, `completed_unvalidated`.
- **Model Provider:** streaming, cancelamento, resposta malformada, loop de reparo, contexto configurado.
- **Model Router:** seleção, modelo indisponível, preferência pelo modelo carregado, fallback.
- **Skill Router:** skill relevante, irrelevante, dependências, conflitos, orçamento.
- **Context Manager:** seleção, orçamento, truncamento, cache, invalidação.
- **Edição:** match exato, zero matches, múltiplos matches, fallback de whitespace, parse após edição.
- **Permission Manager:** cada linha da tabela 20.4, classificação de comandos compostos.
- **Sandbox:** escrita fora do workspace bloqueada, rede bloqueada.
- **Workspace:** path válido, symlink para fora, traversal, junction (Windows).
- **Secret Redactor:** arquivos, output de comandos, trajetórias.
- **Checkpoints:** snapshot, rollback só do agente, proteção de mudança do usuário.
- **Tool Engine:** sucesso, falha, timeout, cancelamento, truncamento.
- **UI:** fluxos de aprovação, cancelamento, rollback.
- **Eval:** a própria suíte roda na CI local.

---

## 33. Definição de pronto

Uma funcionalidade do Cauã AI está pronta quando, onde aplicável:

```text
implementada + typecheck + lint + testes + review + eval sem regressão
```

"Funciona na minha cabeça" não é validação.

---

## 34. Fases de implementação

Construir um **fatiado vertical** cedo (loop mínimo de ponta a ponta) em vez de camadas completas uma por uma. Cada fase tem um critério de saída verificável.

### Fase 0 — Auditoria e decisões

Analisar o ambiente (Bun, Rust, Tauri, Ollama, versões, hardware). Tomar e registrar as decisões da seção 5. Definir a estrutura inicial e os riscos. Nenhum código de produto.

**Saída:** documento de decisões + relatório de ambiente + estrutura proposta.

### Fase 1 — Esqueleto

Tauri 2 + Next.js (static export) + TypeScript + Tailwind. Crate `agent-core`. `src-tauri` como adaptador com um command IPC (`app_info`). CLI `caua-ai` usando o core. Biome, rustfmt e clippy configurados.

**Saída:** `next build`, typecheck, Biome, `cargo test`, `cargo clippy` e `tauri build` (binário) passando; o app abre e mostra a versão do core via IPC; `caua-ai --version` roda. Empacotamento `.deb`/AppImage é validado no Linux (decisão 0001).

### Fase 2 — Workspace e segurança de path

Seleção de pasta, estado do workspace, file tree, bridge de filesystem, validação de paths com testes.

**Saída:** todos os testes de path/symlink/traversal passando.

### Fase 3 — Ollama

Conexão, health check, listagem, metadados, modelos carregados, streaming, cancelamento, structured output, contexto explícito.

**Saída:** chat em streaming cancelável na UI e na CLI com o modelo escolhido.

### Fase 4 — Tools mínimas

`read_file`, `search`, `edit_file`, `write_file`, `list_directory`, `run_command` (todo comando exige aprovação nesta fase). Eventos estruturados. Secret Redactor básico.

**Saída:** testes das tools e do formato de edição passando.

### Fase 5 — Loop mínimo de ponta a ponta

Task → contexto básico (perfil + ripgrep) → modelo → tools → resultado → modelo, com limites, timeout, cancelamento, detecção de loop, estado persistente. Na UI e na CLI.

**Saída:** o agente resolve uma tarefa simples real num repo de teste. A partir daqui, usar o Cauã AI em tarefas pequenas do próprio projeto (dogfooding).

### Fase 6 — Eval baseline

Suíte `evals/` com as primeiras tarefas e runner na CLI.

**Saída:** número baseline de taxa de sucesso registrado.

### Fase 7 — Permissões e sandbox

Classificação de comandos, sandbox do shell, rede bloqueada, modos ASK/AUTO/FULL ACCESS, marcação de conteúdo não confiável.

**Saída:** testes de permissão e sandbox passando; eval sem regressão.

### Fase 8 — Verifier e ciclo de correção

Detecção de comandos de validação, verificação determinística, review por LLM opcional, ciclo de correção com limite, `completed_unvalidated`.

**Saída:** eval com melhora mensurável na taxa de sucesso.

### Fase 9 — Checkpoints, Git e histórico

Shadow repo, rollback seguro, tools de git, histórico por workspace.

**Saída:** testes de rollback com mudanças do usuário passando.

### Fase 10 — Context Manager completo

Repo map (tree-sitter), orçamento por seção, cache com invalidação, higiene de contexto, Explorer como role.

**Saída:** eval sem regressão e com menos tokens por tarefa.

### Fase 11 — Skills

Registry, verificação de licenças, versões condensadas, Skill Router determinístico, dependências e conflitos.

**Saída:** eval por skill mostrando ganho ou neutralidade.

### Fase 12 — Model Router, memória e trajetórias

Roteamento FAST/CODER/REASONER sensível ao modelo carregado e à memória, escalonamento após falhas, memória estruturada, trajetórias opt-in.

**Saída:** eval sem regressão; latência média menor em tarefas triviais.

### Fase 13 — Otimização

Medir primeiro, depois otimizar: contexto, cache, tool calls, paralelismo de leituras, roteamento, filesystem, UI. Não otimizar por especulação.

**Saída:** ganhos comprovados por métrica e eval.

### Fase 14 — Release

Linux x86_64: `.deb` e AppImage. Validar instalação limpa numa máquina/VM sem ambiente de desenvolvimento. Depois: Flatpak e RPM; Windows `.msi`/`.exe`; macOS `.dmg`.

**Saída:** app instalado do zero resolve uma tarefa do eval.

---

## 35. Regras de engenharia

### Deletion first

Antes de adicionar código: já existe? pode reutilizar? pode simplificar? pode remover algo? Só então criar.

### KISS / YAGNI

Arquitetura simples e correta em vez de abstração future-proof. Preservar só as boundaries da seção 7.

### Clean code

Nomes claros, responsabilidades definidas, sem duplicação, sem abstração prematura, sem comentários óbvios, sem funções gigantes, sem estado global desnecessário, sem `any`, tipagem forte, sem código morto. Não refatorar o projeto inteiro por preferência.

### Git workflow do desenvolvimento

`git status` antes, mudanças pequenas e coerentes, `git diff` e `git status` depois. Nunca sobrescrever mudanças do usuário.

### Regra de ouro

```text
Isso é necessário para o coding agent funcionar?
  não → não implemente
  sim → já existe?
          sim → reutilize
          não → implemente a solução mais simples correta
```

---

## 36. Primeira tarefa

Comece pela **Fase 0**. Não implemente o produto.

1. Auditar o ambiente e as ferramentas disponíveis.
2. Confirmar versões estáveis atuais.
3. Medir o hardware (CPU, RAM, GPU, VRAM).
4. Confirmar o Ollama e os modelos locais; estimar qual modelo coder cabe com contexto útil.
5. Tomar e registrar as decisões da seção 5 (plataforma, hardware/modelo, local do loop, porta cloud), consultando o usuário quando necessário.
6. Propor a estrutura inicial do repositório.
7. Identificar riscos.
8. Apresentar a arquitetura mínima e o plano das fases 1–5.
9. Só então iniciar a Fase 1.

Durante toda a implementação: executar comandos reais, validar resultados, corrigir erros, não assumir que algo funcionou, manter o diff limpo, não sair do escopo.

O objetivo final é um coding agent local realmente utilizável, não uma demonstração de IA.

```text
UNDERSTAND → PLAN → IMPLEMENT → EXECUTE → TEST → REVIEW → FIX → VALIDATE
```
