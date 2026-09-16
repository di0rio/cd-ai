//! Compact repo map: workspace files with top-level symbols from tree-sitter (SPEC §16.1).
//!
//! The walk respects gitignore. Secret files are never opened. The on-disk cache is keyed by
//! workspace root and invalidated by mtime+size, never by a TTL (plan 020, D3).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::redactor;
use crate::tools::edit::language_for_path;
use crate::tools::{display_path, sha256_hex};
use crate::workspace::Workspace;

const CACHE_DIR: &str = "cache";
const CACHE_FILE: &str = "repo-map.json";
const MAX_FILE_BYTES: u64 = 128 * 1024;
const MAX_FILES: usize = 1_500;
const MAX_SYMBOLS_PER_FILE: usize = 12;
const MAX_SIG_CHARS: usize = 120;
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    ".next",
    "vendor",
    ".git",
    "coverage",
    "out",
];

/// One file in the map, with the signatures the model can search from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMap {
    pub path: String,
    pub symbols: Vec<String>,
    mtime_ms: u64,
    len: u64,
}

/// Ranked, compact listing for one task.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoMap {
    pub files: Vec<FileMap>,
    /// How many files were re-parsed this build (the rest came from cache).
    pub parsed: usize,
    /// How many cached entries were reused.
    pub reused: usize,
}

/// On-disk cache for one workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct CachedIndex {
    files: BTreeMap<String, CachedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedFile {
    mtime_ms: u64,
    len: u64,
    symbols: Vec<String>,
}

/// Cache directory for one workspace, under the app data dir.
#[derive(Debug, Clone)]
pub struct RepoMapCache {
    path: PathBuf,
}

impl RepoMapCache {
    pub fn open(data_dir: impl AsRef<Path>, workspace: &Workspace) -> Self {
        let key = sha256_hex(workspace.root().to_string_lossy().as_bytes());
        Self {
            path: data_dir.as_ref().join(CACHE_DIR).join(key).join(CACHE_FILE),
        }
    }

    fn load(&self) -> CachedIndex {
        fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self, index: &CachedIndex) {
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string(index) {
            let _ = fs::write(&self.path, text);
        }
    }
}

/// Builds (or reuses) the map and ranks it for `request`.
pub fn build_repo_map(
    workspace: &Workspace,
    request: &str,
    cache: Option<&RepoMapCache>,
) -> RepoMap {
    let previous = cache.map(RepoMapCache::load).unwrap_or_default();
    let mut next = CachedIndex::default();
    let mut files = Vec::new();
    let mut parsed = 0;
    let mut reused = 0;

    let mut builder = ignore::WalkBuilder::new(workspace.root());
    builder
        .standard_filters(true)
        .threads(1)
        .require_git(false)
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !SKIP_DIRS.iter().any(|skip| name == *skip)
        });

    for entry in builder.build() {
        if files.len() >= MAX_FILES {
            break;
        }
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.path();
        if redactor::detect_path_secret(path).is_some() {
            continue;
        }
        if language_for_path(path).is_none() {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let rel = display_path(workspace, path);
        let (mtime_ms, len) = fingerprint(&meta);
        let symbols = if let Some(cached) = previous.files.get(&rel)
            && cached.mtime_ms == mtime_ms
            && cached.len == len
        {
            reused += 1;
            cached.symbols.clone()
        } else {
            parsed += 1;
            extract_symbols(path)
        };
        next.files.insert(
            rel.clone(),
            CachedFile {
                mtime_ms,
                len,
                symbols: symbols.clone(),
            },
        );
        files.push(FileMap {
            path: rel,
            symbols,
            mtime_ms,
            len,
        });
    }

    if let Some(cache) = cache
        && (parsed > 0 || next.files.len() != previous.files.len())
    {
        cache.save(&next);
    }

    rank_files(&mut files, request);
    RepoMap {
        files,
        parsed,
        reused,
    }
}

impl RepoMap {
    /// Compact text for the system prompt. `max_chars` is the section budget.
    pub fn render(&self, max_chars: usize) -> (String, bool) {
        if self.files.is_empty() {
            return (String::new(), false);
        }
        let mut out = String::from("Repo map (ranked for this task; not the whole project):\n");
        let mut truncated = false;
        for file in &self.files {
            let line = if file.symbols.is_empty() {
                format!("{}\n", file.path)
            } else {
                format!("{}: {}\n", file.path, file.symbols.join("; "))
            };
            if out.len() + line.len() > max_chars {
                truncated = true;
                break;
            }
            out.push_str(&line);
        }
        if truncated {
            out.push_str("[repo map truncated]\n");
        }
        (out, truncated)
    }

