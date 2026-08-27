use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

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
            Span::raw(" 允许    "),
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
        Line::from("命令：/help /clear /status /tools /exit"),
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
