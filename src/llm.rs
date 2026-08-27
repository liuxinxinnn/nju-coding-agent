use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self::plain(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::plain(Role::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::plain(Role::Assistant, content)
    }

    pub fn tool(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(call_id.into()),
        }
    }

    fn plain(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl ToolDefinition {
    pub fn function(name: String, description: String, parameters: Value) -> Self {
        Self {
            kind: "function",
            function: FunctionDefinition {
                name,
                description,
                parameters,
            },
        }
    }
}

#[async_trait]
pub trait LanguageModel: Send + Sync {
    async fn complete(&self, messages: &[Message], tools: &[ToolDefinition]) -> Result<Message>;
}

pub struct HttpLanguageModel {
    client: Client,
    endpoint: String,
    api_key: String,
    model: String,
}

impl HttpLanguageModel {
    pub fn new(base_url: &str, api_key: String, model: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| Error::Llm(format!("failed to build HTTP client: {error}")))?;

        Ok(Self {
            client,
            endpoint: chat_completions_endpoint(base_url),
            api_key,
            model,
        })
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    tools: &'a [ToolDefinition],
    tool_choice: &'static str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Deserialize)]
struct ApiErrorEnvelope {
    error: Option<ApiError>,
}

#[derive(Deserialize)]
struct ApiError {
    message: Option<String>,
}

#[async_trait]
impl LanguageModel for HttpLanguageModel {
    async fn complete(&self, messages: &[Message], tools: &[ToolDefinition]) -> Result<Message> {
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&ChatRequest {
                model: &self.model,
                messages,
                tools,
                tool_choice: "auto",
            })
            .send()
            .await
            .map_err(|error| Error::Llm(format!("request failed: {error}")))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| Error::Llm(format!("failed to read response: {error}")))?;

        if !status.is_success() {
            let message = serde_json::from_str::<ApiErrorEnvelope>(&body)
                .ok()
                .and_then(|envelope| envelope.error)
                .and_then(|error| error.message)
                .unwrap_or_else(|| truncate(&body, 1_000));
            return Err(Error::Llm(format!("HTTP {status}: {message}")));
        }

        let mut payload: ChatResponse = serde_json::from_str(&body)
            .map_err(|error| Error::Llm(format!("invalid response JSON: {error}")))?;
        payload
            .choices
            .drain(..)
            .next()
            .map(|choice| choice.message)
            .ok_or_else(|| Error::Llm("response contained no choices".to_owned()))
    }
}

fn chat_completions_endpoint(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_owned()
    } else {
        format!("{base}/chat/completions")
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::chat_completions_endpoint;

    #[test]
    fn builds_chat_endpoint() {
        assert_eq!(
            chat_completions_endpoint("https://example.test/v1/"),
            "https://example.test/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_endpoint("https://example.test/v1/chat/completions"),
            "https://example.test/v1/chat/completions"
        );
    }
}
