use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedToolCall {
    pub name: String,
    pub arguments: BTreeMap<String, String>,
}

/// Extracts tool calls a model wrote as text in the `<function=name><parameter=key>value</parameter></function>`
/// format (qwen3-coder), accepting only tool names in `known_tools`. Prose around the calls is ignored.
///
/// - A block without `</function>` (truncated response) is discarded.
/// - A block whose name is not in `known_tools` is discarded.
/// - Loose `<tool_call>`/`</tool_call>` tags are ignored.
/// - One leading and one trailing `\n` of each value are stripped; inner whitespace is kept.
pub fn parse_text_tool_calls(content: &str, known_tools: &[&str]) -> Vec<ParsedToolCall> {
    let mut calls = Vec::new();
    let mut rest = content;

    while let Some(open) = rest.find("<function=") {
        let after_name_tag = &rest[open + "<function=".len()..];
        let Some(name_end) = after_name_tag.find('>') else {
            break; // `<function=` sem `>`: não há bloco válido mais adiante
        };
        let name = after_name_tag[..name_end].trim();
        if !known_tools.contains(&name) {
            rest = &after_name_tag[name_end + 1..];
            continue;
        }

        let body = &after_name_tag[name_end + 1..];
        let Some(close) = body.find("</function>") else {
            rest = body; // bloco cortado: descarta e segue procurando outra chamada
            continue;
        };

        let params = &body[..close];
        let mut args = BTreeMap::new();
        let mut cursor = params;
        while let Some(param_open) = cursor.find("<parameter=") {
            let after_key_tag = &cursor[param_open + "<parameter=".len()..];
            let Some(key_end) = after_key_tag.find('>') else {
                break;
            };
            let key = after_key_tag[..key_end].trim();
            let value_start = &after_key_tag[key_end + 1..];
            let Some(value_end) = value_start.find("</parameter>") else {
                break;
            };
            let value = strip_single_edge_newline(&value_start[..value_end]);
            args.insert(key.to_string(), value.to_string());
            cursor = &value_start[value_end + "</parameter>".len()..];
        }

        calls.push(ParsedToolCall {
            name: name.to_string(),
            arguments: args,
        });
        rest = &body[close + "</function>".len()..];
    }

    calls
}

fn strip_single_edge_newline(value: &str) -> &str {
    let start = value.strip_prefix('\n').unwrap_or(value);
    start.strip_suffix('\n').unwrap_or(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOOLS: &[&str] = &["read_file", "search", "edit_file", "run_command"];

    #[test]
    fn parses_bare_call() {
        let content = "<function=read_file>\n<parameter=path>\nsrc/app/page.tsx\n</parameter>\n</function>\n</tool_call>";
        let call = parse_text_tool_calls(content, TOOLS);
        assert_eq!(call.len(), 1);
        assert_eq!(call[0].name, "read_file");
        assert_eq!(call[0].arguments["path"], "src/app/page.tsx");
    }

    #[test]
    fn ignores_leading_prose() {
        let content = "I'll run the test suite using the specified command.\n\n<function=run_command>\n<parameter=command>\ncargo test --workspace\n</parameter>\n</function>\n</tool_call>";
        let call = parse_text_tool_calls(content, TOOLS);
        assert_eq!(call.len(), 1);
        assert_eq!(call[0].name, "run_command");
        assert_eq!(call[0].arguments["command"], "cargo test --workspace");
    }

    #[test]
    fn parses_search() {
        let content = "I'll search for all TODO comments in the codebase to help identify pending tasks or areas that need attention.\n\n<function=search>\n<parameter=query>\nTODO\n</parameter>\n</function>\n</tool_call>";
        let call = parse_text_tool_calls(content, TOOLS);
        assert_eq!(call.len(), 1);
        assert_eq!(call[0].name, "search");
        assert_eq!(call[0].arguments["query"], "TODO");
    }

    #[test]
    fn prose_only_returns_nothing() {
        let content = "I need to find the file `apps/cli/src/main.rs` and locate the constant `USAGE` that needs to be renamed to `HELP_TEXT`.\n\nFirst, let me search for the USAGE constant in the file:";
        assert!(parse_text_tool_calls(content, TOOLS).is_empty());
    }

    #[test]
    fn rejects_unknown_tool() {
        let content =
            "<function=delete_everything>\n<parameter=path>\n/\n</parameter>\n</function>";
        assert!(parse_text_tool_calls(content, TOOLS).is_empty());
    }

    #[test]
    fn drops_truncated_call() {
        let content = "<function=read_file>\n<parameter=path>\nsrc/";
        assert!(parse_text_tool_calls(content, TOOLS).is_empty());
    }

    #[test]
    fn keeps_inner_whitespace() {
        let content = "<function=edit_file>\n<parameter=replace>\n    indented line\n</parameter>\n</function>";
        let call = parse_text_tool_calls(content, TOOLS);
        assert_eq!(call.len(), 1);
        assert_eq!(call[0].arguments["replace"], "    indented line");
    }

    #[test]
    fn parses_multiple_calls_in_order() {
        let content = "<function=read_file>\n<parameter=path>\nCargo.toml\n</parameter>\n</function>\n<function=edit_file>\n<parameter=path>\nCargo.toml\n</parameter>\n<parameter=replace>\nversion = \"0.2.0\"\n</parameter>\n</function>";
        let calls = parse_text_tool_calls(content, TOOLS);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[1].name, "edit_file");
        assert_eq!(calls[1].arguments["path"], "Cargo.toml");
        assert_eq!(calls[1].arguments["replace"], "version = \"0.2.0\"");
    }
}
