//! Markdown as blocks to draw, and the file address of a pane that shows one.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use crate::files::Place;
use std::path::Path;

/// A piece of text with one style.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading { level: u8, spans: Vec<Span> },
    Paragraph(Vec<Span>),
    /// A list item's text, `depth` lists in. The marker is empty on its later paragraphs.
    Item { depth: usize, marker: String, spans: Vec<Span> },
    Quote(Vec<Span>),
    Code { lines: Vec<String> },
    Rule,
    /// Rows of cells. The first row is the header.
    Table(Vec<Vec<Vec<Span>>>),
}

/// The style in force while parsing.
#[derive(Default)]
struct Style {
    bold: usize,
    italic: usize,
    strike: usize,
    link: Option<String>,
}

#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    spans: Vec<Span>,
    style: Style,
    /// The lists open now: the next number of an ordered one.
    lists: Vec<Option<u64>>,
    /// The marker of a list item whose text has not been written yet.
    marker: Option<String>,
    quote: usize,
    heading: Option<u8>,
    code: Option<String>,
    table: Option<Vec<Vec<Vec<Span>>>>,
    row: Vec<Vec<Span>>,
}

impl Builder {
    fn push(&mut self, text: &str, code: bool) {
        let s = &self.style;
        self.spans.push(Span {
            text: text.to_string(),
            bold: s.bold > 0,
            italic: s.italic > 0,
            strike: s.strike > 0,
            code,
            link: s.link.clone(),
        });
    }

    /// Writes the text so far as the block it belongs to.
    fn flush(&mut self) {
        if self.spans.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.spans);
        let block = if let Some(level) = self.heading {
            Block::Heading { level, spans }
        } else if !self.lists.is_empty() {
            Block::Item { depth: self.lists.len() - 1, marker: self.marker.take().unwrap_or_default(), spans }
        } else if self.quote > 0 {
            Block::Quote(spans)
        } else {
            Block::Paragraph(spans)
        };
        self.blocks.push(block);
    }
}

fn level_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

pub fn parse(text: &str) -> Vec<Block> {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut b = Builder::default();
    for event in Parser::new_ext(text, options) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => b.heading = Some(level_number(level)),
                Tag::BlockQuote(_) => {
                    b.flush();
                    b.quote += 1;
                }
                Tag::CodeBlock(kind) => {
                    b.flush();
                    let _ = matches!(kind, CodeBlockKind::Fenced(_));
                    b.code = Some(String::new());
                }
                Tag::List(first) => {
                    b.flush();
                    b.lists.push(first);
                }
                Tag::Item => {
                    b.flush();
                    b.marker = Some(match b.lists.last_mut() {
                        Some(Some(n)) => {
                            let marker = format!("{n}.");
                            *n += 1;
                            marker
                        }
                        _ => "•".to_string(),
                    });
                }
                Tag::Emphasis => b.style.italic += 1,
                Tag::Strong => b.style.bold += 1,
                Tag::Strikethrough => b.style.strike += 1,
                Tag::Link { dest_url, .. } => b.style.link = Some(dest_url.to_string()),
                Tag::Image { dest_url, .. } => b.style.link = Some(dest_url.to_string()),
                Tag::Table(_) => b.table = Some(Vec::new()),
                Tag::TableHead | Tag::TableRow => b.row = Vec::new(),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) => {
                    b.flush();
                    b.heading = None;
                }
                TagEnd::Paragraph | TagEnd::Item => b.flush(),
                TagEnd::BlockQuote(_) => {
                    b.flush();
                    b.quote = b.quote.saturating_sub(1);
                }
                TagEnd::CodeBlock => {
                    let code = b.code.take().unwrap_or_default();
                    b.blocks.push(Block::Code { lines: code.trim_end_matches('\n').split('\n').map(str::to_string).collect() });
                }
                TagEnd::List(_) => {
                    b.flush();
                    b.lists.pop();
                }
                TagEnd::Emphasis => b.style.italic = b.style.italic.saturating_sub(1),
                TagEnd::Strong => b.style.bold = b.style.bold.saturating_sub(1),
                TagEnd::Strikethrough => b.style.strike = b.style.strike.saturating_sub(1),
                TagEnd::Link | TagEnd::Image => b.style.link = None,
                TagEnd::TableCell => {
                    let cell = std::mem::take(&mut b.spans);
                    b.row.push(cell);
                }
                TagEnd::TableHead | TagEnd::TableRow => {
                    let row = std::mem::take(&mut b.row);
                    if let Some(table) = b.table.as_mut() {
                        table.push(row);
                    }
                }
                TagEnd::Table => {
                    if let Some(table) = b.table.take() {
                        b.blocks.push(Block::Table(table));
                    }
                }
                _ => {}
            },
            Event::Text(text) => match b.code.as_mut() {
                Some(code) => code.push_str(&text),
                None => b.push(&text, false),
            },
            Event::Code(text) => b.push(&text, true),
            Event::Html(text) | Event::InlineHtml(text) => b.push(text.trim_end(), true),
            Event::SoftBreak => b.push(" ", false),
            Event::HardBreak => b.push("\n", false),
            Event::Rule => {
                b.flush();
                b.blocks.push(Block::Rule);
            }
            Event::TaskListMarker(done) => b.marker = Some(if done { "☑" } else { "☐" }.to_string()),
            _ => {}
        }
    }
    b.flush();
    b.blocks
}

