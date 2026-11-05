//! Copy mode: tuios's fixed vim keys over a pane's screen and history.

use crate::terminal::{Pane, SelectionType, ViMotion};

#[derive(Debug, Clone, Copy, PartialEq)]
struct Find {
    ch: char,
    back: bool,
    /// `t` and `T` stop one cell short.
    till: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct SearchPrompt {
    query: String,
    back: bool,
}

/// What a key asks the app to do.
#[derive(Debug, PartialEq)]
pub enum Effect {
    Stay,
    Exit,
    /// Copy this, then leave copy mode.
    Yank(String),
    Note(String),
}

#[derive(Debug, Default)]
pub struct CopyMode {
    prompt: Option<SearchPrompt>,
    pending_g: bool,
    pending_find: Option<(bool, bool)>,
    last_find: Option<Find>,
    query: String,
    back: bool,
    visual: Option<SelectionType>,
}

impl CopyMode {
    /// A copy mode that opens with the search prompt up, for find.
    pub fn searching() -> Self {
        Self { prompt: Some(SearchPrompt::default()), ..Self::default() }
    }

    /// What the dock shows.
    pub fn label(&self) -> String {
        if let Some(p) = &self.prompt {
            return format!("{}{}", if p.back { '?' } else { '/' }, p.query);
        }
        match self.visual {
            Some(SelectionType::Lines) => "VISUAL LINE".to_string(),
            Some(SelectionType::Block) => "VISUAL BLOCK".to_string(),
            Some(_) => "VISUAL".to_string(),
            None => "COPY".to_string(),
        }
    }

    fn search(&mut self, pane: &mut Pane, back: bool) -> Effect {
        if self.query.is_empty() {
            return Effect::Stay;
        }
        if pane.search(&self.query, back) {
            Effect::Stay
        } else {
            Effect::Note(format!("No match for {}", self.query))
        }
    }

    fn prompt_key(&mut self, pane: &mut Pane, key: &str) -> Effect {
        let Some(prompt) = self.prompt.as_mut() else { return Effect::Stay };
        match key {
            "esc" => self.prompt = None,
            "backspace" => {
                prompt.query.pop();
            }
            "space" => prompt.query.push(' '),
            "enter" => {
                let done = self.prompt.take().unwrap_or_default();
                self.query = done.query;
                self.back = done.back;
                return self.search(pane, done.back);
            }
            k if k.chars().count() == 1 => prompt.query.push_str(k),
            _ => {}
        }
        Effect::Stay
    }

    fn set_visual(&mut self, pane: &mut Pane, kind: SelectionType) {
        // The same key again leaves the visual mode it entered.
        if self.visual == Some(kind) {
            self.visual = None;
            pane.select_clear();
        } else {
            self.visual = Some(kind);
            pane.visual(kind);
        }
    }

    fn find(&self, pane: &mut Pane, f: Find, reverse: bool) {
        pane.find_in_line(f.ch, f.back != reverse, f.till);
    }

