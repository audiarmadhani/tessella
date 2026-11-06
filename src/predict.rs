//! Shows letters typed into a pane on another machine before that machine echoes them,
//! so that typing feels immediate on a slow link. The keys still go to the program as
//! they always do. These guesses only change what is drawn, until real output replaces
//! them.
//!
//! A program may not echo what it reads, as at a password prompt or in vim's normal
//! mode. So a guess shows only after the program has echoed a key since the last Enter,
//! arrow or other key that can change what typing means. The same idea is in mosh.

use std::time::{Duration, Instant};

/// How long a guess waits for its echo. A key the program has not echoed by then is
/// probably one it will not echo.
pub const PATIENCE: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guess {
    pub col: usize,
    pub row: usize,
    pub ch: char,
}

#[derive(Debug, Default)]
pub struct Predictor {
    guesses: Vec<Guess>,
    /// The program has echoed a key since the last reset.
    trusted: bool,
    /// When the oldest guess began to wait.
    waiting_since: Option<Instant>,
    row: Option<usize>,
}

fn blank(ch: char) -> bool {
    ch == ' ' || ch == '\0'
}

impl Predictor {
    /// Forgets every guess, and asks for a new echo before showing another.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn overdue(&self, now: Instant) -> bool {
        self.waiting_since.is_some_and(|since| now.duration_since(since) >= PATIENCE)
    }

    /// Notes a typed character. Letters land on blank cells only: over existing text,
    /// the program may insert, and only it knows what the line then looks like.
    pub fn typed(&mut self, ch: char, cursor: Option<(usize, usize)>, cols: usize, cell: impl Fn(usize, usize) -> char, now: Instant) {
        if self.overdue(now) {
            self.reset();
        }
        let at = self.guesses.last().map(|g| (g.col + 1, g.row)).or(cursor);
        let Some((col, row)) = at else { return self.reset() };
        if col + 1 >= cols || !blank(cell(col, row)) {
            return self.reset();
        }
        if self.guesses.is_empty() {
            self.waiting_since = Some(now);
        }
        self.row = Some(row);
        self.guesses.push(Guess { col, row, ch });
    }

    /// Notes a backspace. With no guess to take back, the echo will show its effect.
    pub fn erased(&mut self) {
        self.guesses.pop();
        if self.guesses.is_empty() {
            self.waiting_since = None;
        }
    }

    /// Compares the guesses with the screen after new output.
    pub fn settle(&mut self, cursor: Option<(usize, usize)>, cell: impl Fn(usize, usize) -> char, now: Instant) {
        if self.guesses.is_empty() && !self.trusted {
            return;
        }
        if self.overdue(now) {
            return self.reset();
        }
        // Text scrolled or the program moved on, so this is no longer the same line.
        if let Some((_, row)) = cursor {
            if self.row.is_some_and(|r| r != row) {
                return self.reset();
            }
            self.row = Some(row);
        }
        while let Some(&guess) = self.guesses.first() {
            let found = cell(guess.col, guess.row);
            // A blank cell looks like an echoed space, so that needs the cursor to pass it.
            let echoed = if blank(guess.ch) {
                blank(found) && cursor.is_some_and(|(col, row)| row == guess.row && col > guess.col)
            } else {
                found == guess.ch
            };
            if echoed {
                self.guesses.remove(0);
                self.trusted = true;
                self.waiting_since = if self.guesses.is_empty() { None } else { Some(now) };
            } else if blank(found) {
                break;
            } else {
                return self.reset();
            }
        }
    }

    /// The guesses to draw, oldest first.
    pub fn shown(&self, now: Instant) -> &[Guess] {
        if self.trusted && !self.overdue(now) {
            &self.guesses
        } else {
            &[]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line as the emulator holds it. Everything else is blank.
    fn line(text: &str) -> impl Fn(usize, usize) -> char + '_ {
        move |col, row| if row == 0 { text.chars().nth(col).unwrap_or(' ') } else { ' ' }
    }

    fn at(p: &Predictor, now: Instant) -> String {
        p.shown(now).iter().map(|g| g.ch).collect()
    }

    /// A predictor that has already seen the program echo a key.
    fn trusted(now: Instant) -> Predictor {
        let mut p = Predictor::default();
        p.typed('a', Some((0, 0)), 80, line(""), now);
        p.settle(Some((1, 0)), line("a"), now);
        p
    }

    #[test]
    fn the_first_key_waits_for_a_real_echo() {
        let now = Instant::now();
        let mut p = Predictor::default();
        p.typed('s', Some((0, 0)), 80, line(""), now);
        assert_eq!(at(&p, now), "");
        p.settle(Some((1, 0)), line("s"), now);
        assert_eq!(at(&p, now), "");
        p.typed('e', Some((1, 0)), 80, line("s"), now);
        assert_eq!(at(&p, now), "e");
    }

    #[test]
    fn a_password_prompt_never_shows_a_letter() {
        let now = Instant::now();
        let mut p = Predictor::default();
        for ch in "hunter2".chars() {
            p.typed(ch, Some((10, 0)), 80, line("Password: "), now);
            assert_eq!(at(&p, now), "");
        }
        p.settle(Some((10, 0)), line("Password: "), now);
        assert_eq!(at(&p, now), "");
    }

    #[test]
    fn keys_typed_ahead_of_the_echo_follow_each_other() {
        let now = Instant::now();
        let mut p = trusted(now);
        p.typed('b', Some((1, 0)), 80, line("a"), now);
        p.typed('c', Some((1, 0)), 80, line("a"), now);
        let columns: Vec<usize> = p.shown(now).iter().map(|g| g.col).collect();
        assert_eq!(columns, [1, 2]);
    }

    #[test]
    fn an_echo_removes_the_guess_it_confirms() {
        let now = Instant::now();
        let mut p = trusted(now);
        p.typed('b', Some((1, 0)), 80, line("a"), now);
        p.typed('c', Some((1, 0)), 80, line("a"), now);
        p.settle(Some((2, 0)), line("ab"), now);
        assert_eq!(at(&p, now), "c");
        p.settle(Some((3, 0)), line("abc"), now);
        assert_eq!(at(&p, now), "");
    }

    #[test]
    fn a_wrong_guess_goes_and_the_next_must_earn_trust_again() {
        let now = Instant::now();
        let mut p = trusted(now);
        p.typed('b', Some((1, 0)), 80, line("a"), now);
        p.settle(Some((2, 0)), line("a*"), now);
        assert_eq!(at(&p, now), "");
        p.typed('c', Some((2, 0)), 80, line("a*"), now);
        assert_eq!(at(&p, now), "");
    }

    #[test]
    fn a_guess_with_no_echo_runs_out_of_patience() {
        let now = Instant::now();
        let mut p = trusted(now);
        p.typed('b', Some((1, 0)), 80, line("a"), now);
        assert_eq!(at(&p, now), "b");
        assert_eq!(at(&p, now + PATIENCE), "");
        p.settle(Some((1, 0)), line("a"), now + PATIENCE);
        p.typed('c', Some((1, 0)), 80, line("a"), now + PATIENCE);
        assert_eq!(at(&p, now + PATIENCE), "");
    }

    #[test]
    fn a_slow_link_keeps_a_steady_stream_of_guesses_alive() {
        let now = Instant::now();
        let mut p = trusted(now);
        p.typed('b', Some((1, 0)), 80, line("a"), now);
        p.typed('c', Some((1, 0)), 80, line("a"), now + Duration::from_secs(2));
        let later = now + Duration::from_secs(2);
        p.settle(Some((2, 0)), line("ab"), later);
        assert_eq!(at(&p, later + Duration::from_secs(2)), "c");
    }

    #[test]
    fn text_in_the_way_stops_the_guessing() {
        let now = Instant::now();
        let mut p = trusted(now);
        p.typed('b', Some((0, 0)), 80, line("a tail"), now);
        assert_eq!(at(&p, now), "");
    }

    #[test]
    fn a_line_end_stops_the_guessing() {
        let now = Instant::now();
        let mut p = trusted(now);
        p.typed('b', Some((4, 0)), 5, line("a"), now);
        assert_eq!(at(&p, now), "");
    }

    #[test]
    fn a_backspace_takes_back_the_last_guess() {
        let now = Instant::now();
        let mut p = trusted(now);
        p.typed('b', Some((1, 0)), 80, line("a"), now);
        p.typed('c', Some((1, 0)), 80, line("a"), now);
        p.erased();
        assert_eq!(at(&p, now), "b");
        p.typed('d', Some((1, 0)), 80, line("a"), now);
        let last = *p.shown(now).last().unwrap();
        assert_eq!((last.col, last.ch), (2, 'd'));
    }

    #[test]
    fn a_new_row_ends_the_trust() {
        let now = Instant::now();
        let mut p = trusted(now);
        p.settle(Some((0, 1)), line("a"), now);
        p.typed('b', Some((0, 1)), 80, line("a"), now);
        assert_eq!(at(&p, now), "");
    }

    #[test]
    fn a_space_counts_only_once_the_cursor_passes_it() {
        let now = Instant::now();
        let mut p = trusted(now);
        p.typed(' ', Some((1, 0)), 80, line("a"), now);
        p.settle(Some((1, 0)), line("a"), now);
        assert_eq!(at(&p, now), " ");
        p.settle(Some((2, 0)), line("a"), now);
        assert_eq!(at(&p, now), "");
    }

    #[test]
    fn a_reset_forgets_all_of_it() {
        let now = Instant::now();
        let mut p = trusted(now);
        p.typed('b', Some((1, 0)), 80, line("a"), now);
        p.reset();
        assert_eq!(at(&p, now), "");
        p.typed('c', Some((1, 0)), 80, line("a"), now);
        assert_eq!(at(&p, now), "");
    }
}
