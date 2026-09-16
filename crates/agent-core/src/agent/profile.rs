//! A deterministic look at the workspace, used as the basic context of every task
//! (plan 015, D10). No model, no network, no guessing: marker files at the root only.
//!
//! Every read goes through `Workspace::resolve`, a file the redactor flags by name is never
//! opened, and whatever is read goes through `redactor::redact` before it can reach the model.

use std::fs;

use crate::redactor;
use crate::workspace::Workspace;

/// Project rules kept from `AGENTS.md`/`CLAUDE.md`.
const MAX_RULES_BYTES: usize = 4 * 1024;
/// A manifest larger than this is not a manifest worth parsing.
const MAX_MANIFEST_BYTES: usize = 256 * 1024;
/// The rendered profile goes inside the system prompt, so it stays small.
const MAX_RENDER_BYTES: usize = 3 * 1024;
/// Root names listed for orientation; the model calls `list_directory` for more.
const MAX_ROOT_ENTRIES: usize = 60;
/// Package scripts worth offering as validation commands, in this order.
const SCRIPT_NAMES: [&str; 6] = ["test", "typecheck", "lint", "check", "build", "verify"];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceProfile {
    pub languages: Vec<String>,
    pub package_manager: Option<String>,
    /// Frameworks inferred from manifests and config files (plan 021). Empty when unknown.
    pub frameworks: Vec<String>,
    pub validation_commands: Vec<String>,
    pub rules_excerpt: Option<String>,
    pub root_entries: Vec<String>,
}

/// Builds the profile of an open workspace.
pub fn workspace_profile(workspace: &Workspace) -> WorkspaceProfile {
    let has = |name: &str| has_root_file(workspace, name);

    let mut languages = Vec::new();
    let mut validation_commands = Vec::new();

    if has("Cargo.toml") {
        languages.push("Rust".to_string());
        validation_commands.push("cargo test".to_string());
        validation_commands.push("cargo clippy".to_string());
    }

    let has_package_json = has("package.json");
    if has_package_json {
        languages.push("JS/TS".to_string());
    } else if has("tsconfig.json") {
        // With a package.json the "JS/TS" entry already says it; alone, tsconfig still means TS.
        languages.push("TS".to_string());
    }
    if has("pyproject.toml") || has("requirements.txt") {
        languages.push("Python".to_string());
    }
    if has("go.mod") {
        languages.push("Go".to_string());
    }

    let package_manager = detect_package_manager(workspace);
    if has_package_json {
        // Lockfile is the hard evidence. Without one, a script body that invokes bun is
        // enough to pick bun — `npm run test` on a bun-only fixture is a false failure
        // (plan 018, D4). Otherwise npm is the safe assumption.
        let runner = package_manager.as_deref().unwrap_or_else(|| {
            if scripts_invoke_bun(workspace) {
                "bun"
            } else {
                "npm"
            }
        });
        for script in package_scripts(workspace) {
            validation_commands.push(format!("{runner} run {script}"));
        }
    }

    WorkspaceProfile {
        languages,
        package_manager,
        frameworks: detect_frameworks(workspace),
        validation_commands,
        rules_excerpt: read_root_file(workspace, "AGENTS.md", MAX_RULES_BYTES)
            .or_else(|| read_root_file(workspace, "CLAUDE.md", MAX_RULES_BYTES)),
        root_entries: root_entries(workspace),
    }
}

/// Validation commands as argv, cheapest first (SPEC §13.1 / plan 018, D4).
pub fn validation_argv(workspace: &Workspace) -> Vec<Vec<String>> {
    let mut commands: Vec<Vec<String>> = workspace_profile(workspace)
        .validation_commands
        .iter()
        .filter_map(|line| split_argv(line))
        .collect();
    commands.sort_by_key(|argv| cost_rank(argv));
    commands
}

