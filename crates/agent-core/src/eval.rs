//! Local eval harness (SPEC §26, plan 016): fixture copy → `run_task` → automatic check → report.
//!
//! The runner never touches the user's project. Each task works on a disposable copy of a fixture,
//! and the eval responder grants every approval (D1). `cd-ai task` stays interactive.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::agent::events::{AgentEvent, AgentEventMessage};
use crate::agent::model::{ChatModel, ModelError, ModelReply, OllamaModel, ScriptedModel};
use crate::agent::runner::{TaskContext, TaskStart, run_task};
use crate::agent::state::{AgentLimits, StopReason, TaskStatus};
use crate::agent::storage::TaskStore;
use crate::agent::tool_calls::TOOL_NAMES;
use crate::events::{ToolEvent, ToolEventMessage};
use crate::ollama::{ModelFunctionCall, ModelToolCall, OllamaClient};
use crate::permissions::{ApprovalAction, ApprovalRequest, ApprovalResponse};
use crate::tool_call::parse_text_tool_calls;
use crate::tools::cancel::CancelToken;
use crate::tools::{RunCommandArgs, ToolEngine, ToolOutput, ToolRequest};
use crate::workspace::{Workspace, WorkspaceError};

/// Default per-task wall budget for an eval (plan 016, D9). Not the product's 45-minute limit.
const DEFAULT_TASK_TIMEOUT_MS: u64 = 600_000;
const DEFAULT_CHECK_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_MAX_ITERATIONS: u32 = 15;

/// How the suite is driven: a recorded script, or a live Ollama model.
pub enum EvalDriver {
    Scripted,
    Ollama {
        client: OllamaClient,
        runtime: tokio::runtime::Handle,
        turn_timeout: std::time::Duration,
    },
}

/// Inputs for one suite run.
pub struct EvalOptions {
    pub suite_dir: PathBuf,
    /// When `None`, the report is written under `evals/results/<stamp>-<model>.json`.
    pub out_path: Option<PathBuf>,
    pub task_filter: Option<String>,
    /// Name stored in the report (`scripted` or the Ollama tag).
    pub model_name: String,
    pub num_ctx: u32,
    pub driver: EvalDriver,
    pub cancel: CancelToken,
}

/// One task file under `evals/tasks/`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalTask {
    pub id: String,
    pub fixture: String,
    pub prompt: String,
    #[serde(default = "default_task_timeout_ms")]
    pub timeout_ms: u64,
    pub max_iterations: Option<u32>,
    pub check: EvalCheck,
    #[serde(default)]
    pub script: Vec<ScriptTurn>,
}

fn default_task_timeout_ms() -> u64 {
    DEFAULT_TASK_TIMEOUT_MS
}

/// Command the harness runs after the agent, on the copied fixture (SPEC §26).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalCheck {
    pub argv: Vec<String>,
    #[serde(default)]
    pub expect_exit: i32,
    #[serde(default = "default_check_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_check_timeout_ms() -> u64 {
    DEFAULT_CHECK_TIMEOUT_MS
}

/// One scripted model turn. `calls` wins when present; otherwise the turn is plain text.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptTurn {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub calls: Vec<ScriptCall>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScriptCall {
    pub name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

/// Full suite report, written as JSON so versions can be compared (SPEC §26).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalReport {
    pub schema_version: u32,
    pub recorded_at: String,
    pub git_head: Option<String>,
    pub model: String,
    pub suite: String,
    pub success_rate: f64,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub duration_ms: u64,
    pub tasks: Vec<EvalTaskResult>,
}

/// One task inside [`EvalReport`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalTaskResult {
    pub id: String,
    pub success: bool,
    pub check_exit_code: Option<i32>,
    pub agent_status: String,
    pub stop_reason: Option<StopReason>,
    pub iterations: u32,
    pub retries: u32,
    pub prompt_tokens: u64,
    pub gen_tokens: u64,
    pub duration_ms: u64,
    pub model_ms: u64,
    pub tool_ms: u64,
    pub tool_call_format_failures: u32,
    pub native_tool_call_turns: u32,
    pub text_tool_call_turns: u32,
    pub rejected_edits: u32,
    pub files_changed: Vec<String>,
    pub error: Option<String>,
}

