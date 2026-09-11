# Plano 001: um comando verifica o repositório inteiro, e agentes têm um guia na raiz

> **Instruções ao executor:** siga o plano passo a passo. Rode cada comando de verificação e confirme o resultado esperado antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Ao terminar, atualize a linha deste plano em `plans/README.md`.
>
> **Drift check (rode primeiro):** compare os trechos de "Estado atual" com os arquivos vivos (`package.json`, `apps/desktop/package.json`). Se não baterem, trate como STOP condition.

## Status

- **Prioridade:** P1
- **Esforço:** S
- **Risco:** LOW
- **Depende de:** nenhum
- **Categoria:** dx
- **Planejado em:** base `158f609` + árvore de trabalho de 2026-09-11

## Por que isso importa

Hoje a verificação está espalhada:

- `bun run check` roda só o Biome.
- O typecheck e os testes do frontend ficam em `apps/desktop`.
- Os checks Rust (fmt, clippy, test) não têm script.

Todos os próximos planos precisam de um gate único para provar que não quebraram nada. Também não existe um guia na raiz para agentes: eles não conhecem as regras do projeto (fronteira de confiança no Rust, frontend sem rede, dados de demonstração só em desenvolvimento) nem os comandos.

## Estado atual

- `package.json` (raiz), linhas 7–13:

  ```json
  "scripts": {
    "tauri": "tauri",
    "dev": "tauri dev",
    "build": "tauri build",
    "check": "biome check .",
    "format": "biome format --write ."
  },
  ```

- `apps/desktop/package.json`, bloco de scripts:

  ```json
  "dev": "next dev -p 1420",
  "build": "next build",
  "typecheck": "tsc --noEmit",
  "test": "bun test"
  ```

- `Cargo.toml` (raiz) é um Cargo workspace com os membros `crates/agent-core`, `apps/cli` e `src-tauri`.
- Os arquivos `apps/desktop/AGENTS.md` e `apps/desktop/CLAUDE.md` são **gerados pelo `next dev`** (recurso `agentRules` do Next 16.3). Não são o guia do projeto e não devem ser editados.
- Documentos que o guia deve apontar:
  - `SPEC.md`
  - `docs/decisions/0001` a `0007`
  - `apps/desktop/PRODUCT.md`
  - `apps/desktop/DESIGN.md`

## Comandos que você vai precisar

| Propósito | Comando | Esperado |
|-----------|---------|----------|
| Biome | `bun run check` | exit 0 |
| Typecheck | `bun run --cwd apps/desktop typecheck` | exit 0 |
| Testes do frontend | `bun run --cwd apps/desktop test` | `0 fail` |
| Formatação Rust | `cargo fmt --all --check` | exit 0 |
| Clippy | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| Testes Rust | `cargo test --workspace` | todas as linhas `test result: ok` |

No Windows, se o PowerShell não encontrar o `cargo`, rode antes: `$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"`.

## Escopo

**Dentro do escopo** (os únicos arquivos que você deve modificar ou criar):

- `package.json` (raiz)
- `AGENTS.md` (raiz, criar)
- `CLAUDE.md` (raiz, criar)

**Fora do escopo:**

- `apps/desktop/AGENTS.md` e `apps/desktop/CLAUDE.md`: são gerenciados pelo Next.
- Qualquer código-fonte.
- `biome.json`.

## Git

- Branch: `advisor/001-verify`.
- Um commit, com mensagem no estilo `chore: add verify script and agent guide`.
- Não faça push.

## Passos

### Passo 1: confirmar que a base passa

Rode cada comando da tabela acima, um por um.

**Verificar:** todos saem com exit 0. Se algum falhar **antes** de você mudar qualquer coisa, isso é STOP.

### Passo 2: adicionar o script `verify`

No `package.json` da raiz, adicione dentro de `"scripts"`, logo depois de `"check"`:

```json
"verify": "bun run check && bun run --cwd apps/desktop typecheck && bun run --cwd apps/desktop test && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace",
```

**Verificar:** `bun run verify` sai com exit 0, e a saída contém `0 fail` e `test result: ok`.

