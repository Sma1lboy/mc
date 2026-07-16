use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::Result as CoreResult;
use crate::instance::InstanceConfig;
use crate::modplatform::modrinth::{ModrinthApi, ProjectDetail};

use super::chunk::stable_hex;
use super::sources::{regular_dir, regular_file};
use super::WikiSourceDocument;

const WIKI_PROJECT_CACHE_DIR: &str = ".wiki-project-cache";
const WIKI_PROJECT_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 3600);
const WIKI_PROJECT_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4);
const WIKI_PROJECT_DESCRIPTION_MAX_BYTES: usize = 4 * 1024;
const WIKI_PROJECT_BODY_MAX_BYTES: usize = 32 * 1024;

pub(super) async fn read_project_wiki_documents(
    source_paths: &[PathBuf],
) -> Vec<WikiSourceDocument> {
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
    let description = bounded_text(&detail.description, WIKI_PROJECT_DESCRIPTION_MAX_BYTES);
    let body = bounded_text(&detail.body, WIKI_PROJECT_BODY_MAX_BYTES);
    let source_uri = format!("provider://{provider}/project/{}", stable_hex(project_id));
    let structured = serde_json::json!({
        "kind": "project_doc",
        "provider": provider,
        "project_id": project_id,
        "title": detail.title,
        "slug": detail.slug,
        "description": description,
        "body": body,
        "body_truncated": detail.body.len() > WIKI_PROJECT_BODY_MAX_BYTES,
        "categories": detail.categories,
        "source": {
            "origin": "provider",
            "type": provider,
            "uri": source_uri,
        },
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

fn bounded_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[TRUNCATED]", &text[..end])
}
