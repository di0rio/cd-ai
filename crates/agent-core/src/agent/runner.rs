//! The loop (plan 015, Part D): task → context → model → tools → result → model → … → report.
//!
//! Everything here is synchronous and runs on one dedicated thread (D1). The loop owns the task
//! state, persists it on every iteration (D8), and stops for a reason it can always name (D7).
//! It never produces `completed`: without the Verifier a finished task is `completed_unvalidated`
//! with evidence (D11).

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::agent::events::{AgentEvent, AgentEventMessage};
use crate::agent::model::{ChatModel, ModelError, ModelReply};
use crate::agent::profile::workspace_profile;
use crate::agent::prompt::{
    inherited_context, is_exhausted, system_prompt, trim_for_budget, wrap_untrusted_tool_result,
};
use crate::agent::state::{
    AgentLimits, CommandRecord, FileChange, StopReason, TaskReport, TaskState, TaskStatus,
};
use crate::agent::storage::TaskStore;
use crate::agent::tool_calls::{
    TOOL_NAMES, outcome_detail, outcome_output, redacted_input, render_outcome, signature,
    to_request, to_request_from_text, tool_specs,
};
use crate::events::{ToolEvent, ToolEventMessage};
use crate::ollama::{ChatEvent, ChatMessage, ModelFunctionCall, ModelToolCall};
use crate::permissions::ApprovalRequest;
use crate::redactor;
use crate::tool_call::parse_text_tool_calls;
use crate::tools::cancel::CancelToken;
use crate::tools::{Responder, ToolEngine, ToolRequest};
use crate::workspace::Workspace;

/// What a resumed task is told, as a user message (D9).
pub const RESUME_NOTE: &str = "A tarefa foi interrompida. Continue de onde parou.";
/// How many times the same call, or the same error, is tolerated before it counts as a loop (D7).
const LOOP_REPEATS: u32 = 3;
/// How many `write_file` calls in a row carrying the same content count as a loop, even when the
/// path changes every time (D7).
const SAME_CONTENT_WRITES: usize = 3;
/// Below this many characters, repeated content is not evidence of anything: empty files, barrels
/// and one-line stubs are legitimately written to several paths in a row. A false positive costs
/// the user a task that was going fine, so the bar is deliberately high.
const LOOP_CONTENT_MIN_CHARS: usize = 200;

/// Everything the loop needs besides the model and the responder.
pub struct TaskContext<'a> {
    /// Where state, transcript and events are written (D8).
    pub store: &'a TaskStore,
    pub workspace: Workspace,
    pub limits: AgentLimits,
    /// Cancels the loop, the model stream and any running command (D6).
    pub cancel: CancelToken,
    /// Course corrections typed while the task runs; drained at the top of each iteration.
    pub steer: Arc<Mutex<VecDeque<String>>>,
    /// ASK / AUTO / FULL ACCESS for this run (plan 017). Default ASK.
    pub permission_mode: crate::permissions::PermissionMode,
}

impl<'a> TaskContext<'a> {
    pub fn new(store: &'a TaskStore, workspace: Workspace) -> Self {
        Self {
            store,
            workspace,
            limits: AgentLimits::default(),
            cancel: CancelToken::default(),
            steer: Arc::default(),
            permission_mode: crate::permissions::PermissionMode::Ask,
        }
    }
}

/// Start a new task, or pick up an interrupted one (D9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStart {
    New {
        request: String,
        model: String,
        num_ctx: u32,
        /// Task this one continues, when the user wrote the request right after another task in
        /// the same workspace. The new task inherits that task's report — request, outcome, files
        /// changed, commands and final summary — and nothing else: the transcript stays where it
        /// is, so continuity never costs a whole window (decision 0008).
        continues: Option<String>,
    },
    Resume {
        task_id: String,
    },
}

/// How a workspace is keyed in the store, so the UI and the CLI list the same tasks.
pub fn workspace_key(workspace: &Workspace) -> String {
    workspace.info().root
}

/// Runs one task to a terminal status and returns its final state.
///
/// Errors never escape: a task that cannot even start comes back `failed` with the reason in
/// `stop_reason`, because the caller always has a task to show the user.
pub fn run_task(
    ctx: TaskContext<'_>,
    model: &mut dyn ChatModel,
    start: TaskStart,
    on_event: &mut dyn FnMut(AgentEventMessage),
    responder: Responder<'_>,
) -> TaskState {
    let key = workspace_key(&ctx.workspace);

    let (mut state, mut messages, opening) = match start {
        TaskStart::New {
            request,
            model: model_name,
            num_ctx,
            continues,
        } => {
            let id = TaskStore::new_task_id();
            let inherited = match &continues {
                Some(previous_id) => match inherit(ctx.store, previous_id, &key) {
                    Ok(block) => Some(block),
                    // Refusing is the honest answer: the only way to name a previous task is to
                    // pick one of this workspace's own, so a failure here means the id is wrong —
                    // and silently dropping the continuity would look exactly like the bug this
                    // chaining exists to fix.
                    Err(problem) => return unstartable(&id, &key, problem),
                },
                None => None,
            };
            let mut state = TaskState::new(&id, &key, &request, &model_name, num_ctx);
            state.continues = continues;
            let profile = workspace_profile(&ctx.workspace).render();
            let messages = vec![
                message("system", &system_prompt(&profile, inherited.as_deref())),
                message("user", &request),
            ];
            // Both go to the transcript: a resume rebuilds the conversation from it.
            (state, messages, 2)
        }
        TaskStart::Resume { task_id } => match ctx.store.load_state(&task_id) {
            Ok(mut state) => {
                if state.workspace != key {
                    return unstartable(
                        &task_id,
                        &key,
                        format!(
                            "a tarefa pertence a outro workspace ({}); abra aquela pasta para retomá-la",
                            state.workspace
                        ),
                    );
                }
                state.status = TaskStatus::Running;
                state.stop_reason = None;
                state.touch();
                let transcript = ctx.store.load_transcript(&task_id).unwrap_or_default();
                let mut messages = resume_messages(transcript, &ctx.workspace);
                messages.push(message("user", RESUME_NOTE));
                // Only the note is new; the rest is already on disk.
                (state, messages, 1)
            }
            Err(error) => return unstartable(&task_id, &key, error.to_string()),
        },
    };

    let mut emitter = Emitter {
        task_id: state.id.clone(),
        sequence: 0,
        store: ctx.store,
        sink: on_event,
    };
    emitter.emit(AgentEvent::TaskStarted {
        summary: state.summary(),
    });
    for new_message in &messages[messages.len() - opening..] {
        let _ = ctx.store.append_transcript(&state.id, new_message);
        if new_message.role == "user" {
            emitter.emit(AgentEvent::UserMessage {
                text: redact(&new_message.content),
            });
        }
    }
    let _ = ctx.store.save_state(&state);

    let mut engine = ToolEngine::with_mode(ctx.workspace.clone(), &state.id, ctx.permission_mode);
    engine.set_cancel(ctx.cancel.clone());

    let (reason, final_text) = run_loop(
        &ctx,
        &mut state,
        &mut messages,
        &mut emitter,
        &mut engine,
        model,
        responder,
    );

    let status = status_for(&reason);
    state.finish(status, reason.clone());
    let report = TaskReport {
        summary: redact(&final_text),
        // Always false in phase 5: the Verifier is phase 8 (D11).
        validated: false,
        evidence: state.commands.clone(),
        files_changed: state
            .files_changed
            .iter()
            .map(|change| change.path.clone())
            .collect(),
    };
    let _ = ctx.store.save_state(&state);
    emitter.emit(AgentEvent::TaskFinished {
        status,
        stop_reason: reason,
        report,
    });
    state
}

