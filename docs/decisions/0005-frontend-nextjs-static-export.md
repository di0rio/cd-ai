# 0005 — Frontend em Next.js com static export

Status: **aceita** (validar na Fase 1).

## Decisão

O frontend desktop usa **Next.js + React + TypeScript + Tailwind CSS**, com componentes **Base UI / coss**, gerenciado com **Bun**.

O Tauri não fornece runtime server-side para o Next. Por isso o Next roda em **static export**:

```text
next build (output: "export") → out/ → Tauri (frontendDist) → binário desktop
```

## Proibido no frontend

- SSR e rotas renderizadas no servidor;
- API routes / route handlers;
- Server Actions;
- middleware;
- otimização de imagens do servidor do Next (usar imagens não otimizadas);
- qualquer dependência de runtime Node em produção.

O backend local é o Rust (decisão 0003). O frontend só apresenta dados e envia comandos via IPC do Tauri.

Rotas dinâmicas por URL devem ser evitadas; a navegação usa estado do cliente ou rotas estáticas.

## Trade-off aceito

A documentação do Tauri recomenda Vite para SPAs. O Next foi escolhido pela familiaridade com o ecossistema. Se o static export gerar atrito real (build lento, HMR instável com o Tauri, recursos do Next inutilizáveis), reabrir esta decisão com evidência.
