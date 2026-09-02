use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::context::ContextUsage;
use crate::session::SessionSummary;

pub(super) fn draw_confirmation(frame: &mut Frame<'_>, prompt: &str) {
    let area = centered_rect(72, 32, frame.area());
    let panel = Paragraph::new(vec![
        Line::from(Span::styled(
            "命令执行确认",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(String::new()),
        Line::from(prompt.to_owned()),
        Line::from(String::new()),
        Line::from(vec![
            Span::styled("Y", Style::default().fg(Color::LightGreen)),
            Span::raw(" 允许（本会话相同命令不再询问）    "),
            Span::styled("N / Esc / Enter", Style::default().fg(Color::LightRed)),
            Span::raw(" 拒绝"),
        ]),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Confirmation")
            .border_style(Style::default().fg(Color::Yellow)),
    )
    .wrap(Wrap { trim: false });
    frame.render_widget(Clear, area);
    frame.render_widget(panel, area);
}

pub(super) fn draw_sessions(
    frame: &mut Frame<'_>,
    current_id: &str,
    sessions: &[SessionSummary],
    selected: usize,
) {
    let area = centered_rect(88, 72, frame.area());
    let visible_rows = usize::from(area.height.saturating_sub(6).max(1));
    let start = selected
        .saturating_add(1)
        .saturating_sub(visible_rows)
        .min(sessions.len().saturating_sub(visible_rows));
    let mut lines = vec![Line::from(Span::styled(
        format!("会话 {} 个 · ● 为当前会话", sessions.len()),
        Style::default().add_modifier(Modifier::BOLD),
    ))];

    if sessions.is_empty() {
        lines.push(Line::from("当前 workspace 还没有保存的会话。"));
    } else {
        for (index, session) in sessions.iter().enumerate().skip(start).take(visible_rows) {
            let current = if session.id == current_id { "●" } else { " " };
            let verification = if session.workspace_revision == 0 {
                "clean"
            } else if session.last_verified_revision == Some(session.workspace_revision) {
                "✓"
            } else {
                "未验证"
            };
            let row = format!(
                "{current} {}  rev {} {}  plan {}  {}",
                truncate_chars(&session.title, 34),
                session.workspace_revision,
                verification,
                if session.planning_enabled {
                    "on"
                } else {
                    "off"
                },
                short_id(&session.id),
            );
            let style = if index == selected {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Blue)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            lines.push(Line::styled(format!(" {} ", row), style));
        }
    }

    lines.extend([
        Line::from(String::new()),
        Line::from(Span::styled(
            "↑/↓ 选择 · Enter 切换 · N 新建 · Esc 关闭",
            Style::default().fg(Color::LightCyan),
        )),
    ]);
    let panel = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Sessions")
                .border_style(Style::default().fg(Color::LightGreen)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(Clear, area);
    frame.render_widget(panel, area);
}

pub(super) fn draw_help(frame: &mut Frame<'_>) {
    let area = centered_rect(76, 70, frame.area());
    let text = vec![
        Line::from("快捷键"),
        Line::from("  Enter           发送任务"),
        Line::from("  Shift+Enter     新增输入行"),
        Line::from("  Ctrl+P/N        输入历史"),
        Line::from("  Up/Down         输入为空时滚动对话"),
        Line::from("  PageUp/PageDown 按页滚动对话"),
        Line::from("  Home/End        对话顶部/底部"),
        Line::from("  Mouse wheel     滚动指针所在的对话/事件栏"),
        Line::from("  Ctrl+Up/Down    逐行滚动事件栏"),
        Line::from("  Ctrl+PgUp/PgDn  按页滚动事件栏"),
        Line::from("  Ctrl+Home/End   事件顶部/恢复追尾"),
        Line::from("  Ctrl+L          折叠/展开事件栏"),
        Line::from("  F1              关闭帮助"),
        Line::from("  Ctrl+D / /exit  退出"),
        Line::from(String::new()),
        Line::from("命令：/help /context /memory /plan on|off /clear /status /tools /exit"),
        Line::from("会话：/sessions 弹窗中 ↑/↓ 选择、Enter 切换、N 新建"),
        Line::from("备用：/new /switch <id> /delete <id>"),
    ];
    let panel = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Help")
                .border_style(Style::default().fg(Color::LightCyan)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(Clear, area);
    frame.render_widget(panel, area);
}

pub(super) fn draw_context(frame: &mut Frame<'_>, usage: ContextUsage, message_count: usize) {
    let area = centered_rect(82, 92, frame.area());
    let used_hundredths = percent_hundredths(usage.used_tokens, usage.window_tokens);
    let mut text = vec![
        Line::from(Span::styled(
            format!(
                "Calibrated current prompt: {} / {} tokens ({}.{:02}%)",
                usage.used_tokens,
                usage.window_tokens,
                used_hundredths / 100,
                used_hundredths % 100
            ),
            Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(
            "Local estimate × {}.{:03} · {} stored messages",
            usage.calibration_millis / 1_000,
            usage.calibration_millis % 1_000,
            message_count,
        )),
        Line::from(String::new()),
        render_context_bar(usage),
        Line::from(String::new()),
    ];
    text.extend(render_context_grid(usage));
    text.extend([
        Line::from(String::new()),
        kv_line(
            "System prompt",
            usage.system_tokens,
            usage.window_tokens,
            Color::LightCyan,
        ),
        kv_line(
            "Tools",
            usage.tool_tokens,
            usage.window_tokens,
            Color::LightYellow,
        ),
        kv_line(
            "Messages",
            usage.message_tokens,
            usage.window_tokens,
            Color::LightMagenta,
        ),
        kv_line(
            "Free space",
            usage.free_tokens,
            usage.window_tokens,
            Color::LightGreen,
        ),
        Line::from(String::new()),
        Line::from(format!(
            "Last API call     prompt {} + completion {} = {} tokens",
            usage.api_usage.last_prompt_tokens,
            usage.api_usage.last_completion_tokens,
            usage.api_usage.last_total_tokens
        )),
        Line::from(format!(
            "Session API total {} requests · prompt {} + completion {} = {} tokens",
            usage.api_usage.requests,
            usage.api_usage.prompt_tokens,
            usage.api_usage.completion_tokens,
            usage.api_usage.total_tokens
        )),
        Line::from(String::new()),
        Line::from(Span::styled(
            "Current prompt is estimated; real API usage calibrates later estimates and is tracked separately.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "Command: /context  |  Esc / Enter close",
            Style::default().fg(Color::LightCyan),
        )),
    ]);
    let panel = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Context")
                .border_style(Style::default().fg(Color::LightMagenta)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(Clear, area);
    frame.render_widget(panel, area);
}

fn render_context_bar(usage: ContextUsage) -> Line<'static> {
    const WIDTH: usize = 48;
    let filled = filled_cells(usage.used_tokens, usage.window_tokens, WIDTH);
    Line::from(vec![
        Span::styled("█".repeat(filled), Style::default().fg(Color::LightMagenta)),
        Span::styled(
            "░".repeat(WIDTH - filled),
            Style::default().fg(Color::DarkGray),
        ),
    ])
}

fn render_context_grid(usage: ContextUsage) -> Vec<Line<'static>> {
    const CELLS: usize = 100;
    const COLUMNS: usize = 25;
    let filled = filled_cells(usage.used_tokens, usage.window_tokens, CELLS);
    let mut lines = Vec::with_capacity(CELLS / COLUMNS);
    for row in 0..(CELLS / COLUMNS) {
        let mut spans = Vec::with_capacity(COLUMNS);
        for column in 0..COLUMNS {
            let index = row * COLUMNS + column;
            spans.push(Span::styled(
                "■ ",
                Style::default().fg(if index < filled {
                    Color::LightMagenta
                } else {
                    Color::DarkGray
                }),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines
}

fn kv_line(label: &'static str, tokens: u64, window_tokens: u64, color: Color) -> Line<'static> {
    let hundredths = percent_hundredths(tokens, window_tokens);
    Line::from(vec![
        Span::styled(
            format!("{label:<14}"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "{:>8} tokens  {:>3}.{:02}%",
            tokens,
            hundredths / 100,
            hundredths % 100
        )),
    ])
}

fn filled_cells(used: u64, window: u64, cells: usize) -> usize {
    let cells_u64 = u64::try_from(cells).unwrap_or(u64::MAX);
    let filled = usize::try_from(
        used.saturating_mul(cells_u64)
            .checked_div(window.max(1))
            .unwrap_or(0)
            .min(cells_u64),
    )
    .unwrap_or(cells);
    if used > 0 && cells > 0 && filled == 0 {
        1
    } else {
        filled
    }
}

fn percent_hundredths(tokens: u64, window_tokens: u64) -> u64 {
    let window = u128::from(window_tokens.max(1));
    let rounded = u128::from(tokens)
        .saturating_mul(10_000)
        .saturating_add(window / 2)
        / window;
    u64::try_from(rounded).unwrap_or(u64::MAX)
}

fn centered_rect(percent_x: u16, percent_y: u16, outer: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(outer);
    let middle = vertical.get(1).copied().unwrap_or(outer);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(middle);
    horizontal.get(1).copied().unwrap_or(outer)
}

fn short_id(id: &str) -> String {
    truncate_chars(id, 22)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    let visible = max_chars.saturating_sub(1);
    format!("{}…", value.chars().take(visible).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::{filled_cells, percent_hundredths};

    #[test]
    fn context_visuals_cap_usage_at_the_available_cells() {
        assert_eq!(filled_cells(0, 1_000_000, 100), 0);
        assert_eq!(filled_cells(1, 1_000_000, 100), 1);
        assert_eq!(filled_cells(6_460, 1_000_000, 100), 1);
        assert_eq!(filled_cells(50, 100, 48), 24);
        assert_eq!(filled_cells(200, 100, 48), 48);
    }

    #[test]
    fn context_percentages_round_to_two_decimal_places() {
        assert_eq!(percent_hundredths(878, 1_000_000), 9);
        assert_eq!(percent_hundredths(2_714, 1_000_000), 27);
        assert_eq!(percent_hundredths(2_868, 1_000_000), 29);
        assert_eq!(percent_hundredths(6_460, 1_000_000), 65);
        assert_eq!(percent_hundredths(1, 8), 1_250);
    }
}
