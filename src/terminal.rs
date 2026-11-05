//! One pane's emulator, and the styled text a renderer draws from it.

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point};
use alacritty_terminal::selection::Selection;
use alacritty_terminal::term::search::{Match, RegexSearch};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};

pub use alacritty_terminal::index::Side;
pub use alacritty_terminal::selection::SelectionType;
pub use alacritty_terminal::vi_mode::ViMotion;

/// A pane's colours. A theme change repaints from this, with no reparse.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub fg: u32,
    pub bg: u32,
    pub ansi: [u32; 16],
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            fg: 0xc0caf5,
            bg: 0x1a1b26,
            ansi: [
                0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6, //
                0x414868, 0xff899d, 0xb9f27c, 0xffc777, 0x8db0ff, 0xc7a9ff, 0xa4daff, 0xc0caf5,
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub col: usize,
    pub text: String,
    pub fg: u32,
    pub bg: Option<u32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Debug, Default, Clone)]
pub struct Screen {
    pub rows: Vec<Vec<Run>>,
    pub cursor: Option<(usize, usize)>,
    /// Copy mode's cursor, which marks a whole cell.
    pub block_cursor: bool,
}

pub struct Pane {
    term: Term<VoidListener>,
    parser: Processor,
    /// Where the current visual selection began, for swapping its ends.
    anchor: Option<Point>,
    /// The search match on screen.
    found: Option<Match>,
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let c = |s: u32| {
        let (x, y) = (((a >> s) & 0xff) as f32, ((b >> s) & 0xff) as f32);
        ((x + (y - x) * t) as u32) << s
    };
    c(16) | c(8) | c(0)
}

/// A selection of the cell at a point. The terminal does not follow the cursor
/// with an empty selection, and the cell under copy mode's cursor counts anyway.
fn one_cell(kind: SelectionType, at: Point) -> Selection {
    let mut selection = Selection::new(kind, at, Side::Left);
    selection.update(at, Side::Right);
    selection
}

/// A query for the literal text, since people search for what they can see.
fn escape_regex(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn indexed(i: u8, palette: &Palette) -> u32 {
    match i {
        0..=15 => palette.ansi[i as usize],
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0u32 } else { 55 + v as u32 * 40 };
            level(i / 36) << 16 | level(i / 6 % 6) << 8 | level(i % 6)
        }
        232..=255 => {
            let v = 8 + (i as u32 - 232) * 10;
            v << 16 | v << 8 | v
        }
    }
}

