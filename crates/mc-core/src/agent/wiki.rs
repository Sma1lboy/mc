//! Scoped wiki-query workflow and local wiki corpus primitives.
//!
//! The workflow owns modpack scope, citations, and durable agent state. Rig is
//! used only as the inner tool-calling loop for open-ended wiki questions.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use rig_core::client::CompletionClient;
use rig_core::completion::Prompt;
use rig_core::completion::ToolDefinition;
use rig_core::message::Message;
use rig_core::tool::Tool;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{CoreError, IoResultExt, Result};

use super::llm::AgentLlmClient;
use super::state::{
    AgentLaunchContext, AgentMessageKind, AgentPhase, AgentRunSnapshot, AgentStatus,
    AgentTraceEvent, AgentWorkflowKind, WikiCitation, WikiScope, WikiThreadState,
};

const WIKI_QUERY_MAX_TURNS: usize = 3;
const WIKI_QUERY_MAX_OUTPUT_TOKENS: u64 = 1200;
const WIKI_QUERY_TEMPERATURE: f64 = 0.2;
const WIKI_SEARCH_MAX_TOP_K: usize = 8;
const WIKI_FILE_MAX_BYTES: u64 = 256 * 1024;

const WIKI_QUERY_PREAMBLE: &str = r#"You answer questions about the current Minecraft modpack only.
Use wiki_search before answering factual questions.
Use wiki_open when a search hit is relevant but the snippet is insufficient.
Do not mention facts that are not supported by retrieved chunks.
If the indexed wiki data does not contain evidence, say that it was not found in the indexed wiki data.
Cite every factual claim with chunk citations in the exact form [chunk:<doc_index>:<chunk_index>]."#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikiSource {
    pub id: String,
    pub label: String,
    pub uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikiDocument {
    pub id: String,
    pub title: String,
    pub source: WikiSource,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikiChunk {
    pub chunk_id: String,
    pub document_id: String,
    pub title: String,
    pub source_label: String,
    pub location: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WikiSearchHit {
    pub chunk_id: String,
    pub title: String,
    pub snippet: String,
    pub source_label: String,
    pub location: String,
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikiTurnOutput {
    pub answer_markdown: String,
    #[serde(default)]
    pub citations: Vec<WikiCitation>,
    pub not_found: bool,
}

pub trait WikiRetriever: Send + Sync + 'static {
    fn search<'a>(
        &'a self,
        scope: &'a WikiScope,
        query: &'a str,
        top_k: usize,
    ) -> BoxFuture<'a, Result<Vec<WikiSearchHit>>>;

    fn open<'a>(
        &'a self,
        scope: &'a WikiScope,
        chunk_id: &'a str,
    ) -> BoxFuture<'a, Result<WikiChunk>>;
}

#[derive(Debug, Clone)]
pub struct LocalWikiCorpus {
    scope: Option<WikiScope>,
    chunks: Vec<WikiChunk>,
}

impl LocalWikiCorpus {
    pub fn empty() -> Self {
        Self {
            scope: None,
            chunks: Vec::new(),
        }
    }

    pub fn from_texts<I, P, C>(scope: WikiScope, texts: I) -> Self
    where
        I: IntoIterator<Item = (P, C)>,
        P: Into<String>,
        C: Into<String>,
    {
        let chunks = texts
            .into_iter()
            .enumerate()
            .map(|(doc_index, (path, content))| {
                let path = path.into();
                let content = content.into();
                chunk_from_text(doc_index, 0, &path, content)
            })
            .collect();
        Self {
            scope: Some(scope),
            chunks,
        }
    }

    pub fn from_paths(scope: WikiScope, paths: &[PathBuf]) -> Result<Self> {
        let mut texts = Vec::new();
        for path in paths {
            if is_archive_path(path) {
                texts.extend(read_archive_wiki_texts(path)?);
            } else {
                let mut files = Vec::new();
                collect_wiki_files(path, &mut files)?;
                files.sort();
                for file in files {
                    if let Ok(content) = std::fs::read_to_string(&file) {
                        texts.push((file.to_string_lossy().to_string(), content));
                    }
                }
            }
        }
        Ok(Self::from_texts(scope, texts))
    }

