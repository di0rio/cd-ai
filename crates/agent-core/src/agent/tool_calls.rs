//! The border between what the model says and what the engine runs (plan 015, D3).
//!
//! Two rules hold here, and they are the reason this module exists instead of a `serde` derive:
//!
//! 1. **No raw path, ever.** A tool call is mapped field by field into a `ToolRequest`; the engine
//!    is what resolves paths through `Workspace::resolve`. Nothing the model writes is
//!    deserialized straight into a request.
//! 2. **Only offered tools.** A name outside `tool_specs()` is an error the model reads, not a
//!    call — in the native shape and in the text fallback alike (D2).

use serde_json::{Map, Value};

use crate::ollama::ToolSpec;
use crate::redactor;
use crate::tool_call::ParsedToolCall;
use crate::tools::{
    DEFAULT_COMMAND_TIMEOUT_MS, EditFileArgs, IfExists, ListDirectoryArgs, ReadFileArgs,
    RunCommandArgs, SearchArgs, ToolError, ToolOutcome, ToolOutput, ToolRequest, WriteFileArgs,
};

/// A tool result longer than this is cut before it goes back to the model (plan 015).
pub const MAX_TOOL_RESULT_CHARS: usize = 12_000;

/// The names offered to the model, in the order of the plan's table. The text fallback accepts
/// exactly these and nothing else.
pub const TOOL_NAMES: [&str; 6] = [
    "read_file",
    "list_directory",
    "search",
    "edit_file",
    "write_file",
    "run_command",
];

/// Args that are not strings in the native shape, so the text fallback knows what to parse.
const INTEGER_ARGS: [&str; 4] = ["start_line", "end_line", "max_results", "timeout_ms"];
const BOOLEAN_ARGS: [&str; 1] = ["regex"];
const ARRAY_ARGS: [&str; 1] = ["argv"];

/// The six tools as the model sees them (D3). Descriptions are in English, like the system prompt.
pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec::function(
            "read_file",
            "Read a UTF-8 text file from the workspace. Optionally only the lines from \
             start_line to end_line (1-indexed).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path relative to the workspace root." },
                    "start_line": { "type": "integer", "description": "First line to read, 1-indexed." },
                    "end_line": { "type": "integer", "description": "Last line to read, inclusive." },
                },
                "required": ["path"],
            }),
        ),
        ToolSpec::function(
            "list_directory",
            "List the entries of one directory in the workspace, without recursion. Use \".\" for \
             the workspace root.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory relative to the workspace root; \".\" is the root." },
                },
                "required": ["path"],
            }),
        ),
        ToolSpec::function(
            "search",
            "Search the workspace for a string (or a regular expression), respecting .gitignore.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Text to look for." },
                    "regex": { "type": "boolean", "description": "Treat the query as a regular expression. Default false." },
                    "path": { "type": "string", "description": "Directory to search in. Default: the workspace root." },
                    "max_results": { "type": "integer", "description": "Maximum matches to return." },
                },
                "required": ["query"],
            }),
        ),
        ToolSpec::function(
            "edit_file",
            "Replace an exact block of text in an existing file. old_text must appear exactly \
             once; include surrounding lines to make it unique.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File relative to the workspace root." },
                    "old_text": { "type": "string", "description": "Exact text to replace." },
                    "new_text": { "type": "string", "description": "Text to put in its place." },
                },
                "required": ["path", "old_text", "new_text"],
            }),
        ),
        ToolSpec::function(
            "write_file",
            "Write a whole file. Use it for new files; for existing ones prefer edit_file.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File relative to the workspace root." },
                    "content": { "type": "string", "description": "Full content of the file." },
                    "if_exists": {
                        "type": "string",
                        "enum": ["error", "overwrite"],
                        "description": "What to do when the file already exists. Default \"error\".",
                    },
                },
                "required": ["path", "content"],
            }),
        ),
        ToolSpec::function(
            "run_command",
            "Run one command with no shell: argv is an array of strings, so pipes, &&, redirects \
             and globs do not work. One command per call.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "argv": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Program and arguments, e.g. [\"cargo\", \"test\"].",
                    },
                    "cwd": { "type": "string", "description": "Directory to run in, relative to the workspace root." },
                    "timeout_ms": { "type": "integer", "description": "Timeout in milliseconds." },
                },
                "required": ["argv"],
            }),
        ),
    ]
}

