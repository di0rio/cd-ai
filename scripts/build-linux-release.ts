// Phase 14: reproducible Linux x86_64 .deb + AppImage + CLI.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const dist = join(root, "dist/linux");

function run(command: string, args: string[]): void {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit" });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function sha256(path: string): string {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function firstExisting(paths: string[]): string | undefined {
  return paths.find((path) => {
    try {
      return statSync(path).isFile();
    } catch {
      return false;
    }
  });
}

function findBundles(): string[] {
  const candidates = [join(root, "target/release/bundle"), join(root, "src-tauri/target/release/bundle")];
  const found: string[] = [];
  for (const base of candidates) {
    for (const kind of ["deb", "appimage"]) {
      const dir = join(base, kind);
      try {
        for (const name of readdirSync(dir)) {
          if (name.endsWith(".deb") || name.endsWith(".AppImage")) {
            found.push(join(dir, name));
          }
        }
      } catch {
        // directory missing on this layout
      }
    }
  }
  return found;
}

if (process.platform !== "linux" || process.arch !== "x64") {
  console.error("build-linux-release: só Linux x86_64 (SPEC §34 Fase 14)");
  process.exit(1);
}

run("bun", [join(root, "scripts/check-release-metadata.ts")]);
run("cargo", ["build", "--release", "-p", "cd-ai-cli"]);
run("bun", ["tauri", "build"]);

mkdirSync(dist, { recursive: true });

const cli = firstExisting([join(root, "target/release/cd-ai"), join(root, "src-tauri/target/release/cd-ai")]);
if (!cli) {
  console.error("build-linux-release: binário da CLI não encontrado em target/release/cd-ai");
  process.exit(1);
}
copyFileSync(cli, join(dist, "cd-ai"));

const bundles = findBundles();
if (bundles.length === 0) {
  console.error("build-linux-release: nenhum .deb/.AppImage em target/release/bundle");
  process.exit(1);
}

const copied: string[] = [join(dist, "cd-ai")];
for (const file of bundles) {
  const dest = join(dist, basename(file));
  copyFileSync(file, dest);
  copied.push(dest);
}

const lines = copied.map((path) => {
  const bytes = statSync(path).size;
  return `${sha256(path)}  ${bytes}  ${basename(path)}`;
});
writeFileSync(join(dist, "SHA256SUMS"), `${lines.join("\n")}\n`);

console.log("\nArtefatos em dist/linux/:");
for (const line of lines) console.log(line);
console.log("\nInstalação (.deb):\n  sudo apt install ./dist/linux/*.deb");
console.log("AppImage:\n  chmod +x dist/linux/*.AppImage && ./dist/linux/*.AppImage");
console.log(
  "Eval com a CLI instalada:\n  cd-ai eval --scripted --suite /usr/share/cd-ai/evals --out /tmp/cd-ai-eval.json",
);