    pub fn key(&mut self, pane: &mut Pane, key: &str) -> Effect {
        if self.prompt.is_some() {
            return self.prompt_key(pane, key);
        }
        if let Some((back, till)) = self.pending_find.take() {
            if let Some(ch) = single(key) {
                let f = Find { ch, back, till };
                self.last_find = Some(f);
                self.find(pane, f, false);
            }
            return Effect::Stay;
        }
        if std::mem::take(&mut self.pending_g) && key == "g" {
            pane.goto_top();
            return Effect::Stay;
        }
        let (rows, _) = (pane.size().1 as i32, ());
        let motion = match key {
            "h" | "left" => Some(ViMotion::Left),
            "l" | "right" => Some(ViMotion::Right),
            "j" | "down" => Some(ViMotion::Down),
            "k" | "up" => Some(ViMotion::Up),
            "w" => Some(ViMotion::SemanticRight),
            "b" => Some(ViMotion::SemanticLeft),
            "e" => Some(ViMotion::SemanticRightEnd),
            "W" => Some(ViMotion::WordRight),
            "B" => Some(ViMotion::WordLeft),
            "E" => Some(ViMotion::WordRightEnd),
            "0" | "home" => Some(ViMotion::First),
            "^" => Some(ViMotion::FirstOccupied),
            "$" | "end" => Some(ViMotion::Last),
            "H" => Some(ViMotion::High),
            "M" => Some(ViMotion::Middle),
            "L" => Some(ViMotion::Low),
            "{" => Some(ViMotion::ParagraphUp),
            "}" => Some(ViMotion::ParagraphDown),
            "%" => Some(ViMotion::Bracket),
            _ => None,
        };
        if let Some(m) = motion {
            pane.motion(m);
            return Effect::Stay;
        }
        match key {
            "esc" | "q" => {
                if self.visual.take().is_some() {
                    pane.select_clear();
                    return Effect::Stay;
                }
                return Effect::Exit;
            }
            "i" if self.visual.is_none() => return Effect::Exit,
            "ctrl+u" => pane.page(rows / 2),
            "ctrl+d" => pane.page(-rows / 2),
            "ctrl+b" | "pgup" => pane.page(rows),
            "ctrl+f" | "pgdown" => pane.page(-rows),
            "g" => self.pending_g = true,
            "G" => pane.goto_bottom(),
            "f" => self.pending_find = Some((false, false)),
            "F" => self.pending_find = Some((true, false)),
            "t" => self.pending_find = Some((false, true)),
            "T" => self.pending_find = Some((true, true)),
            ";" | "," => {
                if let Some(f) = self.last_find {
                    self.find(pane, f, key == ",");
                }
            }
            "/" | "?" => self.prompt = Some(SearchPrompt { query: String::new(), back: key == "?" }),
            "n" => return self.search(pane, self.back),
            "N" => return self.search(pane, !self.back),
            "ctrl+l" => {
                self.query.clear();
                pane.clear_match();
            }
            "v" => self.set_visual(pane, SelectionType::Simple),
            "V" => self.set_visual(pane, SelectionType::Lines),
            "ctrl+v" => self.set_visual(pane, SelectionType::Block),
            "o" if self.visual.is_some() => pane.swap_selection_ends(),
            "y" | "c" if self.visual.is_some() => {
                return match pane.selected_text() {
                    Some(text) if !text.is_empty() => Effect::Yank(text),
                    _ => Effect::Stay,
                };
            }
            _ => {}
        }
        Effect::Stay
    }
}

fn single(key: &str) -> Option<char> {
    if key == "space" {
        return Some(' ');
    }
    let mut chars = key.chars();
    let ch = chars.next()?;
    chars.next().is_none().then_some(ch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Palette;

    fn pane(text: &str) -> Pane {
        let mut p = Pane::new(30, 4);
        p.feed(text.replace('\n', "\r\n").as_bytes());
        p.enter_copy();
        p
    }

    fn keys(c: &mut CopyMode, p: &mut Pane, list: &str) -> Effect {
        let mut last = Effect::Stay;
        for k in list.split(' ') {
            last = c.key(p, k);
        }
        last
    }

    #[test]
    fn q_leaves_and_esc_leaves_visual_first() {
        let (mut c, mut p) = (CopyMode::default(), pane("hello"));
        assert_eq!(c.key(&mut p, "v"), Effect::Stay);
        assert_eq!(c.label(), "VISUAL");
        assert_eq!(c.key(&mut p, "esc"), Effect::Stay);
        assert_eq!(c.label(), "COPY");
        assert_eq!(p.selected_text().unwrap_or_default(), "");
        assert_eq!(c.key(&mut p, "q"), Effect::Exit);
    }

    #[test]
    fn a_word_motion_and_y_copy_exactly_the_word() {
        let (mut c, mut p) = (CopyMode::default(), pane("alpha beta gamma"));
        // The cursor starts past the text: go to the line's start.
        let got = keys(&mut c, &mut p, "0 w v e y");
        assert_eq!(got, Effect::Yank("beta".into()));
    }

    #[test]
    fn line_visual_copies_whole_lines_with_their_breaks() {
        let (mut c, mut p) = (CopyMode::default(), pane("one\ntwo\nthree"));
        let got = keys(&mut c, &mut p, "k V k y");
        assert_eq!(got, Effect::Yank("one\ntwo\n".into()));
    }

    #[test]
    fn block_visual_copies_a_column_from_each_line() {
        let (mut c, mut p) = (CopyMode::default(), pane("abcd\nefgh"));
        let got = keys(&mut c, &mut p, "0 k l ctrl+v j l y");
        assert_eq!(got, Effect::Yank("bc\nfg".into()));
    }

    #[test]
    fn o_swaps_the_ends_so_the_selection_can_grow_the_other_way() {
        let (mut c, mut p) = (CopyMode::default(), pane("abcdef"));
        let got = keys(&mut c, &mut p, "0 l l v l o h y");
        assert_eq!(got, Effect::Yank("bcd".into()));
    }

    #[test]
    fn f_finds_a_character_and_t_stops_short_and_semicolon_repeats() {
        let (mut c, mut p) = (CopyMode::default(), pane("a-b-c-d"));
        assert_eq!(keys(&mut c, &mut p, "0 v f - y"), Effect::Yank("a-".into()));
        let (mut c, mut p) = (CopyMode::default(), pane("a-b-c-d"));
        assert_eq!(keys(&mut c, &mut p, "0 v t - y"), Effect::Yank("a".into()));
        let (mut c, mut p) = (CopyMode::default(), pane("a-b-c-d"));
        assert_eq!(keys(&mut c, &mut p, "0 v f - ; y"), Effect::Yank("a-b-".into()));
    }

    #[test]
    fn gg_reaches_the_oldest_line_and_g_the_newest() {
        let mut p = pane("1\n2\n3\n4\n5\n6\n7\n8");
        let mut c = CopyMode::default();
        assert_eq!(keys(&mut c, &mut p, "g g V y"), Effect::Yank("1\n".into()));
        let mut c = CopyMode::default();
        p.select_clear();
        assert_eq!(keys(&mut c, &mut p, "G V y"), Effect::Yank("8\n".into()));
    }

    #[test]
    fn a_page_up_scrolls_the_history_into_view() {
        let mut p = pane("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12");
        let mut c = CopyMode::default();
        c.key(&mut p, "ctrl+b");
        let top = p.screen(&Palette::default());
        assert_eq!(top.rows[0][0].text, "5");
    }

    #[test]
    fn search_moves_to_a_match_and_n_goes_on_and_a_miss_says_so() {
        let mut p = pane("foo one\nbar\nfoo two\nbaz");
        let mut c = CopyMode::default();
        keys(&mut c, &mut p, "/ t w o enter");
        assert_eq!(keys(&mut c, &mut p, "v e y"), Effect::Yank("two".into()));

        let mut p = pane("foo one\nbar\nfoo two\nbaz");
        let mut c = CopyMode::default();
        keys(&mut c, &mut p, "? f o o enter");
        assert_eq!(p.vi_row(), 2);
        c.key(&mut p, "n");
        assert_eq!(p.vi_row(), 0);
        c.key(&mut p, "N");
        assert_eq!(p.vi_row(), 2);
        assert_eq!(keys(&mut c, &mut p, "/ z z z enter"), Effect::Note("No match for zzz".into()));
    }

    #[test]
    fn a_search_finds_characters_regex_would_treat_as_special() {
        let mut p = pane("cost (usd)\nnothing");
        let mut c = CopyMode::default();
        assert_eq!(keys(&mut c, &mut p, "/ ( u s d ) enter"), Effect::Stay);
        assert_eq!(p.vi_row(), 0);
    }

    #[test]
    fn the_search_prompt_edits_and_cancels_and_shows_in_the_label() {
        let mut p = pane("text");
        let mut c = CopyMode::searching();
        keys(&mut c, &mut p, "a b backspace");
        assert_eq!(c.label(), "/a");
        c.key(&mut p, "esc");
        assert_eq!(c.label(), "COPY");
    }

    #[test]
    fn y_outside_visual_does_nothing() {
        let (mut c, mut p) = (CopyMode::default(), pane("text"));
        assert_eq!(c.key(&mut p, "y"), Effect::Stay);
    }

    #[test]
    fn leaving_copy_mode_returns_to_the_live_screen() {
        let mut p = pane("1\n2\n3\n4\n5\n6\n7");
        p.goto_top();
        p.exit_copy();
        let s = p.screen(&Palette::default());
        assert_eq!(s.rows[3][0].text, "7");
        assert!(s.cursor.is_some() && !s.block_cursor);
    }
}
