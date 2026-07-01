use super::*;

#[test]
fn new_snapshot_starts_running_at_intent_phase() {
    let run = AgentRunSnapshot::new("make an aviation colony pack");
    assert_eq!(run.workflow, AgentWorkflowKind::ModpackBuild);
    assert_eq!(run.schema_version, AGENT_SNAPSHOT_SCHEMA_VERSION);
    assert_eq!(run.status, AgentStatus::Running);
    assert_eq!(run.phase, AgentPhase::IntentExtraction);
    assert!(run.pending_approval.is_none());
    assert!(run.restrictions.is_none());
    assert!(run.id.starts_with("agent-run-"));
}

#[test]
fn transition_methods_keep_status_and_pending_approval_in_lockstep() {
    let approval = ApprovalRequest {
        id: "approval-test".to_string(),
        kind: ApprovalKind::ChooseBasePack,
        title: "title".to_string(),
        message: "message".to_string(),
        options: Vec::new(),
        available_decisions: Vec::new(),
        tools: Vec::new(),
        plan: None,
    };
    let plan = ModpackAgentPlan {
        objective: "objective".to_string(),
        summary_markdown: "summary".to_string(),
        risks: Vec::new(),
        planned_actions: Vec::new(),
        migration_notes: Vec::new(),
    };

    let mut run = AgentRunSnapshot::new("make a pack");

    // request_approval pauses at WaitingForUser with the approval + plan held.
    run.request_approval(
        AgentPhase::ChooseBasePackApproval,
        approval.clone(),
        Some(plan.clone()),
    );
    assert_eq!(run.status, AgentStatus::WaitingForUser);
    assert_eq!(run.phase, AgentPhase::ChooseBasePackApproval);
    assert!(run.pending_approval.is_some());
    assert!(run.plan.is_some());

    // enter_phase leaves the gate into Running and clears the pending approval.
    run.enter_phase(AgentPhase::Executing);
    assert_eq!(run.status, AgentStatus::Running);
    assert_eq!(run.phase, AgentPhase::Executing);
    assert!(run.pending_approval.is_none());

    // complete() from a re-entered gate must clear the pending approval.
    run.request_approval(
        AgentPhase::ConfirmCustomizationApproval,
        approval.clone(),
        None,
    );
    assert!(run.pending_approval.is_some());
    run.complete(AgentPhase::Completed);
    assert_eq!(run.status, AgentStatus::Completed);
    assert_eq!(run.phase, AgentPhase::Completed);
    assert!(run.pending_approval.is_none());

    // fail() from a re-entered gate must likewise clear the pending approval.
    run.request_approval(AgentPhase::ConfirmCustomizationApproval, approval, None);
    assert!(run.pending_approval.is_some());
    run.fail(AgentPhase::Failed);
    assert_eq!(run.status, AgentStatus::Failed);
    assert_eq!(run.phase, AgentPhase::Failed);
    assert!(run.pending_approval.is_none());
}

