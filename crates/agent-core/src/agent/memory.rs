//! Structured memory (SPEC §24.2, plan 022): small, local, user-editable, invalidable.
//!
//! No RAG, no embeddings, no automatic extraction from the model (tool/model text is
//! untrusted). Entries live under the app data directory, keyed like the repo-map cache.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::agent::storage::{StorageError, format_error, io_error, redacted_json, write_atomic};
use crate::tools::sha256_hex;
use crate::workspace::Workspace;

const MEMORY_DIR: &str = "memory";
const MEMORY_FILE: &str = "memory.json";

/// What kind of fact this is (SPEC §24.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Rule,
    Architecture,
    Preference,
    Constraint,
    Learned,
}

impl MemoryKind {
    pub fn parse_slug(value: &str) -> Option<Self> {
        match value {
            "rule" | "regra" => Some(Self::Rule),
            "architecture" | "arquitetura" => Some(Self::Architecture),
            "preference" | "preferencia" | "preferência" => Some(Self::Preference),
            "constraint" | "restricao" | "restrição" => Some(Self::Constraint),
            "learned" | "fato" => Some(Self::Learned),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Architecture => "architecture",
            Self::Preference => "preference",
            Self::Constraint => "constraint",
            Self::Learned => "learned",
        }
    }
}

/// One memory entry. `stale` is how the user invalidates without deleting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub id: String,
    pub kind: MemoryKind,
    pub text: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub stale: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemoryFile {
    #[serde(default)]
    entries: Vec<MemoryEntry>,
}

/// Reads and writes `<data_dir>/memory/<sha256(root)>/memory.json`.
#[derive(Debug, Clone)]
pub struct MemoryStore {
    path: PathBuf,
}

impl MemoryStore {
    pub fn open(data_dir: impl AsRef<Path>, workspace: &Workspace) -> Result<Self, StorageError> {
        let id = sha256_hex(workspace.root().to_string_lossy().as_bytes());
        let dir = data_dir.as_ref().join(MEMORY_DIR).join(id);
        Ok(Self {
            path: dir.join(MEMORY_FILE),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Vec<MemoryEntry> {
        fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str::<MemoryFile>(&text).ok())
            .map(|file| file.entries)
            .unwrap_or_default()
    }

    pub fn add(
        &self,
        kind: MemoryKind,
        text: impl Into<String>,
    ) -> Result<MemoryEntry, StorageError> {
        let mut entries = self.load();
        let next = entries
            .iter()
            .filter_map(|entry| {
                entry
                    .id
                    .strip_prefix("mem_")
                    .and_then(|n| n.parse::<u32>().ok())
            })
            .max()
            .unwrap_or(0)
            + 1;
        let at = crate::events::timestamp();
        let entry = MemoryEntry {
            id: format!("mem_{next}"),
            kind,
            text: text.into(),
            created_at: at.clone(),
            updated_at: at,
            stale: false,
        };
        entries.push(entry.clone());
        self.save(&entries)?;
        Ok(entry)
    }

    pub fn forget(&self, id: &str) -> Result<bool, StorageError> {
        let mut entries = self.load();
        let before = entries.len();
        entries.retain(|entry| entry.id != id);
        if entries.len() == before {
            return Ok(false);
        }
        self.save(&entries)?;
        Ok(true)
    }

    pub fn set_stale(&self, id: &str, stale: bool) -> Result<bool, StorageError> {
        let mut entries = self.load();
        let Some(entry) = entries.iter_mut().find(|entry| entry.id == id) else {
            return Ok(false);
        };
        entry.stale = stale;
        entry.updated_at = crate::events::timestamp();
        self.save(&entries)?;
        Ok(true)
    }

    fn save(&self, entries: &[MemoryEntry]) -> Result<(), StorageError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let file = MemoryFile {
            entries: entries.to_vec(),
        };
        let json = redacted_json(&file)?;
        let text = serde_json::to_string_pretty(&json).map_err(format_error)?;
        write_atomic(&self.path, &text)
    }
}

/// Picks the entries that belong in this task's prompt, clipped to `budget_chars`.
pub fn select_relevant<'a>(
    entries: &'a [MemoryEntry],
    request: &str,
    budget_chars: usize,
) -> Vec<&'a MemoryEntry> {
    let mut scored: Vec<(u32, &MemoryEntry)> = entries
        .iter()
        .filter(|entry| !entry.stale && !entry.text.trim().is_empty())
        .map(|entry| (score(entry, request), entry))
        .filter(|(value, _)| *value > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));

    let mut kept = Vec::new();
    let mut used = 0usize;
    for (_, entry) in scored {
        let size = entry.text.chars().count() + 24;
        if used.saturating_add(size) > budget_chars && !kept.is_empty() {
            continue;
        }
        if used.saturating_add(size) > budget_chars {
            break;
        }
        used += size;
        kept.push(entry);
    }
    kept
}

