//! Compact repo map (SPEC §16.1, plan 020): workspace-relative paths plus the main signatures
//! tree-sitter can name, scored against the current request.
//!
//! The walk respects `.gitignore` (same `ignore` builder as `search`). Secret paths are never
//! opened. A file larger than [`MAX_PARSE_BYTES`] is listed without symbols. The cache on disk
//! is keyed by mtime and size, not by time.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::redactor;
use crate::syntax::language_for_path;
use crate::tools::{display_path, sha256_hex};
use crate::workspace::Workspace;

/// Files bigger than this are named in the map but not parsed.
const MAX_PARSE_BYTES: u64 = 256 * 1024;
/// Hard cap on indexed source files: a map is an orientation, not a dump.
const MAX_INDEXED_FILES: usize = 2_000;
/// Symbols kept per file after scoring.
const MAX_SYMBOLS_PER_FILE: usize = 32;
/// Walk entries to look at before giving up (gitignore already drops the fat dirs).
const MAX_WALK_ENTRIES: usize = 8_000;
const CACHE_SCHEMA: u32 = 1;
const CACHE_DIR: &str = "cache";

/// One named declaration the map can show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    /// First line of the node, trimmed and short.
    pub signature: String,
}

/// One file in the map: path plus the symbols that survived the budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapFile {
    pub path: String,
    pub symbols: Vec<Symbol>,
    /// Higher means more relevant to the request that scored this map.
    #[serde(default)]
    pub score: u32,
}

