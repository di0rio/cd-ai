"use client";

import { memo, type ReactNode, useState } from "react";
import { Icon } from "./icons";

interface CodeBlockProps {
  language: string;
  code: string;
}

function CodeBlock({ language, code }: CodeBlockProps) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // ignore
    }
  };

  return (
    <div className="relative rounded-lg bg-sidebar border border-line overflow-hidden my-2">
      <div className="flex items-center gap-2 px-3 py-1.5 border-b border-line bg-sidebar/50">
        <span className="text-xs text-ink-faint font-mono">{language || "texto"}</span>
        <span className="flex-1" />
        <button
          type="button"
          onClick={handleCopy}
          className="p-1 rounded text-ink-faint hover:text-ink hover:bg-canvas transition-colors"
          aria-label={copied ? "Copiado!" : "Copiar código"}
        >
          {copied ? <Icon name="check" className="size-4 text-ok" /> : <Icon name="copy" className="size-4" />}
        </button>
      </div>
      <pre className="p-3 overflow-x-auto">
        <code className="text-[0.8125rem] leading-relaxed text-ink font-mono">{code}</code>
      </pre>
    </div>
  );
}

function Paragraph({ children }: { children: ReactNode }) {
  return <p className="text-[0.9375rem] leading-relaxed text-pretty">{children}</p>;
}

function Heading({ level, children }: { level: number; children: ReactNode }) {
  const styles: Record<number, string> = {
    1: "text-xl font-semibold",
    2: "text-lg font-semibold",
    3: "text-base font-semibold",
  };
  return <h1 className={`${styles[level] || styles[3]} mt-4 mb-2 text-ink`}>{children}</h1>;
}

function List({ items, ordered }: { items: string[]; ordered: boolean }) {
  return (
    <ul className={`${ordered ? "list-decimal" : "list-disc"} pl-6 space-y-1 my-2`}>
      {items.map((item) => (
        <li key={item} className="text-[0.9375rem] leading-relaxed">
          {parseInline(item)}
        </li>
      ))}
    </ul>
  );
}

function Blockquote({ children }: { children: ReactNode }) {
  return <blockquote className="border-l-2 border-signal pl-4 italic text-ink-muted my-2">{children}</blockquote>;
}

function HorizontalRule() {
  return <hr className="border-line my-4" />;
}

function parseInline(text: string): ReactNode {
  const parts = text.split("`");
  return parts.map((part, index) => {
    const key = `inline-${index}-${part.length}`;
    return index % 2 === 1 ? (
      <code key={key} className="box-decoration-clone rounded-md bg-sidebar px-1 py-0.5 text-[0.85em] font-mono">
        {part}
      </code>
    ) : (
      part
    );
  });
}

export const Markdown = memo(function Markdown({ content }: { content: string }) {
  const lines = content.split("\n");
  const blocks: ReactNode[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    // Code block
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

    // Headings
    const headingMatch = line.match(/^(#{1,3})\s+(.+)$/);
    if (headingMatch) {
      blocks.push(
        <Heading key={blocks.length} level={headingMatch[1].length}>
          {headingMatch[2]}
        </Heading>,
      );
      i++;
      continue;
    }

    // Blockquote
    if (line.startsWith("> ")) {
      const quoteLines: string[] = [];
      while (i < lines.length && lines[i].startsWith("> ")) {
        quoteLines.push(lines[i].slice(2));
        i++;
      }
      blocks.push(<Blockquote key={blocks.length}>{parseInline(quoteLines.join("\n"))}</Blockquote>);
      continue;
    }

    // Horizontal rule
    if (line.match(/^[-*_]{3,}$/)) {
      blocks.push(<HorizontalRule key={blocks.length} />);
      i++;
      continue;
    }

    // Lists
    const listMatch = line.match(/^(\s*)([-*+]|\d+\.)\s+(.+)$/);
    if (listMatch) {
      const ordered = /^\d+\./.test(listMatch[2]);
      const items: string[] = [];
      const indent = listMatch[1].length;
      while (i < lines.length) {
        const currentLine = lines[i];
        const currentMatch = currentLine.match(/^(\s*)([-*+]|\d+\.)\s+(.+)$/);
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

    // Empty line - skip
    if (line.trim() === "") {
      i++;
      continue;
    }

    // Paragraph - collect consecutive non-empty lines
    const paragraphLines: string[] = [];
    while (i < lines.length) {
      const currentLine = lines[i];
      if (
        currentLine.trim() === "" ||
        currentLine.startsWith("```") ||
        currentLine.match(/^#{1,3}\s+/) ||
        currentLine.startsWith("> ") ||
        currentLine.match(/^[-*_]{3,}$/) ||
        currentLine.match(/^(\s*)([-*+]|\d+\.)\s+/)
      ) {
        break;
      }
      paragraphLines.push(currentLine);
      i++;
    }
    if (paragraphLines.length > 0) {
      blocks.push(<Paragraph key={blocks.length}>{parseInline(paragraphLines.join(" "))}</Paragraph>);
    }
  }

  return <div className="prose max-w-none">{blocks}</div>;
});
