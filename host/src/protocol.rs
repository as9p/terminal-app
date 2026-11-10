//! Protocol version 3. Text frames are JSON with a `type` tag. Binary frames are a 4-byte
//! little-endian session number, then bytes, in both directions.

use serde::{Deserialize, Serialize};

use crate::tree::{Op, Place, Tree};

pub const PROTO: u32 = 3;

/// Values of `code` in `HostFrame::Error`.
pub mod code {
    pub const PROTO_MISMATCH: &str = "proto_mismatch";
    pub const BAD_REQUEST: &str = "bad_request";
    pub const INPUT_DROPPED: &str = "input_dropped";
    pub const NOT_ATTACHED: &str = "not_attached";
    pub const NO_SESSION: &str = "no_session";
    pub const BAD_CWD: &str = "bad_cwd";
    pub const BAD_SIZE: &str = "bad_size";
    pub const BAD_KIND: &str = "bad_kind";
    pub const SPAWN_FAILED: &str = "spawn_failed";
    pub const NO_TAB: &str = "no_tab";
    pub const NO_GROUP: &str = "no_group";
    pub const BAD_OP: &str = "bad_op";
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    Hello {
        token: String,
        proto: u32,
    },
    // `uuid` and `kind` stay strings here so an unknown kind can answer `bad_kind` rather
    // than failing the whole frame as `bad_request`.
    Create {
        uuid: String,
        kind: String,
        cwd: String,
        cols: u16,
        rows: u16,
        /// Where its pane goes in the tree. A client that leaves it out gets a tab of its own.
        #[serde(default)]
        place: Place,
    },
    Attach {
        session: u32,
        cols: u16,
        rows: u16,
    },
    Detach {
        session: u32,
    },
    Resize {
        session: u32,
        cols: u16,
        rows: u16,
    },
    Kill {
        session: u32,
    },
    Ack {
        session: u32,
        bytes: u64,
    },
    List,
    TreeGet,
    TreeOp {
        op: Op,
    },
    /// The user has seen the session's tab, which turns a done session idle.
    Seen {
        session: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostFrame {
    HelloOk {
        proto: u32,
        host_version: String,
    },
    Sessions {
        items: Vec<SessionItem>,
    },
    Created {
        session: u32,
        uuid: String,
    },
    Attached {
        session: u32,
        snapshot_bytes: u64,
    },
    Title {
        session: u32,
        title: String,
    },
    /// The folder the session's shell reported at its prompt.
    Cwd {
        session: u32,
        cwd: String,
    },
    Exited {
        session: u32,
        clean: bool,
    },
    /// The host dropped this client, so the app knows to attach again.
    Detached {
        session: u32,
    },
    /// The whole tree, as the answer to `tree_get` and after every change to it.
    Tree {
        tree: Tree,
    },
    /// Every session's Claude state, with the tree after `tree_get` and after every change.
    States {
        items: Vec<StateItem>,
    },
    Error {
        code: String,
        message: String,
    },
}

impl HostFrame {
    pub fn error(code: &str, message: &str) -> HostFrame {
        HostFrame::Error {
            code: code.to_string(),
            message: message.to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionItem {
    pub session: u32,
    pub uuid: String,
    pub kind: String,
    pub title: Option<String>,
    /// The folder the shell last reported. Null until it reports one, and always for a
    /// program with no shell hook.
    pub cwd: Option<String>,
    /// Null while running, otherwise whether it exited cleanly.
    pub exited: Option<bool>,
    pub cols: u16,
    pub rows: u16,
}

pub fn encode_binary(session: u32, bytes: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(4 + bytes.len());
    frame.extend_from_slice(&session.to_le_bytes());
    frame.extend_from_slice(bytes);
    frame
}

/// Returns `None` for a frame too short to hold a session number.
pub fn decode_binary(frame: &[u8]) -> Option<(u32, &[u8])> {
    let (session, bytes) = frame.split_first_chunk::<4>()?;
    Some((u32::from_le_bytes(*session), bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn host_frames_use_the_wire_names() {
        let cases = [
            (
                HostFrame::HelloOk {
                    proto: 2,
                    host_version: "0.1.0".to_string(),
                },
                json!({"type": "hello_ok", "proto": 2, "host_version": "0.1.0"}),
            ),
            (
                HostFrame::Sessions {
                    items: vec![SessionItem {
                        session: 1,
                        uuid: "u".to_string(),
                        kind: "shell".to_string(),
                        title: None,
                        cwd: Some("/tmp".to_string()),
                        exited: Some(true),
                        cols: 80,
                        rows: 24,
                    }],
                },
                json!({"type": "sessions", "items": [{
                    "session": 1, "uuid": "u", "kind": "shell", "title": null,
                    "cwd": "/tmp", "exited": true, "cols": 80, "rows": 24,
                }]}),
            ),
            (
                HostFrame::Created {
                    session: 1,
                    uuid: "u".to_string(),
                },
                json!({"type": "created", "session": 1, "uuid": "u"}),
            ),
            (
                HostFrame::Attached {
                    session: 1,
                    snapshot_bytes: 9,
                },
                json!({"type": "attached", "session": 1, "snapshot_bytes": 9}),
            ),
            (
                HostFrame::Title {
                    session: 1,
                    title: "t".to_string(),
                },
                json!({"type": "title", "session": 1, "title": "t"}),
            ),
            (
                HostFrame::Cwd {
                    session: 1,
                    cwd: "/tmp".to_string(),
                },
                json!({"type": "cwd", "session": 1, "cwd": "/tmp"}),
            ),
            (
                HostFrame::Exited {
                    session: 1,
                    clean: false,
                },
                json!({"type": "exited", "session": 1, "clean": false}),
            ),
            (
                HostFrame::Detached { session: 1 },
                json!({"type": "detached", "session": 1}),
            ),
            (
                HostFrame::Tree {
                    tree: Tree::default(),
                },
                json!({"type": "tree", "tree": {"groups": [], "ungrouped": [], "closed": [], "defaults": {}}}),
            ),
            (
                HostFrame::error(code::BAD_KIND, "m"),
                json!({"type": "error", "code": "bad_kind", "message": "m"}),
            ),
        ];
        for (frame, expected) in cases {
            let value: Value = serde_json::to_value(&frame).unwrap();
            assert_eq!(value, expected);
        }
    }

    #[test]
    fn binary_frames_round_trip_with_a_little_endian_session() {
        let frame = encode_binary(0x0102_0304, b"hi");
        assert_eq!(frame, [4, 3, 2, 1, b'h', b'i']);
        assert_eq!(decode_binary(&frame), Some((0x0102_0304, &b"hi"[..])));
        assert_eq!(decode_binary(&[1, 0, 0, 0]), Some((1, &b""[..])));
        assert_eq!(decode_binary(&[1, 0, 0]), None);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateItem {
    /// The session's uuid, as the tree's panes hold it.
    pub session: String,
    /// `working`, `blocked`, `done` or `idle`. Null for a session that has had no hook event.
    pub state: Option<String>,
    /// When the state began, in milliseconds since the Unix epoch.
    pub since_ms: u64,
}
