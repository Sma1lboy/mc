//! Unit tests for the deterministic modpack tools.
//!
//! Each `tool_*` runs against an in-memory `FakeChatProvider` — no live API key,
//! no network (the archive/build tests spin up throwaway localhost servers).

use std::collections::HashMap;

use crate::agent::compatibility::{
    CompatibilityIssue, CompatibilityReport, CompatibilityStatus, IssueSeverity,
};
use crate::modplatform::Dependency;
use crate::paths::GamePaths;

use super::fake_provider::{
    bytes_server, cdn_file, ctx_of, hit, registry_of, temp_dir, version, zip_index,
    FakeChatProvider,
};
use super::{
    apply_diagnostic_operations, create_diagnostic_snapshot, diagnose_instance_with_total_memory,
    prebuild_wiki_corpus_cache, refresh_wiki_corpus_cache, tool_build_modpack,
    tool_inspect_base_modpack, tool_mod_get_detail, tool_resolve_mods, tool_search_base_modpacks,
    tool_search_mods, tool_validate_modpack_plan, tool_wiki_open, tool_wiki_search,
    wiki_corpus_cache_path, BuildBasePack, BuildModRef, BuildModpackArgs, BuildTarget,
    ChatToolsCtx, DiagnoseInstanceArgs, DiagnosticTrialOperation, InspectBaseModpackArgs,
    LocalPathWikiSource, ModGetDetailArgs, ResolveModsArgs, SearchBaseModpacksArgs, SearchModsArgs,
    ValidateModpackPlanArgs, WikiCorpus, WikiOpenArgs, WikiScope, WikiSearchArgs,
};

// ---------------------------------------------------------------------------
// Tool tests
// ---------------------------------------------------------------------------

fn zip_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::{Cursor, Write};

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in files {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

fn rewrite_first_cached_wiki_chunk(dir: &std::path::Path, content: &str) {
    let cache_path = wiki_corpus_cache_path(dir);
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&cache_path).unwrap()).unwrap();
    value["chunks"][0]["title"] = serde_json::Value::String("Cached wiki chunk".to_string());
    value["chunks"][0]["content"] = serde_json::Value::String(content.to_string());
    std::fs::write(&cache_path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

fn write_cached_wiki_project_detail(
    dir: &std::path::Path,
    provider: &str,
    project_id: &str,
    title: &str,
    body: &str,
) {
    let safe: String = project_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let path = dir
        .join(".wiki-project-cache")
        .join(provider)
        .join("project")
        .join(format!("{safe}.json"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let cached = serde_json::json!({
        "fetched_at": 4_102_444_800u64,
        "data": {
            "id": project_id,
            "slug": project_id,
            "title": title,
            "description": "A cached project description.",
            "body": body,
            "downloads": 42,
            "followers": 7,
            "icon_url": null,
            "categories": ["technology"],
            "gallery": [],
            "source_url": "https://github.com/example/pack",
            "issues_url": null,
            "wiki_url": "https://example.com/wiki",
            "discord_url": null
        }
    });
    std::fs::write(path, serde_json::to_vec_pretty(&cached).unwrap()).unwrap();
}

mod core;
mod diagnostics;
mod wiki_cache;
mod wiki_sources;
mod wiki_structured;
