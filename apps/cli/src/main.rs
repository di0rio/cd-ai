use std::cell::RefCell;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use agent_core::agent::{
    AgentEvent, AgentEventMessage, AgentLimits, AgentRole, OllamaModel, StopReason, TaskContext,
    TaskReport, TaskStart, TaskState, TaskStatus, TaskStore, run_task, workspace_key,
};
use agent_core::eval::{EvalDriver, EvalOptions, EvalTask, default_report_path, run_suite};
use agent_core::ollama::{ChatEvent, ChatMessage, ChatRequest, OllamaClient};
use agent_core::permissions::{
    ApprovalAction, ApprovalRequest, ApprovalResponse, CommandClass, PermissionMode,
};
use agent_core::sandbox;
use agent_core::tools::cancel::CancelToken;
use agent_core::workspace::Workspace;
use agent_core::{RollbackResult, rollback_task};

const USAGE: &str = "uso: cd-ai [--version | --help]
     cd-ai chat --model <nome> [--ctx <n>] <prompt>
     cd-ai task --model <nome> [--ctx <n>] [--workspace <pasta>] [--max-iterations <n>] [--mode ask|auto|full-access] [--continue] <pedido…>
     cd-ai task --resume <id> --model <nome> [--ctx <n>] [--workspace <pasta>] [--mode ask|auto|full-access]
     cd-ai history [--workspace <pasta>]
     cd-ai rollback <id> [--workspace <pasta>] [--force]
     cd-ai eval --model <nome> [--suite <pasta>] [--task <id>] [--out <arquivo>] [--ctx <n>]
     cd-ai eval --scripted [--suite <pasta>] [--task <id>] [--out <arquivo>] [--ctx <n>]

--continue: a tarefa nova continua a última deste workspace e herda o relatório dela (pedido,
arquivos alterados, comandos e resumo), nunca a conversa inteira.
--mode: ASK (padrão) pergunta; AUTO edita sozinho e, com sandbox, escreve sozinho; FULL ACCESS
exige sandbox Linux completo. Rede, destrutivo e secrets sempre pedem aprovação.
history: lista as tarefas do workspace, mais recentes primeiro.
rollback: reverte só o que o agente escreveu; mudanças do usuário no mesmo arquivo são
protegidas, a menos que --force. Não toca no git do projeto.
eval: corre a suíte em evals/ sobre cópias descartáveis e aprova sozinho. --scripted não fala com
o Ollama.";

const DEFAULT_CTX: u32 = 8192;

/// Default context for a task (plan 015, D14). Bigger than `chat`'s: a task carries the workspace
/// profile, the tool results and the whole conversation.
const TASK_CTX: u32 = 16_384;

#[tokio::main]
async fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);

    match args.next().as_deref() {
        Some("--version" | "-V") => {
            let info = agent_core::app_info();
            println!("{} {}", info.name, info.version);
            ExitCode::SUCCESS
        }
        None | Some("--help" | "-h") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some("chat") => chat(args).await,
        Some("task") => task(args).await,
        Some("history") => history(args),
        Some("rollback") => rollback(args),
        Some("eval") => eval_cmd(args).await,
        Some(other) => {
            eprintln!("argumento desconhecido: {other}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// The Ollama base URL from `OLLAMA_HOST`, with the scheme filled in when it is missing.
fn ollama_client() -> Result<OllamaClient, String> {
    let raw = std::env::var("OLLAMA_HOST")
        .unwrap_or_else(|_| agent_core::ollama::DEFAULT_BASE_URL.to_string());
    let base = if raw.starts_with("http") {
        raw
    } else {
        format!("http://{raw}")
    };
    OllamaClient::new(&base)
}

/// Reads the value that follows a flag, reporting the usage error when it is missing.
fn flag_value(args: &mut impl Iterator<Item = String>, flag: &str, what: &str) -> Option<String> {
    match args.next() {
        Some(value) => Some(value),
        None => {
            eprintln!("{flag} exige {what}\n{USAGE}");
            None
        }
    }
}

async fn chat(args: impl Iterator<Item = String>) -> ExitCode {
    let mut model = None;
    let mut ctx = DEFAULT_CTX;
    let mut prompt = Vec::new();
    let mut args = args;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => {
                let Some(value) = flag_value(&mut args, "--model", "um nome") else {
                    return ExitCode::from(2);
                };
                model = Some(value);
            }
            "--ctx" => {
                let Some(value) = flag_value(&mut args, "--ctx", "um número") else {
                    return ExitCode::from(2);
                };
                let Ok(parsed) = value.parse::<u32>() else {
                    eprintln!("--ctx exige um número: {value}\n{USAGE}");
                    return ExitCode::from(2);
                };
                ctx = parsed;
            }
            // Same rule as `task`: `--` ends the flags, an unknown one is a usage error.
            "--" => {
                prompt.extend(args.by_ref());
                break;
            }
            other if other.starts_with('-') => {
                eprintln!("flag desconhecida: {other}\n{USAGE}");
                return ExitCode::from(2);
            }
            other => prompt.push(other.to_string()),
        }
    }

    let Some(model) = model else {
        eprintln!("faltou --model <nome>\n{USAGE}");
        return ExitCode::from(2);
    };

    let client = match ollama_client() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };

    let request = ChatRequest {
        model: model.clone(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: prompt.join(" "),
            ..Default::default()
        }],
        num_ctx: ctx,
        tools: None,
    };

    let mut stdout = std::io::stdout();
    let stream = client.chat_stream(&request, |event| match event {
        ChatEvent::Token { content } => {
            print!("{content}");
            let _ = stdout.flush();
        }
        ChatEvent::Thinking { .. } | ChatEvent::ToolCalls { .. } => {}
        ChatEvent::Done {
            gen_tokens, gen_ms, ..
        } => {
            println!();
            let tok_per_s = if gen_ms > 0 {
                gen_tokens as f64 / gen_ms as f64 * 1000.0
            } else {
                0.0
            };
            eprintln!("{gen_tokens} tokens · {tok_per_s:.1} tok/s");
        }
        ChatEvent::Error { message } => eprintln!("\nerro: {message}"),
    });
    tokio::pin!(stream);

    tokio::select! {
        result = &mut stream => match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(1)
            }
        },
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\ncancelado");
            ExitCode::from(130)
        }
    }
}

