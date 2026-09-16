//! Shadow git repository and safe rollback (SPEC §21, plan 019).
//!
//! The git directory lives under the app data dir. The workspace is only the work tree.
//! Invocations always pass `--git-dir` and `--work-tree` (and the matching env vars) so a
//! spawn from inside another clone — this repo, in tests — cannot discover the wrong `.git`.
//! The user's `.git` is never the GIT_DIR and is listed in `info/exclude`.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use similar::{ChangeTag, TextDiff};

use crate::agent::events::{AgentEvent, AgentEventMessage};
use crate::agent::state::{
    Checkpoint, CheckpointKind, FileChange, RollbackResult, RollbackSkip, TaskState, TaskStatus,
};
use crate::agent::storage::TaskStore;
use crate::events::timestamp;
use crate::tools::sha256_hex;
use crate::workspace::Workspace;

const SHADOW_DIR: &str = "shadow";

/// Why a snapshot or a restore failed. Display is pt-BR: it reaches the CLI and the UI.
#[derive(Debug)]
pub enum CheckpointError {
    GitMissing,
    Git(String),
    Io(String),
    WrongWorkspace { task: String, workspace: String },
    TaskRunning,
    NoBaseline,
    InvalidId(String),
    NotFound(String),
}

impl fmt::Display for CheckpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitMissing => write!(f, "git não encontrado no PATH; checkpoints indisponíveis"),
            Self::Git(message) => write!(f, "git: {message}"),
            Self::Io(message) => write!(f, "erro de E/S: {message}"),
            Self::WrongWorkspace { task, workspace } => write!(
                f,
                "a tarefa {task} pertence a outro workspace ({workspace})"
            ),
            Self::TaskRunning => {
                write!(
                    f,
                    "não é possível reverter uma tarefa que ainda está em andamento"
                )
            }
            Self::NoBaseline => write!(
                f,
                "esta tarefa não tem checkpoint; não há o que reverter com segurança"
            ),
            Self::InvalidId(id) => write!(f, "id de tarefa inválido: {id}"),
            Self::NotFound(id) => write!(f, "tarefa não encontrada: {id}"),
        }
    }
}

impl std::error::Error for CheckpointError {}

impl From<io::Error> for CheckpointError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// Git dir outside the project, work tree = the open workspace.
#[derive(Debug, Clone)]
pub struct ShadowRepo {
    git_dir: PathBuf,
    work_tree: PathBuf,
}

impl ShadowRepo {
    /// Opens (and creates, the first time) the shadow repo for this workspace.
    pub fn open(data_dir: &Path, workspace: &Workspace) -> Result<Self, CheckpointError> {
        let id = sha256_hex(workspace.root().to_string_lossy().as_bytes());
        let git_dir = data_dir.join(SHADOW_DIR).join(id);
        fs::create_dir_all(&git_dir)?;
        let repo = Self {
            git_dir,
            work_tree: workspace.root().to_path_buf(),
        };
        repo.ensure_init()?;
        Ok(repo)
    }

    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// Full snapshot of the work tree (gitignore + `info/exclude`). Returns the commit SHA.
    pub fn snapshot(&self, message: &str) -> Result<String, CheckpointError> {
        self.ensure_init()?;
        self.git(&["add", "-A"])?;
        self.commit(message)
    }

    /// Records only these workspace-relative paths (`add -f`, so gitignored files the agent
    /// touched still land in the snapshot).
    pub fn snapshot_paths(
        &self,
        paths: &[String],
        message: &str,
    ) -> Result<String, CheckpointError> {
        self.ensure_init()?;
        for path in paths {
            if !is_safe_relative(path) {
                continue;
            }
            // A missing path is not fatal: the next commit still has a SHA to hang the task on.
            let _ = self.git(&["add", "-f", "--", path]);
        }
        self.commit(message)
    }

