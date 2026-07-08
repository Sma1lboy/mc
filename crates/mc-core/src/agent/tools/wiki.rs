//! `wiki_search` / `wiki_open` — source-backed local modpack wiki tools.
//!
//! The model may pass only a query or chunk id. The desktop host injects the
//! local source paths, and this module owns the trust boundary: bounded file
//! reads, symlink skipping, stable chunk ids, and cache fingerprinting.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use futures::future::BoxFuture;
use mc_types::JsonValue;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::ChatToolError;
use crate::error::{CoreError, IoResultExt, Result as CoreResult};
use crate::instance::InstanceConfig;
use crate::modplatform::modrinth::{ModrinthApi, ProjectDetail};
use crate::version::pack::PackProfile;

const WIKI_FILE_MAX_BYTES: u64 = 256 * 1024;
const WIKI_CORPUS_MAX_BYTES: usize = 128 * 1024 * 1024;
const WIKI_CORPUS_MAX_DOCUMENTS: usize = 50_000;
const WIKI_ARCHIVE_MAX_BYTES: usize = 512 * 1024;
const WIKI_ARCHIVE_MAX_ENTRIES: usize = 128;
const WIKI_SEARCH_DEFAULT_TOP_K: usize = 5;
const WIKI_SEARCH_MAX_TOP_K: usize = 8;
const WIKI_CHUNK_MAX_LINES: usize = 80;
const WIKI_CHUNK_MAX_BYTES: usize = 64 * 1024;
const WIKI_CORPUS_CACHE_VERSION: u32 = 7;
const WIKI_CORPUS_CACHE_FILE: &str = "wiki-corpus.json";
const WIKI_PROJECT_CACHE_DIR: &str = ".wiki-project-cache";
const WIKI_PROJECT_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 3600);
const WIKI_PROJECT_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4);
const INSTANCE_DATA_MAX_ENTRIES: usize = 200;
const MOD_JAR_ENTRY_MAX_BYTES: u64 = 256 * 1024;
const TAG_STRUCTURED_VALUES_MAX: usize = 128;

const INSTANCE_DATA_DIRS: &[&str] = &[
    "mods",
    "config",
    "resourcepacks",
    "shaderpacks",
    "datapacks",
    "scripts",
    "kubejs",
];

const WIKI_INDEX_DIRS: &[&str] = &[
    "config",
    "defaultconfigs",
    "serverconfig",
    "world/serverconfig",
    "datapacks",
    "scripts",
    "kubejs",
];

const FTB_QUESTS_FILE_MAX_BYTES: u64 = 1024 * 1024;
const FTB_QUESTS_DIRS: &[&str] = &[
    "config/ftbquests",
    "defaultconfigs/ftbquests",
    "serverconfig/ftbquests",
    "world/serverconfig/ftbquests",
];

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiSourceDocument {
    pub title: String,
    pub source_label: String,
    pub uri: String,
    pub content: String,
    pub kind: Option<String>,
    pub structured: Option<Value>,
}

