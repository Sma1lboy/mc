//! Executor tests: run against throwaway localhost servers, no live network.

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;

use crate::modpack::formats::mrpack::{MrpackDependencies, MrpackIndex};
use crate::modplatform::provider::ProviderRegistry;

use super::execute::{build_mrpack_from_base_archive_bytes_with_env_overrides, execute_mrpack_build_to_path_with_registry};
use super::verify::verify_written_mrpack;
use super::ApprovedModpackBuild;

/// Local one-shot-per-connection HTTP server (mirrors the CLI/download test
/// helpers). Accepts connections in a loop so a transient retry inside the
/// downloader never deadlocks the test, replying with the same response each
/// time. Returns the base URL (`http://addr`).
fn one_response_server(status: u16, content_type: &'static str, body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = [0_u8; 4096];
            let _ = stream.read(&mut buf);
            let reason = match status {
                200 => "OK",
                404 => "Not Found",
                500 => "Internal Server Error",
                _ => "OK",
            };
            let headers = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(headers.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    format!("http://{addr}")
}

fn valid_base_mrpack_bytes() -> Vec<u8> {
    let index = MrpackIndex {
        format_version: 1,
        game: "minecraft".to_string(),
        version_id: "base-1.0.0".to_string(),
        name: "Base Pack".to_string(),
        summary: None,
        dependencies: MrpackDependencies {
            minecraft: Some("1.20.1".to_string()),
            fabric_loader: Some("0.15.7".to_string()),
            ..Default::default()
        },
        files: Vec::new(),
    };
    let index_json = serde_json::to_vec(&index).unwrap();
    let mut output = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut output);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("modrinth.index.json", options).unwrap();
        writer.write_all(&index_json).unwrap();
        writer.finish().unwrap();
    }
    output.into_inner()
}

fn approved_build_for_archive(archive_file: serde_json::Value) -> ApprovedModpackBuild {
    ApprovedModpackBuild {
        base_pack: serde_json::json!({ "provider": "modrinth", "title": "Base Pack" }),
        target: serde_json::json!({ "minecraft_version": "1.20.1", "loader": "fabric" }),
        extra_mods: Vec::new(),
        execution_recipe: Some(serde_json::json!({
            "schema_version": 1,
            "kind": "mrpack_from_base_modpack",
            "format": "mrpack",
            "base_pack_ref": {
                "source_ref": { "archive_file": archive_file }
            },
            "extra_mod_refs": []
        })),
    }
}

fn temp_output_path(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "mc-core-execution-{tag}-{}-{nanos}.mrpack",
        std::process::id()
    ))
}

// A transient base-archive HTTP 404 must NOT `?`-propagate out of the
// executor (which would abort `advance()` before the retry machinery runs).
// It must RETURN a manifest the driver classifies as Retry.
#[tokio::test]
async fn transient_base_archive_http_404_returns_retry_manifest() {
    let server = one_response_server(404, "application/octet-stream", b"missing".to_vec());
    let archive_file = serde_json::json!({
        "url": format!("{server}/base.mrpack"),
        "filename": "base.mrpack",
        "sha1": null,
        "sha512": null,
        "size": null,
        "primary": true,
    });
    let approved = approved_build_for_archive(archive_file);
    let output_path = temp_output_path("http404");

    let manifest = execute_mrpack_build_to_path_with_registry(&approved, &output_path, &ProviderRegistry::with_defaults())
        .await
        .expect("a transient 404 must return a manifest, not propagate Err");

    assert_eq!(
        manifest.get("status").and_then(|v| v.as_str()),
        Some("retry")
    );
    assert_eq!(
        manifest.get("error_kind").and_then(|v| v.as_str()),
        Some("download_404")
    );
    assert!(!output_path.exists());
}

// A checksum mismatch on freshly-fetched base-archive bytes (a CDN serving
// corrupt data) is a `CoreError::Checksum`. It must surface as a retryable
// manifest, not abort the run.
#[tokio::test]
async fn transient_base_archive_checksum_mismatch_returns_retry_manifest() {
    let archive = valid_base_mrpack_bytes();
    let server = one_response_server(200, "application/octet-stream", archive);
    let archive_file = serde_json::json!({
        "url": format!("{server}/base.mrpack"),
        "filename": "base.mrpack",
        "sha1": null,
        // Wrong (but well-formed) sha512 -> CoreError::Checksum from verify.
        "sha512": "0".repeat(128),
        "size": null,
        "primary": true,
    });
    let approved = approved_build_for_archive(archive_file);
    let output_path = temp_output_path("checksum");

    let manifest = execute_mrpack_build_to_path_with_registry(&approved, &output_path, &ProviderRegistry::with_defaults())
        .await
        .expect("a transient checksum mismatch must return a manifest, not propagate Err");

    assert_eq!(
        manifest.get("status").and_then(|v| v.as_str()),
        Some("retry")
    );
    assert_eq!(
        manifest.get("error_kind").and_then(|v| v.as_str()),
        Some("source_unavailable")
    );
    assert!(!output_path.exists());
}

