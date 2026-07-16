use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::error::Result as CoreResult;

use super::chunk::stable_hex;
use super::search::symbol_tokens;
use super::sources::{read_text_file_bounded, regular_dir, relative_slash_path};
use super::WikiSourceDocument;

const FTB_QUESTS_FILE_MAX_BYTES: u64 = 1024 * 1024;
const FTB_RAW_DOCUMENT_MAX_BYTES: usize = 64 * 1024;
const FTB_RAW_QUEST_MAX_BYTES: usize = 16 * 1024;
const FTB_QUESTS_DIRS: &[&str] = &[
    "config/ftbquests",
    "defaultconfigs/ftbquests",
    "serverconfig/ftbquests",
    "world/serverconfig/ftbquests",
];

pub(super) fn read_ftb_quest_documents(root: &Path) -> CoreResult<Vec<WikiSourceDocument>> {
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
            format!(
                "FTB Quests source file: {rel}\n\n{}",
                bounded_text(&content, FTB_RAW_DOCUMENT_MAX_BYTES)
            ),
        ));
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

        let bounded_raw = bounded_text(&block, FTB_RAW_QUEST_MAX_BYTES);
        lines.push(String::new());
        lines.push("Raw quest source:".to_string());
        lines.push(bounded_raw.clone());

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
                "raw": bounded_raw,
                "raw_truncated": block.len() > FTB_RAW_QUEST_MAX_BYTES,
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
