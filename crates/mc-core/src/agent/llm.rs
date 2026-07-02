//! OpenRouter config loader for the TS agent brain.
//!
//! The launcher owns the API key (env / repo-root `.env`); the webview asks for
//! it via the `agent_llm_config` command. No LLM calls happen in Rust.

use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};

const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const DEFAULT_OPENROUTER_MODEL: &str = "deepseek/deepseek-v4-flash";
const OPENROUTER_KEY_ENV: &str = "OPENROUTER_API_KEY";
const OPENROUTER_BASE_URL_ENV: &str = "OPENROUTER_BASE_URL";
const OPENROUTER_MODEL_ENV: &str = "OPENROUTER_MODEL";
const MC_AGENT_OPENROUTER_MODEL_ENV: &str = "MC_AGENT_OPENROUTER_MODEL";

#[derive(Debug, Clone)]
pub struct AgentLlmConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
}

impl AgentLlmConfig {
    /// Load config from local process state.
    ///
    /// Key priority:
    /// 1. `OPENROUTER_API_KEY`
    /// 2. `.env` at the repository workspace root
    ///
    /// OpenRouter model override: `MC_AGENT_OPENROUTER_MODEL`, then
    /// `OPENROUTER_MODEL`.
    /// Base URL override: `OPENROUTER_BASE_URL`.
    pub fn from_local(data_dir: &Path) -> Result<Self> {
        let env_files = dotenv_candidates(data_dir);
        config_from_env_files(&env_files)
    }
}

fn config_from_env_files(env_files: &[PathBuf]) -> Result<AgentLlmConfig> {
    let api_key = env_value(OPENROUTER_KEY_ENV, env_files).ok_or_else(|| {
        CoreError::other(format!(
            "OpenRouter API key not found; set {OPENROUTER_KEY_ENV} or put it in .env"
        ))
    })?;
    let model = env_value(MC_AGENT_OPENROUTER_MODEL_ENV, env_files)
        .or_else(|| env_value(OPENROUTER_MODEL_ENV, env_files))
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_OPENROUTER_MODEL.to_string());
    let base_url = env_value(OPENROUTER_BASE_URL_ENV, env_files)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_OPENROUTER_BASE_URL.to_string())
        .trim_end_matches('/')
        .to_string();
    Ok(AgentLlmConfig {
        api_key,
        model,
        base_url,
    })
}

fn env_value(name: &str, env_files: &[PathBuf]) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| dotenv_value_from_files(name, env_files))
}

fn dotenv_candidates(data_dir: &Path) -> Vec<PathBuf> {
    std::env::current_dir()
        .ok()
        .map(|dir| dotenv_candidates_from(&dir, data_dir))
        .unwrap_or_default()
}

fn dotenv_candidates_from(current_dir: &Path, _data_dir: &Path) -> Vec<PathBuf> {
    find_workspace_root(current_dir)
        .map(|root| vec![root.join(".env")])
        .unwrap_or_default()
}

fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if is_workspace_root(&dir) {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn is_workspace_root(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("Cargo.toml"))
        .map(|toml| toml.contains("[workspace]"))
        .unwrap_or(false)
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
    fn reads_openrouter_values_from_dotenv_file() {
        let dir = temp_data_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let env_file = dir.join(".env");
        std::fs::write(
            &env_file,
            r#"
            OPENROUTER_API_KEY="sk-dotenv"
            OPENROUTER_BASE_URL=https://example.com/v1
            export MC_AGENT_OPENROUTER_MODEL='openai/gpt-test'
            "#,
        )
        .unwrap();
        let files = vec![env_file.clone()];

        assert_eq!(
            dotenv_value_from_files("OPENROUTER_API_KEY", &files).as_deref(),
            Some("sk-dotenv")
        );
        assert_eq!(
            dotenv_value_from_files("OPENROUTER_BASE_URL", &files).as_deref(),
            Some("https://example.com/v1")
        );
        assert_eq!(
            dotenv_value_from_files("MC_AGENT_OPENROUTER_MODEL", &files).as_deref(),
            Some("openai/gpt-test")
        );

        let cfg = config_from_env_files(std::slice::from_ref(&env_file)).unwrap();
        assert_eq!(cfg.api_key, "sk-dotenv");
        assert_eq!(cfg.base_url, "https://example.com/v1");
        assert_eq!(cfg.model, "openai/gpt-test");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dotenv_candidates_use_repo_root_env_only() {
        let root = temp_data_dir();
        let child = root.join("desktop").join("src-tauri");
        let data_dir = temp_data_dir();
        std::fs::create_dir_all(&child).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();

        let paths = dotenv_candidates_from(&child, &data_dir);

        assert_eq!(paths, vec![root.join(".env")]);

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(data_dir);
    }
}