#[test]
fn approval_and_tool_traces_expose_stream_event_kinds() {
    let approval = ApprovalRequest {
        id: "approval-test".to_string(),
        kind: ApprovalKind::ChooseBasePack,
        title: "title".to_string(),
        message: "message".to_string(),
        options: Vec::new(),
        available_decisions: Vec::new(),
        tools: Vec::new(),
        plan: None,
    };
    let mut run = AgentRunSnapshot::new("make a pack");

    run.request_approval(AgentPhase::ChooseBasePackApproval, approval, None);

    let interrupt = run
        .pending_interrupt
        .as_ref()
        .expect("approval should pause the run through a durable interrupt");
    assert_eq!(interrupt.kind, AgentInterruptKind::UserApproval);
    assert_eq!(interrupt.approval_id.as_deref(), Some("approval-test"));
    assert_eq!(
        interrupt.approval_kind.as_ref(),
        Some(&ApprovalKind::ChooseBasePack)
    );
    assert_eq!(interrupt.resume_token, "approval-test");

    let approval_event = run
        .trace
        .last()
        .expect("request_approval should emit a stream event");
    assert_eq!(
        approval_event.stream_kind,
        Some(AgentStreamEventKind::ApprovalRequired)
    );
    assert_eq!(
        approval_event
            .output
            .as_ref()
            .and_then(|v| v.get("approval_id"))
            .and_then(|v| v.as_str()),
        Some("approval-test")
    );
    assert_eq!(
        approval_event
            .output
            .as_ref()
            .and_then(|v| v.get("interrupt_id"))
            .and_then(|v| v.as_str()),
        Some(interrupt.id.as_str())
    );

    run.push_tool_call_started(
        AgentPhase::BasePackSearch,
        1,
        "modpack_search",
        serde_json::json!({ "query": "create" }),
    );
    assert_eq!(
        run.trace.last().and_then(|event| event.stream_kind),
        Some(AgentStreamEventKind::ToolCallStarted)
    );

    run.push_tool_trace(AgentToolTrace {
        event: "modpack search completed".to_string(),
        stage: AgentPhase::BasePackSearch,
        iteration: 1,
        tool: "modpack_search".to_string(),
        input: serde_json::json!({ "query": "create" }),
        output: serde_json::json!({ "count": 3 }),
        duration_ms: 12,
        status: "ok".to_string(),
    });
    assert_eq!(
        run.trace.last().and_then(|event| event.stream_kind),
        Some(AgentStreamEventKind::ToolCallResult)
    );

    run.push_stream_event(
        AgentStreamEventKind::ClarificationNeeded,
        "approval message needed clarification",
        Some(AgentPhase::ChooseBasePackApproval),
        serde_json::json!({ "reason": "ambiguous reply" }),
    );
    assert_eq!(
        run.trace.last().and_then(|event| event.stream_kind),
        Some(AgentStreamEventKind::ClarificationNeeded)
    );

    run.enter_phase(AgentPhase::BasePackSearch);
    assert!(run.pending_interrupt.is_none());
}

#[test]
fn snapshot_messages_are_soft_capped_and_keep_original_prompt() {
    let mut run = AgentRunSnapshot::new("original prompt");
    run.push_message(AgentMessageKind::User, "original prompt");

    for idx in 0..(MAX_AGENT_MESSAGES + 25) {
        run.push_message(AgentMessageKind::Assistant, format!("revision {idx}"));
    }

    assert_eq!(run.messages.len(), MAX_AGENT_MESSAGES);
    assert_eq!(run.messages[0].kind, AgentMessageKind::User);
    assert_eq!(run.messages[0].text, "original prompt");
    assert_eq!(
        run.messages.last().map(|message| message.text.as_str()),
        Some(format!("revision {}", MAX_AGENT_MESSAGES + 24).as_str())
    );
}

#[test]
fn snapshot_trace_and_replans_are_soft_capped() {
    let mut run = AgentRunSnapshot::new("make a pack");

    for idx in 0..(MAX_AGENT_TRACE_EVENTS + 10) {
        run.push_trace(format!("trace {idx}"));
    }
    for idx in 0..(MAX_AGENT_REPLANS + 10) {
        run.push_replan(PlanReplanRequest {
            id: format!("replan-{idx}"),
            reason: format!("reason {idx}"),
            from_phase: AgentPhase::ChooseBasePackApproval,
            target_phase: AgentPhase::ConfigureRequirementsApproval,
            restriction_patch: None,
            invalidates: vec![PlanArtifact::BasePack],
        });
    }

    assert_eq!(run.trace.len(), MAX_AGENT_TRACE_EVENTS);
    assert_eq!(
        run.trace.first().map(|event| event.event.as_str()),
        Some("trace 10")
    );
    assert_eq!(
        run.trace.last().map(|event| event.event.as_str()),
        Some(format!("trace {}", MAX_AGENT_TRACE_EVENTS + 9).as_str())
    );
    assert_eq!(run.replans.len(), MAX_AGENT_REPLANS);
    assert_eq!(
        run.replans.first().map(|replan| replan.id.as_str()),
        Some("replan-10")
    );
    assert_eq!(
        run.replans.last().map(|replan| replan.id.as_str()),
        Some(format!("replan-{}", MAX_AGENT_REPLANS + 9).as_str())
    );
}

