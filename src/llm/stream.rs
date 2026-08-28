use std::collections::BTreeMap;

use serde::Deserialize;

use super::{FunctionCall, Message, Role, ToolCall};
use crate::error::{Error, Result};

#[derive(Default)]
pub(super) struct SseDecoder {
    buffer: Vec<u8>,
    data_lines: Vec<String>,
}

impl SseDecoder {
    pub(super) fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line = self.buffer.drain(..=newline).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8(line)
                .map_err(|error| Error::Llm(format!("stream contained invalid UTF-8: {error}")))?;
            self.ingest_line(&line, &mut events);
        }
        Ok(events)
    }

    pub(super) fn finish(mut self) -> Result<Vec<String>> {
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let line = String::from_utf8(std::mem::take(&mut self.buffer))
                .map_err(|error| Error::Llm(format!("stream contained invalid UTF-8: {error}")))?;
            self.ingest_line(line.trim_end_matches('\r'), &mut events);
        }
        self.flush(&mut events);
        Ok(events)
    }

    fn ingest_line(&mut self, line: &str, events: &mut Vec<String>) {
        if line.is_empty() {
            self.flush(events);
        } else if let Some(data) = line.strip_prefix("data:") {
            self.data_lines
                .push(data.strip_prefix(' ').unwrap_or(data).to_owned());
        }
    }

    fn flush(&mut self, events: &mut Vec<String>) {
        if !self.data_lines.is_empty() {
            events.push(self.data_lines.join("\n"));
            self.data_lines.clear();
        }
    }
}

#[derive(Debug, Deserialize)]
struct StreamEnvelope {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    error: Option<StreamApiError>,
}

#[derive(Debug, Deserialize)]
struct StreamApiError {
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Debug, Default, Deserialize)]
struct StreamDelta {
    content: Option<String>,
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<ToolCallChunk>>,
}

#[derive(Debug, Deserialize)]
struct ToolCallChunk {
    index: usize,
    id: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    function: Option<FunctionChunk>,
}

#[derive(Debug, Deserialize)]
struct FunctionChunk {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Default)]
struct ToolCallAccumulator {
    id: Option<String>,
    kind: Option<String>,
    name: String,
    arguments: String,
}

impl ToolCallAccumulator {
    fn ingest(&mut self, chunk: ToolCallChunk) {
        if let Some(id) = chunk.id {
            self.id = Some(id);
        }
        if let Some(kind) = chunk.kind {
            self.kind = Some(kind);
        }
        if let Some(function) = chunk.function {
            if let Some(name) = function.name {
                if name.starts_with(&self.name) {
                    self.name = name;
                } else if !self.name.starts_with(&name) {
                    self.name.push_str(&name);
                }
            }
            if let Some(arguments) = function.arguments {
                self.arguments.push_str(&arguments);
            }
        }
    }

    fn finish(self, index: usize) -> Result<ToolCall> {
        let id = self
            .id
            .ok_or_else(|| Error::Llm(format!("streamed tool call at index {index} has no id")))?;
        if self.name.is_empty() {
            return Err(Error::Llm(format!(
                "streamed tool call at index {index} has no function name"
            )));
        }
        Ok(ToolCall {
            id,
            kind: self.kind.unwrap_or_else(|| "function".to_owned()),
            function: FunctionCall {
                name: self.name,
                arguments: self.arguments,
            },
        })
    }
}

#[derive(Debug, Default)]
pub(super) struct StreamAccumulator {
    content: String,
    reasoning_content: String,
    tool_calls: BTreeMap<usize, ToolCallAccumulator>,
    received_choice: bool,
}

impl StreamAccumulator {
    pub(super) fn ingest_json(&mut self, data: &str) -> Result<Option<String>> {
        let envelope = serde_json::from_str::<StreamEnvelope>(data)
            .map_err(|error| Error::Llm(format!("invalid streaming response JSON: {error}")))?;
        if let Some(error) = envelope.error {
            return Err(Error::Llm(
                error
                    .message
                    .unwrap_or_else(|| "streaming API returned an error".to_owned()),
            ));
        }
        let Some(choice) = envelope.choices.into_iter().next() else {
            return Ok(None);
        };
        self.received_choice = true;
        let delta = choice.delta;
        if let Some(reasoning) = delta.reasoning_content {
            self.reasoning_content.push_str(&reasoning);
        }
        if let Some(chunks) = delta.tool_calls {
            for chunk in chunks {
                self.tool_calls
                    .entry(chunk.index)
                    .or_default()
                    .ingest(chunk);
            }
        }
        if let Some(content) = delta.content.filter(|content| !content.is_empty()) {
            self.content.push_str(&content);
            return Ok(Some(content));
        }
        Ok(None)
    }