    fn ensure_scope(&self, scope: &WikiScope) -> Result<()> {
        if let Some(expected) = self.scope.as_ref() {
            if expected.corpus_id != scope.corpus_id {
                return Err(CoreError::other(format!(
                    "wiki corpus scope mismatch: expected {}, got {}",
                    expected.corpus_id, scope.corpus_id
                )));
            }
        }
        Ok(())
    }

    fn search_sync(
        &self,
        scope: &WikiScope,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<WikiSearchHit>> {
        self.ensure_scope(scope)?;
        let query_terms = search_terms(query);
        if query_terms.is_empty() {
            return Ok(Vec::new());
        }

        let mut hits = self
            .chunks
            .iter()
            .filter_map(|chunk| {
                let haystack = chunk.content.to_ascii_lowercase();
                let title = chunk.title.to_ascii_lowercase();
                let mut score = 0.0_f32;
                for term in &query_terms {
                    if haystack.contains(term) {
                        score += 2.0;
                    }
                    if title.contains(term) {
                        score += 1.0;
                    }
                }
                (score > 0.0).then(|| WikiSearchHit {
                    chunk_id: chunk.chunk_id.clone(),
                    title: chunk.title.clone(),
                    snippet: snippet_for_terms(&chunk.content, &query_terms),
                    source_label: chunk.source_label.clone(),
                    location: chunk.location.clone(),
                    score,
                })
            })
            .collect::<Vec<_>>();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.chunk_id.cmp(&b.chunk_id))
        });
        hits.truncate(top_k.clamp(1, WIKI_SEARCH_MAX_TOP_K));
        Ok(hits)
    }

    fn open_sync(&self, scope: &WikiScope, chunk_id: &str) -> Result<WikiChunk> {
        self.ensure_scope(scope)?;
        self.chunks
            .iter()
            .find(|chunk| chunk.chunk_id == chunk_id)
            .cloned()
            .ok_or_else(|| CoreError::other(format!("wiki chunk not found: {chunk_id}")))
    }
}

impl WikiRetriever for LocalWikiCorpus {
    fn search<'a>(
        &'a self,
        scope: &'a WikiScope,
        query: &'a str,
        top_k: usize,
    ) -> BoxFuture<'a, Result<Vec<WikiSearchHit>>> {
        Box::pin(async move { self.search_sync(scope, query, top_k) })
    }

    fn open<'a>(
        &'a self,
        scope: &'a WikiScope,
        chunk_id: &'a str,
    ) -> BoxFuture<'a, Result<WikiChunk>> {
        Box::pin(async move { self.open_sync(scope, chunk_id) })
    }
}

#[derive(Clone)]
pub struct WikiQueryWorkflow {
    llm: AgentLlmClient,
    retriever: Arc<dyn WikiRetriever>,
}

impl WikiQueryWorkflow {
    pub fn new(llm: AgentLlmClient, retriever: Arc<dyn WikiRetriever>) -> Self {
        Self { llm, retriever }
    }

    pub async fn start(&self, user_prompt: &str, scope: WikiScope) -> Result<AgentRunSnapshot> {
        let (output, traces) = run_rig_wiki_turn(
            &self.llm,
            self.retriever.clone(),
            scope.clone(),
            user_prompt,
            &[],
        )
        .await?;
        let mut run = completed_wiki_turn_snapshot(user_prompt, scope, output);
        attach_wiki_tool_traces(&mut run, traces);
        Ok(run)
    }