    /// Test files that sit next to a file the request scored.
    pub fn related_tests(&self) -> Vec<String> {
        let scored: Vec<&str> = self
            .files
            .iter()
            .filter(|file| !is_test_path(&file.path))
            .take(8)
            .map(|file| file.path.as_str())
            .collect();
        self.files
            .iter()
            .filter(|file| {
                is_test_path(&file.path)
                    && scored.iter().any(|source| related_test(source, &file.path))
            })
            .map(|file| file.path.clone())
            .collect()
    }
}

fn rank_files(files: &mut [FileMap], request: &str) {
    let tokens = query_tokens(request);
    if tokens.is_empty() {
        files.sort_by(|a, b| a.path.cmp(&b.path));
        return;
    }
    files.sort_by(|a, b| {
        score(b, &tokens)
            .cmp(&score(a, &tokens))
            .then_with(|| is_test_path(&a.path).cmp(&is_test_path(&b.path)))
            .then_with(|| a.path.cmp(&b.path))
    });
}

fn score(file: &FileMap, tokens: &[String]) -> u32 {
    let hay = format!(
        "{} {}",
        file.path.to_ascii_lowercase(),
        file.symbols.join(" ").to_ascii_lowercase()
    );
    let mut points = 0_u32;
    for token in tokens {
        if hay.contains(token) {
            points += 2;
        }
    }
    points
}

/// Tokens worth matching, lowercase, length ≥ 3, no stopwords.
pub fn query_tokens(request: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "the", "and", "for", "from", "with", "that", "this", "para", "com", "uma", "por", "dos",
        "das", "que", "nao", "não", "sem", "falha", "rode", "teste",
    ];
    let mut tokens = Vec::new();
    for raw in request.split(|c: char| !c.is_ascii_alphanumeric()) {
        let token = raw.to_ascii_lowercase();
        if token.chars().count() < 3 {
            continue;
        }
        if STOP.contains(&token.as_str()) {
            continue;
        }
        if !tokens.contains(&token) {
            tokens.push(token);
        }
    }
    tokens
}

fn is_test_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains(".test.")
        || lower.contains(".spec.")
        || lower.ends_with("_test.rs")
        || lower.contains("/tests/")
}

fn related_test(source: &str, test: &str) -> bool {
    let stem = Path::new(source)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(source);
    test.contains(stem)
}

fn fingerprint(meta: &fs::Metadata) -> (u64, u64) {
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    (mtime_ms, meta.len())
}

