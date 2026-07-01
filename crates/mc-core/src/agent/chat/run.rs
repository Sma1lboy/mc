//! The streaming tool-use loop: the public entrypoint that drives Rig's built-in
//! multi-turn streaming agent and forwards every text/reasoning/tool event to a
//! [`ChatEventSink`].
//!
//! Rig 0.39 ships a streaming multi-turn tool loop, so we use it directly (option
//! (a) from the design): `agent.stream_prompt(..).with_history(..).multi_turn(n)`
//! yields a stream of [`MultiTurnStreamItem`]s — text deltas, tool calls, tool
//! results (Rig dispatches the registered tools internally and feeds results
//! back), and a final response carrying the updated transcript. We translate
//! those items into [`AgentStreamEvent`]s and re-emit them through the sink.

use std::collections::HashMap;
use std::sync::Mutex;

use futures::StreamExt;

use rig_core::agent::MultiTurnStreamItem;
use rig_core::client::completion::CompletionClient;
use rig_core::completion::Message;
use rig_core::message::{ToolResult, ToolResultContent};
use rig_core::streaming::{StreamedAssistantContent, StreamedUserContent, StreamingPrompt};

use mc_types::AgentStreamEvent;

use crate::agent::llm::AgentLlmClient;
use crate::error::{CoreError, Result};

use super::prompt::CHAT_AGENT_SYSTEM_PROMPT;
use super::tools::{
    BuildModpackTool, ChatToolsCtx, InspectBaseModpackTool, ResolveModsTool,
    SearchBaseModpacksTool, SearchModsTool,
};

/// Max tool round-trips before Rig gives up on a turn. Generous enough for the
/// full search → inspect → search → resolve → build flow within one user turn.
const MAX_TOOL_TURNS: usize = 16;
/// Chat sampling temperature — low, since this is an orchestrator, not a poet.
const CHAT_TEMPERATURE: f64 = 0.3;
/// Per-turn output token budget.
const CHAT_MAX_TOKENS: u64 = 2048;
/// Cap on a tool-result summary emitted to the sink (chars).
const TOOL_SUMMARY_MAX_CHARS: usize = 240;

/// The Tauri-agnostic seam the loop pushes streamed events through. The desktop
/// command layer adapts this to an `ipc::Channel`; tests use [`CollectingSink`].
/// Mirrors how the launcher threads an `Option<watch::Sender<Progress>>`.
pub trait ChatEventSink: Send + Sync {
    /// Handle one streamed event. Called in order; must not block for long.
    fn emit(&self, event: AgentStreamEvent);
}

/// An in-memory sink that records every event, for tests and simple embeddings.
#[derive(Default)]
pub struct CollectingSink {
    events: Mutex<Vec<AgentStreamEvent>>,
}

impl CollectingSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// A snapshot of the events emitted so far, in order.
    pub fn events(&self) -> Vec<AgentStreamEvent> {
        self.events.lock().expect("sink mutex poisoned").clone()
    }
}

impl ChatEventSink for CollectingSink {
    fn emit(&self, event: AgentStreamEvent) {
        self.events.lock().expect("sink mutex poisoned").push(event);
    }
}

/// The full multi-turn transcript: role-tagged messages including tool calls and
/// tool results, exactly what the model needs to continue the conversation.
///
/// It wraps Rig's own [`Message`] so tool-call / tool-result turns round-trip
/// losslessly across turns (and serialize for persistence). The caller keeps the
/// [`ChatTurnOutcome::transcript`] from one turn and passes it back into the
/// next.
#[derive(Debug, Clone, Default)]
pub struct ChatTranscript {
    messages: Vec<Message>,
}

impl ChatTranscript {
    /// An empty transcript (first turn of a conversation).
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a transcript from persisted messages.
    pub fn from_messages(messages: Vec<Message>) -> Self {
        Self { messages }
    }

    /// The messages, oldest first.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Consume into the owned message vec (used to seed the next turn).
    pub fn into_messages(self) -> Vec<Message> {
        self.messages
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }
}

/// The result of one chat turn: the concatenated assistant reply text and the
/// updated transcript to feed into the next turn.
#[derive(Debug, Clone)]
pub struct ChatTurnOutcome {
    pub transcript: ChatTranscript,
    pub reply: String,
}

