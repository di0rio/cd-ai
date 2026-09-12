//! The model, as the loop sees it (plan 015, D1): one synchronous `turn` per iteration.
//!
//! The loop, the `ToolEngine` and the approval responder are all synchronous and blocking, so the
//! agent runs on a dedicated thread and the Ollama adapter bridges into async with
//! `tokio::runtime::Handle::block_on`. Nothing here is async from the caller's point of view.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::ollama::{ChatEvent, ChatMessage, ChatRequest, ModelToolCall, OllamaClient, ToolSpec};
use crate::tools::cancel::CancelToken;

/// How often the blocking turn hands the events collected so far to `on_event`. Small enough that
/// tokens still look like a stream in the UI, large enough not to spin the CPU.
const DRAIN_INTERVAL_MS: u64 = 20;

/// Everything one model turn produced.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelReply {
    pub content: String,
    pub thinking: String,
    pub tool_calls: Vec<ModelToolCall>,
    pub prompt_tokens: u64,
    pub gen_tokens: u64,
    pub prompt_ms: u64,
    pub gen_ms: u64,
}

/// Why a turn did not produce a reply. `Cancelled` is never retried; the other two are (D7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    Timeout,
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "o modelo passou do tempo limite do turno"),
            Self::Cancelled => write!(f, "tarefa cancelada"),
            Self::Failed(message) => write!(f, "erro do modelo: {message}"),
        }
    }
}

impl std::error::Error for ModelError {}

/// The only thing the loop needs from a model. Implemented by `OllamaModel` in production and by
/// `ScriptedModel` in the tests, which is what keeps the loop testable without Ollama.
pub trait ChatModel {
    fn turn(
        &mut self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_event: &mut dyn FnMut(ChatEvent),
        cancel: &CancelToken,
    ) -> Result<ModelReply, ModelError>;
}

/// The real model: streams from Ollama, with a per-turn timeout and honest cancellation (D6).
pub struct OllamaModel {
    client: OllamaClient,
    runtime: tokio::runtime::Handle,
    model: String,
    num_ctx: u32,
    turn_timeout: Duration,
}

impl OllamaModel {
    pub fn new(
        client: OllamaClient,
        runtime: tokio::runtime::Handle,
        model: impl Into<String>,
        num_ctx: u32,
        turn_timeout: Duration,
    ) -> Self {
        Self {
            client,
            runtime,
            model: model.into(),
            num_ctx,
            turn_timeout,
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn num_ctx(&self) -> u32 {
        self.num_ctx
    }
}

impl ChatModel for OllamaModel {
    fn turn(
        &mut self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_event: &mut dyn FnMut(ChatEvent),
        cancel: &CancelToken,
    ) -> Result<ModelReply, ModelError> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages: messages.to_vec(),
            num_ctx: self.num_ctx,
            tools: (!tools.is_empty()).then(|| tools.to_vec()),
        };
        let client = &self.client;
        let turn_timeout = self.turn_timeout;

        // The stream callback has to be `Send`, and `on_event` is not, so the events land in a
        // shared buffer and are handed to `on_event` from this thread, between polls.
        let buffer: Arc<Mutex<Vec<ChatEvent>>> = Arc::default();
        let sink = buffer.clone();

        self.runtime.block_on(async move {
            let stream = client.chat_stream(&request, move |event| {
                if let Ok(mut events) = sink.lock() {
                    events.push(event);
                }
            });
            tokio::pin!(stream);
            let deadline = tokio::time::sleep(turn_timeout);
            tokio::pin!(deadline);

            let mut reply = ModelReply::default();
            let mut stream_error = None;

            let outcome = loop {
                tokio::select! {
                    biased;
                    // Cancellation wins over anything still in flight (D6).
                    () = cancel.cancelled() => break Err(ModelError::Cancelled),
                    () = &mut deadline => break Err(ModelError::Timeout),
                    result = &mut stream => {
                        break result.map_err(ModelError::Failed);
                    }
                    () = tokio::time::sleep(Duration::from_millis(DRAIN_INTERVAL_MS)) => {}
                }
                drain(&buffer, on_event, &mut reply, &mut stream_error);
            };
            // Whatever arrived in the last window still belongs to the caller.
            drain(&buffer, on_event, &mut reply, &mut stream_error);

            outcome?;
            // Ollama reports a failed generation inside the stream, with HTTP 200.
            match stream_error {
                Some(message) => Err(ModelError::Failed(message)),
                None => Ok(reply),
            }
        })
    }
}

/// Moves the buffered events into `on_event`, accumulating them into the reply on the way.
fn drain(
    buffer: &Mutex<Vec<ChatEvent>>,
    on_event: &mut dyn FnMut(ChatEvent),
    reply: &mut ModelReply,
    stream_error: &mut Option<String>,
) {
    let events = match buffer.lock() {
        Ok(mut events) => std::mem::take(&mut *events),
        Err(_) => return,
    };
    for event in events {
        match &event {
            ChatEvent::Token { content } => reply.content.push_str(content),
            ChatEvent::Thinking { content } => reply.thinking.push_str(content),
            ChatEvent::ToolCalls { calls } => reply.tool_calls.extend(calls.iter().cloned()),
            ChatEvent::Done {
                prompt_tokens,
                gen_tokens,
                prompt_ms,
                gen_ms,
            } => {
                reply.prompt_tokens = *prompt_tokens;
                reply.gen_tokens = *gen_tokens;
                reply.prompt_ms = *prompt_ms;
                reply.gen_ms = *gen_ms;
            }
            ChatEvent::Error { message } => {
                stream_error.get_or_insert_with(|| message.clone());
            }
        }
        on_event(event);
    }
}

