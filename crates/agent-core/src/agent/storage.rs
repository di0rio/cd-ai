//! Where a task lives on disk (plan 015, D8): the app data directory, never the workspace.
//!
//! Layout, under `<data_dir>/tasks/<id>/`:
//!
//! - `state.json` — the whole `TaskState`, written atomically (temp + rename);
//! - `transcript.jsonl` — one `ChatMessage` per line, to rebuild the conversation on resume;
//! - `events.jsonl` — one `AgentEventMessage` per line, to replay a task in the UI.
//!
//! Two rules hold for everything here: the store builds its own paths from a validated id, so no
//! path from outside ever reaches the filesystem, and every string goes through the redactor
//! before it is written (SPEC §20.6, §24, §30).

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

use crate::agent::events::AgentEventMessage;
use crate::agent::state::{StopReason, TaskHistoryEntry, TaskState, TaskStatus, TaskSummary};
use crate::ollama::ChatMessage;
use crate::redactor;

/// Override used by the tests and by the CLI, so nothing ever writes to the real user directory.
pub const DATA_DIR_ENV: &str = "CD_AI_DATA_DIR";
const APP_DIR_NAME: &str = "cd-ai";
const TASKS_DIR: &str = "tasks";
const STATE_FILE: &str = "state.json";
const TRANSCRIPT_FILE: &str = "transcript.jsonl";
const EVENTS_FILE: &str = "events.jsonl";
/// Ids are generated here and never come from the model or the UI; the cap keeps paths sane.
const MAX_ID_LEN: usize = 64;

#[derive(Debug)]
pub enum StorageError {
    NoDataDir,
    InvalidId(String),
    NotFound(String),
    Io(String),
    Format(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDataDir => write!(
                f,
                "não foi possível descobrir o diretório de dados do app (defina {DATA_DIR_ENV})"
            ),
            Self::InvalidId(id) => write!(f, "id de tarefa inválido: {id}"),
            Self::NotFound(id) => write!(f, "tarefa não encontrada: {id}"),
            Self::Io(message) => write!(f, "erro de E/S: {message}"),
            Self::Format(message) => write!(f, "arquivo de tarefa corrompido: {message}"),
        }
    }
}

impl std::error::Error for StorageError {}

pub(super) fn io_error(error: io::Error) -> StorageError {
    StorageError::Io(error.to_string())
}

/// The app data directory (D8). Only `std`, and `CD_AI_DATA_DIR` always wins.
pub fn data_dir() -> Result<PathBuf, StorageError> {
    data_dir_from(|key| std::env::var_os(key))
}

/// Split from `data_dir` so the tests can drive the lookup without mutating process env
/// (`std::env::set_var` is unsafe in edition 2024, and env is shared by parallel tests).
fn data_dir_from(get: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, StorageError> {
    if let Some(value) = get(DATA_DIR_ENV).filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(value));
    }

    #[cfg(windows)]
    let base = get("APPDATA").map(PathBuf::from);

    #[cfg(target_os = "macos")]
    let base = get("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
    });

    #[cfg(all(not(windows), not(target_os = "macos")))]
    let base = get("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| get("HOME").map(|home| PathBuf::from(home).join(".local").join("share")));

    base.map(|base| base.join(APP_DIR_NAME))
        .ok_or(StorageError::NoDataDir)
}

/// Reads and writes tasks under `<data_dir>/tasks`. The UI and the CLI share one store.
#[derive(Debug, Clone)]
pub struct TaskStore {
    data_dir: PathBuf,
    root: PathBuf,
}

impl TaskStore {
    /// `data_dir` is the app data directory; tasks go into `<data_dir>/tasks`.
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, StorageError> {
        let data_dir = data_dir.as_ref().to_path_buf();
        let root = data_dir.join(TASKS_DIR);
        fs::create_dir_all(&root).map_err(io_error)?;
        Ok(Self { data_dir, root })
    }