// A structural failure (the fetched base archive is not a valid .mrpack) is
// NOT transient: it must keep blocking back to base-pack selection, never
// turn into a retry.
#[tokio::test]
async fn structural_base_archive_not_mrpack_blocks() {
    let server = one_response_server(200, "application/octet-stream", b"not a zip".to_vec());
    let archive_file = serde_json::json!({
        "url": format!("{server}/base.mrpack"),
        "filename": "base.mrpack",
        "sha1": null,
        "sha512": null,
        "size": null,
        "primary": true,
    });
    let approved = approved_build_for_archive(archive_file);
    let output_path = temp_output_path("structural");

    let manifest = execute_mrpack_build_to_path_with_registry(&approved, &output_path, &ProviderRegistry::with_defaults())
        .await
        .expect("a structural base archive returns a blocked manifest");

    assert_eq!(
        manifest.get("status").and_then(|v| v.as_str()),
        Some("blocked")
    );
    assert_eq!(
        manifest.get("replan_phase").and_then(|v| v.as_str()),
        Some("choose_base_pack_approval")
    );
    assert!(!output_path.exists());
}

// Regression: a base archive shipping an override copy of a file the index
// also manages remotely used to survive the copy and then fail
// `verify_written_mrpack` ("override path ... conflicts with indexed file").
// The conflicting base entry must be dropped; unrelated overrides survive.
#[test]
fn base_override_conflicting_with_indexed_file_is_dropped() {
    let index = MrpackIndex {
        format_version: 1,
        game: "minecraft".to_string(),
        version_id: "base-1.0.0".to_string(),
        name: "Base Pack".to_string(),
        summary: None,
        dependencies: MrpackDependencies {
            minecraft: Some("1.20.1".to_string()),
            fabric_loader: Some("0.15.7".to_string()),
            ..Default::default()
        },
        files: vec![crate::modpack::formats::mrpack::MrpackFile {
            path: "mods/sodium.jar".to_string(),
            hashes: crate::modpack::formats::mrpack::MrpackHashes {
                sha512: "ab".repeat(64),
                sha1: None,
            },
            env: None,
            downloads: vec![
                "https://cdn.modrinth.com/data/AANobbMI/versions/abc/sodium.jar".to_string(),
            ],
            file_size: Some(1024),
        }],
    };
    let index_json = serde_json::to_vec(&index).unwrap();
    let mut base = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut base);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("modrinth.index.json", options).unwrap();
        writer.write_all(&index_json).unwrap();
        // Conflicts with the indexed mods/sodium.jar -> must be dropped.
        writer.start_file("overrides/mods/sodium.jar", options).unwrap();
        writer.write_all(b"stale bundled jar").unwrap();
        // Unrelated override -> must survive.
        writer.start_file("overrides/config/keep.toml", options).unwrap();
        writer.write_all(b"keep = true").unwrap();
        writer.finish().unwrap();
    }
    let base_bytes = base.into_inner();
    let approved = approved_build_for_archive(serde_json::json!({
        "url": "https://cdn.modrinth.com/base.mrpack",
        "filename": "base.mrpack",
        "sha1": null,
        "sha512": null,
        "size": null,
        "primary": true,
    }));

    let built = build_mrpack_from_base_archive_bytes_with_env_overrides(&approved, &base_bytes, &[], &HashMap::new())
        .expect("build must succeed despite the override conflict");
    assert_eq!(
        built.manifest.get("status").and_then(|v| v.as_str()),
        Some("completed")
    );

    let mut names = Vec::new();
    let mut archive = zip::ZipArchive::new(Cursor::new(&built.archive_bytes)).unwrap();
    for i in 0..archive.len() {
        names.push(archive.by_index(i).unwrap().name().to_string());
    }
    assert!(!names.contains(&"overrides/mods/sodium.jar".to_string()));
    assert!(names.contains(&"overrides/config/keep.toml".to_string()));

    // And the written archive passes full verification.
    let output_path = temp_output_path("override-conflict");
    crate::fs::write_atomic(&output_path, &built.archive_bytes).unwrap();
    verify_written_mrpack(&output_path, &approved).expect("verification must pass");
    std::fs::remove_file(&output_path).unwrap();
}
