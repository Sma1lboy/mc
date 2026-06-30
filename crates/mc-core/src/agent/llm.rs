//! Rig-backed LLM runtime for the agent.
//!
//! The workflow owns durable state and phase transitions. This module only
//! owns model configuration and typed structured-output calls.

use std::path::{Path, PathBuf};

use rig_core::prelude::{CompletionClient, TypedPrompt};
use rig_core::providers::openai;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;

use crate::error::{CoreError, Result};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "gpt-5.4-mini";
const OPENAI_KEY_ENV: &str = "OPENAI_API_KEY";
const OPENAI_BASE_URL_ENV: &str = "OPENAI_BASE_URL";
const OPENAI_MODEL_ENV: &str = "MC_AGENT_OPENAI_MODEL";
const FALLBACK_OPENAI_MODEL_ENV: &str = "OPENAI_MODEL";

#[derive(Debug, Clone)]
pub struct AgentLlmConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
}

impl AgentLlmConfig {
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
pub struct AgentLlmClient {
    config: AgentLlmConfig,
    client: openai::Client,
}

impl AgentLlmClient {
    pub fn new(config: AgentLlmConfig) -> Result<Self> {
        let client = openai::Client::builder()
            .api_key(config.api_key.trim())
            .base_url(config.base_url.trim_end_matches('/'))
            .build()
            .map_err(|e| {
                CoreError::other(format!("failed to initialize Rig OpenAI client: {e}"))
            })?;
        Ok(Self { config, client })
    }

    pub fn model(&self) -> &str {
        &self.config.model
    }

    pub(crate) async fn prompt_typed<T>(
        &self,
        instructions: &[&str],
        input: String,
        max_output_tokens: u64,
        temperature: f64,
    ) -> Result<T>
    where
        T: JsonSchema + DeserializeOwned + Send + 'static,
    {
        let preamble = instructions
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let agent = self
            .client
            .agent(self.config.model.clone())
            .preamble(&preamble)
            .temperature(temperature)
            .max_tokens(max_output_tokens)
            .build();
        agent
            .prompt_typed::<T>(input)
            .max_turns(1)
            .await
            .map_err(|e| CoreError::other(format!("Rig structured output failed: {e}")))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_data_dir() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "mc-core-agent-llm-test-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn reads_values_from_dotenv_file() {
        let dir = temp_data_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let env_file = dir.join(".env");
        std::fs::write(
            &env_file,
            r#"
            OPENAI_API_KEY="sk-dotenv"
            OPENAI_BASE_URL=https://example.com/v1
            export MC_AGENT_OPENAI_MODEL='gpt-test'
            "#,
        )
        .unwrap();
        let files = vec![env_file.clone()];

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
}