impl WikiSourceDocument {
    fn text(title: String, source_label: String, uri: String, content: String) -> Self {
        Self {
            title,
            source_label,
            uri,
            content,
            kind: None,
            structured: None,
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
        Self {
            title,
            source_label,
            uri,
            content,
            kind: Some(kind.into()),
            structured: Some(structured),
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
            tokio::task::spawn_blocking(move || read_local_wiki_documents(&paths))
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
        documents.sort_by(|a, b| a.uri.cmp(&b.uri));
        let chunks = documents
            .into_iter()
            .flat_map(|doc| chunks_from_document(&doc))
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
        let query = SearchQuery::parse(&options.query);
        let kind = options.kind.as_deref().and_then(normalize_filter_text);
        let target_id = options.target_id.as_deref().and_then(normalize_filter_text);
        let ingredient_id = options
            .ingredient_id
            .as_deref()
            .and_then(normalize_filter_text);
        let has_structured_filter =
            kind.is_some() || target_id.is_some() || ingredient_id.is_some();
        if query.is_empty() && !has_structured_filter {
            return Ok(Vec::new());
        }
        let mut hits = self
            .chunks
            .iter()
            .filter(|chunk| chunk_matches_kind(chunk, kind.as_deref()))
            .filter(|chunk| chunk_matches_target(chunk, target_id.as_deref()))
            .filter(|chunk| chunk_matches_ingredient(chunk, ingredient_id.as_deref()))
            .filter_map(|chunk| {
                let score = if query.is_empty() {
                    0.0
                } else {
                    score_chunk(chunk, &query)
                } + structured_filter_score(
                    chunk,
                    target_id.as_deref(),
                    ingredient_id.as_deref(),
                ) + source_priority_score(chunk);
                (score > 0.0).then(|| WikiSearchHit {
                    chunk_id: chunk.chunk_id.clone(),
                    document_id: chunk.document_id.clone(),
                    title: chunk.title.clone(),
                    snippet: snippet_for_terms(&chunk.content, &query.snippet_terms),
                    source_label: chunk.source_label.clone(),
                    location: chunk.location.clone(),
                    score,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WikiCorpusCache {
    version: u32,
    corpus_id: String,
    fingerprint: String,
    source_count: usize,
    chunks: Vec<WikiChunk>,
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
        return corpus_from_cache_or_rebuild(scope, path).await;
    }
    build_wiki_corpus_from_paths(scope, source_paths).await
}

async fn build_wiki_corpus_from_paths(
    scope: WikiScope,
    source_paths: Vec<PathBuf>,
) -> CoreResult<WikiCorpus> {
    let source_count = source_paths.len();
    let local_source_paths = source_paths.clone();
    let mut documents =
        tokio::task::spawn_blocking(move || read_local_wiki_documents(&local_source_paths))
            .await
            .map_err(|err| CoreError::other(format!("wiki corpus build task failed: {err}")))??;
    documents.extend(read_project_wiki_documents(&source_paths).await);
    WikiCorpus::from_documents(scope, source_count, documents)
}

fn cacheable_wiki_source_path(source_paths: &[PathBuf]) -> Option<&Path> {
    let [path] = source_paths else {
        return None;
    };
    regular_dir(path).then_some(path.as_path())
}

pub fn wiki_corpus_cache_path(instance_dir: &Path) -> PathBuf {
    instance_dir.join(WIKI_CORPUS_CACHE_FILE)
}

pub async fn prebuild_wiki_corpus_cache(
    modpack_id: String,
    instance_id: Option<String>,
    instance_dir: &Path,
) -> CoreResult<()> {
    refresh_wiki_corpus_cache(modpack_id, instance_id, instance_dir).await
}

pub async fn refresh_wiki_corpus_cache(
    modpack_id: String,
    instance_id: Option<String>,
    instance_dir: &Path,
) -> CoreResult<()> {
    let scope = WikiScope::from_modpack_entry(modpack_id, instance_id)?;
    let fingerprint = wiki_source_fingerprint(instance_dir).await?;
    let corpus = build_wiki_corpus_from_paths(scope, vec![instance_dir.to_path_buf()]).await?;
    write_wiki_corpus_cache(instance_dir, &fingerprint, &corpus)?;
    Ok(())
}

async fn corpus_from_cache_or_rebuild(
    scope: WikiScope,
    instance_dir: &Path,
) -> CoreResult<WikiCorpus> {
    let fingerprint = wiki_source_fingerprint(instance_dir).await?;
    if let Some(corpus) = read_wiki_corpus_cache(instance_dir, &scope, &fingerprint)? {
        tracing::debug!(
            corpus_id = %scope.corpus_id,
            path = %wiki_corpus_cache_path(instance_dir).display(),
            "loaded wiki corpus cache"
        );
        return Ok(corpus);
    }

    tracing::debug!(
        corpus_id = %scope.corpus_id,
        path = %wiki_corpus_cache_path(instance_dir).display(),
        "rebuilding wiki corpus cache"
    );
    let corpus = build_wiki_corpus_from_paths(scope, vec![instance_dir.to_path_buf()]).await?;
    write_wiki_corpus_cache(instance_dir, &fingerprint, &corpus)?;
    Ok(corpus)
}

fn read_wiki_corpus_cache(
    instance_dir: &Path,
    scope: &WikiScope,
    fingerprint: &str,
) -> CoreResult<Option<WikiCorpus>> {
    let cache_path = wiki_corpus_cache_path(instance_dir);
    if !regular_file(&cache_path) {
        return Ok(None);
    }
    let bytes = match std::fs::read(&cache_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(error = %err, path = %cache_path.display(), "failed to read wiki corpus cache");
            return Ok(None);
        }
    };
    let cache: WikiCorpusCache = match serde_json::from_slice(&bytes) {
        Ok(cache) => cache,
        Err(err) => {
            tracing::warn!(error = %err, path = %cache_path.display(), "failed to parse wiki corpus cache");
            return Ok(None);
        }
    };
    if cache.version != WIKI_CORPUS_CACHE_VERSION
        || cache.corpus_id != scope.corpus_id
        || cache.fingerprint != fingerprint
    {
        return Ok(None);
    }
    Ok(Some(WikiCorpus {
        scope: scope.clone(),
        source_count: cache.source_count,
        chunks: cache.chunks,
    }))
}

fn write_wiki_corpus_cache(
    instance_dir: &Path,
    fingerprint: &str,
    corpus: &WikiCorpus,
) -> CoreResult<()> {
    let cache_path = wiki_corpus_cache_path(instance_dir);
    let cache = WikiCorpusCache {
        version: WIKI_CORPUS_CACHE_VERSION,
        corpus_id: corpus.scope.corpus_id.clone(),
        fingerprint: fingerprint.to_string(),
        source_count: corpus.source_count,
        chunks: corpus.chunks.clone(),
    };
    let bytes = serde_json::to_vec(&cache).map_err(|err| CoreError::Parse {
        what: "wiki corpus cache".into(),
        source: err,
    })?;
    crate::fs::write_atomic(&cache_path, &bytes)
}

async fn wiki_source_fingerprint(instance_dir: &Path) -> CoreResult<String> {
    let root = instance_dir.to_path_buf();
    tokio::task::spawn_blocking(move || wiki_source_fingerprint_sync(&root))
        .await
        .map_err(|err| CoreError::other(format!("wiki fingerprint task failed: {err}")))?
}

fn wiki_source_fingerprint_sync(instance_dir: &Path) -> CoreResult<String> {
    if !regular_dir(instance_dir) {
        return Err(CoreError::other(format!(
            "wiki source path does not exist or is not a directory: {}",
            instance_dir.display()
        )));
    }
    let mut entries = Vec::new();
    collect_wiki_fingerprint_entries(instance_dir, &mut entries);
    entries.sort();
    entries.dedup();

    let mut hasher = Sha256::new();
    for entry in entries {
        hasher.update(entry.as_bytes());
        hasher.update(b"\n");
    }
    Ok(hex::encode(hasher.finalize()))
}

fn collect_wiki_fingerprint_entries(root: &Path, entries: &mut Vec<String>) {
    let mut files = Vec::new();
    let _ = collect_wiki_files(root, &mut files);
    for file in files {
        push_fingerprint_entry(root, &file, entries);
    }
    for rel in ["instance.json", "mmc-pack.json"] {
        push_fingerprint_entry(root, &root.join(rel), entries);
    }
    for rel in INSTANCE_DATA_DIRS {
        collect_instance_data_fingerprint_entries(root, &root.join(rel), entries);
    }
}

fn collect_instance_data_fingerprint_entries(root: &Path, dir: &Path, entries: &mut Vec<String>) {
    let Ok(meta) = std::fs::symlink_metadata(dir) else {
        return;
    };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            collect_instance_data_fingerprint_entries(root, &path, entries);
        } else if kind.is_file() {
            push_fingerprint_entry(root, &path, entries);
        }
    }
}

fn push_fingerprint_entry(root: &Path, path: &Path, entries: &mut Vec<String>) {
    if is_wiki_corpus_cache_file(path) {
        return;
    }
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return;
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    entries.push(format!(
        "{}\0{}\0{}",
        relative_slash_path(root, path),
        meta.len(),
        modified
    ));
}

async fn read_project_wiki_documents(source_paths: &[PathBuf]) -> Vec<WikiSourceDocument> {
    let mut docs = Vec::new();
    for path in source_paths {
        if !regular_dir(path) {
            continue;
        }
        match project_wiki_document_from_instance_dir(path).await {
            Ok(Some(doc)) => docs.push(doc),
            Ok(None) => {}
            Err(err) => {
                tracing::debug!(
                    error = %err,
                    path = %path.display(),
                    "failed to load project wiki document"
                );
            }
        }
    }
    docs
}

async fn project_wiki_document_from_instance_dir(
    instance_dir: &Path,
) -> CoreResult<Option<WikiSourceDocument>> {
    let config_path = instance_dir.join("instance.json");
    if !regular_file(&config_path) {
        return Ok(None);
    }
    let config = InstanceConfig::load(&config_path)?;
    let Some(source) = config.source else {
        return Ok(None);
    };
    let provider = source.provider.trim().to_ascii_lowercase();
    let project_id = source.project_id.trim().to_string();
    if project_id.is_empty() {
        return Ok(None);
    }
    let cache_dir = instance_dir.join(WIKI_PROJECT_CACHE_DIR);
    if let Some(detail) = read_cached_project_detail(
        &cache_dir,
        &provider,
        &project_id,
        Some(WIKI_PROJECT_CACHE_TTL),
    ) {
        return Ok(Some(project_detail_document(
            &provider,
            &project_id,
            detail,
        )));
    }

    let detail = if provider == "modrinth" && !cfg!(test) {
        match tokio::time::timeout(
            WIKI_PROJECT_FETCH_TIMEOUT,
            ModrinthApi::new().project_details_cached(
                &project_id,
                &cache_dir,
                WIKI_PROJECT_CACHE_TTL,
            ),
        )
        .await
        {
            Ok(Ok(detail)) => Some(detail),
            Ok(Err(err)) => {
                tracing::debug!(error = %err, project_id = %project_id, "failed to load Modrinth project details");
                read_cached_project_detail(&cache_dir, &provider, &project_id, None)
            }
            Err(_) => {
                tracing::debug!(project_id = %project_id, "timed out loading Modrinth project details");
                read_cached_project_detail(&cache_dir, &provider, &project_id, None)
            }
        }
    } else {
        read_cached_project_detail(&cache_dir, &provider, &project_id, None)
    };

    Ok(detail.map(|detail| project_detail_document(&provider, &project_id, detail)))
}

#[derive(Debug, Deserialize)]
struct CachedWikiProjectDetail {
    fetched_at: u64,
    data: ProjectDetail,
}

fn read_cached_project_detail(
    cache_dir: &Path,
    provider: &str,
    project_id: &str,
    ttl: Option<std::time::Duration>,
) -> Option<ProjectDetail> {
    let safe = safe_project_cache_id(project_id);
    let path = cache_dir
        .join(provider)
        .join("project")
        .join(format!("{safe}.json"));
    let bytes = std::fs::read(path).ok()?;
    let cached: CachedWikiProjectDetail = serde_json::from_slice(&bytes).ok()?;
    if let Some(ttl) = ttl {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        if now.saturating_sub(cached.fetched_at) >= ttl.as_secs() {
            return None;
        }
    }
    Some(cached.data)
}

fn safe_project_cache_id(project_id: &str) -> String {
    project_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

fn project_detail_document(
    provider: &str,
    project_id: &str,
    detail: ProjectDetail,
) -> WikiSourceDocument {
    let structured = serde_json::json!({
        "kind": "project_doc",
        "provider": provider,
        "project_id": project_id,
        "title": detail.title,
        "slug": detail.slug,
        "description": detail.description,
        "body": detail.body,
        "categories": detail.categories,
        "links": {
            "source_url": detail.source_url,
            "issues_url": detail.issues_url,
            "wiki_url": detail.wiki_url,
            "discord_url": detail.discord_url,
        },
    });
    let mut lines = vec![
        "kind: project_doc".to_string(),
        format!(
            "title: {}",
            structured
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or(project_id)
        ),
        format!("provider: {provider}"),
        format!("project_id: {project_id}"),
    ];
    for key in ["description", "body"] {
        if let Some(text) = structured.get(key).and_then(|value| value.as_str()) {
            if !text.trim().is_empty() {
                lines.push(format!("{key}: {text}"));
            }
        }
    }
    if let Some(links) = structured.get("links").and_then(|value| value.as_object()) {
        for (key, value) in links {
            if let Some(url) = value.as_str().filter(|url| !url.trim().is_empty()) {
                lines.push(format!("{key}: {url}"));
            }
        }
    }
    WikiSourceDocument::structured(
        format!(
            "Project: {} ({provider}:{project_id})",
            structured
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or(project_id)
        ),
        "generated:project-doc".to_string(),
        format!("generated://project-doc/{provider}/{project_id}"),
        lines.join("\n"),
        "project_doc",
        structured,
    )
}

fn read_local_wiki_documents(paths: &[PathBuf]) -> CoreResult<Vec<WikiSourceDocument>> {
    let mut docs = Vec::new();
    let mut total_bytes = 0usize;
    for path in paths {
        if !path.exists() {
            return Err(CoreError::other(format!(
                "wiki source path does not exist: {}",
                path.display()
            )));
        }
        if is_symlink(path) {
            continue;
        }
        if is_archive_path(path) {
            push_bounded_docs(&mut docs, read_archive_wiki_texts(path)?, &mut total_bytes);
        } else {
            if regular_dir(path) {
                if let Some(doc) = generated_instance_data_document(path)? {
                    push_bounded_doc(&mut docs, doc, &mut total_bytes);
                }
                push_bounded_docs(
                    &mut docs,
                    read_structured_gameplay_documents(path)?,
                    &mut total_bytes,
                );
                push_bounded_docs(&mut docs, read_ftb_quest_documents(path)?, &mut total_bytes);
            }
            let mut files = Vec::new();
            collect_wiki_files(path, &mut files)?;
            files.sort();
            for file in files {
                if docs.len() >= WIKI_CORPUS_MAX_DOCUMENTS {
                    break;
                }
                let Some(content) = read_text_file_bounded(&file, WIKI_FILE_MAX_BYTES) else {
                    continue;
                };
                push_bounded_doc(
                    &mut docs,
                    document_from_parts(file.to_string_lossy().to_string(), content),
                    &mut total_bytes,
                );
            }
        }
    }
    docs.sort_by(|a, b| a.uri.cmp(&b.uri));
    Ok(docs)
}

fn push_bounded_docs(
    docs: &mut Vec<WikiSourceDocument>,
    incoming: Vec<WikiSourceDocument>,
    total_bytes: &mut usize,
) {
    for doc in incoming {
        push_bounded_doc(docs, doc, total_bytes);
    }
}

fn push_bounded_doc(
    docs: &mut Vec<WikiSourceDocument>,
    doc: WikiSourceDocument,
    total_bytes: &mut usize,
) {
    if docs.len() >= WIKI_CORPUS_MAX_DOCUMENTS {
        return;
    }
    let bytes = doc.content.len();
    if total_bytes.saturating_add(bytes) > WIKI_CORPUS_MAX_BYTES {
        return;
    }
    *total_bytes += bytes;
    docs.push(doc);
}

fn generated_instance_data_document(path: &Path) -> CoreResult<Option<WikiSourceDocument>> {
    let mut lines = vec![
        "Current modpack instance data".to_string(),
        format!("Instance directory: {}", path.display()),
    ];
    let mut has_data = false;

    let instance_config_path = path.join("instance.json");
    if regular_file(&instance_config_path) {
        if let Ok(config) = InstanceConfig::load(&instance_config_path) {
            has_data = true;
            lines.push(String::new());
            lines.push("Instance config:".to_string());
            if let Some(name) = config.name.filter(|name| !name.trim().is_empty()) {
                lines.push(format!("Instance name: {name}"));
            }
            lines.push(format!("Memory: {} MB", config.memory_mb));
            if let Some(server) = config.server.filter(|server| !server.trim().is_empty()) {
                lines.push(format!("Server: {server}"));
            }
            if !config.tags.is_empty() {
                lines.push(format!("Tags: {}", config.tags.join(", ")));
            }
            if let Some(source) = config.source {
                lines.push(format!("Source provider: {}", source.provider));
                lines.push(format!("Source project id: {}", source.project_id));
                if let Some(version_id) = source.version_id {
                    lines.push(format!("Source version id: {version_id}"));
                }
            }
        }
    }

    if let Ok(Some(pack)) = PackProfile::load(path) {
        has_data = true;
        lines.push(String::new());
        lines.push("Version components:".to_string());
        if let Some(mc) = pack.minecraft_version() {
            lines.push(format!("Minecraft version: {mc}"));
        }
        lines.push(format!("Detected loader: {:?}", pack.detect_loader()));
        for component in pack
            .components
            .iter()
            .filter(|component| component.is_active())
        {
            let version = component.version.as_deref().unwrap_or("unknown");
            lines.push(format!("- {}: {version}", component.uid));
        }
    }

    for rel in INSTANCE_DATA_DIRS {
        let entries = collect_instance_data_entries(path, rel)?;
        if entries.is_empty() {
            continue;
        }
        has_data = true;
        lines.push(String::new());
        lines.push(format!("{rel} files:"));
        for entry in entries {
            lines.push(format!("- {entry}"));
        }
    }

    if !has_data {
        return Ok(None);
    }
    Ok(Some(WikiSourceDocument::structured(
        "Current modpack instance data".to_string(),
        "generated:instance-data".to_string(),
        format!("generated://instance-data/{}", path.display()),
        lines.join("\n"),
        "instance_data",
        serde_json::json!({
            "kind": "instance_data",
            "source": {
                "origin": "local",
                "uri": path.display().to_string(),
            },
        }),
    )))
}

fn read_ftb_quest_documents(root: &Path) -> CoreResult<Vec<WikiSourceDocument>> {
    let mut files = Vec::new();
    for rel in FTB_QUESTS_DIRS {
        let dir = root.join(rel);
        if regular_dir(&dir) {
            collect_ftb_quest_files(&dir, &mut files)?;
        }
    }
    files.sort();
    files.dedup();

    let mut docs = Vec::new();
    for file in files {
        if !is_allowed_ftb_quest_file(&file)? {
            continue;
        }
        let Some(content) = read_text_file_bounded(&file, FTB_QUESTS_FILE_MAX_BYTES) else {
            continue;
        };
        let rel = relative_slash_path(root, &file);
        let mut structured = ftb_quest_documents_from_content(&rel, &content);
        docs.append(&mut structured);
        docs.push(WikiSourceDocument::text(
            format!("FTB Quests: {rel}"),
            "generated:ftb-quests".to_string(),
            format!("generated://ftb-quests/{rel}"),
            format!("FTB Quests source file: {rel}\n\n{content}"),
        ));
    }
    Ok(docs)
}

fn read_structured_gameplay_documents(root: &Path) -> CoreResult<Vec<WikiSourceDocument>> {
    let mut labels = HashMap::new();
    let mut docs = read_mod_jar_structured_documents(root, &mut labels)?;
    docs.extend(read_kubejs_recipe_script_documents(root, &labels)?);
    docs.extend(read_local_data_structured_documents(root, &labels)?);
    docs.sort_by(|a, b| a.uri.cmp(&b.uri));
    Ok(docs)
}

fn read_mod_jar_structured_documents(
    root: &Path,
    global_labels: &mut HashMap<String, String>,
) -> CoreResult<Vec<WikiSourceDocument>> {
    let mods_dir = root.join("mods");
    if !regular_dir(&mods_dir) {
        return Ok(Vec::new());
    }

    let mut jars = Vec::new();
    let Ok(read) = std::fs::read_dir(&mods_dir) else {
        return Ok(Vec::new());
    };
    for entry in read.flatten() {
        let path = entry.path();
        if regular_file(&path)
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("jar"))
                .unwrap_or(false)
        {
            jars.push(path);
        }
    }
    jars.sort();

    let mut docs = Vec::new();
    for jar in jars {
        match read_single_mod_jar_structured_documents(root, &jar, global_labels) {
            Ok((mut jar_docs, labels)) => {
                global_labels.extend(labels);
                docs.append(&mut jar_docs);
            }
            Err(err) => {
                tracing::debug!(
                    error = %err,
                    path = %jar.display(),
                    "skipping unreadable mod jar while indexing wiki"
                );
            }
        }
    }
    Ok(docs)
}

fn read_single_mod_jar_structured_documents(
    root: &Path,
    jar_path: &Path,
    global_labels: &HashMap<String, String>,
) -> CoreResult<(Vec<WikiSourceDocument>, HashMap<String, String>)> {
    let file = std::fs::File::open(jar_path).with_path(jar_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|err| CoreError::Zip(err.to_string()))?;
    let mut lang_entries = Vec::new();
    let mut recipe_entries = Vec::new();
    let mut tag_entries = Vec::new();
    let mut patchouli_entries = Vec::new();

    for index in 0..archive.len() {
        let Ok(entry) = archive.by_index(index) else {
            continue;
        };
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if should_skip_virtual_path(&name) {
            continue;
        }
        if is_lang_entry(&name) {
            lang_entries.push(name);
        } else if is_recipe_entry(&name) {
            recipe_entries.push(name);
        } else if is_tag_entry(&name) {
            tag_entries.push(name);
        } else if is_patchouli_entry(&name) {
            patchouli_entries.push(name);
        }
    }

    let mut local_labels = HashMap::new();
    for name in preferred_lang_entries(&lang_entries) {
        if let Some(content) = read_zip_text_by_name(&mut archive, &name) {
            local_labels.extend(labels_from_lang_json(&content));
        }
    }

    let mut labels = global_labels.clone();
    labels.extend(local_labels.clone());
    let jar_rel = relative_slash_path(root, jar_path);
    let mut docs = Vec::new();
    for name in recipe_entries {
        let Some(content) = read_zip_text_by_name(&mut archive, &name) else {
            continue;
        };
        let uri = format!("{}!{name}", jar_path.display());
        if let Some(doc) = recipe_document_from_json(&uri, &jar_rel, &name, &content, &labels) {
            docs.push(doc);
        }
    }
    for name in tag_entries {
        let Some(content) = read_zip_text_by_name(&mut archive, &name) else {
            continue;
        };
        let uri = format!("{}!{name}", jar_path.display());
        if let Some(doc) = tag_document_from_json(&uri, &jar_rel, &name, &content, &labels) {
            docs.push(doc);
        }
    }
    for name in patchouli_entries {
        let Some(content) = read_zip_text_by_name(&mut archive, &name) else {
            continue;
        };
        let uri = format!("{}!{name}", jar_path.display());
        if let Some(doc) = patchouli_document_from_json(&uri, &jar_rel, &name, &content) {
            docs.push(doc);
        }
    }
    Ok((docs, local_labels))
}

fn read_local_data_structured_documents(
    root: &Path,
    labels: &HashMap<String, String>,
) -> CoreResult<Vec<WikiSourceDocument>> {
    let mut files = Vec::new();
    for rel in ["datapacks", "kubejs/data"] {
        let dir = root.join(rel);
        if regular_dir(&dir) {
            collect_json_files(&dir, &mut files)?;
        }
    }
    files.sort();
    files.dedup();

    let mut docs = Vec::new();
    for file in files {
        let Some(entry_name) = virtual_data_entry_name(root, &file) else {
            continue;
        };
        if !(is_recipe_entry(&entry_name)
            || is_tag_entry(&entry_name)
            || is_patchouli_entry(&entry_name))
        {
            continue;
        }
        let Some(content) = read_text_file_bounded(&file, WIKI_FILE_MAX_BYTES) else {
            continue;
        };
        let uri = file.to_string_lossy().to_string();
        let source_rel = relative_slash_path(root, &file);
        if is_recipe_entry(&entry_name) {
            if let Some(doc) =
                recipe_document_from_json(&uri, &source_rel, &entry_name, &content, labels)
            {
                docs.push(doc);
            }
        } else if is_tag_entry(&entry_name) {
            if let Some(doc) =
                tag_document_from_json(&uri, &source_rel, &entry_name, &content, labels)
            {
                docs.push(doc);
            }
        } else if let Some(doc) =
            patchouli_document_from_json(&uri, &source_rel, &entry_name, &content)
        {
            docs.push(doc);
        }
    }
    Ok(docs)
}

fn read_kubejs_recipe_script_documents(
    root: &Path,
    labels: &HashMap<String, String>,
) -> CoreResult<Vec<WikiSourceDocument>> {
    let scripts_dir = root.join("kubejs").join("server_scripts");
    if !regular_dir(&scripts_dir) {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_files_with_extension(&scripts_dir, "js", &mut files)?;
    files.sort();

    let mut docs = Vec::new();
    for file in files {
        let Some(content) = read_text_file_bounded(&file, WIKI_FILE_MAX_BYTES) else {
            continue;
        };
        docs.extend(kubejs_recipe_documents_from_script(
            root, &file, &content, labels,
        ));
    }
    Ok(docs)
}

fn collect_files_with_extension(
    dir: &Path,
    extension: &str,
    files: &mut Vec<PathBuf>,
) -> CoreResult<()> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in read {
        let entry = entry.with_path(dir)?;
        let path = entry.path();
        let kind = entry.file_type().with_path(&path)?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            collect_files_with_extension(&path, extension, files)?;
        } else if kind.is_file()
            && entry.metadata().with_path(&path)?.len() <= WIKI_FILE_MAX_BYTES
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case(extension))
                .unwrap_or(false)
        {
            files.push(path);
        }
    }
    Ok(())
}

fn kubejs_recipe_documents_from_script(
    root: &Path,
    file: &Path,
    content: &str,
    labels: &HashMap<String, String>,
) -> Vec<WikiSourceDocument> {
    let source_rel = relative_slash_path(root, file);
    let file_uri = file.to_string_lossy().to_string();
    let mut docs = Vec::new();

    for (idx, args) in extract_js_call_arguments(content, "event.remove")
        .into_iter()
        .enumerate()
    {
        if let Some(doc) =
            kubejs_recipe_override_document(&file_uri, &source_rel, idx, "remove", &args, None)
        {
            docs.push(doc);
        }
    }

    for (idx, args) in extract_js_call_arguments(content, "event.replaceInput")
        .into_iter()
        .enumerate()
    {
        if let Some(doc) = kubejs_recipe_override_document(
            &file_uri,
            &source_rel,
            idx,
            "replace_input",
            &args,
            Some("input"),
        ) {
            docs.push(doc);
        }
    }

    for (idx, args) in extract_js_call_arguments(content, "event.replaceOutput")
        .into_iter()
        .enumerate()
    {
        if let Some(doc) = kubejs_recipe_override_document(
            &file_uri,
            &source_rel,
            idx,
            "replace_output",
            &args,
            Some("output"),
        ) {
            docs.push(doc);
        }
    }

    for (idx, args) in extract_js_call_arguments(content, "event.custom")
        .into_iter()
        .enumerate()
    {
        let Some(json) = parse_kubejs_json_like_object(&args) else {
            continue;
        };
        let uri = format!("{file_uri}#custom-{idx}");
        let entry_name = format!("{source_rel}#custom-{idx}");
        if let Some(doc) = recipe_document_from_json(&uri, &source_rel, &entry_name, &json, labels)
        {
            docs.push(doc);
        }
    }

    docs
}

fn kubejs_recipe_override_document(
    file_uri: &str,
    source_rel: &str,
    call_index: usize,
    action: &str,
    args: &str,
    replacement_role: Option<&str>,
) -> Option<WikiSourceDocument> {
    let filter = first_js_object_literal(args).unwrap_or(args);
    let target_id = js_object_string_property(filter, "output")
        .or_else(|| js_object_string_property(filter, "result"));
    let recipe_id = js_object_string_property(filter, "id");
    let input_id = js_object_string_property(filter, "input");
    let quoted_args = js_quoted_strings(args);
    let replacement = replacement_role.and_then(|role| {
        let values = quoted_args
            .iter()
            .filter(|value| {
                Some(value.as_str()) != target_id.as_deref()
                    && Some(value.as_str()) != recipe_id.as_deref()
            })
            .cloned()
            .collect::<Vec<_>>();
        match role {
            "input" if values.len() >= 2 => Some(serde_json::json!({
                "from": ingredient_ref_value(&values[0]),
                "to": ingredient_ref_value(&values[1]),
            })),
            "output" if values.len() >= 2 => Some(serde_json::json!({
                "from": ingredient_ref_value(&values[0]),
                "to": ingredient_ref_value(&values[1]),
            })),
            _ => None,
        }
    });
    if target_id.is_none() && recipe_id.is_none() && input_id.is_none() && replacement.is_none() {
        return None;
    }

    let target = target_id
        .as_deref()
        .map(ingredient_ref_value)
        .unwrap_or(Value::Null);
    let input = input_id
        .as_deref()
        .map(ingredient_ref_value)
        .unwrap_or(Value::Null);
    let structured = serde_json::json!({
        "kind": "recipe_override",
        "action": action,
        "target": target,
        "target_id": target_id,
        "recipe_id": recipe_id,
        "input": input,
        "replacement": replacement,
        "source": {
            "origin": "local",
            "type": "kubejs",
            "uri": file_uri,
            "file": source_rel,
        },
    });
    let mut lines = vec![
        "kind: recipe_override".to_string(),
        format!("action: {action}"),
        format!("source: {source_rel}"),
        format!("entry: kubejs:{source_rel}#{action}-{call_index}"),
    ];
    if let Some(target_id) = target_id {
        lines.push(format!("target: {target_id}"));
    }
    if let Some(recipe_id) = recipe_id {
        lines.push(format!("recipe_id: {recipe_id}"));
    }
    if let Some(input_id) = input_id {
        lines.push(format!("input: {input_id}"));
    }
    if let Some(replacement) = structured
        .get("replacement")
        .filter(|value| !value.is_null())
    {
        lines.push(format!("replacement: {replacement}"));
    }

    let target_title = structured
        .pointer("/target/id")
        .and_then(|value| value.as_str())
        .or_else(|| structured.get("recipe_id").and_then(|value| value.as_str()))
        .unwrap_or("recipe");
    Some(WikiSourceDocument::structured(
        format!("KubeJS recipe override: {action} {target_title}"),
        "generated:recipe-override".to_string(),
        format!("generated://recipe-override/{source_rel}#{action}-{call_index}"),
        lines.join("\n"),
        "recipe_override",
        structured,
    ))
}

fn ingredient_ref_value(id: &str) -> Value {
    if let Some(tag) = id.strip_prefix('#') {
        serde_json::json!({ "kind": "tag", "id": format!("#{tag}"), "label": format!("#{tag}") })
    } else {
        serde_json::json!({ "kind": "item", "id": id, "label": id })
    }
}

fn extract_js_call_arguments(content: &str, marker: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = content[cursor..].find(marker) {
        let start = cursor + relative + marker.len();
        let Some(open_relative) = content[start..].find('(') else {
            break;
        };
        let open = start + open_relative;
        if let Some((close, args)) = balanced_delimited(content, open, '(', ')') {
            out.push(args.to_string());
            cursor = close + 1;
        } else {
            break;
        }
    }
    out
}

fn first_js_object_literal(input: &str) -> Option<&str> {
    let open = input.find('{')?;
    balanced_delimited(input, open, '{', '}').map(|(_, body)| body)
}

fn parse_kubejs_json_like_object(input: &str) -> Option<String> {
    let object = first_js_object_literal(input)?;
    let object = normalize_kubejs_object_body(object)?;
    let json = format!("{{{object}}}");
    serde_json::from_str::<Value>(&json).ok()?;
    Some(json)
}

fn normalize_kubejs_object_body(input: &str) -> Option<String> {
    let chars = input.chars().collect::<Vec<_>>();
    let mut out = String::new();
    let mut idx = 0usize;
    while idx < chars.len() {
        let ch = chars[idx];
        if ch == '\'' || ch == '"' {
            let (value, next) = read_js_string_chars(&chars, idx)?;
            out.push_str(&serde_json::to_string(&value).ok()?);
            idx = next;
            continue;
        }
        if ch == '/' && chars.get(idx + 1) == Some(&'/') {
            idx += 2;
            while idx < chars.len() && chars[idx] != '\n' {
                idx += 1;
            }
            continue;
        }
        if ch == '/' && chars.get(idx + 1) == Some(&'*') {
            idx += 2;
            while idx + 1 < chars.len() && !(chars[idx] == '*' && chars[idx + 1] == '/') {
                idx += 1;
            }
            idx = (idx + 2).min(chars.len());
            continue;
        }
        if is_js_identifier_start(ch) {
            let start = idx;
            idx += 1;
            while idx < chars.len() && is_js_identifier_part(chars[idx]) {
                idx += 1;
            }
            let ident = chars[start..idx].iter().collect::<String>();
            let mut lookahead = idx;
            while lookahead < chars.len() && chars[lookahead].is_whitespace() {
                lookahead += 1;
            }
            if chars.get(lookahead) == Some(&':') {
                out.push_str(&serde_json::to_string(&ident).ok()?);
            } else {
                out.push_str(&ident);
            }
            continue;
        }
        out.push(ch);
        idx += 1;
    }
    Some(remove_trailing_json_commas(&out))
}

fn read_js_string_chars(chars: &[char], start: usize) -> Option<(String, usize)> {
    let quote = *chars.get(start)?;
    let mut idx = start + 1;
    let mut out = String::new();
    while idx < chars.len() {
        let ch = chars[idx];
        if ch == quote {
            return Some((out, idx + 1));
        }
        if ch != '\\' {
            out.push(ch);
            idx += 1;
            continue;
        }
        idx += 1;
        let escaped = *chars.get(idx)?;
        match escaped {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'b' => out.push('\u{0008}'),
            'f' => out.push('\u{000c}'),
            '\\' | '"' | '\'' | '/' => out.push(escaped),
            'u' if idx + 4 < chars.len() => {
                let code = chars[idx + 1..=idx + 4].iter().collect::<String>();
                let parsed = u32::from_str_radix(&code, 16).ok()?;
                out.push(char::from_u32(parsed)?);
                idx += 4;
            }
            other => out.push(other),
        }
        idx += 1;
    }
    None
}

fn is_js_identifier_start(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_ascii_alphabetic()
}

fn is_js_identifier_part(ch: char) -> bool {
    is_js_identifier_start(ch) || ch.is_ascii_digit()
}

fn remove_trailing_json_commas(input: &str) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut out = String::new();
    let mut idx = 0usize;
    let mut string_quote: Option<char> = None;
    let mut escaped = false;
    while idx < chars.len() {
        let ch = chars[idx];
        if let Some(quote) = string_quote {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote {
                string_quote = None;
            }
            idx += 1;
            continue;
        }
        if ch == '"' {
            string_quote = Some(ch);
            out.push(ch);
            idx += 1;
            continue;
        }
        if ch == ',' {
            let mut lookahead = idx + 1;
            while lookahead < chars.len() && chars[lookahead].is_whitespace() {
                lookahead += 1;
            }
            if matches!(chars.get(lookahead), Some(&'}') | Some(&']')) {
                idx += 1;
                continue;
            }
        }
        out.push(ch);
        idx += 1;
    }
    out
}

fn js_object_string_property(object: &str, key: &str) -> Option<String> {
    for pattern in [
        format!("{key}:"),
        format!("\"{key}\":"),
        format!("'{key}':"),
    ] {
        if let Some(idx) = object.find(&pattern) {
            let value_start = idx + pattern.len();
            return read_js_string_or_bare_value(&object[value_start..]);
        }
    }
    None
}

fn read_js_string_or_bare_value(input: &str) -> Option<String> {
    let input = input.trim_start();
    let first = input.chars().next()?;
    if first == '\'' || first == '"' {
        let quote = first;
        let mut escaped = false;
        let mut out = String::new();
        for ch in input[first.len_utf8()..].chars() {
            if escaped {
                out.push(ch);
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote {
                return Some(out);
            }
            out.push(ch);
        }
        return None;
    }
    let value = input
        .split(|ch: char| ch == ',' || ch == '}' || ch == ')' || ch.is_whitespace())
        .next()?
        .trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn js_quoted_strings(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = input.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if ch != '\'' && ch != '"' {
            continue;
        }
        let quote = ch;
        let mut escaped = false;
        let mut value = String::new();
        for (end_idx, next) in chars.by_ref() {
            if escaped {
                value.push(next);
                escaped = false;
                continue;
            }
            if next == '\\' {
                escaped = true;
                continue;
            }
            if next == quote {
                out.push(value);
                break;
            }
            value.push(next);
            if end_idx <= idx {
                break;
            }
        }
    }
    out
}

fn balanced_delimited(
    input: &str,
    open_idx: usize,
    open: char,
    close: char,
) -> Option<(usize, &str)> {
    let mut depth = 0usize;
    let mut string_quote: Option<char> = None;
    let mut escaped = false;
    let body_start = open_idx + open.len_utf8();
    for (idx, ch) in input[open_idx..].char_indices() {
        let idx = open_idx + idx;
        if let Some(quote) = string_quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote {
                string_quote = None;
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            string_quote = Some(ch);
            continue;
        }
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some((idx, &input[body_start..idx]));
            }
        }
    }
    None
}

