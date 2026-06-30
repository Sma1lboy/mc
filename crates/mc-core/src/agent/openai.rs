//! OpenAI client for the lightweight agent runtime.
//!
//! API keys are read from process env or local `.env`; never from settings.json.

use std::path::{Path, PathBuf};

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::Serialize;

use crate::error::{CoreError, Result};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "gpt-5.4-mini";
const OPENAI_KEY_ENV: &str = "OPENAI_API_KEY";
const OPENAI_BASE_URL_ENV: &str = "OPENAI_BASE_URL";
const OPENAI_MODEL_ENV: &str = "MC_AGENT_OPENAI_MODEL";
const FALLBACK_OPENAI_MODEL_ENV: &str = "OPENAI_MODEL";

#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
}

impl OpenAiConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: DEFAULT_MODEL.to_string(),
            base_url: DEFAULT_BASE_URL.to_string(),
        }
    }

    /// Load config from local process state.
    ///
    /// Key priority:
    /// 1. `OPENAI_API_KEY`
    /// 2. `.env` in current directory or its parents
    /// 3. `desktop/src-tauri/.env` under current directory or its parents
    /// 4. `<data_dir>/.env`
    ///
    /// Model override: `MC_AGENT_OPENAI_MODEL`, then `OPENAI_MODEL`.
    /// Base URL override: `OPENAI_BASE_URL`.
    pub fn from_local(data_dir: &Path) -> Result<Self> {
        let env_files = dotenv_candidates(data_dir);
        let api_key = env_value(OPENAI_KEY_ENV, &env_files).ok_or_else(|| {
            CoreError::other(format!(
                "OpenAI API key not found; set {OPENAI_KEY_ENV} or put it in .env"
            ))
        })?;
        let model = env_value(OPENAI_MODEL_ENV, &env_files)
            .or_else(|| env_value(FALLBACK_OPENAI_MODEL_ENV, &env_files))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let base_url = env_value(OPENAI_BASE_URL_ENV, &env_files)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        Ok(Self {
            api_key,
            model,
            base_url,
        })
    }

    pub fn local_env_paths(data_dir: &Path) -> Vec<PathBuf> {
        dotenv_candidates(data_dir)
    }
}

#[derive(Clone)]
pub struct OpenAiClient {
    config: OpenAiConfig,
    client: reqwest::Client,
}

impl OpenAiClient {
    pub fn new(config: OpenAiConfig) -> Result<Self> {
        let mut headers = HeaderMap::new();
        let token = format!("Bearer {}", config.api_key.trim());
        let auth = HeaderValue::from_str(&token)
            .map_err(|e| CoreError::other(format!("invalid OpenAI API key header: {e}")))?;
        headers.insert(AUTHORIZATION, auth);
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .user_agent("mc-launcher-agent/0.1")
            .build()?;
        Ok(Self { config, client })
    }

    pub fn model(&self) -> &str {
        &self.config.model
    }

    fn endpoint(&self) -> String {
        format!("{}/responses", self.config.base_url)
    }

    pub(crate) async fn complete(&self, req: &OpenAiTextRequest) -> Result<OpenAiTextResponse> {
        let body = OpenAiResponsesRequest::from_text_request(&self.config.model, req);
        let res = self.client.post(self.endpoint()).json(&body).send().await?;
        let status = res.status();
        let raw_text = res.text().await?;
        if !status.is_success() {
            return Err(CoreError::other(format!(
                "OpenAI responses API returned {status}: {raw_text}"
            )));
        }
        let raw: serde_json::Value =
            serde_json::from_str(&raw_text).map_err(|e| CoreError::Parse {
                what: "OpenAI responses API payload".into(),
                source: e,
            })?;
        let text = extract_output_text(&raw).ok_or_else(|| {
            CoreError::other("OpenAI responses API payload did not contain output text")
        })?;
        Ok(OpenAiTextResponse {
            model: self.config.model.clone(),
            text,
        })
    }
}

pub(crate) struct OpenAiTextRequest {
    pub instructions: Vec<String>,
    pub input: String,
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub text_format: Option<OpenAiTextFormat>,
}

pub(crate) struct OpenAiTextResponse {
    pub model: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub(crate) enum OpenAiTextFormat {
    #[serde(rename = "json_schema")]
    JsonSchema {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        schema: serde_json::Value,
        strict: bool,
    },
}

#[derive(Debug, Serialize)]
struct OpenAiResponsesRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    input: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<OpenAiTextConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Serialize)]
struct OpenAiTextConfig {
    format: OpenAiTextFormat,
}

impl OpenAiResponsesRequest {
    fn from_text_request(model: &str, req: &OpenAiTextRequest) -> Self {
        let instructions = req
            .instructions
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        Self {
            model: model.to_string(),
            instructions: (!instructions.is_empty()).then_some(instructions),
            input: req.input.trim().to_string(),
            text: req
                .text_format
                .clone()
                .map(|format| OpenAiTextConfig { format }),
            max_output_tokens: req.max_output_tokens,
            temperature: req.temperature,
        }
    }
}