/// Compact list of files and signatures, ready to render into the system prompt.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RepoMap {
    pub files: Vec<MapFile>,
    /// How many source files were parsed (before scoring dropped some).
    pub indexed: usize,
    pub from_cache: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiskCache {
    schema: u32,
    root: String,
    files: Vec<CachedFile>,
    profile_markers: Vec<MarkerStamp>,
    profile_render: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedFile {
    path: String,
    mtime_ms: u64,
    size: u64,
    symbols: Vec<Symbol>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MarkerStamp {
    name: String,
    mtime_ms: u64,
    size: u64,
}

/// Builds or refreshes the map for `workspace`, using `data_dir` for the on-disk cache.
pub fn load_repo_map(workspace: &Workspace, data_dir: Option<&Path>, request: &str) -> RepoMap {
    let (mut files, from_cache) = index_workspace(workspace, data_dir);
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let indexed = files.len();
    score_files(&mut files, request);
    files.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
    RepoMap {
        files,
        indexed,
        from_cache,
    }
}

/// Cached workspace profile render, when the marker files have not changed.
pub fn cached_profile_render(workspace: &Workspace, data_dir: Option<&Path>) -> Option<String> {
    let cache = read_cache(workspace, data_dir?)?;
    if cache.schema != CACHE_SCHEMA {
        return None;
    }
    let current = marker_stamps(workspace);
    (cache.profile_markers == current && !cache.profile_render.is_empty())
        .then_some(cache.profile_render)
}

/// Writes the profile render next to the file index so the next task can skip the root scan.
pub fn store_profile_render(workspace: &Workspace, data_dir: &Path, render: &str) {
    let mut cache = read_cache(workspace, data_dir).unwrap_or_else(|| DiskCache {
        schema: CACHE_SCHEMA,
        root: workspace.root().to_string_lossy().into_owned(),
        files: Vec::new(),
        profile_markers: Vec::new(),
        profile_render: String::new(),
    });
    cache.profile_markers = marker_stamps(workspace);
    cache.profile_render = render.to_string();
    let _ = write_cache(workspace, data_dir, &cache);
}

impl RepoMap {
    /// Plain text for the system prompt. `max_chars` is the section budget (chars, not tokens).
    pub fn render(&self, max_chars: usize) -> (String, bool) {
        if self.files.is_empty() {
            return (String::new(), false);
        }
        let mut lines: Vec<String> = Vec::new();
        for file in &self.files {
            lines.push(file.path.clone());
            for symbol in file.symbols.iter().take(MAX_SYMBOLS_PER_FILE) {
                lines.push(format!("  {}", symbol.signature));
            }
        }
        let full = lines.join("\n");
        if full.chars().count() <= max_chars {
            return (full, false);
        }
        // Drop lowest-score files first (the vec is already score-desc).
        let mut keep = lines.len();
        while keep > 0 {
            let candidate = lines[..keep].join("\n");
            if candidate.chars().count() <= max_chars.saturating_sub(24) {
                return (format!("{candidate}\n[repo map truncated]"), true);
            }
            keep = keep.saturating_sub(1);
            // Drop a whole file at a time when possible: back up to the previous path-only line.
            while keep > 0 && lines[keep - 1].starts_with("  ") {
                keep -= 1;
            }
        }
        (String::from("[repo map truncated]"), true)
    }
}

fn index_workspace(workspace: &Workspace, data_dir: Option<&Path>) -> (Vec<MapFile>, bool) {
    let cached = data_dir.and_then(|dir| read_cache(workspace, dir));
    let cached_ok = cached
        .as_ref()
        .is_some_and(|cache| cache.schema == CACHE_SCHEMA);
    let cached_files = if cached_ok {
        cached.as_ref().unwrap().files.clone()
    } else {
        Vec::new()
    };

    let mut by_path: std::collections::HashMap<String, CachedFile> = cached_files
        .into_iter()
        .map(|file| (file.path.clone(), file))
        .collect();

    let mut fresh: Vec<CachedFile> = Vec::new();
    let mut reparsed = 0_usize;

    let mut builder = ignore::WalkBuilder::new(workspace.root());
    builder
        .standard_filters(true)
        .threads(1)
        .require_git(false)
        .sort_by_file_path(|a, b| a.cmp(b));

    for (seen, entry) in builder.build().enumerate() {
        if seen >= MAX_WALK_ENTRIES || fresh.len() >= MAX_INDEXED_FILES {
            break;
        }
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.path();
        if redactor::detect_path_secret(path).is_some() {
            continue;
        }
        let relative = display_path(workspace, path);
        if relative.is_empty() {
            continue;
        }
        let Ok(meta) = fs::metadata(path) else {
            continue;
        };
        let size = meta.len();
        let mtime_ms = mtime_millis(&meta);
        if let Some(hit) = by_path.remove(&relative)
            && hit.mtime_ms == mtime_ms
            && hit.size == size
        {
            fresh.push(hit);
            continue;
        }
        let symbols = if language_for_path(path).is_some() && size <= MAX_PARSE_BYTES {
            reparsed += 1;
            extract_symbols(path)
        } else {
            Vec::new()
        };
        fresh.push(CachedFile {
            path: relative,
            mtime_ms,
            size,
            symbols,
        });
    }

    // Paths that were in the cache but not on disk anymore are dropped by not pushing them.
    if let Some(dir) = data_dir {
        let cache = DiskCache {
            schema: CACHE_SCHEMA,
            root: workspace.root().to_string_lossy().into_owned(),
            files: fresh.clone(),
            profile_markers: cached
                .as_ref()
                .map(|cache| cache.profile_markers.clone())
                .unwrap_or_default(),
            profile_render: cached
                .as_ref()
                .map(|cache| cache.profile_render.clone())
                .unwrap_or_default(),
        };
        let _ = write_cache(workspace, dir, &cache);
    }

    let files: Vec<MapFile> = fresh
        .into_iter()
        .filter(|file| {
            language_for_path(Path::new(&file.path)).is_some() || !file.symbols.is_empty()
        })
        .map(|file| MapFile {
            path: file.path,
            symbols: file.symbols,
            score: 0,
        })
        .collect();
    let from_cache = cached_ok && reparsed == 0 && !files.is_empty();
    (files, from_cache)
}

fn extract_symbols(path: &Path) -> Vec<Symbol> {
    let Ok(bytes) = fs::read(path) else {
        return Vec::new();
    };
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
    collect_symbols(tree.root_node(), content.as_ref(), &mut symbols);
    symbols.truncate(MAX_SYMBOLS_PER_FILE);
    symbols
}

fn collect_symbols(node: tree_sitter::Node<'_>, source: &str, out: &mut Vec<Symbol>) {
    if out.len() >= MAX_SYMBOLS_PER_FILE {
        return;
    }
    if is_declaration(node.kind())
        && let Some(symbol) = symbol_from_node(node, source)
    {
        out.push(symbol);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_symbols(child, source, out);
        if out.len() >= MAX_SYMBOLS_PER_FILE {
            return;
        }
    }
}

fn is_declaration(kind: &str) -> bool {
    matches!(
        kind,
        "function_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "impl_item"
            | "mod_item"
            | "function_declaration"
            | "generator_function_declaration"
            | "method_definition"
            | "class_declaration"
            | "interface_declaration"
            | "type_alias_declaration"
            | "enum_declaration"
            | "export_statement"
    )
}

fn symbol_from_node(node: tree_sitter::Node<'_>, source: &str) -> Option<Symbol> {
    if node.kind() == "export_statement" {
        // Prefer the inner declaration; walking already visits it. Skip the wrapper so
        // `export function soma` is not listed twice.
        return None;
    }
    let name = node
        .child_by_field_name("name")
        .or_else(|| node.child_by_field_name("type"))
        .and_then(|child| child.utf8_text(source.as_bytes()).ok())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| first_identifier(node, source))?;
    let raw = node.utf8_text(source.as_bytes()).ok().unwrap_or("");
    let signature = first_line(raw, 120);
    if signature.is_empty() {
        return None;
    }
    Some(Symbol { name, signature })
}

fn first_identifier(node: tree_sitter::Node<'_>, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "identifier" | "type_identifier" | "property_identifier"
        ) && let Ok(text) = child.utf8_text(source.as_bytes())
        {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn first_line(text: &str, max_chars: usize) -> String {
    let line = text.lines().next().unwrap_or(text).trim();
    if line.chars().count() <= max_chars {
        return line.to_string();
    }
    line.chars().take(max_chars).collect()
}

fn score_files(files: &mut [MapFile], request: &str) {
    let tokens = request_tokens(request);
    for file in files.iter_mut() {
        let mut score = 0_u32;
        let path_l = file.path.to_ascii_lowercase();
        for token in &tokens {
            if path_l.contains(token) {
                score += 8;
            }
            for symbol in &file.symbols {
                if symbol.name.to_ascii_lowercase().contains(token.as_str()) {
                    score += 5;
                }
            }
        }
        if path_l.contains("test") {
            score += 1;
        }
        file.score = score;
    }
}

fn request_tokens(request: &str) -> Vec<String> {
    request
        .to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| word.len() >= 3)
        .filter(|word| !STOP.contains(word))
        .map(ToOwned::to_owned)
        .collect()
}

const STOP: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "you", "are", "was", "test", "tests",
    "file", "files", "please", "just", "the", "uma", "para", "com", "que", "dos", "das", "por",
    "falha", "rode", "os", "as", "função", "funcao", "modulo", "módulo",
];