/// Everything `cd-ai task` accepts, parsed before anything touches the disk or the network.
struct TaskArgs {
    model: String,
    num_ctx: u32,
    workspace: String,
    max_iterations: Option<u32>,
    resume: Option<String>,
    /// Whether this task continues the most recent one of the workspace.
    cont: bool,
    permission_mode: PermissionMode,
    request: String,
}

fn parse_task_args(args: impl Iterator<Item = String>) -> Result<TaskArgs, ExitCode> {
    let mut model = None;
    let mut num_ctx = TASK_CTX;
    let mut workspace = ".".to_string();
    let mut max_iterations = None;
    let mut resume = None;
    let mut cont = false;
    let mut permission_mode = PermissionMode::Ask;
    let mut request = Vec::new();
    let mut args = args;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => {
                let Some(value) = flag_value(&mut args, "--model", "um nome") else {
                    return Err(ExitCode::from(2));
                };
                model = Some(value);
            }
            "--ctx" => num_ctx = parse_number(&mut args, "--ctx")?,
            "--max-iterations" => {
                max_iterations = Some(parse_number(&mut args, "--max-iterations")?);
            }
            "--workspace" => {
                let Some(value) = flag_value(&mut args, "--workspace", "uma pasta") else {
                    return Err(ExitCode::from(2));
                };
                workspace = value;
            }
            "--resume" => {
                let Some(value) = flag_value(&mut args, "--resume", "o id de uma tarefa") else {
                    return Err(ExitCode::from(2));
                };
                resume = Some(value);
            }
            "--continue" => cont = true,
            "--mode" => {
                let Some(value) = flag_value(&mut args, "--mode", "ask, auto ou full-access")
                else {
                    return Err(ExitCode::from(2));
                };
                let Some(mode) = PermissionMode::parse_slug(&value) else {
                    eprintln!("--mode deve ser ask, auto ou full-access\n{USAGE}");
                    return Err(ExitCode::from(2));
                };
                permission_mode = mode;
            }
            // Everything after `--` is the request, so a request may start with a hyphen.
            "--" => {
                request.extend(args.by_ref());
                break;
            }
            // A typo in a flag that picks the model or the workspace must not become request text.
            other if other.starts_with('-') => {
                eprintln!("flag desconhecida: {other}\n{USAGE}");
                return Err(ExitCode::from(2));
            }
            other => request.push(other.to_string()),
        }
    }

    let Some(model) = model else {
        eprintln!("faltou --model <nome>\n{USAGE}");
        return Err(ExitCode::from(2));
    };
    let request = request.join(" ").trim().to_string();
    if resume.is_some() && cont {
        eprintln!(
            "--resume retoma a mesma tarefa e --continue começa outra: escolha um
{USAGE}"
        );
        return Err(ExitCode::from(2));
    }
    if resume.is_some() && !request.is_empty() {
        eprintln!("--resume não aceita um pedido novo\n{USAGE}");
        return Err(ExitCode::from(2));
    }
    if resume.is_none() && request.is_empty() {
        eprintln!("faltou o pedido da tarefa\n{USAGE}");
        return Err(ExitCode::from(2));
    }

    Ok(TaskArgs {
        model,
        num_ctx,
        workspace,
        max_iterations,
        resume,
        cont,
        permission_mode,
        request,
    })
}

/// Everything `cd-ai eval` accepts, parsed before the suite is opened.
struct EvalArgs {
    model: Option<String>,
    scripted: bool,
    suite: String,
    task: Option<String>,
    out: Option<String>,
    num_ctx: u32,
}

