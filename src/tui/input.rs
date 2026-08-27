use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

const MAX_INPUT_LINES: usize = 6;
const INPUT_PLACEHOLDER: &str = "输入编程任务...";

#[derive(Default)]
pub(super) struct InputBuffer {
    lines: Vec<String>,
    pub(super) cursor_line: usize,
    cursor_col: usize,
}

impl InputBuffer {
    pub(super) fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_line: 0,
            cursor_col: 0,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines.first().is_none_or(String::is_empty)
    }

    pub(super) fn clear(&mut self) {
        self.lines = vec![String::new()];
        self.cursor_line = 0;
        self.cursor_col = 0;
    }

    pub(super) fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub(super) fn set_text(&mut self, text: &str) {
        self.lines = text.lines().map(ToOwned::to_owned).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor_line = self.lines.len().saturating_sub(1);
        self.cursor_col = self.current_line_len();
    }

    pub(super) fn insert_char(&mut self, ch: char) {
        self.ensure_invariants();
        let col = self.cursor_col;
        let line = &mut self.lines[self.cursor_line];
        line.insert(char_to_byte_idx(line, col), ch);
        self.cursor_col += 1;
    }

    pub(super) fn backspace(&mut self) {
        self.ensure_invariants();
        if self.cursor_col > 0 {
            let cursor = self.cursor_col;
            let line = &mut self.lines[self.cursor_line];
            let start = char_to_byte_idx(line, cursor - 1);
            let end = char_to_byte_idx(line, cursor);
            line.replace_range(start..end, "");
            self.cursor_col -= 1;
        } else if self.cursor_line > 0 {
            let current = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;
            self.cursor_col = self.current_line_len();
            self.lines[self.cursor_line].push_str(&current);
        }
    }

    pub(super) fn newline(&mut self) {
        self.ensure_invariants();
        if self.lines.len() >= MAX_INPUT_LINES {
            return;
        }
        let cursor = self.cursor_col;
        let line = &mut self.lines[self.cursor_line];
        let tail = line.split_off(char_to_byte_idx(line, cursor));
        self.cursor_line += 1;
        self.lines.insert(self.cursor_line, tail);
        self.cursor_col = 0;
    }

    pub(super) fn move_left(&mut self) {
        self.ensure_invariants();
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.current_line_len();
        }
    }

    pub(super) fn move_right(&mut self) {
        self.ensure_invariants();
        if self.cursor_col < self.current_line_len() {
            self.cursor_col += 1;
        } else if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            self.cursor_col = 0;
        }
    }

    pub(super) fn move_up(&mut self) {
        self.ensure_invariants();
        if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.cursor_col.min(self.current_line_len());
        }
    }

    pub(super) fn move_down(&mut self) {
        self.ensure_invariants();
        if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            self.cursor_col = self.cursor_col.min(self.current_line_len());
        }
    }

    pub(super) fn move_line_start(&mut self) {
        self.cursor_col = 0;
    }

    pub(super) fn move_line_end(&mut self) {
        self.ensure_invariants();
        self.cursor_col = self.current_line_len();
    }

    pub(super) fn cursor_display_col(&self) -> usize {
        let Some(line) = self.lines.get(self.cursor_line) else {
            return 0;
        };
        let byte = char_to_byte_idx(line, self.cursor_col);
        line.get(..byte).map_or(0, UnicodeWidthStr::width)
    }

    pub(super) fn visual_lines(&self) -> Vec<Line<'_>> {
        if self.lines.iter().all(String::is_empty) {
            return vec![Line::from(Span::styled(
                INPUT_PLACEHOLDER,
                Style::default().fg(Color::DarkGray),
            ))];
        }
        self.lines
            .iter()
            .map(|line| Line::from(line.as_str()))
            .collect()
    }

    fn current_line_len(&self) -> usize {
        self.lines
            .get(self.cursor_line)
            .map_or(0, |line| line.chars().count())
    }

    fn ensure_invariants(&mut self) {
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor_line = self.cursor_line.min(self.lines.len().saturating_sub(1));
        self.cursor_col = self.cursor_col.min(self.current_line_len());
    }
}

fn char_to_byte_idx(value: &str, char_idx: usize) -> usize {
    if char_idx == value.chars().count() {
        return value.len();
    }
    value
        .char_indices()
        .nth(char_idx)
        .map_or(value.len(), |(index, _)| index)
}

#[derive(Default)]
pub(super) struct History {
    entries: Vec<String>,
    browse_index: Option<usize>,
}

impl History {
    pub(super) fn push(&mut self, entry: String) {
        if entry.trim().is_empty() {
            return;
        }
        if self.entries.last() != Some(&entry) {
            self.entries.push(entry);
        }
        self.browse_index = None;
    }

    pub(super) fn prev(&mut self) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let index = self
            .browse_index
            .map_or(self.entries.len().saturating_sub(1), |index| {
                index.saturating_sub(1)
            });
        self.browse_index = Some(index);
        self.entries.get(index).cloned()
    }

    pub(super) fn next(&mut self) -> Option<String> {
        let index = self.browse_index?;
        if index + 1 >= self.entries.len() {
            self.browse_index = None;
            return Some(String::new());
        }
        self.browse_index = Some(index + 1);
        self.entries.get(index + 1).cloned()
    }

    pub(super) fn reset(&mut self) {
        self.browse_index = None;
    }
}

#[cfg(test)]
mod tests {
    use super::InputBuffer;

    #[test]
    fn edits_unicode_by_character_index() {
        let mut input = InputBuffer::new();
        input.insert_char('你');
        input.insert_char('好');
        input.move_left();
        input.insert_char('很');
        assert_eq!(input.text(), "你很好");
        input.backspace();
        assert_eq!(input.text(), "你好");
    }
}
