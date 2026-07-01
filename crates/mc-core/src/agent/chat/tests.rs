//! Unit tests for the lean chat agent.
//!
//! Two layers, neither of which needs a live API key:
//! - each deterministic tool against an in-memory `FakeChatProvider`
//!   (no network — the archive/build tests spin up throwaway localhost servers);
//! - the streaming loop (`run_chat_turn`) against a mocked OpenRouter SSE server,
//!   verifying that a streamed tool call is dispatched, its result fed back, and
//!   the events surface through the sink.

use std::collections::HashMap;
use std::sync::Arc;

use futures::future::BoxFuture;
use rig_core::tool::Tool;

use mc_types::AgentStreamEvent;

use crate::agent::{AgentLlmClient, AgentLlmConfig};
use crate::modplatform::provider::{ProviderRegistry, ResourceProvider};
use crate::modplatform::{
    Dependency, HashAlgo, ProjectSideSupport, ProjectVersion, ProviderCaps, ProviderId,
    ResolvedFile, SearchHit, SearchQuery, VersionFile,
};

use super::run::{run_chat_turn, ChatTranscript, CollectingSink};
use super::tools::{
    BuildModRef, BuildModpackArgs, BuildModpackTool, BuildTarget, ChatToolsCtx,
    InspectBaseModpackArgs, InspectBaseModpackTool, ResolveModsArgs, ResolveModsTool,
    SearchBaseModpacksArgs, SearchBaseModpacksTool, SearchModsArgs, SearchModsTool,
};

// ---------------------------------------------------------------------------
// Fake provider
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct FakeChatProvider {
    search_hits: Vec<SearchHit>,
    versions: HashMap<String, Vec<ProjectVersion>>,
    projects: HashMap<String, SearchHit>,
}

impl FakeChatProvider {
    fn find_version(&self, version_id: &str) -> Option<ProjectVersion> {
        self.versions
            .values()
            .flatten()
            .find(|v| v.id == version_id)
            .cloned()
    }
}

impl ResourceProvider for FakeChatProvider {
    fn caps(&self) -> &ProviderCaps {
        static CAPS: ProviderCaps = ProviderCaps {
            id: ProviderId::Modrinth,
            readable_name: "Fake",
            hash_algos: &[],
            needs_api_key: false,
        };
        &CAPS
    }

    fn search<'a>(&'a self, _q: &'a SearchQuery) -> BoxFuture<'a, crate::error::Result<Vec<SearchHit>>> {
        let hits = self.search_hits.clone();
        Box::pin(async move { Ok(hits) })
    }

    fn get_project<'a>(&'a self, project_id: &'a str) -> BoxFuture<'a, crate::error::Result<SearchHit>> {
        let hit = self
            .projects
            .get(project_id)
            .cloned()
            .unwrap_or_else(|| hit(project_id, project_id, project_id));
        Box::pin(async move { Ok(hit) })
    }

    fn get_projects<'a>(
        &'a self,
        project_ids: &'a [String],
    ) -> BoxFuture<'a, crate::error::Result<Vec<SearchHit>>> {
        let hits = project_ids
            .iter()
            .map(|id| {
                self.projects
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| hit(id, id, id))
            })
            .collect();
        Box::pin(async move { Ok(hits) })
    }

    fn list_versions<'a>(
        &'a self,
        project_id: &'a str,
        _game_version: Option<&'a str>,
        _loader: Option<&'a str>,
    ) -> BoxFuture<'a, crate::error::Result<Vec<ProjectVersion>>> {
        let versions = self.versions.get(project_id).cloned().unwrap_or_default();
        Box::pin(async move { Ok(versions) })
    }

    fn resolve_by_hashes<'a>(
        &'a self,
        _algo: HashAlgo,
        hashes: &'a [String],
    ) -> BoxFuture<'a, crate::error::Result<Vec<Option<ResolvedFile>>>> {
        let n = hashes.len();
        Box::pin(async move { Ok(vec![None; n]) })
    }

    fn get_files_bulk<'a>(
        &'a self,
        refs: &'a [(String, String)],
    ) -> BoxFuture<'a, crate::error::Result<Vec<ResolvedFile>>> {
        let mut out = Vec::new();
        for (project_id, version_id) in refs {
            let Some(version) = self.find_version(version_id) else {
                return Box::pin(async move {
                    Err(crate::error::CoreError::other("unknown version"))
                });
            };
            let file = version.primary_file().cloned().unwrap();
            out.push(ResolvedFile {
                provider: ProviderId::Modrinth,
                project_id: project_id.clone(),
                version_id: version.id.clone(),
                file,
                project_name: None,
                project_slug: None,
                authors: Vec::new(),
            });
        }
        Box::pin(async move { Ok(out) })
    }
}

