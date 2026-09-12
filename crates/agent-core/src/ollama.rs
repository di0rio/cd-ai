use std::time::Duration;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ModelInfo {
    pub name: String,
    #[ts(type = "number")]
    pub size_bytes: u64,
    pub parameter_size: String,
    pub quantization: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct LoadedModel {
    pub name: String,
    #[ts(type = "number")]
    pub size_bytes: u64,
    #[ts(type = "number")]
    pub vram_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct OllamaStatus {
    pub reachable: bool,
    pub version: Option<String>,
    pub models: Vec<ModelInfo>,
    pub loaded: Vec<LoadedModel>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    /// Native tool calls the model asked for, mirroring Ollama's `message.tool_calls`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ModelToolCall>,
    /// Only on `role: "tool"` messages, carrying a tool result back to the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

/// One entry of Ollama's `message.tool_calls`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelToolCall {
    pub function: ModelFunctionCall,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelFunctionCall {
    pub name: String,
    /// Ollama sends the arguments already parsed as a JSON object.
    #[ts(type = "Record<string, unknown>")]
    pub arguments: serde_json::Value,
}

/// One tool offered to the model, in Ollama's function-calling shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ToolSpec {
    pub r#type: String,
    pub function: ToolFunctionSpec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ToolFunctionSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema of the parameters.
    #[ts(type = "Record<string, unknown>")]
    pub parameters: serde_json::Value,
}

impl ToolSpec {
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            r#type: "function".to_string(),
            function: ToolFunctionSpec {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    /// Always explicit: the runtime default can silently truncate the prompt (SPEC §4).
    pub num_ctx: u32,
    #[serde(default)]
    #[ts(optional)]
    pub tools: Option<Vec<ToolSpec>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
pub enum ChatEvent {
    Token {
        content: String,
    },
    Thinking {
        content: String,
    },
    ToolCalls {
        calls: Vec<ModelToolCall>,
    },
    Done {
        #[ts(type = "number")]
        prompt_tokens: u64,
        #[ts(type = "number")]
        gen_tokens: u64,
        #[ts(type = "number")]
        prompt_ms: u64,
        #[ts(type = "number")]
        gen_ms: u64,
    },
    Error {
        message: String,
    },
}

/// Splits an NDJSON byte stream into ChatEvents. Keeps partial lines between chunks.
#[derive(Default)]
pub struct NdjsonChatParser {
    buffer: Vec<u8>,
}

#[derive(Debug, Deserialize)]
struct StreamLine {
    message: Option<StreamMessage>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    prompt_eval_count: u64,
    #[serde(default)]
    prompt_eval_duration: u64,
    #[serde(default)]
    eval_count: u64,
    #[serde(default)]
    eval_duration: u64,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ModelToolCall>,
}

impl NdjsonChatParser {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<ChatEvent> {
        self.buffer.extend_from_slice(chunk);

        let mut events = Vec::new();
        let mut line_start = 0;
        let mut consumed = 0;

        for index in 0..self.buffer.len() {
            if self.buffer[index] == b'\n' {
                let line = &self.buffer[line_start..index];
                consumed = index + 1;
                if !line.is_empty() {
                    events.extend(Self::parse_line(line));
                }
                line_start = consumed;
            }
        }

        if consumed > 0 {
            self.buffer.drain(..consumed);
        }
        events
    }

    fn parse_line(line: &[u8]) -> Vec<ChatEvent> {
        let parsed: StreamLine = match serde_json::from_slice(line) {
            Ok(parsed) => parsed,
            Err(_) => {
                return vec![ChatEvent::Error {
                    message: "resposta inválida do Ollama".to_string(),
                }];
            }
        };

        let mut events = Vec::new();
        if let Some(error) = parsed.error {
            events.push(ChatEvent::Error { message: error });
            return events;
        }
        if let Some(message) = parsed.message {
            if let Some(thinking) = message.thinking.filter(|thinking| !thinking.is_empty()) {
                events.push(ChatEvent::Thinking { content: thinking });
            }
            if !message.content.is_empty() {
                events.push(ChatEvent::Token {
                    content: message.content,
                });
            }
            if !message.tool_calls.is_empty() {
                events.push(ChatEvent::ToolCalls {
                    calls: message.tool_calls,
                });
            }
        }
        if parsed.done {
            events.push(ChatEvent::Done {
                prompt_tokens: parsed.prompt_eval_count,
                gen_tokens: parsed.eval_count,
                prompt_ms: parsed.prompt_eval_duration / 1_000_000,
                gen_ms: parsed.eval_duration / 1_000_000,
            });
        }
        events
    }
}

/// Body for POST /api/chat. `tools` is sent only when the caller offers any: models
/// behave differently once the key is present, even when it is an empty list.
fn request_body(request: &ChatRequest) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": request.model,
        "messages": request.messages,
        "stream": true,
        "options": { "num_ctx": request.num_ctx },
    });
    if let Some(tools) = request.tools.as_ref().filter(|tools| !tools.is_empty()) {
        body["tools"] = serde_json::json!(tools);
    }
    body
}

pub struct OllamaClient {
    base_url: String,
    http: reqwest::Client,
}

#[derive(Debug, serde::Deserialize)]
struct VersionResponse {
    version: String,
}

#[derive(Debug, serde::Deserialize)]
struct TagsResponse {
    models: Vec<Tag>,
}

#[derive(Debug, serde::Deserialize)]
struct Tag {
    name: String,
    size: u64,
    #[serde(default)]
    details: TagDetails,
}

#[derive(Debug, serde::Deserialize, Default)]
struct TagDetails {
    #[serde(default)]
    parameter_size: String,
    #[serde(default)]
    quantization_level: String,
}

#[derive(Debug, serde::Deserialize)]
struct PsResponse {
    models: Vec<Ps>,
}

#[derive(Debug, serde::Deserialize)]
struct Ps {
    name: String,
    size: u64,
    size_vram: u64,
}

impl From<Tag> for ModelInfo {
    fn from(tag: Tag) -> Self {
        Self {
            name: tag.name,
            size_bytes: tag.size,
            parameter_size: tag.details.parameter_size,
            quantization: tag.details.quantization_level,
        }
    }
}

impl From<Ps> for LoadedModel {
    fn from(ps: Ps) -> Self {
        Self {
            name: ps.name,
            size_bytes: ps.size,
            vram_bytes: ps.size_vram,
        }
    }
}

impl OllamaClient {
    /// Refuses non-loopback hosts: the app never sends prompts off the machine (decision 0004).
    pub fn new(base_url: &str) -> Result<Self, String> {
        let url =
            reqwest::Url::parse(base_url).map_err(|_| "URL do Ollama inválida".to_string())?;
        let loopback = matches!(
            url.host_str(),
            Some("127.0.0.1") | Some("localhost") | Some("::1")
        );
        if url.scheme() != "http" || !loopback {
            return Err("o Ollama precisa estar nesta máquina (loopback)".to_string());
        }
        let base_url = base_url.trim_end_matches('/').to_string();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { base_url, http })
    }

    /// Never fails; unreachable => reachable: false + error.
    pub async fn status(&self) -> OllamaStatus {
        let version_url = format!("{}/api/version", self.base_url);
        let version = match self.http.get(&version_url).send().await {
            Ok(response) if response.status().is_success() => {
                match response.json::<VersionResponse>().await {
                    Ok(parsed) => Some(parsed.version),
                    Err(error) => {
                        return OllamaStatus {
                            reachable: true,
                            version: None,
                            models: vec![],
                            loaded: vec![],
                            error: Some(error.to_string()),
                        };
                    }
                }
            }
            Ok(response) => {
                return OllamaStatus {
                    reachable: true,
                    version: None,
                    models: vec![],
                    loaded: vec![],
                    error: Some(format!("Ollama respondeu com status {}", response.status())),
                };
            }
            Err(error) => {
                return OllamaStatus {
                    reachable: false,
                    version: None,
                    models: vec![],
                    loaded: vec![],
                    error: Some(error.to_string()),
                };
            }
        };

        let mut error = None;
        let models = {
            let url = format!("{}/api/tags", self.base_url);
            match self.http.get(&url).send().await {
                Ok(response) if response.status().is_success() => {
                    match response.json::<TagsResponse>().await {
                        Ok(parsed) => parsed.models.into_iter().map(ModelInfo::from).collect(),
                        Err(cause) => {
                            error = Some(cause.to_string());
                            vec![]
                        }
                    }
                }
                Ok(response) => {
                    error = Some(format!("Ollama respondeu com status {}", response.status()));
                    vec![]
                }
                Err(cause) => {
                    error = Some(cause.to_string());
                    vec![]
                }
            }
        };
        let loaded = {
            let url = format!("{}/api/ps", self.base_url);
            match self.http.get(&url).send().await {
                Ok(response) if response.status().is_success() => {
                    match response.json::<PsResponse>().await {
                        Ok(parsed) => parsed.models.into_iter().map(LoadedModel::from).collect(),
                        Err(cause) => {
                            error = Some(cause.to_string());
                            vec![]
                        }
                    }
                }
                Ok(response) => {
                    error = Some(format!("Ollama respondeu com status {}", response.status()));
                    vec![]
                }
                Err(cause) => {
                    error = Some(cause.to_string());
                    vec![]
                }
            }
        };

        OllamaStatus {
            reachable: true,
            version,
            models,
            loaded,
            error,
        }
    }

    /// Streams tokens from the model, invoking `on_event` for each parsed event.
    ///
    /// Uses a separate client without the 5s global timeout: a generation can take minutes. The
    /// caller cancels by dropping this future (e.g. aborting the task); the request then closes.
    pub async fn chat_stream(
        &self,
        request: &ChatRequest,
        mut on_event: impl FnMut(ChatEvent) + Send,
    ) -> Result<(), String> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|error| error.to_string())?;

        let body = request_body(request);

        let url = format!("{}/api/chat", self.base_url);
        let mut response = http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|error| error.to_string())?;

        if !response.status().is_success() {
            let error = response
                .text()
                .await
                .unwrap_or_else(|_| "resposta inválida do Ollama".to_string());
            return Err(error);
        }

        let mut parser = NdjsonChatParser::default();
        while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
            for event in parser.push(&chunk) {
                on_event(event);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_models(json: &str) -> Result<Vec<ModelInfo>, serde_json::Error> {
        let response: TagsResponse = serde_json::from_str(json)?;
        Ok(response.models.into_iter().map(ModelInfo::from).collect())
    }

    fn parse_loaded(json: &str) -> Result<Vec<LoadedModel>, serde_json::Error> {
        let response: PsResponse = serde_json::from_str(json)?;
        Ok(response.models.into_iter().map(LoadedModel::from).collect())
    }

    #[test]
    fn new_accepts_loopback() {
        assert!(OllamaClient::new("http://127.0.0.1:11434").is_ok());
        assert!(OllamaClient::new("http://localhost:11434/").is_ok());
    }

    #[test]
    fn new_refuses_remote_host() {
        assert!(OllamaClient::new("http://192.168.0.10:11434").is_err());
        assert!(OllamaClient::new("https://example.com").is_err());
    }

    #[test]
    fn parses_tags() {
        let json = r#"{"models":[{"name":"qwen3:4b","size":2500000000,"details":{"parameter_size":"4.0B","quantization_level":"Q4_K_M","family":"qwen3"}}]}"#;
        let models = parse_models(json).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "qwen3:4b");
        assert_eq!(models[0].quantization, "Q4_K_M");
    }

    #[test]
    fn parses_tags_without_details() {
        let json = r#"{"models":[{"name":"x","size":1}]}"#;
        let models = parse_models(json).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].parameter_size, "");
    }

    #[test]
    fn parses_ps() {
        let json = r#"{"models":[{"name":"qwen3:4b","size":3900000000,"size_vram":3900000000}]}"#;
        let loaded = parse_loaded(json).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].vram_bytes, 3900000000);
    }

    #[test]
    fn parser_emits_tokens_in_order() {
        let chunk = "{\
            \"message\":{\"role\":\"assistant\",\"content\":\"Ol\"},\"done\":false}\n\
            {\"message\":{\"role\":\"assistant\",\"content\":\"á\"},\"done\":false}\n";
        let events = NdjsonChatParser::default().push(chunk.as_bytes());
        assert_eq!(
            events,
            vec![
                ChatEvent::Token {
                    content: "Ol".to_string()
                },
                ChatEvent::Token {
                    content: "á".to_string()
                },
            ]
        );
    }

    #[test]
    fn parser_keeps_partial_line() {
        let mut parser = NdjsonChatParser::default();
        let events = parser.push(b"{\"message\":{\"role\":\"assistant\",\"conten");
        assert!(events.is_empty());

        let events = parser.push(b"t\":\"Ol\"},\"done\":false}");
        assert!(events.is_empty());

        let events = parser.push(b"\n");
        assert_eq!(
            events,
            vec![ChatEvent::Token {
                content: "Ol".to_string()
            }]
        );
    }

    #[test]
    fn parser_handles_split_utf8() {
        // "á" in UTF-8 is 0xC3 0xA1 (2 bytes). Split the line right between them.
        let mut full_line = b"{\"message\":{\"role\":\"assistant\",\"content\":\"".to_vec();
        full_line.extend_from_slice("á".as_bytes());
        full_line.extend_from_slice(b"\"},\"done\":false}\n");
        let split = full_line
            .iter()
            .position(|byte| *byte == 0xC3)
            .expect("á should contain 0xC3");
        let middle = split + 1;

        let mut parser = NdjsonChatParser::default();
        assert!(parser.push(&full_line[..middle]).is_empty());
        let events = parser.push(&full_line[middle..]);
        assert_eq!(
            events,
            vec![ChatEvent::Token {
                content: "á".to_string()
            }]
        );
    }

    #[test]
    fn parser_emits_done_with_metrics() {
        let mut parser = NdjsonChatParser::default();
        let line = "{\
            \"done\":true,\
            \"prompt_eval_count\":10,\
            \"prompt_eval_duration\":2000000,\
            \"eval_count\":5,\
            \"eval_duration\":1000000\
        }\n";
        let events = parser.push(line.as_bytes());
        assert_eq!(
            events,
            vec![ChatEvent::Done {
                prompt_tokens: 10,
                gen_tokens: 5,
                prompt_ms: 2,
                gen_ms: 1,
            }]
        );
    }

    #[test]
    fn parser_emits_error_line() {
        let mut parser = NdjsonChatParser::default();
        let events = parser.push(b"{\"error\":\"model not found\"}\n");
        assert_eq!(
            events,
            vec![ChatEvent::Error {
                message: "model not found".to_string(),
            }]
        );
    }

    #[test]
    fn parser_separates_thinking() {
        let mut parser = NdjsonChatParser::default();
        let events =
            parser.push(b"{\"message\":{\"content\":\"\",\"thinking\":\"hmm\"},\"done\":false}\n");
        assert_eq!(
            events,
            vec![ChatEvent::Thinking {
                content: "hmm".to_string(),
            }]
        );
    }

    #[test]
    fn request_body_includes_tools_only_when_present() {
        let mut request = ChatRequest {
            model: "m".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "oi".to_string(),
                ..Default::default()
            }],
            num_ctx: 2048,
            tools: None,
        };
        assert!(request_body(&request).get("tools").is_none());

        request.tools = Some(vec![]);
        assert!(request_body(&request).get("tools").is_none());

        request.tools = Some(vec![ToolSpec::function(
            "read_file",
            "lê um arquivo do workspace",
            serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
            }),
        )]);
        let body = request_body(&request);
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "read_file");
        assert_eq!(
            body["tools"][0]["function"]["parameters"]["required"][0],
            "path"
        );
    }

    #[test]
    fn parser_emits_tool_calls() {
        let mut parser = NdjsonChatParser::default();
        let line = "{\
            \"message\":{\"role\":\"assistant\",\"content\":\"\",\
            \"tool_calls\":[{\"function\":{\"name\":\"read_file\",\"arguments\":{\"path\":\"a.rs\"}}}]},\
            \"done\":true}\n";
        let events = parser.push(line.as_bytes());
        assert_eq!(
            events,
            vec![
                ChatEvent::ToolCalls {
                    calls: vec![ModelToolCall {
                        function: ModelFunctionCall {
                            name: "read_file".to_string(),
                            arguments: serde_json::json!({ "path": "a.rs" }),
                        },
                    }],
                },
                ChatEvent::Done {
                    prompt_tokens: 0,
                    gen_tokens: 0,
                    prompt_ms: 0,
                    gen_ms: 0,
                },
            ]
        );
    }

    #[test]
    fn tool_message_serializes_role_and_name() {
        let result = ChatMessage {
            role: "tool".to_string(),
            content: "ok: a.rs".to_string(),
            tool_name: Some("read_file".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["role"], "tool");
        assert_eq!(json["tool_name"], "read_file");
        assert_eq!(json["content"], "ok: a.rs");
        // Empty tool_calls never reach the wire.
        assert!(json.get("tool_calls").is_none());

        let user = ChatMessage {
            role: "user".to_string(),
            content: "oi".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_value(&user).unwrap();
        assert!(json.get("tool_name").is_none());
        assert!(json.get("tool_calls").is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn live_chat_streams_tokens() {
        let client = OllamaClient::new(DEFAULT_BASE_URL).unwrap();
        let request = ChatRequest {
            model: "qwen3:4b".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "responda só: ok".to_string(),
                ..Default::default()
            }],
            num_ctx: 2048,
            tools: None,
        };
        let mut saw_token = false;
        let mut saw_done = false;
        client
            .chat_stream(&request, |event| match event {
                ChatEvent::Token { .. } => saw_token = true,
                ChatEvent::Done { .. } => saw_done = true,
                _ => {}
            })
            .await
            .unwrap();
        assert!(saw_token);
        assert!(saw_done);
    }

    #[tokio::test]
    #[ignore]
    async fn live_status() {
        let client = OllamaClient::new(DEFAULT_BASE_URL).unwrap();
        assert!(client.status().await.reachable);
    }
}