### Passo 3: criar o `AGENTS.md` na raiz

Crie `AGENTS.md` com exatamente este conteúdo:

````markdown
# cd-ai — guia para agentes

Coding agent desktop local: Tauri 2 + Next.js (static export) + core em Rust + Ollama.

- Especificação: `SPEC.md`.
- Decisões de arquitetura: `docs/decisions/` (leia antes de mudar a arquitetura).
- Produto e interface: `apps/desktop/PRODUCT.md` e `apps/desktop/DESIGN.md`.

## Comandos

- Verificar tudo (obrigatório antes de dizer que terminou): `bun run verify`
- App em desenvolvimento: `bun run dev` (Tauri + Next na porta 1420)
- Só a UI no navegador: `bun run --cwd apps/desktop dev`, depois abrir `http://localhost:1420/?demo` para ver os dados de demonstração
- Binário release, sem instalador: `bun tauri build --no-bundle`
- CLI: `cargo run -q -p cd-ai-cli -- --version`
- Benchmark de modelos (com o Ollama rodando): `bun scripts/bench-models.ts <modelo> [modelo...]`

## Estrutura

- `crates/agent-core/`: lógica e segurança, em Rust. Crie módulos novos aqui antes de criar crates novos.
- `src-tauri/`: só o adaptador desktop. Commands do Tauri que chamam o `agent-core`.
- `apps/cli/`: binário `cd-ai`, que usa o mesmo core.
- `apps/desktop/`: UI em Next.js com static export. Só apresentação.
- `scripts/`: ferramentas de desenvolvimento.

## Regras

- A fronteira de confiança é o Rust. Validação de path, permissões, execução de comandos e redação de secrets nunca ficam no frontend.
- O frontend não faz rede e não usa recursos de servidor do Next: nada de SSR, API routes, Server Actions, middleware, fontes ou imagens remotas (decisão 0005).
- Tipos do IPC: o Rust é a fonte da verdade; o TypeScript espelha (`apps/desktop/src/lib/ipc.ts`).
- Os dados de demonstração (`apps/desktop/src/lib/demo-session.ts`) só aparecem em desenvolvimento, com `?demo`. Nunca em produção.
- Código e comentários em inglês; UI e documentação em pt-BR. Comentários só quando explicam o porquê.
- Não adicione dependências sem necessidade clara; prefira APIs nativas.
- O projeto se chama `cd-ai` (decisão 0007). A pasta local pode se chamar `caua-ai`.
- Git: sem push, force push, reset destrutivo ou commits, a menos que o usuário peça.

## Pronto

Uma mudança só está pronta com `bun run verify` passando. Se algo não pôde ser verificado, diga isso explicitamente.
````

**Verificar:** `Test-Path AGENTS.md` retorna `True`.

### Passo 4: criar o `CLAUDE.md` na raiz

Crie `CLAUDE.md` contendo apenas a linha:

```text
@AGENTS.md
```

**Verificar:** `Get-Content CLAUDE.md` imprime `@AGENTS.md`.

### Passo 5: verificação final

**Verificar:** `bun run verify` sai com exit 0.

## Plano de testes

Não há teste novo. O próprio `verify` é o artefato: ele precisa passar na árvore limpa.

## Critérios de pronto

- [ ] `bun run verify` sai com exit 0
- [ ] `AGENTS.md` e `CLAUDE.md` existem na raiz
- [ ] `git status` mostra modificados apenas `package.json`, `AGENTS.md` e `CLAUDE.md`
- [ ] A linha deste plano em `plans/README.md` está atualizada

## STOP conditions

- Algum comando do Passo 1 falha antes da sua mudança (a base está quebrada).
- `bun run` no Windows não aceita `&&` dentro do script. Nesse caso, reporte; não troque por um script `.ps1`.
- Já existe `AGENTS.md` ou `CLAUDE.md` na raiz.

## Notas de manutenção

- Todo check novo (por exemplo, testes de E2E) entra no `verify`, não num script paralelo.
- Se o `verify` ficar lento (acima de uns 2 minutos), separe um `verify:quick` sem o clippy, mas mantenha o `verify` completo como gate.