fn preferred_lang_entries(entries: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for lang in ["en_us", "zh_tw", "zh_cn", "zh_hans"] {
        let suffix = format!("/{lang}.json");
        for entry in entries {
            if entry.ends_with(&suffix) {
                out.push(entry.clone());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn read_zip_text_by_name<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Option<String> {
    let mut entry = archive.by_name(name).ok()?;
    if entry.is_dir() {
        return None;
    }
    let mut bytes = Vec::new();
    (&mut entry)
        .take(MOD_JAR_ENTRY_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MOD_JAR_ENTRY_MAX_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn collect_json_files(dir: &Path, files: &mut Vec<PathBuf>) -> CoreResult<()> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in read.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            collect_json_files(&path, files)?;
        } else if kind.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("json"))
                .unwrap_or(false)
        {
            files.push(path);
        }
    }
    Ok(())
}

fn virtual_data_entry_name(root: &Path, path: &Path) -> Option<String> {
    let rel = relative_slash_path(root, path);
    let parts = rel.split('/').collect::<Vec<_>>();
    let data_index = parts.iter().position(|part| *part == "data")?;
    Some(parts[data_index..].join("/"))
}

fn labels_from_lang_json(content: &str) -> HashMap<String, String> {
    let Ok(value) = serde_json::from_str::<HashMap<String, String>>(content) else {
        return HashMap::new();
    };
    value
        .into_iter()
        .filter_map(|(key, label)| translation_key_item_id(&key).map(|id| (id, label)))
        .collect()
}

fn translation_key_item_id(key: &str) -> Option<String> {
    let rest = key
        .strip_prefix("item.")
        .or_else(|| key.strip_prefix("block."))?;
    let (namespace, path) = rest.split_once('.')?;
    if namespace.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("{namespace}:{}", path.replace('.', "_")))
}

fn recipe_document_from_json(
    uri: &str,
    source_rel: &str,
    entry_name: &str,
    content: &str,
    labels: &HashMap<String, String>,
) -> Option<WikiSourceDocument> {
    let value: Value = serde_json::from_str(content).ok()?;
    let result = recipe_result_value(&value, labels)?;
    let result_id = result.get("id")?.as_str()?.to_string();
    let result_label = result
        .get("label")
        .and_then(|value| value.as_str())
        .unwrap_or(&result_id)
        .to_string();
    let recipe_type = value
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .to_string();
    let ingredients = recipe_ingredients_value(&value, labels);
    let pattern = recipe_pattern_value(&value);
    let grid = recipe_grid_value(&value, labels);
    let recipe_id = format!("recipe:{}:{}", result_id, stable_hex(uri));
    let structured = serde_json::json!({
        "kind": "recipe",
        "id": recipe_id,
        "type": recipe_type,
        "result": result,
        "ingredients": ingredients,
        "pattern": pattern,
        "grid": grid,
        "source": {
            "origin": "local",
            "type": source_type_for_uri(uri),
            "uri": uri,
            "file": source_rel,
        },
    });
    let mut lines = vec![
        "kind: recipe".to_string(),
        format!("title: {result_label}"),
        format!("result: {result_id}"),
        format!("result_label: {result_label}"),
        format!("recipe_type: {recipe_type}"),
        format!("source: {source_rel}"),
        format!("entry: {entry_name}"),
    ];
    for ingredient in collect_ingredient_terms(structured.get("ingredients")) {
        lines.push(format!("ingredient: {ingredient}"));
    }
    if let Some(rows) = pattern.as_array() {
        for row in rows.iter().filter_map(|row| row.as_str()) {
            lines.push(format!("pattern: {row}"));
        }
    }
    Some(WikiSourceDocument::structured(
        format!("Recipe: {result_label} ({result_id})"),
        "generated:recipe".to_string(),
        format!("generated://recipe/{entry_name}#{}", stable_hex(uri)),
        lines.join("\n"),
        "recipe",
        structured,
    ))
}

fn recipe_result_value(value: &Value, labels: &HashMap<String, String>) -> Option<Value> {
    let result = primary_recipe_result(value)?;
    let (id, count) = if let Some(id) = result.as_str() {
        (id.to_string(), 1u64)
    } else {
        let object = result.as_object()?;
        let id = object
            .get("item")
            .or_else(|| object.get("id"))
            .and_then(|value| value.as_str())?
            .to_string();
        let count = object
            .get("count")
            .and_then(|value| value.as_u64())
            .unwrap_or(1);
        (id, count)
    };
    Some(serde_json::json!({
        "id": id,
        "label": item_label(&id, labels),
        "count": count,
    }))
}

fn primary_recipe_result(value: &Value) -> Option<&Value> {
    if let Some(result) = value.get("result").or_else(|| value.get("output")) {
        return Some(result);
    }
    match value.get("results") {
        Some(Value::Array(items)) => items.iter().find(|item| recipe_result_id(item).is_some()),
        Some(value) if recipe_result_id(value).is_some() => Some(value),
        _ => None,
    }
}

fn recipe_result_id(value: &Value) -> Option<&str> {
    if let Some(id) = value.as_str() {
        return Some(id);
    }
    value
        .as_object()?
        .get("item")
        .or_else(|| value.get("id"))
        .and_then(|value| value.as_str())
}

fn recipe_ingredients_value(value: &Value, labels: &HashMap<String, String>) -> Value {
    if let Some(key) = value.get("key").and_then(|value| value.as_object()) {
        let mut out = serde_json::Map::new();
        for (symbol, ingredient) in key {
            out.insert(symbol.clone(), ingredient_value(ingredient, labels));
        }
        return Value::Object(out);
    }
    if let Some(items) = value.get("ingredients").and_then(|value| value.as_array()) {
        return Value::Array(
            items
                .iter()
                .map(|ingredient| ingredient_value(ingredient, labels))
                .collect(),
        );
    }
    Value::Null
}

fn recipe_pattern_value(value: &Value) -> Value {
    value
        .get("pattern")
        .and_then(|value| value.as_array())
        .map(|rows| {
            Value::Array(
                rows.iter()
                    .filter_map(|row| row.as_str())
                    .map(|row| Value::String(row.to_string()))
                    .collect(),
            )
        })
        .unwrap_or(Value::Null)
}

fn recipe_grid_value(value: &Value, labels: &HashMap<String, String>) -> Value {
    let Some(pattern) = value.get("pattern").and_then(|value| value.as_array()) else {
        return Value::Null;
    };
    let Some(key) = value.get("key").and_then(|value| value.as_object()) else {
        return Value::Null;
    };
    let rows = pattern
        .iter()
        .filter_map(|row| row.as_str())
        .map(|row| {
            Value::Array(
                row.chars()
                    .map(|symbol| {
                        if symbol == ' ' {
                            Value::Null
                        } else {
                            key.get(&symbol.to_string())
                                .map(|ingredient| ingredient_value(ingredient, labels))
                                .unwrap_or(Value::Null)
                        }
                    })
                    .collect(),
            )
        })
        .collect();
    Value::Array(rows)
}

fn ingredient_value(value: &Value, labels: &HashMap<String, String>) -> Value {
    if let Some(id) = value.as_str() {
        return serde_json::json!({
            "kind": "item",
            "id": id,
            "label": item_label(id, labels),
        });
    }
    if let Some(array) = value.as_array() {
        return serde_json::json!({
            "kind": "alternatives",
            "options": array
                .iter()
                .map(|item| ingredient_value(item, labels))
                .collect::<Vec<_>>(),
        });
    }
    let Some(object) = value.as_object() else {
        return serde_json::json!({ "kind": "unknown", "raw": value });
    };
    if let Some(id) = object
        .get("item")
        .or_else(|| object.get("id"))
        .and_then(|value| value.as_str())
    {
        return serde_json::json!({
            "kind": "item",
            "id": id,
            "label": item_label(id, labels),
        });
    }
    if let Some(tag) = object.get("tag").and_then(|value| value.as_str()) {
        return serde_json::json!({
            "kind": "tag",
            "id": format!("#{tag}"),
            "label": format!("#{tag}"),
        });
    }
    serde_json::json!({ "kind": "unknown", "raw": value })
}

fn collect_ingredient_terms(value: Option<&Value>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(value) = value {
        collect_ingredient_terms_inner(value, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

fn collect_ingredient_terms_inner(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_ingredient_terms_inner(item, out);
            }
        }
        Value::Object(map) => {
            if let Some(id) = map.get("id").and_then(|value| value.as_str()) {
                out.push(id.to_string());
            }
            if let Some(label) = map.get("label").and_then(|value| value.as_str()) {
                out.push(label.to_string());
            }
            for value in map.values() {
                collect_ingredient_terms_inner(value, out);
            }
        }
        _ => {}
    }
}

fn tag_document_from_json(
    uri: &str,
    source_rel: &str,
    entry_name: &str,
    content: &str,
    labels: &HashMap<String, String>,
) -> Option<WikiSourceDocument> {
    let value: Value = serde_json::from_str(content).ok()?;
    let tag_id = tag_id_from_entry_name(entry_name)?;
    let values = value
        .get("values")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let normalized_values = values
        .into_iter()
        .filter_map(|value| tag_value(value, labels))
        .collect::<Vec<_>>();
    let value_terms = collect_ingredient_terms(Some(&Value::Array(normalized_values.clone())));
    let value_count = normalized_values.len();
    let values_truncated = value_count > TAG_STRUCTURED_VALUES_MAX;
    let structured_values = normalized_values
        .iter()
        .take(TAG_STRUCTURED_VALUES_MAX)
        .cloned()
        .collect::<Vec<_>>();
    let structured = serde_json::json!({
        "kind": "tag",
        "id": tag_id,
        "replace": value.get("replace").and_then(|value| value.as_bool()).unwrap_or(false),
        "value_count": value_count,
        "values_truncated": values_truncated,
        "values": structured_values,
        "source": {
            "origin": "local",
            "type": source_type_for_uri(uri),
            "uri": uri,
            "file": source_rel,
        },
    });
    let mut lines = vec![
        "kind: tag".to_string(),
        format!("title: {tag_id}"),
        format!("tag: {tag_id}"),
        format!("source: {source_rel}"),
        format!("entry: {entry_name}"),
        format!("value_count: {value_count}"),
    ];
    for value in value_terms {
        lines.push(format!("value: {value}"));
    }
    Some(WikiSourceDocument::structured(
        format!("Tag: {tag_id}"),
        "generated:tag".to_string(),
        format!("generated://tag/{entry_name}#{}", stable_hex(uri)),
        lines.join("\n"),
        "tag",
        structured,
    ))
}

fn tag_value(value: Value, labels: &HashMap<String, String>) -> Option<Value> {
    if let Some(id) = value.as_str() {
        if let Some(tag) = id.strip_prefix('#') {
            return Some(serde_json::json!({
                "kind": "tag",
                "id": format!("#{tag}"),
                "label": format!("#{tag}"),
            }));
        }
        return Some(serde_json::json!({
            "kind": "item",
            "id": id,
            "label": item_label(id.trim_start_matches('#'), labels),
        }));
    }
    let object = value.as_object()?;
    let id = object.get("id").and_then(|value| value.as_str())?;
    if let Some(tag) = id.strip_prefix('#') {
        return Some(serde_json::json!({
            "kind": "tag",
            "id": format!("#{tag}"),
            "required": object.get("required").and_then(|value| value.as_bool()).unwrap_or(true),
            "label": format!("#{tag}"),
        }));
    }
    Some(serde_json::json!({
        "kind": "item",
        "id": id,
        "required": object.get("required").and_then(|value| value.as_bool()).unwrap_or(true),
        "label": item_label(id.trim_start_matches('#'), labels),
    }))
}

fn patchouli_document_from_json(
    uri: &str,
    source_rel: &str,
    entry_name: &str,
    content: &str,
) -> Option<WikiSourceDocument> {
    let value: Value = serde_json::from_str(content).ok()?;
    let title = value
        .get("name")
        .or_else(|| value.get("title"))
        .and_then(|value| value.as_str())
        .unwrap_or(entry_name)
        .to_string();
    let mut text_lines = Vec::new();
    collect_patchouli_text(&value, &mut text_lines);
    let structured = serde_json::json!({
        "kind": "patchouli_page",
        "title": title,
        "text": text_lines,
        "source": {
            "origin": "local",
            "type": source_type_for_uri(uri),
            "uri": uri,
            "file": source_rel,
        },
    });
    let mut lines = vec![
        "kind: patchouli_page".to_string(),
        format!("title: {title}"),
        format!("source: {source_rel}"),
        format!("entry: {entry_name}"),
    ];
    lines.extend(
        text_lines
            .iter()
            .filter(|line| !line.trim().is_empty())
            .map(|line| format!("text: {line}")),
    );
    Some(WikiSourceDocument::structured(
        format!("Patchouli: {title}"),
        "generated:patchouli".to_string(),
        format!("generated://patchouli/{entry_name}#{}", stable_hex(uri)),
        lines.join("\n"),
        "patchouli_page",
        structured,
    ))
}

fn collect_patchouli_text(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            if !text.trim().is_empty() {
                out.push(text.clone());
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_patchouli_text(item, out);
            }
        }
        Value::Object(map) => {
            for key in ["name", "title", "text"] {
                if let Some(value) = map.get(key) {
                    collect_patchouli_text(value, out);
                }
            }
            if let Some(pages) = map.get("pages") {
                collect_patchouli_text(pages, out);
            }
        }
        _ => {}
    }
}

