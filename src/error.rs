#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("LLM error: {0}")]
    Llm(String),
    #[error("tool error: {0}")]
    Tool(String),
    #[error("agent stopped: {0}")]
    Agent(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
