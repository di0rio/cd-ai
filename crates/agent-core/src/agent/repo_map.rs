//! Compact repo map: files plus their main symbols, built with tree-sitter (SPEC §16.1).
//!
//! Never dumps the project. Walks with `ignore` (gitignore + known heavy dirs), parses
//! TS/JS/Rust, and caches per file by `(mtime, size)` under the app data dir (SPEC §16.4).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::agent::prompt::estimate_text_tokens;
use crate::redactor;
use crate::tools::{display_path, sha256_hex};
use crate::workspace::Workspace;

/// Directories that would drown the map even when gitignore is missing.
const SKIP_DIRS: [&str; 11] = [
    ".git",
    ".next",
    ".cache",
    "__pycache__",
    "build",
    "coverage",
    "dist",
    "node_modules",
    "target",
    "vendor",
    "out",
];
const INDEXABLE: [&str; 5] = ["rs", "ts", "tsx", "js", "jsx"];
const MAX_FILE_BYTES: u64 = 256 * 1024;
const MAX_FILES: usize = 1_500;
const MAX_SYMBOLS_PER_FILE: usize = 48;
const MAX_SIGNATURE_CHARS: usize = 120;
const CACHE_DIR: &str = "context";
const CACHE_FILE: &str = "map.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub kind: String,
    pub name: String,
    pub signature: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedFile {
    mtime_ms: u128,
    size: u64,
    symbols: Vec<Symbol>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CacheEnvelope {
    files: BTreeMap<String, CachedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedFile {
    pub path: String,
    pub symbols: Vec<Symbol>,
    pub score: u32,
}

#[derive(Debug, Clone, Default)]
pub struct RepoMap {
    pub files: Vec<MappedFile>,
    /// How many files were parsed this build (cache misses).
    pub parsed: u32,
    /// How many files reused the cache.
    pub cache_hits: u32,
}

impl RepoMap {
    pub fn is_cache_hit(&self) -> bool {
        self.parsed == 0 && self.cache_hits > 0
    }

    pub fn file_count(&self) -> u32 {
        self.files.len() as u32
    }
}

/// Builds (or refreshes) the repo map for `workspace`. `cache_root` is the app data dir.
pub fn build_repo_map(workspace: &Workspace, cache_root: Option<&Path>) -> RepoMap {
    let cache_path = cache_root.map(|root| cache_file(root, workspace));
    let cache = cache_path
        .as_ref()
        .and_then(|path| load_cache(path))
        .unwrap_or_default();

    let mut live: BTreeMap<String, CachedFile> = BTreeMap::new();
    let mut parsed = 0_u32;
    let mut cache_hits = 0_u32;

    let mut builder = ignore::WalkBuilder::new(workspace.root());
    builder
        .standard_filters(true)
        .follow_links(false)
        .threads(1)
        .require_git(false)
        .sort_by_file_path(|a, b| a.cmp(b))
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            let name = entry.file_name().to_string_lossy();
            !SKIP_DIRS.iter().any(|skip| name.eq_ignore_ascii_case(skip))
        });

    for entry in builder.build() {
        if live.len() >= MAX_FILES {
            break;
        }
        let Ok(entry) = entry else {
            continue;
        };
        let is_file = entry.file_type().is_some_and(|kind| kind.is_file());
        if !is_file {
            continue;
        }
        let path = entry.path();
        if !is_indexable(path) || redactor::detect_path_secret(path).is_some() {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(path) else {
            continue;
        };
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let rel = display_path(workspace, path);
        let stamp = fingerprint(&meta);
        if let Some(cached) = cache.files.get(&rel)
            && cached.mtime_ms == stamp.0
            && cached.size == stamp.1
        {
            live.insert(rel, cached.clone());
            cache_hits += 1;
            continue;
        }
        let Some(symbols) = parse_file(path) else {
            continue;
        };
        live.insert(
            rel,
            CachedFile {
                mtime_ms: stamp.0,
                size: stamp.1,
                symbols,
            },
        );
        parsed += 1;
    }

    if let Some(path) = cache_path {
        let envelope = CacheEnvelope {
            files: live.clone(),
        };
        let _ = save_cache(&path, &envelope);
    }

    let files = live
        .into_iter()
        .map(|(path, file)| MappedFile {
            path,
            symbols: file.symbols,
            score: 0,
        })
        .collect();

    RepoMap {
        files,
        parsed,
        cache_hits,
    }
}

/// Scores files against the user request and sorts the map (highest first).
pub fn rank_for_task(map: &mut RepoMap, request: &str) {
    let tokens = request_tokens(request);
    for file in &mut map.files {
        file.score = score_file(file, request, &tokens);
    }
    map.files
        .sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
}

/// Compact text for the system prompt, cut to `max_tokens` (`chars/4`).
pub fn render_map(map: &RepoMap, max_tokens: u64) -> (String, bool) {
    if map.files.is_empty() {
        return (String::new(), false);
    }
    let mut body = format!(
        "Repo map ({} files, ranked for this task):\n",
        map.files.len()
    );
    for file in &map.files {
        body.push_str(&file.path);
        body.push('\n');
        for symbol in &file.symbols {
            body.push_str("  ");
            body.push_str(&symbol.signature);
            body.push('\n');
        }
    }
    cut_section(&body, max_tokens)
}

/// Paths that look like tests of `file` (sibling `*.test.*` / `*_test.rs`).
pub fn related_tests(map: &RepoMap, file: &str) -> Vec<String> {
    let stem = Path::new(file)
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if stem.is_empty() {
        return Vec::new();
    }
    let parent = Path::new(file)
        .parent()
        .map(display_parent)
        .unwrap_or_default();
    map.files
        .iter()
        .map(|entry| entry.path.as_str())
        .filter(|path| *path != file && is_test_of(path, &stem, &parent))
        .map(str::to_string)
        .collect()
}

fn display_parent(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        String::new()
    } else {
        path.to_string_lossy().replace('\\', "/")
    }
}

