//! Preferences that outlive one run: `<data_dir>/settings.json`, next to `tasks/` (plan 015, D8).
//!
//! What is kept here is the user's own choice, never a catalogue. A model name is configuration and
//! no code depends on it (decision 0002, rule 5), so this file stores a plain string. The permission
//! mode is ASK / AUTO / FULL ACCESS; FULL ACCESS is only honoured when the OS sandbox is ready.
//!
//! Reading never fails: a missing, unreadable or corrupt file all mean "nothing chosen yet". A
//! preference is a convenience, and losing one must never keep the app from opening.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::agent::storage::{
    StorageError, data_dir, format_error, io_error, redacted_json, write_atomic,
};
use crate::permissions::PermissionMode;

const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Model the user last picked in the UI; `None` until they pick one.
    pub model: Option<String>,
    /// ASK / AUTO / FULL ACCESS (SPEC §20.4). Default ASK.
    pub permission_mode: PermissionMode,
}

/// Reads and writes `settings.json` in the app data directory. The path is built here, from the
/// data directory, so no path from outside ever reaches the filesystem.
#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, StorageError> {
        let root = data_dir.as_ref();
        fs::create_dir_all(root).map_err(io_error)?;
        Ok(Self {
            path: root.join(SETTINGS_FILE),
        })
    }

    /// Opens the store at the real data directory (or at `CD_AI_DATA_DIR`).
    pub fn open_default() -> Result<Self, StorageError> {
        Self::open(data_dir()?)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whatever was stored, or the defaults when there is nothing usable on disk.
    pub fn load(&self) -> Settings {
        fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, settings: &Settings) -> Result<(), StorageError> {
        let json = redacted_json(settings)?;
        let text = serde_json::to_string_pretty(&json).map_err(format_error)?;
        write_atomic(&self.path, &text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::storage::TaskStore;
    use tempfile::{TempDir, tempdir};

    /// Every test writes inside a tempdir; the real data directory is never touched.
    fn store() -> (TempDir, SettingsStore) {
        let dir = tempdir().unwrap();
        let store = SettingsStore::open(dir.path()).unwrap();
        (dir, store)
    }

    #[test]
    fn the_chosen_model_round_trips_on_disk() {
        let (dir, store) = store();
        store
            .save(&Settings {
                model: Some("modelo-x".to_string()),
                ..Default::default()
            })
            .unwrap();

        // A second store over the same directory is what the next run of the app sees.
        let reopened = SettingsStore::open(dir.path()).unwrap();
        assert_eq!(reopened.load().model.as_deref(), Some("modelo-x"));
    }

    #[test]
    fn settings_sit_next_to_the_tasks_directory() {
        let dir = tempdir().unwrap();
        let tasks = TaskStore::open(dir.path()).unwrap();
        let settings = SettingsStore::open(dir.path()).unwrap();
        assert_eq!(settings.path().parent(), tasks.root().parent());
        assert_eq!(
            settings.path().file_name().unwrap().to_str(),
            Some(SETTINGS_FILE)
        );
    }

    #[test]
    fn without_a_file_there_is_no_preference() {
        let (_dir, store) = store();
        assert!(!store.path().exists());
        assert_eq!(store.load(), Settings::default());
        assert_eq!(store.load().model, None);
        assert_eq!(store.load().permission_mode, PermissionMode::Ask);
    }

    #[test]
    fn a_corrupt_file_reads_as_no_preference() {
        let (_dir, store) = store();
        for content in ["{ nope", "", "[]", "{\"model\":42}"] {
            fs::write(store.path(), content).unwrap();
            assert_eq!(
                store.load(),
                Settings::default(),
                "conteúdo inválido deve virar ausência de preferência: {content}"
            );
        }
        // And writing again fixes it, instead of leaving the file broken forever.
        store
            .save(&Settings {
                model: Some("modelo-y".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(store.load().model.as_deref(), Some("modelo-y"));
    }

    #[test]
    fn a_new_choice_replaces_the_old_one_and_leaves_no_temp_file() {
        let (dir, store) = store();
        for name in ["modelo-x", "modelo-y"] {
            store
                .save(&Settings {
                    model: Some(name.to_string()),
                    ..Default::default()
                })
                .unwrap();
        }
        assert_eq!(store.load().model.as_deref(), Some("modelo-y"));

        let names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![SETTINGS_FILE.to_string()]);
    }

    #[test]
    fn secrets_are_redacted_before_reaching_disk() {
        let (_dir, store) = store();
        let token = "ghp_abcDEF1234567890abcDEF1234567890";
        store
            .save(&Settings {
                model: Some(token.to_string()),
                ..Default::default()
            })
            .unwrap();
        let text = fs::read_to_string(store.path()).unwrap();
        assert!(!text.contains(token));
    }

    #[test]
    fn permission_mode_round_trips_and_old_files_default_to_ask() {
        let (dir, store) = store();
        store
            .save(&Settings {
                permission_mode: PermissionMode::Auto,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            SettingsStore::open(dir.path())
                .unwrap()
                .load()
                .permission_mode,
            PermissionMode::Auto
        );

        fs::write(store.path(), "{\"model\":null}").unwrap();
        assert_eq!(store.load().permission_mode, PermissionMode::Ask);
    }
}
