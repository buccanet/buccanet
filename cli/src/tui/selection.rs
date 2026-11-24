//! Selecting text with the mouse. The UI reads the mouse for the wheel, which turns the
//! terminal's own selection off, so a drag inside one pane highlights what it covers and
//! releasing copies it, as tmux does.

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;

/// The panes a drag can select in, inside their borders, as the last draw placed them.
#[derive(Clone, Copy, Default)]
pub struct Panes {
    pub rooms: Rect,
    pub messages: Rect,
    pub members: Rect,
    pub input: Rect,
}

impl Panes {
    pub fn at(&self, point: Position) -> Option<Rect> {
        [self.messages, self.input, self.rooms, self.members].into_iter().find(|pane| pane.contains(point))
    }
}

/// A drag in reading order from where it started to where the mouse is, kept inside its pane.
#[derive(Clone, Copy)]
pub struct Selection {
    pub pane: Rect,
    anchor: Position,
    head: Position,
}

fn clamp(pane: Rect, point: Position) -> Position {
    Position { x: point.x.clamp(pane.x, pane.right().saturating_sub(1)), y: point.y.clamp(pane.y, pane.bottom().saturating_sub(1)) }
}

impl Selection {
    pub fn start(pane: Rect, point: Position) -> Self {
        let point = clamp(pane, point);
        Self { pane, anchor: point, head: point }
    }

    pub fn extend(&mut self, point: Position) {
        self.head = clamp(self.pane, point);
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    fn ordered(&self) -> (Position, Position) {
        if (self.anchor.y, self.anchor.x) <= (self.head.y, self.head.x) { (self.anchor, self.head) } else { (self.head, self.anchor) }
    }

    /// The columns selected on `row`: from the start on the first row, to the end on the last.
    fn columns(&self, row: u16) -> std::ops::RangeInclusive<u16> {
        let (start, end) = self.ordered();
        let first = if row == start.y { start.x } else { self.pane.x };
        let last = if row == end.y { end.x } else { self.pane.right().saturating_sub(1) };
        first..=last
    }

    fn rows(&self) -> std::ops::RangeInclusive<u16> {
        let (start, end) = self.ordered();
        start.y..=end.y
    }

    pub fn highlight(&self, buffer: &mut Buffer) {
        for row in self.rows() {
            for column in self.columns(row) {
                if let Some(cell) = buffer.cell_mut(Position { x: column, y: row }) {
                    cell.modifier.insert(Modifier::REVERSED);
                }
            }
        }
    }

    /// The selected text as drawn, one line per row, without the spaces that pad each row.
    pub fn text(&self, buffer: &Buffer) -> String {
        let lines: Vec<String> = self
            .rows()
            .map(|row| {
                let line: String = self
                    .columns(row)
                    .filter_map(|column| buffer.cell(Position { x: column, y: row }).map(|cell| cell.symbol().to_string()))
                    .collect();
                line.trim_end().to_string()
            })
            .collect();
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use ratatui::text::Line;
    use ratatui::widgets::{Paragraph, Widget};

    use super::*;

    #[test]
    fn copies_what_a_drag_covers_inside_its_pane() {
        let area = Rect::new(0, 0, 12, 3);
        let mut buffer = Buffer::empty(area);
        Paragraph::new(vec![Line::raw("hello world"), Line::raw("second row"), Line::raw("third")]).render(area, &mut buffer);
        let mut selection = Selection::start(Rect::new(0, 0, 12, 3), Position { x: 6, y: 0 });
        selection.extend(Position { x: 5, y: 1 });
        assert_eq!(selection.text(&buffer), "world\nsecond");
        selection.extend(Position { x: 40, y: 0 });
        assert_eq!(selection.text(&buffer), "world");
        let mut backwards = Selection::start(area, Position { x: 4, y: 2 });
        backwards.extend(Position { x: 0, y: 1 });
        assert_eq!(backwards.text(&buffer), "second row\nthird");
    }
}
