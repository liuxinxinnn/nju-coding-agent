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
    pub context_window_tokens: u64,
    pub auto_approve: bool,
}

impl Config {
    pub fn from_env(workspace: PathBuf, max_steps: usize, auto_approve: bool) -> Result<Self> {
        let generic_key = non_empty_env("CODING_AGENT_API_KEY");
        let deepseek_key = non_empty_env("DEEPSEEK_API_KEY");
        let using_deepseek_defaults = generic_key.is_none() && deepseek_key.is_some();
        let api_key = generic_key.or(deepseek_key).ok_or_else(|| {
            Error::Config(
                "missing CODING_AGENT_API_KEY or DEEPSEEK_API_KEY environment variable".to_owned(),
            )
        })?;
        let base_url = non_empty_env("CODING_AGENT_BASE_URL")
            .or_else(|| non_empty_env("DEEPSEEK_BASE_URL"))
            .unwrap_or_else(|| {
                if using_deepseek_defaults {
                    "https://api.deepseek.com".to_owned()
                } else {
                    "https://api.openai.com/v1".to_owned()
                }
            });
        let model = non_empty_env("CODING_AGENT_MODEL")
            .or_else(|| non_empty_env("DEEPSEEK_MODEL"))
            .or_else(|| using_deepseek_defaults.then(|| "deepseek-v4-flash".to_owned()))
            .ok_or_else(|| {
                Error::Config(
                    "missing CODING_AGENT_MODEL (DeepSeek defaults to deepseek-v4-flash)"
                        .to_owned(),
                )
            })?;
        let context_window_tokens = env::var("CODING_AGENT_CONTEXT_WINDOW")
            .ok()
            .map(|value| {
                value.parse::<u64>().map_err(|error| {
                    Error::Config(format!(
                        "invalid CODING_AGENT_CONTEXT_WINDOW '{value}': {error}"
                    ))
                })
            })
            .transpose()?
            .unwrap_or(if using_deepseek_defaults {
                1_000_000
            } else {
                crate::context::DEFAULT_CONTEXT_WINDOW_TOKENS
            })
            .max(1);
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
            context_window_tokens,
            auto_approve,
        })
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}