/// Table-driven coverage of the single authoritative normalization pass that
/// `try_apply` now owns. Every case starts from a fresh default (revision 0)
/// and asserts the final stored fields, plus derived `missing_fields` and
/// `warnings`, match the pre-refactor `update_build_restrictions` behavior.
#[test]
fn try_apply_runs_single_authoritative_normalization_pass() {
    struct Case {
        name: &'static str,
        patch: BuildRestrictionPatch,
        version: Option<&'static str>,
        requirement: Option<&'static str>,
        loader: Option<&'static str>,
        tags: Vec<&'static str>,
        notes: Option<&'static str>,
        missing: Vec<&'static str>,
        warnings: Vec<&'static str>,
    }

    let cases = vec![
        Case {
            name: "valid target; tags trimmed+lowercased+deduped; requirement backfilled",
            patch: BuildRestrictionPatch {
                minecraft_version: Some("1.20.1".to_string()),
                minecraft_version_requirement: None,
                loader: Some("Fabric".to_string()),
                feature_tags: vec![" Perf ".to_string(), "perf".to_string(), "QoL".to_string()],
                notes: Some("  keep this  ".to_string()),
            },
            version: Some("1.20.1"),
            requirement: Some("1.20.1"),
            loader: Some("fabric"),
            tags: vec!["perf", "qol"],
            notes: Some("keep this"),
            missing: vec![],
            warnings: vec![],
        },
        Case {
            name: "invalid version dropped WITH a warning (authoritative), not silently",
            patch: BuildRestrictionPatch {
                minecraft_version: Some("99.99".to_string()),
                minecraft_version_requirement: None,
                loader: Some("forge".to_string()),
                feature_tags: vec![],
                notes: None,
            },
            version: None,
            requirement: None,
            loader: Some("forge"),
            tags: vec![],
            notes: None,
            missing: vec!["minecraft_version"],
            warnings: vec!["ignored invalid minecraft_version: 99.99"],
        },
        Case {
            name: "unsupported loader dropped with a warning; raw requirement preserved",
            patch: BuildRestrictionPatch {
                minecraft_version: None,
                minecraft_version_requirement: Some(" 1.20.x ".to_string()),
                loader: Some("modloader".to_string()),
                feature_tags: vec!["adventure".to_string()],
                notes: None,
            },
            version: None,
            requirement: Some("1.20.x"),
            loader: None,
            tags: vec!["adventure"],
            notes: None,
            missing: vec!["minecraft_version", "loader"],
            warnings: vec!["ignored unsupported loader"],
        },
        Case {
            name: "tags capped at 8 BEFORE dedupe, then case-folded dedupe collapses",
            patch: BuildRestrictionPatch {
                minecraft_version: Some("1.19.2".to_string()),
                minecraft_version_requirement: Some("1.19.2".to_string()),
                loader: Some("NeoForge".to_string()),
                feature_tags: vec![
                    "A".to_string(),
                    "a".to_string(),
                    "B".to_string(),
                    "b".to_string(),
                    "C".to_string(),
                    "c".to_string(),
                    "D".to_string(),
                    "d".to_string(),
                    "E".to_string(),
                    "e".to_string(),
                ],
                notes: None,
            },
            version: Some("1.19.2"),
            requirement: Some("1.19.2"),
            loader: Some("neoforge"),
            tags: vec!["a", "b", "c", "d"],
            notes: None,
            missing: vec![],
            warnings: vec![],
        },
        Case {
            name: "empty patch leaves both hard fields missing",
            patch: BuildRestrictionPatch {
                minecraft_version: None,
                minecraft_version_requirement: None,
                loader: None,
                feature_tags: vec![],
                notes: None,
            },
            version: None,
            requirement: None,
            loader: None,
            tags: vec![],
            notes: None,
            missing: vec!["minecraft_version", "loader"],
            warnings: vec![],
        },
    ];

    for case in cases {
        let mut restrictions = BuildRestrictions::default();
        let output = restrictions
            .try_apply(
                0,
                case.patch,
                BuildRestrictionChangeSource::InitialPrompt,
                "test",
            )
            .unwrap_or_else(|err| panic!("{}: try_apply should succeed: {err}", case.name));

        assert_eq!(
            output.restrictions.minecraft_version.as_deref(),
            case.version,
            "{}: minecraft_version",
            case.name
        );
        assert_eq!(
            output.restrictions.minecraft_version_requirement.as_deref(),
            case.requirement,
            "{}: minecraft_version_requirement",
            case.name
        );
        assert_eq!(
            output.restrictions.loader.as_deref(),
            case.loader,
            "{}: loader",
            case.name
        );
        let tags: Vec<&str> = output
            .restrictions
            .feature_tags
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(tags, case.tags, "{}: feature_tags", case.name);
        assert_eq!(
            output.restrictions.notes.as_deref(),
            case.notes,
            "{}: notes",
            case.name
        );
        let missing: Vec<&str> = output.missing_fields.iter().map(String::as_str).collect();
        assert_eq!(missing, case.missing, "{}: missing_fields", case.name);
        let warnings: Vec<&str> = output.warnings.iter().map(String::as_str).collect();
        assert_eq!(warnings, case.warnings, "{}: warnings", case.name);

        // The revision always advances by one and the returned view mirrors
        // the mutated receiver exactly.
        assert_eq!(output.restrictions.revision, 1, "{}: revision", case.name);
        assert_eq!(
            output.restrictions, restrictions,
            "{}: mirrors self",
            case.name
        );
    }
}