fn parse_eval_args(args: impl Iterator<Item = String>) -> Result<EvalArgs, ExitCode> {
    let mut model = None;
    let mut scripted = false;
    let mut suite = "evals".to_string();
    let mut task = None;
    let mut out = None;
    let mut num_ctx = TASK_CTX;
    let mut args = args;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => {
                let Some(value) = flag_value(&mut args, "--model", "um nome") else {
                    return Err(ExitCode::from(2));
                };
                model = Some(value);
            }
            "--scripted" => scripted = true,
            "--suite" => {
                let Some(value) = flag_value(&mut args, "--suite", "uma pasta") else {
                    return Err(ExitCode::from(2));
                };
                suite = value;
            }
            "--task" => {
                let Some(value) = flag_value(&mut args, "--task", "o id de uma tarefa") else {
                    return Err(ExitCode::from(2));
                };
                task = Some(value);
            }
            "--out" => {
                let Some(value) = flag_value(&mut args, "--out", "um arquivo") else {
                    return Err(ExitCode::from(2));
                };
                out = Some(value);
            }
            "--ctx" => num_ctx = parse_number(&mut args, "--ctx")?,
            other if other.starts_with('-') => {
                eprintln!("flag desconhecida: {other}\n{USAGE}");
                return Err(ExitCode::from(2));
            }
            other => {
                eprintln!("argumento inesperado: {other}\n{USAGE}");
                return Err(ExitCode::from(2));
            }
        }
    }

    if scripted && model.is_some() {
        eprintln!("--scripted não aceita --model\n{USAGE}");
        return Err(ExitCode::from(2));
    }
    if !scripted && model.is_none() {
        eprintln!("faltou --model <nome> ou --scripted\n{USAGE}");
        return Err(ExitCode::from(2));
    }

    Ok(EvalArgs {
        model,
        scripted,
        suite,
        task,
        out,
        num_ctx,
    })
}

async fn eval_cmd(args: impl Iterator<Item = String>) -> ExitCode {
    let args = match parse_eval_args(args) {
        Ok(args) => args,
        Err(code) => return code,
    };

    let suite_dir = PathBuf::from(&args.suite);
    let model_name = if args.scripted {
        "scripted".to_string()
    } else {
        args.model.clone().expect("validado em parse_eval_args")
    };
    let out_path = args
        .out
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| default_report_path(&suite_dir, &model_name));

    let driver = if args.scripted {
        EvalDriver::Scripted
    } else {
        let client = match ollama_client() {
            Ok(client) => client,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::from(1);
            }
        };
        EvalDriver::Ollama {
            client,
            runtime: tokio::runtime::Handle::current(),
            turn_timeout: Duration::from_millis(AgentLimits::default().model_turn_timeout_ms),
        }
    };

    let cancel = CancelToken::default();
    let task_cancel = cancel.clone();
    let options = EvalOptions {
        suite_dir,
        out_path: Some(out_path.clone()),
        task_filter: args.task.clone(),
        model_name: model_name.clone(),
        num_ctx: args.num_ctx,
        driver,
        cancel: task_cancel,
    };

    let mut join = tokio::task::spawn_blocking(move || run_suite(options, print_eval_event));

    let finished = tokio::select! {
        joined = &mut join => Some(joined),
        _ = tokio::signal::ctrl_c() => None,
    };

    let Some(joined) = finished else {
        cancel.cancel();
        eprintln!("\ncancelando…");
        let _ = join.await;
        return ExitCode::from(130);
    };

    match joined {
        Ok(Ok(report)) => {
            eprintln!();
            for row in &report.tasks {
                let mark = if row.success { "ok" } else { "falhou" };
                eprintln!(
                    "  {:<12} {mark}  {} iterações  ~{} tok  {} ms",
                    row.id, row.iterations, row.estimated_prompt_tokens, row.duration_ms
                );
            }
            let scored = report.passed + report.failed;
            eprintln!(
                "taxa de sucesso: {:.1}% ({}/{})",
                report.success_rate * 100.0,
                report.passed,
                scored
            );
            eprintln!("resultado: {}", out_path.display());
            if report.all_passed() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Ok(Err(error)) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
        Err(error) => {
            eprintln!("o eval parou de forma inesperada: {error}");
            ExitCode::from(1)
        }
    }
}

