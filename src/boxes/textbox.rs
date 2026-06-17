use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::Widget,
};

/// A multi-line text widget that supports character-level wrapping and an
/// optional interactive cursor.
///
/// Unlike Ratatui's standard `Paragraph`, this widget uses strict character
/// wrapping (no word-wrap) to ensure the cursor position always stays in sync
/// with the rendered text.
pub struct TextBox<'a> {
    pub text: &'a str,
    pub cursor: Option<usize>,
    pub style: Style,
}

impl<'a> TextBox<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            cursor: None,
            style: Style::default(),
        }
    }

    pub fn cursor(mut self, cursor: usize) -> Self {
        self.cursor = Some(cursor);
        self
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// Calculate the height required to render the text within a given width.
    pub fn calculate_height(text: &str, width: u16, has_cursor: bool) -> u16 {
        if width == 0 {
            return 1;
        }
        let w = width as usize;
        let mut lines: u16 = 0;
        let mut paragraphs = text.split('\n').peekable();

        while let Some(paragraph) = paragraphs.next() {
            let is_last = paragraphs.peek().is_none();
            // If it's the last paragraph and we have a cursor, we need an extra slot
            // for the insertion point at the end.
            let n = if is_last && has_cursor {
                paragraph.chars().count() + 1
            } else {
                paragraph.chars().count()
            };

            lines += (n.div_ceil(w)).max(1) as u16;
        }
        lines.max(1)
    }
}

impl<'a> Widget for TextBox<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let mut renderer = WrappingRenderer::new(area, buf, self.style);
        renderer.pre_fill();

        for ch in self.text.chars() {
            renderer.check_cursor(self.cursor);
            if !renderer.render_char(ch) {
                break;
            }
        }

        renderer.finalize_cursor(self.cursor);
        if let Some((cx, cy)) = renderer.cursor_xy
            && let Some(cell) = buf.cell_mut((cx, cy))
        {
            cell.set_style(Style::default().add_modifier(Modifier::REVERSED));
        }
    }
}

/// Internal helper for character-level wrapping.
struct WrappingRenderer<'a> {
    area: Rect,
    buf: &'a mut Buffer,
    style: Style,
    line: u16,
    col: u16,
    byte_idx: usize,
    cursor_xy: Option<(u16, u16)>,
}

impl<'a> WrappingRenderer<'a> {
    fn new(area: Rect, buf: &'a mut Buffer, style: Style) -> Self {
        Self {
            area,
            buf,
            style,
            line: 0,
            col: 0,
            byte_idx: 0,
            cursor_xy: None,
        }
    }

    fn pre_fill(&mut self) {
        for y in self.area.y..self.area.y + self.area.height {
            for x in self.area.x..self.area.x + self.area.width {
                if let Some(cell) = self.buf.cell_mut((x, y)) {
                    cell.set_symbol(" ");
                    cell.set_style(self.style);
                }
            }
        }
    }

    fn check_cursor(&mut self, cursor: Option<usize>) {
        if cursor == Some(self.byte_idx) && self.cursor_xy.is_none() {
            self.cursor_xy = Some((self.area.x + self.col, self.area.y + self.line));
        }
    }

    fn next_line(&mut self) -> bool {
        self.line += 1;
        self.col = 0;
        self.line < self.area.height
    }

    fn render_char(&mut self, ch: char) -> bool {
        if ch == '\n' {
            self.byte_idx += ch.len_utf8();
            return self.next_line();
        }

        if self.col >= self.area.width && !self.next_line() {
            return false;
        }

        let mut buf_str = [0u8; 4];
        let x = self.area.x + self.col;
        let y = self.area.y + self.line;
        if let Some(cell) = self.buf.cell_mut((x, y)) {
            cell.set_symbol(ch.encode_utf8(&mut buf_str));
            cell.set_style(self.style);
        }
        self.col += 1;
        self.byte_idx += ch.len_utf8();
        true
    }

    fn finalize_cursor(&mut self, cursor: Option<usize>) {
        if cursor == Some(self.byte_idx) && self.cursor_xy.is_none() {
            if self.col >= self.area.width {
                let _ = self.next_line();
            }
            if self.line < self.area.height {
                self.cursor_xy = Some((self.area.x + self.col, self.area.y + self.line));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculate_height_basics() {
        // Empty text always needs at least one row.
        assert_eq!(TextBox::calculate_height("", 10, false), 1);
        // A cursor on empty text still fits in one row.
        assert_eq!(TextBox::calculate_height("", 10, true), 1);
        // Short single line.
        assert_eq!(TextBox::calculate_height("hello", 10, false), 1);
        // Wrapping: 5 chars at width 3 -> 2 rows.
        assert_eq!(TextBox::calculate_height("hello", 3, false), 2);
        // Explicit newlines split into paragraphs.
        assert_eq!(TextBox::calculate_height("abc\ndef", 10, false), 2);
        // Width 0 degrades to a single row.
        assert_eq!(TextBox::calculate_height("hello", 0, false), 1);
    }

    #[test]
    fn calculate_height_reserves_a_row_for_a_trailing_cursor() {
        // A full-width last line plus the insertion cursor needs an extra row,
        // matching where the renderer places the wrapped cursor.
        assert_eq!(TextBox::calculate_height("0123456789", 10, false), 1);
        assert_eq!(TextBox::calculate_height("0123456789", 10, true), 2);
    }

    /// Position of the reverse-video cursor cell after rendering, if any.
    fn cursor_pos(text: &str, cursor: usize, w: u16, h: u16) -> Option<(u16, u16)> {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        TextBox::new(text).cursor(cursor).render(area, &mut buf);
        for y in 0..h {
            for x in 0..w {
                if buf[(x, y)]
                    .style()
                    .add_modifier
                    .contains(Modifier::REVERSED)
                {
                    return Some((x, y));
                }
            }
        }
        None
    }

    #[test]
    fn cursor_renders_at_end_and_mid_string() {
        // Cursor at the end sits just past the last char.
        assert_eq!(cursor_pos("ab", 2, 10, 2), Some((2, 0)));
        // Cursor in the middle lands on that char's cell.
        assert_eq!(cursor_pos("abc", 1, 10, 2), Some((1, 0)));
    }

    #[test]
    fn cursor_wraps_to_next_line_past_a_full_width_line() {
        // 10 chars exactly fill row 0; the trailing cursor wraps to (0, 1).
        assert_eq!(cursor_pos("0123456789", 10, 10, 2), Some((0, 1)));
    }
}