    /// Blob at `commit:path`, or `None` when that path is not in the tree (agent created it).
    pub fn show(&self, commit: &str, path: &str) -> Result<Option<Vec<u8>>, CheckpointError> {
        if !is_safe_relative(path) {
            return Ok(None);
        }
        let spec = format!("{commit}:{path}");
        // `cat-file -e` is the existence check: `show` of a missing path is a fatal error whose
        // wording changes across git versions, and must not look like a broken shadow repo.
        match self.git(&["cat-file", "-e", &spec]) {
            Ok(_) => self.git_bytes(&["show", &spec]).map(Some),
            Err(CheckpointError::Git(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn ensure_init(&self) -> Result<(), CheckpointError> {
        if !self.git_dir.join("HEAD").exists() {
            self.git(&["init"])?;
        }
        let exclude = self.git_dir.join("info").join("exclude");
        if let Some(parent) = exclude.parent() {
            fs::create_dir_all(parent)?;
        }
        // `.git/` is the user's repo; the bulky dirs keep a first snapshot cheap on real projects.
        // Agent-touched files are force-added later, so a gitignored edit is still restorable.
        fs::write(
            exclude,
            "\
.git/\n\
.git\n\
node_modules/\n\
target/\n\
dist/\n\
.next/\n\
__pycache__/\n\
.turbo/\n\
coverage/\n",
        )?;
        Ok(())
    }

    fn commit(&self, message: &str) -> Result<String, CheckpointError> {
        self.git(&[
            "-c",
            "user.name=cd-ai",
            "-c",
            "user.email=cd-ai@local",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            message,
        ])?;
        let sha = self.git(&["rev-parse", "HEAD"])?;
        Ok(sha.trim().to_string())
    }

    fn git(&self, args: &[&str]) -> Result<String, CheckpointError> {
        let output = self.git_output(args)?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn git_bytes(&self, args: &[&str]) -> Result<Vec<u8>, CheckpointError> {
        let output = self.git_output(args)?;
        Ok(output.stdout)
    }

    fn git_output(&self, args: &[&str]) -> Result<Output, CheckpointError> {
        let mut command = Command::new("git");
        command
            .arg("--git-dir")
            .arg(&self.git_dir)
            .arg("--work-tree")
            .arg(&self.work_tree)
            .args(args)
            .current_dir(&self.work_tree)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Discovery from cwd would find *this* clone when tests run inside it.
            .env("GIT_DIR", &self.git_dir)
            .env("GIT_WORK_TREE", &self.work_tree)
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_PAGER", "cat")
            .env("GIT_OPTIONAL_LOCKS", "0");
        let output = command.output().map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => CheckpointError::GitMissing,
            _ => CheckpointError::Io(format!("não foi possível executar git: {error}")),
        })?;
        if output.status.success() {
            Ok(output)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let detail = [stderr.trim(), stdout.trim()]
                .into_iter()
                .find(|text| !text.is_empty())
                .unwrap_or("comando git falhou")
                .to_string();
            Err(CheckpointError::Git(detail))
        }
    }
}

/// Restores one finished task: only files the agent wrote, and only when the user has not
/// overwritten them since (unless `force`).
pub fn rollback_task(
    store: &TaskStore,
    workspace: &Workspace,
    task_id: &str,
    force: bool,
) -> Result<RollbackResult, CheckpointError> {
    let mut state = store.load_state(task_id).map_err(storage_error)?;
    let key = workspace.info().root;
    if state.workspace != key {
        return Err(CheckpointError::WrongWorkspace {
            task: task_id.to_string(),
            workspace: state.workspace,
        });
    }
    if matches!(
        state.status,
        TaskStatus::Running | TaskStatus::WaitingApproval
    ) {
        return Err(CheckpointError::TaskRunning);
    }

    let Some(baseline) = baseline_commit(&state) else {
        return Err(CheckpointError::NoBaseline);
    };
    let shadow = ShadowRepo::open(store.data_dir(), workspace)?;
    let mut result = RollbackResult::default();

    for change in &state.files_changed {
        match rollback_one(workspace, &shadow, &baseline, change, force)? {
            FileAction::Restored => result.restored.push(change.path.clone()),
            FileAction::AlreadyClean => result.already_clean.push(change.path.clone()),
            FileAction::Skipped(skip) => result.skipped.push(skip),
        }
    }

    state.rolled_back = true;
    state.touch();
    store.save_state(&state).map_err(storage_error)?;
    let sequence = store
        .load_events(task_id)
        .map(|events| events.len() as u64)
        .unwrap_or(0);
    let _ = store.append_event(&AgentEventMessage::new(
        task_id,
        sequence,
        AgentEvent::RollbackCompleted {
            restored: result.restored.clone(),
            skipped: result.skipped.clone(),
        },
    ));
    Ok(result)
}

enum FileAction {
    Restored,
    AlreadyClean,
    Skipped(RollbackSkip),
}

fn rollback_one(
    workspace: &Workspace,
    shadow: &ShadowRepo,
    baseline: &str,
    change: &FileChange,
    force: bool,
) -> Result<FileAction, CheckpointError> {
    if !is_safe_relative(&change.path) {
        return Ok(FileAction::Skipped(RollbackSkip {
            path: change.path.clone(),
            reason: "caminho recusado".to_string(),
            diff: String::new(),
        }));
    }
    let canonical = workspace
        .resolve(&change.path)
        .map_err(|error| CheckpointError::Io(error.to_string()))?;
    let current = read_optional(&canonical)?;
    let baseline_bytes = shadow.show(baseline, &change.path)?;

    if current.as_deref() == baseline_bytes.as_deref() {
        return Ok(FileAction::AlreadyClean);
    }

    let current_hash = current.as_ref().map(|bytes| sha256_hex(bytes));
    let still_agents = current_hash.as_ref() == Some(&change.hash_after);
    if still_agents || force {
        restore_to(&canonical, baseline_bytes.as_deref())?;
        return Ok(FileAction::Restored);
    }

    let before = bytes_as_text(baseline_bytes.as_deref());
    let after = bytes_as_text(current.as_deref());
    Ok(FileAction::Skipped(RollbackSkip {
        path: change.path.clone(),
        reason: "alterado pelo usuário depois da escrita do agente".to_string(),
        diff: unified_diff(&before, &after),
    }))
}

fn restore_to(path: &Path, baseline: Option<&[u8]>) -> Result<(), CheckpointError> {
    match baseline {
        Some(bytes) => atomic_write(path, bytes),
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(CheckpointError::Io(error.to_string())),
        },
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), CheckpointError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("cd-ai-restore.tmp");
    fs::write(&tmp, bytes).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        CheckpointError::Io(error.to_string())
    })?;
    fs::rename(&tmp, path).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        CheckpointError::Io(error.to_string())
    })
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, CheckpointError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CheckpointError::Io(error.to_string())),
    }
}