fn is_lang_entry(name: &str) -> bool {
    let parts = name.split('/').collect::<Vec<_>>();
    parts.len() == 4 && parts[0] == "assets" && parts[2] == "lang" && parts[3].ends_with(".json")
}

fn is_recipe_entry(name: &str) -> bool {
    let parts = name.split('/').collect::<Vec<_>>();
    parts.len() >= 4
        && parts[0] == "data"
        && matches!(parts[2], "recipe" | "recipes")
        && parts
            .last()
            .map(|part| part.ends_with(".json"))
            .unwrap_or(false)
}

fn is_tag_entry(name: &str) -> bool {
    let parts = name.split('/').collect::<Vec<_>>();
    parts.len() >= 5
        && parts[0] == "data"
        && parts[2] == "tags"
        && parts
            .last()
            .map(|part| part.ends_with(".json"))
            .unwrap_or(false)
}

fn is_patchouli_entry(name: &str) -> bool {
    let parts = name.split('/').collect::<Vec<_>>();
    parts.len() >= 5
        && parts[0] == "data"
        && parts[2] == "patchouli_books"
        && parts
            .last()
            .map(|part| part.ends_with(".json"))
            .unwrap_or(false)
}

fn tag_id_from_entry_name(entry_name: &str) -> Option<String> {
    let parts = entry_name.split('/').collect::<Vec<_>>();
    if parts.len() < 5 || parts[0] != "data" || parts[2] != "tags" {
        return None;
    }
    let namespace = parts[1];
    let path = parts[4..].join("/");
    let path = path.strip_suffix(".json").unwrap_or(&path);
    Some(format!("#{namespace}:{path}"))
}

