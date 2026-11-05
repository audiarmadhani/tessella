//! A searchable list: themes, the command palette, sessions, workspaces and more.

pub const VISIBLE: usize = 9;

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub label: String,
    pub detail: String,
    /// What choosing this item acts on.
    pub value: String,
    /// Starred, in a list that has favourites.
    pub favourite: bool,
}

impl Item {
    pub fn new(label: impl Into<String>, detail: impl Into<String>, value: impl Into<String>) -> Self {
        Self { label: label.into(), detail: detail.into(), value: value.into(), favourite: false }
    }
}

/// Marks the favourites and moves them to the top, in the order they were starred.
/// Everything else keeps its order. A favourite no longer in the list is skipped.
pub fn favourites_first(items: Vec<Item>, favourites: &[String]) -> Vec<Item> {
    let mut out: Vec<Item> = favourites
        .iter()
        .filter_map(|f| items.iter().find(|i| i.value == *f).cloned())
        .map(|mut i| {
            i.favourite = true;
            i
        })
        .collect();
    out.extend(items.into_iter().filter(|i| !favourites.contains(&i.value)));
    out
}

#[derive(Debug, Clone)]
pub struct Picker {
    items: Vec<Item>,
    pub query: String,
    pub selected: usize,
    /// The value in force when the picker opened, to go back to on cancel.
    pub original: String,
}

impl Picker {
    pub fn new(items: Vec<Item>, original: String) -> Self {
        let selected = items.iter().position(|i| i.value == original).unwrap_or(0);
        Self { items, query: String::new(), selected, original }
    }

    /// Replaces the items, keeping the selection on the same item and the query.
    pub fn set_items(&mut self, items: Vec<Item>) {
        let at = self.current().map(|i| i.value);
        self.items = items;
        self.selected = at.and_then(|v| self.filtered().iter().position(|i| i.value == v)).unwrap_or(0);
    }

    /// Starts on the first row, so that favourites at the top are in view.
    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    /// Every word of the query must appear in the label, the detail or the value.
    pub fn filtered(&self) -> Vec<&Item> {
        let query = self.query.to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        self.items
            .iter()
            .filter(|i| {
                let hay = format!("{} {} {}", i.label, i.detail, i.value).to_lowercase();
                words.iter().all(|w| hay.contains(w))
            })
            .collect()
    }

    pub fn current(&self) -> Option<Item> {
        self.filtered().get(self.selected).map(|i| (*i).clone())
    }

    /// The first row to show, keeping the selection on screen.
    pub fn first_visible(&self, count: usize) -> usize {
        self.selected.saturating_sub(VISIBLE / 2).min(count.saturating_sub(VISIBLE))
    }

    /// Moves the selection, wrapping at the ends.
    pub fn step(&mut self, by: i32) {
        let count = self.filtered().len() as i32;
        if count > 0 {
            self.selected = (self.selected as i32 + by).rem_euclid(count) as usize;
        }
    }

    pub fn push(&mut self, text: &str) {
        self.query.push_str(text);
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker() -> Picker {
        let names = ["dracula", "nord", "tokyo_night", "tokyo_night_light", "ayu_light"];
        let items = names.map(|n| Item::new(n, "", n)).to_vec();
        Picker::new(items, "nord".into())
    }

    fn current(p: &Picker) -> Option<String> {
        p.current().map(|i| i.value)
    }

    #[test]
    fn favourites_go_first_in_their_own_order_and_are_marked() {
        let items = ["a", "b", "c", "d"].map(|n| Item::new(n, "", n)).to_vec();
        let out = favourites_first(items, &["c".to_string(), "gone".to_string(), "a".to_string()]);
        let order: Vec<_> = out.iter().map(|i| (i.value.as_str(), i.favourite)).collect();
        assert_eq!(order, [("c", true), ("a", true), ("b", false), ("d", false)]);
    }

    #[test]
    fn starring_keeps_the_selection_on_the_same_item() {
        let mut p = picker();
        p.push("o");
        let before = current(&p);
        let items = ["dracula", "nord", "tokyo_night", "tokyo_night_light", "ayu_light"].map(|n| Item::new(n, "", n)).to_vec();
        p.set_items(favourites_first(items, &["tokyo_night_light".to_string()]));
        assert_eq!(current(&p), before);
        assert_eq!(p.query, "o");
    }

    #[test]
    fn it_opens_on_the_value_in_force() {
        assert_eq!(current(&picker()).as_deref(), Some("nord"));
    }

    #[test]
    fn every_word_of_the_query_must_match() {
        let mut p = picker();
        p.push("tokyo LIGHT");
        assert_eq!(p.filtered().len(), 1);
        assert_eq!(current(&p).as_deref(), Some("tokyo_night_light"));
    }

    #[test]
    fn a_query_matches_the_label_or_the_detail() {
        let items = vec![Item::new("New window", "prefix+c", "new_window"), Item::new("Close", "prefix+x", "close")];
        let mut p = Picker::new(items, String::new());
        p.push("prefix+x");
        assert_eq!(current(&p).as_deref(), Some("close"));
    }

    #[test]
    fn typing_returns_the_selection_to_the_first_match() {
        let mut p = picker();
        p.push("light");
        assert_eq!(current(&p).as_deref(), Some("tokyo_night_light"));
        p.backspace();
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn steps_wrap_at_both_ends() {
        let mut p = picker();
        p.selected = 0;
        p.step(-1);
        assert_eq!(current(&p).as_deref(), Some("ayu_light"));
        p.step(1);
        assert_eq!(current(&p).as_deref(), Some("dracula"));
    }

    #[test]
    fn no_match_selects_nothing_and_stepping_is_safe() {
        let mut p = picker();
        p.push("zzz");
        p.step(1);
        assert_eq!(p.current(), None);
    }

    #[test]
    fn the_window_of_rows_keeps_the_selection_visible() {
        let mut p = picker();
        p.selected = 40;
        assert_eq!(p.first_visible(100), 36);
        p.selected = 99;
        assert_eq!(p.first_visible(100), 91);
        assert_eq!(p.first_visible(3), 0);
    }
}