fn env_value(name: &str, env_files: &[PathBuf]) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| dotenv_value_from_files(name, env_files))
}

fn dotenv_candidates(data_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(mut dir) = std::env::current_dir() {
        loop {
            push_unique(&mut out, dir.join(".env"));
            push_unique(&mut out, dir.join("desktop").join("src-tauri").join(".env"));
            if !dir.pop() {
                break;
            }
        }
    }
    push_unique(&mut out, data_dir.join(".env"));
    out
}

fn push_unique(out: &mut Vec<PathBuf>, path: PathBuf) {
    if !out.iter().any(|existing| existing == &path) {
        out.push(path);
    }
}

fn dotenv_value_from_files(name: &str, paths: &[PathBuf]) -> Option<String> {
    for path in paths {
        let Ok(iter) = dotenvy::from_path_iter(path) else {
            continue;
        };
        for item in iter.flatten() {
            if item.0 == name && !item.1.trim().is_empty() {
                return Some(item.1);
            }
        }
    }
    None
}

fn extract_output_text(raw: &serde_json::Value) -> Option<String> {
    if let Some(s) = raw.get("output_text").and_then(|v| v.as_str()) {
        if !s.trim().is_empty() {
            return Some(s.to_string());
        }
    }

    let mut chunks = Vec::new();
    if let Some(output) = raw.get("output").and_then(|v| v.as_array()) {
        for item in output {
            if let Some(content) = item.get("content").and_then(|v| v.as_array()) {
                for c in content {
                    let text = c
                        .get("text")
                        .and_then(|v| v.as_str())
                        .or_else(|| c.get("output_text").and_then(|v| v.as_str()));
                    if let Some(text) = text.filter(|s| !s.trim().is_empty()) {
                        chunks.push(text.to_string());
                    }
                }
            }
        }
    }
    (!chunks.is_empty()).then(|| chunks.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_data_dir() -> PathBuf {
        let mut dir = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        dir.push(format!(
            "mc-core-agent-openai-test-{}-{}",
            std::process::id(),
            nanos
        ));
        dir
    }

    #[test]
    fn reads_values_from_dotenv_file() {
        let dir = temp_data_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let env_path = dir.join(".env");
        std::fs::write(
            &env_path,
            r#"
            OPENAI_API_KEY="sk-dotenv"
            OPENAI_BASE_URL=https://example.com/v1
            export MC_AGENT_OPENAI_MODEL='gpt-test'
            "#,
        )
        .unwrap();
        let files = vec![env_path];
        assert_eq!(
            dotenv_value_from_files("OPENAI_API_KEY", &files).as_deref(),
            Some("sk-dotenv")
        );
        assert_eq!(
            dotenv_value_from_files("OPENAI_BASE_URL", &files).as_deref(),
            Some("https://example.com/v1")
        );
        assert_eq!(
            dotenv_value_from_files("MC_AGENT_OPENAI_MODEL", &files).as_deref(),
            Some("gpt-test")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn extracts_output_text_from_top_level_or_output_items() {
        let top = serde_json::json!({ "output_text": "hello" });
        assert_eq!(extract_output_text(&top).as_deref(), Some("hello"));

        let nested = serde_json::json!({
            "output": [
                { "content": [ { "type": "output_text", "text": "one" } ] },
                { "content": [ { "type": "output_text", "text": "two" } ] }
            ]
        });
        assert_eq!(extract_output_text(&nested).as_deref(), Some("one\ntwo"));
    }

    #[test]
    fn serializes_json_schema_text_format() {
        let req = OpenAiTextRequest {
            instructions: vec!["classify".to_string()],
            input: "make an aviation colony pack".to_string(),
            max_output_tokens: Some(128),
            temperature: Some(0.0),
            text_format: Some(OpenAiTextFormat::JsonSchema {
                name: "agent_intent".to_string(),
                description: Some("Intent classifier".to_string()),
                strict: true,
                schema: serde_json::json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "intent": {
                            "type": "string",
                            "enum": ["build_modpack", "unknown"]
                        }
                    },
                    "required": ["intent"]
                }),
            }),
        };

        let body = OpenAiResponsesRequest::from_text_request("gpt-test", &req);
        let value = serde_json::to_value(body).unwrap();
        assert_eq!(value["instructions"], "classify");
        assert_eq!(value["input"], "make an aviation colony pack");
        assert_eq!(value["text"]["format"]["type"], "json_schema");
        assert_eq!(value["text"]["format"]["name"], "agent_intent");
        assert_eq!(value["text"]["format"]["strict"], true);
        assert_eq!(
            value["text"]["format"]["schema"]["properties"]["intent"]["enum"][0],
            "build_modpack"
        );
    }
}