fn print_eval_event(task: &EvalTask, message: &AgentEventMessage) {
    match &message.event {
        AgentEvent::ToolCallRequested { tool, input } => {
            let brief = input
                .get("argv")
                .and_then(|value| value.as_array())
                .map(|argv| {
                    let argv: Vec<String> = argv
                        .iter()
                        .map(|item| match item.as_str() {
                            Some(text) => text.to_string(),
                            None => item.to_string(),
                        })
                        .collect();
                    format_argv(&argv)
                })
                .or_else(|| {
                    input
                        .get("query")
                        .and_then(|value| value.as_str())
                        .map(|query| format!("\"{query}\""))
                })
                .or_else(|| {
                    input
                        .get("path")
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_default();
            if brief.is_empty() {
                eprintln!("[{}] → {tool}", task.id);
            } else {
                eprintln!("[{}] → {tool} {brief}", task.id);
            }
        }
        AgentEvent::ToolCallFinished {
            tool, ok, detail, ..
        } => {
            let status = if *ok { "ok" } else { "erro" };
            if detail.is_empty() {
                eprintln!("[{}] → {tool} · {status}", task.id);
            } else {
                eprintln!("[{}] → {tool} · {status} {detail}", task.id);
            }
        }
        AgentEvent::Retrying { attempt, reason } => {
            eprintln!("[{}] tentativa {attempt}: {reason}", task.id);
        }
        AgentEvent::TaskFinished {
            status,
            stop_reason,
            ..
        } => {
            eprintln!(
                "[{}] {} · {}",
                task.id,
                status_label(*status),
                reason_label(stop_reason)
            );
        }
        _ => {}
    }
}

fn parse_number(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<u32, ExitCode> {
    let Some(value) = flag_value(args, flag, "um número") else {
        return Err(ExitCode::from(2));
    };
    match value.parse::<u32>() {
        Ok(parsed) => Ok(parsed),
        Err(_) => {
            eprintln!("{flag} exige um número: {value}\n{USAGE}");
            Err(ExitCode::from(2))
        }
    }
}

fn open_store_and_workspace(
    workspace: &str,
) -> Result<(TaskStore, agent_core::workspace::Workspace), ExitCode> {
    let workspace = match Workspace::open(workspace) {
        Ok(workspace) => workspace,
        Err(error) => {
            eprintln!("{error}");
            return Err(ExitCode::from(1));
        }
    };
    let store = match TaskStore::open_default() {
        Ok(store) => store,
        Err(error) => {
            eprintln!("{error}");
            return Err(ExitCode::from(1));
        }
    };
    Ok((store, workspace))
}

fn parse_workspace_flag(
    args: impl Iterator<Item = String>,
) -> Result<(String, Vec<String>), ExitCode> {
    let mut workspace = ".".to_string();
    let mut rest = Vec::new();
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workspace" => {
                let Some(value) = flag_value(&mut args, "--workspace", "uma pasta") else {
                    return Err(ExitCode::from(2));
                };
                workspace = value;
            }
            "--" => {
                rest.extend(args);
                break;
            }
            other if other.starts_with('-') => {
                eprintln!("flag desconhecida: {other}\n{USAGE}");
                return Err(ExitCode::from(2));
            }
            other => rest.push(other.to_string()),
        }
    }
    Ok((workspace, rest))
}

fn history(args: impl Iterator<Item = String>) -> ExitCode {
    let (workspace, rest) = match parse_workspace_flag(args) {
        Ok(parsed) => parsed,
        Err(code) => return code,
    };
    if !rest.is_empty() {
        eprintln!("history não aceita argumentos posicionais\n{USAGE}");
        return ExitCode::from(2);
    }
    let (store, workspace) = match open_store_and_workspace(&workspace) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let key = workspace_key(&workspace);
    let entries = match store.history(&key) {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };
    if entries.is_empty() {
        println!("nenhuma tarefa neste workspace");
        return ExitCode::SUCCESS;
    }
    for entry in entries {
        let files = if entry.files_changed.is_empty() {
            "nenhum arquivo".to_string()
        } else {
            entry.files_changed.join(", ")
        };
        let checkpoint = entry
            .checkpoint
            .as_deref()
            .map(|sha| &sha[..sha.len().min(8)])
            .unwrap_or("sem checkpoint");
        let rolled = if entry.rolled_back {
            " · revertida"
        } else {
            ""
        };
        println!(
            "{}  {}  {}{rolled}\n  {} · {} comando(s) · checkpoint {checkpoint}",
            entry.summary.id,
            status_label(entry.summary.status),
            entry.summary.title,
            files,
            entry.command_count
        );
    }
    ExitCode::SUCCESS
}

