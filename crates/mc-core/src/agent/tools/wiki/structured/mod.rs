use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use crate::error::{CoreError, IoResultExt, Result as CoreResult};

use super::sources::{
    read_text_file_bounded, regular_dir, regular_file, relative_slash_path,
    should_skip_virtual_path, WIKI_FILE_MAX_BYTES,
};
use super::WikiSourceDocument;

mod kubejs;
mod recipes;

const MOD_JAR_ENTRY_MAX_BYTES: u64 = 256 * 1024;

pub(super) fn read_structured_gameplay_documents(
    root: &Path,
) -> CoreResult<Vec<WikiSourceDocument>> {
    let mut labels = HashMap::new();
    let mut docs = read_mod_jar_structured_documents(root, &mut labels)?;
    docs.extend(kubejs::read_kubejs_recipe_script_documents(root, &labels)?);
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
        if recipes::is_lang_entry(&name) {
            lang_entries.push(name);
        } else if recipes::is_recipe_entry(&name) {
            recipe_entries.push(name);
        } else if recipes::is_tag_entry(&name) {
            tag_entries.push(name);
        } else if recipes::is_patchouli_entry(&name) {
            patchouli_entries.push(name);
        }
    }

    let mut local_labels = HashMap::new();
    for name in preferred_lang_entries(&lang_entries) {
        if let Some(content) = read_zip_text_by_name(&mut archive, &name) {
            local_labels.extend(recipes::labels_from_lang_json(&content));
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
        if let Some(doc) =
            recipes::recipe_document_from_json(&uri, &jar_rel, &name, &content, &labels)
        {
            docs.push(doc);
        }
    }
    for name in tag_entries {
        let Some(content) = read_zip_text_by_name(&mut archive, &name) else {
            continue;
        };
        let uri = format!("{}!{name}", jar_path.display());
        if let Some(doc) = recipes::tag_document_from_json(&uri, &jar_rel, &name, &content, &labels)
        {
            docs.push(doc);
        }
    }
    for name in patchouli_entries {
        let Some(content) = read_zip_text_by_name(&mut archive, &name) else {
            continue;
        };
        let uri = format!("{}!{name}", jar_path.display());
        if let Some(doc) = recipes::patchouli_document_from_json(&uri, &jar_rel, &name, &content) {
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
        if !(recipes::is_recipe_entry(&entry_name)
            || recipes::is_tag_entry(&entry_name)
            || recipes::is_patchouli_entry(&entry_name))
        {
            continue;
        }
        let Some(content) = read_text_file_bounded(&file, WIKI_FILE_MAX_BYTES) else {
            continue;
        };
        let uri = file.to_string_lossy().to_string();
        let source_rel = relative_slash_path(root, &file);
        if recipes::is_recipe_entry(&entry_name) {
            if let Some(doc) =
                recipes::recipe_document_from_json(&uri, &source_rel, &entry_name, &content, labels)
            {
                docs.push(doc);
            }
        } else if recipes::is_tag_entry(&entry_name) {
            if let Some(doc) =
                recipes::tag_document_from_json(&uri, &source_rel, &entry_name, &content, labels)
            {
                docs.push(doc);
            }
        } else if let Some(doc) =
            recipes::patchouli_document_from_json(&uri, &source_rel, &entry_name, &content)
        {
            docs.push(doc);
        }
    }
    Ok(docs)
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
