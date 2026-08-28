use async_trait::async_trait;
use futures_util::StreamExt as _;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

use crate::error::{Error, Result};

mod stream;

use stream::{SseDecoder, StreamAccumulator};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
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
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: Some(call_id.into()),
        }
    }

    fn plain(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: Some(content.into()),
            reasoning_content: None,
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

    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        on_delta: DeltaHandler,
    ) -> Result<Message> {
        let message = self.complete(messages, tools).await?;
        if let Some(content) = message.content.as_deref().filter(|text| !text.is_empty()) {
            on_delta(content);
        }
        Ok(message)
    }
}

pub type DeltaHandler = Arc<dyn Fn(&str) + Send + Sync>;

pub struct HttpLanguageModel {
    client: Client,
    endpoint: String,
    api_key: String,
    model: String,
    deepseek_thinking: bool,
}

impl HttpLanguageModel {
    pub fn new(base_url: &str, api_key: String, model: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| Error::Llm(format!("failed to build HTTP client: {error}")))?;

        let endpoint = chat_completions_endpoint(base_url);
        Ok(Self {
            client,
            deepseek_thinking: endpoint.contains("api.deepseek.com"),
            endpoint,
            api_key,
            model,
        })
    }

    async fn complete_with_single_delta(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        on_delta: &DeltaHandler,
    ) -> Result<Message> {
        let message = <Self as LanguageModel>::complete(self, messages, tools).await?;
        if let Some(content) = message.content.as_deref().filter(|text| !text.is_empty()) {
            on_delta(content);
        }
        Ok(message)
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    tools: &'a [ToolDefinition],
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Clone, Copy, Serialize)]
struct ThinkingConfig {
    #[serde(rename = "type")]
    kind: &'static str,
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
                thinking: self
                    .deepseek_thinking
                    .then_some(ThinkingConfig { kind: "enabled" }),
                reasoning_effort: self.deepseek_thinking.then_some("high"),
                stream: None,
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

