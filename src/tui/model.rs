use chrono::Local;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MessageRole {
    User,
    Agent,
    Plan,
    System,
    Error,
}

impl MessageRole {
    fn label(self) -> &'static str {
        match self {
            Self::User => "USER",
            Self::Agent => "AGENT",
            Self::Plan => "PLAN",
            Self::System => "SYSTEM",
            Self::Error => "ERROR",
        }
    }

    fn style(self) -> Style {
        match self {
            Self::User => Style::default().fg(Color::Black).bg(Color::LightGreen),
            Self::Agent => Style::default().fg(Color::Black).bg(Color::LightBlue),
            Self::Plan => Style::default().fg(Color::Black).bg(Color::LightYellow),
            Self::System => Style::default().fg(Color::Black).bg(Color::Gray),
            Self::Error => Style::default().fg(Color::White).bg(Color::Red),
        }
        .add_modifier(Modifier::BOLD)
    }
}

pub(super) struct ChatMessage {
    timestamp: String,
    role: MessageRole,
    content: String,
}

impl ChatMessage {
    pub(super) fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            timestamp: Local::now().format("%H:%M:%S").to_string(),
            role,
            content: content.into(),
        }
    }

    pub(super) fn render_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from(vec![
            Span::styled(self.timestamp.clone(), Style::default().fg(Color::DarkGray)),
            Span::raw("  "),
            Span::styled(format!(" {:^7} ", self.role.label()), self.role.style()),
        ])];
        let mut code_block = false;
        for text in self.content.lines() {
            if text.trim_start().starts_with("```") {
                code_block = !code_block;
                lines.push(Line::styled(
                    text.to_owned(),
                    Style::default().fg(Color::LightCyan),
                ));
            } else if code_block {
                lines.push(Line::styled(
                    text.to_owned(),
                    Style::default().fg(Color::LightCyan),
                ));
            } else if text.starts_with('#') {
                lines.push(Line::styled(
                    text.to_owned(),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                lines.push(Line::from(text.to_owned()));
            }
        }
        lines.push(Line::from(String::new()));
        lines
    }
}

#[derive(Clone, Copy)]
pub(super) enum EventKind {
    Plan,
    Execute,
    Verify,
    Done,
    State,
    Result,
    Context,
    Info,
    Error,
}

impl EventKind {
    fn label(self) -> &'static str {
        match self {
            Self::Plan => "PLAN",
            Self::Execute => "EXEC",
            Self::Verify => "VERIFY",
            Self::Done => "DONE",
            Self::State => "STATE",
            Self::Result => "RESULT",
            Self::Context => "CTX",
            Self::Info => "INFO",
            Self::Error => "ERROR",
        }
    }

    fn style(self) -> Style {
        match self {
            Self::Plan => Style::default().fg(Color::Black).bg(Color::LightYellow),
            Self::Execute => Style::default().fg(Color::Black).bg(Color::LightBlue),
            Self::Verify => Style::default().fg(Color::Black).bg(Color::LightMagenta),
            Self::Done => Style::default().fg(Color::Black).bg(Color::LightGreen),
            Self::State => Style::default().fg(Color::Black).bg(Color::Magenta),
            Self::Result => Style::default().fg(Color::Black).bg(Color::Gray),
            Self::Context => Style::default().fg(Color::Black).bg(Color::Cyan),
            Self::Info => Style::default().fg(Color::Black).bg(Color::LightCyan),
            Self::Error => Style::default().fg(Color::White).bg(Color::Red),
        }
        .add_modifier(Modifier::BOLD)
    }
}

pub(super) struct EventEntry {
    timestamp: String,
    kind: EventKind,
    text: String,
}

impl EventEntry {
    pub(super) fn new(kind: EventKind, text: impl Into<String>) -> Self {
        Self {
            timestamp: Local::now().format("%H:%M:%S").to_string(),
            kind,
            text: text.into(),
        }
    }

    pub(super) fn render_line(&self) -> Line<'static> {
        Line::from(vec![
            Span::styled(self.timestamp.clone(), Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(format!(" {:^6} ", self.kind.label()), self.kind.style()),
            Span::raw(" "),
            Span::raw(self.text.clone()),
        ])
    }
}

pub(super) fn summarize_arguments(arguments: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments) else {
        return truncate(arguments, 72);
    };
    let Some(object) = value.as_object() else {
        return truncate(arguments, 72);
    };
    if object.is_empty() {
        return "无参数".to_owned();
    }
    let mut keys = object.keys().take(5).cloned().collect::<Vec<_>>();
    if object.len() > keys.len() {
        keys.push("...".to_owned());
    }
    keys.join(", ")
}

pub(super) fn summarize_result(result: &str) -> String {
    let flattened = result
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    truncate(&flattened, 72)
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    format!("{}...", value.chars().take(max_chars).collect::<String>())
}
