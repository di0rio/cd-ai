//! Tree-sitter language selection shared by the editor, the Verifier and the repo map
//! (plan 020). One match on the extension, one pair of grammars: TypeScript and Rust.
//!
//! Lives at the crate root so `tools::edit` and `agent::repo_map` can both use it without
//! a module cycle (the agent already depends on the tools).

use std::path::Path;

use tree_sitter::Language;

/// Grammar for a workspace path, when the file is one the agent can parse.
///
/// `tsx` uses the TSX grammar; `ts`/`js`/`jsx` use TypeScript (same split as the Fase 4
/// parse check). Anything else is `None`: the caller skips, it is not an error.
pub fn language_for_path(path: &Path) -> Option<(&'static str, Language)> {
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    match extension {
        "tsx" => Some((
            "typescript",
            Language::new(tree_sitter_typescript::LANGUAGE_TSX),
        )),
        "rs" => Some(("rust", Language::new(tree_sitter_rust::LANGUAGE))),
        "ts" | "js" | "jsx" => Some((
            "typescript",
            Language::new(tree_sitter_typescript::LANGUAGE_TYPESCRIPT),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn known_extensions_pick_a_grammar_and_the_rest_are_skipped() {
        assert!(language_for_path(Path::new("src/a.ts")).is_some());
        assert!(language_for_path(Path::new("src/a.tsx")).is_some());
        assert!(language_for_path(Path::new("src/a.js")).is_some());
        assert!(language_for_path(Path::new("src/a.jsx")).is_some());
        assert!(language_for_path(Path::new("src/a.rs")).is_some());
        assert!(language_for_path(Path::new("README.md")).is_none());
        assert!(language_for_path(Path::new("Cargo.toml")).is_none());
    }
}
