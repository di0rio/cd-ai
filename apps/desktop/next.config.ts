import type { NextConfig } from "next";

// Tauri serves static files only: no SSR, API routes, Server Actions or middleware (docs/decisions/0005).
const nextConfig: NextConfig = {
  output: "export",
  images: { unoptimized: true },
};

export default nextConfig;
