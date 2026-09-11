import type { Metadata } from "next";
import type { ReactNode } from "react";
import "./globals.css";

export const metadata: Metadata = {
  title: "cd-ai",
};

const DIRECTION_CONTRACT = `<!--
THESIS: conversation-first coding agent in the grammar of the Claude Code desktop app (Code tab); refuses the IDE layout of fixed file tree, activity and terminal panes.
OWN-WORLD: two cool-neutral layers (canvas, sidebar), one teal accent for primary action, selection and live state; system sans, mono only for code, paths and commands; 1px lines, 8-16px radii, translucent top bar.
STORY: pick a task, read what the agent did in plain language; routine exploration folds into one line, edits, commands and errors stay visible; every task ends on an evidence verdict.
FIRST VIEWPORT: 16rem task sidebar left; conversation column max 46rem under a translucent 48px top bar; status line (phase, context) above the composer at the bottom; diff and terminal open on demand at the right.
FORM: pinned by the user (Claude Code desktop canon, no roll), improved with grouped activity and always-visible state.
FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, and DESIGN.md
-->`;

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="pt-BR">
      <body>
        {/* biome-ignore lint/security/noDangerouslySetInnerHtml: static design contract comment, no user input */}
        <div hidden dangerouslySetInnerHTML={{ __html: DIRECTION_CONTRACT }} />
        {children}
      </body>
    </html>
  );
}
