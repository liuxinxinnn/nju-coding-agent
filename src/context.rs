use std::fmt::Write as _;

use crate::error::{Error, Result};
use crate::llm::{Message, Role, ToolDefinition};

pub const DEFAULT_CONTEXT_WINDOW_TOKENS: u64 = 128_000;
const DEFAULT_TRIGGER_PERCENT: u64 = 80;
const DEFAULT_TARGET_PERCENT: u64 = 60;
const MESSAGE_OVERHEAD_TOKENS: u64 = 4;
const SUMMARY_MAX_CHARS: usize = 8_000;
const MESSAGE_PREVIEW_MAX_CHARS: usize = 160;
const SUMMARY_PREFIX: &str = "Earlier conversation summary (local deterministic fallback):";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextUsage {
    pub window_tokens: u64,
    pub used_tokens: u64,
    pub free_tokens: u64,
}

impl ContextUsage {
    pub fn used_percent(self) -> u64 {
        self.used_tokens
            .saturating_mul(100)
            .checked_div(self.window_tokens)
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressionEvent {
    pub covered_messages: usize,
    pub retained_messages: usize,
    pub before_tokens: u64,
    pub after_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct ContextManager {
    window_tokens: u64,
    trigger_percent: u64,
    target_percent: u64,
}

impl ContextManager {
    pub fn new(window_tokens: u64) -> Self {
        Self {
            window_tokens: window_tokens.max(1),
            trigger_percent: DEFAULT_TRIGGER_PERCENT,
            target_percent: DEFAULT_TARGET_PERCENT,
        }
    }

    #[cfg(test)]
    fn with_policy(window_tokens: u64, trigger_percent: u64, target_percent: u64) -> Self {
        Self {
            window_tokens: window_tokens.max(1),
            trigger_percent: trigger_percent.min(100),
            target_percent: target_percent.min(trigger_percent).min(100),
        }
    }

    pub fn usage(&self, messages: &[Message], tools: &[ToolDefinition]) -> ContextUsage {
        let message_tokens = messages.iter().fold(0_u64, |total, message| {
            total.saturating_add(estimate_message_tokens(message))
        });
        let tool_tokens = serde_json::to_string(tools)
            .map(|json| estimate_text_tokens(&json))
            .unwrap_or(0);
        let used_tokens = message_tokens.saturating_add(tool_tokens);
        ContextUsage {
            window_tokens: self.window_tokens,
            used_tokens,
            free_tokens: self.window_tokens.saturating_sub(used_tokens),
        }
    }

    pub fn compact_if_needed(
        &self,
        messages: &mut Vec<Message>,
        tools: &[ToolDefinition],
    ) -> Result<Option<CompressionEvent>> {
        let before = self.usage(messages, tools);
        if before.used_percent() < self.trigger_percent {
            return Ok(None);
        }
        if messages.len() <= 2 {
            return Err(Error::Agent(format!(
                "context is too large (estimated {} tokens, window {})",
                before.used_tokens, before.window_tokens
            )));
        }

        let target_tokens = self
            .window_tokens
            .saturating_mul(self.target_percent)
            .checked_div(100)
            .unwrap_or(0);
        let system = messages
            .first()
            .cloned()
            .ok_or_else(|| Error::Agent("conversation has no system message".to_owned()))?;
        let tool_tokens = self.usage(&[], tools).used_tokens;
        let mut retained_tokens = estimate_message_tokens(&system).saturating_add(tool_tokens);
        let mut keep_start = messages.len();

        for index in (1..messages.len()).rev() {
            let candidate = estimate_message_tokens(&messages[index]);
            if keep_start < messages.len()
                && retained_tokens.saturating_add(candidate) > target_tokens
            {
                break;
            }
            retained_tokens = retained_tokens.saturating_add(candidate);
            keep_start = index;
        }

        // Keep a complete recent user turn so an assistant tool call is never detached
        // from the request that caused it.
        while keep_start > 1 && !matches!(messages[keep_start].role, Role::User) {
            keep_start -= 1;
        }

        if keep_start <= 1 {
            return Err(Error::Agent(format!(
                "context cannot be compressed safely (estimated {} tokens, window {})",
                before.used_tokens, before.window_tokens
            )));
        }

        let summary = summarize_messages(&messages[1..keep_start]);
        let covered_messages = keep_start - 1;
        let retained = messages[keep_start..].to_vec();
        let retained_messages = retained.len();
        let mut compressed = Vec::with_capacity(retained_messages + 2);
        compressed.push(system);
        compressed.push(Message::system(format!("{SUMMARY_PREFIX}\n\n{summary}")));
        compressed.extend(retained);

        let after = self.usage(&compressed, tools);
        if after.used_tokens >= self.window_tokens {
            return Err(Error::Agent(format!(
                "context remains too large after compression (estimated {} tokens, window {})",
                after.used_tokens, after.window_tokens
            )));
        }

        *messages = compressed;
        Ok(Some(CompressionEvent {
            covered_messages,
            retained_messages,
            before_tokens: before.used_tokens,
            after_tokens: after.used_tokens,
        }))
    }
}

impl Default for ContextManager {
    fn default() -> Self {
        Self::new(DEFAULT_CONTEXT_WINDOW_TOKENS)
    }
}

fn estimate_message_tokens(message: &Message) -> u64 {
    let content_tokens = message
        .content
        .as_deref()
        .map(estimate_text_tokens)
        .unwrap_or(0);
    let call_tokens = message
        .tool_calls
        .as_ref()
        .map(|calls| {
            calls.iter().fold(0_u64, |total, call| {
                total
                    .saturating_add(estimate_text_tokens(&call.id))
                    .saturating_add(estimate_text_tokens(&call.function.name))
                    .saturating_add(estimate_text_tokens(&call.function.arguments))
            })
        })
        .unwrap_or(0);
    let call_id_tokens = message
        .tool_call_id
        .as_deref()
        .map(estimate_text_tokens)
        .unwrap_or(0);
    MESSAGE_OVERHEAD_TOKENS
        .saturating_add(content_tokens)
        .saturating_add(call_tokens)
        .saturating_add(call_id_tokens)
}

fn estimate_text_tokens(text: &str) -> u64 {
    let (ascii, non_ascii) = text.chars().fold((0_u64, 0_u64), |counts, character| {
        if character.is_ascii() {
            (counts.0.saturating_add(1), counts.1)
        } else {
            (counts.0, counts.1.saturating_add(1))
        }
    });
    ascii.div_ceil(4).saturating_add(non_ascii)
}

fn summarize_messages(messages: &[Message]) -> String {
    let mut summary = String::from("# Progress summary\n");
    let _ = writeln!(summary, "- Compressed messages: {}", messages.len());
    for message in messages {
        let role = match message.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        let content = message.content.as_deref().unwrap_or_default();
        let mut preview = compact_text(content);
        if preview.is_empty()
            && let Some(calls) = &message.tool_calls
        {
            preview = calls
                .iter()
                .map(|call| format!("{}({})", call.function.name, call.function.arguments))
                .collect::<Vec<_>>()
                .join(", ");
        }
        if preview.is_empty() {
            continue;
        }
        let preview = truncate_chars(&preview, MESSAGE_PREVIEW_MAX_CHARS);
        let _ = writeln!(summary, "- {role}: {preview}");
    }
    truncate_chars(&summary, SUMMARY_MAX_CHARS)
}

fn compact_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut value = text
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    value.push_str("...");
    value
}

#[cfg(test)]
mod tests {
    use super::ContextManager;
    use crate::llm::{Message, Role};

    #[test]
    fn estimates_chinese_more_conservatively_than_ascii() {
        let manager = ContextManager::new(1_000);
        let ascii = manager.usage(&[Message::user("1234")], &[]);
        let chinese = manager.usage(&[Message::user("测试文本")], &[]);
        assert!(chinese.used_tokens > ascii.used_tokens);
    }

    #[test]
    fn compacts_old_history_and_keeps_latest_user_turn() {
        let manager = ContextManager::with_policy(250, 40, 25);
        let mut messages = vec![
            Message::system("system"),
            Message::user("old request ".repeat(20)),
            Message::assistant("old answer ".repeat(20)),
            Message::user("latest request"),
            Message::assistant("latest progress"),
        ];

        let event = manager
            .compact_if_needed(&mut messages, &[])
            .expect("compression")
            .expect("event");

        assert!(event.covered_messages >= 2);
        assert!(event.after_tokens < event.before_tokens);
        assert!(messages.iter().any(|message| {
            message.role == Role::User && message.content.as_deref() == Some("latest request")
        }));
        assert!(messages.get(1).is_some_and(|message| {
            message.role == Role::System
                && message
                    .content
                    .as_deref()
                    .is_some_and(|content| content.contains("Progress summary"))
        }));
    }
}
