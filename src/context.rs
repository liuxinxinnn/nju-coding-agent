use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::llm::{Message, Role, TokenUsage, ToolDefinition};

pub const DEFAULT_CONTEXT_WINDOW_TOKENS: u64 = 128_000;
pub const DEFAULT_TRIGGER_PERCENT: u64 = 80;
pub const DEFAULT_TARGET_PERCENT: u64 = 60;
const MESSAGE_OVERHEAD_TOKENS: u64 = 4;
const CALIBRATION_BASE: u64 = 1_000;
const MIN_CALIBRATION: u64 = 500;
const MAX_CALIBRATION: u64 = 4_000;
const LARGE_TOOL_RESULT_CHARS: usize = 4_000;
const LARGE_TOOL_PREVIEW_CHARS: usize = 800;
const OLD_TOOL_PREVIEW_CHARS: usize = 120;
const SUMMARY_MAX_CHARS: usize = 8_000;
const MESSAGE_PREVIEW_MAX_CHARS: usize = 160;
const SMART_SUMMARY_PREFIX: &str = "Earlier conversation summary (model-generated):";
const FALLBACK_SUMMARY_PREFIX: &str =
    "Earlier conversation summary (local deterministic fallback):";

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiUsage {
    pub requests: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub last_prompt_tokens: u64,
    pub last_completion_tokens: u64,
    pub last_total_tokens: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextState {
    #[serde(default = "default_calibration")]
    pub calibration_millis: u64,
    #[serde(default)]
    pub api_usage: ApiUsage,
}

impl Default for ContextState {
    fn default() -> Self {
        Self {
            calibration_millis: CALIBRATION_BASE,
            api_usage: ApiUsage::default(),
        }
    }
}

const fn default_calibration() -> u64 {
    CALIBRATION_BASE
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextUsage {
    pub window_tokens: u64,
    pub used_tokens: u64,
    pub free_tokens: u64,
    pub system_tokens: u64,
    pub tool_tokens: u64,
    pub message_tokens: u64,
    pub calibration_millis: u64,
    pub api_usage: ApiUsage,
}

impl ContextUsage {
    pub fn used_percent(self) -> u64 {
        self.used_tokens
            .saturating_mul(100)
            .checked_div(self.window_tokens)
            .unwrap_or(0)
    }

    pub fn empty(window_tokens: u64) -> Self {
        let window_tokens = window_tokens.max(1);
        Self {
            window_tokens,
            used_tokens: 0,
            free_tokens: window_tokens,
            system_tokens: 0,
            tool_tokens: 0,
            message_tokens: 0,
            calibration_millis: CALIBRATION_BASE,
            api_usage: ApiUsage::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressionEvent {
    pub covered_messages: usize,
    pub retained_messages: usize,
    pub before_tokens: u64,
    pub after_tokens: u64,
    pub stages: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SummaryRequest {
    keep_start: usize,
    before_tokens: u64,
    stages: Vec<String>,
    transcript: String,
}

#[derive(Debug, Clone)]
pub enum CompressionPreparation {
    None,
    Complete(CompressionEvent),
    NeedsSummary(SummaryRequest),
}

#[derive(Debug, Clone)]
pub struct ContextManager {
    window_tokens: u64,
    trigger_percent: u64,
    target_percent: u64,
    state: ContextState,
}

impl ContextManager {
    pub fn new(window_tokens: u64) -> Self {
        Self {
            window_tokens: window_tokens.max(1),
            trigger_percent: DEFAULT_TRIGGER_PERCENT,
            target_percent: DEFAULT_TARGET_PERCENT,
            state: ContextState::default(),
        }
    }

    #[cfg(test)]
    fn with_policy(window_tokens: u64, trigger_percent: u64, target_percent: u64) -> Self {
        Self {
            window_tokens: window_tokens.max(1),
            trigger_percent: trigger_percent.min(100),
            target_percent: target_percent.min(trigger_percent).min(100),
            state: ContextState::default(),
        }
    }

    pub const fn state(&self) -> ContextState {
        self.state
    }

    pub fn restore_state(&mut self, mut state: ContextState) {
        state.calibration_millis = state
            .calibration_millis
            .clamp(MIN_CALIBRATION, MAX_CALIBRATION);
        self.state = state;
    }

    pub fn record_api_usage(
        &mut self,
        messages: &[Message],
        tools: &[ToolDefinition],
        usage: TokenUsage,
    ) {
        let raw_estimate = raw_usage(messages, tools).used_tokens.max(1);
        let observed_ratio = usage
            .prompt_tokens
            .saturating_mul(CALIBRATION_BASE)
            .checked_div(raw_estimate)
            .unwrap_or(CALIBRATION_BASE)
            .clamp(MIN_CALIBRATION, MAX_CALIBRATION);
        self.state.calibration_millis = if self.state.api_usage.requests == 0 {
            observed_ratio
        } else {
            self.state
                .calibration_millis
                .saturating_mul(3)
                .saturating_add(observed_ratio)
                .checked_div(4)
                .unwrap_or(observed_ratio)
                .clamp(MIN_CALIBRATION, MAX_CALIBRATION)
        };
        let totals = &mut self.state.api_usage;
        totals.requests = totals.requests.saturating_add(1);
        totals.prompt_tokens = totals.prompt_tokens.saturating_add(usage.prompt_tokens);
        totals.completion_tokens = totals
            .completion_tokens
            .saturating_add(usage.completion_tokens);
        totals.total_tokens = totals.total_tokens.saturating_add(usage.total_tokens);
        totals.last_prompt_tokens = usage.prompt_tokens;
        totals.last_completion_tokens = usage.completion_tokens;
        totals.last_total_tokens = usage.total_tokens;
    }

    pub fn usage(&self, messages: &[Message], tools: &[ToolDefinition]) -> ContextUsage {
        let raw = raw_usage(messages, tools);
        let system_tokens = scale_tokens(raw.system_tokens, self.state.calibration_millis);
        let tool_tokens = scale_tokens(raw.tool_tokens, self.state.calibration_millis);
        let message_tokens = scale_tokens(raw.message_tokens, self.state.calibration_millis);
        let used_tokens = system_tokens
            .saturating_add(tool_tokens)
            .saturating_add(message_tokens);
        ContextUsage {
            window_tokens: self.window_tokens,
            used_tokens,
            free_tokens: self.window_tokens.saturating_sub(used_tokens),
            system_tokens,
            tool_tokens,
            message_tokens,
            calibration_millis: self.state.calibration_millis,
            api_usage: self.state.api_usage,
        }
    }

    pub fn prepare_compaction(
        &self,
        messages: &mut Vec<Message>,
        tools: &[ToolDefinition],
    ) -> Result<CompressionPreparation> {
        let before = self.usage(messages, tools);
        if before.used_percent() < self.trigger_percent {
            return Ok(CompressionPreparation::None);
        }
        if messages.len() <= 2 {
            return Err(Error::Agent(format!(
                "context is too large (estimated {} tokens, window {})",
                before.used_tokens, before.window_tokens
            )));
        }

        let target_tokens = self.target_tokens();
        let mut stages = Vec::new();
        let mut keep_start = latest_user_turn_start(messages)?;

        if shrink_large_tool_results(messages, keep_start) {
            stages.push("large-results".to_owned());
            if self.usage(messages, tools).used_tokens <= target_tokens {
                return Ok(CompressionPreparation::Complete(self.event(
                    messages,
                    tools,
                    keep_start,
                    before.used_tokens,
                    stages,
                )));
            }
        }

        if drop_low_value_content(messages, keep_start) {
            stages.push("low-value-content".to_owned());
            keep_start = latest_user_turn_start(messages)?;
            if self.usage(messages, tools).used_tokens <= target_tokens {
                return Ok(CompressionPreparation::Complete(self.event(
                    messages,
                    tools,
                    keep_start,
                    before.used_tokens,
                    stages,
                )));
            }
        }

        if replace_old_tool_results(messages, keep_start) {
            stages.push("old-tool-placeholders".to_owned());
            if self.usage(messages, tools).used_tokens <= target_tokens {
                return Ok(CompressionPreparation::Complete(self.event(
                    messages,
                    tools,
                    keep_start,
                    before.used_tokens,
                    stages,
                )));
            }
        }

        if keep_start <= 1 {
            return Err(Error::Agent(format!(
                "context cannot be compressed safely (estimated {} tokens, window {})",
                before.used_tokens, before.window_tokens
            )));
        }
        let transcript = render_transcript(&messages[1..keep_start]);
        Ok(CompressionPreparation::NeedsSummary(SummaryRequest {
            keep_start,
            before_tokens: before.used_tokens,
            stages,
            transcript,
        }))
    }

    pub fn summary_messages(&self, request: &SummaryRequest) -> Vec<Message> {
        vec![
            Message::system(
                "Summarize an older coding-agent conversation for future continuation. Preserve user requirements, files and symbols changed, architecture decisions, exact commands and results, unresolved failures, and next steps. Omit greetings, repeated narration, and obsolete tool output. Do not invent facts. Return concise Markdown only.",
            ),
            Message::user(format!(
                "Create the continuation summary for this transcript:\n\n{}",
                request.transcript
            )),
        ]
    }

    pub fn finish_summary(
        &self,
        messages: &mut Vec<Message>,
        tools: &[ToolDefinition],
        mut request: SummaryRequest,
        smart_summary: Option<String>,
    ) -> Result<CompressionEvent> {
        let source = &messages[1..request.keep_start];
        let (prefix, summary, stage) = match smart_summary.filter(|text| !text.trim().is_empty()) {
            Some(summary) => (SMART_SUMMARY_PREFIX, summary, "model-summary"),
            None => (
                FALLBACK_SUMMARY_PREFIX,
                summarize_messages(source),
                "deterministic-summary",
            ),
        };
        request.stages.push(stage.to_owned());
        let system = messages[0].clone();
        let retained = messages[request.keep_start..].to_vec();
        let retained_messages = retained.len();
        let covered_messages = request.keep_start - 1;
        let mut compressed = Vec::with_capacity(retained_messages + 2);
        compressed.push(system);
        compressed.push(Message::system(format!("{prefix}\n\n{summary}")));
        compressed.extend(retained);

        let after = self.usage(&compressed, tools);
        if after.used_tokens >= self.window_tokens {
            return Err(Error::Agent(format!(
                "context remains too large after compression (estimated {} tokens, window {})",
                after.used_tokens, after.window_tokens
            )));
        }
        *messages = compressed;
        Ok(CompressionEvent {
            covered_messages,
            retained_messages,
            before_tokens: request.before_tokens,
            after_tokens: after.used_tokens,
            stages: request.stages,
        })
    }

    fn target_tokens(&self) -> u64 {
        self.window_tokens
            .saturating_mul(self.target_percent)
            .checked_div(100)
            .unwrap_or(0)
    }

    fn event(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        keep_start: usize,
        before_tokens: u64,
        stages: Vec<String>,
    ) -> CompressionEvent {
        CompressionEvent {
            covered_messages: keep_start.saturating_sub(1),
            retained_messages: messages.len().saturating_sub(keep_start),
            before_tokens,
            after_tokens: self.usage(messages, tools).used_tokens,
            stages,
        }
    }
}

impl Default for ContextManager {
    fn default() -> Self {
        Self::new(DEFAULT_CONTEXT_WINDOW_TOKENS)
    }
}

#[derive(Default)]
struct RawUsage {
    used_tokens: u64,
    system_tokens: u64,
    tool_tokens: u64,
    message_tokens: u64,
}

fn raw_usage(messages: &[Message], tools: &[ToolDefinition]) -> RawUsage {
    let mut usage = RawUsage {
        tool_tokens: serde_json::to_string(tools)
            .map(|json| estimate_text_tokens(&json))
            .unwrap_or(0),
        ..RawUsage::default()
    };
    for message in messages {
        let tokens = estimate_message_tokens(message);
        match message.role {
            Role::System => usage.system_tokens = usage.system_tokens.saturating_add(tokens),
            Role::Tool => usage.tool_tokens = usage.tool_tokens.saturating_add(tokens),
            Role::User | Role::Assistant => {
                usage.message_tokens = usage.message_tokens.saturating_add(tokens);
            }
        }
    }
    usage.used_tokens = usage
        .system_tokens
        .saturating_add(usage.tool_tokens)
        .saturating_add(usage.message_tokens);
    usage
}

fn scale_tokens(tokens: u64, calibration_millis: u64) -> u64 {
    tokens
        .saturating_mul(calibration_millis)
        .saturating_add(CALIBRATION_BASE - 1)
        .checked_div(CALIBRATION_BASE)
        .unwrap_or(tokens)
}

fn latest_user_turn_start(messages: &[Message]) -> Result<usize> {
    messages
        .iter()
        .rposition(|message| message.role == Role::User)
        .filter(|index| *index > 0)
        .ok_or_else(|| Error::Agent("context has no compressible user turn".to_owned()))
}

fn shrink_large_tool_results(messages: &mut [Message], keep_start: usize) -> bool {
    let mut changed = false;
    for message in &mut messages[1..keep_start] {
        if message.role != Role::Tool {
            continue;
        }
        let Some(content) = message.content.as_deref() else {
            continue;
        };
        let chars = content.chars().count();
        if chars <= LARGE_TOOL_RESULT_CHARS {
            continue;
        }
        message.content = Some(format!(
            "{}\n[large tool result compacted locally; original_chars={chars}]",
            truncate_chars(content, LARGE_TOOL_PREVIEW_CHARS)
        ));
        changed = true;
    }
    changed
}

fn drop_low_value_content(messages: &mut Vec<Message>, keep_start: usize) -> bool {
    let mut changed = false;
    for message in &mut messages[1..keep_start] {
        if message.reasoning_content.take().is_some() {
            changed = true;
        }
        if message.role == Role::Assistant
            && message
                .tool_calls
                .as_ref()
                .is_some_and(|calls| !calls.is_empty())
            && message.content.take().is_some()
        {
            changed = true;
        }
    }
    let mut index = 1;
    let mut boundary = keep_start.min(messages.len());
    while index < boundary {
        if messages[index].role == Role::System
            && !messages[index]
                .content
                .as_deref()
                .is_some_and(|content| content.starts_with("Earlier conversation summary"))
        {
            messages.remove(index);
            boundary = boundary.saturating_sub(1);
            changed = true;
        } else {
            index += 1;
        }
    }
    changed
}

fn replace_old_tool_results(messages: &mut [Message], keep_start: usize) -> bool {
    let mut changed = false;
    for message in &mut messages[1..keep_start] {
        if message.role != Role::Tool {
            continue;
        }
        let Some(content) = message.content.as_deref() else {
            continue;
        };
        let chars = content.chars().count();
        if chars <= OLD_TOOL_PREVIEW_CHARS {
            continue;
        }
        let call_id = message.tool_call_id.as_deref().unwrap_or("unknown");
        message.content = Some(format!(
            "[old tool result omitted; call_id={call_id}; original_chars={chars}]"
        ));
        changed = true;
    }
    changed
}

fn render_transcript(messages: &[Message]) -> String {
    let mut transcript = String::new();
    for message in messages {
        let role = match message.role {
            Role::System => "SYSTEM",
            Role::User => "USER",
            Role::Assistant => "ASSISTANT",
            Role::Tool => "TOOL",
        };
        let _ = writeln!(transcript, "## {role}");
        if let Some(content) = &message.content {
            let _ = writeln!(transcript, "{content}");
        }
        if let Some(calls) = &message.tool_calls {
            for call in calls {
                let _ = writeln!(
                    transcript,
                    "tool_call {} {} {}",
                    call.id, call.function.name, call.function.arguments
                );
            }
        }
        let _ = writeln!(transcript);
    }
    transcript
}

fn estimate_message_tokens(message: &Message) -> u64 {
    let content_tokens = message
        .content
        .as_deref()
        .map(estimate_text_tokens)
        .unwrap_or(0);
    let reasoning_tokens = message
        .reasoning_content
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
        .saturating_add(reasoning_tokens)
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
    use super::{CompressionPreparation, ContextManager, ContextState};
    use crate::llm::{FunctionCall, Message, Role, TokenUsage, ToolCall, ToolDefinition};
    use serde_json::json;

    #[test]
    fn estimates_chinese_more_conservatively_than_ascii() {
        let manager = ContextManager::new(1_000);
        let ascii = manager.usage(&[Message::user("1234")], &[]);
        let chinese = manager.usage(&[Message::user("测试文本")], &[]);
        assert!(chinese.used_tokens > ascii.used_tokens);
    }

    #[test]
    fn usage_breakdown_accounts_for_every_estimated_token() {
        let manager = ContextManager::new(10_000);
        let messages = vec![
            Message::system("system instructions"),
            Message::user("inspect the project"),
            Message::assistant("I will inspect it"),
            Message::tool("call-1", "file contents"),
        ];
        let tools = vec![ToolDefinition::function(
            "read_file".to_owned(),
            "read one file".to_owned(),
            json!({"type": "object"}),
        )];
        let usage = manager.usage(&messages, &tools);
        assert!(usage.system_tokens > 0);
        assert!(usage.tool_tokens > 0);
        assert!(usage.message_tokens > 0);
        assert_eq!(
            usage.used_tokens,
            usage.system_tokens + usage.tool_tokens + usage.message_tokens
        );
        assert_eq!(usage.free_tokens, usage.window_tokens - usage.used_tokens);
    }

    #[test]
    fn real_usage_calibrates_future_estimates_and_accumulates_cost() {
        let mut manager = ContextManager::new(10_000);
        let messages = vec![Message::system("system"), Message::user("12345678")];
        let before = manager.usage(&messages, &[]);
        manager.record_api_usage(
            &messages,
            &[],
            TokenUsage {
                prompt_tokens: before.used_tokens * 2,
                completion_tokens: 7,
                total_tokens: before.used_tokens * 2 + 7,
            },
        );
        let after = manager.usage(&messages, &[]);
        assert_eq!(after.calibration_millis, 2_000);
        assert_eq!(after.used_tokens, before.used_tokens * 2);
        assert_eq!(after.api_usage.requests, 1);
        assert_eq!(after.api_usage.completion_tokens, 7);
    }

    #[test]
    fn restores_out_of_range_calibration_safely() {
        let mut manager = ContextManager::new(1_000);
        manager.restore_state(ContextState {
            calibration_millis: 99_999,
            ..ContextState::default()
        });
        assert_eq!(manager.state().calibration_millis, 4_000);
    }

    #[test]
    fn cheap_stages_shrink_large_old_tool_results_before_summary() {
        let manager = ContextManager::with_policy(700, 40, 30);
        let mut messages = vec![
            Message::system("system"),
            Message::user("old request"),
            tool_call_message("old-call"),
            Message::tool("old-call", "large result ".repeat(500)),
            Message::assistant("old conclusion"),
            Message::user("latest request"),
            Message::assistant("latest progress"),
        ];
        let preparation = manager
            .prepare_compaction(&mut messages, &[])
            .expect("preparation");
        match preparation {
            CompressionPreparation::Complete(event) => {
                assert!(event.stages.contains(&"large-results".to_owned()));
                assert!(event.after_tokens < event.before_tokens);
            }
            CompressionPreparation::NeedsSummary(request) => {
                let event = manager
                    .finish_summary(
                        &mut messages,
                        &[],
                        request,
                        Some("smart summary".to_owned()),
                    )
                    .expect("summary");
                assert!(event.stages.contains(&"large-results".to_owned()));
                assert!(event.stages.contains(&"model-summary".to_owned()));
            }
            CompressionPreparation::None => panic!("expected compaction"),
        }
        assert!(messages.iter().any(|message| {
            message.role == Role::User && message.content.as_deref() == Some("latest request")
        }));
    }

    #[test]
    fn final_summary_keeps_latest_tool_call_and_result_atomic() {
        let manager = ContextManager::with_policy(350, 40, 25);
        let mut messages = vec![
            Message::system("system"),
            Message::user("old request ".repeat(80)),
            Message::assistant("old answer ".repeat(80)),
            Message::user("latest request"),
            tool_call_message("call-latest"),
            Message::tool("call-latest", "current file contents"),
            Message::assistant("latest conclusion"),
        ];
        let request = match manager
            .prepare_compaction(&mut messages, &[])
            .expect("preparation")
        {
            CompressionPreparation::NeedsSummary(request) => request,
            other => panic!("expected summary request, got {other:?}"),
        };
        manager
            .finish_summary(&mut messages, &[], request, None)
            .expect("compression");
        let retained_call = messages.iter().any(|message| {
            message
                .tool_calls
                .as_ref()
                .is_some_and(|calls| calls.iter().any(|call| call.id == "call-latest"))
        });
        let retained_result = messages.iter().any(|message| {
            message.role == Role::Tool && message.tool_call_id.as_deref() == Some("call-latest")
        });
        assert!(retained_call && retained_result);
        assert_eq!(messages[2].role, Role::User);
    }

    #[test]
    fn cheap_cleanup_never_removes_system_messages_from_latest_turn() {
        let manager = ContextManager::with_policy(350, 40, 25);
        let mut messages = vec![
            Message::system("base system"),
            Message::user("old request ".repeat(80)),
            Message::system("old temporary phase prompt ".repeat(20)),
            Message::assistant("old answer ".repeat(80)),
            Message::user("latest request"),
            Message::system("CURRENT EXECUTION PROMPT"),
            Message::assistant("latest progress"),
        ];

        let preparation = manager
            .prepare_compaction(&mut messages, &[])
            .expect("preparation");
        if let CompressionPreparation::NeedsSummary(request) = preparation {
            manager
                .finish_summary(&mut messages, &[], request, None)
                .expect("summary");
        }

        assert!(messages.iter().any(|message| {
            message.role == Role::System
                && message.content.as_deref() == Some("CURRENT EXECUTION PROMPT")
        }));
        assert!(!messages.iter().any(|message| {
            message.role == Role::System
                && message
                    .content
                    .as_deref()
                    .is_some_and(|content| content.starts_with("old temporary"))
        }));
    }

    fn tool_call_message(id: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: None,
            reasoning_content: None,
            tool_calls: Some(vec![ToolCall {
                id: id.to_owned(),
                kind: "function".to_owned(),
                function: FunctionCall {
                    name: "read_file".to_owned(),
                    arguments: r#"{"path":"src/main.rs"}"#.to_owned(),
                },
            }]),
            tool_call_id: None,
        }
    }
}