    pub async fn continue_run(
        &self,
        mut run: AgentRunSnapshot,
        user_message: &str,
    ) -> Result<AgentRunSnapshot> {
        let scope = run
            .wiki
            .as_ref()
            .map(|wiki| wiki.scope.clone())
            .ok_or_else(|| CoreError::other("wiki continuation requires wiki thread state"))?;
        let history = run.messages.clone();
        let (output, traces) = run_rig_wiki_turn(
            &self.llm,
            self.retriever.clone(),
            scope.clone(),
            user_message,
            &history,
        )
        .await?;

        run.status = AgentStatus::WaitingForUser;
        run.phase = AgentPhase::WikiQuery;
        run.workflow = AgentWorkflowKind::WikiQuery;
        run.pending_approval = None;
        run.push_message(AgentMessageKind::User, user_message.trim());
        run.push_message(AgentMessageKind::Assistant, output.answer_markdown.clone());
        update_wiki_thread_state(&mut run, scope, user_message, &output);
        attach_wiki_tool_traces(&mut run, traces);
        Ok(run)
    }
}

pub fn completed_wiki_turn_snapshot(
    user_prompt: &str,
    scope: WikiScope,
    output: WikiTurnOutput,
) -> AgentRunSnapshot {
    let mut run = AgentRunSnapshot::new(user_prompt);
    run.workflow = AgentWorkflowKind::WikiQuery;
    run.launch_context = AgentLaunchContext::from_entry(super::state::AgentEntry::Modpack {
        modpack_id: scope.modpack_id.clone(),
        instance_id: scope.instance_id.clone(),
    });
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::WikiQuery;
    run.pending_approval = None;
    run.push_message(AgentMessageKind::User, user_prompt.trim());
    run.push_message(AgentMessageKind::Assistant, output.answer_markdown.clone());
    update_wiki_thread_state(&mut run, scope, user_prompt, &output);
    run.push_trace("wiki query workflow completed turn");
    run
}