/// The iteration loop. Returns why it stopped and the last text the model produced.
fn run_loop(
    ctx: &TaskContext<'_>,
    state: &mut TaskState,
    messages: &mut Vec<ChatMessage>,
    emitter: &mut Emitter<'_>,
    engine: &mut ToolEngine,
    model: &mut dyn ChatModel,
    responder: Responder<'_>,
) -> (StopReason, String) {
    let specs = tool_specs();
    let started = Instant::now();
    let task_timeout = Duration::from_millis(ctx.limits.task_timeout_ms);

    // A task parked on an approval prompt is spending the human's time, not the machine's, so it
    // must not spend the deadline either. The wait happens inside the caller's `Responder` (the CLI
    // asks in the terminal, the bridge parks on a channel), which the loop cannot see — so the loop
    // wraps it and times the one call that blocks. `Cell` because the wrapper and the loop both
    // touch the counter, on the same thread.
    let blocked_ms = Cell::new(0_u64);
    let mut timed_responder = |request: ApprovalRequest| {
        let waiting = Instant::now();
        let response = responder(request);
        blocked_ms.set(blocked_ms.get() + waiting.elapsed().as_millis() as u64);
        response
    };

    let mut final_text = String::new();
    let mut consecutive_invalid = 0_u32;
    let mut seen_signatures: HashMap<String, u32> = HashMap::new();
    let mut last_error: Option<String> = None;
    let mut error_repeats = 0_u32;
    let mut repeated_write: Option<RepeatedWrite> = None;
    // Command ids are per engine, so this maps them to their record in the state.
    let mut command_records: HashMap<u64, usize> = HashMap::new();

    let stop = loop {
        // 1. Cancellation and the task deadline, before anything else (D6, D7).
        if ctx.cancel.is_cancelled() {
            break StopReason::Cancelled;
        }
        if started
            .elapsed()
            .saturating_sub(Duration::from_millis(blocked_ms.get()))
            >= task_timeout
        {
            break StopReason::TaskTimeout;
        }
        if state.iterations >= ctx.limits.max_iterations {
            break StopReason::MaxIterations;
        }

        // 2. Course corrections typed while the task was running.
        for text in drain_steer(&ctx.steer) {
            let steered = message("user", &text);
            let _ = ctx.store.append_transcript(&state.id, &steered);
            messages.push(steered);
            emitter.emit(AgentEvent::UserMessage {
                text: redact(&text),
            });
        }

        // 3. Context budget (D10).
        if let Some((removed_messages, estimated_tokens)) = trim_for_budget(messages, state.num_ctx)
        {
            emitter.emit(AgentEvent::ContextTrimmed {
                removed_messages,
                estimated_tokens,
            });
        }
        if is_exhausted(messages, state.num_ctx) {
            break StopReason::ContextExhausted;
        }

        // 4. The turn.
        state.iterations += 1;
        emitter.emit(AgentEvent::ModelTurnStarted {
            iteration: state.iterations,
            model: state.model.clone(),
        });

        // 5. Ask the model, retrying transient failures (D7).
        let mut attempt = 0_u32;
        let reply = loop {
            let result = {
                let mut forward = |event: ChatEvent| forward_chat_event(emitter, event);
                model.turn(messages, &specs, &mut forward, &ctx.cancel)
            };
            match result {
                Ok(reply) => break reply,
                // Cancellation is never retried.
                Err(ModelError::Cancelled) => return (StopReason::Cancelled, final_text),
                Err(error) => {
                    let message = error.to_string();
                    state.errors.push(redact(&message));
                    if attempt < ctx.limits.max_model_retries {
                        attempt += 1;
                        state.retries += 1;
                        emitter.emit(AgentEvent::Retrying {
                            attempt,
                            reason: redact(&message),
                        });
                        continue;
                    }
                    let _ = ctx.store.save_state(state);
                    return (StopReason::ModelError { message }, final_text);
                }
            }
        };
        state.metrics.prompt_tokens += reply.prompt_tokens;
        state.metrics.gen_tokens += reply.gen_tokens;
        state.metrics.model_ms += reply.prompt_ms + reply.gen_ms;

        // 6. Native tool calls first, the text format as fallback (D2).
        let turn = read_turn(&reply);

        // Only the prose is a message: the `<function=…>` markup of the text format is machinery,
        // and showing it made the user read every call twice (once raw, once as the tool line).
        if !turn.text.trim().is_empty() {
            final_text = turn.text.clone();
            emitter.emit(AgentEvent::AssistantMessage {
                content: redact(&turn.text),
            });
        }
        // The model gets its own turn back as prose plus the calls in the native shape, never as
        // the text markup it wrote: the loop already parsed it, and replaying the markup is how a
        // model learns to keep answering in a format it was never offered. The calls themselves
        // stay, so the model still sees what it asked for next to the result that answers it, and
        // so a resumed task can tell an unanswered call from a finished turn (D9).
        let assistant = ChatMessage {
            role: "assistant".to_string(),
            content: turn.text.clone(),
            tool_calls: turn.tool_calls.clone(),
            tool_name: None,
        };
        let _ = ctx.store.append_transcript(&state.id, &assistant);
        messages.push(assistant);

        // 7. No tool calls: the model says it is done (D11).
        if turn.calls.is_empty() {
            let _ = ctx.store.save_state(state);
            break StopReason::Finished;
        }

        // 8. Run each call.
        for call in turn.calls {
            if ctx.cancel.is_cancelled() {
                return (StopReason::Cancelled, final_text);
            }
            emitter.emit(AgentEvent::ToolCallRequested {
                tool: call.name.clone(),
                input: redacted_input(&call.input),
            });

            let request = match call.request {
                Ok(request) => request,
                // A malformed call is answered with the reason, so the model can repair it
                // (SPEC §4.3). Three in a row and the task gives up (D7).
                Err(problem) => {
                    consecutive_invalid += 1;
                    state.errors.push(redact(&problem));
                    emitter.emit(AgentEvent::ToolCallFinished {
                        tool: call.name.clone(),
                        ok: false,
                        detail: redact(&problem),
                        output: None,
                    });
                    push_tool_result(
                        ctx,
                        state,
                        messages,
                        &call.name,
                        &format!("erro: {problem}"),
                    );
                    if consecutive_invalid >= ctx.limits.max_invalid_tool_calls {
                        let _ = ctx.store.save_state(state);
                        return (StopReason::InvalidToolCalls, final_text);
                    }
                    continue;
                }
            };
            consecutive_invalid = 0;

            // Loop detection before running: the third identical call is not executed (D7).
            let signature = signature(&request);
            let repeats = seen_signatures.entry(signature).or_insert(0);
            *repeats += 1;
            if *repeats >= LOOP_REPEATS {
                let detail = format!(
                    "{} com os mesmos argumentos {} vezes",
                    call.name, LOOP_REPEATS
                );
                let _ = ctx.store.save_state(state);
                return (StopReason::LoopDetected { detail }, final_text);
            }

            // Rewriting the same content under another name is not progress, and the exact
            // signature never catches it: one byte of path is enough to look like a new call. Only
            // a run of consecutive writes counts — any other tool in between is a change in the
            // project, so the run resets — and only content long enough that writing it twice
            // cannot be a legitimate stub (D7).
            if let ToolRequest::WriteFile(args) = &request {
                let content = normalize_content(&args.content);
                let mut run = match repeated_write.take() {
                    Some(previous) if previous.content == content => previous,
                    _ => RepeatedWrite {
                        content,
                        paths: Vec::new(),
                    },
                };
                run.paths.push(args.path.clone());
                if run.paths.len() >= SAME_CONTENT_WRITES
                    && run.content.chars().count() >= LOOP_CONTENT_MIN_CHARS
                {
                    let detail = format!(
                        "tentou gravar o mesmo conteúdo {} vezes seguidas, mudando só o caminho ({}); \
                         nada mudou no projeto entre as tentativas",
                        run.paths.len(),
                        run.paths.join(", ")
                    );
                    let _ = ctx.store.save_state(state);
                    return (StopReason::LoopDetected { detail }, final_text);
                }
                repeated_write = Some(run);
            } else {
                repeated_write = None;
            }

            let started_tool = Instant::now();
            let blocked_before = blocked_ms.get();
            let outcome = {
                let mut sink = |event: ToolEventMessage| {
                    apply_tool_event(state, &event.event, &mut command_records);
                    // The UI needs to know it is waiting on a human before the prompt shows up.
                    if matches!(event.event, ToolEvent::ApprovalRequired { .. }) {
                        state.status = TaskStatus::WaitingApproval;
                        emitter.emit(AgentEvent::StatusChanged {
                            status: TaskStatus::WaitingApproval,
                            reason: None,
                        });
                    }
                    let decided = matches!(
                        event.event,
                        ToolEvent::ApprovalGranted { .. } | ToolEvent::ApprovalDenied { .. }
                    );
                    emitter.emit(AgentEvent::Tool(event.event));
                    if decided {
                        state.status = TaskStatus::Running;
                        emitter.emit(AgentEvent::StatusChanged {
                            status: TaskStatus::Running,
                            reason: None,
                        });
                    }
                };
                engine.run_tool(request, &mut sink, &mut timed_responder)
            };
            // `tool_ms` is what the tool cost; the human's part of the wall clock is its own number.
            let waited = blocked_ms.get() - blocked_before;
            state.metrics.tool_ms +=
                (started_tool.elapsed().as_millis() as u64).saturating_sub(waited);
            state.metrics.approval_wait_ms += waited;

            let detail = outcome_detail(&outcome);
            emitter.emit(AgentEvent::ToolCallFinished {
                tool: call.name.clone(),
                ok: outcome.ok,
                detail: redact(&detail),
                output: outcome_output(&outcome).map(|text| redact(&text)),
            });
            push_tool_result(ctx, state, messages, &call.name, &render_outcome(&outcome));

            // The same error over and over is a loop too (D7).
            if outcome.ok {
                last_error = None;
                error_repeats = 0;
            } else {
                if last_error.as_deref() == Some(detail.as_str()) {
                    error_repeats += 1;
                } else {
                    error_repeats = 1;
                    last_error = Some(detail.clone());
                }
                if error_repeats >= LOOP_REPEATS {
                    let _ = ctx.store.save_state(state);
                    return (
                        StopReason::LoopDetected {
                            detail: format!("o mesmo erro {LOOP_REPEATS} vezes: {detail}"),
                        },
                        final_text,
                    );
                }
            }
        }

        // 9. The state on disk is never more than one iteration old (D8).
        state.touch();
        let _ = ctx.store.save_state(state);
    };

    (stop, final_text)
}

/// One tool call waiting to run, in whichever shape the model wrote it.
struct PendingCall {
    name: String,
    /// The arguments as the model sent them, for the event.
    input: Value,
    request: Result<ToolRequest, String>,
}

/// A model turn split into the three things the loop does with it: what to run, what the user
/// reads, and what goes back to the model as its own message.
struct ModelTurn {
    calls: Vec<PendingCall>,
    /// Prose only. In the text fallback the markup is stripped by the parser that consumed it.
    text: String,
    tool_calls: Vec<ModelToolCall>,
}

/// A run of consecutive `write_file` calls that all carried the same content, whatever the path.
struct RepeatedWrite {
    content: String,
    paths: Vec<String>,
}