fn resolve(color: Color, palette: &Palette) -> Option<u32> {
    match color {
        Color::Spec(c) => Some((c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32),
        Color::Indexed(i) => Some(indexed(i, palette)),
        Color::Named(n) => match n {
            NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::DimForeground => None,
            NamedColor::Background | NamedColor::Cursor => None,
            n => palette.ansi.get(n as usize).copied(),
        },
    }
}

fn dim(c: u32) -> u32 {
    let f = |shift: u32| (((c >> shift) & 0xff) * 2 / 3) << shift;
    f(16) | f(8) | f(0)
}

impl Pane {
    pub fn new(cols: u16, rows: u16) -> Self {
        let size = TermSize::new(cols.max(1) as usize, rows.max(1) as usize);
        Self { term: Term::new(Config { kitty_keyboard: true, ..Config::default() }, &size, VoidListener), parser: Processor::new(), anchor: None, found: None }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        if cols > 0 && rows > 0 {
            self.term.resize(TermSize::new(cols as usize, rows as usize));
        }
    }

    pub fn size(&self) -> (usize, usize) {
        (self.term.columns(), self.term.screen_lines())
    }

    /// Moves the view through the history, as many lines as `lines`, up when positive.
    pub fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_pages(&mut self, up: bool) {
        self.term.scroll_display(if up { Scroll::PageUp } else { Scroll::PageDown });
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    /// A full-screen program owns its screen, so a wheel means arrows to it.
    pub fn alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    #[cfg(test)]
    fn all_text(&self) -> Vec<String> {
        let grid = self.term.grid();
        let history = grid.history_size() as i32;
        let rows = self.size().1 as i32;
        (-history..rows).map(|y| (0..self.size().0).map(|x| grid[Line(y)][Column(x)].c).collect()).collect()
    }

    #[cfg(test)]
    fn text_lines_view(&self) -> String {
        let grid = self.term.grid();
        let offset = grid.display_offset() as i32;
        (0..self.size().0).map(|x| grid[Line(-offset)][Column(x)].c).collect()
    }

    /// The screen as plain lines, top first, without the blank ones after the last.
    pub fn text_lines(&self) -> Vec<String> {
        let grid = self.term.grid();
        let (cols, rows) = self.size();
        let mut lines: Vec<String> = (0..rows)
            .map(|y| {
                let line: String = (0..cols).map(|x| grid[Line(y as i32)][Column(x)].c).collect();
                line.trim_end().to_string()
            })
            .collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines
    }

    /// Where the view sits in the history, as the top and height of a scrollbar's thumb
    /// in fractions of its track. None with no history to scroll.
    pub fn scrollbar(&self) -> Option<(f32, f32)> {
        let grid = self.term.grid();
        let history = grid.history_size();
        if history == 0 || self.alt_screen() {
            return None;
        }
        let rows = self.term.screen_lines() as f32;
        let total = history as f32 + rows;
        Some(((history - grid.display_offset()) as f32 / total, rows / total))
    }

    /// Scrolls so the view's top is this fraction of the way down the history.
    pub fn scroll_to_fraction(&mut self, fraction: f32) {
        let history = self.term.grid().history_size() as i32;
        let total = history + self.term.screen_lines() as i32;
        let top = (fraction.clamp(0.0, 1.0) * total as f32) as i32;
        let offset = (history - top).clamp(0, history);
        let delta = offset - self.term.grid().display_offset() as i32;
        self.term.scroll_display(Scroll::Delta(delta));
    }

    /// Whether the program asked for the mouse.
    pub fn wants_mouse(&self) -> bool {
        self.term.mode().intersects(TermMode::MOUSE_MODE)
    }

    /// What to send the program for a mouse button over a cell, if it asked for the
    /// mouse. Button 0 is the left one, and 64 and 65 the wheel up and down.
    pub fn mouse_report(&self, button: u8, pressed: bool, col: usize, row: usize) -> Option<Vec<u8>> {
        let mode = *self.term.mode();
        if !mode.intersects(TermMode::MOUSE_MODE) {
            return None;
        }
        if mode.contains(TermMode::SGR_MOUSE) {
            let end = if pressed { 'M' } else { 'm' };
            return Some(format!("\x1b[<{button};{};{}{end}", col + 1, row + 1).into_bytes());
        }
        // The old encoding holds a position in a byte, with 32 added, and has one
        // code for any button let go.
        let place = |n: usize| (n.min(222) + 33) as u8;
        Some(vec![0x1b, b'[', b'M', if pressed { button } else { 3 } + 32, place(col), place(row)])
    }

    /// What to send when the wheel turns one step over a cell, if the program asked.
    pub fn wheel_report(&self, up: bool, col: usize, row: usize) -> Option<Vec<u8>> {
        self.mouse_report(if up { 64 } else { 65 }, true, col, row)
    }

    /// What to send when the mouse moves to a cell with the left button down, if the
    /// program asked to follow a drag.
    pub fn drag_report(&self, col: usize, row: usize) -> Option<Vec<u8>> {
        let follows = self.term.mode().intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION);
        follows.then(|| self.mouse_report(32, true, col, row)).flatten()
    }

    /// What to send when the mouse moves to a cell with no button down, if the program
    /// asked to follow all motion.
    pub fn motion_report(&self, col: usize, row: usize) -> Option<Vec<u8>> {
        self.term.mode().contains(TermMode::MOUSE_MOTION).then(|| self.mouse_report(35, true, col, row)).flatten()
    }

    /// Whether a paste goes between the markers the program asked for.
    pub fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    // ----- selection -----

    /// The point of a cell on screen, wherever the view is scrolled to.
    pub fn point_at(&self, col: usize, row: usize) -> Point {
        let offset = self.term.grid().display_offset() as i32;
        let (cols, rows) = self.size();
        Point::new(Line(row.min(rows.saturating_sub(1)) as i32 - offset), Column(col.min(cols.saturating_sub(1))))
    }

    pub fn select_start(&mut self, kind: SelectionType, point: Point, side: Side) {
        self.term.selection = Some(Selection::new(kind, point, side));
    }

    pub fn select_update(&mut self, point: Point, side: Side) {
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    pub fn select_clear(&mut self) {
        self.term.selection = None;
        self.anchor = None;
    }

    /// Drops a selection that covers nothing, such as from a plain click.
    pub fn select_finish(&mut self) {
        if self.term.selection.as_ref().is_some_and(Selection::is_empty) {
            self.term.selection = None;
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|t| !t.is_empty())
    }

    // ----- copy mode -----

    pub fn enter_copy(&mut self) {
        if !self.term.mode().contains(TermMode::VI) {
            self.term.selection = None;
            self.term.toggle_vi_mode();
        }
    }

    pub fn exit_copy(&mut self) {
        if self.term.mode().contains(TermMode::VI) {
            self.term.toggle_vi_mode();
        }
        self.select_clear();
        self.found = None;
        self.scroll_to_bottom();
    }

    pub fn motion(&mut self, motion: ViMotion) {
        self.found = None;
        self.term.vi_motion(motion);
    }

    /// Moves the cursor and the view by lines, up when positive.
    pub fn page(&mut self, lines: i32) {
        self.found = None;
        // The view moves with the cursor, as a page does. The last step re-reads
        // the cursor, so a selection in progress follows it too.
        let cursor = self.term.vi_mode_cursor.scroll(&self.term, lines);
        self.term.vi_mode_cursor = cursor;
        self.term.scroll_display(Scroll::Delta(lines));
        self.term.vi_goto_point(self.term.vi_mode_cursor.point);
    }

    pub fn goto_top(&mut self) {
        self.found = None;
        self.term.vi_goto_point(Point::new(self.term.topmost_line(), Column(0)));
    }

    pub fn goto_bottom(&mut self) {
        self.found = None;
        self.term.vi_goto_point(Point::new(self.term.bottommost_line(), Column(0)));
    }

    /// Starts a visual selection at the cursor, or changes the kind of one.
    pub fn visual(&mut self, kind: SelectionType) {
        match self.term.selection.as_mut() {
            Some(selection) => selection.ty = kind,
            None => {
                let at = self.term.vi_mode_cursor.point;
                self.anchor = Some(at);
                self.term.selection = Some(one_cell(kind, at));
            }
        }
    }

    pub fn swap_selection_ends(&mut self) {
        let (Some(anchor), Some(selection)) = (self.anchor, self.term.selection.as_ref()) else { return };
        let kind = selection.ty;
        let cursor = self.term.vi_mode_cursor.point;
        self.anchor = Some(cursor);
        self.term.selection = Some(one_cell(kind, cursor));
        self.term.vi_goto_point(anchor);
    }

    /// Moves to a character on this line. `till` stops the cell before it.
    pub fn find_in_line(&mut self, ch: char, back: bool, till: bool) {
        let at = self.term.vi_mode_cursor.point;
        let row = &self.term.grid()[at.line];
        let cols = self.term.columns() as i32;
        let step = if back { -1 } else { 1 };
        // A repeat of t starts past the cell it stopped short of.
        let mut col = at.column.0 as i32 + step;
        while (0..cols).contains(&col) {
            if row[Column(col as usize)].c == ch {
                let target = if till { col - step } else { col };
                self.term.vi_goto_point(Point::new(at.line, Column(target as usize)));
                return;
            }
            col += step;
        }
    }

    /// Moves to the next match, wrapping. False when there is none.
    pub fn search(&mut self, query: &str, back: bool) -> bool {
        let Ok(mut regex) = RegexSearch::new(&escape_regex(query)) else { return false };
        let cursor = self.term.vi_mode_cursor.point;
        let (origin, direction, side) = if back {
            (cursor.sub(&self.term, Boundary::None, 1), Direction::Left, Side::Left)
        } else {
            (cursor.add(&self.term, Boundary::None, 1), Direction::Right, Side::Left)
        };
        match self.term.search_next(&mut regex, origin, direction, side, None) {
            Some(found) => {
                let start = *found.start();
                self.found = Some(found);
                self.term.vi_goto_point(start);
                true
            }
            None => false,
        }
    }

    pub fn clear_match(&mut self) {
        self.found = None;
    }

    /// The row of copy mode's cursor in the whole history, from the first line on screen.
    pub fn vi_row(&self) -> i32 {
        self.term.vi_mode_cursor.point.line.0
    }

    /// Whether the program asked for the kitty keyboard protocol.
    pub fn kitty_keyboard(&self) -> bool {
        self.term.mode().intersects(TermMode::KITTY_KEYBOARD_PROTOCOL)
    }

    pub fn app_cursor(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    pub fn screen(&self, palette: &Palette) -> Screen {
        let content = self.term.renderable_content();
        // Scrolled back, lines run from minus the offset, and the cursor is below view.
        let offset = content.display_offset as i32;
        let block_cursor = content.mode.contains(TermMode::VI);
        let cursor = if block_cursor {
            usize::try_from(content.cursor.point.line.0 + offset).ok().map(|row| (content.cursor.point.column.0, row))
        } else {
            (content.mode.contains(TermMode::SHOW_CURSOR) && offset == 0)
                .then(|| (content.cursor.point.column.0, content.cursor.point.line.0.max(0) as usize))
        };
        let selection = content.selection;
        let highlight = mix(palette.bg, palette.ansi[4], 0.45);
        let found = mix(palette.bg, palette.ansi[3], 0.55);
        let mut rows: Vec<Vec<Run>> = vec![Vec::new(); self.term.screen_lines()];
        for item in content.display_iter {
            let cell = item.cell;
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let Some(row) = usize::try_from(item.point.line.0 + offset).ok().and_then(|r| rows.get_mut(r)) else {
                continue;
            };
            let mut fg = resolve(cell.fg, palette).unwrap_or(palette.fg);
            let mut bg = resolve(cell.bg, palette);
            if cell.flags.contains(Flags::INVERSE) {
                let old_fg = fg;
                fg = bg.unwrap_or(palette.bg);
                bg = Some(old_fg);
            }
            if cell.flags.contains(Flags::DIM) {
                fg = dim(fg);
            }
            if selection.is_some_and(|s| s.contains(item.point)) {
                bg = Some(highlight);
            } else if self.found.as_ref().is_some_and(|m| m.contains(&item.point)) {
                bg = Some(found);
            }
            let (bold, italic) = (cell.flags.contains(Flags::BOLD), cell.flags.contains(Flags::ITALIC));
            let underline = cell.flags.contains(Flags::UNDERLINE);
            let ch = if cell.c == '\0' { ' ' } else { cell.c };
            // An unstyled blank draws nothing, so it needs no run.
            if ch == ' ' && bg.is_none() && !underline {
                continue;
            }
            let col = item.point.column.0;
            // A fallback glyph can be wider than a cell, so only plain ASCII
            // shares a run. Everything else is placed by its own column.
            let joins = ch.is_ascii()
                && row.last().is_some_and(|r: &Run| {
                    r.text.is_ascii()
                        && r.col + r.text.len() == col
                        && (r.fg, r.bg, r.bold, r.italic, r.underline) == (fg, bg, bold, italic, underline)
                });
            if joins {
                row.last_mut().expect("joins saw a last run").text.push(ch);
            } else {
                row.push(Run { col, text: ch.to_string(), fg, bg, bold, italic, underline });
            }
        }
        Screen { rows, cursor, block_cursor }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        Palette::default()
    }

    fn text(screen: &Screen, row: usize) -> String {
        let mut line = String::new();
        for run in &screen.rows[row] {
            while line.chars().count() < run.col {
                line.push(' ');
            }
            line.push_str(&run.text);
        }
        line
    }

    #[test]
    fn plain_text_lands_on_its_row_and_moves_the_cursor() {
        let mut p = Pane::new(20, 3);
        p.feed(b"hello\r\nworld");
        let s = p.screen(&palette());
        assert_eq!(text(&s, 0), "hello");
        assert_eq!(text(&s, 1), "world");
        assert_eq!(s.cursor, Some((5, 1)));
    }

    #[test]
    fn a_colour_change_starts_a_new_run_at_its_column() {
        let mut p = Pane::new(20, 2);
        p.feed(b"ab\x1b[31mcd\x1b[0m");
        let row = &p.screen(&palette()).rows[0];
        assert_eq!(row.len(), 2);
        assert_eq!((row[1].col, row[1].text.as_str(), row[1].fg), (2, "cd", palette().ansi[1]));
    }

    #[test]
    fn a_new_palette_recolours_existing_text_without_a_reparse() {
        let mut p = Pane::new(20, 2);
        p.feed(b"\x1b[31mcd\x1b[0m plain");
        let other = Palette { fg: 0x111111, bg: 0xeeeeee, ansi: [0x222222; 16] };
        let row = &p.screen(&other).rows[0];
        assert_eq!(row[0].fg, 0x222222);
        assert_eq!(row.last().unwrap().fg, 0x111111);
    }

    #[test]
    fn inverse_swaps_foreground_and_background() {
        let mut p = Pane::new(20, 2);
        p.feed(b"\x1b[7mx");
        let run = &p.screen(&palette()).rows[0][0];
        assert_eq!((run.fg, run.bg), (palette().bg, Some(palette().fg)));
    }

    #[test]
    fn a_wide_character_keeps_the_next_cell_on_its_column() {
        let mut p = Pane::new(20, 2);
        p.feed("日x".as_bytes());
        let row = &p.screen(&palette()).rows[0];
        assert_eq!((row[0].col, row[1].col), (0, 2));
    }

    #[test]
    fn resize_changes_the_grid_and_ignores_a_zero_size() {
        let mut p = Pane::new(20, 5);
        p.resize(40, 10);
        assert_eq!(p.size(), (40, 10));
        p.resize(0, 0);
        assert_eq!(p.size(), (40, 10));
    }

    #[test]
    fn scrolling_shows_earlier_lines_and_hides_the_cursor_then_returns() {
        let mut p = Pane::new(20, 2);
        p.feed(b"one\r\ntwo\r\nthree");
        assert_eq!(text(&p.screen(&palette()), 0), "two");
        p.scroll(1);
        let up = p.screen(&palette());
        assert_eq!(text(&up, 0), "one");
        assert_eq!(text(&up, 1), "two");
        assert_eq!(up.cursor, None);
        p.scroll_to_bottom();
        assert_eq!(text(&p.screen(&palette()), 1), "three");
    }

    #[test]
    fn a_drag_selects_across_cells_and_lines_and_marks_them_on_screen() {
        let mut p = Pane::new(20, 3);
        p.feed(b"hello world\r\nsecond line");
        p.select_start(SelectionType::Simple, p.point_at(6, 0), Side::Left);
        p.select_update(p.point_at(5, 1), Side::Right);
        assert_eq!(p.selected_text().as_deref(), Some("world\nsecond"));
        let row = &p.screen(&palette()).rows[0];
        assert!(row.iter().any(|r| r.text.contains("world") && r.bg.is_some()));
    }

    #[test]
    fn a_double_click_takes_the_word_and_a_triple_click_the_line() {
        let mut p = Pane::new(20, 3);
        p.feed(b"hello world\r\nnext");
        p.select_start(SelectionType::Semantic, p.point_at(7, 0), Side::Left);
        assert_eq!(p.selected_text().as_deref(), Some("world"));
        p.select_start(SelectionType::Lines, p.point_at(7, 0), Side::Left);
        assert_eq!(p.selected_text().as_deref(), Some("hello world\n"));
    }

    #[test]
    fn a_plain_click_leaves_no_selection() {
        let mut p = Pane::new(20, 3);
        p.feed(b"hello");
        p.select_start(SelectionType::Simple, p.point_at(1, 0), Side::Left);
        p.select_finish();
        assert_eq!(p.selected_text(), None);
    }

    #[test]
    fn a_selection_follows_its_text_when_scrolled_back() {
        let mut p = Pane::new(20, 2);
        p.feed(b"one\r\ntwo\r\nthree");
        p.scroll(1);
        p.select_start(SelectionType::Simple, p.point_at(0, 0), Side::Left);
        p.select_update(p.point_at(2, 0), Side::Right);
        assert_eq!(p.selected_text().as_deref(), Some("one"));
    }

    #[test]
    fn text_lines_are_the_screen_without_trailing_blanks() {
        let mut pane = Pane::new(10, 4);
        pane.feed(b"one\r\ntwo");
        assert_eq!(pane.text_lines(), ["one", "two"]);
    }

    #[test]
    fn the_scrollbar_follows_the_view_through_the_history() {
        let mut pane = Pane::new(20, 4);
        assert_eq!(pane.scrollbar(), None);
        for i in 0..12 {
            pane.feed(format!("line {i}\r\n").as_bytes());
        }
        let (top, height) = pane.scrollbar().unwrap();
        assert!((top + height - 1.0).abs() < 0.01, "at the bottom, got {top} {height}");
        pane.scroll_to_fraction(0.0);
        assert_eq!(pane.scrollbar().unwrap().0, 0.0);
        assert!(pane.text_lines_view().starts_with("line 0"));
        pane.scroll_to_fraction(1.0);
        assert!((pane.scrollbar().unwrap().0 + height - 1.0).abs() < 0.01);
    }

    #[test]
    fn text_survives_a_pane_narrowing_and_widening_again() {
        let rows = ["/v1/sessions", "/v1/invoices", "/v1/search?q=acme", "/health", "/v1/webhooks/stripe", "/v1/users/me", "/v1/users/42", "/v1/invoices/981", "/v1/webhooks/stripe", "/v1/search?q=tile", "/v1/sessions/7", "/v1/users/me", "/health", "/v1/invoices"];
        let plain: Vec<String> = rows.iter().enumerate().map(|(i, p)| format!("14:02:{i:02} GET    {p:24} 200 12ms")).collect();
        let chains: [&[(u16, u16)]; 4] = [
            &[(54, 17), (27, 18), (25, 16), (54, 17), (52, 15), (54, 17)],
            &[(27, 18), (25, 16), (54, 16)],
            &[(54, 17), (25, 16), (54, 16)],
            &[(25, 16), (54, 17)],
        ];
        for chain in chains {
            for snapshot_first in [false, true] {
                let mut pane = if snapshot_first { Pane::new(111, 36) } else { Pane::new(54, 17) };
                if snapshot_first {
                    pane.resize(111, 36);
                    pane.resize(54, 17);
                }
                for line in &plain {
                    pane.feed(line.as_bytes());
                    pane.feed(b"\r\n");
                }
                for (c, r) in chain {
                    pane.resize(*c, *r);
                }
                let all = pane.all_text();
                let found: Vec<String> = all.iter().map(|t| t.trim_end().to_string()).filter(|t| !t.is_empty()).collect();
                assert_eq!(found, plain, "after {chain:?}, snapshot first {snapshot_first}");
            }
        }
    }

    #[test]
    fn clicks_report_a_press_and_a_release_and_drags_only_if_asked() {
        let mut pane = Pane::new(80, 24);
        assert_eq!(pane.mouse_report(0, true, 4, 5), None);
        pane.feed(b"\x1b[?1000h\x1b[?1006h");
        assert_eq!(pane.mouse_report(0, true, 4, 5).unwrap(), b"\x1b[<0;5;6M");
        assert_eq!(pane.mouse_report(0, false, 4, 5).unwrap(), b"\x1b[<0;5;6m");
        assert_eq!(pane.drag_report(6, 5), None);
        pane.feed(b"\x1b[?1002h");
        assert_eq!(pane.drag_report(6, 5).unwrap(), b"\x1b[<32;7;6M");
        pane.feed(b"\x1b[?1002l\x1b[?1006l");
        // The old encoding has one code for any button let go.
        pane.feed(b"\x1b[?1000h");
        assert_eq!(pane.mouse_report(0, false, 0, 0).unwrap(), [0x1b, b'[', b'M', 35, 33, 33]);
    }

    #[test]
    fn the_wheel_reports_to_a_program_only_if_it_asked_for_the_mouse() {
        let mut pane = Pane::new(80, 24);
        assert_eq!(pane.wheel_report(true, 1, 2), None);
        pane.feed(b"\x1b[?1000h");
        assert_eq!(pane.wheel_report(false, 1, 2).unwrap(), [0x1b, b'[', b'M', 97, 34, 35]);
        pane.feed(b"\x1b[?1006h");
        assert_eq!(pane.wheel_report(true, 1, 2).unwrap(), b"\x1b[<64;2;3M");
        pane.feed(b"\x1b[?1000l");
        assert_eq!(pane.wheel_report(true, 1, 2), None);
    }

    #[test]
    fn a_program_can_ask_for_bracketed_paste_and_the_alternate_screen() {
        let mut p = Pane::new(20, 2);
        assert!(!p.bracketed_paste() && !p.alt_screen());
        p.feed(b"\x1b[?2004h\x1b[?1049h");
        assert!(p.bracketed_paste() && p.alt_screen());
    }

    #[test]
    fn a_program_can_turn_the_kitty_keyboard_on_and_off() {
        let mut p = Pane::new(20, 2);
        assert!(!p.kitty_keyboard());
        p.feed(b"\x1b[>1u");
        assert!(p.kitty_keyboard());
        p.feed(b"\x1b[<u");
        assert!(!p.kitty_keyboard());
    }

    #[test]
    fn the_program_can_hide_the_cursor() {
        let mut p = Pane::new(20, 2);
        p.feed(b"\x1b[?25l");
        assert_eq!(p.screen(&palette()).cursor, None);
    }

    #[test]
    fn indexed_colours_cover_the_cube_and_the_greys() {
        assert_eq!(indexed(196, &palette()), 0xff0000);
        assert_eq!(indexed(232, &palette()), 0x080808);
    }
}