fn hit(id: &str, slug: &str, title: &str) -> SearchHit {
    SearchHit {
        id: id.to_string(),
        slug: slug.to_string(),
        title: title.to_string(),
        description: format!("{title} desc"),
        author: "author".to_string(),
        downloads: 4242,
        icon_url: None,
        gallery_url: None,
        categories: Vec::new(),
        client_side: ProjectSideSupport::Required,
        server_side: ProjectSideSupport::Required,
    }
}

fn cdn_file(project_id: &str) -> VersionFile {
    VersionFile {
        url: format!("https://cdn.modrinth.com/data/{project_id}/versions/v/{project_id}.jar"),
        filename: format!("{project_id}.jar"),
        sha1: Some(format!("{project_id}-sha1")),
        sha512: Some(format!("{project_id}-sha512")),
        size: Some(100),
        primary: true,
        client_side: ProjectSideSupport::Required,
        server_side: ProjectSideSupport::Required,
    }
}

fn version(id: &str, file: VersionFile, dependencies: Vec<Dependency>) -> ProjectVersion {
    ProjectVersion {
        id: id.to_string(),
        name: format!("{id} name"),
        version_number: "1.0.0".to_string(),
        game_versions: vec!["1.20.1".to_string()],
        loaders: vec!["fabric".to_string()],
        files: vec![file],
        dependencies,
        client_side: ProjectSideSupport::Required,
        server_side: ProjectSideSupport::Required,
    }
}

fn registry_of(provider: FakeChatProvider) -> Arc<ProviderRegistry> {
    Arc::new(ProviderRegistry::new().with(Arc::new(provider)))
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("mc-chat-{tag}-{}-{nanos}", std::process::id()))
}

// ---------------------------------------------------------------------------
// Throwaway localhost servers
// ---------------------------------------------------------------------------

/// Serve `body` once with a Content-Length (used for archive downloads).
fn bytes_server(body: Vec<u8>) -> String {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 16384];
            let _ = stream.read(&mut buf);
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(headers.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    format!("http://{addr}")
}

/// Serve a queue of SSE bodies, one per accepted connection (Rig opens a new
/// connection per turn because we respond with `Connection: close`).
fn sse_server(bodies: Vec<Vec<u8>>) -> String {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for body in bodies {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut buf = [0u8; 65536];
                    let _ = stream.read(&mut buf);
                    let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
                    let _ = stream.write_all(headers.as_bytes());
                    let _ = stream.write_all(&body);
                    let _ = stream.flush();
                }
                Err(_) => break,
            }
        }
    });
    format!("http://{addr}")
}

fn sse_tool_call(id: &str, name: &str, args: serde_json::Value) -> Vec<u8> {
    let args_string = serde_json::to_string(&args).unwrap();
    let chunk = serde_json::json!({
        "id": "gen-tool",
        "model": "mock",
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": id,
                    "type": "function",
                    "function": { "name": name, "arguments": args_string }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    });
    format!("data: {chunk}\n\ndata: [DONE]\n\n").into_bytes()
}

fn sse_final_text(text: &str) -> Vec<u8> {
    let chunk = serde_json::json!({
        "id": "gen-final",
        "model": "mock",
        "choices": [{
            "index": 0,
            "delta": { "content": text },
            "finish_reason": "stop"
        }]
    });
    format!("data: {chunk}\n\ndata: [DONE]\n\n").into_bytes()
}

fn mock_llm(base_url: String) -> AgentLlmClient {
    let mut cfg = AgentLlmConfig::new("test-key");
    cfg.base_url = base_url;
    cfg.model = "mock-model".to_string();
    AgentLlmClient::new(cfg).unwrap()
}

// ---------------------------------------------------------------------------
// Tool tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn search_base_modpacks_maps_provider_hits() {
    let provider = FakeChatProvider {
        search_hits: vec![hit("packid", "cool-pack", "Cool Pack")],
        ..Default::default()
    };
    let tool = SearchBaseModpacksTool {
        registry: registry_of(provider),
    };
    let out = tool
        .call(SearchBaseModpacksArgs {
            query: "tech exploration".to_string(),
            mc_version: Some("1.20.1".to_string()),
            loader: Some("fabric".to_string()),
        })
        .await
        .unwrap();
    assert_eq!(out.candidates.len(), 1);
    let c = &out.candidates[0];
    assert_eq!(c.provider, "modrinth");
    assert_eq!(c.project_id, "packid");
    assert_eq!(c.slug, "cool-pack");
    assert_eq!(c.title, "Cool Pack");
    assert_eq!(c.downloads, 4242);
}