pub fn render(entries: &[&MemoryEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut out = String::from("Memory:");
    for entry in entries {
        out.push_str("\n- [");
        out.push_str(entry.kind.as_str());
        out.push_str("] ");
        out.push_str(entry.text.trim());
    }
    out
}

fn score(entry: &MemoryEntry, request: &str) -> u32 {
    let priority = match entry.kind {
        MemoryKind::Constraint => 8,
        MemoryKind::Rule => 6,
        MemoryKind::Architecture => 4,
        MemoryKind::Preference => 3,
        MemoryKind::Learned => 2,
    };
    let overlap = token_overlap(&entry.text, request);
    // Constraints and rules always contribute a base score so they survive a request
    // that does not repeat their words. Other kinds need at least one overlapping token.
    match entry.kind {
        MemoryKind::Constraint | MemoryKind::Rule => priority + overlap,
        _ if overlap == 0 => 0,
        _ => priority + overlap,
    }
}

fn token_overlap(text: &str, request: &str) -> u32 {
    let request_tokens = tokens(request);
    if request_tokens.is_empty() {
        return 0;
    }
    tokens(text)
        .into_iter()
        .filter(|token| request_tokens.contains(token))
        .count() as u32
}

fn tokens(text: &str) -> Vec<String> {
    text.to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| token.len() >= 3)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, tempfile::TempDir, MemoryStore, Workspace) {
        let data = tempdir().unwrap();
        let project = tempdir().unwrap();
        let workspace = Workspace::open(project.path()).unwrap();
        let store = MemoryStore::open(data.path(), &workspace).unwrap();
        (data, project, store, workspace)
    }

    #[test]
    fn add_list_forget_and_stale_round_trip() {
        let (_data, _project, store, _) = store();
        let added = store
            .add(MemoryKind::Rule, "testes deste projeto usam bun")
            .unwrap();
        assert_eq!(added.id, "mem_1");
        assert_eq!(store.load().len(), 1);

        assert!(store.set_stale("mem_1", true).unwrap());
        assert!(store.load()[0].stale);
        assert!(store.forget("mem_1").unwrap());
        assert!(store.load().is_empty());
        assert!(!store.forget("mem_1").unwrap());
    }

    #[test]
    fn stale_entries_are_not_selected() {
        let entries = vec![
            MemoryEntry {
                id: "mem_1".into(),
                kind: MemoryKind::Rule,
                text: "use bun".into(),
                created_at: "t".into(),
                updated_at: "t".into(),
                stale: true,
            },
            MemoryEntry {
                id: "mem_2".into(),
                kind: MemoryKind::Constraint,
                text: "não commitar secrets".into(),
                created_at: "t".into(),
                updated_at: "t".into(),
                stale: false,
            },
        ];
        let picked = select_relevant(&entries, "corrija o teste", 4_000);
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].id, "mem_2");
    }

    #[test]
    fn learned_facts_need_overlap_rules_do_not() {
        let entries = vec![
            MemoryEntry {
                id: "mem_1".into(),
                kind: MemoryKind::Learned,
                text: "o servidor de staging cai sexta".into(),
                created_at: "t".into(),
                updated_at: "t".into(),
                stale: false,
            },
            MemoryEntry {
                id: "mem_2".into(),
                kind: MemoryKind::Rule,
                text: "sempre rode bun test".into(),
                created_at: "t".into(),
                updated_at: "t".into(),
                stale: false,
            },
        ];
        let picked = select_relevant(&entries, "corrija a soma", 4_000);
        let ids: Vec<&str> = picked.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["mem_2"]);

        let picked = select_relevant(&entries, "o staging está fora", 4_000);
        let ids: Vec<&str> = picked.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"mem_1"), "{ids:?}");
    }

    #[test]
    fn budget_drops_the_lowest_score() {
        let entries = vec![
            MemoryEntry {
                id: "mem_1".into(),
                kind: MemoryKind::Constraint,
                text: "aaaa bbbb cccc".into(),
                created_at: "t".into(),
                updated_at: "t".into(),
                stale: false,
            },
            MemoryEntry {
                id: "mem_2".into(),
                kind: MemoryKind::Learned,
                text: "xxxx yyyy zzzz soma".into(),
                created_at: "t".into(),
                updated_at: "t".into(),
                stale: false,
            },
        ];
        let picked = select_relevant(&entries, "corrija a soma", 40);
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].id, "mem_1");
    }

    #[test]
    fn secrets_are_redacted_before_reaching_disk() {
        let (_data, _project, store, _) = store();
        let token = "ghp_abcDEF1234567890abcDEF1234567890";
        store
            .add(MemoryKind::Preference, format!("token {token}"))
            .unwrap();
        let text = fs::read_to_string(store.path()).unwrap();
        assert!(!text.contains(token), "{text}");
        assert!(text.contains("mem_1"));
    }

    #[test]
    fn render_lists_kind_and_text() {
        let entry = MemoryEntry {
            id: "mem_1".into(),
            kind: MemoryKind::Rule,
            text: "use bun".into(),
            created_at: "t".into(),
            updated_at: "t".into(),
            stale: false,
        };
        let rendered = render(&[&entry]);
        assert!(rendered.starts_with("Memory:"));
        assert!(rendered.contains("[rule] use bun"));
    }

    #[test]
    fn kind_slugs_accept_portuguese() {
        assert_eq!(MemoryKind::parse_slug("regra"), Some(MemoryKind::Rule));
        assert_eq!(
            MemoryKind::parse_slug("arquitetura"),
            Some(MemoryKind::Architecture)
        );
        assert_eq!(MemoryKind::parse_slug("nope"), None);
    }
}