fn split_argv(line: &str) -> Option<Vec<String>> {
    let parts: Vec<String> = line.split_whitespace().map(str::to_string).collect();
    if parts.is_empty() { None } else { Some(parts) }
}

/// Lower is cheaper: typecheck → lint → tests → check/verify → build (SPEC §13.1).
fn cost_rank(argv: &[String]) -> u8 {
    let joined = argv.join(" ").to_ascii_lowercase();
    if joined.contains("typecheck") || joined.contains("cargo check") {
        0
    } else if joined.contains("lint") || joined.contains("clippy") || joined.contains("fmt") {
        1
    } else if joined.contains("test") {
        2
    } else if joined.contains("build") {
        4
    } else {
        3
    }
}

impl WorkspaceProfile {
    /// Plain text for the system prompt, capped at 3 KiB. In English, like the prompt around it.
    pub fn render(&self) -> String {
        let mut out = String::new();
        if !self.languages.is_empty() {
            out.push_str(&format!("Languages: {}\n", self.languages.join(", ")));
        }
        if !self.frameworks.is_empty() {
            out.push_str(&format!("Frameworks: {}\n", self.frameworks.join(", ")));
        }
        if let Some(manager) = &self.package_manager {
            out.push_str(&format!("Package manager: {manager}\n"));
        }
        if !self.validation_commands.is_empty() {
            out.push_str(&format!(
                "Validation commands: {}\n",
                self.validation_commands.join("; ")
            ));
        }
        if !self.root_entries.is_empty() {
            out.push_str(&format!("Root entries: {}\n", self.root_entries.join(", ")));
        }
        if let Some(rules) = &self.rules_excerpt {
            out.push_str("Project rules:\n");
            out.push_str(rules.trim_end());
            out.push('\n');
        }
        if out.len() > MAX_RENDER_BYTES {
            const NOTE: &str = "\n[profile truncated]\n";
            let keep = truncate_bytes(&out, MAX_RENDER_BYTES - NOTE.len());
            out = format!("{keep}{NOTE}");
        }
        out
    }
}

fn has_root_file(workspace: &Workspace, name: &str) -> bool {
    workspace
        .resolve(name)
        .map(|path| path.is_file())
        .unwrap_or(false)
}

/// Reads a file at the workspace root, or nothing at all if it looks like a secret by name.
fn read_root_file(workspace: &Workspace, name: &str, max_bytes: usize) -> Option<String> {
    let path = workspace.resolve(name).ok()?;
    if redactor::detect_path_secret(&path).is_some() {
        return None;
    }
    if !path.is_file() {
        return None;
    }
    let text = fs::read_to_string(&path).ok()?;
    Some(redactor::redact(truncate_bytes(&text, max_bytes)).text)
}

/// Lockfile at the root, which is the only hard evidence of which manager the project uses.
fn detect_package_manager(workspace: &Workspace) -> Option<String> {
    const LOCKFILES: [(&str, &str); 5] = [
        ("bun.lock", "bun"),
        ("bun.lockb", "bun"),
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("package-lock.json", "npm"),
    ];
    LOCKFILES
        .iter()
        .find(|(file, _)| has_root_file(workspace, file))
        .map(|(_, manager)| (*manager).to_string())
}

