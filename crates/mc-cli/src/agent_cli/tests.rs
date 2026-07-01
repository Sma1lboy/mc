use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use mc_core::agent::{
        AgentExecutionMetadata, AgentExecutionStatus, ApprovalDecisionSpec, ApprovalOption,
        UserDecisionKind,
    };
    use mc_core::modpack::formats::mrpack::{MrpackDependencies, MrpackIndex};

    fn temp_data_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("mc-agent-cli-{tag}-{}-{nanos}", std::process::id()))
    }

    fn temp_mrpack_path(tag: &str) -> PathBuf {
        temp_data_dir(tag).with_extension("mrpack")
    }

    fn zip_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::{Cursor, Write};
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default();
            for (path, bytes) in files {
                zip.start_file(*path, options).unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
        }
        cursor.into_inner()
    }

    fn base_archive_for_cli_execute() -> Vec<u8> {
        let base_index = MrpackIndex {
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
        let base_index_json = serde_json::to_vec(&base_index).unwrap();
        zip_bytes(&[("modrinth.index.json", &base_index_json)])
    }

    fn one_response_server(status: u16, content_type: &'static str, body: Vec<u8>) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
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

    fn approval_route_runtime(decision: serde_json::Value) -> mc_core::agent::MainAgentRuntime {
        let body = openrouter_response_body(decision.to_string());
        let base_url = one_response_server(200, "application/json", body);
        let mut cfg = mc_core::agent::AgentLlmConfig::new("test-key");
        cfg.base_url = base_url;
        let llm = mc_core::agent::AgentLlmClient::new(cfg).unwrap();
        mc_core::agent::MainAgentRuntime::new(llm)
    }

    fn openrouter_response_body(output_text: String) -> Vec<u8> {
        serde_json::json!({
            "id": "chatcmpl_test",
            "object": "chat.completion",
            "created": 0,
            "model": "gpt-test",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": output_text
                },
                "finish_reason": "stop",
                "native_finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 1,
                "completion_tokens": 1,
                "total_tokens": 2
            }
        })
        .to_string()
        .into_bytes()
    }

    fn archive_file_payload(url: &str, size: usize) -> serde_json::Value {
        serde_json::json!({
            "url": url,
            "filename": "base.mrpack",
            "sha1": null,
            "sha512": null,
            "size": size,
            "primary": true,
        })
    }

    fn execution_ready_snapshot(
        session_id: &str,
        base_url: &str,
        base_size: usize,
    ) -> AgentRunSnapshot {
        let mut run = AgentRunSnapshot::new("make a pack");
        run.id = session_id.to_string();
        run.status = AgentStatus::Running;
        run.phase = AgentPhase::ExecutionReady;
        run.pending_approval = None;
        run.approved_build = Some(mc_core::agent::ApprovedModpackBuild {
            base_pack: serde_json::json!({
                "provider": "modrinth",
                "title": "Base Pack",
            }),
            target: serde_json::json!({
                "minecraft_version": "1.20.1",
                "loader": "fabric",
            }),
            extra_mods: Vec::new(),
            execution_recipe: Some(serde_json::json!({
                "schema_version": 1,
                "kind": "mrpack_from_base_modpack",
                "format": "mrpack",
                "base_pack_ref": {
                    "source_ref": {
                        "archive_file": archive_file_payload(base_url, base_size)
                    }
                },
                "extra_mod_refs": []
            })),
        });
        run.execution = Some(AgentExecutionMetadata {
            status: AgentExecutionStatus::NotStarted,
            manifest: None,
            blocked: None,
        });
        run
    }

    fn customization_approval_snapshot(
        session_id: &str,
        base_url: &str,
        base_size: usize,
    ) -> AgentRunSnapshot {
        let mut run = AgentRunSnapshot::new("make a pack");
        run.id = session_id.to_string();
        run.status = AgentStatus::WaitingForUser;
        run.phase = AgentPhase::ConfirmCustomizationApproval;
        run.pending_approval = Some(ApprovalRequest {
            id: "approval-test".to_string(),
            kind: ApprovalKind::ConfirmCustomization,
            title: "Confirm customization plan".to_string(),
            message: "Ready to execute after confirmation".to_string(),
            options: vec![ApprovalOption {
                id: "confirm:recommended_customization".to_string(),
                label: "Confirm recommended plan".to_string(),
                description: None,
                payload: Some(serde_json::json!({
                    "base_pack": {
                        "provider": "modrinth",
                        "title": "Base Pack"
                    },
                    "target": {
                        "minecraft_version": "1.20.1",
                        "loader": "fabric"
                    },
                    "extra_mods": [],
                    "execution_recipe": {
                        "schema_version": 1,
                        "kind": "mrpack_from_base_modpack",
                        "format": "mrpack",
                        "base_pack_ref": {
                            "source_ref": {
                                "archive_file": archive_file_payload(base_url, base_size)
                            }
                        },
                        "extra_mod_refs": []
                    }
                })),
            }],
            available_decisions: vec![
                ApprovalDecisionSpec {
                    kind: UserDecisionKind::Approve,
                    label: "Confirm recommended plan".to_string(),
                    requires_selected_option: true,
                    requires_message: false,
                },
                ApprovalDecisionSpec {
                    kind: UserDecisionKind::Revise,
                    label: "Change extra mods".to_string(),
                    requires_selected_option: false,
                    requires_message: true,
                },
            ],
            tools: Vec::new(),
            plan: None,
        });
        run
    }

    fn blocked_customization_snapshot(session_id: &str) -> AgentRunSnapshot {
        let mut run = AgentRunSnapshot::new("make a pack");
        run.id = session_id.to_string();
        run.status = AgentStatus::WaitingForUser;
        run.phase = AgentPhase::ConfirmCustomizationApproval;
        run.pending_approval = Some(ApprovalRequest {
            id: "approval-test".to_string(),
            kind: ApprovalKind::ConfirmCustomization,
            title: "Customization planning is blocked".to_string(),
            message: "Could not produce a verified compatible extra-mod plan.".to_string(),
            options: vec![ApprovalOption {
                id: "back:choose_base_pack".to_string(),
                label: "Back to base-pack selection".to_string(),
                description: None,
                payload: Some(serde_json::json!({
                    "action": "back_to_base_pack",
                    "base_pack": {
                        "provider": "modrinth",
                        "project_id": "base-project",
                        "title": "Base Pack"
                    }
                })),
            }],
            available_decisions: vec![
                ApprovalDecisionSpec {
                    kind: UserDecisionKind::Approve,
                    label: "Back to base-pack selection".to_string(),
                    requires_selected_option: true,
                    requires_message: false,
                },
                ApprovalDecisionSpec {
                    kind: UserDecisionKind::Cancel,
                    label: "Cancel".to_string(),
                    requires_selected_option: false,
                    requires_message: false,
                },
            ],
            tools: Vec::new(),
            plan: None,
        });
        run
    }

    fn base_archive_server() -> (Vec<u8>, String) {
        let archive = base_archive_for_cli_execute();
        let server = one_response_server(200, "application/octet-stream", archive.clone());
        (archive, format!("{server}/base.mrpack"))
    }

    fn save_snapshot(data_dir: &Path, run: &AgentRunSnapshot) -> mc_core::agent::AgentSessionStore {
        let store = mc_core::agent::AgentSessionStore::new(data_dir);
        store.save_snapshot(run).unwrap();
        store
    }

    fn assert_mrpack_contains_index(path: &Path) {
        assert!(path.exists());
        let file = std::fs::File::open(path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert!(archive.by_name("modrinth.index.json").is_ok());
    }

    fn assert_missing_session_error(err: anyhow::Error, data_dir: &Path) {
        let text = err.to_string();
        assert!(text.contains("Session 'missing-session' was not found"));
        assert!(text.contains("mc agent list"));
        assert!(!text.contains("snapshot.json"));
        assert!(!text.contains(data_dir.to_string_lossy().as_ref()));
    }

    #[test]
    fn default_agent_output_path_uses_agent_data_dir() {
        let data_dir =
            std::env::temp_dir().join(format!("mc-agent-cli-test-{}", std::process::id()));

        let output = default_agent_output_path(&data_dir, "session-123");

        assert_eq!(
            output,
            data_dir
                .join("agent")
                .join("artifacts")
                .join("session-123.mrpack")
        );
        assert!(output.is_absolute());
    }

    #[test]
    fn agent_start_surface_supports_home_only_for_now() {
        let variants: Vec<_> = AgentStartSurface::value_variants()
            .iter()
            .filter_map(|surface| surface.to_possible_value())
            .map(|value| value.get_name().to_string())
            .collect();
        assert_eq!(variants, vec!["home"]);
    }

    #[test]
    fn agent_entry_from_start_flags_accepts_home_only() {
        assert_eq!(
            agent_entry_from_start_flags(AgentStartSurface::Home).unwrap(),
            AgentEntry::Home
        );
    }

    #[test]
    fn agent_export_subcommand_routes_to_mrpack_artifact_export() {
        use clap::{FromArgMatches, Subcommand};

        let cmd = AgentAction::augment_subcommands(clap::Command::new("agent"));
        let matches = cmd
            .try_get_matches_from([
                "agent",
                "export",
                "--session-id",
                "session-123",
                "--output",
                "pack.mrpack",
            ])
            .expect("agent export should parse as the mrpack export command");
        let action = AgentAction::from_arg_matches(&matches).expect("action should decode");

        let AgentAction::Execute {
            session_id, output, ..
        } = action
        else {
            panic!("agent export should route to the existing artifact execution path");
        };
        assert_eq!(session_id, "session-123");
        assert_eq!(output, PathBuf::from("pack.mrpack"));
    }

    #[tokio::test]
    async fn missing_session_returns_friendly_error_without_internal_path() {
        let data_dir = temp_data_dir("missing-show");
        let err = cmd_agent_show_with_dir(&data_dir, "missing-session", true)
            .expect_err("missing session should be user-facing");
        assert_missing_session_error(err, &data_dir);
        let runtime = deterministic_agent_runtime().unwrap();
        let err = cmd_agent_continue_with_runtime(
            &data_dir,
            &runtime,
            "missing-session",
            "Continue",
            true,
        )
        .await
        .expect_err("missing session should be user-facing");

        assert_missing_session_error(err, &data_dir);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn continue_completed_session_returns_clear_status_error() {
        let data_dir = temp_data_dir("completed-continue");
        let session_id = "completed-continue-session";
        let mut run = AgentRunSnapshot::new("make a pack");
        run.id = session_id.to_string();
        run.status = AgentStatus::Completed;
        run.phase = AgentPhase::Completed;
        save_snapshot(&data_dir, &run);
        let runtime = deterministic_agent_runtime().unwrap();

        let err =
            cmd_agent_continue_with_runtime(&data_dir, &runtime, session_id, "Continue", true)
                .await
                .expect_err("completed session should not continue");
        let text = err.to_string();

        assert!(text.contains("This session is completed and cannot be continued."));
        assert!(!text.contains("pending approval"));
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn execution_ready_next_step_points_to_explicit_export_command() {
        let mut run = AgentRunSnapshot::new("make a pack");
        run.id = "session-123".to_string();
        run.status = AgentStatus::Running;
        run.phase = AgentPhase::ExecutionReady;

        let next = execution_next_step_command(&run).expect("execution-ready next step");

        assert_eq!(
            next,
            "mc agent export --session-id session-123 --output <path>"
        );
    }

    #[test]
    fn blocked_customization_next_step_only_advertises_back() {
        let run = blocked_customization_snapshot("blocked-customization-session");

        let next_steps = pending_approval_next_step_lines(&run);

        assert_eq!(next_steps.len(), 1);
        assert_eq!(
            next_steps[0],
            "mc agent continue --session-id blocked-customization-session --message \"Back to base-pack selection\""
        );
    }

    #[test]
    fn customization_next_steps_include_confirm_and_revise_when_available() {
        let run = customization_approval_snapshot(
            "customization-session",
            "https://example.invalid/base.mrpack",
            1024,
        );

        let next_steps = pending_approval_next_step_lines(&run);

        assert!(next_steps.iter().any(|line| line.contains(
            "mc agent continue --session-id customization-session --message \"Confirm this mod plan and continue\""
        )));
        assert!(
            next_steps
                .iter()
                .any(|line| line.contains("Remove tech and machinery mods"))
        );
    }

    #[tokio::test]
    async fn continue_to_execution_ready_does_not_write_artifact() {
        let data_dir = temp_data_dir("continue-ready");
        let session_id = "continue-ready-session";
        let (base_archive, base_url) = base_archive_server();
        let run = customization_approval_snapshot(session_id, &base_url, base_archive.len());
        let store = save_snapshot(&data_dir, &run);
        let output = default_agent_output_path(&data_dir, session_id);
        let runtime = approval_route_runtime(serde_json::json!({
            "decision": "approve",
            "selected_option_id": "confirm:recommended_customization",
            "message": null,
            "rationale": "user confirmed"
        }));

        let next =
            cmd_agent_continue_with_runtime(&data_dir, &runtime, session_id, "Confirm plan", true)
                .await
                .expect("continue should reach execution-ready without executing");

        assert_eq!(next.status, AgentStatus::Running);
        assert_eq!(next.phase, AgentPhase::ExecutionReady);
        assert!(!output.exists(), "continue must not write mrpack artifacts");
        let saved = store.load_snapshot(session_id).unwrap();
        assert_eq!(saved.phase, AgentPhase::ExecutionReady);
        assert_eq!(
            execution_next_step_command(&saved).as_deref(),
            Some("mc agent export --session-id continue-ready-session --output <path>")
        );
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn continue_with_unrelated_approval_message_stays_at_gate_without_artifact() {
        let data_dir = temp_data_dir("continue-unrelated");
        let session_id = "continue-unrelated-session";
        let (base_archive, base_url) = base_archive_server();
        let run = customization_approval_snapshot(session_id, &base_url, base_archive.len());
        let store = save_snapshot(&data_dir, &run);
        let output = default_agent_output_path(&data_dir, session_id);
        let runtime = approval_route_runtime(serde_json::json!({
            "decision": "needs_clarification",
            "selected_option_id": null,
            "message": null,
            "rationale": "user message is unrelated to the current approval gate"
        }));

        let next = cmd_agent_continue_with_runtime(
            &data_dir,
            &runtime,
            session_id,
            "I want to go to the beach for coffee.",
            true,
        )
        .await
        .expect("continue should save a clarification snapshot instead of failing");

        assert_eq!(next.status, AgentStatus::WaitingForUser);
        assert_eq!(next.phase, AgentPhase::ConfirmCustomizationApproval);
        assert!(next.approved_build.is_none());
        assert!(next.execution.is_none());
        assert!(
            !output.exists(),
            "invalid continue input must not write artifacts"
        );
        let saved = store.load_snapshot(session_id).unwrap();
        assert_eq!(saved.phase, AgentPhase::ConfirmCustomizationApproval);
        assert_eq!(
            saved
                .pending_approval
                .as_ref()
                .map(|approval| &approval.kind),
            Some(&ApprovalKind::ConfirmCustomization)
        );
        let last = saved
            .messages
            .last()
            .expect("clarification should be saved in the snapshot");
        assert_eq!(last.kind, mc_core::agent::AgentMessageKind::Assistant);
        assert!(
            last.text.contains("does not match") && last.text.contains("state was left unchanged"),
            "unexpected clarification: {}",
            last.text
        );
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn clarification_message_is_shown_after_save_trace() {
        let mut snapshot = customization_approval_snapshot(
            "clarification-display-session",
            "https://example.invalid/base.mrpack",
            1024,
        );
        snapshot.push_message(
            AgentMessageKind::Assistant,
            "Choose an available option, describe a change, or cancel.",
        );
        snapshot.push_trace(
            "approval message needed clarification at customization approval: unrelated input",
        );
        snapshot.push_trace("saved local agent session");

        assert_eq!(
            latest_approval_clarification_message(&snapshot),
            Some("Choose an available option, describe a change, or cancel.")
        );
    }

    #[test]
    fn customization_unresolved_requests_are_rendered_from_validation_payload() {
        let payload = serde_json::json!({
            "validation": {
                "unresolved_goals": [{
                    "label": "Add Advent of Ascension 3",
                    "diagnosis": "No compatible Fabric 1.20.1 candidates were available.",
                    "next_step": "Revise the request, keep the current plan, or change the target and replan."
                }]
            }
        });

        let lines = customization_unresolved_request_lines(&payload);

        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("Add Advent of Ascension 3"));
        assert!(lines[0].contains("No compatible Fabric 1.20.1 candidates"));
        assert!(lines[0].contains("change the target"));
    }

    #[tokio::test]
    async fn execute_writes_artifact_to_requested_output() {
        let data_dir = temp_data_dir("execute-ready");
        let session_id = "execute-ready-session";
        let (base_archive, base_url) = base_archive_server();
        let run = execution_ready_snapshot(session_id, &base_url, base_archive.len());
        save_snapshot(&data_dir, &run);
        let output = temp_mrpack_path("explicit-output");

        let next = cmd_agent_execute_with_dir(&data_dir, session_id, &output, true)
            .await
            .expect("execute should write requested output");

        assert_eq!(next.status, AgentStatus::Completed);
        assert_mrpack_contains_index(&output);
        let default_output = default_agent_output_path(&data_dir, session_id);
        assert!(
            !default_output.exists(),
            "execute must honor --output instead of writing the default path"
        );
        let _ = std::fs::remove_file(output);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn execute_before_approval_returns_clear_error_without_writing() {
        let data_dir = temp_data_dir("execute-unapproved");
        let session_id = "unapproved-session";
        let mut run = AgentRunSnapshot::new("make a pack");
        run.id = session_id.to_string();
        run.status = AgentStatus::WaitingForUser;
        run.phase = AgentPhase::ConfirmCustomizationApproval;
        save_snapshot(&data_dir, &run);
        let output = temp_mrpack_path("unapproved-output");

        let err = cmd_agent_execute_with_dir(&data_dir, session_id, &output, true)
            .await
            .expect_err("execute before approval should fail");

        assert!(
            err.to_string()
                .contains("does not have an approved executable plan"),
            "unexpected error: {err}"
        );
        assert!(!output.exists());
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn execute_completed_session_copies_existing_artifact_to_requested_output() {
        let data_dir = temp_data_dir("execute-completed-copy");
        let session_id = "completed-session";
        let source = temp_mrpack_path("completed-source");
        let output = temp_mrpack_path("completed-new-output");
        let archive = base_archive_for_cli_execute();
        if let Some(parent) = source.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&source, &archive).unwrap();
        let mut run = AgentRunSnapshot::new("make a pack");
        run.id = session_id.to_string();
        run.status = AgentStatus::Completed;
        run.phase = AgentPhase::Completed;
        run.execution = Some(AgentExecutionMetadata {
            status: AgentExecutionStatus::Completed,
            manifest: Some(serde_json::json!({
                "status": "completed",
                "format": "mrpack",
                "output_path": source.to_string_lossy(),
                "output_size": archive.len(),
            })),
            blocked: None,
        });
        save_snapshot(&data_dir, &run);

        let next = cmd_agent_execute_with_dir(&data_dir, session_id, &output, true)
            .await
            .expect("completed execute should copy recorded artifact");

        assert_eq!(next.status, AgentStatus::Completed);
        assert_mrpack_contains_index(&output);
        let manifest = next
            .execution
            .as_ref()
            .and_then(|execution| execution.manifest.as_ref())
            .expect("completed manifest should be present");
        assert_eq!(
            manifest.get("output_path").and_then(|v| v.as_str()),
            Some(output.to_string_lossy().as_ref())
        );
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_file(output);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn exec_smoke_manifest_builds_ready_retry_failed_and_blocked_outcomes() {
        let cases = [
            (AgentExecSmokeOutcome::Ready, None, "status", "ready"),
            (
                AgentExecSmokeOutcome::Retry,
                Some("cdn timed out"),
                "error_kind",
                "network_timeout",
            ),
            (
                AgentExecSmokeOutcome::Failed,
                Some("corrupt archive"),
                "reason",
                "corrupt archive",
            ),
            (
                AgentExecSmokeOutcome::Completed,
                None,
                "status",
                "completed",
            ),
            (
                AgentExecSmokeOutcome::BlockedRequirements,
                Some("target mismatch"),
                "replan_phase",
                "requirements",
            ),
        ];

        for (outcome, reason, field, expected) in cases {
            let manifest = exec_smoke_manifest(outcome, reason);
            assert_eq!(manifest.get(field).and_then(|v| v.as_str()), Some(expected));
        }
    }
}
