use super::*;

const MODPACK_BUILD_REACT_PROMPT: &str = r#"You are running the modpack_build ReAct workflow.

Use this workflow as guidance, not as a schema:
1. Understand the user's Minecraft version, loader, play style, required mods, and constraints.
2. If critical information is missing, request a runtime interrupt for concise user clarification instead of guessing.
3. Search for a base modpack before proposing from-scratch construction.
4. Decide from facts whether extra mods are needed. Do not force extra mods.
5. If a base modpack already satisfies the request, recommend it without adding redundant mods.
6. Check version, loader, dependency, and obvious conflict compatibility before asking for approval.
7. Request a runtime interrupt before selecting a base pack, approving customization, or writing/exporting files.

Tool-use rules:
- Use tools for facts; do not invent provider projects, files, URLs, hashes, loaders, or versions.
- Prefer short search terms over sentence-like queries.
- Keep provider tool outputs factual and compact.
- Human input is handled by the runtime interrupt channel, not by a tool call.
- Do not emit internal progress-control fields such as next_state, should_advance, or phase."#;

pub(super) fn modpack_build_react_prompt() -> &'static str {
    MODPACK_BUILD_REACT_PROMPT
}

pub(super) fn modpack_build_react_tool_specs() -> Vec<AgentToolSpec> {
    vec![
        update_build_restrictions_tool_spec(),
        extract_modpack_goals_tool_spec(),
        modpack_search_tool_spec(),
        modpack_get_detail_tool_spec(),
        mod_search_tool_spec(),
        mod_get_detail_tool_spec(),
        compatibility_check_tool_spec(),
        plan_customization_tool_spec(),
    ]
}

pub(super) fn begin_modpack_build_react_run(user_prompt: &str) -> AgentRunSnapshot {
    let mut run = AgentRunSnapshot::new(user_prompt);
    run.tools = modpack_build_react_tool_specs();
    run.push_trace("entered prompt-guided ReAct runner for modpack_build");
    run.push_message(AgentMessageKind::User, user_prompt);
    run.push_trace("created run");
    run
}

fn extract_modpack_goals_tool_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: EXTRACT_MODPACK_GOALS_TOOL.to_string(),
        description: "Extract explicit user-facing modpack feature goals that the customization planner must satisfy. Use after update_build_restrictions when the user names required features such as minimap, inventory management, dungeons, structures, exploration, magic, tech, or QoL. Do not include Minecraft version or loader here; those belong to update_build_restrictions.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["goals"],
            "properties": {
                "goals": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Short English goal labels. Examples: minimap, inventory management, dungeons, structures, exploration, performance, magic."
                },
                "rationale": {
                    "type": ["string", "null"],
                    "description": "Short reason for the extracted goal set."
                }
            }
        }),
        output_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["goals"],
            "properties": {
                "goals": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Normalized goal labels that feed mod planning."
                }
            }
        }),
    }
}

fn modpack_search_tool_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: "modpack_search".to_string(),
        description:
            "Search provider catalogs for existing modpacks using factual query and compatibility filters."
                .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["queries"],
            "properties": {
                "queries": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Short English search terms."
                },
                "minecraft_version": {
                    "type": ["string", "null"],
                    "description": "Concrete Minecraft version filter when known."
                },
                "loader": {
                    "type": ["string", "null"],
                    "description": "Loader filter: fabric, forge, neoforge, or quilt."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 12,
                    "default": 6
                }
            }
        }),
        output_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["items"],
            "properties": {
                "items": {
                    "type": "array",
                    "description": "Provider search hits with ids, titles, summaries, and compatibility hints."
                },
                "warnings": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            }
        }),
    }
}

fn modpack_get_detail_tool_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: "modpack_get_detail".to_string(),
        description:
            "Fetch factual metadata, versions, files, and modlist hints for one provider modpack."
                .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["provider", "project_id"],
            "properties": {
                "provider": { "type": "string", "description": "Provider id such as modrinth or curseforge." },
                "project_id": { "type": "string", "description": "Provider project id." },
                "minecraft_version": { "type": ["string", "null"] },
                "loader": { "type": ["string", "null"] }
            }
        }),
        output_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["project"],
            "properties": {
                "project": { "type": "object" },
                "versions": { "type": "array" },
                "files": { "type": "array" },
                "warnings": { "type": "array", "items": { "type": "string" } }
            }
        }),
    }
}

fn mod_search_tool_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: "mod_search".to_string(),
        description:
            "Search provider catalogs for mods using factual query and target compatibility filters."
                .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["queries"],
            "properties": {
                "queries": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Short English mod or feature search terms."
                },
                "minecraft_version": { "type": ["string", "null"] },
                "loader": { "type": ["string", "null"] },
                "exclude_project_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "default": []
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 20,
                    "default": 10
                }
            }
        }),
        output_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["items"],
            "properties": {
                "items": { "type": "array" },
                "warnings": { "type": "array", "items": { "type": "string" } }
            }
        }),
    }
}

fn mod_get_detail_tool_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: "mod_get_detail".to_string(),
        description:
            "Fetch factual metadata, versions, dependencies, and files for one provider mod."
                .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["provider", "project_id"],
            "properties": {
                "provider": { "type": "string" },
                "project_id": { "type": "string" },
                "minecraft_version": { "type": ["string", "null"] },
                "loader": { "type": ["string", "null"] }
            }
        }),
        output_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["project"],
            "properties": {
                "project": { "type": "object" },
                "versions": { "type": "array" },
                "dependencies": { "type": "array" },
                "files": { "type": "array" },
                "warnings": { "type": "array", "items": { "type": "string" } }
            }
        }),
    }
}

fn compatibility_check_tool_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: "compatibility_check".to_string(),
        description:
            "Check selected modpack and mod facts against Minecraft version, loader, dependency, and conflict constraints."
                .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["target", "base_pack", "mods"],
            "properties": {
                "target": {
                    "type": "object",
                    "description": "Minecraft version and loader constraints."
                },
                "base_pack": {
                    "type": ["object", "null"],
                    "description": "Selected base modpack fact payload, or null for scratch planning."
                },
                "mods": {
                    "type": "array",
                    "description": "Candidate mod fact payloads."
                }
            }
        }),
        output_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["compatible", "warnings"],
            "properties": {
                "compatible": { "type": "boolean" },
                "accepted_mods": { "type": "array" },
                "rejected_mods": { "type": "array" },
                "warnings": { "type": "array", "items": { "type": "string" } }
            }
        }),
    }
}

fn plan_customization_tool_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: "plan_customization".to_string(),
        description:
            "Plan extra mods for the selected base pack and prepare a customization approval draft."
                .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {}
        }),
        output_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["approval_ready"],
            "properties": {
                "approval_ready": {
                    "type": "boolean",
                    "description": "True when confirm_customization can be requested through the runtime interrupt channel."
                }
            }
        }),
    }
}
