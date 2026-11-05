//! Turns a key press into the bytes a terminal program expects.

#[derive(Debug, Default, Clone, Copy)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

fn csi_or_ss3(letter: char, mods: Mods, app_cursor: bool) -> Vec<u8> {
    let code = 1 + mods.shift as u8 + 2 * mods.alt as u8 + 4 * mods.ctrl as u8;
    if code > 1 {
        format!("\x1b[1;{code}{letter}").into_bytes()
    } else if app_cursor {
        format!("\x1bO{letter}").into_bytes()
    } else {
        format!("\x1b[{letter}").into_bytes()
    }
}

fn tilde(n: u8, mods: Mods) -> Vec<u8> {
    let code = 1 + mods.shift as u8 + 2 * mods.alt as u8 + 4 * mods.ctrl as u8;
    if code > 1 {
        format!("\x1b[{n};{code}~").into_bytes()
    } else {
        format!("\x1b[{n}~").into_bytes()
    }
}

fn ctrl_byte(key: &str) -> Option<u8> {
    let [b] = key.as_bytes() else { return None };
    match b.to_ascii_lowercase() {
        c @ b'a'..=b'z' => Some(c - b'a' + 1),
        b' ' | b'@' | b'2' => Some(0),
        b'[' | b'3' => Some(0x1b),
        b'\\' | b'4' => Some(0x1c),
        b']' | b'5' => Some(0x1d),
        b'^' | b'6' => Some(0x1e),
        b'_' | b'-' | b'7' => Some(0x1f),
        b'8' | b'?' => Some(0x7f),
        _ => None,
    }
}

