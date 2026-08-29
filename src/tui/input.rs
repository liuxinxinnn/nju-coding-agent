use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const MAX_INPUT_LINES: usize = 6;
const INPUT_PLACEHOLDER: &str = "输入编程任务...";

#[derive(Default)]
pub(super) struct InputBuffer {
    lines: Vec<String>,
    pub(super) cursor_line: usize,
    cursor_col: usize,
}

pub(super) struct InputView {
    pub(super) lines: Vec<Line<'static>>,
    pub(super) cursor_row: usize,
    pub(super) cursor_col: usize,
}

struct VisualSegment {
    logical_line: usize,
    start_char: usize,
    end_char: usize,
    text: String,
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

    pub(super) fn move_up(&mut self, width: usize) {
        self.ensure_invariants();
        let segments = self.visual_segments(width);
        let (row, display_col) = self.visual_cursor(&segments);
        if row > 0 {
            self.move_to_visual_segment(&segments[row - 1], display_col);
        }
    }

    pub(super) fn move_down(&mut self, width: usize) {
        self.ensure_invariants();
        let segments = self.visual_segments(width);
        let (row, display_col) = self.visual_cursor(&segments);
        if let Some(segment) = segments.get(row + 1) {
            self.move_to_visual_segment(segment, display_col);
        }
    }

    pub(super) fn move_line_start(&mut self) {
        self.cursor_col = 0;
    }

    pub(super) fn move_line_end(&mut self) {
        self.ensure_invariants();
        self.cursor_col = self.current_line_len();
    }

    pub(super) fn visual(&self, width: usize) -> InputView {
        let segments = self.visual_segments(width);
        let (cursor_row, cursor_col) = self.visual_cursor(&segments);
        let mut lines = segments
            .into_iter()
            .map(|segment| Line::from(segment.text))
            .collect::<Vec<_>>();
        if self.is_empty() {
            lines[0] = Line::from(Span::styled(
                INPUT_PLACEHOLDER,
                Style::default().fg(Color::DarkGray),
            ));
        }
        InputView {
            lines,
            cursor_row,
            cursor_col,
        }
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

    fn visual_segments(&self, width: usize) -> Vec<VisualSegment> {
        let width = width.max(1);
        let mut segments = Vec::new();
        for (logical_line, line) in self.lines.iter().enumerate() {
            let mut text = String::new();
            let mut display_width = 0_usize;
            let mut start_char = 0;
            let mut end_char = 0;
            for (char_index, character) in line.chars().enumerate() {
                let char_width = UnicodeWidthChar::width(character).unwrap_or(0);
                if !text.is_empty() && display_width.saturating_add(char_width) > width {
                    segments.push(VisualSegment {
                        logical_line,
                        start_char,
                        end_char: char_index,
                        text: std::mem::take(&mut text),
                    });
                    start_char = char_index;
                    display_width = 0;
                }
                text.push(character);
                display_width = display_width.saturating_add(char_width);
                end_char = char_index + 1;
                if display_width >= width {
                    segments.push(VisualSegment {
                        logical_line,
                        start_char,
                        end_char,
                        text: std::mem::take(&mut text),
                    });
                    start_char = end_char;
                    display_width = 0;
                }
            }
            let needs_cursor_row = logical_line == self.cursor_line
                && self.cursor_col == end_char
                && start_char == end_char;
            if !text.is_empty() || line.is_empty() || needs_cursor_row {
                segments.push(VisualSegment {
                    logical_line,
                    start_char,
                    end_char,
                    text,
                });
            }
        }
        if segments.is_empty() {
            segments.push(VisualSegment {
                logical_line: 0,
                start_char: 0,
                end_char: 0,
                text: String::new(),
            });
        }
        segments
    }

    fn visual_cursor(&self, segments: &[VisualSegment]) -> (usize, usize) {
        for (row, segment) in segments.iter().enumerate() {
            if segment.logical_line != self.cursor_line {
                continue;
            }
            let is_last_for_line = segments
                .get(row + 1)
                .is_none_or(|next| next.logical_line != segment.logical_line);
            let contains_cursor = (self.cursor_col >= segment.start_char
                && self.cursor_col < segment.end_char)
                || (segment.start_char == segment.end_char
                    && self.cursor_col == segment.start_char)
                || (is_last_for_line && self.cursor_col == segment.end_char);
            if contains_cursor {
                let characters = self.cursor_col.saturating_sub(segment.start_char);
                let byte = char_to_byte_idx(&segment.text, characters);
                let display_col = segment.text.get(..byte).map_or(0, UnicodeWidthStr::width);
                return (row, display_col);
            }
        }
        (segments.len().saturating_sub(1), 0)
    }

    fn move_to_visual_segment(&mut self, segment: &VisualSegment, display_col: usize) {
        self.cursor_line = segment.logical_line;
        let mut width = 0_usize;
        let mut characters = 0;
        for character in segment.text.chars() {
            let next = width.saturating_add(UnicodeWidthChar::width(character).unwrap_or(0));
            if next > display_col {
                break;
            }
            width = next;
            characters += 1;
        }
        self.cursor_col = segment.start_char.saturating_add(characters);
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

    #[test]
    fn soft_wraps_without_changing_submitted_text_or_cursor() {
        let mut input = InputBuffer::new();
        input.set_text("123456你好abcdef");

        let view = input.visual(8);
        let rendered = view
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(rendered, input.text());
        assert!(view.lines.len() >= 2);
        assert_eq!(view.cursor_row, view.lines.len() - 1);
        assert!(view.cursor_col < 8);
    }

    #[test]
    fn up_and_down_move_between_soft_wrapped_rows() {
        let mut input = InputBuffer::new();
        input.set_text("abcdefghijklmnopqrst");
        assert_eq!(input.visual(6).cursor_row, 3);

        input.move_up(6);
        assert_eq!(input.visual(6).cursor_row, 2);
        input.move_down(6);
        assert_eq!(input.visual(6).cursor_row, 3);
    }

    #[test]
    fn exact_width_manual_line_does_not_add_a_blank_visual_row() {
        let mut input = InputBuffer::new();
        input.set_text("abcdef\nnext");

        let view = input.visual(6);

        assert_eq!(view.lines.len(), 2);
        assert_eq!(view.cursor_row, 1);
    }
}