#[test]
fn try_apply_preserves_existing_target_when_partial_patch_omits_hard_fields() {
    let mut restrictions = BuildRestrictions {
        revision: 7,
        minecraft_version: Some("1.20.1".to_string()),
        minecraft_version_requirement: Some("1.20.1".to_string()),
        loader: Some("fabric".to_string()),
        feature_tags: vec!["adventure".to_string(), "magic".to_string()],
        notes: Some("old notes".to_string()),
        history: Vec::new(),
    };

    let output = restrictions
        .try_apply(
            7,
            BuildRestrictionPatch {
                minecraft_version: None,
                minecraft_version_requirement: None,
                loader: None,
                feature_tags: vec!["exploration".to_string(), "structures".to_string()],
                notes: Some("lighter exploration; no magic".to_string()),
            },
            BuildRestrictionChangeSource::UserRevise,
            "remove magic without changing target",
        )
        .expect("partial patch should merge into current restrictions");

    assert_eq!(
        output.restrictions.minecraft_version.as_deref(),
        Some("1.20.1")
    );
    assert_eq!(
        output.restrictions.minecraft_version_requirement.as_deref(),
        Some("1.20.1")
    );
    assert_eq!(output.restrictions.loader.as_deref(), Some("fabric"));
    assert!(output.missing_fields.is_empty());
    assert_eq!(
        output.restrictions.feature_tags,
        vec!["exploration".to_string(), "structures".to_string()]
    );
    assert_eq!(
        output.restrictions.notes.as_deref(),
        Some("lighter exploration; no magic")
    );
}

#[test]
fn try_apply_bumps_revision_and_appends_normalized_history() {
    let mut restrictions = BuildRestrictions {
        revision: 4,
        ..Default::default()
    };
    let output = restrictions
        .try_apply(
            4,
            BuildRestrictionPatch {
                minecraft_version: Some("1.20.1".to_string()),
                minecraft_version_requirement: None,
                loader: Some("Fabric".to_string()),
                feature_tags: vec![" Combat ".to_string(), "combat".to_string()],
                notes: None,
            },
            BuildRestrictionChangeSource::UserRevise,
            "revise to fabric 1.20.1",
        )
        .expect("apply should succeed on a matching base revision");

    assert_eq!(restrictions.revision, 5);
    assert_eq!(output.restrictions.revision, 5);
    assert_eq!(restrictions.history.len(), 1);
    let change = &restrictions.history[0];
    assert_eq!(change.revision, 5);
    assert_eq!(change.source, BuildRestrictionChangeSource::UserRevise);
    assert_eq!(change.summary, "revise to fabric 1.20.1");
    // History stores the NORMALIZED patch, not the raw model output.
    assert_eq!(change.patch.loader.as_deref(), Some("fabric"));
    assert_eq!(change.patch.feature_tags, vec!["combat".to_string()]);
    assert_eq!(
        change.patch.minecraft_version_requirement.as_deref(),
        Some("1.20.1")
    );
}

#[test]
fn try_apply_rejects_revision_mismatch_without_mutating() {
    let mut restrictions = BuildRestrictions {
        revision: 3,
        minecraft_version: Some("1.20.1".to_string()),
        minecraft_version_requirement: Some("1.20.1".to_string()),
        loader: Some("fabric".to_string()),
        ..Default::default()
    };
    let before = restrictions.clone();
    let err = restrictions
        .try_apply(
            2,
            BuildRestrictionPatch {
                minecraft_version: Some("1.19.2".to_string()),
                minecraft_version_requirement: None,
                loader: Some("forge".to_string()),
                feature_tags: vec![],
                notes: None,
            },
            BuildRestrictionChangeSource::UserRevise,
            "stale write",
        )
        .expect_err("a stale base_revision must be rejected");

    assert!(
        err.to_string().contains("revision mismatch"),
        "unexpected error: {err}"
    );
    // The optimistic-concurrency guard leaves the receiver untouched.
    assert_eq!(restrictions, before);
}