fn extract_symbols(path: &Path) -> Vec<String> {
    let Ok(bytes) = fs::read(path) else {
        return Vec::new();
    };
    if bytes.contains(&0) {
        return Vec::new();
    }
    let content = String::from_utf8_lossy(&bytes);
    let Some((_, language)) = language_for_path(path) else {
        return Vec::new();
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(content.as_bytes(), None) else {
        return Vec::new();
    };
    let mut symbols = Vec::new();
    collect_symbols(tree.root_node(), content.as_bytes(), &mut symbols, 0);
    symbols.truncate(MAX_SYMBOLS_PER_FILE);
    symbols
}

const INTERESTING: &[&str] = &[
    "function_item",
    "struct_item",
    "enum_item",
    "trait_item",
    "impl_item",
    "mod_item",
    "type_item",
    "const_item",
    "static_item",
    "function_declaration",
    "generator_function_declaration",
    "class_declaration",
    "interface_declaration",
    "type_alias_declaration",
    "enum_declaration",
    "method_definition",
    "public_method_definition",
    "export_statement",
];

fn collect_symbols(node: tree_sitter::Node<'_>, src: &[u8], out: &mut Vec<String>, depth: u32) {
    if out.len() >= MAX_SYMBOLS_PER_FILE || depth > 6 {
        return;
    }
    let kind = node.kind();
    if INTERESTING.contains(&kind) {
        if let Some(sig) = signature_of(node, src)
            && !out.iter().any(|seen| seen == &sig)
        {
            out.push(sig);
        }
        // Function bodies are noise; impl/class/mod still have methods inside.
        if matches!(
            kind,
            "function_item"
                | "function_declaration"
                | "generator_function_declaration"
                | "method_definition"
                | "public_method_definition"
                | "const_item"
                | "static_item"
                | "type_item"
                | "type_alias_declaration"
        ) {
            return;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_symbols(child, src, out, depth + 1);
    }
}

fn signature_of(node: tree_sitter::Node<'_>, src: &[u8]) -> Option<String> {
    let text = node.utf8_text(src).ok()?.trim();
    let first = text.lines().next()?.trim().trim_end_matches('{').trim();
    if first.is_empty() {
        return None;
    }
    Some(cut_chars(first, MAX_SIG_CHARS))
}

fn cut_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;
    use std::fs;
    use std::thread;
    use std::time::Duration;
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
    fn extracts_typescript_and_rust_signatures() {
        let (_dir, ws) = workspace(&[
            (
                "src/soma.ts",
                "export function soma(a: number, b: number): number {\n  return a + b;\n}\n",
            ),
            (
                "src/lib.rs",
                "pub fn add(a: i32, b: i32) -> i32 { a + b }\npub struct Foo;\n",
            ),
        ]);
        let map = build_repo_map(&ws, "soma add", None);
        let soma = map.files.iter().find(|f| f.path == "src/soma.ts").unwrap();
        assert!(
            soma.symbols.iter().any(|s| s.contains("soma")),
            "{:?}",
            soma.symbols
        );
        let rust = map.files.iter().find(|f| f.path == "src/lib.rs").unwrap();
        assert!(
            rust.symbols.iter().any(|s| s.contains("add")),
            "{:?}",
            rust.symbols
        );
        assert!(
            rust.symbols.iter().any(|s| s.contains("Foo")),
            "{:?}",
            rust.symbols
        );
    }

    #[test]
    fn ranks_the_file_named_in_the_request() {
        let (_dir, ws) = workspace(&[
            ("src/alpha.ts", "export function alpha() {}\n"),
            ("src/beta.ts", "export function beta() {}\n"),
            ("src/beta.test.ts", "import { beta } from \"./beta\";\n"),
        ]);
        let map = build_repo_map(&ws, "corrija beta", None);
        assert_eq!(map.files[0].path, "src/beta.ts");
        assert!(map.related_tests().iter().any(|p| p.contains("beta.test")));
    }

    #[test]
    fn gitignore_and_secrets_are_skipped() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "ignored/\n").unwrap();
        fs::create_dir(dir.path().join("ignored")).unwrap();
        fs::write(
            dir.path().join("ignored/secret.ts"),
            "export const x = 1;\n",
        )
        .unwrap();
        fs::write(dir.path().join(".env"), "TOKEN=abc\n").unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/ok.ts"), "export function ok() {}\n").unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let map = build_repo_map(&ws, "ok", None);
        assert!(map.files.iter().any(|f| f.path == "src/ok.ts"));
        assert!(!map.files.iter().any(|f| f.path.contains("ignored")));
        assert!(!map.files.iter().any(|f| f.path.contains(".env")));
    }

    #[test]
    fn cache_reuses_unchanged_files_and_reparses_after_mtime() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("a.ts"), "export function a() {}\n").unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let data = tempdir().unwrap();
        let cache = RepoMapCache::open(data.path(), &ws);

        let first = build_repo_map(&ws, "a", Some(&cache));
        assert_eq!(first.parsed, 1);
        let second = build_repo_map(&ws, "a", Some(&cache));
        assert_eq!(second.reused, 1);
        assert_eq!(second.parsed, 0);

        thread::sleep(Duration::from_millis(1_100));
        fs::write(src.join("a.ts"), "export function aRenamed() {}\n").unwrap();
        let third = build_repo_map(&ws, "aRenamed", Some(&cache));
        assert_eq!(third.parsed, 1);
        assert!(
            third.files[0]
                .symbols
                .iter()
                .any(|s| s.contains("aRenamed")),
            "{:?}",
            third.files[0].symbols
        );
    }

    #[test]
    fn render_respects_the_char_budget() {
        let files: Vec<FileMap> = (0..40)
            .map(|i| FileMap {
                path: format!("src/f{i:02}.ts"),
                symbols: vec![format!("export function f{i}()")],
                mtime_ms: 0,
                len: 10,
            })
            .collect();
        let map = RepoMap {
            files,
            parsed: 0,
            reused: 0,
        };
        let (text, truncated) = map.render(200);
        assert!(truncated);
        assert!(text.contains("[repo map truncated]"));
        assert!(text.len() <= 240, "{}", text.len());
    }
}
