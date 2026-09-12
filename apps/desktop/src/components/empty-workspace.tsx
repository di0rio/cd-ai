import type { WorkspaceInfo } from "@/lib/ipc";
import { Icon, type IconName } from "./icons";

const FACTS: { icon: IconName; text: string }[] = [
  { icon: "folder", text: "Lê, edita e executa comandos só dentro do workspace escolhido." },
  { icon: "lock", text: "Roda com os seus modelos do Ollama. Nada sai do computador." },
  { icon: "check", text: "Uma tarefa só termina com evidência: testes, typecheck e diff." },
];

type EmptyWorkspaceProps = {
  workspace: WorkspaceInfo | null;
  error: string | null;
  onOpen: () => void;
};

export function EmptyWorkspace({ workspace, error, onOpen }: EmptyWorkspaceProps) {
  return (
    <div className="flex min-h-0 flex-1 items-center justify-center overflow-y-auto px-6 pt-12">
      <div className="w-full max-w-md">
        <h2 className="text-[1.375rem] leading-tight font-semibold tracking-[-0.02em] text-balance">
          {workspace ? workspace.name : "Abra um workspace para começar"}
        </h2>
        {workspace ? (
          <p className="mt-2 break-all font-mono text-[0.9375rem] text-ink-muted">
            <code className="text-ink-muted">{workspace.root}</code>
          </p>
        ) : (
          <p className="mt-2 text-[0.9375rem] text-pretty text-ink-muted">
            O cd-ai trabalha dentro da pasta que você escolher e não sai dela.
          </p>
        )}
        <ul className="mt-6 space-y-3">
          {FACTS.map((fact) => (
            <li key={fact.text} className="flex gap-3 text-ink-muted">
              <Icon name={fact.icon} className="mt-0.5 size-4 text-ink-faint" />
              <span className="text-pretty">{fact.text}</span>
            </li>
          ))}
        </ul>
        <button
          type="button"
          onClick={onOpen}
          className="mt-8 inline-flex h-9 items-center gap-2 rounded-lg bg-signal px-3.5 font-medium text-signal-ink transition-[scale,opacity] active:scale-[0.97]"
        >
          <Icon name="folder" />
          {workspace ? "Trocar workspace" : "Abrir workspace"}
        </button>
        {error && (
          <p role="alert" className="mt-3 text-sm text-bad">
            {error}
          </p>
        )}
      </div>
    </div>
  );
}
