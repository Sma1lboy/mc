use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

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
    // executor (which would abort the artifact tool before retry classification).
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

        let manifest = execute_mrpack_build_to_path(&approved, &output_path)
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
        assert!(manifest_is_retryable_external_error(&manifest));
        let outcome = classify_execution_outcome(&manifest).unwrap();
        assert!(matches!(outcome.kind, ExecutionOutcomeKind::Retry));
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

        let manifest = execute_mrpack_build_to_path(&approved, &output_path)
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
        assert!(manifest_is_retryable_external_error(&manifest));
        let outcome = classify_execution_outcome(&manifest).unwrap();
        assert!(matches!(outcome.kind, ExecutionOutcomeKind::Retry));
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

        let manifest = execute_mrpack_build_to_path(&approved, &output_path)
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
        assert!(!manifest_is_retryable_external_error(&manifest));
        let outcome = classify_execution_outcome(&manifest).unwrap();
        assert!(matches!(outcome.kind, ExecutionOutcomeKind::Blocked));
        assert!(!output_path.exists());
    }
}
