use std::process::ExitCode;

const USAGE: &str = "uso: cd-ai [--version | --help]";

fn main() -> ExitCode {
    let arg = std::env::args().nth(1);

    match arg.as_deref() {
        Some("--version" | "-V") => {
            let info = agent_core::app_info();
            println!("{} {}", info.name, info.version);
            ExitCode::SUCCESS
        }
        None | Some("--help" | "-h") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("argumento desconhecido: {other}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
