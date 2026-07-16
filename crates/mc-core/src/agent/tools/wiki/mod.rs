//! `wiki_search` / `wiki_open` — source-backed local modpack wiki tools.
//!
//! The model may pass only a query or chunk id. The desktop host injects the
//! local source paths, and this module owns the trust boundary: bounded file
//! reads, symlink skipping, stable chunk ids, and cache fingerprinting.

use std::path::{Path, PathBuf};

use futures::future::BoxFuture;
use mc_types::JsonValue;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ChatToolError;
use crate::error::{CoreError, Result as CoreResult};

mod cache;
mod chunk;
mod ftb;
mod privacy;
mod project;
mod search;
mod sources;
mod structured;

pub use cache::{prebuild_wiki_corpus_cache, refresh_wiki_corpus_cache, wiki_corpus_cache_path};

pub(crate) fn sanitize_private_text(content: &str) -> String {
    privacy::sanitize_private_text(content)
}

const WIKI_CORPUS_MAX_BYTES: usize = 128 * 1024 * 1024;
const WIKI_CORPUS_MAX_DOCUMENTS: usize = 50_000;
const WIKI_SEARCH_DEFAULT_TOP_K: usize = 5;
const WIKI_SEARCH_MAX_TOP_K: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct WikiScope {
    pub modpack_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    pub corpus_id: String,
}

