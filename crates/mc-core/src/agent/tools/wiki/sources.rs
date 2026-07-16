use std::io::Read;
use std::path::{Path, PathBuf};

use crate::error::{CoreError, IoResultExt, Result as CoreResult};
use crate::instance::InstanceConfig;
use crate::version::pack::PackProfile;

use super::cache::WIKI_CORPUS_CACHE_FILE;
use super::chunk::stable_hex;
use super::privacy::{finalize_wiki_documents, sanitize_wiki_text};
use super::{WikiSourceDocument, WIKI_CORPUS_MAX_BYTES, WIKI_CORPUS_MAX_DOCUMENTS};

pub(super) const INSTANCE_DATA_DIRS: &[&str] = &[
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

pub(super) const WIKI_FILE_MAX_BYTES: u64 = 256 * 1024;
const WIKI_ARCHIVE_MAX_BYTES: usize = 512 * 1024;
const WIKI_ARCHIVE_MAX_ENTRIES: usize = 128;
const INSTANCE_DATA_MAX_ENTRIES: usize = 200;

pub(super) fn read_local_wiki_documents(paths: &[PathBuf]) -> CoreResult<Vec<WikiSourceDocument>> {
    let mut docs = Vec::new();
    let mut total_bytes = 0usize;
    let namespace_roots = paths.len() > 1;
    for path in paths {
        let first_doc = docs.len();
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
                    super::structured::read_structured_gameplay_documents(path)?,
                    &mut total_bytes,
                );
                push_bounded_docs(
                    &mut docs,
                    super::ftb::read_ftb_quest_documents(path)?,
                    &mut total_bytes,
                );
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
                    document_from_file(path, &file, content),
                    &mut total_bytes,
                );
            }
        }
        if namespace_roots {
            namespace_source_documents(&mut docs[first_doc..], path);
        }
    }
    finalize_wiki_documents(&mut docs, paths);
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
    let mut lines = vec!["Current modpack instance data".to_string()];
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
        "generated://instance-data".to_string(),
        lines.join("\n"),
        "instance_data",
        serde_json::json!({
            "kind": "instance_data",
            "source": {
                "origin": "local",
                "type": "generated",
                "uri": "generated://instance-data",
            },
        }),
    )))
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

pub(super) fn relative_slash_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub(super) fn collect_wiki_files(path: &Path, files: &mut Vec<PathBuf>) -> CoreResult<()> {
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
        if is_generic_collector_excluded_file(path) {
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
        let raw_name = file.name();
        if !is_allowed_wiki_archive_entry(raw_name) {
            continue;
        }
        let name = raw_name.replace('\\', "/");
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
        let Some(content) = sanitize_wiki_text(Path::new(&name), &content) else {
            continue;
        };
        let archive_id = stable_hex(&path.to_string_lossy());
        let uri = format!("archive://{archive_id}/{name}");
        docs.push(WikiSourceDocument::text(
            format!("Archive entry: {name}"),
            format!("archive:{name}"),
            uri,
            content,
        ));
    }
    docs.sort_by(|a, b| a.uri.cmp(&b.uri));
    Ok(docs)
}

fn namespace_source_documents(docs: &mut [WikiSourceDocument], root: &Path) {
    let prefix = format!("source://{}", stable_hex(&root.to_string_lossy()));
    for doc in docs {
        namespace_source_location(&mut doc.uri, &prefix);
        namespace_source_location(&mut doc.provenance.uri, &prefix);
        if let Some(serde_json::Value::String(uri)) = doc
            .structured
            .as_mut()
            .and_then(|value| value.pointer_mut("/source/uri"))
        {
            namespace_source_location(uri, &prefix);
        }
    }
}

fn namespace_source_location(location: &mut String, prefix: &str) {
    if location.starts_with("archive://") {
        return;
    }
    let suffix = location.replace("://", "/");
    *location = format!("{prefix}/{}", suffix.trim_start_matches(['/', '\\']));
}

fn document_from_file(root: &Path, path: &Path, content: String) -> WikiSourceDocument {
    let uri = if root == path {
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("local-source")
            .to_string()
    } else {
        relative_slash_path(root, path)
    };
    WikiSourceDocument::text(uri.clone(), uri.clone(), uri, content)
}

pub(super) fn read_text_file_bounded(path: &Path, max_bytes: u64) -> Option<String> {
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
    let content = String::from_utf8(bytes).ok()?;
    sanitize_wiki_text(path, &content)
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
    let bytes = name.as_bytes();
    let unsafe_segment = name.split(|ch| ch == '/' || ch == '\\').any(|segment| {
        segment == ".." || segment.starts_with('~') || segment.as_bytes().get(1) == Some(&b':')
    });
    if name.starts_with('/')
        || name.starts_with('\\')
        || bytes.get(1) == Some(&b':')
        || name.contains("://")
        || unsafe_segment
        || should_skip_virtual_path(name)
    {
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

pub(super) fn should_skip_virtual_path(path: &str) -> bool {
    path.split(['/', '\\'])
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

fn is_generic_collector_excluded_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            name.eq_ignore_ascii_case("instance.json")
                || name.eq_ignore_ascii_case(WIKI_CORPUS_CACHE_FILE)
        })
        .unwrap_or(false)
}

pub(super) fn is_wiki_corpus_cache_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case(WIKI_CORPUS_CACHE_FILE))
        .unwrap_or(false)
}

pub(super) fn regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|meta| !meta.file_type().is_symlink() && meta.is_file())
        .unwrap_or(false)
}

pub(super) fn regular_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|meta| !meta.file_type().is_symlink() && meta.is_dir())
        .unwrap_or(false)
}

pub(super) fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
}