    /// Opens the store at the real data directory (or at `CD_AI_DATA_DIR`).
    pub fn open_default() -> Result<Self, StorageError> {
        Self::open(data_dir()?)
    }

    /// The app data directory this store was opened on (parent of `tasks/`).
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The `tasks` directory itself.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `task_<unix_ms>_<pid hex><counter>`: unique across processes and within one process,
    /// and made only of `[a-z0-9_]` so it is always a valid path segment.
    pub fn new_task_id() -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let pid = std::process::id();
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("task_{millis}_{pid:x}{counter}")
    }

    pub fn save_state(&self, state: &TaskState) -> Result<(), StorageError> {
        let dir = self.task_dir(&state.id)?;
        fs::create_dir_all(&dir).map_err(io_error)?;
        let json = redacted_json(state)?;
        let text = serde_json::to_string_pretty(&json).map_err(format_error)?;
        write_atomic(&dir.join(STATE_FILE), &text)
    }

    pub fn load_state(&self, id: &str) -> Result<TaskState, StorageError> {
        let path = self.task_dir(id)?.join(STATE_FILE);
        let text = fs::read_to_string(&path).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => StorageError::NotFound(id.to_string()),
            _ => io_error(error),
        })?;
        serde_json::from_str(&text).map_err(format_error)
    }

    pub fn append_transcript(&self, id: &str, message: &ChatMessage) -> Result<(), StorageError> {
        self.append_json(id, TRANSCRIPT_FILE, message)
    }

    /// Messages in order. A line that cannot be parsed is skipped: a process killed mid-write can
    /// leave a partial last line, and losing one message must not make a task unresumable (D9).
    pub fn load_transcript(&self, id: &str) -> Result<Vec<ChatMessage>, StorageError> {
        Ok(self
            .load_lines(id, TRANSCRIPT_FILE)?
            .into_iter()
            .filter_map(|line| serde_json::from_str::<ChatMessage>(&line).ok())
            .collect())
    }

    pub fn append_event(&self, message: &AgentEventMessage) -> Result<(), StorageError> {
        self.append_json(&message.task_id, EVENTS_FILE, message)
    }

    /// Stored events, in order, for replaying a task. A line that cannot be parsed is skipped,
    /// like `load_transcript` does: one lost event must not make a whole task unreadable.
    pub fn load_events(&self, id: &str) -> Result<Vec<AgentEventMessage>, StorageError> {
        Ok(self
            .load_lines(id, EVENTS_FILE)?
            .into_iter()
            .filter_map(|line| serde_json::from_str::<AgentEventMessage>(&line).ok())
            .collect())
    }

    /// Tasks of one workspace, most recently updated first. Unreadable task folders are ignored:
    /// a corrupt task must not hide the rest of the list.
    pub fn list(&self, workspace: &str) -> Result<Vec<TaskSummary>, StorageError> {
        let mut summaries: Vec<(String, TaskSummary)> = Vec::new();
        for id in self.task_ids()? {
            let Ok(state) = self.load_state(&id) else {
                continue;
            };
            if state.workspace != workspace {
                continue;
            }
            summaries.push((id, state.summary()));
        }
        // `updated_at` has one-second resolution, so the id (which carries milliseconds) breaks ties.
        summaries.sort_by(|(left_id, left), (right_id, right)| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right_id.cmp(left_id))
        });
        Ok(summaries.into_iter().map(|(_, summary)| summary).collect())
    }

    /// Richer list for the history surface (plan 019): same filter and order as `list`.
    pub fn history(&self, workspace: &str) -> Result<Vec<TaskHistoryEntry>, StorageError> {
        let mut entries: Vec<(String, TaskHistoryEntry)> = Vec::new();
        for id in self.task_ids()? {
            let Ok(state) = self.load_state(&id) else {
                continue;
            };
            if state.workspace != workspace {
                continue;
            }
            entries.push((id, state.history_entry()));
        }
        entries.sort_by(|(left_id, left), (right_id, right)| {
            right
                .summary
                .updated_at
                .cmp(&left.summary.updated_at)
                .then_with(|| right_id.cmp(left_id))
        });
        Ok(entries.into_iter().map(|(_, entry)| entry).collect())
    }

    /// A task still marked `running` or `waiting_approval` means the app died on it: mark it
    /// cancelled with `Interrupted` so the user can resume it (D9). Returns the ids touched.
    pub fn recover_interrupted(&self) -> Result<Vec<String>, StorageError> {
        let mut recovered = Vec::new();
        for id in self.task_ids()? {
            let Ok(mut state) = self.load_state(&id) else {
                continue;
            };
            if !matches!(
                state.status,
                TaskStatus::Running | TaskStatus::WaitingApproval
            ) {
                continue;
            }
            state.finish(TaskStatus::Cancelled, StopReason::Interrupted);
            self.save_state(&state)?;
            recovered.push(id);
        }
        recovered.sort();
        Ok(recovered)
    }

    /// Every path this store touches is built here, from a validated id.
    fn task_dir(&self, id: &str) -> Result<PathBuf, StorageError> {
        validate_id(id)?;
        Ok(self.root.join(id))
    }

    fn task_ids(&self) -> Result<Vec<String>, StorageError> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io_error(error)),
        };
        let mut ids = Vec::new();
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if validate_id(&name).is_err() || !entry.path().is_dir() {
                continue;
            }
            ids.push(name);
        }
        ids.sort();
        Ok(ids)
    }

    fn append_json<T: Serialize>(
        &self,
        id: &str,
        file: &str,
        value: &T,
    ) -> Result<(), StorageError> {
        let dir = self.task_dir(id)?;
        fs::create_dir_all(&dir).map_err(io_error)?;
        let json = redacted_json(value)?;
        let mut line = serde_json::to_string(&json).map_err(format_error)?;
        line.push('\n');
        let mut handle = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(file))
            .map_err(io_error)?;
        handle.write_all(line.as_bytes()).map_err(io_error)
    }

    fn load_lines(&self, id: &str, file: &str) -> Result<Vec<String>, StorageError> {
        let path = self.task_dir(id)?.join(file);
        let handle = match fs::File::open(&path) {
            Ok(handle) => handle,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io_error(error)),
        };
        Ok(BufReader::new(handle)
            .lines()
            .map_while(Result::ok)
            .filter(|line| !line.trim().is_empty())
            .collect())
    }
}