/// Whether a file is Markdown, by its name.
pub fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|e| {
        let e = e.to_string_lossy().to_lowercase();
        matches!(e.as_str(), "md" | "markdown" | "mdown" | "mkd")
    })
}

fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'/' | b'-' | b'_' | b'.' | b'~' | b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Reads the `%20` kind of escape in an address.
pub fn decode(text: &str) -> Option<String> {
    let encoded = text.as_bytes();
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut at = 0;
    while at < encoded.len() {
        let hex = (encoded[at] == b'%').then(|| encoded.get(at + 1..at + 3)).flatten();
        match hex.and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()) {
            Some(byte) => {
                bytes.push(byte);
                at += 3;
            }
            None => {
                bytes.push(encoded[at]);
                at += 1;
            }
        }
    }
    String::from_utf8(bytes).ok()
}

/// The address of a file, for a pane that shows it. A file on another machine has
/// that machine in the address, as `file://m1/a.md`.
pub fn file_uri(place: &Place) -> String {
    format!("file://{}{}", encode(&place.host).replace('/', "%2F"), encode(&place.path.to_string_lossy()))
}

/// The Markdown file a pane's address names, if it names one.
pub fn markdown_place(uri: &str) -> Option<Place> {
    let rest = uri.strip_prefix("file://")?;
    let (host, path) = rest.split_at(rest.find('/')?);
    let place = Place::new(&decode(host)?, decode(path)?);
    is_markdown(&place.path).then_some(place)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn headings_and_paragraphs_keep_their_inline_styles() {
        let blocks = parse("# Title\n\nSome **bold** and `code` and [a link](https://x.y).\n");
        assert!(matches!(&blocks[0], Block::Heading { level: 1, spans } if text(spans) == "Title"));
        let Block::Paragraph(spans) = &blocks[1] else { panic!("{blocks:?}") };
        assert!(spans.iter().any(|s| s.bold && s.text == "bold"));
        assert!(spans.iter().any(|s| s.code && s.text == "code"));
        assert!(spans.iter().any(|s| s.link.as_deref() == Some("https://x.y") && s.text == "a link"));
    }

    #[test]
    fn lists_number_and_nest() {
        let blocks = parse("1. one\n2. two\n   - inner\n");
        let items: Vec<_> = blocks
            .iter()
            .map(|b| match b {
                Block::Item { depth, marker, spans } => (*depth, marker.clone(), text(spans)),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(items, [(0, "1.".into(), "one".into()), (0, "2.".into(), "two".into()), (1, "•".into(), "inner".into())]);
    }

    #[test]
    fn task_items_show_their_state() {
        let blocks = parse("- [x] done\n- [ ] todo\n");
        assert!(matches!(&blocks[0], Block::Item { marker, .. } if marker == "☑"));
        assert!(matches!(&blocks[1], Block::Item { marker, .. } if marker == "☐"));
    }

    #[test]
    fn code_keeps_its_lines_and_is_not_styled() {
        let blocks = parse("```rust\nlet **a** = 1;\n\nb();\n```\n");
        assert_eq!(blocks, [Block::Code { lines: vec!["let **a** = 1;".into(), String::new(), "b();".into()] }]);
    }

    #[test]
    fn quotes_rules_and_tables_are_their_own_blocks() {
        let blocks = parse("> said\n\n---\n\n| a | b |\n|---|---|\n| 1 | 2 |\n");
        assert!(matches!(&blocks[0], Block::Quote(s) if text(s) == "said"));
        assert_eq!(blocks[1], Block::Rule);
        let Block::Table(rows) = &blocks[2] else { panic!("{blocks:?}") };
        assert_eq!(rows.len(), 2);
        assert_eq!(text(&rows[1][1]), "2");
    }

    #[test]
    fn a_file_address_goes_there_and_back_with_awkward_names() {
        let path = Path::new("/tmp/my notes #1/100%.md");
        let uri = file_uri(&Place::new("", path));
        assert!(!uri.contains(' ') && !uri.contains('#'));
        assert_eq!(markdown_place(&uri), Some(Place::new("", path)));
        let far = Place::new("m1", "/a b/c.md");
        assert_eq!(file_uri(&far), "file://m1/a%20b/c.md");
        assert_eq!(markdown_place(&file_uri(&far)), Some(far));
    }

    #[test]
    fn only_markdown_files_count() {
        assert!(is_markdown(Path::new("a.MD")));
        assert!(!is_markdown(Path::new("a.rs")));
        assert_eq!(markdown_place("file:///a.txt"), None);
        assert_eq!(markdown_place("tuios://review/x"), None);
    }
}
