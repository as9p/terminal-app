//! Session states from Claude Code hook events: working, blocked, done and idle. Pure logic:
//! the caller passes the time of each event, and `tick` turns a finished turn into done once
//! its short wait is over. The rules follow DESIGN.md ("Session states", "Alerts") and the
//! hook tests in docs/risky-tests.md, section 3.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::Value;

/// How long a turn that ended with no helper running waits before it shows as done. Any event
/// of the main session in that time cancels it, which covers a helper whose report lands just
/// after the turn ended (docs/risky-tests.md, ruling a).
pub const DONE_DELAY: Duration = Duration::from_secs(2);

/// The name Claude Code gives the tool that starts a helper.
const HELPER_TOOL: &str = "Agent";

/// Not a Claude hook: the shell hook's `claude` sends it once a typed `claude` has returned,
/// however it ended, so one that died without `SessionEnd` does not stay working.
pub const CLAUDE_EXIT: &str = "ClaudeExit";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentState {
    Working,
    Blocked,
    Done,
    Idle,
}

impl AgentState {
    /// The name the protocol sends.
    pub fn name(self) -> &'static str {
        match self {
            AgentState::Working => "working",
            AgentState::Blocked => "blocked",
            AgentState::Done => "done",
            AgentState::Idle => "idle",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookEvent {
    /// The chat started, cleared or was compacted. `chat` is Claude's session id.
    Start {
        chat: String,
    },
    Prompt,
    /// A tool call, before or after it ran. `edited` is the file an `Edit` or `Write` names.
    /// `helper_started` is true for the main session's call that starts a helper. The
    /// tracker does not count helpers with it: `Stop` says whether one is still running.
    /// `helper` is the `agent_id` of the helper that made the call, which shares the main
    /// session's id.
    Tool {
        edited: Option<PathBuf>,
        helper_started: bool,
        helper: Option<String>,
    },
    /// A permission prompt. `helper` is the `agent_id` of the helper that raised it.
    NeedsInput {
        helper: Option<String>,
    },
    HelperStop {
        helper: Option<String>,
    },
    /// The main session ended its turn. `helper_running` is true while one of its helpers
    /// is still running, which holds done back until the turn that follows its report.
    Stop {
        helper_running: bool,
    },
    End,
}

/// The event for one hook call, or None for one that changes nothing. `event` is the hook's
/// name, in any case. An event that carries `agent_id` came from a helper.
pub fn parse(event: &str, payload: &Value) -> Option<HookEvent> {
    let helper = payload
        .get("agent_id")
        .filter(|id| !id.is_null())
        .map(|id| id.as_str().map_or_else(|| id.to_string(), str::to_string));
    let text = |key: &str| payload.get(key).and_then(Value::as_str);
    match event.to_ascii_lowercase().as_str() {
        "sessionstart" => text("session_id").map(|chat| HookEvent::Start {
            chat: chat.to_string(),
        }),
        "userpromptsubmit" if helper.is_none() => Some(HookEvent::Prompt),
        "pretooluse" | "posttooluse" | "posttoolusefailure" => {
            let tool = text("tool_name");
            Some(HookEvent::Tool {
                edited: edited_file(payload),
                helper_started: helper.is_none() && tool == Some(HELPER_TOOL),
                helper,
            })
        }
        "permissionrequest" => Some(HookEvent::NeedsInput { helper }),
        // `idle_prompt` only comes after the turn ended, which is already done.
        "notification" if text("notification_type") == Some("permission_prompt") => {
            Some(HookEvent::NeedsInput { helper })
        }
        "subagentstop" => Some(HookEvent::HelperStop { helper }),
        "stop" if helper.is_some() => Some(HookEvent::HelperStop { helper }),
        "stop" => Some(HookEvent::Stop {
            helper_running: helper_running(payload),
        }),
        "sessionend" => Some(HookEvent::End),
        name if name.eq_ignore_ascii_case(CLAUDE_EXIT) => Some(HookEvent::End),
        _ => None,
    }
}

/// The file an `Edit` or `Write` call names, joined to the session's folder when relative.
fn edited_file(payload: &Value) -> Option<PathBuf> {
    let tool = payload.get("tool_name").and_then(Value::as_str)?;
    if tool != "Edit" && tool != "Write" {
        return None;
    }
    let file = PathBuf::from(payload.pointer("/tool_input/file_path")?.as_str()?);
    match payload.get("cwd").and_then(Value::as_str) {
        Some(cwd) if file.is_relative() => Some(PathBuf::from(cwd).join(file)),
        _ => Some(file),
    }
}

/// True when `background_tasks` lists a running helper. Other task kinds, such as a
/// helper's own background command, do not hold done back.
fn helper_running(payload: &Value) -> bool {
    let Some(tasks) = payload.get("background_tasks").and_then(Value::as_array) else {
        return false;
    };
    tasks.iter().any(|task| {
        task.get("type").and_then(Value::as_str) == Some("subagent")
            && task.get("status").and_then(Value::as_str) == Some("running")
    })
}

/// What one event did to a session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Change {
    /// The session's state after the event, None when it has none.
    pub state: Option<AgentState>,
    /// True when `state` differs from the state before the event.
    pub changed: bool,
    /// The first file the session edited, reported once.
    pub first_edit: Option<PathBuf>,
    /// The chat id a `Start` gave.
    pub chat: Option<String>,
}

#[derive(Debug, Default)]
struct Entry {
    state: Option<AgentState>,
    /// When `state` last changed, so blocked sessions can be taken oldest first.
    since: Option<Instant>,
    /// When a turn that ended with no helper running becomes done.
    done_at: Option<Instant>,
    edited: bool,
    /// The helper whose permission prompt set blocked, None when the main session's did.
    /// That helper's next event means it got past the prompt.
    blocked_by: Option<String>,
}

impl Entry {
    fn set(&mut self, state: Option<AgentState>, now: Instant) {
        if self.state != state {
            self.state = state;
            self.since = Some(now);
        }
    }
}

/// The state of every session that has had a hook event, by session uuid.
#[derive(Debug, Default)]
pub struct Tracker {
    sessions: BTreeMap<String, Entry>,
}

impl Tracker {
    pub fn on_event(&mut self, session: &str, ev: HookEvent, now: Instant) -> Change {
        let before = self.state(session);
        let mut change = Change::default();
        match ev {
            HookEvent::Start { chat } => {
                let entry = self.sessions.entry(session.to_string()).or_default();
                if entry.state.is_none() {
                    entry.set(Some(AgentState::Idle), now);
                }
                change.chat = Some(chat);
            }
            HookEvent::Prompt => self.work(session, None, now),
            HookEvent::Tool { edited, helper, .. } => {
                // A helper's call never clears the main session's blocked, only its own.
                match helper {
                    Some(helper) => self.helper_moved_on(session, &helper, now),
                    None => self.work(session, None, now),
                }
                if let Some(file) = edited {
                    let entry = self.sessions.entry(session.to_string()).or_default();
                    if !entry.edited {
                        entry.edited = true;
                        change.first_edit = Some(file);
                    }
                }
            }
            HookEvent::NeedsInput { helper } => {
                let entry = self.sessions.entry(session.to_string()).or_default();
                // The main session's prompt stays the one to answer while it is up.
                let main_waits =
                    entry.state == Some(AgentState::Blocked) && entry.blocked_by.is_none();
                if !main_waits {
                    entry.blocked_by = helper;
                }
                entry.done_at = None;
                entry.set(Some(AgentState::Blocked), now);
            }
            HookEvent::HelperStop { helper } => {
                if let Some(helper) = helper {
                    self.helper_moved_on(session, &helper, now);
                }
            }
            HookEvent::Stop { helper_running } => {
                let done_at = (!helper_running).then(|| now + DONE_DELAY);
                self.work(session, done_at, now);
            }
            HookEvent::End => {
                self.sessions.remove(session);
            }
        }
        change.state = self.state(session);
        change.changed = change.state != before;
        change
    }

