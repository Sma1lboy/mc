//! Source-backed wiki search/open tools for the TS-side agent brain.
//!
//! Tool callers should depend on [`WikiCorpus`] search/open behavior. Concrete
//! sources, such as local paths or future remote wiki adapters, only implement
//! [`WikiSource`] and feed documents into the same corpus shape.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};

use super::ChatToolError;
use crate::error::{CoreError, IoResultExt, Result as CoreResult};
use crate::instance::InstanceConfig;
use crate::version::pack::PackProfile;

const WIKI_FILE_MAX_BYTES: u64 = 256 * 1024;
const WIKI_SEARCH_DEFAULT_TOP_K: usize = 5;
const WIKI_SEARCH_MAX_TOP_K: usize = 8;
const WIKI_CHUNK_MAX_LINES: usize = 80;
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
        Box::pin(async move { read_local_wiki_documents(&self.paths) })
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
        for source in sources {
            documents.extend(source.load_documents().await?);
        }
        documents.sort_by(|a, b| a.uri.cmp(&b.uri));
        let chunks = documents
            .into_iter()
            .enumerate()
            .flat_map(|(doc_index, doc)| chunks_from_document(doc_index, &doc))
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
    /// Same source list used for `wiki_search`; `chunk_id` is stable within it.
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
    let sources = source_paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .map(|path| {
            Box::new(LocalPathWikiSource::new(vec![PathBuf::from(path)])) as Box<dyn WikiSource>
        })
        .collect::<Vec<_>>();
    WikiCorpus::from_sources(scope, sources).await
}

fn read_local_wiki_documents(paths: &[PathBuf]) -> CoreResult<Vec<WikiSourceDocument>> {
    let mut docs = Vec::new();
    for path in paths {
        if is_archive_path(path) {
            docs.extend(read_archive_wiki_texts(path)?);
        } else {
            if path.is_dir() {
                if let Some(doc) = generated_instance_data_document(path)? {
                    docs.push(doc);
                }
            }
            let mut files = Vec::new();
            collect_wiki_files(path, &mut files)?;
            files.sort();
            for file in files {
                if let Ok(content) = std::fs::read_to_string(&file) {
                    docs.push(document_from_parts(
                        file.to_string_lossy().to_string(),
                        content,
                    ));
                }
            }
        }
    }
    docs.sort_by(|a, b| a.uri.cmp(&b.uri));
    Ok(docs)
}

fn generated_instance_data_document(path: &Path) -> CoreResult<Option<WikiSourceDocument>> {
    let mut lines = vec![
        "Current modpack instance data".to_string(),
        format!("Instance directory: {}", path.display()),
    ];
    let mut has_data = false;

    let instance_config_path = path.join("instance.json");
    if instance_config_path.is_file() {
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

fn collect_instance_data_entries(root: &Path, rel: &str) -> CoreResult<Vec<String>> {
    let dir = root.join(rel);
    if !dir.is_dir() {
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
    for entry in std::fs::read_dir(dir).with_path(dir)? {
        let entry = entry.with_path(dir)?;
        let path = entry.path();
        if path.is_dir() {
            collect_instance_data_entries_inner(root, &path, entries)?;
            continue;
        }
        if path.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            entries.push(rel);
        }
        if entries.len() >= INSTANCE_DATA_MAX_ENTRIES {
            break;
        }
    }
    Ok(())
}

fn collect_wiki_files(path: &Path, files: &mut Vec<PathBuf>) -> CoreResult<()> {
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

fn read_archive_wiki_texts(path: &Path) -> CoreResult<Vec<WikiSourceDocument>> {
    let file = std::fs::File::open(path).with_path(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|err| CoreError::Zip(err.to_string()))?;
    let mut docs = Vec::new();
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
            docs.push(document_from_parts(
                format!("{}!{}", path.display(), name),
                content,
            ));
        }
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

fn chunks_from_document(doc_index: usize, doc: &WikiSourceDocument) -> Vec<WikiChunk> {
    let lines = doc.content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return vec![chunk_from_lines(doc_index, 0, doc, 1, 1, "")];
    }
    lines
        .chunks(WIKI_CHUNK_MAX_LINES)
        .enumerate()
        .map(|(chunk_index, chunk_lines)| {
            let start = chunk_index * WIKI_CHUNK_MAX_LINES + 1;
            let end = start + chunk_lines.len() - 1;
            chunk_from_lines(
                doc_index,
                chunk_index,
                doc,
                start,
                end,
                &chunk_lines.join("\n"),
            )
        })
        .collect()
}

fn chunk_from_lines(
    doc_index: usize,
    chunk_index: usize,
    doc: &WikiSourceDocument,
    start_line: usize,
    end_line: usize,
    content: &str,
) -> WikiChunk {
    WikiChunk {
        chunk_id: format!("chunk:{doc_index}:{chunk_index}"),
        document_id: format!("doc:{doc_index}"),
        title: doc.title.clone(),
        source_label: doc.source_label.clone(),
        location: format!("lines {start_line}-{end_line}"),
        content: content.to_string(),
    }
}

fn is_allowed_wiki_file(path: &Path) -> CoreResult<bool> {
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
