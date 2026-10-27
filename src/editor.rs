//! A small text field: single- or multi-line, soft-wrapped when drawn.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone)]
pub struct TextArea {
    lines: Vec<String>,
    row: usize,
    col: usize,
    multiline: bool,
    scroll: usize,
    /// Where the text was last drawn, and each visual line as
    /// (row, first char, end char), so clicks map back to a position.
    inner: Rect,
    segments: Vec<(usize, usize, usize)>,
    /// A selected word as (row, start, end); typing replaces it.
    selection: Option<(usize, usize, usize)>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '_'
}

impl TextArea {
    /// The cursor starts at the end of `text`.
    pub fn new(text: &str, multiline: bool) -> Self {
        let mut lines: Vec<String> = if multiline {
            text.split('\n').map(String::from).collect()
        } else {
            vec![text.replace('\n', " ")]
        };
        if lines.is_empty() {
            lines.push(String::new());
        }
        let row = lines.len() - 1;
        let col = lines[row].chars().count();
        Self {
            lines,
            row,
            col,
            multiline,
            scroll: 0,
            inner: Rect::default(),
            segments: Vec::new(),
            selection: None,
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Replaces the text, keeping the cursor where it can stay.
    pub fn set_text(&mut self, text: &str) {
        let (row, col) = (self.row, self.col);
        let (inner, scroll) = (self.inner, self.scroll);
        *self = Self::new(text, self.multiline);
        (self.inner, self.scroll) = (inner, scroll);
        self.row = row.min(self.lines.len() - 1);
        self.col = col.min(self.len(self.row));
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    pub fn set_cursor(&mut self, row: usize, col: usize) {
        self.row = row.min(self.lines.len() - 1);
        self.col = col.min(self.len(self.row));
    }

    /// Char range of the word under or just before the cursor.
    fn word_bounds(&self) -> Option<(usize, usize)> {
        let is_word = |c: char| c.is_alphanumeric() || c == '\'' || c == '-';
        let chars: Vec<char> = self.lines[self.row].chars().collect();
        let mut at = self.col.min(chars.len());
        if at > 0 && (at == chars.len() || !is_word(chars[at])) && is_word(chars[at - 1]) {
            at -= 1;
        }
        if at >= chars.len() || !is_word(chars[at]) {
            return None;
        }
        let (mut start, mut end) = (at, at);
        while start > 0 && is_word(chars[start - 1]) {
            start -= 1;
        }
        while end < chars.len() && is_word(chars[end]) {
            end += 1;
        }
        Some((start, end))
    }

    /// The selection, else the word under or just before the cursor.
    pub fn word_at_cursor(&self) -> String {
        let (start, end) = match self.selection {
            Some((_, start, end)) => (start, end),
            None => match self.word_bounds() {
                Some(bounds) => bounds,
                None => return String::new(),
            },
        };
        self.lines[self.row].chars().take(end).skip(start).collect()
    }

    /// Selects the word at the cursor; false when there's none.
    pub fn select_word(&mut self) -> bool {
        let Some((start, end)) = self.word_bounds() else {
            return false;
        };
        self.selection = Some((self.row, start, end));
        self.col = end;
        true
    }

    pub fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    /// Start of the previous word, or the end of the line above.
    fn word_left(&mut self) {
        if self.col == 0 {
            if self.row > 0 {
                self.row -= 1;
                self.col = self.len(self.row);
            }
            return;
        }
        let chars: Vec<char> = self.lines[self.row].chars().collect();
        let mut col = self.col;
        while col > 0 && !is_word(chars[col - 1]) {
            col -= 1;
        }
        while col > 0 && is_word(chars[col - 1]) {
            col -= 1;
        }
        self.col = col;
    }

    /// End of the next word, or the start of the line below.
    fn word_right(&mut self) {
        let chars: Vec<char> = self.lines[self.row].chars().collect();
        if self.col >= chars.len() {
            if self.row + 1 < self.lines.len() {
                self.row += 1;
                self.col = 0;
            }
            return;
        }
        let mut col = self.col;
        while col < chars.len() && !is_word(chars[col]) {
            col += 1;
        }
        while col < chars.len() && is_word(chars[col]) {
            col += 1;
        }
        self.col = col;
    }

    fn delete_word_back(&mut self) {
        let (row, end) = (self.row, self.col);
        self.word_left();
        if self.row != row {
            // At the start of a line: join it to the one above, like Backspace.
            let line = self.lines.remove(row);
            self.lines[self.row].push_str(&line);
            return;
        }
        let (a, b) = (self.byte(row, self.col), self.byte(row, end));
        self.lines[row].drain(a..b);
    }

    /// Moves the cursor to a clicked screen cell; false when outside the text.
    pub fn click(&mut self, x: u16, y: u16) -> bool {
        let inner = self.inner;
        if !inner.contains(Position::new(x, y)) {
            return false;
        }
        self.selection = None;
        let line = self.scroll + (y - inner.y) as usize;
        let Some(&(row, start, end)) = self.segments.get(line).or(self.segments.last()) else {
            return false;
        };
        let target = (x - inner.x) as usize;
        let mut cells = 0;
        self.col = end;
        for (i, ch) in self.lines[row].chars().enumerate().take(end).skip(start) {
            let w = ch.width().unwrap_or(0);
            if cells + w > target {
                self.col = i;
                break;
            }
            cells += w;
        }
        self.row = row;
        true
    }

    fn len(&self, row: usize) -> usize {
        self.lines[row].chars().count()
    }

    fn byte(&self, row: usize, col: usize) -> usize {
        self.lines[row]
            .char_indices()
            .nth(col)
            .map_or(self.lines[row].len(), |(i, _)| i)
    }

    /// Applies an editing key; false when the key isn't an editing key.
    pub fn input(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if let Some((row, start, end)) = self.selection.take() {
            let replaces = matches!(key.code, KeyCode::Char(_) if !ctrl && !alt);
            if replaces || matches!(key.code, KeyCode::Backspace | KeyCode::Delete) {
                let (a, b) = (self.byte(row, start), self.byte(row, end));
                self.lines[row].drain(a..b);
                (self.row, self.col) = (row, start);
                if !replaces {
                    return true;
                }
            }
        }
        let (row, col) = (self.row, self.col);
        match key.code {
            KeyCode::Left if ctrl || alt => self.word_left(),
            KeyCode::Right if ctrl || alt => self.word_right(),
            // Option+arrows arrive as Alt+b / Alt+f in many macOS terminals.
            KeyCode::Char('b') if alt => self.word_left(),
            KeyCode::Char('f') if alt => self.word_right(),
            KeyCode::Char('w') if ctrl => self.delete_word_back(),
            KeyCode::Backspace if ctrl || alt => self.delete_word_back(),
            KeyCode::Char('a') if ctrl => self.col = 0,
            KeyCode::Char('e') if ctrl => self.col = self.len(row),
            KeyCode::Char('u') if ctrl => {
                let b = self.byte(row, col);
                self.lines[row].drain(..b);
                self.col = 0;
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                let b = self.byte(row, col);
                self.lines[row].insert(b, c);
                self.col += 1;
            }
            KeyCode::Enter if self.multiline => {
                let b = self.byte(row, col);
                let rest = self.lines[row].split_off(b);
                self.lines.insert(row + 1, rest);
                self.row += 1;
                self.col = 0;
            }
            KeyCode::Backspace if col > 0 => {
                let b = self.byte(row, col - 1);
                self.lines[row].remove(b);
                self.col -= 1;
            }
            KeyCode::Backspace if row > 0 => {
                let line = self.lines.remove(row);
                self.row -= 1;
                self.col = self.len(self.row);
                self.lines[self.row].push_str(&line);
            }
            KeyCode::Delete if col < self.len(row) => {
                let b = self.byte(row, col);
                self.lines[row].remove(b);
            }
            KeyCode::Delete if row + 1 < self.lines.len() => {
                let next = self.lines.remove(row + 1);
                self.lines[row].push_str(&next);
            }
            KeyCode::Left if col > 0 => self.col -= 1,
            KeyCode::Left if row > 0 => {
                self.row -= 1;
                self.col = self.len(self.row);
            }
            KeyCode::Right if col < self.len(row) => self.col += 1,
            KeyCode::Right if row + 1 < self.lines.len() => {
                self.row += 1;
                self.col = 0;
            }
            KeyCode::Up if row > 0 => {
                self.row -= 1;
                self.col = col.min(self.len(self.row));
            }
            KeyCode::Down if row + 1 < self.lines.len() => {
                self.row += 1;
                self.col = col.min(self.len(self.row));
            }
            KeyCode::Home => self.col = 0,
            KeyCode::End => self.col = self.len(row),
            KeyCode::Backspace
            | KeyCode::Delete
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down => {}
            _ => return false,
        }
        true
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect, block: Block, focused: bool) {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let width = inner.width.max(1) as usize;
        let height = inner.height.max(1) as usize;

        // Wrap by display cells, so wide characters take their real space.
        let mut segments = Vec::new();
        for (r, line) in self.lines.iter().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            let (mut start, mut cells) = (0, 0);
            for (i, ch) in chars.iter().enumerate() {
                let w = ch.width().unwrap_or(0);
                if cells + w > width && i > start {
                    segments.push((r, start, i));
                    start = i;
                    cells = 0;
                }
                cells += w;
            }
            segments.push((r, start, chars.len()));
            // A cursor just past a full visual line gets a line of its own.
            if r == self.row && self.col == chars.len() && cells >= width && !chars.is_empty() {
                segments.push((r, chars.len(), chars.len()));
            }
        }
        let cursor_line = segments
            .iter()
            .rposition(|&(r, s, e)| r == self.row && s <= self.col && self.col <= e)
            .unwrap_or(0);
        let cursor_start = segments[cursor_line].1;
        let cursor_cell: usize = self.lines[self.row]
            .chars()
            .take(self.col)
            .skip(cursor_start)
            .map(|c| c.width().unwrap_or(0))
            .sum();
        let cursor = (cursor_line, cursor_cell);
        if cursor.0 < self.scroll {
            self.scroll = cursor.0;
        } else if cursor.0 >= self.scroll + height {
            self.scroll = cursor.0 + 1 - height;
        }
        let shown: Vec<Line> = segments
            .iter()
            .skip(self.scroll)
            .take(height)
            .map(|&(r, s, e)| {
                let chars: Vec<char> = self.lines[r].chars().take(e).skip(s).collect();
                match self.selection {
                    Some((row, a, b)) if row == r && a < e && b > s => {
                        let (a, b) = (a.max(s) - s, b.min(e) - s);
                        let part = |x: usize, y: usize| chars[x..y].iter().collect::<String>();
                        Line::from(vec![
                            Span::raw(part(0, a)),
                            Span::styled(part(a, b), Style::new().add_modifier(Modifier::REVERSED)),
                            Span::raw(part(b, chars.len())),
                        ])
                    }
                    _ => Line::raw(chars.into_iter().collect::<String>()),
                }
            })
            .collect();
        self.inner = inner;
        self.segments = segments;
        frame.render_widget(Paragraph::new(shown), inner);
        if focused {
            frame.set_cursor_position(Position::new(
                inner.x + cursor.1 as u16,
                inner.y + (cursor.0 - self.scroll) as u16,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn edits_across_lines() {
        let mut t = TextArea::new("héllo", true);
        t.input(key(KeyCode::Backspace));
        t.input(key(KeyCode::Enter));
        for c in "wörld".chars() {
            t.input(key(KeyCode::Char(c)));
        }
        assert_eq!(t.text(), "héll\nwörld");
        t.input(key(KeyCode::Home));
        t.input(key(KeyCode::Backspace));
        assert_eq!(t.text(), "héllwörld");
        t.input(key(KeyCode::Delete));
        assert_eq!(t.text(), "héllörld");
    }

    #[test]
    fn single_line_ignores_enter() {
        let mut t = TextArea::new("a\nb", false);
        assert!(!t.input(key(KeyCode::Enter)));
        assert_eq!(t.text(), "a b");
        t.input(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(t.text(), "");
    }

    #[test]
    fn typing_replaces_a_selected_word() {
        let mut t = TextArea::new("open the dewy now", false);
        t.set_cursor(0, 10);
        assert!(t.select_word());
        assert_eq!(t.word_at_cursor(), "dewy");
        for c in "TUI".chars() {
            t.input(key(KeyCode::Char(c)));
        }
        assert_eq!(t.text(), "open the TUI now");
        t.set_cursor(0, 1);
        t.select_word();
        t.input(key(KeyCode::Backspace));
        assert_eq!(t.text(), " the TUI now");
        t.select_word(); // nothing under the cursor
        t.input(key(KeyCode::Right));
        assert!(!t.has_selection());
    }

    #[test]
    fn jumps_and_deletes_by_word() {
        let alt = |code| KeyEvent::new(code, KeyModifiers::ALT);
        let mut t = TextArea::new("open the dewy to the right.", false);
        t.input(alt(KeyCode::Left)); // before "right"
        t.input(alt(KeyCode::Char('b'))); // before "the"
        t.input(alt(KeyCode::Left)); // before "to"
        t.input(alt(KeyCode::Left)); // before "dewy"
        assert_eq!(t.word_at_cursor(), "dewy");
        t.input(alt(KeyCode::Right)); // after "dewy"
        t.input(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        for c in "TUI".chars() {
            t.input(key(KeyCode::Char(c)));
        }
        assert_eq!(t.text(), "open the TUI to the right.");
    }

    #[test]
    fn click_places_the_cursor_on_wrapped_text() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut t = TextArea::new("open the dewy to the right", true);
        let mut terminal = Terminal::new(TestBackend::new(12, 5)).unwrap();
        // 10 cells wide inside the border: "open the d" / "ewy to the" / " right".
        terminal
            .draw(|f| t.render(f, f.area(), Block::bordered(), true))
            .unwrap();
        assert!(t.click(3, 2)); // second visual line, third cell: "y"
        assert_eq!(t.word_at_cursor(), "dewy");
        assert!(!t.click(0, 0), "border is outside the text");
        assert!(t.click(10, 3)); // past the end of the last line
        t.input(key(KeyCode::Char('!')));
        assert_eq!(t.text(), "open the dewy to the right!");
    }
}