fn rollback(args: impl Iterator<Item = String>) -> ExitCode {
    let mut workspace = ".".to_string();
    let mut force = false;
    let mut id = None;
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workspace" => {
                let Some(value) = flag_value(&mut args, "--workspace", "uma pasta") else {
                    return ExitCode::from(2);
                };
                workspace = value;
            }
            "--force" => force = true,
            "--" => {
                if id.is_none() {
                    id = args.next();
                }
                break;
            }
            other if other.starts_with('-') => {
                eprintln!("flag desconhecida: {other}\n{USAGE}");
                return ExitCode::from(2);
            }
            other => {
                if id.is_some() {
                    eprintln!("rollback aceita um único id\n{USAGE}");
                    return ExitCode::from(2);
                }
                id = Some(other.to_string());
            }
        }
    }
    let Some(id) = id else {
        eprintln!("faltou o id da tarefa\n{USAGE}");
        return ExitCode::from(2);
    };
    let (store, workspace) = match open_store_and_workspace(&workspace) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    match rollback_task(&store, &workspace, &id, force) {
        Ok(result) => {
            print_rollback(&result);
            if result.skipped.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn print_rollback(result: &RollbackResult) {
    if result.restored.is_empty() && result.skipped.is_empty() && result.already_clean.is_empty() {
        println!("nada a reverter (a tarefa não escreveu arquivos)");
        return;
    }
    for path in &result.restored {
        println!("revertido: {path}");
    }
    for path in &result.already_clean {
        println!("já estava no checkpoint: {path}");
    }
    for skip in &result.skipped {
        eprintln!(
            "protegido (mudança do usuário): {} — {}",
            skip.path, skip.reason
        );
        if !skip.diff.is_empty() {
            eprintln!("{}", skip.diff);
        }
        eprintln!("use --force para sobrescrever este arquivo");
    }
}

async fn task(args: impl Iterator<Item = String>) -> ExitCode {
    let args = match parse_task_args(args) {
        Ok(args) => args,
        Err(code) => return code,
    };

    let workspace = match Workspace::open(&args.workspace) {
        Ok(workspace) => workspace,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };
    let store = match TaskStore::open_default() {
        Ok(store) => store,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };
    // A task the app left running is closed before anything else looks at it (D9).
    if let Err(error) = store.recover_interrupted() {
        eprintln!("aviso: não foi possível recuperar tarefas interrompidas: {error}");
    }

    let start = match resolve_start(&store, &workspace, &args) {
        Ok(start) => start,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(1);
        }
    };

    if args.permission_mode == PermissionMode::FullAccess && !sandbox::status().available {
        eprintln!(
            "FULL ACCESS exige sandbox ativo ({})",
            sandbox::status().detail
        );
        return ExitCode::from(1);
    }

    let client = match ollama_client() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };

    let mut limits = AgentLimits::default();
    if let Some(max) = args.max_iterations {
        limits.max_iterations = max;
    }
    let turn_timeout = Duration::from_millis(limits.model_turn_timeout_ms);
    let cancel = CancelToken::default();
    let task_cancel = cancel.clone();
    let approval_cancel = cancel.clone();
    // `run_task` blocks and `OllamaModel` drives the stream with `Handle::block_on`, so the loop
    // needs a thread of its own and the handle of this runtime (D1).
    let handle = tokio::runtime::Handle::current();
    let model_name = args.model.clone();
    let num_ctx = args.num_ctx;
    let permission_mode = args.permission_mode;

    let mut join = tokio::task::spawn_blocking(move || {
        let mut ctx = TaskContext::new(&store, workspace);
        ctx.limits = limits;
        ctx.cancel = task_cancel;
        ctx.permission_mode = permission_mode;
        let mut model = OllamaModel::new(client, handle, model_name, num_ctx, turn_timeout);

        let printer = Rc::new(RefCell::new(Printer::default()));
        let events = printer.clone();
        let mut on_event = move |message: AgentEventMessage| events.borrow_mut().event(&message);
        let mut responder = move |request: ApprovalRequest| {
            printer.borrow_mut().close_lines();
            ask_approval(&request, &approval_cancel)
        };

        run_task(ctx, &mut model, start, &mut on_event, &mut responder)
    });

    let finished = tokio::select! {
        joined = &mut join => Some(joined),
        _ = tokio::signal::ctrl_c() => None,
    };

    let Some(joined) = finished else {
        cancel.cancel();
        eprintln!("\ncancelando…");
        // Waiting matters: the loop still has to kill child processes and save the state.
        let _ = join.await;
        return ExitCode::from(130);
    };

    match joined {
        Ok(state) => ExitCode::from(exit_code(&state)),
        Err(error) => {
            eprintln!("a tarefa parou de forma inesperada: {error}");
            ExitCode::from(1)
        }
    }
}

/// Decides between a new task and a resume. A resume is checked here because `run_task` accepts an
/// unknown id and comes back with an imprecise reason.
fn resolve_start(
    store: &TaskStore,
    workspace: &Workspace,
    args: &TaskArgs,
) -> Result<TaskStart, String> {
    let key = workspace_key(workspace);
    let Some(task_id) = args.resume.clone() else {
        // `--continue` names the workspace's most recent task; the core validates it again and
        // builds what is inherited from its report.
        let continues = if args.cont {
            let last = store
                .list(&key)
                .map_err(|error| format!("não foi possível ler as tarefas: {error}"))?
                .into_iter()
                .next()
                .ok_or("não há tarefa anterior neste workspace para continuar".to_string())?;
            eprintln!("continuando de {} ({})", last.id, last.title);
            Some(last.id)
        } else {
            None
        };
        return Ok(TaskStart::New {
            request: args.request.clone(),
            model: args.model.clone(),
            num_ctx: args.num_ctx,
            continues,
        });
    };
    let state = store
        .load_state(&task_id)
        .map_err(|error| format!("não foi possível retomar {task_id}: {error}"))?;
    if state.workspace != key {
        return Err(format!(
            "a tarefa {task_id} pertence a outro workspace ({}); abra aquela pasta para retomá-la",
            state.workspace
        ));
    }
    Ok(TaskStart::Resume { task_id })
}

fn exit_code(state: &TaskState) -> u8 {
    match state.status {
        TaskStatus::Completed | TaskStatus::CompletedUnvalidated => 0,
        TaskStatus::Cancelled => 130,
        // `run_task` never returns the two live statuses; if it ever did, it did not deliver.
        TaskStatus::Failed | TaskStatus::Running | TaskStatus::WaitingApproval => 1,
    }
}