/// Maps one native tool call into a request. The error text goes back to the model as the tool
/// result, so it is written in pt-BR like every other message it reads (SPEC §4.3).
pub fn to_request(name: &str, args: &Value) -> Result<ToolRequest, String> {
    if !TOOL_NAMES.contains(&name) {
        return Err(format!(
            "tool desconhecida: {name}. Disponíveis: {}",
            TOOL_NAMES.join(", ")
        ));
    }
    let args = args
        .as_object()
        .ok_or_else(|| "os argumentos precisam ser um objeto JSON".to_string())?;

    match name {
        "read_file" => Ok(ToolRequest::ReadFile(ReadFileArgs {
            path: required_string(args, "path")?,
            start_line: optional_u64(args, "start_line")?,
            end_line: optional_u64(args, "end_line")?,
        })),
        "list_directory" => Ok(ToolRequest::ListDirectory(ListDirectoryArgs {
            path: required_string(args, "path")?,
        })),
        "search" => Ok(ToolRequest::Search(SearchArgs {
            query: required_string(args, "query")?,
            regex: optional_bool(args, "regex")?.unwrap_or(false),
            path: optional_string(args, "path")?,
            max_results: optional_u64(args, "max_results")?.map(|value| value as usize),
        })),
        "edit_file" => Ok(ToolRequest::EditFile(EditFileArgs {
            path: required_string(args, "path")?,
            old_text: required_string(args, "old_text")?,
            new_text: required_string(args, "new_text")?,
        })),
        "write_file" => Ok(ToolRequest::WriteFile(WriteFileArgs {
            path: required_string(args, "path")?,
            content: required_string(args, "content")?,
            if_exists: match optional_string(args, "if_exists")?.as_deref() {
                None | Some("error") => IfExists::Error,
                Some("overwrite") => IfExists::Overwrite,
                Some(other) => {
                    return Err(format!(
                        "if_exists precisa ser \"error\" ou \"overwrite\", não {other:?}"
                    ));
                }
            },
        })),
        "run_command" => Ok(ToolRequest::RunCommand(RunCommandArgs {
            argv: required_argv(args)?,
            cwd: optional_string(args, "cwd")?,
            timeout_ms: Some(
                optional_u64(args, "timeout_ms")?.unwrap_or(DEFAULT_COMMAND_TIMEOUT_MS),
            ),
        })),
        // `TOOL_NAMES` is checked above, so this is unreachable in practice.
        other => Err(format!("tool desconhecida: {other}")),
    }
}

/// Maps a tool call the model wrote as text (D2, qwen3-coder shape). Every value arrives as a
/// string, so integers and booleans are parsed and `argv` is required to be a JSON array.
pub fn to_request_from_text(call: &ParsedToolCall) -> Result<ToolRequest, String> {
    if !TOOL_NAMES.contains(&call.name.as_str()) {
        return Err(format!(
            "tool desconhecida: {}. Disponíveis: {}",
            call.name,
            TOOL_NAMES.join(", ")
        ));
    }
    let mut args = Map::new();
    for (key, value) in &call.arguments {
        let parsed =
            if INTEGER_ARGS.contains(&key.as_str()) {
                Value::from(value.trim().parse::<u64>().map_err(|_| {
                    format!(
                        "{key} precisa ser um número inteiro, não {:?}",
                        value.trim()
                    )
                })?)
            } else if BOOLEAN_ARGS.contains(&key.as_str()) {
                Value::from(value.trim().parse::<bool>().map_err(|_| {
                    format!("{key} precisa ser true ou false, não {:?}", value.trim())
                })?)
            } else if ARRAY_ARGS.contains(&key.as_str()) {
                // Never split on whitespace: quoting rules are exactly what argv exists to avoid.
                let array: Value = serde_json::from_str(value.trim()).map_err(|_| {
                    format!("{key} precisa ser um array JSON de strings, como [\"cargo\",\"test\"]")
                })?;
                if !array.is_array() {
                    return Err(format!("{key} precisa ser um array JSON de strings"));
                }
                array
            } else {
                Value::from(value.clone())
            };
        args.insert(key.clone(), parsed);
    }
    to_request(&call.name, &Value::Object(args))
}

