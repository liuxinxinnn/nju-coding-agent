use ratatui::layout::{Constraint, Direction, Layout, Rect};

#[derive(Clone, Copy)]
pub(super) struct Areas {
    pub(super) workspace: Rect,
    pub(super) runtime: Rect,
    pub(super) chat: Rect,
    pub(super) events: Rect,
    pub(super) input: Rect,
    pub(super) footer: Rect,
}

pub(super) fn split(area: Rect, events_collapsed: bool) -> Option<Areas> {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(5),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(area);
    let [header, center, input, footer] = root.as_ref() else {
        return None;
    };
    let header = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(*header);
    let [workspace, runtime] = header.as_ref() else {
        return None;
    };
    let center = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(32),
            Constraint::Length(if events_collapsed { 1 } else { 38 }),
        ])
        .split(*center);
    let [chat, events] = center.as_ref() else {
        return None;
    };
    Some(Areas {
        workspace: *workspace,
        runtime: *runtime,
        chat: *chat,
        events: *events,
        input: *input,
        footer: *footer,
    })
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::split;

    #[test]
    fn collapsing_events_expands_conversation() {
        let area = Rect::new(0, 0, 120, 40);
        let expanded = split(area, false).expect("expanded layout");
        let collapsed = split(area, true).expect("collapsed layout");
        assert!(collapsed.chat.width > expanded.chat.width);
        assert_eq!(collapsed.events.width, 1);
    }
}