/// Content as loop detection compares it: line endings and edge whitespace do not make a rewrite
/// a different rewrite.
fn normalize_content(content: &str) -> String {
    content.replace("\r\n", "\n").trim().to_string()
}

/// Native `tool_calls` when there are any; otherwise the text format of the CODER model (D2).
fn read_turn(reply: &ModelReply) -> ModelTurn {
    if !reply.tool_calls.is_empty() {
        return ModelTurn {
            calls: reply
                .tool_calls
                .iter()
                .map(|call| PendingCall {
                    name: call.function.name.clone(),
                    input: call.function.arguments.clone(),
                    request: to_request(&call.function.name, &call.function.arguments),
                })
                .collect(),
            text: reply.content.clone(),
            tool_calls: reply.tool_calls.clone(),
        };
    }

    // `parse_text_tool_calls` already drops any name that was not offered, and hands back the
    // content without the blocks it consumed.
    let parsed = parse_text_tool_calls(&reply.content, &TOOL_NAMES);
    if parsed.calls.is_empty() {
        // Nothing was consumed, so nothing was stripped: the reply is plain prose.
        return ModelTurn {
            calls: Vec::new(),
            text: reply.content.clone(),
            tool_calls: Vec::new(),
        };
    }

    let mut calls = Vec::new();
    let mut tool_calls = Vec::new();
    for call in &parsed.calls {
        let input = Value::Object(
            call.arguments
                .iter()
                .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                .collect(),
        );
        tool_calls.push(ModelToolCall {
            function: ModelFunctionCall {
                name: call.name.clone(),
                arguments: input.clone(),
            },
        });
        calls.push(PendingCall {
            name: call.name.clone(),
            input,
            request: to_request_from_text(call),
        });
    }
    ModelTurn {
        calls,
        text: parsed.prose,
        tool_calls,
    }
}

/// Appends a tool result to the conversation and to the transcript.
fn push_tool_result(
    ctx: &TaskContext<'_>,
    state: &TaskState,
    messages: &mut Vec<ChatMessage>,
    tool: &str,
    content: &str,
) {
    let result = ChatMessage {
        role: "tool".to_string(),
        content: wrap_untrusted_tool_result(content),
        tool_calls: Vec::new(),
        tool_name: Some(tool.to_string()),
    };
    let _ = ctx.store.append_transcript(&state.id, &result);
    messages.push(result);
}

/// Keeps the evidence of the task up to date from what the engine reports. Deriving it from the
/// events instead of from the outcomes is what gives the report hashes and durations.
fn apply_tool_event(
    state: &mut TaskState,
    event: &ToolEvent,
    command_records: &mut HashMap<u64, usize>,
) {
    match event {
        ToolEvent::FileRead { path, .. } => {
            if !state.files_read.iter().any(|seen| seen == path) {
                state.files_read.push(path.clone());
            }
        }
        ToolEvent::FileChanged {
            path, hash_after, ..
        } => match state
            .files_changed
            .iter_mut()
            .find(|change| &change.path == path)
        {
            Some(change) => change.hash_after = hash_after.clone(),
            None => state.files_changed.push(FileChange {
                path: path.clone(),
                hash_after: hash_after.clone(),
            }),
        },
        ToolEvent::CommandStarted { id, argv, .. } => {
            command_records.insert(*id, state.commands.len());
            state.commands.push(CommandRecord {
                argv: argv.clone(),
                exit_code: None,
                duration_ms: 0,
            });
        }
        ToolEvent::CommandCompleted {
            id,
            exit_code,
            duration_ms,
            ..
        } => {
            if let Some(index) = command_records.get(id)
                && let Some(record) = state.commands.get_mut(*index)
            {
                record.exit_code = *exit_code;
                record.duration_ms = *duration_ms;
            }
        }
        ToolEvent::ToolFailed { message, .. } => state.errors.push(message.clone()),
        _ => {}
    }
}

/// Maps what the model streams onto the task's own channel (D12).
fn forward_chat_event(emitter: &mut Emitter<'_>, event: ChatEvent) {
    match event {
        ChatEvent::Token { content } => emitter.emit(AgentEvent::Token {
            content: redact(&content),
        }),
        ChatEvent::Thinking { content } => emitter.emit(AgentEvent::Thinking {
            content: redact(&content),
        }),
        // Tool calls surface as `ToolCallRequested`, one per call, once they are mapped.
        ChatEvent::ToolCalls { .. } => {}
        ChatEvent::Done {
            prompt_tokens,
            gen_tokens,
            prompt_ms,
            gen_ms,
        } => emitter.emit(AgentEvent::ModelTurnCompleted {
            prompt_tokens,
            gen_tokens,
            prompt_ms,
            gen_ms,
        }),
        // A failed generation comes back as `ModelError`/`Retrying` from `turn`.
        ChatEvent::Error { .. } => {}
    }
}

/// Builds what a new task inherits from the task it continues.
///
/// The previous summary is the last thing that task's model said, read from its transcript: the
/// report is the state plus that text, so nothing new has to be persisted and a task that ended
/// before chaining existed can be continued just the same. Reading one file at the start of a
/// task is cheap; the point of option B is that what travels is the report, not the conversation.
fn inherit(store: &TaskStore, previous_id: &str, workspace: &str) -> Result<String, String> {
    let previous = store
        .load_state(previous_id)
        .map_err(|error| format!("não foi possível continuar a tarefa {previous_id}: {error}"))?;
    if previous.workspace != workspace {
        return Err(format!(
            "a tarefa {previous_id} pertence a outro workspace ({}); abra aquela pasta para continuá-la",
            previous.workspace
        ));
    }
    Ok(inherited_context(
        &previous,
        &last_assistant_text(store, previous_id),
    ))
}

/// The final text of a task: its report summary, already redacted on disk (D8).
fn last_assistant_text(store: &TaskStore, id: &str) -> String {
    store
        .load_transcript(id)
        .unwrap_or_default()
        .into_iter()
        .rev()
        .find(|message| message.role == "assistant" && !message.content.trim().is_empty())
        .map(|message| message.content)
        .unwrap_or_default()
}

/// Rebuilds the conversation of an interrupted task (D9).
///
/// The last assistant turn whose tool calls have no results is dropped: sending it again would
/// leave the model waiting for answers that never came. A transcript without a system prompt
/// (a task from before the prompt changed, or a truncated file) gets a fresh one.
fn resume_messages(mut transcript: Vec<ChatMessage>, workspace: &Workspace) -> Vec<ChatMessage> {
    if let Some(index) = transcript
        .iter()
        .rposition(|message| message.role == "assistant" && !message.tool_calls.is_empty())
    {
        let expected = transcript[index].tool_calls.len();
        let answered = transcript[index + 1..]
            .iter()
            .filter(|message| message.role == "tool")
            .count();
        if answered < expected {
            transcript.truncate(index);
        }
    }
    if transcript.first().map(|first| first.role.as_str()) != Some("system") {
        // Only a truncated or pre-`system` transcript lands here, so the rebuilt prompt carries no
        // inherited block: the real one is the first line of every transcript this loop writes.
        let profile = workspace_profile(workspace).render();
        transcript.insert(0, message("system", &system_prompt(&profile, None)));
    }
    transcript
}

/// A task that could not even be loaded, as a state the caller can show.
///
/// `StopReason` has no storage variant, so the reason travels in `ModelError`, which is the one
/// free-text failure of the contract. The message says what actually happened.
fn unstartable(task_id: &str, workspace: &str, problem: String) -> TaskState {
    let mut state = TaskState::new(task_id, workspace, "", "", 0);
    state.errors.push(problem.clone());
    state.finish(
        TaskStatus::Failed,
        StopReason::ModelError { message: problem },
    );
    state
}

/// The status each stop reason lands on. `Finished` is the only success, and it is never
/// `completed` (D11).
fn status_for(reason: &StopReason) -> TaskStatus {
    match reason {
        StopReason::Finished => TaskStatus::CompletedUnvalidated,
        StopReason::Cancelled | StopReason::Interrupted => TaskStatus::Cancelled,
        StopReason::MaxIterations
        | StopReason::TaskTimeout
        | StopReason::LoopDetected { .. }
        | StopReason::InvalidToolCalls
        | StopReason::ModelError { .. }
        | StopReason::ContextExhausted => TaskStatus::Failed,
    }
}

fn drain_steer(steer: &Mutex<VecDeque<String>>) -> Vec<String> {
    match steer.lock() {
        Ok(mut queue) => queue.drain(..).collect(),
        // A poisoned queue means another thread panicked while steering; losing the text is
        // better than taking the task down with it.
        Err(_) => Vec::new(),
    }
}

fn message(role: &str, content: &str) -> ChatMessage {
    ChatMessage {
        role: role.to_string(),
        content: content.to_string(),
        ..Default::default()
    }
}

/// Free text leaves the loop redacted, both to the channel and to disk (SPEC §20.6).
fn redact(text: &str) -> String {
    redactor::redact(text).text
}

/// Numbers, timestamps and persistence for every event of one task (D12).
struct Emitter<'a> {
    task_id: String,
    sequence: u64,
    store: &'a TaskStore,
    sink: &'a mut dyn FnMut(AgentEventMessage),
}

