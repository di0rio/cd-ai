use std::io::Write;
use std::process::ExitCode;

use agent_core::ollama::{ChatEvent, ChatMessage, ChatRequest, OllamaClient};

const USAGE: &str = "uso: cd-ai [--version | --help | chat --model <nome> [--ctx <n>] <prompt>]";

const DEFAULT_CTX: u32 = 8192;

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
        Some(other) => {
            eprintln!("argumento desconhecido: {other}\n{USAGE}");
            ExitCode::from(2)
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
                let Some(value) = args.next() else {
                    eprintln!("--model exige um nome\n{USAGE}");
                    return ExitCode::from(2);
                };
                model = Some(value);
            }
            "--ctx" => {
                let Some(value) = args.next() else {
                    eprintln!("--ctx exige um número\n{USAGE}");
                    return ExitCode::from(2);
                };
                let Ok(parsed) = value.parse::<u32>() else {
                    eprintln!("--ctx exige um número: {value}\n{USAGE}");
                    return ExitCode::from(2);
                };
                ctx = parsed;
            }
            other => prompt.push(other.to_string()),
        }
    }

    let Some(model) = model else {
        eprintln!("faltou --model <nome>\n{USAGE}");
        return ExitCode::from(2);
    };

    let raw = std::env::var("OLLAMA_HOST")
        .unwrap_or_else(|_| agent_core::ollama::DEFAULT_BASE_URL.to_string());
    let base = if raw.starts_with("http") {
        raw
    } else {
        format!("http://{raw}")
    };
    let client = match OllamaClient::new(&base) {
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
        }],
        num_ctx: ctx,
    };

    let mut stdout = std::io::stdout();
    let stream = client.chat_stream(&request, |event| match event {
        ChatEvent::Token { content } => {
            print!("{content}");
            let _ = stdout.flush();
        }
        ChatEvent::Thinking { .. } => {}
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
