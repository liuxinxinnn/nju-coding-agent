use std::env;
use std::path::PathBuf;

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub workspace: PathBuf,
    pub max_steps: usize,
    pub auto_approve: bool,
}

impl Config {
    pub fn from_env(workspace: PathBuf, max_steps: usize, auto_approve: bool) -> Result<Self> {
        let api_key = required_env("CODING_AGENT_API_KEY")?;
        let model = required_env("CODING_AGENT_MODEL")?;
        let base_url = env::var("CODING_AGENT_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_owned());
        let workspace = workspace
            .canonicalize()
            .map_err(|error| Error::Config(format!("invalid workspace: {error}")))?;

        if !workspace.is_dir() {
            return Err(Error::Config(format!(
                "workspace is not a directory: {}",
                workspace.display()
            )));
        }

        Ok(Self {
            api_key,
            base_url,
            model,
            workspace,
            max_steps: max_steps.max(1),
            auto_approve,
        })
    }
}

fn required_env(name: &str) -> Result<String> {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| Error::Config(format!("missing environment variable {name}")))
}
