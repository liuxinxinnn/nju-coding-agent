use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::context::ContextUsage;

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
        Line::from("  Ctrl+L          折叠/展开事件栏"),
        Line::from("  F1              关闭帮助"),
        Line::from("  Ctrl+D / /exit  退出"),
        Line::from(String::new()),
        Line::from("命令：/help /context /plan on|off /clear /status /tools /exit"),
        Line::from("会话：/new /sessions /switch <id> /delete <id>"),
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
    let area = centered_rect(82, 72, frame.area());
    let used_tenths = percent_tenths(usage.used_tokens, usage.window_tokens);
    let mut text = vec![
        Line::from(Span::styled(
            format!(
                "Estimated current prompt: {} / {} tokens ({}.{:01}%)",
                usage.used_tokens,
                usage.window_tokens,
                used_tenths / 10,
                used_tenths % 10
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(
            "Conservative local estimate · {} stored messages",
            message_count
        )),
        Line::from(String::new()),
        render_context_bar(usage),
        Line::from(String::new()),
    ];
    text.extend(render_context_grid(usage));
    text.extend([
        Line::from(String::new()),
        kv_line("System prompt", usage.system_tokens, usage.window_tokens),
        kv_line("Tools", usage.tool_tokens, usage.window_tokens),
        kv_line("Messages", usage.message_tokens, usage.window_tokens),
        kv_line("Free space", usage.free_tokens, usage.window_tokens),
        Line::from(String::new()),
        Line::from(Span::styled(
            "The estimate includes system instructions, tool definitions/results, and conversation history.",
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

fn kv_line(label: &'static str, tokens: u64, window_tokens: u64) -> Line<'static> {
    let tenths = percent_tenths(tokens, window_tokens);
    Line::from(vec![
        Span::styled(format!("{label:<14}"), Style::default().fg(Color::Gray)),
        Span::raw(format!(
            "{:>8} tokens  {:>3}.{:01}%",
            tokens,
            tenths / 10,
            tenths % 10
        )),
    ])
}

fn filled_cells(used: u64, window: u64, cells: usize) -> usize {
    let cells_u64 = u64::try_from(cells).unwrap_or(u64::MAX);
    usize::try_from(
        used.saturating_mul(cells_u64)
            .checked_div(window.max(1))
            .unwrap_or(0)
            .min(cells_u64),
    )
    .unwrap_or(cells)
}

fn percent_tenths(tokens: u64, window_tokens: u64) -> u64 {
    tokens
        .saturating_mul(1_000)
        .checked_div(window_tokens.max(1))
        .unwrap_or(0)
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

#[cfg(test)]
mod tests {
    use super::{filled_cells, percent_tenths};

    #[test]
    fn context_visuals_cap_usage_at_the_available_cells() {
        assert_eq!(filled_cells(50, 100, 48), 24);
        assert_eq!(filled_cells(200, 100, 48), 48);
        assert_eq!(percent_tenths(1, 8), 125);
    }
}
