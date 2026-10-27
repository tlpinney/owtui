//! A small text field: single- or multi-line, soft-wrapped when drawn.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::text::Line;
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
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
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
        let (row, col) = (self.row, self.col);
        match key.code {
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
        let mut visual: Vec<String> = Vec::new();
        let mut cursor = (0, 0);
        for (r, line) in self.lines.iter().enumerate() {
            let mut current = String::new();
            let mut cells = 0;
            for (i, ch) in line.chars().enumerate() {
                let w = ch.width().unwrap_or(0);
                if cells + w > width && !current.is_empty() {
                    visual.push(std::mem::take(&mut current));
                    cells = 0;
                }
                if r == self.row && i == self.col {
                    cursor = (visual.len(), cells);
                }
                current.push(ch);
                cells += w;
            }
            if r == self.row && self.col >= line.chars().count() {
                if cells >= width {
                    visual.push(std::mem::take(&mut current));
                    cells = 0;
                }
                cursor = (visual.len(), cells);
            }
            visual.push(current);
        }
        if cursor.0 < self.scroll {
            self.scroll = cursor.0;
        } else if cursor.0 >= self.scroll + height {
            self.scroll = cursor.0 + 1 - height;
        }
        let shown: Vec<Line> = visual
            .into_iter()
            .skip(self.scroll)
            .take(height)
            .map(Line::raw)
            .collect();
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
}