fn source_type_for_uri(uri: &str) -> &'static str {
    if uri.contains(".jar!") {
        "mod_jar"
    } else if uri.contains("/kubejs/") || uri.starts_with("kubejs/") {
        "kubejs"
    } else if uri.contains("/datapacks/") || uri.starts_with("datapacks/") {
        "datapack"
    } else {
        "local"
    }
}

fn item_label(id: &str, labels: &HashMap<String, String>) -> String {
    labels
        .get(id)
        .cloned()
        .unwrap_or_else(|| pretty_item_id(id))
}

fn pretty_item_id(id: &str) -> String {
    let path = id
        .trim_start_matches('#')
        .split_once(':')
        .map(|(_, path)| path)
        .unwrap_or(id);
    path.split(['_', '/', '.'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn collect_ftb_quest_files(dir: &Path, files: &mut Vec<PathBuf>) -> CoreResult<()> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in read.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            collect_ftb_quest_files(&path, files)?;
        } else if kind.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn is_allowed_ftb_quest_file(path: &Path) -> CoreResult<bool> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(_) => return Ok(false),
    };
    if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > FTB_QUESTS_FILE_MAX_BYTES {
        return Ok(false);
    }
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return Ok(false);
    };
    Ok(matches!(
        ext.to_ascii_lowercase().as_str(),
        "snbt" | "json" | "json5" | "txt" | "md"
    ))
}