/// Compact terminal output: the model's text on stdout, everything else on stderr.
///
/// One tool call is one line (`→ read_file src/x.rs · ok`), written in two halves, so it tracks
/// which half-written line is on screen and closes it before anything else prints.
#[derive(Default)]
struct Printer {
    tool_open: bool,
    text_open: bool,
    /// What the open tool line already showed, so the finished half does not repeat it.
    brief: String,
}

impl Printer {
    fn event(&mut self, message: &AgentEventMessage) {
        match &message.event {
            AgentEvent::Token { content } => self.token(content),
            // The brief is built here, inline, because naming `serde_json::Value` would mean a
            // direct dependency the CLI does not otherwise need.
            AgentEvent::ToolCallRequested { tool, input } => {
                let brief = input
                    .get("argv")
                    .and_then(|value| value.as_array())
                    .map(|argv| {
                        let argv: Vec<String> = argv
                            .iter()
                            .map(|item| match item.as_str() {
                                Some(text) => text.to_string(),
                                None => item.to_string(),
                            })
                            .collect();
                        format_argv(&argv)
                    })
                    .or_else(|| {
                        input
                            .get("query")
                            .and_then(|value| value.as_str())
                            .map(|query| format!("\"{query}\""))
                    })
                    .or_else(|| {
                        input
                            .get("path")
                            .and_then(|value| value.as_str())
                            .map(str::to_string)
                    })
                    .unwrap_or_default();
                self.tool_started(tool, brief);
            }
            AgentEvent::ToolCallFinished {
                tool, ok, detail, ..
            } => self.tool_finished(tool, *ok, detail),
            AgentEvent::Retrying { attempt, reason } => {
                self.close_lines();
                eprintln!("tentativa {attempt}: {reason}");
            }
            AgentEvent::ContextTrimmed {
                removed_messages, ..
            } => {
                self.close_lines();
                eprintln!("contexto cortado: {removed_messages} resultado(s) antigo(s) omitido(s)");
            }
            AgentEvent::ContextCompacted {
                estimated_tokens, ..
            } => {
                self.close_lines();
                eprintln!("contexto compactado (~{estimated_tokens} tok)");
            }
            AgentEvent::RoleChanged { role, reason } => {
                self.close_lines();
                let label = match role {
                    AgentRole::Explorer => "Explorer",
                    AgentRole::Coder => "Coder",
                };
                eprintln!("role: {label} ({reason})");
            }
            AgentEvent::TaskFinished {
                status,
                stop_reason,
                report,
            } => self.finished(*status, stop_reason, report),
            _ => {}
        }
    }

    fn token(&mut self, content: &str) {
        self.close_tool_line();
        print!("{content}");
        let _ = std::io::stdout().flush();
        self.text_open = true;
    }

    fn tool_started(&mut self, tool: &str, brief: String) {
        self.close_lines();
        if brief.is_empty() {
            eprint!("→ {tool}");
        } else {
            eprint!("→ {tool} {brief}");
        }
        let _ = std::io::stderr().flush();
        self.tool_open = true;
        self.brief = brief;
    }

    fn tool_finished(&mut self, tool: &str, ok: bool, detail: &str) {
        let status = if ok { "ok" } else { "erro" };
        // On success the detail often repeats the path the request line already showed.
        let repeats = ok && !self.brief.is_empty() && detail.starts_with(self.brief.as_str());
        let extra = if detail.is_empty() || repeats {
            String::new()
        } else {
            format!(" {detail}")
        };
        if self.tool_open {
            eprintln!(" · {status}{extra}");
            self.tool_open = false;
        } else {
            eprintln!("→ {tool} · {status}{extra}");
        }
        self.brief.clear();
    }

    fn finished(&mut self, status: TaskStatus, reason: &StopReason, report: &TaskReport) {
        self.close_lines();
        eprintln!("\n{} · {}", status_label(status), reason_label(reason));
        for command in &report.evidence {
            let code = match command.exit_code {
                Some(code) => format!("exit {code}"),
                None => "sem código de saída".to_string(),
            };
            eprintln!("  {} · {code}", format_argv(&command.argv));
        }
        for path in &report.files_changed {
            eprintln!("  alterado: {path}");
        }
    }

    fn close_tool_line(&mut self) {
        if self.tool_open {
            eprintln!();
            self.tool_open = false;
        }
    }

    fn close_lines(&mut self) {
        if self.text_open {
            println!();
            self.text_open = false;
        }
        self.close_tool_line();
    }
}

/// argv exactly as it will be executed: every element quoted, nothing joined by spaces.
fn format_argv(argv: &[String]) -> String {
    format!("{argv:?}")
}

fn status_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Completed => "concluída",
        TaskStatus::CompletedUnvalidated => "concluída (não validada)",
        TaskStatus::Failed => "falhou",
        TaskStatus::Cancelled => "cancelada",
        TaskStatus::Running => "em andamento",
        TaskStatus::WaitingApproval => "esperando aprovação",
    }
}