impl WikiScope {
    pub fn from_modpack_entry(modpack_id: String, instance_id: Option<String>) -> CoreResult<Self> {
        let modpack_id = modpack_id.trim().to_string();
        if modpack_id.is_empty() {
            return Err(CoreError::other("wiki search requires modpack_id"));
        }
        let instance_id = instance_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty());
        let corpus_id = match instance_id.as_deref() {
            Some(instance_id) => format!("modpack:{modpack_id}:instance:{instance_id}"),
            None => format!("modpack:{modpack_id}"),
        };
        Ok(Self {
            modpack_id,
            instance_id,
            corpus_id,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum WikiProvenanceOrigin {
    LocalFile,
    ArchiveEntry,
    ModJar,
    Generated,
    Provider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum WikiTrust {
    Untrusted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum WikiSensitivity {
    Public,
    InstanceLocal,
    Redacted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct WikiProvenance {
    pub origin: WikiProvenanceOrigin,
    pub trust: WikiTrust,
    pub sensitivity: WikiSensitivity,
    pub uri: String,
}

impl WikiProvenance {
    fn for_document(source_label: &str, uri: &str, structured: Option<&Value>) -> Self {
        let source_type = structured
            .and_then(|value| value.pointer("/source/type"))
            .and_then(Value::as_str);
        let source_origin = structured
            .and_then(|value| value.pointer("/source/origin"))
            .and_then(Value::as_str);
        let origin = if source_label == "generated:project-doc" || source_origin == Some("provider")
        {
            WikiProvenanceOrigin::Provider
        } else if source_type == Some("mod_jar") || source_origin == Some("mod_jar") {
            WikiProvenanceOrigin::ModJar
        } else if uri.starts_with("archive://") {
            WikiProvenanceOrigin::ArchiveEntry
        } else if source_type == Some("generated") {
            WikiProvenanceOrigin::Generated
        } else if source_origin == Some("local") {
            WikiProvenanceOrigin::LocalFile
        } else if source_label.starts_with("generated:") {
            WikiProvenanceOrigin::Generated
        } else {
            WikiProvenanceOrigin::LocalFile
        };
        let sensitivity = if matches!(
            origin,
            WikiProvenanceOrigin::ModJar | WikiProvenanceOrigin::Provider
        ) {
            WikiSensitivity::Public
        } else {
            WikiSensitivity::InstanceLocal
        };
        let provenance_uri = structured
            .and_then(|value| value.pointer("/source/uri"))
            .and_then(Value::as_str)
            .unwrap_or(uri)
            .to_string();
        Self {
            origin,
            trust: WikiTrust::Untrusted,
            sensitivity,
            uri: provenance_uri,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiSourceDocument {
    pub title: String,
    pub source_label: String,
    pub uri: String,
    pub content: String,
    pub kind: Option<String>,
    pub structured: Option<Value>,
    pub provenance: WikiProvenance,
}

impl WikiSourceDocument {
    fn text(title: String, source_label: String, uri: String, content: String) -> Self {
        let provenance = WikiProvenance::for_document(&source_label, &uri, None);
        Self {
            title,
            source_label,
            uri,
            content,
            kind: None,
            structured: None,
            provenance,
        }
    }

    fn structured(
        title: String,
        source_label: String,
        uri: String,
        content: String,
        kind: impl Into<String>,
        structured: Value,
    ) -> Self {
        let provenance = WikiProvenance::for_document(&source_label, &uri, Some(&structured));
        Self {
            title,
            source_label,
            uri,
            content,
            kind: Some(kind.into()),
            structured: Some(structured),
            provenance,
        }
    }
}

pub trait WikiSource: Send + Sync {
    fn load_documents<'a>(&'a self) -> BoxFuture<'a, CoreResult<Vec<WikiSourceDocument>>>;
}

#[derive(Debug, Clone)]
pub struct LocalPathWikiSource {
    paths: Vec<PathBuf>,
}

impl LocalPathWikiSource {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self { paths }
    }
}

impl WikiSource for LocalPathWikiSource {
    fn load_documents<'a>(&'a self) -> BoxFuture<'a, CoreResult<Vec<WikiSourceDocument>>> {
        let paths = self.paths.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || sources::read_local_wiki_documents(&paths))
                .await
                .map_err(|err| CoreError::other(format!("wiki corpus build task failed: {err}")))?
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct WikiChunk {
    pub chunk_id: String,
    pub document_id: String,
    pub title: String,
    pub source_label: String,
    pub location: String,
    pub content: String,
    pub provenance: WikiProvenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[specta(type = Option<JsonValue>)]
    pub structured: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct WikiSearchHit {
    pub chunk_id: String,
    pub document_id: String,
    pub title: String,
    pub snippet: String,
    pub source_label: String,
    pub location: String,
    pub score: f32,
    pub provenance: WikiProvenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[specta(type = Option<JsonValue>)]
    pub structured: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct WikiCorpus {
    scope: WikiScope,
    source_count: usize,
    chunks: Vec<WikiChunk>,
}

impl WikiCorpus {
    pub async fn from_sources(
        scope: WikiScope,
        sources: Vec<Box<dyn WikiSource>>,
    ) -> CoreResult<Self> {
        let source_count = sources.len();
        let mut documents = Vec::new();
        let mut total_bytes = 0usize;
        for source in sources {
            for doc in source.load_documents().await? {
                if documents.len() >= WIKI_CORPUS_MAX_DOCUMENTS {
                    break;
                }
                let bytes = doc.content.len();
                if total_bytes.saturating_add(bytes) > WIKI_CORPUS_MAX_BYTES {
                    break;
                }
                total_bytes += bytes;
                documents.push(doc);
            }
        }
        Self::from_documents(scope, source_count, documents)
    }

    fn from_documents(
        scope: WikiScope,
        source_count: usize,
        mut documents: Vec<WikiSourceDocument>,
    ) -> CoreResult<Self> {
        privacy::finalize_wiki_documents(&mut documents, &[]);
        documents.sort_by(|a, b| a.uri.cmp(&b.uri));
        let chunks = documents
            .into_iter()
            .flat_map(|doc| chunk::chunks_from_document(&doc))
            .collect();
        Ok(Self {
            scope,
            source_count,
            chunks,
        })
    }

    pub fn scope(&self) -> &WikiScope {
        &self.scope
    }

    pub fn source_count(&self) -> usize {
        self.source_count
    }

    pub async fn search(&self, query: &str, top_k: usize) -> CoreResult<Vec<WikiSearchHit>> {
        self.search_with_options(&WikiSearchOptions {
            query: query.to_string(),
            top_k,
            kind: None,
            target_id: None,
            ingredient_id: None,
            include_structured: true,
        })
        .await
    }

    pub async fn search_with_options(
        &self,
        options: &WikiSearchOptions,
    ) -> CoreResult<Vec<WikiSearchHit>> {
        let query = search::SearchQuery::parse(&options.query);
        let kind = options
            .kind
            .as_deref()
            .and_then(search::normalize_filter_text);
        let target_id = options
            .target_id
            .as_deref()
            .and_then(search::normalize_filter_text);
        let ingredient_id = options
            .ingredient_id
            .as_deref()
            .and_then(search::normalize_filter_text);
        let has_structured_filter =
            kind.is_some() || target_id.is_some() || ingredient_id.is_some();
        if query.is_empty() && !has_structured_filter {
            return Ok(Vec::new());
        }
        let mut hits = self
            .chunks
            .iter()
            .filter(|chunk| search::chunk_matches_kind(chunk, kind.as_deref()))
            .filter(|chunk| search::chunk_matches_target(chunk, target_id.as_deref()))
            .filter(|chunk| search::chunk_matches_ingredient(chunk, ingredient_id.as_deref()))
            .filter_map(|chunk| {
                let score = if query.is_empty() {
                    0.0
                } else {
                    search::score_chunk(chunk, &query)
                } + search::structured_filter_score(
                    chunk,
                    target_id.as_deref(),
                    ingredient_id.as_deref(),
                ) + search::source_priority_score(chunk);
                (score > 0.0).then(|| WikiSearchHit {
                    chunk_id: chunk.chunk_id.clone(),
                    document_id: chunk.document_id.clone(),
                    title: chunk.title.clone(),
                    snippet: search::snippet_for_terms(&chunk.content, &query.snippet_terms),
                    source_label: chunk.source_label.clone(),
                    location: chunk.location.clone(),
                    score,
                    provenance: chunk.provenance.clone(),
                    kind: chunk.kind.clone(),
                    structured: options
                        .include_structured
                        .then(|| chunk.structured.clone())
                        .flatten(),
                })
            })
            .collect::<Vec<_>>();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.chunk_id.cmp(&b.chunk_id))
        });
        hits.truncate(options.top_k.clamp(1, WIKI_SEARCH_MAX_TOP_K));
        Ok(hits)
    }

    pub async fn open(&self, chunk_id: &str) -> CoreResult<WikiChunk> {
        self.chunks
            .iter()
            .find(|chunk| chunk.chunk_id == chunk_id)
            .cloned()
            .ok_or_else(|| CoreError::other(format!("wiki chunk not found: {chunk_id}")))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct WikiSearchArgs {
    pub modpack_id: String,
    #[serde(default)]
    pub instance_id: Option<String>,
    /// Local files, directories, `.mrpack`, or `.zip` archives selected by the host.
    #[serde(default)]
    pub source_paths: Vec<String>,
    pub query: String,
    #[serde(default)]
    pub top_k: Option<usize>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub target_id: Option<String>,
    #[serde(default)]
    pub ingredient_id: Option<String>,
    #[serde(default)]
    pub include_structured: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct WikiSearchOptions {
    pub query: String,
    pub top_k: usize,
    pub kind: Option<String>,
    pub target_id: Option<String>,
    pub ingredient_id: Option<String>,
    pub include_structured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct WikiOpenArgs {
    pub modpack_id: String,
    #[serde(default)]
    pub instance_id: Option<String>,
    /// Same host-injected source list used for `wiki_search`.
    #[serde(default)]
    pub source_paths: Vec<String>,
    pub chunk_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct WikiSearchOutput {
    pub scope: WikiScope,
    pub source_count: usize,
    pub hits: Vec<WikiSearchHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct WikiOpenOutput {
    pub scope: WikiScope,
    pub chunk: WikiChunk,
}

pub async fn tool_wiki_search(
    args: WikiSearchArgs,
) -> std::result::Result<WikiSearchOutput, ChatToolError> {
    let top_k = args.top_k.unwrap_or(WIKI_SEARCH_DEFAULT_TOP_K);
    let options = WikiSearchOptions {
        query: args.query,
        top_k,
        kind: args.kind,
        target_id: args.target_id,
        ingredient_id: args.ingredient_id,
        include_structured: args.include_structured.unwrap_or(true),
    };
    let corpus =
        corpus_from_tool_args(args.modpack_id, args.instance_id, args.source_paths).await?;
    let hits = corpus.search_with_options(&options).await?;
    Ok(WikiSearchOutput {
        scope: corpus.scope,
        source_count: corpus.source_count,
        hits,
    })
}

pub async fn tool_wiki_open(
    args: WikiOpenArgs,
) -> std::result::Result<WikiOpenOutput, ChatToolError> {
    let corpus =
        corpus_from_tool_args(args.modpack_id, args.instance_id, args.source_paths).await?;
    let chunk = corpus.open(&args.chunk_id).await?;
    Ok(WikiOpenOutput {
        scope: corpus.scope,
        chunk,
    })
}

async fn corpus_from_tool_args(
    modpack_id: String,
    instance_id: Option<String>,
    source_paths: Vec<String>,
) -> CoreResult<WikiCorpus> {
    let scope = WikiScope::from_modpack_entry(modpack_id, instance_id)?;
    let source_paths = source_paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if let Some(path) = cacheable_wiki_source_path(&source_paths) {
        return cache::corpus_from_cache_or_rebuild(scope, path).await;
    }
    build_wiki_corpus_from_paths(scope, source_paths).await
}

async fn build_wiki_corpus_from_paths(
    scope: WikiScope,
    source_paths: Vec<PathBuf>,
) -> CoreResult<WikiCorpus> {
    let source_count = source_paths.len();
    let local_source_paths = source_paths.clone();
    let mut documents = tokio::task::spawn_blocking(move || {
        sources::read_local_wiki_documents(&local_source_paths)
    })
    .await
    .map_err(|err| CoreError::other(format!("wiki corpus build task failed: {err}")))??;
    documents.extend(project::read_project_wiki_documents(&source_paths).await);
    privacy::finalize_wiki_documents(&mut documents, &source_paths);
    WikiCorpus::from_documents(scope, source_count, documents)
}

fn cacheable_wiki_source_path(source_paths: &[PathBuf]) -> Option<&Path> {
    let [path] = source_paths else {
        return None;
    };
    sources::regular_dir(path).then_some(path.as_path())
}
