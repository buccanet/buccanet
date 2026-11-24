//! The input box: text with a cursor, edited with the keys of a shell prompt, wrapped to the
//! width of the box so it grows line by line as Claude Code's does.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// What is typed and where the cursor stands, as a byte offset on a character boundary.
#[derive(Default)]
pub struct Editor {
    text: String,
    cursor: usize,
}

/// The input laid out for a box `width` cells wide: its rows, and the cursor's column and row.
pub struct Layout {
    pub rows: Vec<String>,
    pub cursor: (u16, u16),
}

impl Editor {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Replaces the text and puts the cursor at its end.
    pub fn set(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.len();
    }

    pub fn clear(&mut self) {
        self.set(String::new());
    }

    /// Hands the text over and empties the box.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }

    pub fn insert(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    fn previous(&self, from: usize) -> usize {
        self.text[..from].char_indices().next_back().map_or(0, |(index, _)| index)
    }

    fn next(&self, from: usize) -> usize {
        self.text[from..].chars().next().map_or(from, |c| from + c.len_utf8())
    }

    pub fn left(&mut self) {
        self.cursor = self.previous(self.cursor);
    }

    pub fn right(&mut self) {
        self.cursor = self.next(self.cursor);
    }

    fn line_start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |index| index + 1)
    }

    fn line_end(&self) -> usize {
        self.text[self.cursor..].find('\n').map_or(self.text.len(), |index| self.cursor + index)
    }

    pub fn home(&mut self) {
        self.cursor = self.line_start();
    }

    pub fn end(&mut self) {
        self.cursor = self.line_end();
    }

    /// The start of the word before the cursor, skipping the spaces that precede it.
    fn word_start(&self) -> usize {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end_matches(char::is_whitespace);
        trimmed.rfind(char::is_whitespace).map_or(0, |index| index + trimmed[index..].chars().next().map_or(1, char::len_utf8))
    }

    /// The end of the word after the cursor, skipping the spaces that follow it.
    fn word_end(&self) -> usize {
        let after = &self.text[self.cursor..];
        let skipped = after.len() - after.trim_start_matches(char::is_whitespace).len();
        let rest = &after[skipped..];
        self.cursor + skipped + rest.find(char::is_whitespace).unwrap_or(rest.len())
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_start();
    }

    pub fn word_right(&mut self) {
        self.cursor = self.word_end();
    }

    fn remove(&mut self, from: usize, to: usize) {
        self.text.replace_range(from..to, "");
        self.cursor = from;
    }

    pub fn backspace(&mut self) {
        self.remove(self.previous(self.cursor), self.cursor);
    }

    pub fn delete(&mut self) {
        self.remove(self.cursor, self.next(self.cursor));
    }

    pub fn delete_word(&mut self) {
        self.remove(self.word_start(), self.cursor);
    }

    pub fn delete_to_line_start(&mut self) {
        self.remove(self.line_start(), self.cursor);
    }

    pub fn delete_to_line_end(&mut self) {
        let end = self.line_end();
        let cursor = self.cursor;
        self.remove(cursor, end);
    }

    /// Wraps the text after `prefix` into rows `width` cells wide, breaking at line breaks and
    /// wherever a row is full.
    pub fn layout(&self, prefix: &str, width: u16) -> Layout {
        let indent = prefix.width();
        let width = usize::from(width).max(indent + 1);
        let mut rows = vec![prefix.to_string()];
        let (mut column, mut cursor) = (indent, (indent, 0));
        for (index, c) in self.text.char_indices() {
            if index == self.cursor {
                cursor = (column, rows.len() - 1);
            }
            let cells = c.width().unwrap_or(0);
            if c == '\n' || column + cells > width {
                rows.push(" ".repeat(indent));
                column = indent;
            }
            if c != '\n' {
                rows.last_mut().expect("rows start with one").push(c);
                column += cells;
            }
        }
        if self.cursor == self.text.len() {
            if column >= width {
                rows.push(" ".repeat(indent));
                column = indent;
            }
            cursor = (column, rows.len() - 1);
        }
        Layout { rows, cursor: (cursor.0 as u16, cursor.1 as u16) }
    }

    /// Puts the cursor on the character drawn at `column`, `row` of a layout with `prefix`, or
    /// at the end of that row when the point lies past it.
    pub fn click(&mut self, prefix: &str, width: u16, column: u16, row: u16) {
        let mut best = self.text.len();
        for index in self.text.char_indices().map(|(index, _)| index).chain([self.text.len()]) {
            let probe = Editor { text: self.text.clone(), cursor: index };
            let (at_column, at_row) = probe.layout(prefix, width).cursor;
            if at_row == row && at_column <= column {
                best = index;
            }
            if at_row > row {
                break;
            }
        }
        self.cursor = best;
    }

    /// Moves the cursor one row up or down in a layout `width` wide; false on the first or last row.
    pub fn vertical(&mut self, prefix: &str, width: u16, delta: i16) -> bool {
        let layout = self.layout(prefix, width);
        let (column, row) = layout.cursor;
        let Some(target) = row.checked_add_signed(delta).filter(|target| usize::from(*target) < layout.rows.len()) else {
            return false;
        };
        self.click(prefix, width, column, target);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(text: &str) -> Editor {
        let mut editor = Editor::default();
        editor.set(text);
        editor
    }

    #[test]
    fn edits_at_the_cursor_and_by_word() {
        let mut input = editor("hello world");
        input.word_left();
        input.insert("big ");
        assert_eq!(input.text(), "hello big world");
        input.delete_word();
        assert_eq!(input.text(), "hello world");
        input.home();
        input.delete();
        input.end();
        input.backspace();
        assert_eq!(input.text(), "ello worl");
        input.home();
        input.word_right();
        input.delete_to_line_end();
        assert_eq!(input.text(), "ello");
    }

    #[test]
    fn wraps_rows_and_follows_the_cursor() {
        let mut input = editor("abcdefgh\nij");
        let layout = input.layout("› ", 6);
        assert_eq!(layout.rows, vec!["› abcd", "  efgh", "  ij"]);
        assert_eq!(layout.cursor, (4, 2));
        assert!(input.vertical("› ", 6, -1));
        assert_eq!(input.layout("› ", 6).cursor, (4, 1));
        input.click("› ", 6, 3, 0);
        input.insert("X");
        assert_eq!(input.text(), "aXbcdefgh\nij");
        assert!(!editor("one row").vertical("› ", 20, -1));
    }

    #[test]
    fn keeps_accents_whole() {
        let mut input = editor("été");
        input.left();
        input.backspace();
        assert_eq!(input.text(), "éé");
    }
}