fn reason_label(reason: &StopReason) -> String {
    match reason {
        StopReason::Finished => "o modelo respondeu sem chamar tools".to_string(),
        StopReason::Verified => "os checks de validação passaram".to_string(),
        StopReason::MaxIterations => "limite de iterações".to_string(),
        StopReason::TaskTimeout => "tempo limite da tarefa".to_string(),
        StopReason::LoopDetected { detail } => format!("loop detectado: {detail}"),
        StopReason::InvalidToolCalls => "chamadas de tool inválidas demais".to_string(),
        StopReason::ModelError { message } => message.clone(),
        StopReason::ContextExhausted => "contexto esgotado".to_string(),
        StopReason::Cancelled => "cancelada pelo usuário".to_string(),
        StopReason::Interrupted => "interrompida".to_string(),
    }
}

fn class_label(class: &CommandClass) -> &'static str {
    match class {
        CommandClass::Read => "leitura",
        CommandClass::Validate => "validação",
        CommandClass::Write => "escrita",
        CommandClass::Network => "rede",
        CommandClass::Destructive => "destrutivo",
        CommandClass::Unknown => "desconhecido",
    }
}

/// How often the approval wait looks at the cancel token while nobody has typed anything.
const APPROVAL_POLL: Duration = Duration::from_millis(100);

/// The terminal lines, read by one detached thread for the whole process.
///
/// The thread stays parked in `read_line` until something is typed, and on Windows that read is
/// *not* woken by Ctrl+C once tokio installs its handler (the console default is suppressed).
/// Reading here, off the task thread, is what lets the wait below give up on cancellation; the
/// thread is detached on purpose, so a parked read never holds the process back from exiting.
fn stdin_lines() -> &'static Mutex<Receiver<String>> {
    static LINES: OnceLock<Mutex<Receiver<String>>> = OnceLock::new();
    LINES.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            loop {
                let mut line = String::new();
                match std::io::stdin().read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Mutex::new(receiver)
    })
}

