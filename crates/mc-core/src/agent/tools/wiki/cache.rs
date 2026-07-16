use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{CoreError, Result as CoreResult};

use super::sources::{
    collect_wiki_files, is_wiki_corpus_cache_file, regular_dir, regular_file, relative_slash_path,
    INSTANCE_DATA_DIRS,
};
use super::{build_wiki_corpus_from_paths, privacy, WikiChunk, WikiCorpus, WikiScope};

const WIKI_CORPUS_CACHE_VERSION: u32 = 8;
pub(super) const WIKI_CORPUS_CACHE_FILE: &str = "wiki-corpus.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WikiCorpusCache {
    version: u32,
    corpus_id: String,
    fingerprint: String,
    source_count: usize,
    chunks: Vec<WikiChunk>,
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

pub(super) async fn corpus_from_cache_or_rebuild(
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
    let mut chunks = cache.chunks;
    if privacy::finalize_wiki_chunks(&mut chunks, &[instance_dir.to_path_buf()]) {
        tracing::warn!(path = %cache_path.display(), "discarding unsafe wiki corpus cache");
        return Ok(None);
    }
    Ok(Some(WikiCorpus {
        scope: scope.clone(),
        source_count: cache.source_count,
        chunks,
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