        parse_chat_response(&body)
    }

    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        on_delta: DeltaHandler,
    ) -> Result<Message> {
        let response = match self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&ChatRequest {
                model: &self.model,
                messages,
                tools,
                thinking: self
                    .deepseek_thinking
                    .then_some(ThinkingConfig { kind: "enabled" }),
                reasoning_effort: self.deepseek_thinking.then_some("high"),
                stream: Some(true),
            })
            .send()
            .await
        {
            Ok(response) => response,
            Err(_) => {
                return self
                    .complete_with_single_delta(messages, tools, &on_delta)
                    .await;
            }
        };

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .map_err(|error| Error::Llm(format!("failed to read response: {error}")))?;
            let message = serde_json::from_str::<ApiErrorEnvelope>(&body)
                .ok()
                .and_then(|envelope| envelope.error)
                .and_then(|error| error.message)
                .unwrap_or_else(|| truncate(&body, 1_000));
            if matches!(status.as_u16(), 400 | 415 | 422) {
                return self
                    .complete_with_single_delta(messages, tools, &on_delta)
                    .await;
            }
            return Err(Error::Llm(format!("HTTP {status}: {message}")));
        }

        let mut byte_stream = response.bytes_stream();
        let mut decoder = SseDecoder::default();
        let mut accumulator = StreamAccumulator::default();
        let mut emitted_text = false;

        while let Some(chunk) = byte_stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(_error) if !emitted_text => {
                    return self
                        .complete_with_single_delta(messages, tools, &on_delta)
                        .await;
                }
                Err(error) => {
                    return Err(Error::Llm(format!(
                        "failed to read streaming response after text was emitted: {error}"
                    )));
                }
            };
            let events = match decoder.push(&chunk) {
                Ok(events) => events,
                Err(_) if !emitted_text => {
                    return self
                        .complete_with_single_delta(messages, tools, &on_delta)
                        .await;
                }
                Err(error) => return Err(error),
            };
            for data in events {
                if data == "[DONE]" {
                    continue;
                }
                match accumulator.ingest_json(&data) {
                    Ok(Some(delta)) => {
                        emitted_text = true;
                        on_delta(&delta);
                    }
                    Ok(None) => {}
                    Err(_) if !emitted_text => {
                        return self
                            .complete_with_single_delta(messages, tools, &on_delta)
                            .await;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        let final_events = match decoder.finish() {
            Ok(events) => events,
            Err(_) if !emitted_text => {
                return self
                    .complete_with_single_delta(messages, tools, &on_delta)
                    .await;
            }
            Err(error) => return Err(error),
        };
        for data in final_events {
            if data != "[DONE]" {
                match accumulator.ingest_json(&data) {
                    Ok(Some(delta)) => {
                        emitted_text = true;
                        on_delta(&delta);
                    }
                    Ok(None) => {}
                    Err(_) if !emitted_text => {
                        return self
                            .complete_with_single_delta(messages, tools, &on_delta)
                            .await;
                    }
                    Err(error) => return Err(error),
                }
            }
        }

        match accumulator.finish() {
            Ok(message) => Ok(message),
            Err(_) if !emitted_text => {
                self.complete_with_single_delta(messages, tools, &on_delta)
                    .await
            }
            Err(error) => Err(Error::Llm(format!(
                "streaming response could not be assembled after text was emitted: {error}"
            ))),
        }
    }
}

fn parse_chat_response(body: &str) -> Result<Message> {
    let mut payload: ChatResponse = serde_json::from_str(body)
        .map_err(|error| Error::Llm(format!("invalid response JSON: {error}")))?;
    payload
        .choices
        .drain(..)
        .next()
        .map(|choice| choice.message)
        .ok_or_else(|| Error::Llm("response contained no choices".to_owned()))
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
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;

    use super::{
        DeltaHandler, HttpLanguageModel, LanguageModel, Message, chat_completions_endpoint,
        parse_chat_response,
    };

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

    #[test]
    fn parses_plain_text_and_rejects_malformed_or_empty_responses() {
        let plain = parse_chat_response(
            r#"{"choices":[{"message":{"role":"assistant","content":"hello"}}]}"#,
        )
        .expect("plain response");
        assert_eq!(plain.content.as_deref(), Some("hello"));
        assert!(parse_chat_response(r#"{"choices":[]}"#).is_err());
        assert!(parse_chat_response(r#"{"unexpected":true}"#).is_err());
        assert!(parse_chat_response("not json").is_err());
    }

    #[test]
    fn parses_one_and_multiple_tool_calls_with_optional_content() {
        let one = parse_chat_response(
            r#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a.py\"}"}}]}}]}"#,
        )
        .expect("one tool call");
        assert_eq!(one.tool_calls.expect("call").len(), 1);

        let mixed = parse_chat_response(
            r#"{"choices":[{"message":{"role":"assistant","content":"working","tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{}"}},{"id":"call-2","type":"function","function":{"name":"list_files","arguments":"{}"}}]}}]}"#,
        )
        .expect("mixed response");
        assert_eq!(mixed.content.as_deref(), Some("working"));
        assert_eq!(mixed.tool_calls.expect("calls").len(), 2);
    }

    #[tokio::test]
    async fn streams_text_deltas_and_returns_the_aggregated_message() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("server address");
        let (request_tx, request_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept request");
            let mut request = vec![0_u8; 16_384];
            let length = socket.read(&mut request).expect("read request");
            request.truncate(length);
            request_tx
                .send(String::from_utf8_lossy(&request).into_owned())
                .expect("send captured request");
            let body = concat!(
                "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n"
            );
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .expect("write streaming response");
        });
        let model = HttpLanguageModel::new(
            &format!("http://{address}/v1"),
            "test-key".to_owned(),
            "test-model".to_owned(),
        )
        .expect("model");
        let streamed = Arc::new(Mutex::new(String::new()));
        let streamed_copy = Arc::clone(&streamed);
        let handler: DeltaHandler = Arc::new(move |delta| {
            streamed_copy.lock().expect("stream lock").push_str(delta);
        });

        let message = model
            .complete_stream(&[Message::user("hello")], &[], handler)
            .await
            .expect("stream completion");

        server.join().expect("server thread");
        let request = request_rx.recv().expect("captured request");
        assert!(request.contains("\"stream\":true"));
        assert_eq!(streamed.lock().expect("stream lock").as_str(), "Hello");
        assert_eq!(message.content.as_deref(), Some("Hello"));
        assert!(message.tool_calls.is_none());
    }
}
