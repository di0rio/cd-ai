"use client";

import { memo, type ReactNode, useEffect, useState } from "react";
import { Icon } from "./icons";

function CodeBlock({ language, code }: { language: string; code: string }) {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 2_000);
    return () => clearTimeout(timer);
  }, [copied]);

  return (
    <div className="relative my-2 overflow-hidden rounded-lg border border-line bg-sidebar">
      <div className="flex items-center gap-2 border-b border-line bg-sidebar/50 px-3 py-1.5">
        <span className="font-mono text-xs text-ink-faint">{language || "texto"}</span>
        <span className="flex-1" />
        <button
          type="button"
          onClick={() => {
            navigator.clipboard.writeText(code).then(
              () => setCopied(true),
              () => {},
            );
          }}
          className="rounded p-1 text-ink-faint transition-colors hover:bg-canvas hover:text-ink"
          aria-label={copied ? "Copiado!" : "Copiar código"}
        >
          {copied ? <Icon name="check" className="size-4 text-ok" /> : <Icon name="copy" className="size-4" />}
        </button>
      </div>
      <pre className="overflow-x-auto p-3">
        <code className="font-mono text-[0.8125rem] leading-relaxed text-ink">{code}</code>
      </pre>
    </div>
  );
}

function Heading({ level, children }: { level: 1 | 2 | 3; children: ReactNode }) {
  const Tag = `h${level}` as const;
  const size = level === 1 ? "text-xl" : level === 2 ? "text-lg" : "text-base";
  return <Tag className={`${size} mt-4 mb-2 font-semibold text-ink`}>{children}</Tag>;
}

function List({ items, ordered }: { items: string[]; ordered: boolean }) {
  const Tag = ordered ? "ol" : "ul";
  return (
    <Tag className={`my-2 space-y-1 pl-6 ${ordered ? "list-decimal" : "list-disc"}`}>
      {items.map((item, index) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: list items can repeat
        <li key={index} className="text-[0.9375rem] leading-relaxed">
          {parseInline(item)}
        </li>
      ))}
    </Tag>
  );
}

function parseInline(text: string): ReactNode {
  const parts = text.split("`");
  return parts.map((part, index) => {
    const key = `inline-${index}-${part.length}`;
    return index % 2 === 1 ? (
      <code key={key} className="box-decoration-clone rounded-md bg-sidebar px-1 py-0.5 font-mono text-[0.85em]">
        {part}
      </code>
    ) : (
      part
    );
  });
}

const BLOCK_START = /^(```|#{1,3}\s+|> |[-*_]{3,}$|\s*([-*+]|\d+\.)\s+)/;

export const Markdown = memo(function Markdown({ content }: { content: string }) {
  const lines = content.split("\n");
  const blocks: ReactNode[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    if (line.startsWith("```")) {
      const language = line.slice(3).trim();
      const codeLines: string[] = [];
      i++;
      while (i < lines.length && !lines[i].startsWith("```")) {
        codeLines.push(lines[i]);
        i++;
      }
      blocks.push(<CodeBlock key={blocks.length} language={language} code={codeLines.join("\n")} />);
      i++;
      continue;
    }

    const headingMatch = line.match(/^(#{1,3})\s+(.+)$/);
    if (headingMatch) {
      const level = headingMatch[1].length as 1 | 2 | 3;
      blocks.push(
        <Heading key={blocks.length} level={level}>
          {headingMatch[2]}
        </Heading>,
      );
      i++;
      continue;
    }

    if (line.startsWith("> ")) {
      const quoteLines: string[] = [];
      while (i < lines.length && lines[i].startsWith("> ")) {
        quoteLines.push(lines[i].slice(2));
        i++;
      }
      blocks.push(
        <blockquote key={blocks.length} className="my-2 border-l-2 border-signal pl-4 text-ink-muted italic">
          {parseInline(quoteLines.join("\n"))}
        </blockquote>,
      );
      continue;
    }

    if (/^[-*_]{3,}$/.test(line)) {
      blocks.push(<hr key={blocks.length} className="my-4 border-line" />);
      i++;
      continue;
    }

    const listMatch = line.match(/^(\s*)([-*+]|\d+\.)\s+(.+)$/);
    if (listMatch) {
      const ordered = /^\d+\./.test(listMatch[2]);
      const items: string[] = [];
      const indent = listMatch[1].length;
      while (i < lines.length) {
        const currentMatch = lines[i].match(/^(\s*)([-*+]|\d+\.)\s+(.+)$/);
        if (currentMatch && currentMatch[1].length === indent) {
          items.push(currentMatch[3]);
          i++;
        } else {
          break;
        }
      }
      blocks.push(<List key={blocks.length} items={items} ordered={ordered} />);
      continue;
    }

    if (line.trim() === "") {
      i++;
      continue;
    }

    const paragraphLines: string[] = [];
    while (i < lines.length && lines[i].trim() !== "" && !BLOCK_START.test(lines[i])) {
      paragraphLines.push(lines[i]);
      i++;
    }
    if (paragraphLines.length > 0) {
      blocks.push(
        <p key={blocks.length} className="text-[0.9375rem] leading-relaxed text-pretty">
          {parseInline(paragraphLines.join(" "))}
        </p>,
      );
    }
  }

  return <div className="max-w-none">{blocks}</div>;
});