/// Ids are `[a-z0-9_]` only, so no separator, drive letter or `..` can appear in a path.
fn validate_id(id: &str) -> Result<(), StorageError> {
    let valid = !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
    if valid {
        Ok(())
    } else {
        Err(StorageError::InvalidId(id.to_string()))
    }
}

/// Serializes and redacts every string in the result (D8: nothing reaches disk unredacted).
/// Walking the JSON covers all free text at once — message content, errors, report summaries,
/// argv, tool arguments — instead of a per-field list that a new field could slip past.
pub(super) fn redacted_json<T: Serialize>(value: &T) -> Result<Value, StorageError> {
    let mut json = serde_json::to_value(value).map_err(format_error)?;
    redact_in_place(&mut json);
    Ok(json)
}

fn redact_in_place(value: &mut Value) {
    match value {
        Value::String(text) => {
            let redacted = redactor::redact(text);
            if redacted.count() > 0 {
                *text = redacted.text;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact_in_place),
        Value::Object(entries) => entries.values_mut().for_each(redact_in_place),
        _ => {}
    }
}

/// Temp file in the same directory, then rename: a reader never sees a half-written state.
pub(super) fn write_atomic(path: &Path, contents: &str) -> Result<(), StorageError> {
    let temp = path.with_extension("tmp");
    fs::write(&temp, contents).map_err(io_error)?;
    fs::rename(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        io_error(error)
    })
}