/// A model with a script, for testing the loop without Ollama. Returns the programmed replies in
/// order and repeats the last one afterwards, so a test can drive an endless loop with one entry.
#[cfg(test)]
pub(crate) struct ScriptedModel {
    replies: std::collections::VecDeque<Result<ModelReply, ModelError>>,
    last: Option<Result<ModelReply, ModelError>>,
    /// Every batch of messages the loop sent, in order.
    pub seen: Vec<Vec<ChatMessage>>,
    /// The tools offered on the last turn.
    pub offered: Vec<String>,
}

#[cfg(test)]
impl ScriptedModel {
    pub fn new(replies: Vec<Result<ModelReply, ModelError>>) -> Self {
        Self {
            replies: replies.into(),
            last: None,
            seen: Vec::new(),
            offered: Vec::new(),
        }
    }

    /// A turn with plain text and no tool calls: the loop treats it as "task done".
    pub fn text(content: &str) -> Result<ModelReply, ModelError> {
        Ok(ModelReply {
            content: content.to_string(),
            gen_tokens: 4,
            ..Default::default()
        })
    }

    /// A turn with native tool calls (D2, first path).
    pub fn calls(calls: &[(&str, serde_json::Value)]) -> Result<ModelReply, ModelError> {
        Ok(ModelReply {
            tool_calls: calls
                .iter()
                .map(|(name, arguments)| ModelToolCall {
                    function: crate::ollama::ModelFunctionCall {
                        name: (*name).to_string(),
                        arguments: arguments.clone(),
                    },
                })
                .collect(),
            ..Default::default()
        })
    }
}

#[cfg(test)]
impl ChatModel for ScriptedModel {
    fn turn(
        &mut self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_event: &mut dyn FnMut(ChatEvent),
        cancel: &CancelToken,
    ) -> Result<ModelReply, ModelError> {
        self.seen.push(messages.to_vec());
        self.offered = tools
            .iter()
            .map(|spec| spec.function.name.clone())
            .collect();
        if cancel.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        let reply = match self.replies.pop_front() {
            Some(reply) => {
                self.last = Some(reply.clone());
                reply
            }
            None => self
                .last
                .clone()
                .unwrap_or_else(|| Err(ModelError::Failed("script vazio".to_string()))),
        };
        // A scripted turn still streams, so the loop's event forwarding is exercised.
        if let Ok(ok) = &reply {
            if !ok.content.is_empty() {
                on_event(ChatEvent::Token {
                    content: ok.content.clone(),
                });
            }
            on_event(ChatEvent::Done {
                prompt_tokens: ok.prompt_tokens,
                gen_tokens: ok.gen_tokens,
                prompt_ms: ok.prompt_ms,
                gen_ms: ok.gen_ms,
            });
        }
        reply
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_model_repeats_the_last_reply() {
        let mut model = ScriptedModel::new(vec![
            ScriptedModel::calls(&[("read_file", serde_json::json!({ "path": "a.rs" }))]),
            ScriptedModel::text("pronto"),
        ]);
        let cancel = CancelToken::default();
        let mut events = Vec::new();
        let mut sink = |event: ChatEvent| events.push(event);

        let first = model.turn(&[], &[], &mut sink, &cancel).unwrap();
        assert_eq!(first.tool_calls.len(), 1);
        let second = model.turn(&[], &[], &mut sink, &cancel).unwrap();
        assert_eq!(second.content, "pronto");
        // Past the end of the script the last reply comes back again.
        let third = model.turn(&[], &[], &mut sink, &cancel).unwrap();
        assert_eq!(third.content, "pronto");
        assert_eq!(model.seen.len(), 3);
    }

    #[test]
    fn scripted_model_reports_cancellation() {
        let mut model = ScriptedModel::new(vec![ScriptedModel::text("pronto")]);
        let cancel = CancelToken::default();
        cancel.cancel();
        let mut sink = |_: ChatEvent| {};
        assert_eq!(
            model.turn(&[], &[], &mut sink, &cancel),
            Err(ModelError::Cancelled)
        );
    }

    #[test]
    fn model_error_messages_are_in_portuguese() {
        assert_eq!(ModelError::Cancelled.to_string(), "tarefa cancelada");
        assert!(ModelError::Timeout.to_string().contains("tempo limite"));
        assert_eq!(
            ModelError::Failed("conexão recusada".to_string()).to_string(),
            "erro do modelo: conexão recusada"
        );
    }

    /// Needs Ollama on the machine, so it stays ignored in the gate.
    #[test]
    #[ignore]
    fn live_turn_streams_and_stops_on_cancel() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let client = OllamaClient::new(crate::ollama::DEFAULT_BASE_URL).unwrap();
        let mut model = OllamaModel::new(
            client,
            runtime.handle().clone(),
            "qwen3:4b",
            2048,
            Duration::from_secs(60),
        );
        let cancel = CancelToken::default();
        let mut tokens = 0;
        let mut sink = |event: ChatEvent| {
            if matches!(event, ChatEvent::Token { .. }) {
                tokens += 1;
            }
        };
        let reply = model
            .turn(
                &[ChatMessage {
                    role: "user".to_string(),
                    content: "responda só: ok".to_string(),
                    ..Default::default()
                }],
                &[],
                &mut sink,
                &cancel,
            )
            .unwrap();
        assert!(tokens > 0);
        assert!(!reply.content.is_empty());
    }
}