/// True when a package.json script body is invoked with bun (plan 018, D4).
fn scripts_invoke_bun(workspace: &Workspace) -> bool {
    let Some(text) = read_root_file(workspace, "package.json", MAX_MANIFEST_BYTES) else {
        return false;
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    let Some(scripts) = manifest.get("scripts").and_then(|value| value.as_object()) else {
        return false;
    };
    scripts
        .values()
        .filter_map(|value| value.as_str())
        .any(|body| body.split_whitespace().next() == Some("bun"))
}

/// Names of the known scripts declared in `package.json`, in the order of `SCRIPT_NAMES`.
/// A manifest that does not parse (truncated, or invalid JSON) simply yields nothing.
fn package_scripts(workspace: &Workspace) -> Vec<String> {
    let Some(text) = read_root_file(workspace, "package.json", MAX_MANIFEST_BYTES) else {
        return Vec::new();
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(scripts) = manifest.get("scripts").and_then(|value| value.as_object()) else {
        return Vec::new();
    };
    SCRIPT_NAMES
        .iter()
        .filter(|name| scripts.contains_key(**name))
        .map(|name| (*name).to_string())
        .collect()
}

/// Frameworks the Skill Router can match (plan 021). Marker files first, then package.json.
fn detect_frameworks(workspace: &Workspace) -> Vec<String> {
    let mut found = Vec::new();
    let mut add = |name: &str| {
        if !found.iter().any(|existing| existing == name) {
            found.push(name.to_string());
        }
    };

    const NEXT_CONFIGS: [&str; 4] = [
        "next.config.ts",
        "next.config.js",
        "next.config.mjs",
        "next.config.cjs",
    ];
    if NEXT_CONFIGS
        .iter()
        .any(|name| has_root_file(workspace, name))
    {
        add("nextjs");
        add("react");
    }

    const TAILWIND_CONFIGS: [&str; 4] = [
        "tailwind.config.ts",
        "tailwind.config.js",
        "tailwind.config.cjs",
        "tailwind.config.mjs",
    ];
    if TAILWIND_CONFIGS
        .iter()
        .any(|name| has_root_file(workspace, name))
    {
        add("tailwind");
    }

    if let Some(text) = read_root_file(workspace, "package.json", MAX_MANIFEST_BYTES)
        && let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&text)
    {
        let mut keys: Vec<&str> = Vec::new();
        for field in ["dependencies", "devDependencies", "peerDependencies"] {
            if let Some(map) = manifest.get(field).and_then(|value| value.as_object()) {
                keys.extend(map.keys().map(|key| key.as_str()));
            }
        }
        if keys
            .iter()
            .any(|key| *key == "react" || *key == "react-dom")
        {
            add("react");
        }
        if keys.contains(&"next") {
            add("nextjs");
            add("react");
        }
        if keys.contains(&"tailwindcss") {
            add("tailwind");
        }
        if let Some(scripts) = manifest.get("scripts").and_then(|value| value.as_object()) {
            let uses_next = scripts
                .values()
                .filter_map(|value| value.as_str())
                .any(|body| {
                    let first = body.split_whitespace().next().unwrap_or("");
                    first == "next" || first.ends_with("/next")
                });
            if uses_next {
                add("nextjs");
                add("react");
            }
        }
    }

    found
}

/// Sorted root names, directories marked with `/` like `list_directory` shows them.
fn root_entries(workspace: &Workspace) -> Vec<String> {
    let Ok(entries) = fs::read_dir(workspace.root()) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => format!("{name}/"),
                _ => name,
            }
        })
        .collect();
    names.sort();
    names.truncate(MAX_ROOT_ENTRIES);
    names
}