fn bytes_as_text(bytes: Option<&[u8]>) -> String {
    match bytes {
        None => String::new(),
        Some(bytes) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn unified_diff(before: &str, after: &str) -> String {
    let diff = TextDiff::from_lines(before, after);
    let mut out = String::new();
    let mut header_pending = true;
    for change in diff.iter_all_changes() {
        if change.tag() == ChangeTag::Equal {
            continue;
        }
        if header_pending {
            out.push_str("--- checkpoint\n+++ disco\n");
            header_pending = false;
        }
        let sign = match change.tag() {
            ChangeTag::Delete => '-',
            ChangeTag::Insert => '+',
            ChangeTag::Equal => ' ',
        };
        out.push(sign);
        out.push_str(change.value());
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    if out.len() > 8 * 1024 {
        out.truncate(8 * 1024);
        out.push_str("\n… diff truncado\n");
    }
    out
}

pub fn record_checkpoint(
    state: &mut TaskState,
    commit: String,
    kind: CheckpointKind,
) -> Checkpoint {
    let checkpoint = Checkpoint {
        commit,
        kind,
        created_at: timestamp(),
    };
    state.checkpoints.push(checkpoint.clone());
    checkpoint
}

pub fn baseline_commit(state: &TaskState) -> Option<String> {
    state
        .checkpoints
        .iter()
        .find(|checkpoint| checkpoint.kind == CheckpointKind::Baseline)
        .map(|checkpoint| checkpoint.commit.clone())
}

/// Workspace-relative paths only: no absolute, no `..`, no NUL. Display paths from the engine
/// already look like this; the check is the last gate before `git add` / restore.
fn is_safe_relative(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "." {
        return false;
    }
    let as_path = Path::new(trimmed);
    if as_path.is_absolute() {
        return false;
    }
    as_path.components().all(|component| {
        matches!(
            component,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    })
}

fn storage_error(error: crate::agent::storage::StorageError) -> CheckpointError {
    match error {
        crate::agent::storage::StorageError::InvalidId(id) => CheckpointError::InvalidId(id),
        crate::agent::storage::StorageError::NotFound(id) => CheckpointError::NotFound(id),
        other => CheckpointError::Io(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state::{FileChange, TaskState};
    use crate::tools::sha256_hex;
    use tempfile::tempdir;

    fn git_works() -> bool {
        Command::new("git")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn workspace_with(files: &[(&str, &str)]) -> (tempfile::TempDir, Workspace) {
        let dir = tempdir().unwrap();
        for (path, content) in files {
            let full = dir.path().join(path);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(full, content).unwrap();
        }
        let workspace = Workspace::open(dir.path()).unwrap();
        (dir, workspace)
    }

    fn change(path: &str, content: &str) -> FileChange {
        FileChange {
            path: path.to_string(),
            hash_after: sha256_hex(content.as_bytes()),
        }
    }

    fn prepare_task(
        store: &TaskStore,
        workspace: &Workspace,
        files: Vec<FileChange>,
        baseline: &str,
    ) -> String {
        let id = TaskStore::new_task_id();
        let mut state = TaskState::new(&id, workspace.info().root, "pedido", "modelo", 16_384);
        state.files_changed = files;
        state.checkpoints.push(Checkpoint {
            commit: baseline.to_string(),
            kind: CheckpointKind::Baseline,
            created_at: timestamp(),
        });
        state.finish(
            crate::agent::state::TaskStatus::CompletedUnvalidated,
            crate::agent::state::StopReason::Finished,
        );
        store.save_state(&state).unwrap();
        id
    }

    #[test]
    fn snapshot_does_not_create_a_dot_git_in_the_workspace() {
        if !git_works() {
            return;
        }
        let data = tempdir().unwrap();
        let (_ws_dir, workspace) = workspace_with(&[("a.txt", "um\n")]);
        let repo = ShadowRepo::open(data.path(), &workspace).unwrap();
        let commit = repo.snapshot("baseline").unwrap();
        assert_eq!(commit.len(), 40);
        assert!(!workspace.root().join(".git").exists());
        assert!(repo.git_dir().join("HEAD").exists());
        assert!(repo.git_dir().starts_with(data.path().join(SHADOW_DIR)));
    }

    #[test]
    fn rollback_restores_agent_file_and_keeps_unrelated_user_edits() {
        if !git_works() {
            return;
        }
        let data = tempdir().unwrap();
        let store = TaskStore::open(data.path()).unwrap();
        let (_ws_dir, workspace) = workspace_with(&[("a.txt", "orig-a\n"), ("b.txt", "orig-b\n")]);
        let shadow = ShadowRepo::open(data.path(), &workspace).unwrap();
        let baseline = shadow.snapshot("baseline").unwrap();

        fs::write(workspace.root().join("a.txt"), "agent-a\n").unwrap();
        fs::write(workspace.root().join("b.txt"), "user-b\n").unwrap();
        let id = prepare_task(
            &store,
            &workspace,
            vec![change("a.txt", "agent-a\n")],
            &baseline,
        );

        let result = rollback_task(&store, &workspace, &id, false).unwrap();
        assert_eq!(result.restored, vec!["a.txt"]);
        assert!(result.skipped.is_empty());
        assert_eq!(
            fs::read_to_string(workspace.root().join("a.txt")).unwrap(),
            "orig-a\n"
        );
        assert_eq!(
            fs::read_to_string(workspace.root().join("b.txt")).unwrap(),
            "user-b\n"
        );
        assert!(store.load_state(&id).unwrap().rolled_back);
    }

    #[test]
    fn rollback_skips_a_file_the_user_changed_after_the_agent() {
        if !git_works() {
            return;
        }
        let data = tempdir().unwrap();
        let store = TaskStore::open(data.path()).unwrap();
        let (_ws_dir, workspace) = workspace_with(&[("a.txt", "orig\n")]);
        let shadow = ShadowRepo::open(data.path(), &workspace).unwrap();
        let baseline = shadow.snapshot("baseline").unwrap();

        fs::write(workspace.root().join("a.txt"), "agent\n").unwrap();
        let id = prepare_task(
            &store,
            &workspace,
            vec![change("a.txt", "agent\n")],
            &baseline,
        );
        fs::write(workspace.root().join("a.txt"), "user-after\n").unwrap();

        let result = rollback_task(&store, &workspace, &id, false).unwrap();
        assert!(result.restored.is_empty());
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].path, "a.txt");
        assert!(result.skipped[0].diff.contains("user-after"));
        assert_eq!(
            fs::read_to_string(workspace.root().join("a.txt")).unwrap(),
            "user-after\n"
        );

        let forced = rollback_task(&store, &workspace, &id, true).unwrap();
        assert_eq!(forced.restored, vec!["a.txt"]);
        assert_eq!(
            fs::read_to_string(workspace.root().join("a.txt")).unwrap(),
            "orig\n"
        );
    }

    #[test]
    fn rollback_deletes_a_file_the_agent_created_unless_the_user_edited_it() {
        if !git_works() {
            return;
        }
        let data = tempdir().unwrap();
        let store = TaskStore::open(data.path()).unwrap();
        let (_ws_dir, workspace) = workspace_with(&[("keep.txt", "keep\n")]);
        let shadow = ShadowRepo::open(data.path(), &workspace).unwrap();
        let baseline = shadow.snapshot("baseline").unwrap();

        fs::write(workspace.root().join("novo.txt"), "criado\n").unwrap();
        let id = prepare_task(
            &store,
            &workspace,
            vec![change("novo.txt", "criado\n")],
            &baseline,
        );

        let result = rollback_task(&store, &workspace, &id, false).unwrap();
        assert_eq!(result.restored, vec!["novo.txt"]);
        assert!(!workspace.root().join("novo.txt").exists());
        assert_eq!(
            fs::read_to_string(workspace.root().join("keep.txt")).unwrap(),
            "keep\n"
        );

        fs::write(workspace.root().join("outro.txt"), "criado\n").unwrap();
        let other = prepare_task(
            &store,
            &workspace,
            vec![change("outro.txt", "criado\n")],
            &baseline,
        );
        fs::write(workspace.root().join("outro.txt"), "user-edit\n").unwrap();
        let skipped = rollback_task(&store, &workspace, &other, false).unwrap();
        assert!(skipped.restored.is_empty());
        assert_eq!(skipped.skipped[0].path, "outro.txt");
        assert_eq!(
            fs::read_to_string(workspace.root().join("outro.txt")).unwrap(),
            "user-edit\n"
        );
    }

    #[test]
    fn rollback_does_not_touch_the_users_git() {
        if !git_works() {
            return;
        }
        let data = tempdir().unwrap();
        let store = TaskStore::open(data.path()).unwrap();
        let (_ws_dir, workspace) = workspace_with(&[("a.txt", "orig\n")]);

        let init = Command::new("git")
            .args(["init"])
            .current_dir(workspace.root())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap();
        assert!(
            init.status.success(),
            "{}",
            String::from_utf8_lossy(&init.stderr)
        );
        let commit_user = Command::new("git")
            .args([
                "-c",
                "user.name=dev",
                "-c",
                "user.email=dev@local",
                "-c",
                "commit.gpgsign=false",
                "add",
                "a.txt",
            ])
            .current_dir(workspace.root())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap();
        assert!(commit_user.status.success());
        let commit_user = Command::new("git")
            .args([
                "-c",
                "user.name=dev",
                "-c",
                "user.email=dev@local",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "wip",
            ])
            .current_dir(workspace.root())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap();
        assert!(
            commit_user.status.success(),
            "{}",
            String::from_utf8_lossy(&commit_user.stderr)
        );
        let head_before = fs::read_to_string(workspace.root().join(".git/HEAD")).unwrap();
        let log_before = Command::new("git")
            .args(["log", "--oneline"])
            .current_dir(workspace.root())
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap();
        let log_before = String::from_utf8_lossy(&log_before.stdout).into_owned();

        let shadow = ShadowRepo::open(data.path(), &workspace).unwrap();
        let baseline = shadow.snapshot("baseline").unwrap();
        fs::write(workspace.root().join("a.txt"), "agent\n").unwrap();
        let id = prepare_task(
            &store,
            &workspace,
            vec![change("a.txt", "agent\n")],
            &baseline,
        );
        rollback_task(&store, &workspace, &id, false).unwrap();

        let head_after = fs::read_to_string(workspace.root().join(".git/HEAD")).unwrap();
        let log_after = Command::new("git")
            .args(["log", "--oneline"])
            .current_dir(workspace.root())
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap();
        assert_eq!(head_before, head_after);
        assert_eq!(log_before, String::from_utf8_lossy(&log_after.stdout));
        assert_eq!(
            fs::read_to_string(workspace.root().join("a.txt")).unwrap(),
            "orig\n"
        );
        assert!(!shadow.git_dir().starts_with(workspace.root()));
    }

    #[test]
    fn rollback_of_a_running_task_is_refused() {
        if !git_works() {
            return;
        }
        let data = tempdir().unwrap();
        let store = TaskStore::open(data.path()).unwrap();
        let (_ws_dir, workspace) = workspace_with(&[("a.txt", "x\n")]);
        let mut state = TaskState::new("task_1", workspace.info().root, "pedido", "m", 16_384);
        store.save_state(&state).unwrap();
        let error = rollback_task(&store, &workspace, "task_1", false).unwrap_err();
        assert!(matches!(error, CheckpointError::TaskRunning));

        state.status = TaskStatus::CompletedUnvalidated;
        store.save_state(&state).unwrap();
        let error = rollback_task(&store, &workspace, "task_1", false).unwrap_err();
        assert!(matches!(error, CheckpointError::NoBaseline));
    }

    #[test]
    fn a_second_rollback_is_already_clean() {
        if !git_works() {
            return;
        }
        let data = tempdir().unwrap();
        let store = TaskStore::open(data.path()).unwrap();
        let (_ws_dir, workspace) = workspace_with(&[("a.txt", "orig\n")]);
        let shadow = ShadowRepo::open(data.path(), &workspace).unwrap();
        let baseline = shadow.snapshot("baseline").unwrap();
        fs::write(workspace.root().join("a.txt"), "agent\n").unwrap();
        let id = prepare_task(
            &store,
            &workspace,
            vec![change("a.txt", "agent\n")],
            &baseline,
        );
        rollback_task(&store, &workspace, &id, false).unwrap();
        let second = rollback_task(&store, &workspace, &id, false).unwrap();
        assert!(second.restored.is_empty());
        assert_eq!(second.already_clean, vec!["a.txt"]);
        assert_eq!(
            fs::read_to_string(workspace.root().join("a.txt")).unwrap(),
            "orig\n"
        );
    }

    #[test]
    fn unsafe_paths_are_never_passed_to_git() {
        assert!(!is_safe_relative("../secret"));
        assert!(!is_safe_relative("/etc/passwd"));
        assert!(!is_safe_relative(""));
        assert!(is_safe_relative("src/a.rs"));
        assert!(is_safe_relative("a.txt"));
    }
}
