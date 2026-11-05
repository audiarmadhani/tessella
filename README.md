# Tessella

A native macOS GUI for [tuios](https://github.com/Gaurav-Gosain/tuios), written in Rust.

> **Early alpha.** Expect rough edges and missing pieces. It is tested with tuios 0.11.1, and it
> talks to a hidden tuios command with no stability promise, so a tuios update can break it.

Tessella is a window onto your tuios sessions. tuios still runs your terminals, so
closing Tessella leaves them running, and your tuios keys, themes, hosts and scripts
work with nothing to set up. Tessella is an independent project, not part of tuios.

## Features

- Panes, splits, workspaces and floating popups, with the layout tuios keeps: BSP,
  master-stack and scrolling, from the command palette. Drag a split line to resize,
  and drag a header to move a pane or float. Double-click a header to rename
- Saved layouts, and pane pickers: numbered labels, and a list of panes with a preview
- A sidebar of sessions, terminals, agents and a file manager: new, rename, copy, cut,
  paste, trash, delete, search, and send to another machine. Right-click a file for its menu
- An agents view: the Inbox with approvals and questions, agent actions, mail, and a review
  of a pane's changes with notes. It lists agents on your other machines, and you can mail
  them. `tuios ask-human` questions appear as a card to click
- Your tuios prefix and keybindings, plus tuios's GUI chords with `ctrl+shift` read as `cmd`.
  Wait after the prefix and it shows its keys
- New panes on any machine in your tuios `[hosts]`
- Mouse selection, a scrollbar, and copy mode with tuios's vim keys and search. A program
  that asks for the mouse gets clicks and the wheel; hold `shift` to select its text instead
- A command palette, and every tuios theme with a live preview and favourites

## Install

You need macOS, Rust, and tuios. Tessella is tested with tuios 0.11.1.

```sh
git clone https://github.com/audiarmadhani/tessella
cd tessella
cargo build --release
./target/release/tessella
```

To try it without touching your live sessions, run `scripts/try.sh`. It uses a private
tuios daemon with a copy of your tuios keys.

Otherwise Tessella attaches to your running tuios daemon, to the last active session:

```sh
tessella                  # the last active session
tessella --session work   # this session, created if missing
tessella --host build     # a session on a machine in your tuios [hosts]
```

A session has one size for all of its clients, and it follows the one you used last. So
with a terminal client on the same session, its panes resize as you move between them.

## Keys

Tessella reads your keys from tuios, so change them in your tuios config to change them
in both. `tuios keybinds list` shows them, and in Tessella the prefix followed by `?`
does too.

These are Tessella's own, on top of tuios's:

| Key | Action |
|---|---|
| `cmd+n` | New pane, asking which machine if you have hosts |
| `cmd+1` to `cmd+9` | Go to a workspace |
| `cmd+shift+t` | Theme picker |
| `cmd+,` | Open settings |
| `cmd+=`, `cmd+-`, `cmd+0` | Zoom the text and interface in, out, and back |
| `cmd+left`, `cmd+right`, `cmd+backspace` | Start of the line, end of the line, delete to its start |

macOS keeps `ctrl+left` and `ctrl+right` for switching Spaces by default. Turn that off in
System Settings under Keyboard Shortcuts, Mission Control, and they move by word.

## Settings

`cmd+,` or the gear in the dock opens the settings window. It writes to
`~/.config/tessella/config.toml`, which you can also edit by hand: saving applies it.
Every setting is optional, and the file lists them all:

```toml
[font]
family = "JetBrainsMono Nerd Font Mono"
size = 13

[sidebar]
visible = true
width = 250
agents_height = 190

[panes]
confirm_close = true

[animation]
enabled = true
duration_ms = 140

[keys]
leader = "ctrl+a"       # Change the prefix in Tessella only
which_key_ms = 500      # How long the prefix waits before it shows its keys. 0 is never

[keys.prefix]
"n" = "new_window"      # A key after the prefix, in Tessella only
"z" = "none"            # Remove one

[keys.app]
"cmd+n" = "tessella_new_pane"
```

A Nerd Font is used if one is installed, so that icons draw, and otherwise SF Mono. The
font picker lists fonts of one width, since terminal text sits on a grid.
Animations also turn off when macOS is set to reduce motion.

## Limits

- Tessella talks to `tuios gui-bridge`, a hidden command with no stability promise, so
  a tuios update can break it.
- Some things tuios draws itself are not here yet: hints, paste buffers, the transfers panel
  and tape recording. Their keys say so when pressed.
- The scrolling layout cannot be panned yet, so columns fully off screen are hard to reach.
- The file manager works on this machine, so a pane on another machine shows no files.
- Answering a question or approval for a session on another machine means attaching there.
- Text input methods for languages such as Japanese are not supported yet.

## License

MIT. See [LICENSE](LICENSE).