#[tokio::test]
async fn search_mods_maps_provider_hits() {
    let provider = FakeChatProvider {
        search_hits: vec![hit("sodium", "sodium", "Sodium")],
        ..Default::default()
    };
    let tool = SearchModsTool {
        registry: registry_of(provider),
    };
    let out = tool
        .call(SearchModsArgs {
            query: "performance".to_string(),
            mc_version: "1.20.1".to_string(),
            loader: "fabric".to_string(),
        })
        .await
        .unwrap();
    assert_eq!(out.mods.len(), 1);
    assert_eq!(out.mods[0].project_id, "sodium");
    assert_eq!(out.mods[0].provider, "modrinth");
}

#[tokio::test]
async fn resolve_mods_walks_required_dependencies() {
    let mut versions = HashMap::new();
    versions.insert(
        "root".to_string(),
        vec![version(
            "root-v1",
            cdn_file("root"),
            vec![Dependency {
                project_id: Some("dep".to_string()),
                version_id: None,
                dependency_type: "required".to_string(),
            }],
        )],
    );
    versions.insert(
        "dep".to_string(),
        vec![version("dep-v1", cdn_file("dep"), Vec::new())],
    );
    let provider = FakeChatProvider {
        versions,
        ..Default::default()
    };
    let tool = ResolveModsTool {
        registry: registry_of(provider),
    };
    let out = tool
        .call(ResolveModsArgs {
            project_ids: vec!["root".to_string()],
            mc_version: "1.20.1".to_string(),
            loader: "fabric".to_string(),
            already_installed: None,
        })
        .await
        .unwrap();

    let mut ids: Vec<_> = out.resolved.iter().map(|r| r.project_id.clone()).collect();
    ids.sort();
    assert_eq!(ids, vec!["dep".to_string(), "root".to_string()]);
    assert!(out.unresolved.is_empty());
    // Resolved refs carry real version ids + urls echoed straight from the provider.
    let root = out.resolved.iter().find(|r| r.project_id == "root").unwrap();
    assert_eq!(root.version_id, "root-v1");
    assert!(root.url.starts_with("https://cdn.modrinth.com/data/root/"));
}

#[tokio::test]
async fn resolve_mods_honors_already_installed() {
    let mut versions = HashMap::new();
    versions.insert(
        "root".to_string(),
        vec![version("root-v1", cdn_file("root"), Vec::new())],
    );
    let provider = FakeChatProvider {
        versions,
        ..Default::default()
    };
    let tool = ResolveModsTool {
        registry: registry_of(provider),
    };
    let out = tool
        .call(ResolveModsArgs {
            project_ids: vec!["root".to_string()],
            mc_version: "1.20.1".to_string(),
            loader: "fabric".to_string(),
            already_installed: Some(vec!["modrinth:root".to_string()]),
        })
        .await
        .unwrap();
    assert!(out.resolved.is_empty(), "already-installed root should not be resolved again");
}

#[tokio::test]
async fn inspect_base_modpack_parses_modlist_and_enriches() {
    // Minimal .mrpack referencing one Modrinth project via its CDN download url.
    let index = serde_json::json!({
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": "1.0.0",
        "name": "Base Pack",
        "dependencies": { "minecraft": "1.20.1", "fabric-loader": "0.15.7" },
        "files": [{
            "path": "mods/sodium.jar",
            "downloads": ["https://cdn.modrinth.com/data/sodium/versions/v/sodium.jar"],
            "hashes": { "sha512": "h" },
            "fileSize": 100
        }]
    });
    let archive = zip_index(serde_json::to_vec(&index).unwrap());
    let archive_url = bytes_server(archive);

    let mut base_file = cdn_file("basepack");
    base_file.url = archive_url;
    let mut versions = HashMap::new();
    versions.insert("basepack".to_string(), vec![version("basepack-v1", base_file, Vec::new())]);

    let mut projects = HashMap::new();
    let mut sodium_hit = hit("sodium", "sodium", "Sodium");
    sodium_hit.categories = vec!["optimization".to_string()];
    projects.insert("sodium".to_string(), sodium_hit);

    let provider = FakeChatProvider {
        versions,
        projects,
        ..Default::default()
    };
    let tool = InspectBaseModpackTool {
        registry: registry_of(provider),
    };
    let out = tool
        .call(InspectBaseModpackArgs {
            project_id: "basepack".to_string(),
            mc_version: Some("1.20.1".to_string()),
            loader: Some("fabric".to_string()),
        })
        .await
        .unwrap();

    assert_eq!(out.mod_count, 1);
    assert_eq!(out.mods.len(), 1);
    assert_eq!(out.mods[0].title, "Sodium");
    assert_eq!(out.covered_features, vec!["optimization".to_string()]);
    assert_eq!(out.mc_version.as_deref(), Some("1.20.1"));
}