fn update_wiki_thread_state(
    run: &mut AgentRunSnapshot,
    scope: WikiScope,
    user_prompt: &str,
    output: &WikiTurnOutput,
) {
    let mut cited_chunk_ids = run
        .wiki
        .as_ref()
        .map(|wiki| wiki.cited_chunk_ids.clone())
        .unwrap_or_default();
    let source_uris = run
        .wiki
        .as_ref()
        .map(|wiki| wiki.source_uris.clone())
        .unwrap_or_default();
    for citation in &output.citations {
        if !cited_chunk_ids.contains(&citation.chunk_id) {
            cited_chunk_ids.push(citation.chunk_id.clone());
        }
    }
    run.wiki = Some(WikiThreadState {
        scope,
        source_uris,
        last_query: Some(user_prompt.trim().to_string()),
        focused_entities: Vec::new(),
        cited_chunk_ids,
    });
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WikiSearchArgs {
    query: String,
    #[serde(default)]
    top_k: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WikiOpenArgs {
    chunk_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WikiSearchOutput {
    hits: Vec<WikiSearchHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WikiOpenOutput {
    chunk: WikiChunk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WikiToolTrace {
    tool: String,
    input: serde_json::Value,
    output: serde_json::Value,
}

type WikiToolTraceRecorder = Arc<Mutex<Vec<WikiToolTrace>>>;

#[derive(Clone)]
struct WikiSearchTool {
    scope: WikiScope,
    retriever: Arc<dyn WikiRetriever>,
    recorder: WikiToolTraceRecorder,
}

#[derive(Clone)]
struct WikiOpenTool {
    scope: WikiScope,
    retriever: Arc<dyn WikiRetriever>,
    recorder: WikiToolTraceRecorder,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct WikiToolError(String);

impl Tool for WikiSearchTool {
    const NAME: &'static str = "wiki_search";
    type Error = WikiToolError;
    type Args = WikiSearchArgs;
    type Output = WikiSearchOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Search the current modpack's scoped wiki corpus. Use this before answering factual questions about the modpack. The modpack scope is injected by the runtime; do not ask for or pass a modpack id.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Short search query for the current modpack wiki, for example \"aether portal\"."
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "Maximum hits to return. Defaults to 5 and cannot exceed 8.",
                        "minimum": 1,
                        "maximum": 8
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> std::result::Result<Self::Output, Self::Error> {
        let top_k = args.top_k.unwrap_or(5).clamp(1, WIKI_SEARCH_MAX_TOP_K);
        let input = json!({ "query": args.query, "top_k": top_k });
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let hits = self
            .retriever
            .search(&self.scope, &query, top_k)
            .await
            .map_err(|err| WikiToolError(err.to_string()))?;
        let output = WikiSearchOutput { hits };
        record_tool_trace(&self.recorder, Self::NAME, input, json!(output));
        Ok(output)
    }
}

impl Tool for WikiOpenTool {
    const NAME: &'static str = "wiki_open";
    type Error = WikiToolError;
    type Args = WikiOpenArgs;
    type Output = WikiOpenOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Open one wiki chunk from the current modpack's scoped wiki corpus by chunk_id returned from wiki_search. Use when the search snippet is relevant but not enough to answer with evidence.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "chunk_id": {
                        "type": "string",
                        "description": "A chunk_id returned by wiki_search, for example \"chunk:0:0\"."
                    }
                },
                "required": ["chunk_id"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> std::result::Result<Self::Output, Self::Error> {
        let input = json!({ "chunk_id": args.chunk_id });
        let chunk_id = input
            .get("chunk_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let chunk = self
            .retriever
            .open(&self.scope, &chunk_id)
            .await
            .map_err(|err| WikiToolError(err.to_string()))?;
        let output = WikiOpenOutput { chunk };
        record_tool_trace(&self.recorder, Self::NAME, input, json!(output));
        Ok(output)
    }
}

async fn run_rig_wiki_turn(
    llm: &AgentLlmClient,
    retriever: Arc<dyn WikiRetriever>,
    scope: WikiScope,
    user_prompt: &str,
    history: &[super::state::AgentMessage],
) -> Result<(WikiTurnOutput, Vec<WikiToolTrace>)> {
    let recorder = Arc::new(Mutex::new(Vec::new()));
    let agent = llm
        .client
        .agent(llm.config.model.clone())
        .preamble(WIKI_QUERY_PREAMBLE)
        .temperature(WIKI_QUERY_TEMPERATURE)
        .max_tokens(WIKI_QUERY_MAX_OUTPUT_TOKENS)
        .tool(WikiSearchTool {
            scope: scope.clone(),
            retriever: retriever.clone(),
            recorder: recorder.clone(),
        })
        .tool(WikiOpenTool {
            scope,
            retriever,
            recorder: recorder.clone(),
        })
        .build();

    let response = agent
        .prompt(user_prompt.to_string())
        .with_history(rig_history_from_agent_messages(history))
        .with_tool_concurrency(1)
        .max_turns(WIKI_QUERY_MAX_TURNS)
        .extended_details()
        .await
        .map_err(|err| CoreError::other(format!("Rig wiki tool loop failed: {err}")))?;

    let traces = recorder
        .lock()
        .map_err(|_| CoreError::other("wiki tool trace recorder poisoned"))?
        .clone();
    let output = wiki_turn_output_from_response(&response.output, &traces);
    Ok((output, traces))
}

fn rig_history_from_agent_messages(messages: &[super::state::AgentMessage]) -> Vec<Message> {
    messages
        .iter()
        .filter_map(|message| match message.kind {
            AgentMessageKind::User => Some(Message::user(message.text.clone())),
            AgentMessageKind::Assistant => Some(Message::assistant(message.text.clone())),
            AgentMessageKind::System | AgentMessageKind::Tool => None,
        })
        .collect()
}

fn wiki_turn_output_from_response(answer: &str, traces: &[WikiToolTrace]) -> WikiTurnOutput {
    let allowed_citations = citations_from_tool_traces(traces);
    let cited_ids = citation_ids_from_answer(answer);
    let citations = cited_ids
        .iter()
        .filter_map(|id| allowed_citations.get(id).cloned())
        .collect::<Vec<_>>();
    let has_invalid_citation = cited_ids
        .iter()
        .any(|id| !allowed_citations.contains_key(id));
    let answer_says_not_found = answer_indicates_not_found(answer);
    let no_retrieved_evidence = allowed_citations.is_empty();
    let missing_required_citation =
        !answer_says_not_found && !allowed_citations.is_empty() && cited_ids.is_empty();
    let not_found = no_retrieved_evidence || has_invalid_citation || missing_required_citation;
    let answer_markdown = if has_invalid_citation {
        "I found wiki data, but the model cited chunks that were not retrieved. Please retry the question."
            .to_string()
    } else if missing_required_citation {
        "I found wiki data, but the model did not cite retrieved chunks. Please retry the question."
            .to_string()
    } else if no_retrieved_evidence && !answer_says_not_found {
        "I could not find an answer in the indexed wiki data.".to_string()
    } else if answer.trim().is_empty() {
        "I could not find an answer in the indexed wiki data.".to_string()
    } else {
        answer.trim().to_string()
    };
    WikiTurnOutput {
        answer_markdown,
        citations,
        not_found,
    }
}

fn answer_indicates_not_found(answer: &str) -> bool {
    let lower = answer.to_ascii_lowercase();
    lower.contains("not found")
        || lower.contains("could not find")
        || lower.contains("did not find")
        || lower.contains("no indexed wiki data")
}

fn citations_from_tool_traces(traces: &[WikiToolTrace]) -> HashMap<String, WikiCitation> {
    let mut citations = HashMap::new();
    for trace in traces {
        match trace.tool.as_str() {
            "wiki_search" => {
                let Some(hits) = trace.output.get("hits").and_then(|v| v.as_array()) else {
                    continue;
                };
                for hit in hits {
                    let Some(chunk_id) = hit.get("chunk_id").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    citations
                        .entry(chunk_id.to_string())
                        .or_insert_with(|| WikiCitation {
                            chunk_id: chunk_id.to_string(),
                            title: json_string(hit, "title", chunk_id),
                            source_label: json_string(hit, "source_label", "-"),
                            location: json_string(hit, "location", "-"),
                        });
                }
            }
            "wiki_open" => {
                let Some(chunk) = trace.output.get("chunk") else {
                    continue;
                };
                let Some(chunk_id) = chunk.get("chunk_id").and_then(|v| v.as_str()) else {
                    continue;
                };
                citations.insert(
                    chunk_id.to_string(),
                    WikiCitation {
                        chunk_id: chunk_id.to_string(),
                        title: json_string(chunk, "title", chunk_id),
                        source_label: json_string(chunk, "source_label", "-"),
                        location: json_string(chunk, "location", "-"),
                    },
                );
            }
            _ => {}
        }
    }
    citations
}

fn attach_wiki_tool_traces(run: &mut AgentRunSnapshot, traces: Vec<WikiToolTrace>) {
    for (idx, trace) in traces.into_iter().enumerate() {
        run.trace.push(AgentTraceEvent {
            at_ms: now_ms_for_trace(),
            event: "wiki tool dispatched".to_string(),
            stage: Some(AgentPhase::WikiQuery),
            iteration: Some(idx as u32),
            tool: Some(trace.tool),
            input: Some(trace.input),
            output: Some(trace.output),
            duration_ms: None,
            status: Some("completed".to_string()),
        });
    }
}

fn record_tool_trace(
    recorder: &WikiToolTraceRecorder,
    tool: &str,
    input: serde_json::Value,
    output: serde_json::Value,
) {
    if let Ok(mut traces) = recorder.lock() {
        traces.push(WikiToolTrace {
            tool: tool.to_string(),
            input,
            output,
        });
    }
}

fn chunk_from_text(doc_index: usize, chunk_index: usize, path: &str, content: String) -> WikiChunk {
    let line_count = content.lines().count().max(1);
    WikiChunk {
        chunk_id: format!("chunk:{doc_index}:{chunk_index}"),
        document_id: format!("doc:{doc_index}"),
        title: path.to_string(),
        source_label: path.to_string(),
        location: format!("lines 1-{line_count}"),
        content,
    }
}

fn collect_wiki_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !path.exists() {
        return Err(CoreError::other(format!(
            "wiki source path does not exist: {}",
            path.display()
        )));
    }
    if path.is_file() {
        if is_allowed_wiki_file(path)? {
            files.push(path.to_path_buf());
        }
        return Ok(());
    }
    if should_skip_dir(path) {
        return Ok(());
    }
    for entry in std::fs::read_dir(path).with_path(path)? {
        let entry = entry.with_path(path)?;
        collect_wiki_files(&entry.path(), files)?;
    }
    Ok(())
}

fn read_archive_wiki_texts(path: &Path) -> Result<Vec<(String, String)>> {
    let file = std::fs::File::open(path).with_path(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|err| CoreError::Zip(err.to_string()))?;
    let mut texts = Vec::new();
    for index in 0..archive.len() {
        let Ok(mut file) = archive.by_index(index) else {
            continue;
        };
        if file.is_dir() {
            continue;
        }
        let name = file.name().to_string();
        if !is_allowed_wiki_archive_entry(&name, file.size()) {
            continue;
        }
        let mut content = String::new();
        if file.read_to_string(&mut content).is_ok() {
            texts.push((format!("{}!{}", path.display(), name), content));
        }
    }
    texts.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(texts)
}

fn is_allowed_wiki_file(path: &Path) -> Result<bool> {
    let meta = std::fs::metadata(path).with_path(path)?;
    if !meta.is_file() || meta.len() > WIKI_FILE_MAX_BYTES {
        return Ok(false);
    }
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return Ok(false);
    };
    Ok(allowed_wiki_extension(ext))
}

fn is_allowed_wiki_archive_entry(name: &str, size: u64) -> bool {
    if size > WIKI_FILE_MAX_BYTES || should_skip_virtual_path(name) {
        return false;
    }
    let Some(ext) = Path::new(name).extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    allowed_wiki_extension(ext)
}

fn is_archive_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "mrpack" | "zip"))
        .unwrap_or(false)
}

fn should_skip_dir(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        should_skip_path_segment(&name)
    })
}