    pub(super) fn finish(self) -> Result<Message> {
        if !self.received_choice {
            return Err(Error::Llm(
                "streaming response contained no choices".to_owned(),
            ));
        }
        let content = (!self.content.is_empty()).then_some(self.content);
        let reasoning_content =
            (!self.reasoning_content.is_empty()).then_some(self.reasoning_content);
        let tool_calls = if self.tool_calls.is_empty() {
            None
        } else {
            Some(
                self.tool_calls
                    .into_iter()
                    .map(|(index, call)| call.finish(index))
                    .collect::<Result<Vec<_>>>()?,
            )
        };
        Ok(Message {
            role: Role::Assistant,
            content,
            reasoning_content,
            tool_calls,
            tool_call_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{SseDecoder, StreamAccumulator};

    #[test]
    fn decodes_events_split_across_byte_chunks() {
        let mut decoder = SseDecoder::default();
        assert!(
            decoder
                .push(b"data: {\"cho")
                .expect("first chunk")
                .is_empty()
        );
        let events = decoder
            .push(b"ices\":[]}\r\n\r\ndata: [DONE]\n\n")
            .expect("second chunk");

        assert_eq!(events, vec![r#"{"choices":[]}"#, "[DONE]"]);
    }

    #[test]
    fn aggregates_text_reasoning_and_fragmented_tool_calls() {
        let mut accumulator = StreamAccumulator::default();
        let first = accumulator
            .ingest_json(
                r#"{"choices":[{"delta":{"reasoning_content":"think","content":"Hel","tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":"read_","arguments":"{\"pa"}}]}}]}"#,
            )
            .expect("first delta");
        let second = accumulator
            .ingest_json(
                r#"{"choices":[{"delta":{"content":"lo","tool_calls":[{"index":0,"function":{"name":"file","arguments":"th\":\"a.py\"}"}}]}}]}"#,
            )
            .expect("second delta");

        let message = accumulator.finish().expect("complete message");

        assert_eq!(first.as_deref(), Some("Hel"));
        assert_eq!(second.as_deref(), Some("lo"));
        assert_eq!(message.content.as_deref(), Some("Hello"));
        assert_eq!(message.reasoning_content.as_deref(), Some("think"));
        let call = &message.tool_calls.expect("tool calls")[0];
        assert_eq!(call.id, "call-1");
        assert_eq!(call.function.name, "read_file");
        assert_eq!(call.function.arguments, r#"{"path":"a.py"}"#);
    }

    #[test]
    fn aggregates_multiple_tool_calls_by_index() {
        let mut accumulator = StreamAccumulator::default();
        accumulator
            .ingest_json(
                r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call-2","type":"function","function":{"name":"list_files","arguments":"{}"}},{"index":0,"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a.py\"}"}}]}}]}"#,
            )
            .expect("tool delta");

        let calls = accumulator
            .finish()
            .expect("message")
            .tool_calls
            .expect("calls");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id, "call-1");
        assert_eq!(calls[1].id, "call-2");
    }

    #[test]
    fn rejects_empty_choices_api_errors_and_incomplete_tool_calls() {
        let mut empty = StreamAccumulator::default();
        assert!(empty.ingest_json(r#"{"choices":[]}"#).is_ok());
        assert!(empty.finish().is_err());

        let mut api_error = StreamAccumulator::default();
        assert!(
            api_error
                .ingest_json(r#"{"choices":[],"error":{"message":"bad request"}}"#)
                .is_err()
        );

        let mut incomplete = StreamAccumulator::default();
        incomplete
            .ingest_json(
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{}"}}]}}]}"#,
            )
            .expect("delta");
        assert!(incomplete.finish().is_err());
    }
}
