# Plano 007: nenhum `style=` inline no HTML exportado, compatível com a CSP

> **Instruções ao executor:** siga o plano passo a passo. Rode cada comando de verificação e confirme o resultado esperado antes de avançar. Se alguma STOP condition acontecer, pare e reporte; não improvise. Ao terminar, atualize a linha deste plano em `plans/README.md`.
>
> **Drift check (rode primeiro):** confirme que a linha abaixo existe em `apps/desktop/src/components/sidebar.tsx`. Se não existir, STOP.

## Status

- **Prioridade:** P3
- **Esforço:** S
- **Risco:** LOW
- **Depende de:** `plans/001-verify-e-guia-de-agentes.md`
- **Categoria:** bug
- **Planejado em:** base `158f609` + árvore de trabalho de 2026-09-11

## Por que isso importa

O `tauri.conf.json` define a CSP:

```text
default-src 'self'; connect-src ipc: http://ipc.localhost; style-src 'self' 'unsafe-inline'; img-src 'self' data:
```

O Tauri injeta hashes e nonces nas diretivas de script e de estilo durante o build. Quando `style-src` recebe um nonce ou hash, o navegador **ignora** o `'unsafe-inline'`, e atributos `style="..."` no HTML estático passam a ser bloqueados.

O `next/image` gera `style="color:transparent"` no HTML exportado. O efeito visual é nulo, mas gera violação de CSP e força a manter um `'unsafe-inline'` que não deveria ser necessário. Como o app usa static export com imagens não otimizadas, o `next/image` não traz nenhum benefício aqui.

## Estado atual

- `apps/desktop/src/components/sidebar.tsx`, linhas 1 e 37:

  ```tsx
  import Image from "next/image";
  ...
  <Image src="/icon.svg" alt="" width={20} height={20} className="size-5 rounded-[5px]" />
  ```

- `apps/desktop/out/index.html` (depois de `bun run --cwd apps/desktop build`) contém `style="color:transparent"`.
- Na mesma árvore, o `<img>` foi trocado por `<Image>` para calar a regra `lint/performance/noImgElement` do Biome. Essa regra não se aplica a static export sem otimização de imagem.
- Padrão de supressão do Biome no projeto, com justificativa (em `apps/desktop/src/components/conversation.tsx`):

  ```tsx
  // biome-ignore lint/suspicious/noArrayIndexKey: the activity log is append-only, so positions are stable
  ```

## Comandos que você vai precisar

| Propósito | Comando | Esperado |
|-----------|---------|----------|
| Build do front | `bun run --cwd apps/desktop build` | exit 0 |
| Buscar estilo inline | `Select-String -Path apps/desktop/out/*.html -Pattern 'style="'` | nenhuma saída |
| Gate | `bun run verify` | exit 0 |

## Escopo

**Dentro do escopo:**

- `apps/desktop/src/components/sidebar.tsx`

**Fora do escopo:**

- `src-tauri/tauri.conf.json`. Remover o `'unsafe-inline'` da CSP é uma decisão separada: exige confirmar que nenhum `style` via React é renderizado no HTML estático.
- Qualquer outro componente.

## Git

- Branch: `advisor/007-no-inline-style`.
- Mensagem no estilo `fix(ui): avoid inline style from next/image`.
- Não faça push.

## Passos

### Passo 1: trocar por `<img>`

1. Remova `import Image from "next/image";`.
2. Troque a linha 37 por:

   ```tsx
   {/* biome-ignore lint/performance/noImgElement: static export with unoptimized images; next/image only adds an inline style the CSP blocks */}
   <img src="/icon.svg" alt="" width={20} height={20} className="size-5 rounded-[5px]" />
   ```

**Verificar:** `bun run check` sai com exit 0.

### Passo 2: confirmar o HTML

Rode `bun run --cwd apps/desktop build` e depois `Select-String -Path apps/desktop/out/*.html -Pattern 'style="'`.

**Verificar:** a busca não imprime nada. Se imprimir, veja a STOP condition.

### Passo 3: gate

**Verificar:** `bun run verify` sai com exit 0.

## Plano de testes

Nenhum teste novo. A verificação é a busca no HTML exportado (Passo 2).

## Critérios de pronto

- [ ] `Select-String -Path apps/desktop/out/*.html -Pattern 'style="'` não retorna nada
- [ ] `bun run verify` sai com exit 0
- [ ] `git status` mostra modificado apenas `apps/desktop/src/components/sidebar.tsx`
- [ ] A linha deste plano em `plans/README.md` está atualizada

## STOP conditions

- Ainda aparece `style="` no HTML depois da troca, vindo de outro componente. Reporte o trecho e não mexa em outros arquivos.
- O Biome não aceita o comentário de supressão nessa posição. Reporte a mensagem exata.

## Notas de manutenção

- Não use a prop `style` em componentes renderizados no build estático. Para valores dinâmicos, prefira classes do Tailwind, elementos nativos (como `<meter>`, usado no `composer.tsx`) ou atribuições via CSSOM em event handlers. Estas últimas não são bloqueadas pela CSP.
