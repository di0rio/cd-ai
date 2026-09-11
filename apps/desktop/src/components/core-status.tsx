"use client";

import { useEffect, useState } from "react";
import { type AppInfo, getAppInfo } from "@/lib/ipc";

type Status = { state: "loading" } | { state: "ready"; info: AppInfo } | { state: "unavailable" };

export function CoreStatus() {
  const [status, setStatus] = useState<Status>({ state: "loading" });

  useEffect(() => {
    getAppInfo().then(
      (info) => setStatus({ state: "ready", info }),
      () => setStatus({ state: "unavailable" }),
    );
  }, []);

  const label = {
    loading: "Conectando ao core…",
    ready: status.state === "ready" ? `core v${status.info.version}` : "",
    unavailable: "Core indisponível (fora do Tauri)",
  }[status.state];

  return (
    <output aria-live="polite" className="block">
      {label}
    </output>
  );
}