fn marker_stamps(workspace: &Workspace) -> Vec<MarkerStamp> {
    const NAMES: &[&str] = &[
        "Cargo.toml",
        "package.json",
        "tsconfig.json",
        "AGENTS.md",
        "CLAUDE.md",
        "bun.lock",
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "pyproject.toml",
        "go.mod",
    ];
    let mut stamps = Vec::new();
    for name in NAMES {
        let path = workspace.root().join(name);
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        stamps.push(MarkerStamp {
            name: (*name).to_string(),
            mtime_ms: mtime_millis(&meta),
            size: meta.len(),
        });
    }
    stamps
}

fn cache_path(workspace: &Workspace, data_dir: &Path) -> PathBuf {
    let id = sha256_hex(workspace.root().to_string_lossy().as_bytes());
    data_dir.join(CACHE_DIR).join(id).join("repo-map.json")
}

fn read_cache(workspace: &Workspace, data_dir: &Path) -> Option<DiskCache> {
    let text = fs::read_to_string(cache_path(workspace, data_dir)).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_cache(workspace: &Workspace, data_dir: &Path, cache: &DiskCache) -> std::io::Result<()> {
    let path = cache_path(workspace, data_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string(cache).unwrap_or_else(|_| "{}".to_string());
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)
}

fn mtime_millis(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;
    use std::fs;
    use tempfile::tempdir;

    fn project(files: &[(&str, &str)]) -> (tempfile::TempDir, Workspace) {
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
    fn typescript_function_lands_in_the_map_and_scores_against_the_request() {
        let (_dir, workspace) = project(&[(
            "src/soma.ts",
            "export function soma(a: number, b: number): number {\n  return a + b;\n}\n",
        )]);
        let map = load_repo_map(&workspace, None, "corrija a soma");
        assert_eq!(map.files.len(), 1);
        assert_eq!(map.files[0].path, "src/soma.ts");
        assert!(
            map.files[0]
                .symbols
                .iter()
                .any(|symbol| symbol.name == "soma"),
            "{:?}",
            map.files[0].symbols
        );
        assert!(map.files[0].score > 0);
        let (rendered, cut) = map.render(4_000);
        assert!(rendered.contains("src/soma.ts"));
        assert!(rendered.contains("soma"));
        assert!(!cut);
    }

    #[test]
    fn rust_struct_and_fn_are_indexed() {
        let (_dir, workspace) = project(&[(
            "src/lib.rs",
            "pub struct Point { x: i32 }\npub fn origin() -> Point { Point { x: 0 } }\n",
        )]);
        let map = load_repo_map(&workspace, None, "Point origin");
        let names: Vec<&str> = map.files[0]
            .symbols
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect();
        assert!(names.contains(&"Point"), "{names:?}");
        assert!(names.contains(&"origin"), "{names:?}");
    }

    #[test]
    fn gitignored_and_secret_files_are_not_opened() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "secret.ts\n").unwrap();
        fs::write(dir.path().join("ok.ts"), "export function ok() {}\n").unwrap();
        fs::write(
            dir.path().join("secret.ts"),
            "export function leaked() {}\n",
        )
        .unwrap();
        fs::write(dir.path().join(".env"), "TOKEN=abc\n").unwrap();
        let workspace = Workspace::open(dir.path()).unwrap();
        let map = load_repo_map(&workspace, None, "leaked");
        let paths: Vec<&str> = map.files.iter().map(|file| file.path.as_str()).collect();
        assert!(paths.contains(&"ok.ts"), "{paths:?}");
        assert!(
            !paths.iter().any(|path| path.contains("secret")),
            "{paths:?}"
        );
        assert!(!paths.iter().any(|path| path.contains(".env")), "{paths:?}");
    }

    #[test]
    fn cache_reuses_unchanged_files_and_reparses_after_an_edit() {
        let (dir, workspace) = project(&[("src/a.ts", "export function oldName() {}\n")]);
        let data = tempdir().unwrap();
        let first = load_repo_map(&workspace, Some(data.path()), "oldName");
        assert!(
            first.files[0]
                .symbols
                .iter()
                .any(|symbol| symbol.name == "oldName")
        );

        let second = load_repo_map(&workspace, Some(data.path()), "oldName");
        assert!(second.from_cache, "o segundo load tem que bater no cache");

        let path = dir.path().join("src/a.ts");
        fs::write(&path, "export function newName() { return 1; }\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(&path, "export function newName() { return 1; }\n").unwrap();

        let third = load_repo_map(&workspace, Some(data.path()), "newName");
        assert!(
            third.files[0]
                .symbols
                .iter()
                .any(|symbol| symbol.name == "newName"),
            "{:?}",
            third.files[0].symbols
        );
    }

    #[test]
    fn a_tight_budget_truncates_the_map() {
        let (_dir, workspace) = project(&[
            ("src/a.ts", "export function alpha() {}\n"),
            ("src/b.ts", "export function beta() {}\n"),
            ("src/c.ts", "export function gamma() {}\n"),
        ]);
        let map = load_repo_map(&workspace, None, "alpha");
        let (rendered, cut) = map.render(40);
        assert!(cut);
        assert!(rendered.contains("[repo map truncated]"));
        assert!(rendered.chars().count() <= 80);
    }
}
