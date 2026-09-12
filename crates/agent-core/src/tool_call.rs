use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedToolCall {
    pub name: String,
    pub arguments: BTreeMap<String, String>,
}

/// What a text reply held: the calls that were taken out of it, and the prose that was left.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextToolCalls {
    pub calls: Vec<ParsedToolCall>,
    /// The content without the blocks that became `calls`, trimmed. Only the parser knows which
    /// bytes it consumed, so it is the parser that hands back the clean text: a caller trying to
    /// strip the markup again by hand would either leave residue or eat prose.
    pub prose: String,
}

const FUNCTION_OPEN: &str = "<function=";
const FUNCTION_CLOSE: &str = "</function>";
const PARAMETER_OPEN: &str = "<parameter=";
const PARAMETER_CLOSE: &str = "</parameter>";
/// Wrappers the CODER model sprinkles around its calls, often unpaired (a `</tool_call>` with no
/// opening). They carry nothing, so they never reach the user.
const WRAPPER_TAGS: [&str; 2] = ["<tool_call>", "</tool_call>"];

/// Extracts tool calls a model wrote as text in the `<function=name><parameter=key>value</parameter></function>`
/// format (qwen3-coder), accepting only tool names in `known_tools`, and returns the prose around
/// them.
///
/// - A block without `</function>` (truncated response) is discarded.
/// - A block whose name is not in `known_tools` is discarded, and stays in `prose`: it was not
///   consumed, and the loop shows the model's mistake instead of hiding it.
/// - Loose `<tool_call>`/`</tool_call>` tags are ignored and removed from `prose`.
/// - One leading and one trailing `\n` of each value are stripped; inner whitespace is kept.
pub fn parse_text_tool_calls(content: &str, known_tools: &[&str]) -> TextToolCalls {
    let mut calls = Vec::new();
    let mut prose = String::new();
    // Byte offsets into `content`: `kept` is where the prose not yet copied starts, `search` is
    // where the next opening tag is looked for.
    let mut kept = 0;
    let mut search = 0;

    while let Some(found) = content[search..].find(FUNCTION_OPEN) {
        let open = search + found;
        let name_start = open + FUNCTION_OPEN.len();
        let Some(found_end) = content[name_start..].find('>') else {
            break; // `<function=` sem `>`: não há bloco válido mais adiante
        };
        let name_end = name_start + found_end;
        let name = content[name_start..name_end].trim();
        if !known_tools.contains(&name) {
            search = name_end + 1;
            continue;
        }

        let body = name_end + 1;
        let Some(found_close) = content[body..].find(FUNCTION_CLOSE) else {
            search = body; // bloco cortado: descarta e segue procurando outra chamada
            continue;
        };
        let close = body + found_close;

        calls.push(ParsedToolCall {
            name: name.to_string(),
            arguments: parse_arguments(&content[body..close]),
        });
        prose.push_str(&content[kept..open]);
        kept = close + FUNCTION_CLOSE.len();
        search = kept;
    }
    prose.push_str(&content[kept..]);

    for tag in WRAPPER_TAGS {
        prose = prose.replace(tag, "");
    }
    TextToolCalls {
        calls,
        prose: prose.trim().to_string(),
    }
}

fn parse_arguments(params: &str) -> BTreeMap<String, String> {
    let mut args = BTreeMap::new();
    let mut cursor = params;
    while let Some(param_open) = cursor.find(PARAMETER_OPEN) {
        let after_key_tag = &cursor[param_open + PARAMETER_OPEN.len()..];
        let Some(key_end) = after_key_tag.find('>') else {
            break;
        };
        let key = after_key_tag[..key_end].trim();
        let value_start = &after_key_tag[key_end + 1..];
        let Some(value_end) = value_start.find(PARAMETER_CLOSE) else {
            break;
        };
        let value = strip_single_edge_newline(&value_start[..value_end]);
        args.insert(key.to_string(), value.to_string());
        cursor = &value_start[value_end + PARAMETER_CLOSE.len()..];
    }
    args
}