/// Run one streaming chat turn.
///
/// Feeds `user_message` (plus the prior `transcript`) to the model, streams the
/// assistant's text/reasoning and any tool calls, lets Rig dispatch the
/// deterministic tools and feed their results back, and repeats until the model
/// produces a final text answer. Every step is emitted through `sink`; the
/// returned [`ChatTurnOutcome`] carries the final reply and the updated
/// transcript.
///
/// Requires a live model endpoint (OpenRouter). Tests point `llm`'s base URL at a
/// local mock SSE server.
pub async fn run_chat_turn(
    llm: &AgentLlmClient,
    tools: &ChatToolsCtx,
    transcript: ChatTranscript,
    user_message: impl Into<String>,
    sink: &dyn ChatEventSink,
) -> Result<ChatTurnOutcome> {
    let user_message = user_message.into();

    let agent = llm
        .client()
        .agent(llm.model())
        .preamble(CHAT_AGENT_SYSTEM_PROMPT)
        .temperature(CHAT_TEMPERATURE)
        .max_tokens(CHAT_MAX_TOKENS)
        .tool(SearchBaseModpacksTool {
            registry: tools.registry.clone(),
        })
        .tool(InspectBaseModpackTool {
            registry: tools.registry.clone(),
        })
        .tool(SearchModsTool {
            registry: tools.registry.clone(),
        })
        .tool(ResolveModsTool {
            registry: tools.registry.clone(),
        })
        .tool(BuildModpackTool {
            registry: tools.registry.clone(),
            output_dir: tools.output_dir.clone(),
        })
        .build();

    let history = transcript.into_messages();
    // With `.with_history`, Rig returns the updated history on the FinalResponse.
    // Keep a copy as a fallback so a turn never silently drops prior context.
    let fallback_history = history.clone();

    let mut stream = agent
        .stream_prompt(user_message)
        .with_history(history)
        .multi_turn(MAX_TOOL_TURNS)
        .await;

    let mut reply = String::new();
    let mut final_history: Option<Vec<Message>> = None;
    // internal_call_id -> tool name, so a ToolResult event can name its tool.
    let mut tool_names: HashMap<String, String> = HashMap::new();

    while let Some(item) = stream.next().await {
        match item {
            Ok(MultiTurnStreamItem::StreamAssistantItem(content)) => match content {
                StreamedAssistantContent::Text(text) => {
                    reply.push_str(&text.text);
                    sink.emit(AgentStreamEvent::TextDelta { delta: text.text });
                }
                StreamedAssistantContent::ToolCall {
                    tool_call,
                    internal_call_id,
                } => {
                    tool_names.insert(internal_call_id, tool_call.function.name.clone());
                    sink.emit(AgentStreamEvent::ToolCall {
                        name: tool_call.function.name,
                        args: tool_call.function.arguments,
                    });
                }
                StreamedAssistantContent::Reasoning(reasoning) => {
                    let delta = reasoning.display_text();
                    if !delta.is_empty() {
                        sink.emit(AgentStreamEvent::Reasoning { delta });
                    }
                }
                StreamedAssistantContent::ReasoningDelta { reasoning, .. }
                    if !reasoning.is_empty() =>
                {
                    sink.emit(AgentStreamEvent::Reasoning { delta: reasoning });
                }
                // ToolCallDelta / Final are internal-progress items we don't surface.
                _ => {}
            },
            Ok(MultiTurnStreamItem::StreamUserItem(StreamedUserContent::ToolResult {
                tool_result,
                internal_call_id,
            })) => {
                let name = tool_names
                    .get(&internal_call_id)
                    .cloned()
                    .unwrap_or_else(|| "tool".to_string());
                sink.emit(AgentStreamEvent::ToolResult {
                    name,
                    summary: summarize_tool_result(&tool_result),
                });
            }
            Ok(MultiTurnStreamItem::FinalResponse(final_response)) => {
                if reply.trim().is_empty() {
                    reply = final_response.response().to_string();
                }
                final_history = final_response.history().map(<[Message]>::to_vec);
            }
            // CompletionCall metadata is not surfaced.
            Ok(_) => {}
            Err(err) => {
                let message = err.to_string();
                sink.emit(AgentStreamEvent::Error {
                    message: message.clone(),
                });
                return Err(CoreError::other(format!("chat turn failed: {message}")));
            }
        }
    }

    sink.emit(AgentStreamEvent::Done);
    let transcript = ChatTranscript::from_messages(final_history.unwrap_or(fallback_history));
    Ok(ChatTurnOutcome { transcript, reply })
}

/// Flatten a tool result's text content into a short, single-line summary for the
/// `ToolResult` event. Non-text content (images) is ignored.
fn summarize_tool_result(result: &ToolResult) -> String {
    let mut text = String::new();
    for content in result.content.iter() {
        if let ToolResultContent::Text(t) = content {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(t.text.trim());
        }
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return "(no textual result)".to_string();
    }
    if text.chars().count() > TOOL_SUMMARY_MAX_CHARS {
        let truncated: String = text.chars().take(TOOL_SUMMARY_MAX_CHARS).collect();
        format!("{truncated}…")
    } else {
        text
    }
}
