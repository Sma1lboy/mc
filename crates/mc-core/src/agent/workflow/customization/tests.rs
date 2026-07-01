use super::*;

#[cfg(test)]
mod base_coverage_tests {
    use super::super::planning_state::mark_goal_status;
    use super::*;
    use crate::agent::{AgentLlmClient, AgentLlmConfig};
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    fn openrouter_body(content: serde_json::Value) -> Vec<u8> {
        serde_json::json!({
            "id": "chatcmpl_test",
            "object": "chat.completion",
            "created": 0,
            "model": "gpt-test",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": content.to_string() },
                "finish_reason": "stop",
                "native_finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })
        .to_string()
        .into_bytes()
    }

    fn one_response_server(body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0_u8; 16384];
                let _ = stream.read(&mut buf);
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(headers.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        format!("http://{addr}")
    }

    fn coverage_llm(content: serde_json::Value) -> AgentLlmClient {
        let mut cfg = AgentLlmConfig::new("test-key");
        cfg.base_url = one_response_server(openrouter_body(content));
        AgentLlmClient::new(cfg).unwrap()
    }

    fn dummy_llm() -> AgentLlmClient {
        AgentLlmClient::new(AgentLlmConfig::new("test-key")).unwrap()
    }

    fn forge_target() -> TargetCompatibility {
        TargetCompatibility {
            minecraft_version: Some("1.20.1".to_string()),
            loader: Some("forge".to_string()),
            version_id: None,
            version_name: None,
            version_number: None,
            game_versions: vec!["1.20.1".to_string()],
            loaders: vec!["forge".to_string()],
            primary_file: None,
            dependencies: Vec::new(),
        }
    }

    fn base_modlist_with_two_mods() -> BaseModlistCache {
        BaseModlistCache {
            refs: vec![
                ModRef::new(ProviderId::Modrinth, "aqua"),
                ModRef::new(ProviderId::Modrinth, "arcane"),
            ],
            source_format: "modrinth".to_string(),
            fetch_count: 1,
        }
    }

    fn restrictions(tags: &[&str]) -> BuildRestrictions {
        BuildRestrictions {
            feature_tags: tags.iter().map(|t| (*t).to_string()).collect(),
            ..Default::default()
        }
    }

    fn enrich_titles(state: &mut ModPlanState, entries: &[(&str, &[&str])]) {
        for (entry, (title, cats)) in state.base_set.iter_mut().zip(entries.iter()) {
            entry.title = Some((*title).to_string());
            if let Some(obj) = entry.payload.as_object_mut() {
                obj.insert("categories".to_string(), serde_json::json!(cats));
            }
        }
    }

    fn test_base() -> SelectedBasePack {
        SelectedBasePack {
            provider: ProviderId::Modrinth,
            project_id: "ocean-pack".to_string(),
            slug: "ocean-pack".to_string(),
            title: "Ocean Pack".to_string(),
            description: Some("An ocean exploration base pack".to_string()),
        }
    }

    fn goal_status(state: &ModPlanState, id: &str) -> GoalStatus {
        state
            .goals
            .iter()
            .find(|goal| goal.id == id)
            .map(|goal| goal.status.clone())
            .unwrap()
    }

    // (a) A base pack whose mods cover a theme goal -> that goal is marked
    // Covered, with no addition and its search query dropped.
    #[tokio::test]
    async fn base_pack_coverage_marks_goal_covered_without_addition() {
        let target = forge_target();
        let base_modlist = base_modlist_with_two_mods();
        let mut state = initialize_mod_plan_state(
            &target,
            &base_modlist,
            Some(&restrictions(&["ocean", "magic"])),
        );
        enrich_titles(
            &mut state,
            &[
                ("Aquaculture", &["adventure", "food"]),
                ("Arcane Arts", &["magic", "technology"]),
            ],
        );

        let llm = coverage_llm(serde_json::json!({
            "covered_goals": [{
                "goal_id": "theme:ocean",
                "covering_mods": ["Aquaculture"],
                "rationale": "Aquaculture already provides ocean content"
            }]
        }));
        let mut run = AgentRunSnapshot::new("ocean pack with magic");

        analyze_base_pack_coverage(
            &llm,
            &mut run,
            "ocean pack with magic",
            &test_base(),
            &mut state,
        )
        .await
        .unwrap();

        assert_eq!(goal_status(&state, "theme:ocean"), GoalStatus::Covered);
        assert_eq!(goal_status(&state, "theme:magic"), GoalStatus::Open);
        assert_eq!(state.base_covered_goals, vec!["theme:ocean".to_string()]);
        assert!(
            !state
                .pending_queries
                .iter()
                .any(|query| query.goal_id == "theme:ocean")
        );
        assert!(
            state
                .pending_queries
                .iter()
                .any(|query| query.goal_id == "theme:magic")
        );
        assert!(state.additions.is_empty());
    }

    // (b) When every theme goal is covered by the base pack, the loop finishes
    // with no extra mods and returns Validated (not Blocked).
    #[tokio::test]
    async fn all_goals_covered_yields_empty_validated_plan() {
        let target = forge_target();
        let base_modlist = base_modlist_with_two_mods();
        let mut state = initialize_mod_plan_state(
            &target,
            &base_modlist,
            Some(&restrictions(&["ocean", "magic"])),
        );
        for id in ["theme:ocean", "theme:magic"] {
            mark_goal_status(&mut state, id, GoalStatus::Covered);
            state.base_covered_goals.push(id.to_string());
        }
        state.pending_queries.clear();

        let mut run = AgentRunSnapshot::new("ocean pack with magic");
        run.mod_plan = Some(state);

        let result = run_customization_planning_loop(
            &dummy_llm(),
            &mut run,
            "ocean pack with magic",
            &test_base(),
            &target,
            &[],
            &base_modlist,
        )
        .await
        .unwrap();

        let CustomizationPlanningResult::Validated(validated) = result else {
            panic!("all-covered plan should validate, not block");
        };
        assert!(
            validated.extra_mods.is_empty(),
            "base pack covers everything; no extra mods should be added"
        );
        let coverage = &validated.validation["base_pack_coverage"];
        assert_eq!(
            coverage["covered_goal_ids"].as_array().map(Vec::len),
            Some(2)
        );
        assert!(
            validated.validation["unresolved_goals"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    // (c) Base-covered goals are absent from the unresolved-goals list.
    #[test]
    fn base_covered_goals_absent_from_unresolved_goals() {
        let target = forge_target();
        let base_modlist = base_modlist_with_two_mods();
        let mut state = initialize_mod_plan_state(
            &target,
            &base_modlist,
            Some(&restrictions(&["ocean", "magic"])),
        );
        mark_goal_status(&mut state, "theme:ocean", GoalStatus::Covered);
        state.base_covered_goals.push("theme:ocean".to_string());

        let unresolved = unresolved_mod_plan_goals(&state, None);

        assert!(
            unresolved
                .iter()
                .any(|goal| goal.get("goal_id").and_then(|v| v.as_str()) == Some("theme:magic"))
        );
        assert!(
            !unresolved
                .iter()
                .any(|goal| goal.get("goal_id").and_then(|v| v.as_str()) == Some("theme:ocean"))
        );
    }
}