pub(super) fn format_error(error: serde_json::Error) -> StorageError {
    StorageError::Format(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::events::AgentEvent;
    use crate::agent::state::{CommandRecord, TaskReport};
    use tempfile::{TempDir, tempdir};

    /// Every test writes inside a tempdir; the real data directory is never touched.
    fn store() -> (TempDir, TaskStore) {
        let dir = tempdir().unwrap();
        let store = TaskStore::open(dir.path()).unwrap();
        (dir, store)
    }

    fn state(id: &str, workspace: &str) -> TaskState {
        TaskState::new(id, workspace, "arrume o teste", "modelo-x", 16_384)
    }

    #[test]
    fn data_dir_override_wins() {
        let path = data_dir_from(|key| {
            (key == DATA_DIR_ENV).then(|| OsString::from("C:/tmp/cd-ai-dados"))
        })
        .unwrap();
        assert_eq!(path, PathBuf::from("C:/tmp/cd-ai-dados"));
    }

    #[test]
    fn data_dir_falls_back_to_the_platform_directory() {
        let path = data_dir_from(|key| match key {
            "APPDATA" => Some(OsString::from("C:/Users/x/AppData/Roaming")),
            "HOME" => Some(OsString::from("/home/x")),
            _ => None,
        })
        .unwrap();
        assert!(path.ends_with(APP_DIR_NAME), "{path:?} termina em cd-ai");
        assert!(path.parent().is_some());
    }

    #[test]
    fn data_dir_without_any_home_is_an_error() {
        let result = data_dir_from(|_| None);
        assert!(matches!(result, Err(StorageError::NoDataDir)));
    }

    #[test]
    fn new_task_ids_are_valid_and_unique() {
        let first = TaskStore::new_task_id();
        let second = TaskStore::new_task_id();
        assert_ne!(first, second);
        assert!(validate_id(&first).is_ok(), "{first}");
        assert!(first.starts_with("task_"));
    }

    #[test]
    fn state_round_trips_on_disk() {
        let (_dir, store) = store();
        let mut original = state("task_1", "C:/projeto");
        original.iterations = 3;
        original.metrics.prompt_tokens = 1_000;
        store.save_state(&original).unwrap();
        let loaded = store.load_state("task_1").unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn atomic_write_leaves_no_temp_file() {
        let (_dir, store) = store();
        let mut current = state("task_1", "C:/projeto");
        store.save_state(&current).unwrap();
        current.iterations = 1;
        store.save_state(&current).unwrap();

        let names: Vec<String> = fs::read_dir(store.root().join("task_1"))
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&STATE_FILE.to_string()));
        assert!(
            !names.iter().any(|name| name.ends_with(".tmp")),
            "nenhum .tmp deve sobrar: {names:?}"
        );
    }

    #[test]
    fn transcript_round_trips_in_order() {
        let (_dir, store) = store();
        store.save_state(&state("task_1", "C:/projeto")).unwrap();
        for (role, content) in [("user", "arrume"), ("assistant", "ok"), ("tool", "exit 0")] {
            store
                .append_transcript(
                    "task_1",
                    &ChatMessage {
                        role: role.to_string(),
                        content: content.to_string(),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        let messages = store.load_transcript("task_1").unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[2].content, "exit 0");
    }

    #[test]
    fn transcript_skips_a_partial_last_line() {
        let (_dir, store) = store();
        store
            .append_transcript(
                "task_1",
                &ChatMessage {
                    role: "user".to_string(),
                    content: "arrume".to_string(),
                    ..Default::default()
                },
            )
            .unwrap();
        let path = store.root().join("task_1").join(TRANSCRIPT_FILE);
        let mut handle = OpenOptions::new().append(true).open(&path).unwrap();
        handle.write_all(b"{\"role\":\"assis").unwrap();

        let messages = store.load_transcript("task_1").unwrap();
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn events_round_trip_as_streamed_json() {
        let (_dir, store) = store();
        let current = state("task_1", "C:/projeto");
        store
            .append_event(&AgentEventMessage::new(
                "task_1",
                0,
                AgentEvent::TaskStarted {
                    summary: current.summary(),
                },
            ))
            .unwrap();
        store
            .append_event(&AgentEventMessage::new(
                "task_1",
                1,
                AgentEvent::TaskFinished {
                    status: TaskStatus::CompletedUnvalidated,
                    stop_reason: StopReason::Finished,
                    report: TaskReport {
                        summary: "pronto".to_string(),
                        validated: false,
                        evidence: vec![CommandRecord {
                            argv: vec!["cargo".to_string(), "test".to_string()],
                            exit_code: Some(0),
                            duration_ms: 10,
                        }],
                        files_changed: vec!["src/a.rs".to_string()],
                    },
                },
            ))
            .unwrap();

        let events = store.load_events("task_1").unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0].event, AgentEvent::TaskStarted { .. }));
        assert_eq!(events[1].sequence, 1);
        let AgentEvent::TaskFinished { report, .. } = &events[1].event else {
            panic!("o segundo evento é taskFinished: {:?}", events[1].event);
        };
        assert!(!report.validated);
        assert_eq!(report.evidence[0].exit_code, Some(0));
    }

    #[test]
    fn event_lines_that_do_not_parse_are_skipped() {
        let (_dir, store) = store();
        store
            .append_event(&AgentEventMessage::new(
                "task_1",
                0,
                AgentEvent::UserMessage {
                    text: "arrume".to_string(),
                },
            ))
            .unwrap();
        let path = store.root().join("task_1").join(EVENTS_FILE);
        let mut handle = OpenOptions::new().append(true).open(&path).unwrap();
        handle.write_all(b"{\"event\":\"tas").unwrap();

        let events = store.load_events("task_1").unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn history_includes_files_and_checkpoint() {
        let (_dir, store) = store();
        let mut current = state("task_1", "C:/a");
        current.files_changed.push(crate::agent::state::FileChange {
            path: "src/a.rs".to_string(),
            hash_after: "abc".to_string(),
        });
        current.checkpoints.push(crate::agent::state::Checkpoint {
            commit: "deadbeef".to_string(),
            kind: crate::agent::state::CheckpointKind::Baseline,
            created_at: "2026-09-16T00:00:00Z".to_string(),
        });
        store.save_state(&current).unwrap();
        let history = store.history("C:/a").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].files_changed, vec!["src/a.rs"]);
        assert_eq!(history[0].checkpoint.as_deref(), Some("deadbeef"));
        assert!(!history[0].rolled_back);
    }

    #[test]
    fn list_filters_by_workspace_and_puts_recent_first() {
        let (_dir, store) = store();
        for (id, workspace) in [("task_1", "C:/a"), ("task_2", "C:/b"), ("task_3", "C:/a")] {
            store.save_state(&state(id, workspace)).unwrap();
        }
        let listed = store.list("C:/a").unwrap();
        let ids: Vec<&str> = listed.iter().map(|summary| summary.id.as_str()).collect();
        assert_eq!(ids, vec!["task_3", "task_1"]);
        assert_eq!(store.list("C:/b").unwrap().len(), 1);
        assert!(store.list("C:/nada").unwrap().is_empty());
    }

    #[test]
    fn recover_interrupted_cancels_running_tasks() {
        let (_dir, store) = store();
        store.save_state(&state("task_1", "C:/a")).unwrap();

        let mut waiting = state("task_2", "C:/a");
        waiting.status = TaskStatus::WaitingApproval;
        store.save_state(&waiting).unwrap();

        let mut done = state("task_3", "C:/a");
        done.finish(TaskStatus::CompletedUnvalidated, StopReason::Finished);
        store.save_state(&done).unwrap();

        let recovered = store.recover_interrupted().unwrap();
        assert_eq!(recovered, vec!["task_1", "task_2"]);
        for id in ["task_1", "task_2"] {
            let state = store.load_state(id).unwrap();
            assert_eq!(state.status, TaskStatus::Cancelled);
            assert_eq!(state.stop_reason, Some(StopReason::Interrupted));
        }
        let untouched = store.load_state("task_3").unwrap();
        assert_eq!(untouched.status, TaskStatus::CompletedUnvalidated);
        // Running it twice recovers nothing new.
        assert!(store.recover_interrupted().unwrap().is_empty());
    }

    #[test]
    fn invalid_ids_never_reach_the_filesystem() {
        let (dir, store) = store();
        for id in ["../x", "..", "", "Task_1", "task 1", "task/1", "c:task"] {
            assert!(
                matches!(store.load_state(id), Err(StorageError::InvalidId(_))),
                "id deveria ser recusado: {id}"
            );
            let message = ChatMessage::default();
            assert!(matches!(
                store.append_transcript(id, &message),
                Err(StorageError::InvalidId(_))
            ));
            let mut bad = state("task_1", "C:/a");
            bad.id = id.to_string();
            assert!(matches!(
                store.save_state(&bad),
                Err(StorageError::InvalidId(_))
            ));
        }
        // Nothing was created outside the tasks directory.
        let stray: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(stray, vec![TASKS_DIR.to_string()]);
    }

    #[test]
    fn secrets_are_redacted_before_reaching_disk() {
        let (_dir, store) = store();
        let token = "ghp_abcDEF1234567890abcDEF1234567890";

        let mut current = state("task_1", "C:/projeto");
        current.errors.push(format!("falhou com {token}"));
        current.commands.push(CommandRecord {
            argv: vec!["curl".to_string(), format!("--header={token}")],
            exit_code: Some(1),
            duration_ms: 5,
        });
        store.save_state(&current).unwrap();

        store
            .append_transcript(
                "task_1",
                &ChatMessage {
                    role: "user".to_string(),
                    content: format!("use o token {token}"),
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .append_event(&AgentEventMessage::new(
                "task_1",
                0,
                AgentEvent::AssistantMessage {
                    content: format!("o token é {token}"),
                },
            ))
            .unwrap();

        for file in [STATE_FILE, TRANSCRIPT_FILE, EVENTS_FILE] {
            let text = fs::read_to_string(store.root().join("task_1").join(file)).unwrap();
            assert!(!text.contains(token), "{file} não pode conter o token");
            assert!(
                text.contains("[REDIGIDO:segredo]"),
                "{file} deve mostrar a marca de redação"
            );
        }
        // Reading back keeps the redacted text, not the secret.
        let loaded = store.load_state("task_1").unwrap();
        assert!(loaded.errors[0].contains("[REDIGIDO:segredo]"));
        let messages = store.load_transcript("task_1").unwrap();
        assert!(!messages[0].content.contains("ghp_"));
    }

    #[test]
    fn missing_files_load_as_empty_and_missing_state_is_not_found() {
        let (_dir, store) = store();
        assert!(store.load_transcript("task_9").unwrap().is_empty());
        assert!(store.load_events("task_9").unwrap().is_empty());
        assert!(matches!(
            store.load_state("task_9"),
            Err(StorageError::NotFound(_))
        ));
        assert!(store.list("C:/a").unwrap().is_empty());
        assert!(store.recover_interrupted().unwrap().is_empty());
    }

    #[test]
    fn corrupt_state_is_reported_as_a_format_error() {
        let (_dir, store) = store();
        let dir = store.root().join("task_1");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(STATE_FILE), "{ nope").unwrap();
        assert!(matches!(
            store.load_state("task_1"),
            Err(StorageError::Format(_))
        ));
        // A corrupt task does not break the listing of the others.
        store.save_state(&state("task_2", "C:/a")).unwrap();
        assert_eq!(store.list("C:/a").unwrap().len(), 1);
    }
}