fn should_skip_virtual_path(path: &str) -> bool {
    path.split('/')
        .map(|segment| segment.to_ascii_lowercase())
        .any(|segment| should_skip_path_segment(&segment))
}

fn should_skip_path_segment(segment: &str) -> bool {
    matches!(
        segment,
        "mods" | "resourcepacks" | "shaderpacks" | ".git" | "versions"
    )
}

fn allowed_wiki_extension(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "md" | "txt"
            | "snbt"
            | "json"
            | "json5"
            | "jsonc"
            | "toml"
            | "properties"
            | "cfg"
            | "js"
            | "zs"
            | "lang"
            | "yaml"
            | "yml"
    )
}

fn search_terms(query: &str) -> Vec<String> {
    query
        .split(|ch: char| !ch.is_alphanumeric())
        .map(|term| term.trim().to_ascii_lowercase())
        .filter(|term| term.len() >= 2)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}

fn snippet_for_terms(content: &str, terms: &[String]) -> String {
    let lower = content.to_ascii_lowercase();
    let start = terms
        .iter()
        .filter_map(|term| lower.find(term))
        .min()
        .unwrap_or(0);
    let start = content[..start.min(content.len())]
        .rfind(['\n', '.', ';'])
        .map(|idx| idx + 1)
        .unwrap_or(start.saturating_sub(80));
    let end = (start + 260).min(content.len());
    content[start..end].trim().replace('\n', " ")
}

fn citation_ids_from_answer(answer: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = answer;
    while let Some(start) = rest.find("[chunk:") {
        let after = &rest[start + 1..];
        let Some(end) = after.find(']') else {
            break;
        };
        let id = &after[..end];
        if !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_string());
        }
        rest = &after[end + 1..];
    }
    ids
}

fn json_string(value: &serde_json::Value, key: &str, fallback: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or(fallback)
        .to_string()
}

fn now_ms_for_trace() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}