/// One typed line, or `None` when the task was cancelled or stdin ended.
fn wait_for_line(lines: &Receiver<String>, cancel: &CancelToken) -> Option<String> {
    loop {
        if cancel.is_cancelled() {
            return None;
        }
        match lines.recv_timeout(APPROVAL_POLL) {
            Ok(line) => return Some(line),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// Asks the user in the terminal, showing the exact action (D13).
///
/// Without an interactive stdin there is nobody to ask, so the answer is a denial: the CLI has no
/// auto-approval flag, by design (decision 0001).
fn ask_approval(request: &ApprovalRequest, cancel: &CancelToken) -> ApprovalResponse {
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        eprintln!("sem terminal interativo: negado");
        return ApprovalResponse::Denied {
            reason: Some("sem terminal interativo".to_string()),
        };
    }

    eprintln!();
    match &request.action {
        ApprovalAction::RunCommand { argv, class, cwd } => {
            eprintln!("rodar comando ({}):", class_label(class));
            eprintln!("  argv: {}", format_argv(argv));
            eprintln!("  cwd:  {cwd}");
        }
        ApprovalAction::EditFile { path, diff } => {
            eprintln!("editar {path}:");
            for line in diff.lines() {
                eprintln!("  {line}");
            }
        }
        ApprovalAction::WriteFile { path, size } => {
            eprintln!("escrever {path} ({size} bytes)");
        }
        ApprovalAction::ReadFile { path } => eprintln!("ler {path}"),
    }
    eprint!("Aprovar? [s/N] ");
    let _ = std::io::stderr().flush();

    let lines = stdin_lines()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(answer) = wait_for_line(&lines, cancel) else {
        if cancel.is_cancelled() {
            eprintln!("\ncancelado: negado");
            return ApprovalResponse::Denied {
                reason: Some("cancelado pelo usuário".to_string()),
            };
        }
        eprintln!("não foi possível ler a resposta: negado");
        return ApprovalResponse::Denied {
            reason: Some("não foi possível ler a resposta".to_string()),
        };
    };
    match answer.trim().to_lowercase().as_str() {
        "s" | "sim" | "y" | "yes" => ApprovalResponse::Granted,
        _ => ApprovalResponse::Denied {
            reason: Some("negado no terminal".to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_is_required_and_a_new_task_needs_a_request() {
        let args = ["--model", "qwen3", "arrume", "a", "soma"].map(String::from);
        let parsed = parse_task_args(args.into_iter()).expect("argumentos válidos");
        assert_eq!(parsed.model, "qwen3");
        assert_eq!(parsed.request, "arrume a soma");
        // D14: 16384 unless asked otherwise.
        assert_eq!(parsed.num_ctx, TASK_CTX);
        assert_eq!(parsed.workspace, ".");
        assert!(parsed.resume.is_none());
        assert_eq!(parsed.permission_mode, PermissionMode::Ask);

        let args = ["--model", "qwen3"].map(String::from);
        assert!(parse_task_args(args.into_iter()).is_err());
        let args = ["arrume", "a", "soma"].map(String::from);
        assert!(parse_task_args(args.into_iter()).is_err());
    }

    #[test]
    fn resume_takes_no_request_and_numbers_must_parse() {
        let args = ["--resume", "task_1", "--model", "qwen3"].map(String::from);
        let parsed = parse_task_args(args.into_iter()).expect("argumentos válidos");
        assert_eq!(parsed.resume.as_deref(), Some("task_1"));
        assert!(parsed.request.is_empty());

        let args = ["--resume", "task_1", "--model", "qwen3", "de", "novo"].map(String::from);
        assert!(parse_task_args(args.into_iter()).is_err());
        let args = ["--model", "qwen3", "--ctx", "muito", "oi"].map(String::from);
        assert!(parse_task_args(args.into_iter()).is_err());
        let args = ["--model", "qwen3", "--max-iterations", "4", "oi"].map(String::from);
        let parsed = parse_task_args(args.into_iter()).expect("argumentos válidos");
        assert_eq!(parsed.max_iterations, Some(4));
    }

    #[test]
    fn an_unknown_flag_is_a_usage_error_and_double_dash_ends_the_flags() {
        let args = ["--model", "qwen3", "--modl", "x", "oi"].map(String::from);
        assert!(parse_task_args(args.into_iter()).is_err());

        let args = ["--model", "qwen3", "--", "--arrume", "a", "soma"].map(String::from);
        let parsed = parse_task_args(args.into_iter()).expect("argumentos válidos");
        assert_eq!(parsed.request, "--arrume a soma");
        assert_eq!(parsed.model, "qwen3");
    }

    #[test]
    fn mode_flag_parses_and_rejects_unknown_values() {
        let args = ["--model", "qwen3", "--mode", "auto", "oi"].map(String::from);
        let parsed = parse_task_args(args.into_iter()).expect("argumentos válidos");
        assert_eq!(parsed.permission_mode, PermissionMode::Auto);

        let args = ["--model", "qwen3", "--mode", "full-access", "oi"].map(String::from);
        let parsed = parse_task_args(args.into_iter()).expect("argumentos válidos");
        assert_eq!(parsed.permission_mode, PermissionMode::FullAccess);

        let args = ["--model", "qwen3", "--mode", "yolo", "oi"].map(String::from);
        assert!(parse_task_args(args.into_iter()).is_err());
    }

    #[test]
    fn the_approval_wait_gives_up_when_the_task_is_cancelled() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let cancel = CancelToken::default();
        sender
            .send("s\n".to_string())
            .expect("enfileirar a resposta");
        assert_eq!(wait_for_line(&receiver, &cancel).as_deref(), Some("s\n"));

        let waiter = cancel.clone();
        let ticker = std::thread::spawn(move || {
            std::thread::sleep(APPROVAL_POLL);
            waiter.cancel();
        });
        // Nothing is ever sent: only the cancellation ends this wait.
        assert!(wait_for_line(&receiver, &cancel).is_none());
        ticker.join().expect("a thread de cancelamento termina");

        // A closed stdin ends it too, instead of waiting forever.
        drop(sender);
        assert!(wait_for_line(&receiver, &CancelToken::default()).is_none());
    }

    #[test]
    fn exit_codes_follow_the_status() {
        let mut state = TaskState::new("task_1", "C:/p", "pedido", "qwen3", TASK_CTX);
        state.status = TaskStatus::CompletedUnvalidated;
        assert_eq!(exit_code(&state), 0);
        state.status = TaskStatus::Completed;
        assert_eq!(exit_code(&state), 0);
        state.status = TaskStatus::Cancelled;
        assert_eq!(exit_code(&state), 130);
        state.status = TaskStatus::Failed;
        assert_eq!(exit_code(&state), 1);
    }

    #[test]
    fn every_stop_reason_has_a_label() {
        assert_eq!(
            reason_label(&StopReason::LoopDetected {
                detail: "read_file a.rs".to_string()
            }),
            "loop detectado: read_file a.rs"
        );
        assert_eq!(
            reason_label(&StopReason::ModelError {
                message: "sem resposta".to_string()
            }),
            "sem resposta"
        );
        assert!(!reason_label(&StopReason::Finished).is_empty());
        assert_eq!(
            reason_label(&StopReason::Verified),
            "os checks de validação passaram"
        );
        assert_eq!(status_label(TaskStatus::Failed), "falhou");
        assert_eq!(status_label(TaskStatus::Completed), "concluída");
        assert_eq!(class_label(&CommandClass::Validate), "validação");
        assert_eq!(
            format_argv(&["bun".to_string(), "test".to_string()]),
            "[\"bun\", \"test\"]"
        );
    }

    #[test]
    fn eval_requires_model_or_scripted_and_rejects_both() {
        assert!(parse_eval_args(std::iter::empty()).is_err());
        let args = ["--scripted"].map(String::from);
        let parsed = parse_eval_args(args.into_iter()).expect("scripted válido");
        assert!(parsed.scripted);
        assert_eq!(parsed.suite, "evals");
        assert_eq!(parsed.num_ctx, TASK_CTX);

        let args = ["--model", "qwen3-coder:30b", "--task", "soma"].map(String::from);
        let parsed = parse_eval_args(args.into_iter()).expect("model válido");
        assert_eq!(parsed.model.as_deref(), Some("qwen3-coder:30b"));
        assert_eq!(parsed.task.as_deref(), Some("soma"));

        let args = ["--scripted", "--model", "qwen3"].map(String::from);
        assert!(parse_eval_args(args.into_iter()).is_err());
    }
}