/// The kitty keyboard form of a key that has no distinct legacy bytes, for a
/// program that asked for it. None leaves the key to the legacy encoding.
fn kitty_encode(key: &str, mods: Mods) -> Option<Vec<u8>> {
    let code = 1 + mods.shift as u8 + 2 * mods.alt as u8 + 4 * mods.ctrl as u8;
    let with_mods = |n: u32| {
        Some(if code > 1 { format!("\x1b[{n};{code}u") } else { format!("\x1b[{n}u") }.into_bytes())
    };
    match key {
        "escape" => with_mods(27),
        "enter" if code > 1 => with_mods(13),
        "backspace" if code > 1 => with_mods(127),
        "tab" if mods.ctrl || mods.alt => with_mods(9),
        "space" if mods.ctrl || mods.alt => with_mods(32),
        k if mods.ctrl || mods.alt => {
            let mut chars = k.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii() => with_mods(c.to_ascii_lowercase() as u32),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `key` is the key's name and `key_char` the text it would type. None means the
/// key sends nothing to the terminal. `kitty` is whether the program asked for
/// the kitty keyboard protocol.
pub fn encode(key: &str, key_char: Option<&str>, mods: Mods, app_cursor: bool, kitty: bool) -> Option<Vec<u8>> {
    if kitty {
        if let Some(bytes) = kitty_encode(key, mods) {
            return Some(bytes);
        }
    } else if key == "enter" && mods.shift && !mods.ctrl && !mods.alt {
        // Plain enter runs a line. A newline does that in a shell too, and starts a
        // new line in programs that tell the two apart, such as Claude Code.
        return Some(vec![b'\n']);
    }
    let named = match key {
        "enter" => Some(vec![b'\r']),
        "tab" if mods.shift => Some(b"\x1b[Z".to_vec()),
        "tab" => Some(vec![b'\t']),
        "backspace" if mods.ctrl => Some(vec![0x08]),
        "backspace" => Some(vec![0x7f]),
        "escape" => Some(vec![0x1b]),
        "up" => return Some(csi_or_ss3('A', mods, app_cursor)),
        "down" => return Some(csi_or_ss3('B', mods, app_cursor)),
        "right" => return Some(csi_or_ss3('C', mods, app_cursor)),
        "left" => return Some(csi_or_ss3('D', mods, app_cursor)),
        "home" => return Some(csi_or_ss3('H', mods, app_cursor)),
        "end" => return Some(csi_or_ss3('F', mods, app_cursor)),
        "pageup" => return Some(tilde(5, mods)),
        "pagedown" => return Some(tilde(6, mods)),
        "delete" => return Some(tilde(3, mods)),
        _ => None,
    };

    let mut out = Vec::new();
    if mods.alt {
        out.push(0x1b);
    }
    if let Some(bytes) = named {
        out.extend(bytes);
        return Some(out);
    }
    let name = if key == "space" { " " } else { key };
    if mods.ctrl {
        out.push(ctrl_byte(name)?);
        return Some(out);
    }
    if mods.alt {
        // Option plus a letter is Meta for a terminal, not the composed character.
        if name.chars().count() == 1 {
            out.extend(name.as_bytes());
            return Some(out);
        }
        return None;
    }
    if let Some(text) = key_char.filter(|t| !t.is_empty()) {
        return Some(text.as_bytes().to_vec());
    }
    (name.chars().count() == 1).then(|| name.as_bytes().to_vec())
}

/// The bytes a key sends, from its name in tuios's spelling, such as `ctrl+b`.
pub fn encode_name(name: &str) -> Option<Vec<u8>> {
    if name == "+" {
        return Some(b"+".to_vec());
    }
    let (mods_text, key) = name.rsplit_once('+').unwrap_or(("", name));
    let has = |m: &str| mods_text.split('+').any(|x| x == m);
    let mods = Mods { ctrl: has("ctrl"), alt: has("alt"), shift: has("shift") };
    encode(key, None, mods, false, false)
}

/// What cmd plus a key sends, as in other Mac terminals: to the start or end of the
/// line, and delete to its start. Other cmd keys belong to the app.
pub fn cmd_chord(key: &str) -> Option<Vec<u8>> {
    match key {
        "left" => Some(vec![0x01]),
        "right" => Some(vec![0x05]),
        "backspace" => Some(vec![0x15]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_arrows_move_along_the_line_and_other_cmd_keys_send_nothing() {
        assert_eq!(cmd_chord("left").unwrap(), [1]);
        assert_eq!(cmd_chord("right").unwrap(), [5]);
        assert_eq!(cmd_chord("backspace").unwrap(), [0x15]);
        assert_eq!(cmd_chord("c"), None);
    }

    const NONE: Mods = Mods { ctrl: false, alt: false, shift: false };
    const CTRL: Mods = Mods { ctrl: true, alt: false, shift: false };
    const ALT: Mods = Mods { ctrl: false, alt: true, shift: false };
    const SHIFT: Mods = Mods { ctrl: false, alt: false, shift: true };

    fn plain(key: &str, ch: Option<&str>, mods: Mods) -> Vec<u8> {
        encode(key, ch, mods, false, false).unwrap()
    }

    #[test]
    fn shift_enter_is_distinct_from_enter() {
        assert_eq!(encode("enter", None, NONE, false, false).unwrap(), b"\r");
        assert_eq!(encode("enter", None, SHIFT, false, false).unwrap(), b"\n");
        assert_eq!(encode("enter", None, NONE, false, true).unwrap(), b"\r");
        assert_eq!(encode("enter", None, SHIFT, false, true).unwrap(), b"\x1b[13;2u");
    }

    #[test]
    fn the_kitty_protocol_gives_ambiguous_keys_their_own_codes() {
        let k = |key, mods| encode(key, None, mods, false, true).unwrap();
        assert_eq!(k("escape", NONE), b"\x1b[27u");
        assert_eq!(k("c", CTRL), b"\x1b[99;5u");
        assert_eq!(k("enter", CTRL), b"\x1b[13;5u");
        assert_eq!(k("b", ALT), b"\x1b[98;3u");
        assert_eq!(encode("a", Some("a"), NONE, false, true).unwrap(), b"a");
        assert_eq!(k("up", NONE), b"\x1b[A");
    }

    #[test]
    fn any_leader_types_as_its_key_and_not_as_its_name() {
        assert_eq!(encode_name("`").unwrap(), b"`");
        assert_eq!(encode_name("ctrl+b").unwrap(), [2]);
        assert_eq!(encode_name("ctrl+a").unwrap(), [1]);
        assert_eq!(encode_name("ctrl+space").unwrap(), [0]);
        assert_eq!(encode_name("alt+a").unwrap(), b"\x1ba");
    }

    #[test]
    fn typed_text_is_sent_as_itself() {
        assert_eq!(plain("a", Some("a"), NONE), b"a");
        assert_eq!(plain("a", Some("A"), SHIFT), b"A");
        assert_eq!(plain("space", Some(" "), NONE), b" ");
    }

    #[test]
    fn control_keys_match_a_terminal() {
        assert_eq!(plain("enter", None, NONE), b"\r");
        assert_eq!(plain("backspace", None, NONE), b"\x7f");
        assert_eq!(plain("escape", None, NONE), b"\x1b");
        assert_eq!(plain("tab", None, SHIFT), b"\x1b[Z");
        assert_eq!(plain("c", None, CTRL), [3]);
        assert_eq!(plain("space", None, CTRL), [0]);
    }

    #[test]
    fn arrows_follow_application_cursor_mode() {
        assert_eq!(encode("up", None, NONE, false, false).unwrap(), b"\x1b[A");
        assert_eq!(encode("up", None, NONE, true, false).unwrap(), b"\x1bOA");
        assert_eq!(encode("left", None, CTRL, true, false).unwrap(), b"\x1b[1;5D");
    }

    #[test]
    fn option_is_meta_rather_than_a_composed_character() {
        assert_eq!(plain("b", Some("∫"), ALT), b"\x1bb");
        assert_eq!(plain("backspace", None, ALT), b"\x1b\x7f");
    }

    #[test]
    fn a_key_with_no_terminal_meaning_sends_nothing() {
        assert_eq!(encode("f13", None, NONE, false, false), None);
        assert_eq!(encode("é", None, CTRL, false, false), None);
    }
}