fn collect_instance_data_entries(root: &Path, rel: &str) -> CoreResult<Vec<String>> {
    let dir = root.join(rel);
    if !regular_dir(&dir) {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    collect_instance_data_entries_inner(root, &dir, &mut entries)?;
    entries.sort();
    entries.truncate(INSTANCE_DATA_MAX_ENTRIES);
    Ok(entries)
}

fn collect_instance_data_entries_inner(
    root: &Path,
    dir: &Path,
    entries: &mut Vec<String>,
) -> CoreResult<()> {
    if entries.len() >= INSTANCE_DATA_MAX_ENTRIES {
        return Ok(());
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in read.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            collect_instance_data_entries_inner(root, &path, entries)?;
            continue;
        }
        if kind.is_file() {
            entries.push(relative_slash_path(root, &path));
        }
        if entries.len() >= INSTANCE_DATA_MAX_ENTRIES {
            break;
        }
    }
    Ok(())
}

fn relative_slash_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn collect_wiki_files(path: &Path, files: &mut Vec<PathBuf>) -> CoreResult<()> {
    collect_wiki_files_inner(path, path, files)
}

fn collect_wiki_files_inner(root: &Path, path: &Path, files: &mut Vec<PathBuf>) -> CoreResult<()> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if path == root => {
            return Err(CoreError::other(format!(
                "wiki source path does not exist: {} ({err})",
                path.display()
            )));
        }
        Err(_) => return Ok(()),
    };
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_file() {
        if is_wiki_corpus_cache_file(path) {
            return Ok(());
        }
        if is_allowed_wiki_file(path)? {
            files.push(path.to_path_buf());
        }
        return Ok(());
    }
    if !meta.is_dir() || !should_descend_wiki_dir(root, path) {
        return Ok(());
    }
    let read = match std::fs::read_dir(path) {
        Ok(read) => read,
        Err(err) if path == root => return Err(err).with_path(path),
        Err(_) => return Ok(()),
    };
    for entry in read.flatten() {
        collect_wiki_files_inner(root, &entry.path(), files)?;
    }
    Ok(())
}

fn read_archive_wiki_texts(path: &Path) -> CoreResult<Vec<WikiSourceDocument>> {
    if is_symlink(path) {
        return Ok(Vec::new());
    }
    let file = std::fs::File::open(path).with_path(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|err| CoreError::Zip(err.to_string()))?;
    let mut docs = Vec::new();
    let mut total = 0usize;
    for index in 0..archive.len().min(WIKI_ARCHIVE_MAX_ENTRIES) {
        let Ok(mut file) = archive.by_index(index) else {
            continue;
        };
        if file.is_dir() {
            continue;
        }
        let name = file.name().to_string();
        if !is_allowed_wiki_archive_entry(&name) {
            continue;
        }
        let mut bytes = Vec::new();
        let read = (&mut file)
            .take(WIKI_FILE_MAX_BYTES + 1)
            .read_to_end(&mut bytes);
        if read.is_err() || bytes.len() as u64 > WIKI_FILE_MAX_BYTES {
            continue;
        }
        if total.saturating_add(bytes.len()) > WIKI_ARCHIVE_MAX_BYTES {
            break;
        }
        total += bytes.len();
        let Ok(content) = String::from_utf8(bytes) else {
            continue;
        };
        docs.push(document_from_parts(
            format!("{}!{}", path.display(), name),
            content,
        ));
    }
    docs.sort_by(|a, b| a.uri.cmp(&b.uri));
    Ok(docs)
}

