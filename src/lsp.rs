use crate::extensions::Extension;
use gtk::glib;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::{self, BufRead, BufReader, Write},
    os::unix::process::CommandExt,
    path::Path,
    process::{Child, Stdio},
    sync::mpsc::{self, SyncSender},
    thread,
};

const MAX_MESSAGE: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Position {
    pub line: usize,
    pub character: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edit {
    pub range: Range,
    pub new_text: String,
}

pub enum Event {
    Message(Value),
    Disconnected(String),
}

pub struct Server {
    child: Option<Child>,
    sender: SyncSender<Value>,
    pub ready: bool,
    pub completion: bool,
    pub actions: bool,
    pub sync_kind: u64,
    pub open_close: bool,
    pub save: Option<bool>,
    pub synced_revision: i64,
    pub synced_text: String,
    pub name: String,
    pub settings: Value,
}

impl Server {
    #[allow(deprecated)]
    pub fn start(
        extension: &Extension,
        root: &Path,
        events: glib::Sender<Event>,
    ) -> io::Result<Self> {
        let mut child = extension
            .command()
            .process_group(0)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child.stdout.take().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let (sender, outgoing) = mpsc::sync_channel(24);
        let write_events = events.clone();
        thread::spawn(move || {
            for message in outgoing {
                if let Err(err) = write_message(&mut stdin, &message) {
                    let _ = write_events.send(Event::Disconnected(err.to_string()));
                    break;
                }
            }
        });
        thread::spawn(move || {
            let mut input = BufReader::new(stdout);
            loop {
                match read_message(&mut input) {
                    Ok(Some(message)) => {
                        if events.send(Event::Message(message)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {
                        let _ = events.send(Event::Disconnected("El servidor terminó.".into()));
                        break;
                    }
                    Err(error) => {
                        let _ = events.send(Event::Disconnected(error.to_string()));
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child: Some(child),
            sender,
            ready: false,
            completion: false,
            actions: false,
            sync_kind: 0,
            open_close: false,
            save: None,
            synced_revision: -1,
            synced_text: String::new(),
            name: extension.name.clone(),
            settings: extension.settings.clone(),
        })
    }

    pub fn send(&self, message: Value) -> bool {
        self.sender.try_send(message).is_ok()
    }

    pub fn notify(&self, method: &str, params: Value) -> bool {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    pub fn request(&self, id: u64, method: &str, params: Value) -> bool {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
    }

    pub fn capabilities(&mut self, capabilities: &Value) {
        let enabled = |key| match capabilities.get(key) {
            Some(Value::Bool(value)) => *value,
            Some(Value::Object(_)) => true,
            _ => false,
        };
        self.completion = enabled("completionProvider");
        self.actions = enabled("codeActionProvider");
        let sync = &capabilities["textDocumentSync"];
        self.sync_kind = sync
            .as_u64()
            .or_else(|| sync["change"].as_u64())
            .unwrap_or(0);
        self.open_close =
            sync.is_number() && self.sync_kind != 0 || sync["openClose"].as_bool().unwrap_or(false);
        self.save = match &sync["save"] {
            Value::Bool(true) => Some(false),
            Value::Object(options) => Some(
                options
                    .get("includeText")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            ),
            _ => None,
        };
        self.ready = true;
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // On Linux each server leads its own process group. Stop its workers as well.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

fn write_message(mut output: impl Write, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(output, "Content-Length: {}\r\n\r\n", body.len())?;
    output.write_all(&body)?;
    output.flush()
}

fn read_message(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    use io::Read;
    let mut length = None;
    let mut header = String::new();
    let mut header_budget = 8_192u64;
    loop {
        header.clear();
        let count = input.take(header_budget + 1).read_line(&mut header)?;
        if count == 0 && length.is_none() {
            return Ok(None);
        }
        if count == 0 || count as u64 > header_budget {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Cabecera LSP inválida",
            ));
        }
        header_budget -= count as u64;
        if header == "\r\n" || header == "\n" {
            break;
        }
        if let Some((key, value)) = header.split_once(':') {
            if key.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse::<usize>().ok();
            }
        }
    }
    let length = length
        .filter(|size| *size <= MAX_MESSAGE)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Longitud LSP inválida"))?;
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

pub fn end_position(text: &str) -> Position {
    let line = text.bytes().filter(|&byte| byte == b'\n').count();
    let tail = text.rsplit('\n').next().unwrap_or_default();
    Position {
        line,
        character: tail.encode_utf16().count(),
    }
}

/// Converts a UTF-16 LSP position into a Unicode character offset for GtkTextBuffer.
pub fn text_offset(text: &str, position: Position) -> Option<i32> {
    let mut offset = 0usize;
    let mut lines = text.split('\n');
    for _ in 0..position.line {
        offset += lines.next()?.chars().count() + 1;
    }
    let line = lines.next()?;
    let mut units = 0;
    for ch in line.chars() {
        if units == position.character {
            return i32::try_from(offset).ok();
        }
        units += ch.len_utf16();
        offset += 1;
    }
    (units == position.character)
        .then(|| i32::try_from(offset).ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_positions_reject_a_split_surrogate() {
        let text = "uno\né🙌x\n";
        assert_eq!(
            text_offset(
                text,
                Position {
                    line: 1,
                    character: 3
                }
            ),
            Some(6)
        );
        assert_eq!(
            text_offset(
                text,
                Position {
                    line: 1,
                    character: 2
                }
            ),
            None
        );
        assert_eq!(
            end_position(text),
            Position {
                line: 2,
                character: 0
            }
        );
    }

    #[test]
    fn stdio_frames_use_byte_lengths_and_leave_following_messages() {
        let first = json!({"label": "á🙌"});
        let second = json!({"result": null});
        let mut bytes = Vec::new();
        write_message(&mut bytes, &first).unwrap();
        write_message(&mut bytes, &second).unwrap();
        let mut input = BufReader::new(bytes.as_slice());
        assert_eq!(read_message(&mut input).unwrap(), Some(first));
        assert_eq!(read_message(&mut input).unwrap(), Some(second));
        assert_eq!(read_message(&mut input).unwrap(), None);
        assert!(read_message(&mut &b"Content-Length: 9000000\r\n\r\n"[..]).is_err());
    }
}