/// Canonical text of a request, for the loop detection of D7. Keys are sorted (serde_json maps are
/// ordered), so the same call always produces the same signature.
pub fn signature(request: &ToolRequest) -> String {
    match serde_json::to_value(request) {
        Ok(value) => value.to_string(),
        // A request that cannot be serialized cannot be compared either; the name still groups it.
        Err(_) => request.tool_name().to_string(),
    }
}

/// Plain text of an outcome for the model: never the JSON of `ToolOutcome`.
pub fn render_outcome(outcome: &ToolOutcome) -> String {
    if let Some(error) = &outcome.error {
        // A cancelled task gets the bare reason: there is no partial result to report.
        return cut(&format!("erro: {error}"));
    }
    let Some(data) = &outcome.data else {
        return "erro: a tool não devolveu resultado".to_string();
    };

    let mut text = match data {
        ToolOutput::ReadFile(result) => match &result.secret {
            // An approved secret file comes back anonymised (design §6.4), never as content.
            Some(view) => format!(
                "{} (arquivo de secret: só metadados)\n{}",
                result.path,
                serde_json::to_string(view).unwrap_or_else(|_| "{}".to_string())
            ),
            None => format!(
                "{} (linhas {}-{} de {})\n{}",
                result.path, result.start_line, result.end_line, result.total_lines, result.text
            ),
        },
        ToolOutput::ListDirectory(result) => {
            if result.entries.is_empty() {
                "(pasta vazia)".to_string()
            } else {
                result
                    .entries
                    .iter()
                    .map(|entry| {
                        if entry.is_dir {
                            format!("{}/", entry.name)
                        } else {
                            format!("{} ({} bytes)", entry.name, entry.size)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
        ToolOutput::Search(result) => {
            let mut lines = vec![format!("{} resultados", result.total)];
            lines.extend(
                result
                    .matches
                    .iter()
                    .map(|found| format!("{}:{}: {}", found.path, found.line_number, found.line)),
            );
            lines.join("\n")
        }
        ToolOutput::EditFile(result) => format!(
            "ok: {} (+{} −{})",
            result.path, result.added, result.removed
        ),
        ToolOutput::WriteFile(result) => format!("ok: {} ({} bytes)", result.path, result.size),
        ToolOutput::RunCommand(result) => {
            let head = if result.timed_out {
                "timeout".to_string()
            } else {
                match result.exit_code {
                    Some(code) => format!("exit {code}"),
                    None => "cancelado".to_string(),
                }
            };
            if result.output.is_empty() {
                head
            } else {
                format!("{head}\n{}", result.output)
            }
        }
    };

    if let Some(truncation) = &outcome.truncated {
        text.push_str(&format!(
            "\n[truncado: {} de {}; {}]",
            truncation.shown, truncation.total, truncation.how_to_get_more
        ));
    }
    cut(&text)
}

/// One line about an outcome, for the `ToolCallFinished` event and the CLI.
pub fn outcome_detail(outcome: &ToolOutcome) -> String {
    if let Some(error) = &outcome.error {
        return error.to_string();
    }
    match &outcome.data {
        Some(ToolOutput::ReadFile(result)) => {
            format!("{} ({} linhas)", result.path, result.total_lines)
        }
        Some(ToolOutput::ListDirectory(result)) => {
            format!("{} ({} entradas)", result.path, result.entries.len())
        }
        Some(ToolOutput::Search(result)) => format!("{} resultados", result.total),
        Some(ToolOutput::EditFile(result)) => {
            format!("{} (+{} −{})", result.path, result.added, result.removed)
        }
        Some(ToolOutput::WriteFile(result)) => {
            format!("{} ({} bytes)", result.path, result.size)
        }
        Some(ToolOutput::RunCommand(result)) => {
            if result.timed_out {
                "timeout".to_string()
            } else {
                match result.exit_code {
                    Some(code) => format!("exit {code}"),
                    None => "cancelado".to_string(),
                }
            }
        }
        None => "sem resultado".to_string(),
    }
}

/// The output worth keeping in the event stream: only `run_command`, where the tool events carry
/// no output of their own. Everything else is already described by its own `ToolEvent`.
pub fn outcome_output(outcome: &ToolOutcome) -> Option<String> {
    match &outcome.data {
        Some(ToolOutput::RunCommand(result)) if !result.output.is_empty() => {
            Some(cut(&result.output))
        }
        _ => None,
    }
}

/// Redacts every string of a value, for the `input` of `ToolCallRequested`: the arguments the
/// model wrote may quote a secret it read (SPEC §20.6).
pub fn redacted_input(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(redactor::redact(text).text),
        Value::Array(items) => Value::Array(items.iter().map(redacted_input).collect()),
        Value::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, item)| (key.clone(), redacted_input(item)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Cuts a tool result at `MAX_TOOL_RESULT_CHARS`, saying so.
fn cut(text: &str) -> String {
    if text.chars().count() <= MAX_TOOL_RESULT_CHARS {
        return text.to_string();
    }
    let kept: String = text.chars().take(MAX_TOOL_RESULT_CHARS).collect();
    format!("{kept}\n[resultado cortado em {MAX_TOOL_RESULT_CHARS} caracteres]")
}

fn required_string(args: &Map<String, Value>, key: &str) -> Result<String, String> {
    match args.get(key) {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(other) => Err(format!(
            "{key} precisa ser uma string, não {}",
            type_name(other)
        )),
        None => Err(format!("falta o argumento obrigatório {key}")),
    }
}

fn optional_string(args: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(other) => Err(format!(
            "{key} precisa ser uma string, não {}",
            type_name(other)
        )),
    }
}

fn optional_u64(args: &Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("{key} precisa ser um inteiro positivo")),
        Some(other) => Err(format!(
            "{key} precisa ser um número inteiro, não {}",
            type_name(other)
        )),
    }
}

fn optional_bool(args: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(other) => Err(format!(
            "{key} precisa ser true ou false, não {}",
            type_name(other)
        )),
    }
}

/// `argv` is only ever an array of strings: a command line as one string would need quoting rules
/// that `run_command` exists to avoid (design §2.6).
fn required_argv(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let Some(value) = args.get("argv") else {
        return Err("falta o argumento obrigatório argv".to_string());
    };
    let Some(items) = value.as_array() else {
        return Err(
            "argv precisa ser um array de strings, como [\"cargo\",\"test\"]; \
             uma linha de comando em uma string só não é aceita"
                .to_string(),
        );
    };
    if items.is_empty() {
        return Err("argv está vazio".to_string());
    }
    items
        .iter()
        .map(|item| match item {
            Value::String(text) => Ok(text.clone()),
            other => Err(format!(
                "cada item de argv precisa ser uma string, não {}",
                type_name(other)
            )),
        })
        .collect()
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "nulo",
        Value::Bool(_) => "booleano",
        Value::Number(_) => "número",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "objeto",
    }
}

/// The error a tool error becomes when it goes back to the model, as one line.
pub fn render_error(error: &ToolError) -> String {
    format!("erro: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{
        CommandResult, DirEntry, EditFileResult, ListDirectoryResult, ReadFileResult, SearchMatch,
        SearchResult, Truncation, WriteFileResult,
    };
    use std::collections::BTreeMap;

    fn parsed(name: &str, args: &[(&str, &str)]) -> ParsedToolCall {
        ParsedToolCall {
            name: name.to_string(),
            arguments: args
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn specs_cover_the_six_tools_with_required_fields() {
        let specs = tool_specs();
        let names: Vec<&str> = specs
            .iter()
            .map(|spec| spec.function.name.as_str())
            .collect();
        assert_eq!(names, TOOL_NAMES.to_vec());
        for spec in &specs {
            assert_eq!(spec.r#type, "function");
            assert_eq!(spec.function.parameters["type"], "object");
            assert!(spec.function.parameters["required"].is_array());
            assert!(!spec.function.description.is_empty());
        }
        let run = specs.last().unwrap();
        assert_eq!(
            run.function.parameters["properties"]["argv"]["type"],
            "array"
        );
    }

    #[test]
    fn maps_every_tool_from_native_arguments() {
        let request = to_request(
            "read_file",
            &serde_json::json!({ "path": "src/a.rs", "start_line": 2, "end_line": 9 }),
        )
        .unwrap();
        assert_eq!(
            request,
            ToolRequest::ReadFile(ReadFileArgs {
                path: "src/a.rs".to_string(),
                start_line: Some(2),
                end_line: Some(9),
            })
        );

        let request = to_request("list_directory", &serde_json::json!({ "path": "." })).unwrap();
        assert_eq!(
            request,
            ToolRequest::ListDirectory(ListDirectoryArgs {
                path: ".".to_string()
            })
        );

        let request = to_request(
            "search",
            &serde_json::json!({ "query": "TODO", "regex": true, "max_results": 5 }),
        )
        .unwrap();
        assert_eq!(
            request,
            ToolRequest::Search(SearchArgs {
                query: "TODO".to_string(),
                regex: true,
                path: None,
                max_results: Some(5),
            })
        );

        let request = to_request(
            "edit_file",
            &serde_json::json!({ "path": "a.ts", "old_text": "a - b", "new_text": "a + b" }),
        )
        .unwrap();
        assert_eq!(
            request,
            ToolRequest::EditFile(EditFileArgs {
                path: "a.ts".to_string(),
                old_text: "a - b".to_string(),
                new_text: "a + b".to_string(),
            })
        );

        let request = to_request(
            "write_file",
            &serde_json::json!({ "path": "b.ts", "content": "x", "if_exists": "overwrite" }),
        )
        .unwrap();
        assert_eq!(
            request,
            ToolRequest::WriteFile(WriteFileArgs {
                path: "b.ts".to_string(),
                content: "x".to_string(),
                if_exists: IfExists::Overwrite,
            })
        );

        let request = to_request(
            "run_command",
            &serde_json::json!({ "argv": ["cargo", "test"], "cwd": "crates" }),
        )
        .unwrap();
        assert_eq!(
            request,
            ToolRequest::RunCommand(RunCommandArgs {
                argv: vec!["cargo".to_string(), "test".to_string()],
                cwd: Some("crates".to_string()),
                timeout_ms: Some(DEFAULT_COMMAND_TIMEOUT_MS),
            })
        );
    }

    #[test]
    fn unknown_tool_is_refused_in_both_shapes() {
        let error = to_request("delete_everything", &serde_json::json!({ "path": "/" }))
            .expect_err("tool fora da lista");
        assert!(error.starts_with("tool desconhecida"));
        assert!(error.contains("read_file"));

        let error = to_request_from_text(&parsed("delete_everything", &[("path", "/")]))
            .expect_err("tool fora da lista");
        assert!(error.starts_with("tool desconhecida"));
    }

    #[test]
    fn missing_or_mistyped_arguments_are_errors() {
        assert!(
            to_request("read_file", &serde_json::json!({}))
                .unwrap_err()
                .contains("path")
        );
        assert!(
            to_request("read_file", &serde_json::json!({ "path": 7 }))
                .unwrap_err()
                .contains("string")
        );
        assert!(
            to_request("read_file", &serde_json::json!("src/a.rs"))
                .unwrap_err()
                .contains("objeto JSON")
        );
        assert!(
            to_request(
                "search",
                &serde_json::json!({ "query": "x", "regex": "sim" })
            )
            .unwrap_err()
            .contains("true ou false")
        );
        assert!(
            to_request(
                "write_file",
                &serde_json::json!({ "path": "a", "content": "b", "if_exists": "apagar" })
            )
            .unwrap_err()
            .contains("if_exists")
        );
    }

    #[test]
    fn argv_is_only_accepted_as_a_json_array() {
        // A command line in one string is refused, in the native shape and in the text fallback.
        assert!(
            to_request("run_command", &serde_json::json!({ "argv": "cargo test" }))
                .unwrap_err()
                .contains("array de strings")
        );
        assert!(
            to_request_from_text(&parsed("run_command", &[("argv", "cargo test")]))
                .unwrap_err()
                .contains("array JSON")
        );
        assert!(
            to_request("run_command", &serde_json::json!({ "argv": ["cargo", 1] }))
                .unwrap_err()
                .contains("string")
        );
        assert!(
            to_request("run_command", &serde_json::json!({ "argv": [] }))
                .unwrap_err()
                .contains("vazio")
        );

        let request =
            to_request_from_text(&parsed("run_command", &[("argv", "[\"cargo\",\"test\"]")]))
                .unwrap();
        assert_eq!(
            request,
            ToolRequest::RunCommand(RunCommandArgs {
                argv: vec!["cargo".to_string(), "test".to_string()],
                cwd: None,
                timeout_ms: Some(DEFAULT_COMMAND_TIMEOUT_MS),
            })
        );
    }

    #[test]
    fn text_fallback_parses_integers_and_booleans() {
        let request = to_request_from_text(&parsed(
            "read_file",
            &[("path", "src/a.rs"), ("start_line", "3"), ("end_line", "8")],
        ))
        .unwrap();
        assert_eq!(
            request,
            ToolRequest::ReadFile(ReadFileArgs {
                path: "src/a.rs".to_string(),
                start_line: Some(3),
                end_line: Some(8),
            })
        );

        let request =
            to_request_from_text(&parsed("search", &[("query", "TODO"), ("regex", "true")]))
                .unwrap();
        assert_eq!(
            request,
            ToolRequest::Search(SearchArgs {
                query: "TODO".to_string(),
                regex: true,
                path: None,
                max_results: None,
            })
        );

        assert!(
            to_request_from_text(&parsed(
                "read_file",
                &[("path", "a"), ("start_line", "dois")]
            ))
            .unwrap_err()
            .contains("número inteiro")
        );
    }

    #[test]
    fn signature_is_stable_and_separates_arguments() {
        let first = to_request("read_file", &serde_json::json!({ "path": "a.rs" })).unwrap();
        let same = to_request_from_text(&parsed("read_file", &[("path", "a.rs")])).unwrap();
        let other = to_request("read_file", &serde_json::json!({ "path": "b.rs" })).unwrap();
        assert_eq!(signature(&first), signature(&same));
        assert_ne!(signature(&first), signature(&other));
    }

    #[test]
    fn renders_each_outcome_as_plain_text() {
        let read = ToolOutcome::ok(
            ToolOutput::ReadFile(ReadFileResult {
                path: "src/a.rs".to_string(),
                text: "fn main() {}".to_string(),
                start_line: 1,
                end_line: 1,
                total_lines: 1,
                is_truncated: false,
                redacted: 0,
                secret: None,
            }),
            None,
        );
        assert_eq!(
            render_outcome(&read),
            "src/a.rs (linhas 1-1 de 1)\nfn main() {}"
        );

        let list = ToolOutcome::ok(
            ToolOutput::ListDirectory(ListDirectoryResult {
                path: ".".to_string(),
                entries: vec![
                    DirEntry {
                        name: "src".to_string(),
                        is_dir: true,
                        is_file: false,
                        size: 0,
                    },
                    DirEntry {
                        name: "Cargo.toml".to_string(),
                        is_dir: false,
                        is_file: true,
                        size: 42,
                    },
                ],
            }),
            None,
        );
        assert_eq!(render_outcome(&list), "src/\nCargo.toml (42 bytes)");

        let search = ToolOutcome::ok(
            ToolOutput::Search(SearchResult {
                matches: vec![SearchMatch {
                    path: "src/a.rs".to_string(),
                    line_number: 9,
                    line: "// TODO".to_string(),
                }],
                total: 1,
            }),
            None,
        );
        assert_eq!(render_outcome(&search), "1 resultados\nsrc/a.rs:9: // TODO");

        let edit = ToolOutcome::ok(
            ToolOutput::EditFile(EditFileResult {
                path: "a.ts".to_string(),
                changed_lines: 1,
                removed: 1,
                added: 1,
                hash_before: "a".to_string(),
                hash_after: "b".to_string(),
                fuzzy: false,
            }),
            None,
        );
        assert_eq!(render_outcome(&edit), "ok: a.ts (+1 −1)");

        let write = ToolOutcome::ok(
            ToolOutput::WriteFile(WriteFileResult {
                path: "b.ts".to_string(),
                size: 12,
            }),
            None,
        );
        assert_eq!(render_outcome(&write), "ok: b.ts (12 bytes)");

        let command = ToolOutcome::ok(
            ToolOutput::RunCommand(CommandResult {
                id: 0,
                exit_code: Some(1),
                duration_ms: 10,
                output: "falhou\n".to_string(),
                truncated: false,
                timed_out: false,
            }),
            None,
        );
        assert_eq!(render_outcome(&command), "exit 1\nfalhou\n");
    }

    #[test]
    fn renders_timeout_and_errors() {
        let timed_out = ToolOutcome::ok(
            ToolOutput::RunCommand(CommandResult {
                id: 0,
                exit_code: None,
                duration_ms: 30_000,
                output: String::new(),
                truncated: false,
                timed_out: true,
            }),
            None,
        );
        assert_eq!(render_outcome(&timed_out), "timeout");

        // A cancelled task gets the bare reason, with no partial output (plan 015 notes).
        let cancelled = ToolOutcome::err(ToolError::Cancelled);
        assert_eq!(render_outcome(&cancelled), "erro: tarefa cancelada");

        let denied = ToolOutcome::err(ToolError::PermissionDenied {
            reason: "comando negado".to_string(),
        });
        assert_eq!(
            render_outcome(&denied),
            "erro: permissão negada: comando negado"
        );
        assert_eq!(outcome_detail(&denied), "permissão negada: comando negado");
    }

    #[test]
    fn truncation_note_reaches_the_model() {
        let read = ToolOutcome::ok(
            ToolOutput::ReadFile(ReadFileResult {
                path: "big.rs".to_string(),
                text: "linha".to_string(),
                start_line: 1,
                end_line: 1,
                total_lines: 900,
                is_truncated: true,
                redacted: 0,
                secret: None,
            }),
            Some(Truncation {
                shown: 1,
                total: 900,
                how_to_get_more: "read_file com startLine/endLine".to_string(),
            }),
        );
        let text = render_outcome(&read);
        assert!(text.contains("[truncado: 1 de 900; read_file com startLine/endLine]"));
    }

    #[test]
    fn long_results_are_cut_at_the_limit() {
        let read = ToolOutcome::ok(
            ToolOutput::ReadFile(ReadFileResult {
                path: "big.rs".to_string(),
                text: "á".repeat(MAX_TOOL_RESULT_CHARS * 2),
                start_line: 1,
                end_line: 1,
                total_lines: 1,
                is_truncated: false,
                redacted: 0,
                secret: None,
            }),
            None,
        );
        let text = render_outcome(&read);
        assert!(text.ends_with("[resultado cortado em 12000 caracteres]"));
        assert!(text.chars().count() < MAX_TOOL_RESULT_CHARS + 60);
    }

    #[test]
    fn only_run_command_keeps_its_output_in_the_event() {
        let command = ToolOutcome::ok(
            ToolOutput::RunCommand(CommandResult {
                id: 0,
                exit_code: Some(0),
                duration_ms: 1,
                output: "ok\n".to_string(),
                truncated: false,
                timed_out: false,
            }),
            None,
        );
        assert_eq!(outcome_output(&command), Some("ok\n".to_string()));
        let write = ToolOutcome::ok(
            ToolOutput::WriteFile(WriteFileResult {
                path: "b.ts".to_string(),
                size: 1,
            }),
            None,
        );
        assert_eq!(outcome_output(&write), None);
    }

    #[test]
    fn input_of_an_event_is_redacted() {
        let token = "ghp_abcDEF1234567890abcDEF1234567890";
        let input = serde_json::json!({
            "argv": ["curl", format!("--header={token}")],
            "timeout_ms": 1000,
        });
        let redacted = redacted_input(&input);
        let text = redacted.to_string();
        assert!(!text.contains(token));
        assert!(text.contains("[REDIGIDO:segredo]"));
        assert_eq!(redacted["timeout_ms"], 1000);
    }

    #[test]
    fn error_line_uses_the_portuguese_display() {
        assert_eq!(
            render_error(&ToolError::OutsideWorkspace),
            "erro: caminho fora do workspace"
        );
    }
}