fn document_from_parts(uri: String, content: String) -> WikiSourceDocument {
    WikiSourceDocument::text(uri.clone(), uri.clone(), uri, content)
}

fn read_text_file_bounded(path: &Path, max_bytes: u64) -> Option<String> {
    if is_symlink(path) {
        return None;
    }
    let mut file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > max_bytes {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn ftb_quest_documents_from_content(rel: &str, content: &str) -> Vec<WikiSourceDocument> {
    let mut docs = Vec::new();
    let mut seen = HashSet::new();
    let chapter_title = first_snbt_string_value(content, "title");
    for block in snbt_object_blocks(content) {
        if !looks_like_quest_block(&block) {
            continue;
        }
        let Some(title) = first_snbt_string_value(&block, "title") else {
            continue;
        };
        let marker = stable_hex(&block);
        if !seen.insert(marker.clone()) {
            continue;
        }

        let mut lines = vec![
            format!("FTB Quests source file: {rel}"),
            format!("Quest title: {title}"),
        ];
        if let Some(chapter) = chapter_title.as_deref().filter(|chapter| *chapter != title) {
            lines.push(format!("Chapter title: {chapter}"));
            lines.push(format!("title: \"{chapter}\""));
        }
        for value in snbt_string_values_for_key(&block, "subtitle") {
            lines.push(format!("Quest subtitle: {value}"));
        }
        for value in snbt_string_values_for_key(&block, "description") {
            lines.push(format!("Quest description: {value}"));
        }

        let mut tokens = symbol_tokens(&block);
        tokens.sort();
        tokens.dedup();
        for token in tokens {
            lines.push(format!("Quest token: {token}"));
        }

        lines.push(String::new());
        lines.push("Raw quest source:".to_string());
        lines.push(block.clone());

        docs.push(WikiSourceDocument::structured(
            format!("FTB Quest: {title} ({rel})"),
            "generated:ftb-quests".to_string(),
            format!("generated://ftb-quests/{rel}#quest-{marker}"),
            lines.join("\n"),
            "quest",
            serde_json::json!({
                "kind": "quest",
                "title": title,
                "chapter": chapter_title.clone(),
                "source": {
                    "origin": "local",
                    "type": "ftbquests",
                    "uri": rel,
                },
                "raw": block,
            }),
        ));
    }
    docs
}

fn snbt_object_blocks(content: &str) -> Vec<String> {
    let mut stack = Vec::new();
    let mut blocks = Vec::new();
    let mut in_string = false;
    let mut escaped = false;

    for (idx, ch) in content.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => stack.push(idx),
            '}' => {
                if let Some(start) = stack.pop() {
                    blocks.push(content[start..idx + ch.len_utf8()].to_string());
                }
            }
            _ => {}
        }
    }
    blocks
}

fn looks_like_quest_block(block: &str) -> bool {
    let lower = block.to_ascii_lowercase();
    lower.contains("title:")
        && !lower.contains("quests:")
        && (lower.contains("tasks:")
            || lower.contains("rewards:")
            || lower.contains("description:")
            || lower.contains("item:"))
}

fn first_snbt_string_value(text: &str, key: &str) -> Option<String> {
    snbt_string_values_for_key(text, key).into_iter().next()
}

fn snbt_string_values_for_key(text: &str, key: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let needle = format!("{}:", key.to_ascii_lowercase());
    let mut values = Vec::new();
    let mut offset = 0usize;
    while let Some(pos) = lower[offset..].find(&needle) {
        let start = offset + pos + needle.len();
        let end = value_scan_end(text, start);
        values.extend(quoted_strings(&text[start..end]));
        offset = start;
    }
    values
}

fn value_scan_end(text: &str, start: usize) -> usize {
    let mut end = start;
    let mut in_string = false;
    let mut escaped = false;
    let mut square = 0i32;
    let mut brace = 0i32;
    for (rel, ch) in text[start..].char_indices() {
        end = start + rel + ch.len_utf8();
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '[' => square += 1,
            ']' => square -= 1,
            '{' => brace += 1,
            '}' => {
                if brace == 0 && square == 0 {
                    return start + rel;
                }
                brace -= 1;
            }
            '\n' if square <= 0 && brace <= 0 => return start + rel,
            _ => {}
        }
    }
    end
}

fn quoted_strings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut in_string = false;
    let mut escaped = false;
    for ch in text.chars() {
        if in_string {
            if escaped {
                buf.push(ch);
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => {
                    if !buf.trim().is_empty() {
                        out.push(std::mem::take(&mut buf));
                    }
                    in_string = false;
                }
                _ => buf.push(ch),
            }
        } else if ch == '"' {
            in_string = true;
            buf.clear();
        }
    }
    out
}

fn chunks_from_document(doc: &WikiSourceDocument) -> Vec<WikiChunk> {
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    let mut current_bytes = 0usize;
    let mut start_line = 1usize;
    let mut line_no = 1usize;
    for line in doc.content.lines() {
        for segment in split_line_by_bytes(line, WIKI_CHUNK_MAX_BYTES) {
            let segment_bytes = segment.len() + 1;
            if !current.is_empty()
                && (current.len() >= WIKI_CHUNK_MAX_LINES
                    || current_bytes.saturating_add(segment_bytes) > WIKI_CHUNK_MAX_BYTES)
            {
                push_chunk(
                    &mut chunks,
                    doc,
                    start_line,
                    line_no.saturating_sub(1),
                    &current,
                );
                current.clear();
                current_bytes = 0;
                start_line = line_no;
            }
            current_bytes += segment_bytes;
            current.push(segment);
        }
        line_no += 1;
    }
    if current.is_empty() {
        chunks.push(chunk_from_content(0, doc, 1, 1, ""));
    } else {
        push_chunk(
            &mut chunks,
            doc,
            start_line,
            line_no.saturating_sub(1),
            &current,
        );
    }
    chunks
}

fn split_line_by_bytes(line: &str, max: usize) -> Vec<String> {
    if line.len() <= max {
        return vec![line.to_string()];
    }
    let mut out = Vec::new();
    let mut buf = String::new();
    for ch in line.chars() {
        if !buf.is_empty() && buf.len() + ch.len_utf8() > max {
            out.push(std::mem::take(&mut buf));
        }
        buf.push(ch);
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

fn push_chunk(
    chunks: &mut Vec<WikiChunk>,
    doc: &WikiSourceDocument,
    start_line: usize,
    end_line: usize,
    lines: &[String],
) {
    let content = lines.join("\n");
    chunks.push(chunk_from_content(
        chunks.len(),
        doc,
        start_line,
        end_line,
        &content,
    ));
}

fn chunk_from_content(
    chunk_index: usize,
    doc: &WikiSourceDocument,
    start_line: usize,
    end_line: usize,
    content: &str,
) -> WikiChunk {
    let doc_hash = stable_hex(&doc.uri);
    let content_hash = stable_hex(content);
    WikiChunk {
        chunk_id: format!("chunk:{doc_hash}:{chunk_index}:{content_hash}"),
        document_id: format!("doc:{doc_hash}"),
        title: doc.title.clone(),
        source_label: doc.source_label.clone(),
        location: format!("lines {start_line}-{end_line}"),
        content: content.to_string(),
        kind: doc.kind.clone(),
        structured: doc.structured.clone(),
    }
}

fn stable_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())[..16].to_string()
}

#[derive(Debug, Clone)]
struct SearchQuery {
    terms: Vec<String>,
    special_terms: Vec<String>,
    normalized_phrase: String,
    snippet_terms: Vec<String>,
}

impl SearchQuery {
    fn parse(input: &str) -> Self {
        let mut terms = search_terms(input);
        let mut special_terms = symbol_tokens(input);
        for special in &special_terms {
            terms.extend(search_terms(special));
        }
        terms.sort();
        terms.dedup();
        special_terms.sort();
        special_terms.dedup();
        let mut snippet_terms = terms.clone();
        snippet_terms.extend(special_terms.iter().cloned());
        snippet_terms.sort();
        snippet_terms.dedup();
        Self {
            terms,
            special_terms,
            normalized_phrase: normalize_search_text(input),
            snippet_terms,
        }
    }

    fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.special_terms.is_empty()
    }
}

fn score_chunk(chunk: &WikiChunk, query: &SearchQuery) -> f32 {
    let title = SearchText::new(&chunk.title);
    let source = SearchText::new(&format!("{} {}", chunk.source_label, chunk.location));
    let content = SearchText::new(&chunk.content);

    let mut score = 0.0_f32;
    if query.normalized_phrase.len() >= 4 {
        if title.normalized.contains(&query.normalized_phrase) {
            score += 14.0;
        }
        if content.normalized.contains(&query.normalized_phrase) {
            score += 8.0;
        }
    }

    for special in &query.special_terms {
        let special = special.as_str();
        if title.lower.contains(special) {
            score += 9.0;
        }
        if content.lower.contains(special) {
            score += 7.0;
        }
        if source.lower.contains(special) {
            score += 4.0;
        }
    }

    for term in &query.terms {
        score += title.term_score(term, 5.0, 2.5);
        score += source.term_score(term, 2.0, 1.0);
        score += content.term_score(term, 1.4, 0.9);
    }

    if score <= 0.0 {
        for term in &query.terms {
            score += title.fuzzy_score(term, 3.0);
            score += source.fuzzy_score(term, 1.0);
            score += content.fuzzy_score(term, 0.7);
        }
    }

    if score > 0.0 {
        score *= source_weight(&chunk.source_label);
    }
    score
}

fn normalize_filter_text(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_ascii_lowercase())
}

fn chunk_matches_kind(chunk: &WikiChunk, kind: Option<&str>) -> bool {
    let Some(kind) = kind else {
        return true;
    };
    chunk
        .kind
        .as_deref()
        .map(|chunk_kind| chunk_kind.eq_ignore_ascii_case(kind))
        .unwrap_or(false)
}

fn chunk_matches_target(chunk: &WikiChunk, target_id: Option<&str>) -> bool {
    let Some(target_id) = target_id else {
        return true;
    };
    let Some(structured) = chunk.structured.as_ref() else {
        return false;
    };
    structured_target_ids(structured)
        .into_iter()
        .any(|id| id.eq_ignore_ascii_case(target_id))
}

