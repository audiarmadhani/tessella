//! Proves the bridge round trip with no window: attach, type a command into the
//! focused pane, and watch for its output. Run with scripts/dev.sh, which keeps it off your own sessions.

use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tessella::bridge::{self, Frame};

// The shell has to compute the marker, so a terminal echo or a line editor
// redrawing the typed text can never pass for the command's output.
const TYPED: &str = "echo tessella-$((40+2))\n";
const MARKER: &str = "tessella-42";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target = bridge::Target { session: Some("probe".to_string()), host: None };
    let mut b = bridge::spawn(&target, bridge::Grid { cols: 100, rows: 30, ..Default::default() })?;
    let mut reader = b.reader;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(frame) = bridge::read_frame(&mut reader) {
            if tx.send(frame).is_err() {
                break;
            }
        }
    });

    let start = Instant::now();
    let mut events: BTreeMap<String, u32> = BTreeMap::new();
    let mut output = String::new();
    let mut typed = false;
    let mut asked_for_pane = false;
    let mut ok = false;

    while start.elapsed() < Duration::from_secs(15) {
        let Ok(frame) = rx.recv_timeout(Duration::from_millis(250)) else { continue };
        match frame {
            Frame::Event(v) => {
                let kind = v["type"].as_str().unwrap_or("?").to_string();
                let count = events.entry(kind.clone()).or_default();
                *count += 1;
                if *count == 1 {
                    let text = v.to_string();
                    let cut = text.char_indices().nth(400).map_or(text.len(), |(i, _)| i);
                    println!("[{:>5}ms] {kind}: {}", start.elapsed().as_millis(), &text[..cut]);
                }
                if kind == "state" && !typed {
                    let state = &v["state"];
                    // A new session has no panes until something asks for one.
                    let empty = state["windows"].as_array().is_some_and(|w| w.is_empty());
                    if empty && !asked_for_pane {
                        b.writer.command(&serde_json::json!({"cmd": "action", "name": "new_window", "req": 1}))?;
                        asked_for_pane = true;
                        println!("[{:>5}ms] asked for a pane", start.elapsed().as_millis());
                    }
                    let focused = state["focused"].as_str().unwrap_or("");
                    let pty = state["windows"].as_array().and_then(|ws| {
                        ws.iter().find(|w| w["id"] == focused).and_then(|w| w["pty"].as_str())
                    });
                    if let Some(pty) = pty {
                        b.writer.input(pty, TYPED.as_bytes())?;
                        typed = true;
                        println!("[{:>5}ms] typed into pty {pty}", start.elapsed().as_millis());
                    }
                }
            }
            Frame::Snapshot { pty, cols, rows, bytes } => {
                println!("snapshot {pty} {cols}x{rows}, {} bytes", bytes.len());
            }
            Frame::Output { bytes, .. } => {
                output.push_str(&String::from_utf8_lossy(&bytes));
                if output.contains(MARKER) {
                    println!("[{:>5}ms] output round trip ok", start.elapsed().as_millis());
                    ok = true;
                    break;
                }
            }
            Frame::Resize { pty, cols, rows } => println!("resize {pty} {cols}x{rows}"),
            Frame::Unknown(kind) => println!("unknown frame kind {kind}"),
        }
    }

    println!("events seen: {events:?}");
    let _ = b.child.kill();
    let _ = b.child.wait();
    if !ok {
        println!("--- output tail\n{output}");
        return Err("no round trip".into());
    }
    Ok(())
}