#[tokio::test]
async fn build_modpack_from_scratch_writes_verified_mrpack() {
    let mut versions = HashMap::new();
    versions.insert(
        "sodium".to_string(),
        vec![version("sodium-v1", cdn_file("sodium"), Vec::new())],
    );
    let provider = FakeChatProvider {
        versions,
        ..Default::default()
    };
    let out_dir = temp_dir("build");
    let tool = BuildModpackTool {
        registry: registry_of(provider),
        output_dir: out_dir.clone(),
    };
    let out = tool
        .call(BuildModpackArgs {
            target: BuildTarget {
                mc_version: "1.20.1".to_string(),
                loader: "fabric".to_string(),
            },
            base_pack: None,
            extra_mods: vec![BuildModRef {
                provider: Some("modrinth".to_string()),
                project_id: "sodium".to_string(),
                version_id: "sodium-v1".to_string(),
                title: Some("Sodium".to_string()),
            }],
            // A path-traversal attempt: it must be reduced to a bare basename
            // inside the sandbox, never escaping output_dir.
            output_filename: "../../my pack".to_string(),
        })
        .await
        .unwrap();

    // "verifying" is the post-write status returned by the deterministic executor.
    assert_eq!(out.status, "verifying", "manifest: {}", out.manifest);
    let raw = out.output_path.expect("output path");
    let path = std::path::Path::new(&raw);
    assert_eq!(
        path.parent(),
        Some(out_dir.as_path()),
        "build must stay inside the sandbox output dir: {raw}"
    );
    assert_eq!(
        path.file_name().unwrap().to_string_lossy(),
        "my pack.mrpack",
        "traversal segments must be stripped to a bare basename: {raw}"
    );
    assert!(path.exists(), "mrpack should be on disk");
    let _ = std::fs::remove_dir_all(&out_dir);
}

// ---------------------------------------------------------------------------
// Streaming loop test (mocked model)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn chat_turn_streams_text_and_dispatches_tool_call() {
    // Turn 1: the (mocked) model calls search_base_modpacks. Turn 2: after the
    // tool result is fed back, it answers with text.
    let base_url = sse_server(vec![
        sse_tool_call(
            "call_1",
            "search_base_modpacks",
            serde_json::json!({ "query": "tech", "mc_version": "1.20.1", "loader": "fabric" }),
        ),
        sse_final_text("Here is a base pack you could use."),
    ]);
    let llm = mock_llm(base_url);

    let provider = FakeChatProvider {
        search_hits: vec![hit("packid", "cool-pack", "Cool Pack")],
        ..Default::default()
    };
    let tools = ChatToolsCtx::new(registry_of(provider), temp_dir("loop"));
    let sink = CollectingSink::new();

    let outcome = run_chat_turn(
        &llm,
        &tools,
        ChatTranscript::new(),
        "make me a tech pack for 1.20.1 fabric",
        &sink,
    )
    .await
    .unwrap();

    let events = sink.events();

    // The tool call was surfaced with its name + parsed args.
    let tool_call = events
        .iter()
        .find_map(|e| match e {
            AgentStreamEvent::ToolCall { name, args } => Some((name.clone(), args.clone())),
            _ => None,
        })
        .expect("a ToolCall event");
    assert_eq!(tool_call.0, "search_base_modpacks");
    assert_eq!(tool_call.1["query"], "tech");

    // Rig dispatched the tool and fed the result back; we surfaced a ToolResult.
    let tool_result = events.iter().any(|e| {
        matches!(e, AgentStreamEvent::ToolResult { name, .. } if name == "search_base_modpacks")
    });
    assert!(tool_result, "expected a ToolResult for the dispatched tool: {events:?}");

    // Assistant text streamed through, and the turn ended with Done.
    assert!(events.iter().any(|e| matches!(e, AgentStreamEvent::TextDelta { .. })));
    assert_eq!(events.last(), Some(&AgentStreamEvent::Done));

    assert!(outcome.reply.contains("base pack"), "reply: {}", outcome.reply);
    // The transcript grew: user + assistant turns (with the tool round-trip).
    assert!(outcome.transcript.len() >= 2, "transcript: {:?}", outcome.transcript);
}

fn zip_index(index_json: Vec<u8>) -> Vec<u8> {
    use std::io::{Cursor, Write};
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("modrinth.index.json", options).unwrap();
        zip.write_all(&index_json).unwrap();
        zip.finish().unwrap();
    }
    cursor.into_inner()
}
