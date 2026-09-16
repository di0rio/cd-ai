"use client";

import { useEffect, useState } from "react";
import { getOllamaStatus, type OllamaStatus } from "@/lib/ipc";
import { Tooltip, TooltipContent, TooltipTrigger } from "./ui/tooltip";

export type OllamaView = { state: "loading" } | { state: "ready"; status: OllamaStatus } | { state: "unavailable" };

export function useOllamaStatus(): OllamaView {
  const [view, setView] = useState<OllamaView>({ state: "loading" });

  useEffect(() => {
    let cancelled = false;
    const load = () =>
      getOllamaStatus().then(
        (status) => {
          if (!cancelled) setView({ state: "ready", status });
        },
        () => {
          if (!cancelled) setView({ state: "unavailable" });
        },
      );
    load();
    const timer = setInterval(load, 15_000);
    window.addEventListener("focus", load);
    return () => {
      cancelled = true;
      clearInterval(timer);
      window.removeEventListener("focus", load);
    };
  }, []);

  return view;
}

export function ollamaModels(view: OllamaView): { models: string[]; loaded: string[] } {
  if (view.state !== "ready") return { models: [], loaded: [] };
  return {
    models: view.status.models.map((model) => model.name),
    loaded: view.status.loaded.map((model) => model.name),
  };
}

export function OllamaIndicator({ view }: { view: OllamaView }) {
  let dot: string;
  let label: string;
  let detail: string | undefined;

  if (view.state === "unavailable") {
    dot = "bg-ink-faint";
    label = "Ollama · indisponível fora do Tauri";
  } else if (view.state === "ready" && !view.status.reachable) {
    dot = "bg-bad";
    label = "Ollama offline";
    detail = view.status.error ?? undefined;
  } else if (view.state === "ready" && view.status.loaded.length > 0) {
    dot = "bg-ok";
    label = `${view.status.loaded[0].name} carregado`;
  } else if (view.state === "ready") {
    dot = "bg-ok";
    label = `Ollama ${view.status.version ?? "?"} · ${view.status.models.length} modelos`;
  } else {
    dot = "bg-ink-faint";
    label = "Ollama · verificando…";
  }

  const line = (
    <p className="flex items-center gap-2">
      <span className={`size-1.5 rounded-full ${dot}`} />
      {label}
    </p>
  );

  return (
    <output aria-live="polite">
      {detail ? (
        <Tooltip>
          <TooltipTrigger asChild>{line}</TooltipTrigger>
          <TooltipContent>{detail}</TooltipContent>
        </Tooltip>
      ) : (
        line
      )}
      {detail && <span className="sr-only">{detail}</span>}
    </output>
  );
}
