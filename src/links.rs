//! Finds the link at a place in a line of terminal text.

use crate::markdown::decode;
use std::ops::Range;

/// What a piece of text points at. A file is not known to exist until someone looks.
#[derive(Debug, Clone, PartialEq)]
pub enum Link {
    Url(String),
    File { path: String, line: Option<u32> },
}

impl Link {
    /// What the link goes to, in words for the person about to click it.
    pub fn target(&self) -> String {
        match self {
            Self::Url(url) => url.clone(),
            Self::File { path, line: Some(line) } => format!("{path}:{line}"),
            Self::File { path, line: None } => path.clone(),
        }
    }
}

/// Only these open from a pane. A program chooses what it prints, so a scheme that
/// starts another app cannot be opened by one click on some text.
const WEB_SCHEMES: [&str; 3] = ["https://", "http://", "mailto:"];

/// A link a program named with a terminal hyperlink.
pub fn from_uri(uri: &str) -> Option<Link> {
    if WEB_SCHEMES.iter().any(|scheme| uri.starts_with(scheme)) {
        return Some(Link::Url(uri.to_string()));
    }
    let rest = uri.strip_prefix("file://")?;
    let path = decode(&rest[rest.find('/')?..])?;
    Some(Link::File { path, line: None })
}

fn trim_end_of_url(text: &[char]) -> usize {
    let mut end = text.len();
    while end > 0 {
        let c = text[end - 1];
        let open = |o: char| text[..end].iter().filter(|x| **x == o).count();
        let unbalanced = (c == ')' && open(')') > open('(')) || (c == ']' && open(']') > open('['));
        if !(unbalanced || ".,;:!?'\"`>".contains(c)) {
            break;
        }
        end -= 1;
    }
    end
}

/// `a/b.rs:12:5` as the path and line. A path ends at the first number after a colon.
fn split_line(text: &str) -> (String, Option<u32>) {
    let mut parts = text.split(':');
    let path = parts.next().unwrap_or_default().to_string();
    (path, parts.next().and_then(|n| n.parse().ok()))
}

fn looks_like_path(path: &str) -> bool {
    let name_like = path.rsplit('/').next().is_some_and(|last| last.contains('.') && !last.ends_with('.'));
    let has_letter = path.chars().any(char::is_alphabetic);
    has_letter && (path.contains('/') || path.starts_with('~') || name_like)
}

/// The link at character `at` of a line, with the characters it covers.
pub fn find(line: &str, at: usize) -> Option<(Range<usize>, Link)> {
    let chars: Vec<char> = line.chars().collect();
    if chars.get(at).is_none_or(|c| c.is_whitespace()) {
        return None;
    }
    let is_space = |i: &usize| chars[*i].is_whitespace();
    let start = (0..at).rev().find(is_space).map_or(0, |i| i + 1);
    let end = (at..chars.len()).find(is_space).unwrap_or(chars.len());
    let word: String = chars[start..end].iter().collect();

    for scheme in WEB_SCHEMES {
        let Some(byte) = word.find(scheme) else { continue };
        let from = start + word[..byte].chars().count();
        let until = from + trim_end_of_url(&chars[from..end]);
        if (from..until).contains(&at) {
            let url: String = chars[from..until].iter().collect();
            return Some((from..until, Link::Url(url)));
        }
    }

    let stops = |c: char| "()[]{}<>\"'`,;=|".contains(c);
    let from = (start..at).rev().find(|i| stops(chars[*i])).map_or(start, |i| i + 1);
    let mut until = (at..end).find(|i| stops(chars[*i])).unwrap_or(end);
    while until > from && ".:,".contains(chars[until - 1]) {
        until -= 1;
    }
    if !(from..until).contains(&at) {
        return None;
    }
    let text: String = chars[from..until].iter().collect();
    let (path, line) = split_line(&text);
    looks_like_path(&path).then_some((from..until, Link::File { path, line }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link_in(line: &str, word: &str) -> Option<(String, Link)> {
        let at = line.find(word).expect("the word is in the line");
        let at = line[..at].chars().count();
        find(line, at + 1).map(|(range, link)| (line.chars().skip(range.start).take(range.len()).collect(), link))
    }

    #[test]
    fn an_address_stops_before_the_punctuation_around_it() {
        let (text, link) = link_in("see (https://a.dev/x_(y)?q=1), then", "https").unwrap();
        assert_eq!(text, "https://a.dev/x_(y)?q=1");
        assert_eq!(link, Link::Url(text));
        let (text, _) = link_in("at https://a.dev.", "https").unwrap();
        assert_eq!(text, "https://a.dev");
    }

    #[test]
    fn a_path_keeps_its_line_number() {
        assert_eq!(link_in("error in src/main.rs:12:5: bad", "src").unwrap().1, Link::File { path: "src/main.rs".into(), line: Some(12) });
        assert_eq!(link_in("open ~/notes.md now", "~/").unwrap().1, Link::File { path: "~/notes.md".into(), line: None });
        assert_eq!(link_in("wrote README.md.", "README").unwrap().1, Link::File { path: "README.md".into(), line: None });
        assert_eq!(link_in("file=\"a/b.txt\"", "a/b").unwrap().0, "a/b.txt");
    }

    #[test]
    fn plain_words_are_not_links() {
        assert_eq!(link_in("hello world", "hello"), None);
        assert_eq!(link_in("1.2.3 released", "1.2"), None);
        assert_eq!(find("a b", 1), None);
    }

    #[test]
    fn a_program_cannot_start_another_app_with_a_link() {
        assert_eq!(from_uri("https://a.dev"), Some(Link::Url("https://a.dev".into())));
        assert_eq!(from_uri("x-evil://run"), None);
        assert_eq!(from_uri("file://mac/a%20b.txt"), Some(Link::File { path: "/a b.txt".into(), line: None }));
    }
}
