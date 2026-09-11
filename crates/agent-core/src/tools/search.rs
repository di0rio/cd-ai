use grep_regex::RegexMatcherBuilder;
use grep_searcher::{Searcher, SearcherBuilder, Sink, SinkMatch};

use crate::redactor;
use crate::tools::{
    MAX_SEARCH_BYTES, MAX_SEARCH_RESULTS, SearchArgs, SearchMatch, SearchResult, ToolEngine,
    ToolError, display_path,
};

/// Total match ceiling: keeps `total` bounded on pathological queries.
const MAX_TOTAL_MATCHES: u64 = 1_000_000;

/// ripgrep engine (`grep-regex` + `grep-searcher`) over an `ignore` walk (design D4).
/// `regex: false` is a literal search via `\Q..\E` — no regex, no ReDoS.
pub fn search(engine: &mut ToolEngine, args: SearchArgs) -> Result<SearchResult, ToolError> {
    if args.query.is_empty() {
        return Err(ToolError::Io("query vazia".to_string()));
    }
    let root = match args.path.as_deref() {
        Some(path) => engine.workspace.resolve(path)?,
        None => engine.workspace.root().to_path_buf(),
    };
    let metadata = std::fs::metadata(&root).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ToolError::NotFound
        } else {
            ToolError::Io(error.to_string())
        }
    })?;
    if !metadata.is_dir() {
        return Err(ToolError::NotADirectory);
    }

    let matcher = if args.regex {
        RegexMatcherBuilder::new().build(&args.query)
    } else {
        // Literal mode is implemented as a fixed-strings matcher (design D4), the same
        // approach ripgrep uses — no user text is ever evaluated as a regex.
        RegexMatcherBuilder::new()
            .fixed_strings(true)
            .build(&args.query)
    }
    .map_err(|error| ToolError::Io(format!("regex inválida: {error}")))?;

    let max_results = args.max_results.unwrap_or(MAX_SEARCH_RESULTS);
    let mut collector = Collector {
        current_path: String::new(),
        stored: Vec::new(),
        total: 0,
        max_results: max_results.max(1),
        max_bytes: MAX_SEARCH_BYTES,
        bytes_used: 0,
        max_total: MAX_TOTAL_MATCHES,
        stopped: false,
    };

    let mut builder = ignore::WalkBuilder::new(&root);
    builder
        .standard_filters(true)
        .threads(1)
        .require_git(false)
        .sort_by_file_path(|a, b| a.cmp(b));
    let walker = builder.build();

    let mut searcher = SearcherBuilder::new().line_number(true).build();
    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return Err(ToolError::Io(error.to_string())),
        };
        let is_file = entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file());
        if !is_file {
            continue;
        }
        let path = entry.path();
        collector.current_path = display_path(&engine.workspace, path);
        searcher
            .search_path(&matcher, path, &mut collector)
            .map_err(|error| ToolError::Io(error.to_string()))?;
        if collector.stopped {
            break;
        }
    }

    Ok(SearchResult {
        matches: collector.stored,
        total: collector.total,
    })
}

struct Collector {
    current_path: String,
    stored: Vec<SearchMatch>,
    total: u64,
    max_results: usize,
    max_bytes: usize,
    bytes_used: usize,
    max_total: u64,
    stopped: bool,
}

