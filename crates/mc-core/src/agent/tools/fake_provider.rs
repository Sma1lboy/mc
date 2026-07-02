//! In-memory fake provider + tiny localhost servers for tool tests.

use std::collections::HashMap;
use std::sync::Arc;

use futures::future::BoxFuture;

use crate::modplatform::provider::{ProviderRegistry, ResourceProvider};
use crate::modplatform::{
    Dependency, HashAlgo, ProjectSideSupport, ProjectVersion, ProviderCaps, ProviderId,
    ResolvedFile, SearchHit, SearchQuery, VersionFile,
};

use super::ChatToolsCtx;

#[derive(Clone, Default)]
pub(super) struct FakeChatProvider {
    pub(super) search_hits: Vec<SearchHit>,
    pub(super) versions: HashMap<String, Vec<ProjectVersion>>,
    pub(super) projects: HashMap<String, SearchHit>,
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

pub(super) fn hit(id: &str, slug: &str, title: &str) -> SearchHit {
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

pub(super) fn cdn_file(project_id: &str) -> VersionFile {
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

pub(super) fn version(id: &str, file: VersionFile, dependencies: Vec<Dependency>) -> ProjectVersion {
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

pub(super) fn registry_of(provider: FakeChatProvider) -> Arc<ProviderRegistry> {
    Arc::new(ProviderRegistry::new().with(Arc::new(provider)))
}

/// Read-only ctx: every tool but build_modpack ignores output_dir.
pub(super) fn ctx_of(provider: FakeChatProvider) -> ChatToolsCtx {
    ChatToolsCtx::new(registry_of(provider), std::path::PathBuf::new())
}

pub(super) fn temp_dir(tag: &str) -> std::path::PathBuf {
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
pub(super) fn bytes_server(body: Vec<u8>) -> String {
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

pub(super) fn zip_index(index_json: Vec<u8>) -> Vec<u8> {
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