/// Cuts at `max_bytes` without splitting a char.
fn truncate_bytes(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::{TempDir, tempdir};

    fn workspace(files: &[(&str, &str)]) -> (TempDir, Workspace) {
        let dir = tempdir().unwrap();
        for (name, content) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, content).unwrap();
        }
        let workspace = Workspace::open(dir.path()).unwrap();
        (dir, workspace)
    }

    #[test]
    fn detects_rust_and_js_with_bun() {
        let (_dir, ws) = workspace(&[
            ("Cargo.toml", "[package]\nname = \"x\"\n"),
            (
                "package.json",
                r#"{ "scripts": { "verify": "bun run check", "test": "bun test", "dev": "next" } }"#,
            ),
            ("bun.lock", "{}"),
        ]);
        let profile = workspace_profile(&ws);
        assert_eq!(profile.languages, vec!["Rust", "JS/TS"]);
        assert_eq!(profile.package_manager.as_deref(), Some("bun"));
        assert_eq!(
            profile.frameworks,
            vec!["nextjs".to_string(), "react".to_string()]
        );
        assert_eq!(
            profile.validation_commands,
            vec![
                "cargo test",
                "cargo clippy",
                "bun run test",
                "bun run verify"
            ]
        );
        // `dev` is not a validation script.
        assert!(
            !profile
                .validation_commands
                .iter()
                .any(|c| c.contains("dev"))
        );
    }

    #[test]
    fn package_manager_comes_from_the_lockfile() {
        for (lockfile, manager) in [
            ("pnpm-lock.yaml", "pnpm"),
            ("yarn.lock", "yarn"),
            ("package-lock.json", "npm"),
            ("bun.lockb", "bun"),
        ] {
            let (_dir, ws) = workspace(&[
                ("package.json", r#"{ "scripts": { "test": "x" } }"#),
                (lockfile, ""),
            ]);
            let profile = workspace_profile(&ws);
            assert_eq!(profile.package_manager.as_deref(), Some(manager));
            assert_eq!(
                profile.validation_commands,
                vec![format!("{manager} run test")]
            );
        }
    }

    #[test]
    fn without_a_lockfile_scripts_fall_back_to_npm() {
        let (_dir, ws) = workspace(&[("package.json", r#"{ "scripts": { "lint": "x" } }"#)]);
        let profile = workspace_profile(&ws);
        assert_eq!(profile.package_manager, None);
        assert_eq!(profile.validation_commands, vec!["npm run lint"]);
    }

    #[test]
    fn without_a_lockfile_a_bun_script_uses_bun() {
        let (_dir, ws) = workspace(&[("package.json", r#"{ "scripts": { "test": "bun test" } }"#)]);
        let profile = workspace_profile(&ws);
        assert_eq!(profile.package_manager, None);
        assert_eq!(profile.validation_commands, vec!["bun run test"]);
        assert_eq!(
            validation_argv(&ws),
            vec![vec![
                "bun".to_string(),
                "run".to_string(),
                "test".to_string()
            ]]
        );
    }

    #[test]
    fn validation_argv_is_cheapest_first() {
        let (_dir, ws) = workspace(&[
            ("Cargo.toml", "[package]\nname = \"x\"\n"),
            (
                "package.json",
                r#"{ "scripts": { "test": "bun test", "typecheck": "tsc", "lint": "biome check ." } }"#,
            ),
            ("bun.lock", "{}"),
        ]);
        let argv = validation_argv(&ws);
        let rendered: Vec<String> = argv.iter().map(|cmd| cmd.join(" ")).collect();
        assert_eq!(
            rendered,
            vec![
                "bun run typecheck",
                "cargo clippy",
                "bun run lint",
                "cargo test",
                "bun run test",
            ]
        );
    }

    #[test]
    fn detects_the_other_languages() {
        let (_dir, ws) = workspace(&[
            ("tsconfig.json", "{}"),
            ("pyproject.toml", ""),
            ("go.mod", "module x"),
        ]);
        let profile = workspace_profile(&ws);
        assert_eq!(profile.languages, vec!["TS", "Python", "Go"]);
        assert!(profile.validation_commands.is_empty());
        assert!(profile.frameworks.is_empty());
    }

    #[test]
    fn detects_react_next_and_tailwind_from_manifests() {
        let (_dir, ws) = workspace(&[
            (
                "package.json",
                r#"{ "dependencies": { "react": "19", "next": "15" }, "devDependencies": { "tailwindcss": "4" } }"#,
            ),
            ("next.config.ts", "export default {};\n"),
            ("tailwind.config.ts", "export default {};\n"),
        ]);
        let profile = workspace_profile(&ws);
        assert!(profile.frameworks.contains(&"react".to_string()));
        assert!(profile.frameworks.contains(&"nextjs".to_string()));
        assert!(profile.frameworks.contains(&"tailwind".to_string()));
        assert!(profile.render().contains("Frameworks:"));
    }

    #[test]
    fn dotenv_at_root_is_never_read() {
        let token = "ghp_abcDEF1234567890abcDEF1234567890";
        let (_dir, ws) = workspace(&[
            ("Cargo.toml", "[package]"),
            (".env", &format!("GITHUB_TOKEN={token}\n")),
        ]);
        let profile = workspace_profile(&ws);
        let rendered = profile.render();
        assert!(!rendered.contains(token), "o token não pode aparecer");
        assert!(!rendered.contains("GITHUB_TOKEN"));
        // The name itself is not a secret: it stays in the listing, like `list_directory` shows it.
        assert!(profile.root_entries.contains(&".env".to_string()));
        // And reading it directly is refused.
        assert_eq!(read_root_file(&ws, ".env", MAX_RULES_BYTES), None);
    }

    #[test]
    fn rules_come_from_agents_md_cut_at_four_kib() {
        let long = "regra do projeto\n".repeat(600);
        let (_dir, ws) = workspace(&[("AGENTS.md", &long), ("CLAUDE.md", "não usar")]);
        let profile = workspace_profile(&ws);
        let rules = profile.rules_excerpt.expect("AGENTS.md deve ser lido");
        assert!(rules.len() <= MAX_RULES_BYTES, "{} bytes", rules.len());
        assert!(rules.starts_with("regra do projeto"));
        assert!(!rules.contains("não usar"));
    }

    #[test]
    fn claude_md_is_the_fallback() {
        let (_dir, ws) = workspace(&[("CLAUDE.md", "regras alternativas")]);
        let profile = workspace_profile(&ws);
        assert_eq!(
            profile.rules_excerpt.as_deref(),
            Some("regras alternativas")
        );
    }

    #[test]
    fn secrets_inside_the_rules_are_redacted() {
        let token = "ghp_abcDEF1234567890abcDEF1234567890";
        let (_dir, ws) = workspace(&[("AGENTS.md", &format!("use {token} no deploy"))]);
        let profile = workspace_profile(&ws);
        let rules = profile.rules_excerpt.unwrap();
        assert!(!rules.contains(token));
        assert!(rules.contains("[REDIGIDO:segredo]"));
    }

    #[test]
    fn root_entries_are_sorted_capped_and_mark_directories() {
        let dir = tempdir().unwrap();
        for index in 0..70 {
            fs::write(dir.path().join(format!("f{index:03}.txt")), "x").unwrap();
        }
        fs::create_dir(dir.path().join("src")).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let profile = workspace_profile(&ws);
        assert_eq!(profile.root_entries.len(), MAX_ROOT_ENTRIES);
        assert_eq!(profile.root_entries[0], "f000.txt");
        let mut sorted = profile.root_entries.clone();
        sorted.sort();
        assert_eq!(sorted, profile.root_entries);

        // With few entries the directory marker shows up.
        let (_dir, small) = workspace(&[("src/main.rs", "fn main() {}")]);
        assert_eq!(
            workspace_profile(&small).root_entries,
            vec!["src/".to_string()]
        );
    }

    #[test]
    fn render_stays_within_three_kib() {
        let long = "linha de regra bem comprida para encher o orçamento\n".repeat(200);
        let (_dir, ws) = workspace(&[("AGENTS.md", &long), ("Cargo.toml", "[package]")]);
        let rendered = workspace_profile(&ws).render();
        assert!(
            rendered.len() <= MAX_RENDER_BYTES,
            "{} bytes",
            rendered.len()
        );
        assert!(rendered.contains("Languages: Rust"));
        assert!(rendered.ends_with("[profile truncated]\n"));
    }

    #[test]
    fn empty_workspace_renders_only_the_listing() {
        let dir = tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let profile = workspace_profile(&ws);
        assert!(profile.languages.is_empty());
        assert!(profile.root_entries.is_empty());
        assert_eq!(profile.render(), "");
    }
}