impl EvalReport {
    /// Every selected task passed its automatic check.
    pub fn all_passed(&self) -> bool {
        self.failed == 0 && self.skipped == 0 && self.passed > 0
    }
}

/// Why a suite could not even start (a task that runs and fails is a row, not this).
#[derive(Debug)]
pub enum EvalError {
    Io(io::Error),
    Json { path: String, message: String },
    Suite(String),
    Workspace(WorkspaceError),
    Storage(crate::agent::storage::StorageError),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "erro de E/S: {error}"),
            Self::Json { path, message } => write!(f, "JSON inválido em {path}: {message}"),
            Self::Suite(message) => write!(f, "{message}"),
            Self::Workspace(error) => write!(f, "{error}"),
            Self::Storage(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for EvalError {}

impl From<io::Error> for EvalError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<WorkspaceError> for EvalError {
    fn from(error: WorkspaceError) -> Self {
        Self::Workspace(error)
    }
}

impl From<crate::agent::storage::StorageError> for EvalError {
    fn from(error: crate::agent::storage::StorageError) -> Self {
        Self::Storage(error)
    }
}

/// Loads every `*.json` in `suite/tasks`, sorted by file name.
pub fn load_suite(suite_dir: &Path) -> Result<Vec<EvalTask>, EvalError> {
    let tasks_dir = suite_dir.join("tasks");
    if !tasks_dir.is_dir() {
        return Err(EvalError::Suite(format!(
            "pasta de tarefas não encontrada: {}",
            tasks_dir.display()
        )));
    }
    let mut entries: Vec<_> = fs::read_dir(&tasks_dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    let mut tasks = Vec::new();
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let text = fs::read_to_string(&path)?;
        let task: EvalTask = serde_json::from_str(&text).map_err(|error| EvalError::Json {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
        tasks.push(task);
    }
    if tasks.is_empty() {
        return Err(EvalError::Suite(format!(
            "nenhuma tarefa em {}",
            tasks_dir.display()
        )));
    }
    Ok(tasks)
}

/// Where a report lands when the caller does not pick a path.
pub fn default_report_path(suite_dir: &Path, model: &str) -> PathBuf {
    let stamp = crate::events::timestamp().replace(':', "");
    suite_dir
        .join("results")
        .join(format!("{}-{}.json", stamp, sanitize_filename(model)))
}

/// Runs the suite, writes the JSON report, and returns it.
pub fn run_suite(
    options: EvalOptions,
    mut on_event: impl FnMut(&EvalTask, &AgentEventMessage),
) -> Result<EvalReport, EvalError> {
    let started = Instant::now();
    let mut tasks = load_suite(&options.suite_dir)?;
    if let Some(id) = &options.task_filter {
        let original = tasks.len();
        tasks.retain(|task| &task.id == id);
        if tasks.is_empty() {
            return Err(EvalError::Suite(format!(
                "tarefa {id} não encontrada (suite tem {original} tarefa(s))"
            )));
        }
    }

    let data = TempBox::new("data")?;
    let store = TaskStore::open(&data.path)?;

    let mut results = Vec::new();
    for task in &tasks {
        if options.cancel.is_cancelled() {
            results.push(skipped_result(&task.id, "cancelado"));
            continue;
        }
        results.push(run_one(&options, &store, task, &mut on_event));
    }

    let passed = results.iter().filter(|row| row.success).count() as u32;
    let skipped = results
        .iter()
        .filter(|row| row.error.as_deref() == Some("cancelado"))
        .count() as u32;
    let failed = results.len() as u32 - passed - skipped;
    let scored = passed + failed;
    let success_rate = if scored == 0 {
        0.0
    } else {
        f64::from(passed) / f64::from(scored)
    };

    let report = EvalReport {
        schema_version: 1,
        recorded_at: crate::events::timestamp(),
        git_head: git_head(&options.suite_dir),
        model: options.model_name.clone(),
        suite: options.suite_dir.display().to_string(),
        success_rate,
        passed,
        failed,
        skipped,
        duration_ms: started.elapsed().as_millis() as u64,
        tasks: results,
    };

    let out = options
        .out_path
        .unwrap_or_else(|| default_report_path(&options.suite_dir, &options.model_name));
    write_report(&report, &out)?;
    Ok(report)
}

/// Writes `report` as pretty JSON, creating parent directories.
pub fn write_report(report: &EvalReport, path: &Path) -> Result<(), EvalError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(report)
        .map_err(|error| EvalError::Suite(error.to_string()))?;
    fs::write(path, text)?;
    Ok(())
}

fn run_one(
    options: &EvalOptions,
    store: &TaskStore,
    task: &EvalTask,
    on_event: &mut dyn FnMut(&EvalTask, &AgentEventMessage),
) -> EvalTaskResult {
    let started = Instant::now();
    match run_one_inner(options, store, task, on_event) {
        Ok(mut result) => {
            result.duration_ms = started.elapsed().as_millis() as u64;
            result
        }
        Err(error) => EvalTaskResult {
            id: task.id.clone(),
            success: false,
            check_exit_code: None,
            agent_status: String::new(),
            stop_reason: None,
            iterations: 0,
            retries: 0,
            prompt_tokens: 0,
            gen_tokens: 0,
            duration_ms: started.elapsed().as_millis() as u64,
            model_ms: 0,
            tool_ms: 0,
            tool_call_format_failures: 0,
            native_tool_call_turns: 0,
            text_tool_call_turns: 0,
            rejected_edits: 0,
            files_changed: Vec::new(),
            error: Some(error.to_string()),
        },
    }
}

fn run_one_inner(
    options: &EvalOptions,
    store: &TaskStore,
    task: &EvalTask,
    on_event: &mut dyn FnMut(&EvalTask, &AgentEventMessage),
) -> Result<EvalTaskResult, EvalError> {
    let fixture = options.suite_dir.join("fixtures").join(&task.fixture);
    if !fixture.is_dir() {
        return Err(EvalError::Suite(format!(
            "fixture não encontrada: {}",
            fixture.display()
        )));
    }

    let copy = TempBox::new(&task.id)?;
    copy_dir(&fixture, &copy.path)?;
    let workspace = Workspace::open(&copy.path)?;

    let mut boxed: Box<dyn ChatModel> = match &options.driver {
        EvalDriver::Scripted => {
            if task.script.is_empty() {
                return Err(EvalError::Suite(format!(
                    "tarefa {} não tem script; --scripted exige o campo script",
                    task.id
                )));
            }
            Box::new(ScriptedModel::once(script_to_replies(&task.script)))
        }
        EvalDriver::Ollama {
            client,
            runtime,
            turn_timeout,
        } => Box::new(OllamaModel::new(
            client.clone(),
            runtime.clone(),
            options.model_name.clone(),
            options.num_ctx,
            *turn_timeout,
        )),
    };

    let mut counter = FormatCounter {
        inner: boxed.as_mut(),
        native_turns: 0,
        text_turns: 0,
    };

    let mut ctx = TaskContext::new(store, workspace.clone());
    let limits = AgentLimits {
        task_timeout_ms: task.timeout_ms,
        max_iterations: task.max_iterations.unwrap_or(DEFAULT_MAX_ITERATIONS),
        ..AgentLimits::default()
    };
    ctx.limits = limits;
    ctx.cancel = options.cancel.clone();

    let start = TaskStart::New {
        request: task.prompt.clone(),
        model: options.model_name.clone(),
        num_ctx: options.num_ctx,
        continues: None,
    };

    let mut events = Vec::new();
    let mut sink = |message: AgentEventMessage| {
        on_event(task, &message);
        events.push(message);
    };
    let mut responder = |_: ApprovalRequest| ApprovalResponse::Granted;
    let state = run_task(ctx, &mut counter, start, &mut sink, &mut responder);

    let (format_failures, rejected_edits) = tally_events(&events);
    let (check_exit, check_error) = run_check(&workspace, &task.check)?;
    let check_ok = check_exit == Some(task.check.expect_exit);

    Ok(EvalTaskResult {
        id: task.id.clone(),
        success: check_ok,
        check_exit_code: check_exit,
        agent_status: status_slug(state.status),
        stop_reason: state.stop_reason.clone(),
        iterations: state.iterations,
        retries: state.retries,
        prompt_tokens: state.metrics.prompt_tokens,
        gen_tokens: state.metrics.gen_tokens,
        duration_ms: 0,
        model_ms: state.metrics.model_ms,
        tool_ms: state.metrics.tool_ms,
        tool_call_format_failures: format_failures,
        native_tool_call_turns: counter.native_turns,
        text_tool_call_turns: counter.text_turns,
        rejected_edits,
        files_changed: state
            .files_changed
            .iter()
            .map(|change| change.path.clone())
            .collect(),
        error: check_error.filter(|_| !check_ok),
    })
}

fn skipped_result(id: &str, reason: &str) -> EvalTaskResult {
    EvalTaskResult {
        id: id.to_string(),
        success: false,
        check_exit_code: None,
        agent_status: status_slug(TaskStatus::Cancelled),
        stop_reason: Some(StopReason::Cancelled),
        iterations: 0,
        retries: 0,
        prompt_tokens: 0,
        gen_tokens: 0,
        duration_ms: 0,
        model_ms: 0,
        tool_ms: 0,
        tool_call_format_failures: 0,
        native_tool_call_turns: 0,
        text_tool_call_turns: 0,
        rejected_edits: 0,
        files_changed: Vec::new(),
        error: Some(reason.to_string()),
    }
}

fn status_slug(status: TaskStatus) -> String {
    match status {
        TaskStatus::Running => "running".to_string(),
        TaskStatus::WaitingApproval => "waiting_approval".to_string(),
        TaskStatus::CompletedUnvalidated => "completed_unvalidated".to_string(),
        TaskStatus::Failed => "failed".to_string(),
        TaskStatus::Cancelled => "cancelled".to_string(),
    }
}

fn run_check(
    workspace: &Workspace,
    check: &EvalCheck,
) -> Result<(Option<i32>, Option<String>), EvalError> {
    if check.argv.is_empty() {
        return Err(EvalError::Suite("check.argv vazio".to_string()));
    }
    let mut engine = ToolEngine::new(workspace.clone(), "eval-check");
    let mut responder = |_: ApprovalRequest| ApprovalResponse::Granted;
    let mut sink = |_: ToolEventMessage| {};
    let outcome = engine.run_tool(
        ToolRequest::RunCommand(RunCommandArgs {
            argv: check.argv.clone(),
            cwd: None,
            timeout_ms: Some(check.timeout_ms),
        }),
        &mut sink,
        &mut responder,
    );
    match outcome.data {
        Some(ToolOutput::RunCommand(result)) => {
            let error = if result.exit_code == Some(check.expect_exit) {
                None
            } else {
                let output = result.output.trim();
                if output.is_empty() {
                    Some(format!(
                        "check saiu com {:?} (esperado {})",
                        result.exit_code, check.expect_exit
                    ))
                } else {
                    Some(output.chars().take(500).collect())
                }
            };
            Ok((result.exit_code, error))
        }
        None => {
            let message = outcome
                .error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "check sem resultado".to_string());
            Ok((None, Some(message)))
        }
        Some(_) => Ok((None, Some("check não foi um comando".to_string()))),
    }
}

/// Mapping failures: `ToolCallRequested` then `ToolCallFinished` without a `ToolStarted`.
/// Rejected edits: `ApprovalDenied` after an edit/write `ApprovalRequired`.
pub fn tally_events(events: &[AgentEventMessage]) -> (u32, u32) {
    let mut format_failures = 0;
    let mut rejected_edits = 0;
    let mut awaiting_engine = false;
    let mut pending_edit = false;
    for message in events {
        match &message.event {
            AgentEvent::ToolCallRequested { .. } => awaiting_engine = true,
            AgentEvent::Tool(ToolEvent::ToolStarted { .. }) => awaiting_engine = false,
            AgentEvent::ToolCallFinished { ok, .. } => {
                if awaiting_engine && !*ok {
                    format_failures += 1;
                }
                awaiting_engine = false;
            }
            AgentEvent::Tool(ToolEvent::ApprovalRequired { action, .. }) => {
                pending_edit = matches!(
                    action,
                    ApprovalAction::EditFile { .. } | ApprovalAction::WriteFile { .. }
                );
            }
            AgentEvent::Tool(ToolEvent::ApprovalDenied { .. }) => {
                if pending_edit {
                    rejected_edits += 1;
                }
                pending_edit = false;
            }
            AgentEvent::Tool(ToolEvent::ApprovalGranted { .. }) => pending_edit = false,
            _ => {}
        }
    }
    (format_failures, rejected_edits)
}

fn script_to_replies(script: &[ScriptTurn]) -> Vec<Result<ModelReply, ModelError>> {
    script
        .iter()
        .map(|turn| {
            if turn.calls.is_empty() {
                ScriptedModel::text(turn.text.as_deref().unwrap_or("pronto"))
            } else {
                Ok(ModelReply {
                    content: turn.text.clone().unwrap_or_default(),
                    tool_calls: turn
                        .calls
                        .iter()
                        .map(|call| ModelToolCall {
                            function: ModelFunctionCall {
                                name: call.name.clone(),
                                arguments: call.arguments.clone(),
                            },
                        })
                        .collect(),
                    gen_tokens: 4,
                    ..Default::default()
                })
            }
        })
        .collect()
}

struct FormatCounter<'a> {
    inner: &'a mut dyn ChatModel,
    native_turns: u32,
    text_turns: u32,
}

