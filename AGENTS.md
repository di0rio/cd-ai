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
- Tarefa pela CLI (com o Ollama rodando): `cargo run -q -p cd-ai-cli -- task --model <modelo> [--workspace <pasta>] "<pedido>"` (as aprovações são pedidas no terminal; retome com `--resume <id>`)
- Eval da suíte (headless; `--scripted` não precisa do Ollama): `cargo run -q -p cd-ai-cli -- eval --scripted` ou `cargo run -q -p cd-ai-cli -- eval --model <modelo>`
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