fn is_test_of(path: &str, stem: &str, parent: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dir = Path::new(path)
        .parent()
        .map(display_parent)
        .unwrap_or_default();
    if dir != parent {
        return false;
    }
    name.starts_with(&format!("{stem}.test."))
        || name.starts_with(&format!("{stem}.spec."))
        || name == format!("{stem}_test.rs")
        || name == format!("test_{stem}.rs")
}

pub(crate) fn cut_section(text: &str, max_tokens: u64) -> (String, bool) {
    if max_tokens == 0 {
        return (String::new(), !text.is_empty());
    }
    if estimate_text_tokens(text) <= max_tokens {
        return (text.to_string(), false);
    }
    let max_chars = (max_tokens as usize).saturating_mul(4);
    const NOTE: &str = "\n[section truncated]";
    let keep = max_chars.saturating_sub(NOTE.len());
    let kept: String = text.chars().take(keep).collect();
    (format!("{kept}{NOTE}"), true)
}

fn cache_file(data_dir: &Path, workspace: &Workspace) -> PathBuf {
    let key = sha256_hex(workspace.root().as_os_str().as_encoded_bytes());
    data_dir.join(CACHE_DIR).join(key).join(CACHE_FILE)
}

fn load_cache(path: &Path) -> Option<CacheEnvelope> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_cache(path: &Path, cache: &CacheEnvelope) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string(cache).map_err(std::io::Error::other)?;
    fs::write(path, text)
}

fn is_indexable(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| INDEXABLE.contains(&ext))
}

fn fingerprint(meta: &fs::Metadata) -> (u128, u64) {
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    (mtime_ms, meta.len())
}

fn parse_file(path: &Path) -> Option<Vec<Symbol>> {
    let bytes = fs::read(path).ok()?;
    let source = std::str::from_utf8(&bytes).ok()?;
    let language = language_for(path)?;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source, None)?;
    let mut symbols = Vec::new();
    collect_symbols(tree.root_node(), source.as_bytes(), &mut symbols);
    symbols.truncate(MAX_SYMBOLS_PER_FILE);
    Some(symbols)
}

fn language_for(path: &Path) -> Option<tree_sitter::Language> {
    match path.extension()?.to_str()? {
        "rs" => Some(tree_sitter::Language::new(tree_sitter_rust::LANGUAGE)),
        "tsx" | "jsx" => Some(tree_sitter::Language::new(
            tree_sitter_typescript::LANGUAGE_TSX,
        )),
        "ts" | "js" => Some(tree_sitter::Language::new(
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        )),
        _ => None,
    }
}