fn strip_single_edge_newline(value: &str) -> &str {
    let start = value.strip_prefix('\n').unwrap_or(value);
    start.strip_suffix('\n').unwrap_or(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOOLS: &[&str] = &[
        "read_file",
        "search",
        "edit_file",
        "run_command",
        "write_file",
    ];

    #[test]
    fn parses_bare_call() {
        let content = "<function=read_file>\n<parameter=path>\nsrc/app/page.tsx\n</parameter>\n</function>\n</tool_call>";
        let parsed = parse_text_tool_calls(content, TOOLS);
        assert_eq!(parsed.calls.len(), 1);
        assert_eq!(parsed.calls[0].name, "read_file");
        assert_eq!(parsed.calls[0].arguments["path"], "src/app/page.tsx");
        assert!(parsed.prose.is_empty(), "{:?}", parsed.prose);
    }

    #[test]
    fn ignores_leading_prose() {
        let content = "I'll run the test suite using the specified command.\n\n<function=run_command>\n<parameter=command>\ncargo test --workspace\n</parameter>\n</function>\n</tool_call>";
        let parsed = parse_text_tool_calls(content, TOOLS);
        assert_eq!(parsed.calls.len(), 1);
        assert_eq!(parsed.calls[0].name, "run_command");
        assert_eq!(
            parsed.calls[0].arguments["command"],
            "cargo test --workspace"
        );
        assert_eq!(
            parsed.prose,
            "I'll run the test suite using the specified command."
        );
    }

    #[test]
    fn parses_search() {
        let content = "I'll search for all TODO comments in the codebase to help identify pending tasks or areas that need attention.\n\n<function=search>\n<parameter=query>\nTODO\n</parameter>\n</function>\n</tool_call>";
        let parsed = parse_text_tool_calls(content, TOOLS);
        assert_eq!(parsed.calls.len(), 1);
        assert_eq!(parsed.calls[0].name, "search");
        assert_eq!(parsed.calls[0].arguments["query"], "TODO");
    }

    #[test]
    fn prose_only_returns_nothing() {
        let content = "I need to find the file `apps/cli/src/main.rs` and locate the constant `USAGE` that needs to be renamed to `HELP_TEXT`.\n\nFirst, let me search for the USAGE constant in the file:";
        let parsed = parse_text_tool_calls(content, TOOLS);
        assert!(parsed.calls.is_empty());
        assert_eq!(parsed.prose, content.trim());
    }

    #[test]
    fn rejects_unknown_tool() {
        let content =
            "<function=delete_everything>\n<parameter=path>\n/\n</parameter>\n</function>";
        assert!(parse_text_tool_calls(content, TOOLS).calls.is_empty());
    }

    #[test]
    fn drops_truncated_call() {
        let content = "<function=read_file>\n<parameter=path>\nsrc/";
        assert!(parse_text_tool_calls(content, TOOLS).calls.is_empty());
    }

    #[test]
    fn keeps_inner_whitespace() {
        let content = "<function=edit_file>\n<parameter=replace>\n    indented line\n</parameter>\n</function>";
        let parsed = parse_text_tool_calls(content, TOOLS);
        assert_eq!(parsed.calls.len(), 1);
        assert_eq!(parsed.calls[0].arguments["replace"], "    indented line");
    }

    #[test]
    fn parses_multiple_calls_in_order() {
        let content = "<function=read_file>\n<parameter=path>\nCargo.toml\n</parameter>\n</function>\n<function=edit_file>\n<parameter=path>\nCargo.toml\n</parameter>\n<parameter=replace>\nversion = \"0.2.0\"\n</parameter>\n</function>";
        let parsed = parse_text_tool_calls(content, TOOLS);
        assert_eq!(parsed.calls.len(), 2);
        assert_eq!(parsed.calls[0].name, "read_file");
        assert_eq!(parsed.calls[1].name, "edit_file");
        assert_eq!(parsed.calls[1].arguments["path"], "Cargo.toml");
        assert_eq!(parsed.calls[1].arguments["replace"], "version = \"0.2.0\"");
        assert!(parsed.prose.is_empty(), "{:?}", parsed.prose);
    }

    // The reply the user actually saw in the conversation, markup and orphan `</tool_call>` and
    // all: everything but the first sentence is machinery and must not reach the UI.
    #[test]
    fn the_real_reply_keeps_only_its_prose() {
        let content = "Vou criar um código TypeScript ainda mais simples e direto ao ponto, como uma função que calcula o aluguel de carro com base em dias e valor diário. <function=write_file> <parameter=path> aluguelCarro/simple.ts </parameter> <parameter=content> export const aluguel = (dias: number, diaria: number) => dias * diaria; </parameter> </function> </tool_call>";
        let parsed = parse_text_tool_calls(content, TOOLS);

        assert_eq!(parsed.calls.len(), 1);
        assert_eq!(parsed.calls[0].name, "write_file");
        assert_eq!(
            parsed.prose,
            "Vou criar um código TypeScript ainda mais simples e direto ao ponto, como uma função que calcula o aluguel de carro com base em dias e valor diário."
        );
        for junk in ["<function=", "</function>", "<parameter=", "tool_call"] {
            assert!(
                !parsed.prose.contains(junk),
                "sobrou {junk}: {}",
                parsed.prose
            );
        }
    }

    #[test]
    fn prose_between_two_calls_survives() {
        let content = "Primeiro leio.\n<function=read_file>\n<parameter=path>\na.rs\n</parameter>\n</function>\nDepois procuro.\n<function=search>\n<parameter=query>\nTODO\n</parameter>\n</function>";
        let parsed = parse_text_tool_calls(content, TOOLS);
        assert_eq!(parsed.calls.len(), 2);
        assert_eq!(parsed.prose, "Primeiro leio.\n\nDepois procuro.");
    }
}