impl Sink for Collector {
    type Error = std::io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch) -> Result<bool, Self::Error> {
        if self.stopped {
            return Ok(false);
        }
        self.total += 1;
        if self.total > self.max_total || self.bytes_used >= self.max_bytes {
            self.stopped = true;
            return Ok(false);
        }
        if self.stored.len() >= self.max_results {
            return Ok(true); // keep counting total, stop storing samples
        }
        let line = mat
            .lines()
            .next()
            .map(|l| String::from_utf8_lossy(l).into_owned())
            .map(|line| line.trim_end_matches(['\n', '\r']).to_string())
            .unwrap_or_default();
        let redacted_line = redactor::redact(&line).text;
        if self.bytes_used + redacted_line.len() > self.max_bytes {
            return Ok(true);
        }
        self.bytes_used += redacted_line.len();
        self.stored.push(SearchMatch {
            path: self.current_path.clone(),
            line_number: mat.line_number().unwrap_or(0),
            line: redacted_line,
        });
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    fn boot(dir: &Path) -> ToolEngine {
        let ws = crate::workspace::Workspace::open(dir).unwrap();
        ToolEngine::new(ws, "task_search")
    }

    #[test]
    fn finds_literal_occurrences() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.txt"),
            "linha com TODO\noutra\nmais TODO aqui",
        )
        .unwrap();
        std::fs::write(dir.path().join("b.md"), "sem nada").unwrap();
        let mut engine = boot(dir.path());

        let result = search(
            &mut engine,
            SearchArgs {
                query: "TODO".into(),
                regex: false,
                path: None,
                max_results: None,
            },
        )
        .unwrap();
        assert_eq!(result.total, 2);
        assert_eq!(result.matches.len(), 2);
        assert!(result.matches.iter().all(|m| m.path == "a.txt"));
        assert_eq!(result.matches[0].line_number, 1);
    }

    #[test]
    fn respects_gitignore() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "ignored/\n").unwrap();
        std::fs::create_dir(dir.path().join("ignored")).unwrap();
        std::fs::write(dir.path().join("ignored/secret.txt"), "ache-me\n").unwrap();
        std::fs::write(dir.path().join("visivel.txt"), "ache-me\n").unwrap();
        let mut engine = boot(dir.path());

        let result = search(
            &mut engine,
            SearchArgs {
                query: "ache-me".into(),
                regex: false,
                path: None,
                max_results: None,
            },
        )
        .unwrap();
        assert_eq!(result.total, 1);
        assert_eq!(result.matches[0].path, "visivel.txt");
    }

    #[test]
    fn searches_into_subpath() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn procura() {}").unwrap();
        std::fs::write(dir.path().join("outro.txt"), "procura").unwrap();
        let mut engine = boot(dir.path());

        let result = search(
            &mut engine,
            SearchArgs {
                query: "procura".into(),
                regex: false,
                path: Some("src".into()),
                max_results: None,
            },
        )
        .unwrap();
        assert_eq!(result.total, 1);
        assert_eq!(result.matches[0].path, "src/main.rs");
    }

    #[test]
    fn regex_mode_works() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "tom\naxb\n").unwrap();
        let mut engine = boot(dir.path());

        let result = search(
            &mut engine,
            SearchArgs {
                query: "t.m".into(),
                regex: true,
                path: None,
                max_results: None,
            },
        )
        .unwrap();
        assert_eq!(result.total, 1);
        assert_eq!(result.matches[0].line, "tom");
    }

    #[test]
    fn literal_regex_mode_is_literal() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "a.b\naxb\n").unwrap();
        let mut engine = boot(dir.path());

        let result = search(
            &mut engine,
            SearchArgs {
                query: "a.b".into(),
                regex: false,
                path: None,
                max_results: None,
            },
        )
        .unwrap();
        assert_eq!(result.total, 1);
        assert_eq!(result.matches[0].line, "a.b");
    }

    #[test]
    fn empty_query_rejected() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let err = search(
            &mut engine,
            SearchArgs {
                query: "".into(),
                regex: false,
                path: None,
                max_results: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::Io(_)));
    }

    #[test]
    fn max_results_truncates_samples_but_counts_total() {
        let dir = tempdir().unwrap();
        let mut content = String::new();
        for i in 0..20 {
            content.push_str(&format!("item {i} achado\n"));
        }
        std::fs::write(dir.path().join("many.txt"), &content).unwrap();
        let mut engine = boot(dir.path());

        let result = search(
            &mut engine,
            SearchArgs {
                query: "achado".into(),
                regex: false,
                path: None,
                max_results: Some(5),
            },
        )
        .unwrap();
        assert_eq!(result.total, 20);
        assert_eq!(result.matches.len(), 5);
    }

    #[test]
    fn redacts_secrets_in_matches() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("log.txt"), "token ghp_abcDEF123\n").unwrap();
        let mut engine = boot(dir.path());
        let result = search(
            &mut engine,
            SearchArgs {
                query: "ghp".into(),
                regex: false,
                path: None,
                max_results: None,
            },
        )
        .unwrap();
        assert_eq!(result.total, 1);
        assert!(!result.matches[0].line.contains("ghp_abcDEF123"));
        assert!(result.matches[0].line.contains("[REDIGIDO:"));
    }

    #[test]
    fn search_on_file_is_not_a_directory() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("arquivo.txt"), "x").unwrap();
        let mut engine = boot(dir.path());
        let err = search(
            &mut engine,
            SearchArgs {
                query: "x".into(),
                regex: false,
                path: Some("arquivo.txt".into()),
                max_results: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::NotADirectory));
    }
}
