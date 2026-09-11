"use client";

import { useEffect, useState } from "react";
import { getOllamaStatus, type OllamaStatus } from "@/lib/ipc";

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
  let title: string | undefined;

  if (status.state === "unavailable") {
    dot = "bg-ink-faint";
    label = "Ollama · indisponível fora do Tauri";
  } else if (status.state === "ready" && !status.status.reachable) {
    dot = "bg-bad";
    label = "Ollama offline";
    title = status.status.error ?? undefined;
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

  return (
    <output aria-live="polite" title={title}>
      <p className="flex items-center gap-2">
        <span className={`size-1.5 rounded-full ${dot}`} />
        {label}
      </p>
    </output>
  );
}
