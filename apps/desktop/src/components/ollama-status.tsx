"use client";

import { useEffect, useState } from "react";
import { getOllamaStatus, type OllamaStatus } from "@/lib/ipc";
import { Tooltip, TooltipContent, TooltipTrigger } from "./ui/tooltip";

type Status = { state: "loading" } | { state: "ready"; status: OllamaStatus } | { state: "unavailable" };

export function OllamaIndicator() {
  const [status, setStatus] = useState<Status>({ state: "loading" });

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setInterval>;

    const fetchStatus = () =>
      getOllamaStatus().then(
        (result) => {
          if (!cancelled) setStatus({ state: "ready", status: result });
        },
        () => {
          if (!cancelled) setStatus({ state: "unavailable" });
        },
      );

    fetchStatus();
    timer = setInterval(fetchStatus, 15_000);
    window.addEventListener("focus", fetchStatus);
    return () => {
      cancelled = true;
      clearInterval(timer);
      window.removeEventListener("focus", fetchStatus);
    };
  }, []);

  let dot: string;
  let label: string;
  // Only the offline case has one: the error the core reported.
  let detail: string | undefined;

  if (status.state === "unavailable") {
    dot = "bg-ink-faint";
    label = "Ollama · indisponível fora do Tauri";
  } else if (status.state === "ready" && !status.status.reachable) {
    dot = "bg-bad";
    label = "Ollama offline";
    detail = status.status.error ?? undefined;
  } else if (status.state === "ready" && status.status.loaded.length > 0) {
    dot = "bg-ok";
    label = `${status.status.loaded[0].name} carregado`;
  } else if (status.state === "ready") {
    dot = "bg-ok";
    label = `Ollama ${status.status.version ?? "?"} · ${status.status.models.length} modelos`;
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
      {/* The tooltip only opens on hover, so the error still has to reach the live region. */}
      {detail && <span className="sr-only">{detail}</span>}
    </output>
  );
}