impl Emitter<'_> {
    fn emit(&mut self, event: AgentEvent) {
        self.sequence += 1;
        let message = AgentEventMessage::new(self.task_id.clone(), self.sequence, event);
        // Best effort: a task must not die because its log could not be appended.
        let _ = self.store.append_event(&message);
        (self.sink)(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::model::ScriptedModel;
    use crate::agent::prompt::{INHERITED_BEGIN, UNTRUSTED_TOOL_BEGIN, UNTRUSTED_TOOL_END};
    use crate::permissions::{ApprovalRequest, ApprovalResponse};
    use crate::tools::command::test_argv;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::{TempDir, tempdir};

    /// A store and a workspace, both in tempdirs: the real data directory is never touched.
    struct Harness {
        _data: TempDir,
        _project: TempDir,
        store: TaskStore,
        workspace: Workspace,
    }

    fn harness(files: &[(&str, &str)]) -> Harness {
        let data = tempdir().unwrap();
        let project = tempdir().unwrap();
        for (name, content) in files {
            let path = project.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, content).unwrap();
        }
        let store = TaskStore::open(data.path()).unwrap();
        let workspace = Workspace::open(project.path()).unwrap();
        Harness {
            _data: data,
            _project: project,
            store,
            workspace,
        }
    }

    impl Harness {
        fn context(&self) -> TaskContext<'_> {
            TaskContext::new(&self.store, self.workspace.clone())
        }
    }

    fn start(request: &str) -> TaskStart {
        TaskStart::New {
            request: request.to_string(),
            model: "modelo-x".to_string(),
            num_ctx: 16_384,
            continues: None,
        }
    }

    fn start_continuing(request: &str, previous: &str) -> TaskStart {
        TaskStart::New {
            request: request.to_string(),
            model: "modelo-x".to_string(),
            num_ctx: 16_384,
            continues: Some(previous.to_string()),
        }
    }

    fn grant(_: ApprovalRequest) -> ApprovalResponse {
        ApprovalResponse::Granted
    }

    fn deny(_: ApprovalRequest) -> ApprovalResponse {
        ApprovalResponse::Denied {
            reason: Some("não agora".to_string()),
        }
    }

    /// Runs a task, collecting every event.
    fn run(
        ctx: TaskContext<'_>,
        model: &mut dyn ChatModel,
        start: TaskStart,
        responder: &mut dyn FnMut(ApprovalRequest) -> ApprovalResponse,
    ) -> (TaskState, Vec<AgentEventMessage>) {
        let mut events = Vec::new();
        let mut sink = |message: AgentEventMessage| events.push(message);
        let state = run_task(ctx, model, start, &mut sink, responder);
        (state, events)
    }

    fn tool_messages(model: &ScriptedModel, turn: usize) -> Vec<&ChatMessage> {
        model.seen[turn]
            .iter()
            .filter(|message| message.role == "tool")
            .collect()
    }

    // 1 ────────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn a_reply_without_tools_finishes_unvalidated() {
        let harness = harness(&[("Cargo.toml", "[package]\nname = \"x\"\n")]);
        let mut model = ScriptedModel::new(vec![ScriptedModel::text("arrumei nada, estava ok")]);
        let (state, events) = run(
            harness.context(),
            &mut model,
            start("olhe o projeto"),
            &mut grant,
        );

        assert_eq!(state.status, TaskStatus::CompletedUnvalidated);
        assert_eq!(state.stop_reason, Some(StopReason::Finished));
        assert_eq!(state.iterations, 1);

        let finished = events
            .iter()
            .find_map(|message| match &message.event {
                AgentEvent::TaskFinished { report, status, .. } => Some((report, *status)),
                _ => None,
            })
            .expect("taskFinished");
        assert!(!finished.0.validated, "a fase 5 nunca valida (D11)");
        assert_eq!(finished.0.summary, "arrumei nada, estava ok");
        assert_eq!(finished.1, TaskStatus::CompletedUnvalidated);

        // The task was offered exactly the six tools.
        assert_eq!(model.offered, TOOL_NAMES.to_vec());
        // The context is the system prompt plus the request.
        assert_eq!(model.seen[0][0].role, "system");
        assert!(model.seen[0][0].content.contains("You are cd-ai"));
        assert!(model.seen[0][0].content.contains("Cargo.toml"));
        assert_eq!(model.seen[0][1].content, "olhe o projeto");
    }

    // 2 ────────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn a_native_read_file_runs_and_comes_back_as_a_tool_message() {
        let harness = harness(&[("src/soma.ts", "export const soma = (a, b) => a - b;\n")]);
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::calls(&[("read_file", serde_json::json!({ "path": "src/soma.ts" }))]),
            ScriptedModel::text("li o arquivo"),
        ]);
        let (state, events) = run(
            harness.context(),
            &mut model,
            start("leia a soma"),
            &mut grant,
        );

        assert_eq!(state.status, TaskStatus::CompletedUnvalidated);
        assert_eq!(state.files_read, vec!["src/soma.ts"]);

        let results = tool_messages(&model, 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tool_name.as_deref(), Some("read_file"));
        assert!(
            results[0].content.starts_with(UNTRUSTED_TOOL_BEGIN),
            "{}",
            results[0].content
        );
        assert!(results[0].content.contains("src/soma.ts (linhas 1-1 de 1)"));
        assert!(results[0].content.contains("a - b"));
        assert!(results[0].content.contains(UNTRUSTED_TOOL_END));

        // The events name the tool and say it went fine.
        assert!(events.iter().any(|message| matches!(
            &message.event,
            AgentEvent::ToolCallRequested { tool, .. } if tool == "read_file"
        )));
        assert!(events.iter().any(|message| matches!(
            &message.event,
            AgentEvent::ToolCallFinished { tool, ok: true, .. } if tool == "read_file"
        )));
        assert!(
            events.iter().any(|message| matches!(
                &message.event,
                AgentEvent::Tool(ToolEvent::FileRead { .. })
            ))
        );
    }

    // 3 ────────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn the_text_fallback_runs_too() {
        let harness = harness(&[("src/a.rs", "fn main() {}\n")]);
        let mut model = ScriptedModel::new(vec![
            // No native tool_calls: the CODER model writes the call as text (D2, decision 0002).
            ScriptedModel::text(
                "Vou ler o arquivo.\n\n<function=read_file>\n<parameter=path>\nsrc/a.rs\n</parameter>\n</function>\n</tool_call>",
            ),
            ScriptedModel::text("pronto"),
        ]);
        let (state, _) = run(harness.context(), &mut model, start("leia"), &mut grant);

        assert_eq!(state.status, TaskStatus::CompletedUnvalidated);
        assert_eq!(state.files_read, vec!["src/a.rs"]);
        let results = tool_messages(&model, 1);
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("fn main() {}"));
    }

    // 4 ────────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn an_unknown_tool_is_an_error_and_three_in_a_row_fail_the_task() {
        let harness = harness(&[]);
        let mut model = ScriptedModel::new(vec![ScriptedModel::calls(&[(
            "delete_everything",
            serde_json::json!({ "path": "/" }),
        )])]);
        let (state, events) = run(
            harness.context(),
            &mut model,
            start("apague tudo"),
            &mut grant,
        );

        assert_eq!(state.status, TaskStatus::Failed);
        assert_eq!(state.stop_reason, Some(StopReason::InvalidToolCalls));
        // One invalid call per iteration, three iterations.
        assert_eq!(state.iterations, 3);
        let results = tool_messages(&model, 1);
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("erro: tool desconhecida"));
        assert!(events.iter().any(|message| matches!(
            &message.event,
            AgentEvent::ToolCallFinished { ok: false, detail, .. } if detail.contains("desconhecida")
        )));
        // Nothing was ever run, so no tool event was emitted.
        assert!(
            !events
                .iter()
                .any(|message| matches!(&message.event, AgentEvent::Tool(_)))
        );
    }

    #[test]
    fn a_mistyped_argument_is_answered_instead_of_executed() {
        let harness = harness(&[]);
        let mut model = ScriptedModel::new(vec![
            // argv as a single string is refused by design (§2.6).
            ScriptedModel::calls(&[("run_command", serde_json::json!({ "argv": "cargo test" }))]),
            ScriptedModel::text("entendi, uso array"),
        ]);
        let (state, _) = run(
            harness.context(),
            &mut model,
            start("rode os testes"),
            &mut grant,
        );

        assert_eq!(state.stop_reason, Some(StopReason::Finished));
        let results = tool_messages(&model, 1);
        assert!(results[0].content.contains("array de strings"));
        assert!(state.commands.is_empty(), "nada foi executado");
    }

    // 5 ────────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn the_same_call_three_times_is_a_loop() {
        let harness = harness(&[("src/a.rs", "fn main() {}\n")]);
        // The script repeats its last reply, so the model keeps asking for the same file.
        let mut model = ScriptedModel::new(vec![ScriptedModel::calls(&[(
            "read_file",
            serde_json::json!({ "path": "src/a.rs" }),
        )])]);
        let (state, _) = run(
            harness.context(),
            &mut model,
            start("leia sem parar"),
            &mut grant,
        );

        assert_eq!(state.status, TaskStatus::Failed);
        match state.stop_reason {
            Some(StopReason::LoopDetected { ref detail }) => {
                assert!(detail.contains("read_file"), "{detail}");
            }
            other => panic!("esperava loopDetected, veio {other:?}"),
        }
        // The third identical call is detected before it runs.
        assert_eq!(state.iterations, 3);
    }

    // 6 ────────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn max_iterations_stops_a_model_that_never_finishes() {
        let harness = harness(&[("a.rs", "1\n"), ("b.rs", "2\n")]);
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::calls(&[("read_file", serde_json::json!({ "path": "a.rs" }))]),
            ScriptedModel::calls(&[("read_file", serde_json::json!({ "path": "b.rs" }))]),
            ScriptedModel::calls(&[("list_directory", serde_json::json!({ "path": "." }))]),
        ]);
        let mut ctx = harness.context();
        ctx.limits.max_iterations = 2;
        let (state, _) = run(ctx, &mut model, start("leia tudo"), &mut grant);

        assert_eq!(state.status, TaskStatus::Failed);
        assert_eq!(state.stop_reason, Some(StopReason::MaxIterations));
        assert_eq!(state.iterations, 2);
        assert_eq!(model.seen.len(), 2);
    }

    // 7 ────────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn a_denied_approval_goes_back_to_the_model_and_the_loop_continues() {
        let harness = harness(&[]);
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::calls(&[(
                "write_file",
                serde_json::json!({ "path": "novo.txt", "content": "oi" }),
            )]),
            ScriptedModel::text("não pude escrever, paro aqui"),
        ]);
        let (state, events) = run(
            harness.context(),
            &mut model,
            start("crie o arquivo"),
            &mut deny,
        );

        // The task keeps going after the refusal and finishes normally.
        assert_eq!(state.status, TaskStatus::CompletedUnvalidated);
        assert_eq!(state.stop_reason, Some(StopReason::Finished));
        let results = tool_messages(&model, 1);
        assert!(
            results[0].content.contains("erro: permissão negada"),
            "{}",
            results[0].content
        );
        assert!(state.files_changed.is_empty());
        assert!(!harness.workspace.root().join("novo.txt").exists());

        // The UI is told it is waiting on a human, and then that it is running again.
        let statuses: Vec<TaskStatus> = events
            .iter()
            .filter_map(|message| match &message.event {
                AgentEvent::StatusChanged { status, .. } => Some(*status),
                _ => None,
            })
            .collect();
        assert_eq!(
            statuses,
            vec![TaskStatus::WaitingApproval, TaskStatus::Running]
        );
        assert!(events.iter().any(|message| matches!(
            &message.event,
            AgentEvent::Tool(ToolEvent::ApprovalDenied { .. })
        )));
    }

    // 8 ────────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn cancelling_during_a_command_stops_the_task_quickly() {
        let harness = harness(&[]);
        let mut model = ScriptedModel::new(vec![ScriptedModel::calls(&[(
            "run_command",
            serde_json::json!({ "argv": test_argv::sleep_secs(5), "timeout_ms": 30_000 }),
        )])]);
        let ctx = harness.context();
        let cancel = ctx.cancel.clone();
        let watcher = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            cancel.cancel();
        });

        let started = Instant::now();
        let (state, events) = run(ctx, &mut model, start("durma"), &mut grant);
        let elapsed = started.elapsed();
        watcher.join().unwrap();

        assert!(
            elapsed < Duration::from_secs(3),
            "o cancelamento precisa voltar rápido, levou {elapsed:?}"
        );
        assert_eq!(state.status, TaskStatus::Cancelled);
        assert_eq!(state.stop_reason, Some(StopReason::Cancelled));
        // The command is in the evidence, with no exit code (plan 015 notes).
        assert_eq!(state.commands.len(), 1);
        assert_eq!(state.commands[0].exit_code, None);
        // And the model was told, without any partial output.
        assert!(events.iter().any(|message| matches!(
            &message.event,
            AgentEvent::ToolCallFinished { ok: false, detail, .. } if detail == "tarefa cancelada"
        )));
    }

    // 9 ────────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn state_is_persisted_and_an_interrupted_task_resumes() {
        let harness = harness(&[("a.rs", "fn a() {}\n")]);
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::calls(&[("read_file", serde_json::json!({ "path": "a.rs" }))]),
            ScriptedModel::text("li o arquivo"),
        ]);
        let (state, _) = run(
            harness.context(),
            &mut model,
            start("leia a.rs"),
            &mut grant,
        );

        // The state is on disk, with the evidence of the run.
        let saved = harness.store.load_state(&state.id).unwrap();
        assert_eq!(saved.status, TaskStatus::CompletedUnvalidated);
        assert_eq!(saved.iterations, 2);
        assert_eq!(saved.files_read, vec!["a.rs"]);
        assert_eq!(harness.store.list(&saved.workspace).unwrap().len(), 1);

        // Pretend the app died mid-task: D9 marks it cancelled/interrupted.
        let mut interrupted = saved.clone();
        interrupted.status = TaskStatus::Running;
        interrupted.stop_reason = None;
        harness.store.save_state(&interrupted).unwrap();
        assert_eq!(
            harness.store.recover_interrupted().unwrap(),
            vec![state.id.clone()]
        );
        let recovered = harness.store.load_state(&state.id).unwrap();
        assert_eq!(recovered.stop_reason, Some(StopReason::Interrupted));

        // Resuming rebuilds the conversation and adds the note.
        let mut resumed_model =
            ScriptedModel::new(vec![ScriptedModel::text("continuei e terminei")]);
        let (resumed, events) = run(
            harness.context(),
            &mut resumed_model,
            TaskStart::Resume {
                task_id: state.id.clone(),
            },
            &mut grant,
        );
        assert_eq!(resumed.id, state.id);
        assert_eq!(resumed.status, TaskStatus::CompletedUnvalidated);

        let seen = &resumed_model.seen[0];
        assert_eq!(seen[0].role, "system");
        assert_eq!(seen[1].content, "leia a.rs");
        assert!(seen.iter().any(|message| message.role == "tool"));
        assert_eq!(seen.last().unwrap().role, "user");
        assert_eq!(seen.last().unwrap().content, RESUME_NOTE);
        assert!(events.iter().any(|message| matches!(
            &message.event,
            AgentEvent::UserMessage { text } if text == RESUME_NOTE
        )));
    }

    #[test]
    fn resuming_drops_an_assistant_turn_whose_tools_never_answered() {
        let harness = harness(&[("a.rs", "fn a() {}\n")]);
        let id = TaskStore::new_task_id();
        let mut state = TaskState::new(
            &id,
            workspace_key(&harness.workspace),
            "leia a.rs",
            "modelo-x",
            16_384,
        );
        state.status = TaskStatus::Cancelled;
        state.stop_reason = Some(StopReason::Interrupted);
        harness.store.save_state(&state).unwrap();
        for message in [
            message("system", "regras antigas"),
            message("user", "leia a.rs"),
        ] {
            harness.store.append_transcript(&id, &message).unwrap();
        }
        // An assistant turn with a tool call that was never answered.
        harness
            .store
            .append_transcript(
                &id,
                &ChatMessage {
                    role: "assistant".to_string(),
                    content: String::new(),
                    tool_calls: vec![crate::ollama::ModelToolCall {
                        function: crate::ollama::ModelFunctionCall {
                            name: "read_file".to_string(),
                            arguments: serde_json::json!({ "path": "a.rs" }),
                        },
                    }],
                    tool_name: None,
                },
            )
            .unwrap();

        let mut model = ScriptedModel::new(vec![ScriptedModel::text("retomei")]);
        let (resumed, _) = run(
            harness.context(),
            &mut model,
            TaskStart::Resume { task_id: id },
            &mut grant,
        );
        assert_eq!(resumed.status, TaskStatus::CompletedUnvalidated);
        let seen = &model.seen[0];
        assert!(
            !seen.iter().any(|message| !message.tool_calls.is_empty()),
            "a chamada sem resposta foi descartada"
        );
        assert_eq!(seen.last().unwrap().content, RESUME_NOTE);
    }

    #[test]
    fn resuming_a_task_of_another_workspace_is_refused() {
        let other = harness(&[]);
        let harness = harness(&[]);
        let id = TaskStore::new_task_id();
        let state = TaskState::new(
            &id,
            workspace_key(&other.workspace),
            "pedido",
            "modelo-x",
            16_384,
        );
        harness.store.save_state(&state).unwrap();

        let mut model = ScriptedModel::new(vec![ScriptedModel::text("nunca chamado")]);
        let (resumed, _) = run(
            harness.context(),
            &mut model,
            TaskStart::Resume { task_id: id },
            &mut grant,
        );
        assert_eq!(resumed.status, TaskStatus::Failed);
        assert!(model.seen.is_empty(), "o modelo não é chamado");
    }

    // 10 ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn a_small_context_trims_the_oldest_tool_results() {
        // Each read is ~6 KiB ≈ 1500 tokens. Two of them fit under 75% of 5120; the third does not.
        let big = "linha de arquivo bem comprida\n".repeat(200);
        let harness = harness(&[("a.rs", &big), ("b.rs", &big), ("c.rs", &big)]);
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::calls(&[("read_file", serde_json::json!({ "path": "a.rs" }))]),
            ScriptedModel::calls(&[("read_file", serde_json::json!({ "path": "b.rs" }))]),
            ScriptedModel::calls(&[("read_file", serde_json::json!({ "path": "c.rs" }))]),
            ScriptedModel::text("li os três"),
        ]);
        let (state, events) = run(
            harness.context(),
            &mut model,
            TaskStart::New {
                request: "leia os três".to_string(),
                model: "modelo-x".to_string(),
                num_ctx: 5_120,
                continues: None,
            },
            &mut grant,
        );

        assert_eq!(state.stop_reason, Some(StopReason::Finished));
        let trimmed: Vec<(u32, u64)> = events
            .iter()
            .filter_map(|message| match &message.event {
                AgentEvent::ContextTrimmed {
                    removed_messages,
                    estimated_tokens,
                } => Some((*removed_messages, *estimated_tokens)),
                _ => None,
            })
            .collect();
        assert!(!trimmed.is_empty(), "esperava ContextTrimmed");
        assert!(trimmed[0].0 >= 1);
        // The oldest result is gone from what the model sees; the two most recent are intact.
        let results = tool_messages(&model, model.seen.len() - 1);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].content, crate::agent::prompt::OMITTED_RESULT);
        assert!(results[1].content.contains("b.rs (linhas 1-200 de 200)"));
        assert!(results[2].content.contains("c.rs (linhas 1-200 de 200)"));
    }

    // 11 ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn a_model_error_is_retried_then_gives_up() {
        let harness = harness(&[]);
        // The script repeats its last entry, so every attempt fails the same way.
        let mut model = ScriptedModel::new(vec![Err(ModelError::Failed(
            "conexão recusada".to_string(),
        ))]);
        let (state, events) = run(
            harness.context(),
            &mut model,
            start("faça algo"),
            &mut grant,
        );

        assert_eq!(state.status, TaskStatus::Failed);
        match state.stop_reason {
            Some(StopReason::ModelError { ref message }) => {
                assert!(message.contains("conexão recusada"), "{message}");
            }
            other => panic!("esperava modelError, veio {other:?}"),
        }
        // 1 + max_model_retries attempts (D7).
        assert_eq!(
            model.seen.len(),
            1 + AgentLimits::default().max_model_retries as usize
        );
        assert_eq!(state.retries, AgentLimits::default().max_model_retries);
        let retries = events
            .iter()
            .filter(|message| matches!(&message.event, AgentEvent::Retrying { .. }))
            .count();
        assert_eq!(retries, AgentLimits::default().max_model_retries as usize);
    }

    // 12 ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn a_secret_in_the_request_is_redacted_on_disk() {
        let token = "ghp_abcDEF1234567890abcDEF1234567890";
        let harness = harness(&[]);
        let mut model = ScriptedModel::new(vec![ScriptedModel::text("não vou usar o token")]);
        let (state, events) = run(
            harness.context(),
            &mut model,
            TaskStart::New {
                request: format!("use o token {token} no deploy"),
                model: "modelo-x".to_string(),
                num_ctx: 16_384,
                continues: None,
            },
            &mut grant,
        );

        let dir = harness.store.root().join(&state.id);
        for file in ["transcript.jsonl", "state.json", "events.jsonl"] {
            let text = fs::read_to_string(dir.join(file)).unwrap();
            assert!(!text.contains(token), "{file} não pode conter o token");
            assert!(
                text.contains("[REDIGIDO:segredo]"),
                "{file} deve mostrar a marca de redação"
            );
        }
        // And the live event stream is redacted too.
        let user_text = events
            .iter()
            .find_map(|message| match &message.event {
                AgentEvent::UserMessage { text } => Some(text.clone()),
                _ => None,
            })
            .expect("userMessage");
        assert!(!user_text.contains(token));
    }

    // Extras ───────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn a_command_is_approved_run_and_recorded_as_evidence() {
        let harness = harness(&[]);
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::calls(&[(
                "run_command",
                serde_json::json!({ "argv": test_argv::echo("oi") }),
            )]),
            ScriptedModel::text("rodei o comando"),
        ]);
        let (state, events) = run(harness.context(), &mut model, start("diga oi"), &mut grant);

        assert_eq!(state.stop_reason, Some(StopReason::Finished));
        assert_eq!(state.commands.len(), 1);
        assert_eq!(state.commands[0].exit_code, Some(0));
        let results = tool_messages(&model, 1);
        assert!(results[0].content.contains("exit 0"));
        assert!(results[0].content.contains("oi"));

        // The report carries the command as evidence, unvalidated.
        let report = events
            .iter()
            .find_map(|message| match &message.event {
                AgentEvent::TaskFinished { report, .. } => Some(report.clone()),
                _ => None,
            })
            .expect("taskFinished");
        assert_eq!(report.evidence.len(), 1);
        assert_eq!(report.evidence[0].exit_code, Some(0));
        assert!(!report.validated);
        // `run_command` is the one tool whose output rides along in the event.
        assert!(events.iter().any(|message| matches!(
            &message.event,
            AgentEvent::ToolCallFinished { output: Some(text), .. } if text.contains("oi")
        )));
    }

    /// SPEC §34 Phase 5 exit: the loop reads, edits and runs tests on the soma fixture.
    /// The model is scripted — this environment has no Ollama — but the tools, permissions,
    /// evidence and `completed_unvalidated` path are the real ones.
    #[test]
    fn soma_fixture_is_fixed_and_its_tests_run() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../evals/fixtures/soma");
        let package = fs::read_to_string(fixture.join("package.json")).unwrap();
        let soma = fs::read_to_string(fixture.join("src/soma.ts")).unwrap();
        let tests = fs::read_to_string(fixture.join("src/soma.test.ts")).unwrap();
        assert!(
            soma.contains("a - b"),
            "o fixture de aceite ainda precisa do bug proposital"
        );

        let harness = harness(&[
            ("package.json", &package),
            ("src/soma.ts", &soma),
            ("src/soma.test.ts", &tests),
        ]);
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::calls(&[("read_file", serde_json::json!({ "path": "src/soma.ts" }))]),
            ScriptedModel::calls(&[(
                "edit_file",
                serde_json::json!({
                    "path": "src/soma.ts",
                    "old_text": "a - b",
                    "new_text": "a + b",
                }),
            )]),
            ScriptedModel::calls(&[(
                "run_command",
                serde_json::json!({ "argv": ["bun", "test"] }),
            )]),
            ScriptedModel::text(
                "Corrigi `soma` para somar e rodei os testes. Não validado por um Verifier.",
            ),
        ]);
        let (state, events) = run(
            harness.context(),
            &mut model,
            start("O teste de soma falha. Corrija e rode os testes."),
            &mut grant,
        );

        assert_eq!(state.status, TaskStatus::CompletedUnvalidated);
        assert_eq!(state.stop_reason, Some(StopReason::Finished));
        let content = fs::read_to_string(harness.workspace.root().join("src/soma.ts")).unwrap();
        assert!(content.contains("a + b"), "{content}");
        assert!(!content.contains("a - b"), "{content}");
        assert_eq!(state.files_changed.len(), 1);
        assert_eq!(state.files_changed[0].path, "src/soma.ts");
        assert_eq!(state.commands.len(), 1);
        assert_eq!(state.commands[0].argv, ["bun", "test"]);
        assert_eq!(
            state.commands[0].exit_code,
            Some(0),
            "bun test tem de passar depois da correção; stderr da tool: {:?}",
            tool_messages(&model, 3).first().map(|m| &m.content)
        );

        let report = events
            .iter()
            .find_map(|message| match &message.event {
                AgentEvent::TaskFinished { report, .. } => Some(report.clone()),
                _ => None,
            })
            .expect("taskFinished");
        assert!(!report.validated, "a fase 5 nunca valida (D11)");
        assert_eq!(report.files_changed, vec!["src/soma.ts"]);
        assert_eq!(report.evidence[0].exit_code, Some(0));
    }

    #[test]
    fn an_edit_is_recorded_with_its_hash() {
        let harness = harness(&[("src/soma.ts", "export const soma = (a, b) => a - b;\n")]);
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::calls(&[(
                "edit_file",
                serde_json::json!({
                    "path": "src/soma.ts",
                    "old_text": "a - b",
                    "new_text": "a + b",
                }),
            )]),
            ScriptedModel::text("corrigi a soma"),
        ]);
        let (state, _) = run(
            harness.context(),
            &mut model,
            start("corrija a soma"),
            &mut grant,
        );

        assert_eq!(state.stop_reason, Some(StopReason::Finished));
        assert_eq!(state.files_changed.len(), 1);
        assert_eq!(state.files_changed[0].path, "src/soma.ts");
        assert_eq!(state.files_changed[0].hash_after.len(), 64);
        let content = fs::read_to_string(harness.workspace.root().join("src/soma.ts")).unwrap();
        assert!(content.contains("a + b"));
        let results = tool_messages(&model, 1);
        assert!(results[0].content.contains("ok: src/soma.ts"));
    }

    #[test]
    fn a_cancelled_task_never_calls_the_model() {
        let harness = harness(&[]);
        let mut model = ScriptedModel::new(vec![ScriptedModel::text("nunca chamado")]);
        let ctx = harness.context();
        ctx.cancel.cancel();
        let (state, events) = run(ctx, &mut model, start("faça algo"), &mut grant);

        assert_eq!(state.status, TaskStatus::Cancelled);
        assert_eq!(state.stop_reason, Some(StopReason::Cancelled));
        assert_eq!(state.iterations, 0);
        assert!(model.seen.is_empty());
        // The task still opens and closes properly on the channel.
        assert!(matches!(events[0].event, AgentEvent::TaskStarted { .. }));
        assert!(matches!(
            events.last().unwrap().event,
            AgentEvent::TaskFinished { .. }
        ));
    }

    #[test]
    fn steering_reaches_the_model_as_a_user_message() {
        let harness = harness(&[("a.rs", "fn a() {}\n")]);
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::calls(&[("read_file", serde_json::json!({ "path": "a.rs" }))]),
            ScriptedModel::text("ok, mudei de rumo"),
        ]);
        let ctx = harness.context();
        ctx.steer
            .lock()
            .unwrap()
            .push_back("na verdade, olhe só o b.rs".to_string());
        let (state, events) = run(ctx, &mut model, start("leia a.rs"), &mut grant);

        assert_eq!(state.stop_reason, Some(StopReason::Finished));
        // The correction is in the first turn the model saw, right after the request.
        assert_eq!(model.seen[0].len(), 3);
        assert_eq!(model.seen[0][2].role, "user");
        assert_eq!(model.seen[0][2].content, "na verdade, olhe só o b.rs");
        assert_eq!(
            events
                .iter()
                .filter(|message| matches!(&message.event, AgentEvent::UserMessage { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn events_are_numbered_in_order_and_replayable_from_disk() {
        let harness = harness(&[("a.rs", "fn a() {}\n")]);
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::calls(&[("read_file", serde_json::json!({ "path": "a.rs" }))]),
            ScriptedModel::text("pronto"),
        ]);
        let (state, events) = run(harness.context(), &mut model, start("leia"), &mut grant);

        let sequences: Vec<u64> = events.iter().map(|message| message.sequence).collect();
        assert_eq!(
            sequences,
            (1..=events.len() as u64).collect::<Vec<u64>>(),
            "a sequência é densa e crescente"
        );
        assert!(events.iter().all(|message| message.task_id == state.id));

        // The replay of Part E reads exactly the same events back, typed.
        let replayed = harness.store.load_events(&state.id).unwrap();
        assert_eq!(replayed.len(), events.len());
        assert_eq!(replayed[0].sequence, 1);
        assert!(matches!(
            replayed.last().unwrap().event,
            AgentEvent::TaskFinished { .. }
        ));
    }

    #[test]
    fn the_same_error_three_times_is_a_loop() {
        let harness = harness(&[]);
        // Different arguments each time, so only the repeated error can stop it.
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::calls(&[("read_file", serde_json::json!({ "path": "a.rs" }))]),
            ScriptedModel::calls(&[("read_file", serde_json::json!({ "path": "b.rs" }))]),
            ScriptedModel::calls(&[("read_file", serde_json::json!({ "path": "c.rs" }))]),
        ]);
        let (state, _) = run(
            harness.context(),
            &mut model,
            start("leia o que não existe"),
            &mut grant,
        );

        assert_eq!(state.status, TaskStatus::Failed);
        match state.stop_reason {
            Some(StopReason::LoopDetected { ref detail }) => {
                assert!(detail.contains("arquivo não encontrado"), "{detail}");
            }
            other => panic!("esperava loopDetected, veio {other:?}"),
        }
    }

    #[test]
    fn a_task_timeout_stops_the_loop() {
        let harness = harness(&[]);
        let mut model = ScriptedModel::new(vec![ScriptedModel::text("pronto")]);
        let mut ctx = harness.context();
        ctx.limits.task_timeout_ms = 0;
        let (state, _) = run(ctx, &mut model, start("demore"), &mut grant);

        assert_eq!(state.status, TaskStatus::Failed);
        assert_eq!(state.stop_reason, Some(StopReason::TaskTimeout));
        assert!(model.seen.is_empty());
    }

    #[test]
    fn waiting_for_a_human_never_spends_the_task_deadline() {
        let harness = harness(&[]);
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::calls(&[(
                "write_file",
                serde_json::json!({ "path": "novo.txt", "content": "oi" }),
            )]),
            ScriptedModel::text("escrevi o arquivo"),
        ]);
        let mut ctx = harness.context();
        // The person takes longer to answer than the whole task is allowed to last.
        ctx.limits.task_timeout_ms = 1_000;
        let mut slow = |request: ApprovalRequest| {
            std::thread::sleep(Duration::from_millis(1_500));
            grant(request)
        };
        let (state, _) = run(ctx, &mut model, start("crie o arquivo"), &mut slow);

        assert_eq!(state.status, TaskStatus::CompletedUnvalidated);
        assert_eq!(state.stop_reason, Some(StopReason::Finished));
        assert!(harness.workspace.root().join("novo.txt").exists());

        // And the wait is reported as what it is, instead of hiding inside the tool time.
        assert!(
            state.metrics.approval_wait_ms >= 1_400,
            "a espera humana precisa aparecer: {}",
            state.metrics.approval_wait_ms
        );
        assert!(
            state.metrics.tool_ms < 500,
            "tool_ms mede a ferramenta, não a espera: {}",
            state.metrics.tool_ms
        );
    }

    #[test]
    fn real_time_still_counts_against_the_deadline() {
        let harness = harness(&[]);
        let mut model = ScriptedModel::new(vec![ScriptedModel::calls(&[(
            "run_command",
            serde_json::json!({ "argv": test_argv::sleep_secs(1), "timeout_ms": 30_000 }),
        )])]);
        let mut ctx = harness.context();
        ctx.limits.task_timeout_ms = 200;
        let (state, _) = run(ctx, &mut model, start("durma um pouco"), &mut grant);

        assert_eq!(state.status, TaskStatus::Failed);
        assert_eq!(state.stop_reason, Some(StopReason::TaskTimeout));
        // The command ran and its time is tool time, with no human wait to discount.
        assert_eq!(state.commands.len(), 1);
        assert!(state.metrics.tool_ms >= 900, "{}", state.metrics.tool_ms);
        assert_eq!(state.metrics.approval_wait_ms, 0);
    }

    #[test]
    fn an_exhausted_context_stops_the_task() {
        let harness = harness(&[]);
        let mut model = ScriptedModel::new(vec![ScriptedModel::text("pronto")]);
        let (state, _) = run(
            harness.context(),
            &mut model,
            TaskStart::New {
                // The system prompt alone is well past 90% of 16 tokens.
                request: "pedido".to_string(),
                model: "modelo-x".to_string(),
                num_ctx: 16,
                continues: None,
            },
            &mut grant,
        );
        assert_eq!(state.status, TaskStatus::Failed);
        assert_eq!(state.stop_reason, Some(StopReason::ContextExhausted));
    }

    #[test]
    fn a_path_outside_the_workspace_never_reaches_the_disk() {
        let harness = harness(&[]);
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::calls(&[("read_file", serde_json::json!({ "path": "../../fora.txt" }))]),
            ScriptedModel::text("não consegui sair"),
        ]);
        let (state, _) = run(
            harness.context(),
            &mut model,
            start("leia fora"),
            &mut grant,
        );

        assert_eq!(state.stop_reason, Some(StopReason::Finished));
        let results = tool_messages(&model, 1);
        assert!(
            results[0]
                .content
                .contains("erro: caminho fora do workspace"),
            "{}",
            results[0].content
        );
        assert!(state.files_read.is_empty());
    }

    /// The reply the user saw in the conversation: one sentence of prose, the call written as
    /// text, and an orphan `</tool_call>` the model closed without ever opening.
    const REAL_TEXT_CALL: &str = "Vou criar um código TypeScript ainda mais simples e direto ao ponto, como uma função que calcula o aluguel de carro com base em dias e valor diário. <function=write_file>\n<parameter=path>\naluguelCarro/simple.ts\n</parameter>\n<parameter=content>\nexport const aluguel = (dias: number, diaria: number) => dias * diaria;\n</parameter>\n</function>\n</tool_call>";
    const REAL_PROSE: &str = "Vou criar um código TypeScript ainda mais simples e direto ao ponto, como uma função que calcula o aluguel de carro com base em dias e valor diário.";

    fn assistant_messages(events: &[AgentEventMessage]) -> Vec<String> {
        events
            .iter()
            .filter_map(|message| match &message.event {
                AgentEvent::AssistantMessage { content } => Some(content.clone()),
                _ => None,
            })
            .collect()
    }

    fn report_of(events: &[AgentEventMessage]) -> TaskReport {
        events
            .iter()
            .find_map(|message| match &message.event {
                AgentEvent::TaskFinished { report, .. } => Some(report.clone()),
                _ => None,
            })
            .expect("taskFinished")
    }

    #[test]
    fn the_text_markup_never_reaches_the_conversation() {
        let harness = harness(&[]);
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::text(REAL_TEXT_CALL),
            ScriptedModel::text("criei aluguelCarro/simple.ts"),
        ]);
        let (state, events) = run(
            harness.context(),
            &mut model,
            start("calcule o aluguel"),
            &mut grant,
        );

        assert_eq!(state.stop_reason, Some(StopReason::Finished));
        assert!(
            harness
                .workspace
                .root()
                .join("aluguelCarro/simple.ts")
                .exists()
        );

        // The user reads the prose once, and never the markup.
        let seen = assistant_messages(&events);
        assert_eq!(
            seen,
            vec![
                REAL_PROSE.to_string(),
                "criei aluguelCarro/simple.ts".to_string()
            ]
        );
        for junk in ["<function=", "<parameter=", "tool_call"] {
            assert!(
                !seen.iter().any(|text| text.contains(junk)),
                "sobrou {junk} em {seen:?}"
            );
        }

        // The model gets its turn back as prose plus the call in the native shape.
        let assistant = model.seen[1]
            .iter()
            .find(|message| message.role == "assistant")
            .expect("turno do assistente");
        assert_eq!(assistant.content, REAL_PROSE);
        assert_eq!(assistant.tool_calls.len(), 1);
        assert_eq!(assistant.tool_calls[0].function.name, "write_file");
        assert_eq!(
            assistant.tool_calls[0].function.arguments["path"],
            serde_json::json!("aluguelCarro/simple.ts")
        );

        // And the summary of the report is prose too.
        assert_eq!(report_of(&events).summary, "criei aluguelCarro/simple.ts");
    }

    #[test]
    fn a_reply_that_is_only_a_text_call_says_nothing_to_the_user() {
        let harness = harness(&[("a.rs", "fn a() {}\n")]);
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::text(
                "<function=read_file>\n<parameter=path>\na.rs\n</parameter>\n</function>\n</tool_call>",
            ),
            ScriptedModel::text("li o arquivo"),
        ]);
        let (state, events) = run(harness.context(), &mut model, start("leia"), &mut grant);

        assert_eq!(state.stop_reason, Some(StopReason::Finished));
        assert_eq!(state.files_read, vec!["a.rs"]);
        // Only the closing turn is a message: the first one was nothing but markup.
        assert_eq!(
            assistant_messages(&events),
            vec!["li o arquivo".to_string()]
        );
        let assistant = model.seen[1]
            .iter()
            .find(|message| message.role == "assistant")
            .expect("turno do assistente");
        assert!(assistant.content.is_empty(), "{}", assistant.content);
        assert_eq!(assistant.tool_calls.len(), 1);
    }

    #[test]
    fn the_same_content_under_new_names_is_a_loop() {
        let harness = harness(&[]);
        let content = "export function calcularAluguel(dias: number, diaria: number): number {\n  if (dias <= 0) throw new Error(\"dias precisa ser maior que zero\");\n  if (diaria <= 0) throw new Error(\"a diária precisa ser maior que zero\");\n  return dias * diaria;\n}\n\nconsole.log(calcularAluguel(3, 120));\n";
        assert!(content.chars().count() >= LOOP_CONTENT_MIN_CHARS);
        // The real run: the same file five times, with a new name each time.
        let mut model = ScriptedModel::new(
            [
                "aluguelCarro/simple.ts",
                "aluguelCarro/calcularAluguel.ts",
                "aluguelCarro/aluguel.ts",
                "aluguelCarro/index.ts",
                "aluguelCarro/main.ts",
            ]
            .iter()
            .map(|path| {
                ScriptedModel::calls(&[(
                    "write_file",
                    serde_json::json!({ "path": path, "content": content }),
                )])
            })
            .collect(),
        );
        let (state, _) = run(
            harness.context(),
            &mut model,
            start("calcule o aluguel"),
            &mut grant,
        );

        assert_eq!(state.status, TaskStatus::Failed);
        match state.stop_reason {
            Some(StopReason::LoopDetected { ref detail }) => {
                assert!(detail.contains("mesmo conteúdo"), "{detail}");
                assert!(detail.contains("aluguelCarro/aluguel.ts"), "{detail}");
            }
            other => panic!("esperava loopDetected, veio {other:?}"),
        }
        // The third write is stopped before it runs, well short of the five of the real run.
        assert_eq!(state.iterations, 3);
        let root = harness.workspace.root();
        assert!(root.join("aluguelCarro/simple.ts").exists());
        assert!(root.join("aluguelCarro/calcularAluguel.ts").exists());
        assert!(!root.join("aluguelCarro/aluguel.ts").exists());
    }

    #[test]
    fn two_writes_with_different_content_are_not_a_loop() {
        let harness = harness(&[]);
        let source =
            "export function soma(a: number, b: number): number {\n  return a + b;\n}\n".repeat(4);
        let test = "import { soma } from \"./soma\";\n\nit(\"soma\", () => {\n  expect(soma(2, 2)).toBe(4);\n});\n".repeat(4);
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::calls(&[(
                "write_file",
                serde_json::json!({ "path": "src/soma.ts", "content": source }),
            )]),
            ScriptedModel::calls(&[(
                "write_file",
                serde_json::json!({ "path": "src/soma.test.ts", "content": test }),
            )]),
            ScriptedModel::text("criei a função e o teste"),
        ]);
        let (state, _) = run(
            harness.context(),
            &mut model,
            start("escreva a soma e o teste"),
            &mut grant,
        );

        assert_eq!(state.stop_reason, Some(StopReason::Finished));
        let root = harness.workspace.root();
        assert!(root.join("src/soma.ts").exists());
        assert!(root.join("src/soma.test.ts").exists());
    }

    #[test]
    fn repeating_a_short_stub_is_not_a_loop() {
        let harness = harness(&[]);
        // Three identical barrels in a row is normal scaffolding, not a model going in circles.
        let mut model = ScriptedModel::new(
            ["a", "b", "c"]
                .iter()
                .map(|dir| {
                    ScriptedModel::calls(&[(
                        "write_file",
                        serde_json::json!({ "path": format!("src/{dir}/index.ts"), "content": "export {};\n" }),
                    )])
                })
                .chain([ScriptedModel::text("criei os três barrels")])
                .collect(),
        );
        let (state, _) = run(
            harness.context(),
            &mut model,
            start("crie os barrels"),
            &mut grant,
        );

        assert_eq!(state.stop_reason, Some(StopReason::Finished));
        let root = harness.workspace.root();
        for dir in ["a", "b", "c"] {
            assert!(root.join(format!("src/{dir}/index.ts")).exists());
        }
    }

    #[test]
    fn two_calls_in_one_turn_both_run_in_order() {
        let harness = harness(&[("a.rs", "fn a() {}\n"), ("b.rs", "fn b() {}\n")]);
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::calls(&[
                ("read_file", serde_json::json!({ "path": "a.rs" })),
                ("read_file", serde_json::json!({ "path": "b.rs" })),
            ]),
            ScriptedModel::text("li os dois"),
        ]);
        let (state, _) = run(
            harness.context(),
            &mut model,
            start("leia os dois"),
            &mut grant,
        );

        assert_eq!(state.stop_reason, Some(StopReason::Finished));
        assert_eq!(state.files_read, vec!["a.rs", "b.rs"]);
        let results = tool_messages(&model, 1);
        assert_eq!(results.len(), 2);
        assert!(results[0].content.contains("fn a()"));
        assert!(results[1].content.contains("fn b()"));
    }

    // 13 ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn a_continued_task_inherits_the_previous_report_and_not_its_transcript() {
        let harness = harness(&[(
            "src/soma.ts",
            "export const soma = (a, b) => a - b;
",
        )]);
        let mut first = ScriptedModel::new(vec![
            ScriptedModel::calls(&[("read_file", serde_json::json!({ "path": "src/soma.ts" }))]),
            ScriptedModel::text("li a soma; o operador está trocado"),
        ]);
        let (previous, _) = run(
            harness.context(),
            &mut first,
            start("olhe a soma"),
            &mut grant,
        );
        assert_eq!(previous.status, TaskStatus::CompletedUnvalidated);

        let mut second = ScriptedModel::new(vec![ScriptedModel::text("pronto")]);
        let (state, _) = run(
            harness.context(),
            &mut second,
            start_continuing("agora conserte", &previous.id),
            &mut grant,
        );

        assert_eq!(state.continues.as_deref(), Some(previous.id.as_str()));
        let system = &second.seen[0][0].content;
        assert!(system.contains(INHERITED_BEGIN), "o bloco é delimitado");
        assert!(system.contains("olhe a soma"), "o pedido anterior");
        assert!(
            system.contains("li a soma; o operador está trocado"),
            "o resumo anterior"
        );
        // O que não é herdado: a conversa da tarefa anterior (isso seria a opção A).
        assert!(
            !system.contains("export const soma"),
            "nenhum resultado de tool da tarefa anterior viaja junto"
        );
        assert_eq!(second.seen[0][1].content, "agora conserte");
    }

    // 14 ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn a_task_that_continues_nothing_carries_no_inherited_block() {
        let harness = harness(&[(
            "Cargo.toml",
            "[package]
name = \"x\"
",
        )]);
        let mut model = ScriptedModel::new(vec![ScriptedModel::text("pronto")]);
        let (state, _) = run(harness.context(), &mut model, start("olhe"), &mut grant);

        assert_eq!(state.continues, None);
        assert!(!model.seen[0][0].content.contains(INHERITED_BEGIN));
    }

    // 15 ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn the_chain_survives_a_reload_from_disk() {
        let harness = harness(&[(
            "Cargo.toml",
            "[package]
name = \"x\"
",
        )]);
        let mut first = ScriptedModel::new(vec![ScriptedModel::text("olhei")]);
        let (previous, _) = run(harness.context(), &mut first, start("olhe"), &mut grant);
        let mut second = ScriptedModel::new(vec![ScriptedModel::text("pronto")]);
        let (state, _) = run(
            harness.context(),
            &mut second,
            start_continuing("continue", &previous.id),
            &mut grant,
        );

        let loaded = harness.store.load_state(&state.id).unwrap();
        assert_eq!(loaded.continues.as_deref(), Some(previous.id.as_str()));
        let listed = harness
            .store
            .list(&workspace_key(&harness.workspace))
            .unwrap();
        let summary = listed
            .iter()
            .find(|summary| summary.id == state.id)
            .expect("a tarefa está na lista");
        assert_eq!(summary.continues.as_deref(), Some(previous.id.as_str()));
    }

    // 16 ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn continuing_a_task_that_does_not_exist_fails_before_the_model_runs() {
        let harness = harness(&[]);
        let mut model = ScriptedModel::new(vec![ScriptedModel::text("não devia rodar")]);
        let (state, events) = run(
            harness.context(),
            &mut model,
            start_continuing("continue", "task_inexistente"),
            &mut grant,
        );

        assert_eq!(state.status, TaskStatus::Failed);
        assert!(model.seen.is_empty(), "o modelo nunca foi chamado");
        assert!(
            events.is_empty(),
            "uma tarefa que nem começou não emite nada"
        );
    }
}