fn collect_symbols(node: tree_sitter::Node<'_>, source: &[u8], out: &mut Vec<Symbol>) {
    if out.len() >= MAX_SYMBOLS_PER_FILE {
        return;
    }
    if let Some(kind) = interesting_kind(node.kind()) {
        let name = node
            .child_by_field_name("name")
            .and_then(|child| child.utf8_text(source).ok())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or("")
            .to_string();
        let signature = signature_line(node, source);
        if !signature.is_empty() {
            out.push(Symbol {
                kind: kind.to_string(),
                name,
                signature,
                line: node.start_position().row as u32 + 1,
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_symbols(child, source, out);
        if out.len() >= MAX_SYMBOLS_PER_FILE {
            return;
        }
    }
}

fn interesting_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "function_item"
        | "function_declaration"
        | "generator_function_declaration"
        | "function_signature" => Some("fn"),
        "method_definition" | "method_signature" => Some("method"),
        "struct_item" => Some("struct"),
        "enum_item" | "enum_declaration" => Some("enum"),
        "trait_item" => Some("trait"),
        "impl_item" => Some("impl"),
        "type_item" | "type_alias_declaration" => Some("type"),
        "class_declaration" | "abstract_class_declaration" => Some("class"),
        "interface_declaration" => Some("interface"),
        "mod_item" => Some("mod"),
        "const_item" => Some("const"),
        _ => None,
    }
}

fn signature_line(node: tree_sitter::Node<'_>, source: &[u8]) -> String {
    let text = node.utf8_text(source).unwrap_or("").trim();
    let line = text.lines().next().unwrap_or("").trim();
    let line = line
        .trim_end_matches('{')
        .trim()
        .trim_end_matches(';')
        .trim();
    if line.chars().count() <= MAX_SIGNATURE_CHARS {
        return line.to_string();
    }
    let kept: String = line.chars().take(MAX_SIGNATURE_CHARS).collect();
    format!("{kept}…")
}

fn request_tokens(request: &str) -> Vec<String> {
    request
        .split(|ch: char| !ch.is_alphanumeric())
        .map(|token| token.to_ascii_lowercase())
        .filter(|token| token.len() >= 3 && !STOPWORDS.contains(&token.as_str()))
        .collect()
}

const STOPWORDS: [&str; 16] = [
    "the", "and", "for", "com", "uma", "que", "por", "dos", "das", "this", "that", "from", "with",
    "sem", "para", "you",
];

fn score_file(file: &MappedFile, request: &str, tokens: &[String]) -> u32 {
    let path_lower = file.path.to_ascii_lowercase();
    let mut score = 0_u32;
    if request.contains(&file.path) {
        score += 20;
    }
    for token in tokens {
        if path_lower.contains(token) {
            score += 6;
        }
        for symbol in &file.symbols {
            if symbol.name.to_ascii_lowercase() == *token {
                score += 8;
            } else if symbol.signature.to_ascii_lowercase().contains(token) {
                score += 2;
            }
        }
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn extracts_rust_and_typescript_symbols() {
        let (_dir, ws) = workspace(&[
            (
                "src/lib.rs",
                "pub fn soma(a: i32, b: i32) -> i32 { a + b }\n\
                 pub struct Conta { pub valor: i32 }\n\
                 impl Conta { pub fn nova() -> Self { Self { valor: 0 } } }\n",
            ),
            (
                "src/soma.ts",
                "export function soma(a: number, b: number): number { return a + b; }\n\
                 export class Calc { add(n: number) { return n; } }\n\
                 export interface Ops { run(): void }\n",
            ),
        ]);
        let map = build_repo_map(&ws, None);
        let rust = map
            .files
            .iter()
            .find(|file| file.path == "src/lib.rs")
            .unwrap();
        let names: Vec<&str> = rust.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"soma"), "{names:?}");
        assert!(names.contains(&"Conta"), "{names:?}");
        let ts = map
            .files
            .iter()
            .find(|file| file.path == "src/soma.ts")
            .unwrap();
        let ts_names: Vec<&str> = ts.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(ts_names.contains(&"soma"), "{ts_names:?}");
        assert!(
            ts_names
                .iter()
                .any(|name| *name == "Calc" || ts.symbols.iter().any(|s| s.kind == "class"))
        );
        assert!(ts.symbols.iter().any(|s| s.kind == "interface"));
    }

    #[test]
    fn skips_node_modules_and_secrets() {
        let (_dir, ws) = workspace(&[
            ("src/app.ts", "export function app() { return 1; }\n"),
            (
                "node_modules/lib/index.js",
                "export function noise() { return 0; }\n",
            ),
            (".env", "SECRET=1\n"),
        ]);
        let map = build_repo_map(&ws, None);
        assert_eq!(map.files.len(), 1);
        assert_eq!(map.files[0].path, "src/app.ts");
    }

    #[test]
    fn ranks_the_file_named_in_the_request() {
        let (_dir, ws) = workspace(&[
            (
                "src/soma.ts",
                "export function soma(a: number, b: number): number { return a; }\n",
            ),
            (
                "src/greet.ts",
                "export function greet(name: string): string { return name; }\n",
            ),
        ]);
        let mut map = build_repo_map(&ws, None);
        rank_for_task(&mut map, "O teste de soma falha. Corrija.");
        assert_eq!(map.files[0].path, "src/soma.ts");
        assert!(map.files[0].score > map.files[1].score);
    }

    #[test]
    fn cache_reparses_only_the_file_that_changed() {
        let data = tempdir().unwrap();
        let (_dir, ws) = workspace(&[
            ("src/a.ts", "export function a() { return 1; }\n"),
            ("src/b.ts", "export function b() { return 2; }\n"),
        ]);
        let first = build_repo_map(&ws, Some(data.path()));
        assert_eq!(first.parsed, 2);
        let second = build_repo_map(&ws, Some(data.path()));
        assert_eq!(second.parsed, 0);
        assert_eq!(second.cache_hits, 2);
        assert!(second.is_cache_hit());

        fs::write(
            ws.root().join("src/a.ts"),
            "export function a() { return 1; }\nexport function extra() { return 3; }\n",
        )
        .unwrap();
        let third = build_repo_map(&ws, Some(data.path()));
        assert_eq!(third.parsed, 1, "só a.ts mudou");
        assert_eq!(third.cache_hits, 1);
        let a = third
            .files
            .iter()
            .find(|file| file.path == "src/a.ts")
            .unwrap();
        assert!(a.symbols.iter().any(|s| s.name == "extra"));
    }

    #[test]
    fn the_map_is_much_smaller_than_dumping_sources() {
        let body = format!(
            "export function piece(n: number): number {{\n{}\n  return n;\n}}\n",
            "  // padding line that never belongs in a map\n".repeat(80)
        );
        let (_dir, ws) = workspace(&[
            ("src/a.ts", &body),
            ("src/b.ts", &body),
            ("src/c.ts", &body),
        ]);
        let mut map = build_repo_map(&ws, None);
        rank_for_task(&mut map, "piece");
        let dumped: usize = body.len() * 3;
        let (full, cut) = render_map(&map, 10_000);
        assert!(!cut);
        assert!(
            full.len() * 8 < dumped,
            "mapa {} vs dump {}",
            full.len(),
            dumped
        );
        assert!(full.contains("src/a.ts"));
        assert!(full.contains("piece"));
        let (tight, cut) = render_map(&map, 8);
        assert!(cut);
        assert!(tight.contains("[section truncated]"));
    }

    #[test]
    fn related_tests_find_the_sibling() {
        let (_dir, ws) = workspace(&[
            ("src/soma.ts", "export function soma() { return 0; }\n"),
            ("src/soma.test.ts", "test('soma', () => {});\n"),
            ("src/greet.test.ts", "test('greet', () => {});\n"),
        ]);
        let map = build_repo_map(&ws, None);
        let tests = related_tests(&map, "src/soma.ts");
        assert_eq!(tests, vec!["src/soma.test.ts"]);
    }

    #[test]
    fn cut_section_marks_the_overflow() {
        let (text, cut) = cut_section(&"abcd".repeat(50), 4);
        assert!(cut);
        assert!(text.contains("[section truncated]"));
        assert!(estimate_text_tokens(&text) <= 5);
    }
}
