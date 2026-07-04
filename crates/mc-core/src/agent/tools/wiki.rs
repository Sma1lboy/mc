//! `wiki_search` / `wiki_open` — source-backed local modpack wiki tools.
//!
//! The model may pass only a query or chunk id. The desktop host injects the
//! local source paths, and this module owns the trust boundary: bounded file
//! reads, symlink skipping, stable chunk ids, and cache fingerprinting.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::ChatToolError;
use crate::error::{CoreError, IoResultExt, Result as CoreResult};
use crate::instance::InstanceConfig;
use crate::version::pack::PackProfile;

const WIKI_FILE_MAX_BYTES: u64 = 256 * 1024;
const WIKI_CORPUS_MAX_BYTES: usize = 3 * 1024 * 1024;
const WIKI_CORPUS_MAX_DOCUMENTS: usize = 800;
const WIKI_ARCHIVE_MAX_BYTES: usize = 512 * 1024;
const WIKI_ARCHIVE_MAX_ENTRIES: usize = 128;
const WIKI_SEARCH_DEFAULT_TOP_K: usize = 5;
const WIKI_SEARCH_MAX_TOP_K: usize = 8;
const WIKI_CHUNK_MAX_LINES: usize = 80;
const WIKI_CHUNK_MAX_BYTES: usize = 64 * 1024;
const WIKI_CORPUS_CACHE_VERSION: u32 = 2;
const WIKI_CORPUS_CACHE_FILE: &str = "wiki-corpus.json";
const INSTANCE_DATA_MAX_ENTRIES: usize = 200;

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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct WikiSearchHit {
    pub chunk_id: String,
    pub title: String,
    pub snippet: String,
    pub source_label: String,
    pub location: String,
    pub score: f32,
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
        let terms = search_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut hits = self
            .chunks
            .iter()
            .filter_map(|chunk| {
                let content = chunk.content.to_ascii_lowercase();
                let title = chunk.title.to_ascii_lowercase();
                let mut score = 0.0_f32;
                for term in &terms {
                    if content.contains(term) {
                        score += 2.0;
                    }
                    if title.contains(term) {
                        score += 1.0;
                    }
                }
                (score > 0.0).then(|| WikiSearchHit {
                    chunk_id: chunk.chunk_id.clone(),
                    title: chunk.title.clone(),
                    snippet: snippet_for_terms(&chunk.content, &terms),
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
    let corpus =
        corpus_from_tool_args(args.modpack_id, args.instance_id, args.source_paths).await?;
    let hits = corpus.search(&args.query, top_k).await?;
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
    tokio::task::spawn_blocking(move || {
        let documents = read_local_wiki_documents(&source_paths)?;
        WikiCorpus::from_documents(scope, source_count, documents)
    })
    .await
    .map_err(|err| CoreError::other(format!("wiki corpus build task failed: {err}")))?
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
    let bytes = serde_json::to_vec_pretty(&cache).map_err(|err| CoreError::Parse {
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
    Ok(Some(WikiSourceDocument {
        title: "Current modpack instance data".to_string(),
        source_label: "generated:instance-data".to_string(),
        uri: format!("generated://instance-data/{}", path.display()),
        content: lines.join("\n"),
    }))
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
        docs.push(WikiSourceDocument {
            title: format!("FTB Quests: {rel}"),
            source_label: "generated:ftb-quests".to_string(),
            uri: format!("generated://ftb-quests/{rel}"),
            content: format!("FTB Quests source file: {rel}\n\n{content}"),
        });
    }
    Ok(docs)
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
    WikiSourceDocument {
        title: uri.clone(),
        source_label: uri.clone(),
        uri,
        content,
    }
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
    }
}

fn stable_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())[..16].to_string()
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