fn chunk_matches_ingredient(chunk: &WikiChunk, ingredient_id: Option<&str>) -> bool {
    let Some(ingredient_id) = ingredient_id else {
        return true;
    };
    let Some(structured) = chunk.structured.as_ref() else {
        return false;
    };
    structured_ingredient_ids(structured)
        .into_iter()
        .any(|id| id.eq_ignore_ascii_case(ingredient_id))
}

fn structured_filter_score(
    chunk: &WikiChunk,
    target_id: Option<&str>,
    ingredient_id: Option<&str>,
) -> f32 {
    let mut score = 0.0;
    if target_id.is_some() && chunk_matches_target(chunk, target_id) {
        score += 40.0;
    }
    if ingredient_id.is_some() && chunk_matches_ingredient(chunk, ingredient_id) {
        score += 20.0;
    }
    score
}

fn source_priority_score(chunk: &WikiChunk) -> f32 {
    match chunk.kind.as_deref() {
        Some("recipe") | Some("recipe_override") => match structured_source_type(&chunk.structured)
        {
            Some("kubejs") => 30.0,
            Some("datapack") => 24.0,
            Some("local") => 18.0,
            Some("mod_jar") => 8.0,
            _ => 0.0,
        },
        _ => 0.0,
    }
}

fn structured_source_type(structured: &Option<Value>) -> Option<&str> {
    structured
        .as_ref()?
        .pointer("/source/type")
        .and_then(|value| value.as_str())
}

fn structured_target_ids(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(id) = value.pointer("/result/id").and_then(|value| value.as_str()) {
        out.push(id.to_string());
    }
    if let Some(id) = value.pointer("/target/id").and_then(|value| value.as_str()) {
        out.push(id.to_string());
    }
    if let Some(id) = value.pointer("/target_id").and_then(|value| value.as_str()) {
        out.push(id.to_string());
    }
    if let Some(id) = value.pointer("/id").and_then(|value| value.as_str()) {
        if value
            .get("kind")
            .and_then(|kind| kind.as_str())
            .map(|kind| matches!(kind, "tag" | "recipe_override"))
            .unwrap_or(false)
        {
            out.push(id.to_string());
        }
    }
    collect_result_ids(value.get("results"), &mut out);
    out.sort();
    out.dedup();
    out
}

fn collect_result_ids(value: Option<&Value>, out: &mut Vec<String>) {
    match value {
        Some(Value::Array(items)) => {
            for item in items {
                collect_result_ids(Some(item), out);
            }
        }
        Some(Value::Object(map)) => {
            for key in ["id", "item"] {
                if let Some(id) = map.get(key).and_then(|value| value.as_str()) {
                    out.push(id.to_string());
                }
            }
            if let Some(item) = map.get("result").or_else(|| map.get("output")) {
                collect_result_ids(Some(item), out);
            }
        }
        Some(Value::String(id)) => out.push(id.to_string()),
        _ => {}
    }
}

fn structured_ingredient_ids(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    collect_structured_ids(value.get("ingredients"), &mut out);
    collect_structured_ids(value.get("grid"), &mut out);
    collect_structured_ids(value.get("input"), &mut out);
    collect_structured_ids(value.get("replacement"), &mut out);
    out.sort();
    out.dedup();
    out
}

fn collect_structured_ids(value: Option<&Value>, out: &mut Vec<String>) {
    match value {
        Some(Value::Array(items)) => {
            for item in items {
                collect_structured_ids(Some(item), out);
            }
        }
        Some(Value::Object(map)) => {
            if let Some(id) = map.get("id").and_then(|value| value.as_str()) {
                out.push(id.to_string());
            }
            if let Some(item) = map.get("item").and_then(|value| value.as_str()) {
                out.push(item.to_string());
            }
            if let Some(tag) = map.get("tag").and_then(|value| value.as_str()) {
                out.push(format!("#{tag}"));
            }
            for value in map.values() {
                collect_structured_ids(Some(value), out);
            }
        }
        Some(Value::String(id)) => out.push(id.to_string()),
        _ => {}
    }
}

#[derive(Debug)]
struct SearchText {
    lower: String,
    normalized: String,
    tokens: Vec<String>,
    counts: HashMap<String, usize>,
}

impl SearchText {
    fn new(text: &str) -> Self {
        let lower = text.to_ascii_lowercase();
        let normalized = normalize_search_text(text);
        let mut tokens = search_terms(text);
        tokens.extend(
            symbol_tokens(text)
                .into_iter()
                .flat_map(|token| search_terms(&token)),
        );
        tokens.sort();
        let mut counts = HashMap::new();
        for token in &tokens {
            *counts.entry(token.clone()).or_insert(0) += 1;
        }
        tokens.dedup();
        Self {
            lower,
            normalized,
            tokens,
            counts,
        }
    }

    fn term_score(&self, term: &str, exact_weight: f32, fuzzy_weight: f32) -> f32 {
        if let Some(count) = self.counts.get(term) {
            return exact_weight * (*count).min(4) as f32;
        }
        self.fuzzy_score(term, fuzzy_weight)
    }

    fn fuzzy_score(&self, term: &str, weight: f32) -> f32 {
        if term.len() < 3 {
            return 0.0;
        }
        let best = self
            .tokens
            .iter()
            .map(|candidate| fuzzy_similarity(term, candidate))
            .fold(0.0_f32, f32::max);
        if best > 0.66 {
            weight * (1.0 + (best - 0.66) * 2.0)
        } else {
            0.0
        }
    }
}

fn source_weight(source_label: &str) -> f32 {
    let lower = source_label.to_ascii_lowercase();
    if lower == "generated:recipe" {
        1.55
    } else if lower == "generated:recipe-override" {
        1.6
    } else if lower == "generated:ftb-quests" {
        1.35
    } else if lower == "generated:patchouli" {
        1.3
    } else if lower == "generated:tag" {
        1.2
    } else if lower == "generated:project-doc" {
        1.05
    } else if lower == "generated:instance-data" {
        0.75
    } else if lower.contains("kubejs") || lower.contains("scripts") {
        1.15
    } else {
        1.0
    }
}

fn is_allowed_wiki_file(path: &Path) -> CoreResult<bool> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(_) => return Ok(false),
    };
    if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > WIKI_FILE_MAX_BYTES {
        return Ok(false);
    }
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return Ok(false);
    };
    Ok(allowed_wiki_extension(ext))
}

fn is_allowed_wiki_archive_entry(name: &str) -> bool {
    if should_skip_virtual_path(name) {
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

fn should_descend_wiki_dir(root: &Path, path: &Path) -> bool {
    if path == root {
        return true;
    }
    let rel = relative_slash_path(root, path);
    if rel
        .split('/')
        .any(|segment| should_skip_path_segment(&segment.to_ascii_lowercase()))
    {
        return false;
    }
    WIKI_INDEX_DIRS.iter().any(|dir| {
        rel == *dir || rel.starts_with(&format!("{dir}/")) || dir.starts_with(&format!("{rel}/"))
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
        "mods"
            | "resourcepacks"
            | "shaderpacks"
            | ".git"
            | "versions"
            | "logs"
            | "saves"
            | "crash-reports"
            | "screenshots"
            | "backups"
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

fn is_wiki_corpus_cache_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case(WIKI_CORPUS_CACHE_FILE))
        .unwrap_or(false)
}

fn regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|meta| !meta.file_type().is_symlink() && meta.is_file())
        .unwrap_or(false)
}

fn regular_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|meta| !meta.file_type().is_symlink() && meta.is_dir())
        .unwrap_or(false)
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
}

fn search_terms(query: &str) -> Vec<String> {
    let mut terms = query
        .split(|ch: char| !ch.is_alphanumeric())
        .map(|term| term.trim().to_ascii_lowercase())
        .filter(|term| term.len() >= 2)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    terms.sort();
    terms
}

fn symbol_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut buf = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || matches!(ch, ':' | '_' | '-' | '.' | '/') {
            buf.push(ch.to_ascii_lowercase());
        } else {
            push_symbol_token(&mut tokens, &mut buf);
        }
    }
    push_symbol_token(&mut tokens, &mut buf);
    tokens
}

fn push_symbol_token(tokens: &mut Vec<String>, buf: &mut String) {
    if buf.len() >= 3
        && buf
            .chars()
            .any(|ch| matches!(ch, ':' | '_' | '-' | '.' | '/'))
        && buf.chars().any(|ch| ch.is_ascii_alphabetic())
    {
        tokens.push(std::mem::take(buf));
    } else {
        buf.clear();
    }
}

fn normalize_search_text(text: &str) -> String {
    let mut out = String::new();
    let mut last_space = true;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim().to_string()
}

fn fuzzy_similarity(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    if a.len() < 3 || b.len() < 3 {
        return 0.0;
    }
    if is_subsequence(a, b) || is_subsequence(b, a) {
        return 0.82;
    }
    let a_set = trigrams(a);
    let b_set = trigrams(b);
    if a_set.is_empty() || b_set.is_empty() {
        return 0.0;
    }
    let intersection = a_set.intersection(&b_set).count() as f32;
    let union = a_set.union(&b_set).count() as f32;
    intersection / union
}

fn trigrams(input: &str) -> HashSet<String> {
    let chars = input.chars().collect::<Vec<_>>();
    if chars.len() < 3 {
        return HashSet::new();
    }
    chars
        .windows(3)
        .map(|w| w.iter().collect::<String>())
        .collect()
}

fn is_subsequence(short: &str, long: &str) -> bool {
    if short.len() > long.len() {
        return false;
    }
    let mut chars = short.chars();
    let mut next = chars.next();
    for ch in long.chars() {
        if Some(ch) == next {
            next = chars.next();
            if next.is_none() {
                return true;
            }
        }
    }
    next.is_none()
}

fn snippet_for_terms(content: &str, terms: &[String]) -> String {
    let lower = content.to_ascii_lowercase();
    let start = terms
        .iter()
        .filter_map(|term| lower.find(term))
        .min()
        .unwrap_or(0);
    let prefix_chars = content[..start.min(content.len())].chars().count();
    content
        .chars()
        .skip(prefix_chars.saturating_sub(80))
        .take(260)
        .collect::<String>()
        .trim()
        .replace('\n', " ")
}