    /// Turns every session whose wait for done is over into done, and returns those.
    pub fn tick(&mut self, now: Instant) -> Vec<(String, AgentState)> {
        let mut out = Vec::new();
        for (session, entry) in &mut self.sessions {
            if entry.done_at.is_some_and(|at| at <= now) {
                entry.done_at = None;
                entry.set(Some(AgentState::Done), now);
                out.push((session.clone(), AgentState::Done));
            }
        }
        out
    }

    /// Alex looked at the session: done becomes idle. Nothing else changes.
    pub fn seen(&mut self, session: &str) -> Change {
        let Some(entry) = self.sessions.get_mut(session) else {
            return Change::default();
        };
        let changed = entry.state == Some(AgentState::Done);
        if changed {
            entry.set(Some(AgentState::Idle), Instant::now());
        }
        Change {
            state: entry.state,
            changed,
            ..Change::default()
        }
    }

    pub fn state(&self, session: &str) -> Option<AgentState> {
        self.sessions.get(session).and_then(|entry| entry.state)
    }

    /// Every session that has a state, with when that state began.
    pub fn all(&self) -> Vec<(String, AgentState, Instant)> {
        self.sessions
            .iter()
            .filter_map(|(session, entry)| Some((session.clone(), entry.state?, entry.since?)))
            .collect()
    }

    /// The blocked sessions, the one blocked longest first.
    pub fn blocked_oldest_first(&self) -> Vec<String> {
        let mut blocked: Vec<_> = self
            .sessions
            .iter()
            .filter(|(_, entry)| entry.state == Some(AgentState::Blocked))
            .map(|(session, entry)| (entry.since, session.clone()))
            .collect();
        blocked.sort();
        blocked.into_iter().map(|(_, session)| session).collect()
    }

    /// Marks the session working and sets or cancels its wait for done.
    fn work(&mut self, session: &str, done_at: Option<Instant>, now: Instant) {
        let entry = self.sessions.entry(session.to_string()).or_default();
        entry.done_at = done_at;
        entry.blocked_by = None;
        entry.set(Some(AgentState::Working), now);
    }

    /// An event of `helper`: when its own permission prompt set blocked, it got past it and
    /// the session is working again. Anything else is left alone.
    fn helper_moved_on(&mut self, session: &str, helper: &str, now: Instant) {
        let Some(entry) = self.sessions.get_mut(session) else {
            return;
        };
        if entry.state == Some(AgentState::Blocked) && entry.blocked_by.as_deref() == Some(helper) {
            entry.blocked_by = None;
            entry.set(Some(AgentState::Working), now);
        }
    }
}
