//! The client side of `tuios gui-bridge`.
//!
//! Both directions carry frames: a 4-byte big-endian length of everything after
//! it, a 1-byte kind, then the payload. See internal/guibridge/bridge.go.

use std::io::{self, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

const KIND_JSON: u8 = 1;
const KIND_OUTPUT: u8 = 2;
const KIND_SNAP: u8 = 3;
const KIND_RESIZE: u8 = 4;
const KIND_INPUT: u8 = 2;
const MAX_FRAME: usize = 16 << 20;

#[derive(Debug)]
pub enum Frame {
    Event(serde_json::Value),
    Output { pty: String, bytes: Vec<u8> },
    Snapshot { pty: String, cols: u16, rows: u16, bytes: Vec<u8> },
    Resize { pty: String, cols: u16, rows: u16 },
    /// A kind this client does not know. A later bridge can add kinds.
    Unknown(u8),
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

/// Splits `u8 id length, id, rest`.
fn split_pty(payload: &[u8]) -> io::Result<(String, &[u8])> {
    let (&len, rest) = payload.split_first().ok_or_else(|| bad("frame has no pty id"))?;
    let len = len as usize;
    if rest.len() < len {
        return Err(bad("pty id is longer than the frame"));
    }
    let id = String::from_utf8_lossy(&rest[..len]).into_owned();
    Ok((id, &rest[len..]))
}

fn size(rest: &[u8]) -> io::Result<(u16, u16, &[u8])> {
    if rest.len() < 4 {
        return Err(bad("frame has no size"));
    }
    let cols = u16::from_be_bytes([rest[0], rest[1]]);
    let rows = u16::from_be_bytes([rest[2], rest[3]]);
    Ok((cols, rows, &rest[4..]))
}

pub fn decode(kind: u8, payload: &[u8]) -> io::Result<Frame> {
    Ok(match kind {
        KIND_JSON => Frame::Event(serde_json::from_slice(payload).map_err(|e| bad(&e.to_string()))?),
        KIND_OUTPUT => {
            let (pty, bytes) = split_pty(payload)?;
            Frame::Output { pty, bytes: bytes.to_vec() }
        }
        KIND_SNAP => {
            let (pty, rest) = split_pty(payload)?;
            let (cols, rows, bytes) = size(rest)?;
            Frame::Snapshot { pty, cols, rows, bytes: bytes.to_vec() }
        }
        KIND_RESIZE => {
            let (pty, rest) = split_pty(payload)?;
            let (cols, rows, _) = size(rest)?;
            Frame::Resize { pty, cols, rows }
        }
        other => Frame::Unknown(other),
    })
}

pub fn read_frame(r: &mut impl Read) -> io::Result<Frame> {
    let mut head = [0u8; 4];
    r.read_exact(&mut head)?;
    let len = u32::from_be_bytes(head) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(bad("frame length is out of range"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    decode(body[0], &body[1..])
}

fn write_frame(w: &mut impl Write, kind: u8, payload: &[u8]) -> io::Result<()> {
    w.write_all(&((payload.len() + 1) as u32).to_be_bytes())?;
    w.write_all(&[kind])?;
    w.write_all(payload)?;
    w.flush()
}

pub struct Writer(ChildStdin);

impl Writer {
    pub fn command(&mut self, cmd: &serde_json::Value) -> io::Result<()> {
        write_frame(&mut self.0, KIND_JSON, cmd.to_string().as_bytes())
    }

    pub fn input(&mut self, pty: &str, bytes: &[u8]) -> io::Result<()> {
        let id = pty.as_bytes();
        if id.len() > u8::MAX as usize {
            return Err(bad("pty id is too long"));
        }
        let mut payload = Vec::with_capacity(1 + id.len() + bytes.len());
        payload.push(id.len() as u8);
        payload.extend_from_slice(id);
        payload.extend_from_slice(bytes);
        write_frame(&mut self.0, KIND_INPUT, &payload)
    }
}

pub struct Bridge {
    pub child: Child,
    pub reader: BufReader<ChildStdout>,
    pub writer: Writer,
}

/// The size a client attaches at. The daemon sizes the session from this at the
/// attach, so a resize sent later is a correction and can lose the race.
#[derive(Debug, Clone, Copy, Default)]
pub struct Grid {
    pub cols: u16,
    pub rows: u16,
    /// Device pixels. With no cell size, the bridge keeps no insets.
    pub cell_width: i32,
    pub cell_height: i32,
    /// Top, left, right, bottom, in device pixels.
    pub insets: [i32; 4],
}

/// What to attach to. With neither, tuios picks the session last active.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Target {
    pub session: Option<String>,
    pub host: Option<String>,
}

/// Starts a bridge, using `tuios` from the PATH unless TESSELLA_TUIOS_BIN names one.
pub fn spawn(target: &Target, grid: Grid) -> io::Result<Bridge> {
    let bin = std::env::var("TESSELLA_TUIOS_BIN").unwrap_or_else(|_| "tuios".to_string());
    let mut command = Command::new(&bin);
    command.arg("gui-bridge");
    if let Some(session) = &target.session {
        command.args(["--session", session]);
    }
    if let Some(host) = &target.host {
        command.args(["--host", host]);
    }
    command.args(["--cols", &grid.cols.to_string(), "--rows", &grid.rows.to_string()]);
    if grid.cell_width > 0 && grid.cell_height > 0 {
        let [top, left, right, bottom] = grid.insets;
        command
            .args(["--cell-width", &grid.cell_width.to_string()])
            .args(["--cell-height", &grid.cell_height.to_string()])
            .args(["--insets", &format!("{top},{left},{right},{bottom}")]);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                io::Error::new(e.kind(), "tuios is not on your PATH. Tessella needs it: https://github.com/Gaurav-Gosain/tuios")
            } else {
                e
            }
        })?;
    let reader = BufReader::new(child.stdout.take().expect("stdout is piped"));
    let writer = Writer(child.stdin.take().expect("stdin is piped"));
    Ok(Bridge { child, reader, writer })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_frame_round_trips_through_the_wire_format() {
        let mut wire = vec![0, 0, 0, 6, KIND_OUTPUT, 2];
        wire.extend_from_slice(b"p1hi");
        match read_frame(&mut wire.as_slice()).unwrap() {
            Frame::Output { pty, bytes } => {
                assert_eq!(pty, "p1");
                assert_eq!(bytes, b"hi");
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn snapshot_carries_its_size() {
        let mut wire = vec![0, 0, 0, 8, KIND_SNAP, 1, b'p', 0, 100, 0, 30];
        wire.push(b'x');
        match read_frame(&mut wire.as_slice()).unwrap() {
            Frame::Snapshot { pty, cols, rows, bytes } => {
                assert_eq!((pty.as_str(), cols, rows), ("p", 100, 30));
                assert_eq!(bytes, b"x");
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_kind_is_skipped_not_an_error() {
        let wire = vec![0, 0, 0, 2, 99, 7];
        assert!(matches!(read_frame(&mut wire.as_slice()).unwrap(), Frame::Unknown(99)));
    }

    #[test]
    fn a_pty_id_longer_than_the_frame_is_refused() {
        assert!(decode(KIND_OUTPUT, &[9, b'a']).is_err());
    }
}
