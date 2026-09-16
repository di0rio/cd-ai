// Phase 14: keep Cargo, Tauri and Linux bundle metadata in lockstep.
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

type LinuxFiles = Record<string, string>;
type TauriConf = {
  productName?: string;
  mainBinaryName?: string;
  version?: string;
  identifier?: string;
  bundle?: {
    targets?: string[];
    icon?: string[];
    category?: string;
    linux?: {
      deb?: { files?: LinuxFiles; section?: string; recommends?: string[] };
      appimage?: { files?: LinuxFiles };
    };
  };
};

function fail(message: string): never {
  console.error(`check-release-metadata: ${message}`);
  process.exit(1);
}

function readTauri(): TauriConf {
  const path = join(root, "src-tauri/tauri.conf.json");
  return JSON.parse(readFileSync(path, "utf8")) as TauriConf;
}

function cargoWorkspaceVersion(): string {
  const text = readFileSync(join(root, "Cargo.toml"), "utf8");
  const block = text.match(/\[workspace\.package\]\s+([\s\S]*?)(?:\n\[|$)/);
  const version = block?.[1]?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  if (!version) fail("Cargo.toml workspace.package.version ausente");
  return version;
}

const conf = readTauri();
const cargoVersion = cargoWorkspaceVersion();

if (conf.productName !== "cd-ai") fail(`productName=${conf.productName}`);
if (conf.identifier !== "dev.cdai.desktop") fail(`identifier=${conf.identifier}`);
if (conf.mainBinaryName !== "cd-ai-desktop") {
  fail(`mainBinaryName deve ser cd-ai-desktop (CLI já é cd-ai); veio ${conf.mainBinaryName}`);
}
if (conf.version !== cargoVersion) {
  fail(`versão divergente: tauri.conf.json=${conf.version} Cargo.toml=${cargoVersion}`);
}

const targets = conf.bundle?.targets ?? [];
if (targets.length !== 2 || !targets.includes("deb") || !targets.includes("appimage")) {
  fail(`bundle.targets deve ser ["deb","appimage"]; veio ${JSON.stringify(targets)}`);
}
if (conf.bundle?.category !== "DeveloperTool") fail(`category=${conf.bundle?.category}`);

const requiredIcons = [
  "src-tauri/icons/32x32.png",
  "src-tauri/icons/128x128.png",
  "src-tauri/icons/128x128@2x.png",
  "src-tauri/icons/icon.icns",
  "src-tauri/icons/icon.ico",
  "apps/desktop/public/icon.svg",
];
for (const icon of requiredIcons) {
  if (!existsSync(join(root, icon))) fail(`ícone ausente: ${icon}`);
}

const requiredFiles: LinuxFiles = {
  "/usr/bin/cd-ai": "../target/release/cd-ai",
  "/usr/share/cd-ai/evals/tasks": "../evals/tasks",
  "/usr/share/cd-ai/evals/fixtures": "../evals/fixtures",
};

function assertLinuxFiles(label: string, files: LinuxFiles | undefined) {
  if (!files) fail(`${label}.files ausente`);
  for (const [dest, src] of Object.entries(requiredFiles)) {
    if (files[dest] !== src) fail(`${label} ${dest} deve ser ${src}; veio ${files[dest]}`);
  }
}

assertLinuxFiles("bundle.linux.deb", conf.bundle?.linux?.deb?.files);
assertLinuxFiles("bundle.linux.appimage", conf.bundle?.linux?.appimage?.files);

if (conf.bundle?.linux?.deb?.section !== "devel") {
  fail(`deb.section=${conf.bundle?.linux?.deb?.section}`);
}
if (!conf.bundle?.linux?.deb?.recommends?.includes("git")) {
  fail("deb.recommends deve incluir git");
}

console.log(`check-release-metadata: ok (cd-ai ${conf.version}, GUI cd-ai-desktop, CLI /usr/bin/cd-ai)`);