impl ChatModel for FormatCounter<'_> {
    fn turn(
        &mut self,
        messages: &[crate::ollama::ChatMessage],
        tools: &[crate::ollama::ToolSpec],
        on_event: &mut dyn FnMut(crate::ollama::ChatEvent),
        cancel: &CancelToken,
    ) -> Result<ModelReply, ModelError> {
        let reply = self.inner.turn(messages, tools, on_event, cancel)?;
        if !reply.tool_calls.is_empty() {
            self.native_turns += 1;
        } else if !parse_text_tool_calls(&reply.content, &TOOL_NAMES)
            .calls
            .is_empty()
        {
            self.text_turns += 1;
        }
        Ok(reply)
    }
}

fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "node_modules" || name == "target" || name == ".git" {
            continue;
        }
        let meta = entry.metadata()?;
        if meta.file_type().is_symlink() {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if meta.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "model".to_string()
    } else {
        cleaned
    }
}

fn git_head(dir: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Disposable directory under the system temp dir. Removed on drop.
struct TempBox {
    path: PathBuf,
}

impl TempBox {
    fn new(label: &str) -> io::Result<Self> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let safe: String = label
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        let path =
            std::env::temp_dir().join(format!("cd-ai-eval-{safe}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }
}

impl Drop for TempBox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::events::AgentEventMessage;
    use crate::events::timestamp;
    use tempfile::tempdir;

    fn evals_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evals")
    }

    fn silent(_: &EvalTask, _: &AgentEventMessage) {}

    fn options(suite: PathBuf, filter: Option<&str>, out: PathBuf) -> EvalOptions {
        EvalOptions {
            suite_dir: suite,
            out_path: Some(out),
            task_filter: filter.map(str::to_string),
            model_name: "scripted".to_string(),
            num_ctx: 16_384,
            driver: EvalDriver::Scripted,
            cancel: CancelToken::default(),
        }
    }

    #[test]
    fn load_suite_reads_the_three_baseline_tasks() {
        let tasks = load_suite(&evals_dir()).expect("suite de evals");
        let ids: Vec<_> = tasks.iter().map(|task| task.id.as_str()).collect();
        assert!(ids.contains(&"soma"), "{ids:?}");
        assert!(ids.contains(&"greet"), "{ids:?}");
        assert!(ids.contains(&"dobro"), "{ids:?}");
        for task in &tasks {
            assert!(!task.prompt.is_empty(), "{}", task.id);
            assert!(!task.check.argv.is_empty(), "{}", task.id);
            assert!(!task.script.is_empty(), "{}", task.id);
        }
    }

    #[test]
    fn scripted_soma_passes_its_check() {
        let out = tempdir().unwrap();
        let report = run_suite(
            options(evals_dir(), Some("soma"), out.path().join("soma.json")),
            silent,
        )
        .expect("eval soma");
        assert_eq!(report.passed, 1, "{:?}", report.tasks);
        assert_eq!(report.failed, 0);
        assert!(report.all_passed());
        assert_eq!(report.success_rate, 1.0);
        let soma = &report.tasks[0];
        assert!(soma.success);
        assert_eq!(soma.check_exit_code, Some(0));
        assert!(
            soma.files_changed
                .iter()
                .any(|path| path.contains("soma.ts"))
        );
        assert_eq!(soma.tool_call_format_failures, 0);
        assert_eq!(soma.rejected_edits, 0);
        assert!(soma.native_tool_call_turns > 0);
    }

    #[test]
    fn scripted_baseline_suite_all_three_pass() {
        let out = tempdir().unwrap();
        let report = run_suite(
            options(evals_dir(), None, out.path().join("suite.json")),
            silent,
        )
        .expect("eval suite");
        assert_eq!(report.passed, 3, "{:?}", report.tasks);
        assert_eq!(report.failed, 0);
        assert!(report.all_passed());
        assert_eq!(report.success_rate, 1.0);
    }

    #[test]
    fn an_agent_that_does_not_edit_fails_the_check() {
        let suite = tempdir().unwrap();
        let fixture_src = evals_dir().join("fixtures/soma");
        let fixture_dst = suite.path().join("fixtures/soma");
        copy_dir(&fixture_src, &fixture_dst).unwrap();
        fs::create_dir_all(suite.path().join("tasks")).unwrap();
        fs::write(
            suite.path().join("tasks/noop.json"),
            r#"{
              "id": "noop",
              "fixture": "soma",
              "prompt": "não mexa em nada",
              "timeoutMs": 60000,
              "check": { "argv": ["bun", "test"], "expectExit": 0, "timeoutMs": 60000 },
              "script": [{ "text": "está tudo certo" }]
            }"#,
        )
        .unwrap();
        let report = run_suite(
            options(
                suite.path().to_path_buf(),
                None,
                suite.path().join("out.json"),
            ),
            silent,
        )
        .expect("eval noop");
        assert_eq!(report.passed, 0);
        assert_eq!(report.failed, 1);
        assert!(!report.tasks[0].success);
        assert_ne!(report.tasks[0].check_exit_code, Some(0));
    }

    #[test]
    fn unknown_task_filter_is_an_error() {
        let out = tempdir().unwrap();
        let error = run_suite(
            options(evals_dir(), Some("nao-existe"), out.path().join("x.json")),
            silent,
        )
        .unwrap_err();
        assert!(error.to_string().contains("nao-existe"), "{error}");
    }

    #[test]
    fn tally_counts_a_mapping_failure_without_tool_started() {
        let events = vec![
            AgentEventMessage {
                task_id: "t".to_string(),
                sequence: 1,
                at: timestamp(),
                event: AgentEvent::ToolCallRequested {
                    tool: "edit_file".to_string(),
                    input: serde_json::json!({}),
                },
            },
            AgentEventMessage {
                task_id: "t".to_string(),
                sequence: 2,
                at: timestamp(),
                event: AgentEvent::ToolCallFinished {
                    tool: "edit_file".to_string(),
                    ok: false,
                    detail: "faltou old_text".to_string(),
                    output: None,
                },
            },
        ];
        let (failures, rejected) = tally_events(&events);
        assert_eq!(failures, 1);
        assert_eq!(rejected, 0);
    }

    #[test]
    fn tally_counts_denied_edits() {
        let events = vec![
            AgentEventMessage {
                task_id: "t".to_string(),
                sequence: 1,
                at: timestamp(),
                event: AgentEvent::Tool(ToolEvent::ApprovalRequired {
                    id: "aprv_0001".to_string(),
                    task_id: "t".to_string(),
                    action: ApprovalAction::EditFile {
                        path: "a.ts".to_string(),
                        diff: "- a\n+ b".to_string(),
                    },
                }),
            },
            AgentEventMessage {
                task_id: "t".to_string(),
                sequence: 2,
                at: timestamp(),
                event: AgentEvent::Tool(ToolEvent::ApprovalDenied {
                    id: "aprv_0001".to_string(),
                    reason: Some("não".to_string()),
                }),
            },
        ];
        let (failures, rejected) = tally_events(&events);
        assert_eq!(failures, 0);
        assert_eq!(rejected, 1);
    }

    #[test]
    fn sanitize_replaces_model_tag_punctuation() {
        assert_eq!(sanitize_filename("qwen3-coder:30b"), "qwen3-coder_30b");
    }
}
