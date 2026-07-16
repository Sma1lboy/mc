use sha2::{Digest, Sha256};

use super::{WikiChunk, WikiSourceDocument};

const WIKI_CHUNK_MAX_LINES: usize = 80;
const WIKI_CHUNK_MAX_BYTES: usize = 64 * 1024;

pub(super) fn chunks_from_document(doc: &WikiSourceDocument) -> Vec<WikiChunk> {
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
        provenance: doc.provenance.clone(),
        kind: doc.kind.clone(),
        structured: doc.structured.clone(),
    }
}

pub(super) fn stable_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())[..16].to_string()
}
