//! Session states from Claude hook events: which event sets which state, the hold on "done"
//! while a helper runs, and reading the hook JSON. The JSON in `fixtures/hooks` is copied from
//! real hook logs (docs/risky-tests.md, section 3) with personal paths replaced.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;
use term_host::states::{AgentState, Change, DONE_DELAY, HookEvent, Tracker, parse};

fn fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hooks")
        .join(name);
    let text = fs::read_to_string(&path).expect("read the fixture");
    serde_json::from_str(&text).expect("the fixture is JSON")
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// A tool call of the main session that edits nothing.
fn tool() -> HookEvent {
    HookEvent::Tool {
        edited: None,
        helper_started: false,
        from_helper: false,
    }
}

/// A tool call of the main session that edits `file`.
fn edit(file: &str) -> HookEvent {
    HookEvent::Tool {
        edited: Some(PathBuf::from(file)),
        helper_started: false,
        from_helper: false,
    }
}

/// A tool call made by a helper of the session.
fn helper_tool() -> HookEvent {
    HookEvent::Tool {
        edited: None,
        helper_started: false,
        from_helper: true,
    }
}

fn stop() -> HookEvent {
    HookEvent::Stop {
        helper_running: false,
    }
}

/// Runs the delayed "done" past its wait and returns the state after.
fn settle(tracker: &mut Tracker, session: &str, after: Instant) -> Option<AgentState> {
    tracker.tick(after + DONE_DELAY);
    tracker.state(session)
}

#[test]
fn prompt_then_stop_is_done() {
    let t0 = Instant::now();
    let mut tracker = Tracker::default();
    let change = tracker.on_event("a", HookEvent::Prompt, t0);
    assert_eq!(change.state, Some(AgentState::Working));
    assert!(change.changed);

    // Done waits a short while in case the turn picks up again.
    let stopped = t0 + secs(1);
    let change = tracker.on_event("a", stop(), stopped);
    assert_eq!(change.state, Some(AgentState::Working));
    assert!(!change.changed);
    assert!(
        tracker
            .tick(stopped + DONE_DELAY - Duration::from_millis(1))
            .is_empty()
    );
    assert_eq!(tracker.state("a"), Some(AgentState::Working));

    assert_eq!(
        tracker.tick(stopped + DONE_DELAY),
        [("a".to_string(), AgentState::Done)]
    );
    assert_eq!(tracker.state("a"), Some(AgentState::Done));
    assert!(tracker.tick(stopped + secs(60)).is_empty());
}

#[test]
fn stop_with_helper_running_stays_working() {
    let t0 = Instant::now();
    let mut tracker = Tracker::default();
    tracker.on_event("a", HookEvent::Prompt, t0);
    tracker.on_event(
        "a",
        HookEvent::Tool {
            edited: None,
            helper_started: true,
            from_helper: false,
        },
        t0,
    );
    let change = tracker.on_event(
        "a",
        HookEvent::Stop {
            helper_running: true,
        },
        t0 + secs(1),
    );
    assert_eq!(change.state, Some(AgentState::Working));
    assert!(tracker.tick(t0 + secs(60)).is_empty());
    assert_eq!(tracker.state("a"), Some(AgentState::Working));
}

#[test]
fn stop_after_last_helper_is_done() {
    let t0 = Instant::now();
    let mut tracker = Tracker::default();
    tracker.on_event("a", HookEvent::Prompt, t0);
    tracker.on_event(
        "a",
        HookEvent::Stop {
            helper_running: true,
        },
        t0 + secs(1),
    );
    tracker.on_event("a", helper_tool(), t0 + secs(2));
    let change = tracker.on_event("a", HookEvent::HelperStop, t0 + secs(3));
    assert_eq!(change.state, Some(AgentState::Working));
    assert!(!change.changed);

    // The helper's report comes in as a prompt, then the short turn ends with no helper left.
    tracker.on_event("a", HookEvent::Prompt, t0 + secs(3));
    tracker.on_event("a", stop(), t0 + secs(4));
    assert_eq!(
        settle(&mut tracker, "a", t0 + secs(4)),
        Some(AgentState::Done)
    );
}

#[test]
fn event_during_done_wait_keeps_working() {
    let t0 = Instant::now();
    let mut tracker = Tracker::default();
    tracker.on_event("a", HookEvent::Prompt, t0);
    tracker.on_event("a", stop(), t0 + secs(1));
    // A helper report that lands just after the stop picks the turn up again.
    tracker.on_event("a", HookEvent::Prompt, t0 + secs(2));
    assert!(tracker.tick(t0 + secs(10)).is_empty());
    assert_eq!(tracker.state("a"), Some(AgentState::Working));

    tracker.on_event("a", stop(), t0 + secs(11));
    assert_eq!(
        settle(&mut tracker, "a", t0 + secs(11)),
        Some(AgentState::Done)
    );
}

#[test]
fn helper_stop_never_alerts() {
    let t0 = Instant::now();
    let mut tracker = Tracker::default();
    tracker.on_event("a", HookEvent::Prompt, t0);
    let change = tracker.on_event("a", HookEvent::HelperStop, t0 + secs(1));
    assert_eq!(change.state, Some(AgentState::Working));
    assert!(!change.changed);
    assert!(tracker.tick(t0 + secs(60)).is_empty());

    tracker.on_event("a", stop(), t0 + secs(61));
    assert_eq!(
        settle(&mut tracker, "a", t0 + secs(61)),
        Some(AgentState::Done)
    );
    tracker.seen("a");
    for ev in [HookEvent::HelperStop, helper_tool()] {
        let change = tracker.on_event("a", ev, t0 + secs(70));
        assert_eq!(change.state, Some(AgentState::Idle));
        assert!(!change.changed);
    }
    assert!(tracker.tick(t0 + secs(120)).is_empty());

    // A session that only ever saw a helper finish has no state.
    let change = tracker.on_event("b", HookEvent::HelperStop, t0);
    assert_eq!(change, Change::default());
    assert_eq!(tracker.state("b"), None);
}

#[test]
fn needs_input_is_blocked_until_next_prompt() {
    let t0 = Instant::now();
    let mut tracker = Tracker::default();
    tracker.on_event("a", HookEvent::Prompt, t0);
    let change = tracker.on_event("a", HookEvent::NeedsInput, t0 + secs(1));
    assert_eq!(change.state, Some(AgentState::Blocked));
    assert!(change.changed);

    // A helper's work and a helper finishing share the session id and must not clear it.
    for ev in [helper_tool(), HookEvent::HelperStop] {
        assert_eq!(
            tracker.on_event("a", ev, t0 + secs(2)).state,
            Some(AgentState::Blocked)
        );
    }
    assert!(tracker.tick(t0 + secs(60)).is_empty());
    assert_eq!(tracker.state("a"), Some(AgentState::Blocked));

    let change = tracker.on_event("a", HookEvent::Prompt, t0 + secs(61));
    assert_eq!(change.state, Some(AgentState::Working));
    assert!(change.changed);

    // The main session's own tool call after the answer clears it too.
    tracker.on_event("b", HookEvent::NeedsInput, t0);
    assert_eq!(
        tracker.on_event("b", tool(), t0 + secs(1)).state,
        Some(AgentState::Working)
    );

    // So does the end of the turn, which then becomes done.
    tracker.on_event("c", HookEvent::NeedsInput, t0);
    let change = tracker.on_event("c", stop(), t0 + secs(1));
    assert_eq!(change.state, Some(AgentState::Working));
    assert!(change.changed);
    assert_eq!(
        settle(&mut tracker, "c", t0 + secs(1)),
        Some(AgentState::Done)
    );
}

#[test]
fn seen_turns_done_into_idle() {
    let t0 = Instant::now();
    let mut tracker = Tracker::default();
    tracker.on_event("a", HookEvent::Prompt, t0);
    tracker.on_event("a", stop(), t0);
    settle(&mut tracker, "a", t0);

    let change = tracker.seen("a");
    assert_eq!(change.state, Some(AgentState::Idle));
    assert!(change.changed);
    assert!(!tracker.seen("a").changed);
    assert_eq!(tracker.state("a"), Some(AgentState::Idle));

    tracker.on_event("b", HookEvent::Prompt, t0);
    let change = tracker.seen("b");
    assert_eq!(change.state, Some(AgentState::Working));
    assert!(!change.changed);

    assert_eq!(tracker.seen("nobody"), Change::default());
    assert_eq!(tracker.state("nobody"), None);
}

#[test]
fn seen_does_not_clear_blocked() {
    let t0 = Instant::now();
    let mut tracker = Tracker::default();
    tracker.on_event("a", HookEvent::NeedsInput, t0);
    let change = tracker.seen("a");
    assert_eq!(change.state, Some(AgentState::Blocked));
    assert!(!change.changed);
    assert_eq!(tracker.blocked_oldest_first(), ["a"]);
}

#[test]
fn end_clears_state() {
    let t0 = Instant::now();
    let mut tracker = Tracker::default();
    tracker.on_event("a", HookEvent::NeedsInput, t0);
    let change = tracker.on_event("a", HookEvent::End, t0 + secs(1));
    assert_eq!(change.state, None);
    assert!(change.changed);
    assert_eq!(tracker.state("a"), None);
    assert!(tracker.blocked_oldest_first().is_empty());

    // An end during the wait for done leaves nothing to become done.
    tracker.on_event("b", HookEvent::Prompt, t0);
    tracker.on_event("b", stop(), t0 + secs(1));
    tracker.on_event("b", HookEvent::End, t0 + secs(2));
    assert!(tracker.tick(t0 + secs(60)).is_empty());
    assert_eq!(tracker.state("b"), None);

    // Ending a session that never had a state changes nothing.
    assert!(!tracker.on_event("c", HookEvent::End, t0).changed);
}

#[test]
fn blocked_list_is_oldest_first() {
    let t0 = Instant::now();
    let mut tracker = Tracker::default();
    tracker.on_event("a", HookEvent::NeedsInput, t0 + secs(2));
    tracker.on_event("b", HookEvent::NeedsInput, t0 + secs(1));
    tracker.on_event("c", HookEvent::NeedsInput, t0 + secs(3));
    tracker.on_event("d", HookEvent::Prompt, t0);
    // Asking again while blocked keeps its place.
    tracker.on_event("a", HookEvent::NeedsInput, t0 + secs(4));
    assert_eq!(tracker.blocked_oldest_first(), ["b", "a", "c"]);

    tracker.on_event("b", HookEvent::Prompt, t0 + secs(5));
    assert_eq!(tracker.blocked_oldest_first(), ["a", "c"]);
}

#[test]
fn first_edit_reported_once() {
    let t0 = Instant::now();
    let mut tracker = Tracker::default();
    assert_eq!(tracker.on_event("a", tool(), t0).first_edit, None);
    let change = tracker.on_event("a", edit("/repo/one.rs"), t0);
    assert_eq!(change.first_edit, Some(PathBuf::from("/repo/one.rs")));
    assert_eq!(
        tracker.on_event("a", edit("/other/two.rs"), t0).first_edit,
        None
    );
    assert_eq!(
        tracker.on_event("a", edit("/repo/one.rs"), t0).first_edit,
        None
    );

    // Each session has its own first edit.
    let change = tracker.on_event("b", edit("/other/two.rs"), t0);
    assert_eq!(change.first_edit, Some(PathBuf::from("/other/two.rs")));
}

#[test]
fn start_gives_chat_and_idle() {
    let t0 = Instant::now();
    let mut tracker = Tracker::default();
    let chat = "22222222-2222-4222-8222-222222222222".to_string();
    let change = tracker.on_event("a", HookEvent::Start { chat: chat.clone() }, t0);
    assert_eq!(change.chat.as_deref(), Some(chat.as_str()));
    assert_eq!(change.state, Some(AgentState::Idle));
    assert!(change.changed);

    // A start in the middle of a turn, as after compacting, keeps the state.
    tracker.on_event("a", HookEvent::Prompt, t0);
    let change = tracker.on_event("a", HookEvent::Start { chat }, t0);
    assert_eq!(change.state, Some(AgentState::Working));
    assert!(!change.changed);
}

#[test]
fn parse_reads_edit_path() {
    let write = fixture("pre_tool_use_write.json");
    assert_eq!(
        parse("PreToolUse", &write),
        Some(edit("/Users/test/chat/notes.txt"))
    );
    assert_eq!(
        parse("PreToolUse", &fixture("pre_tool_use_edit.json")),
        Some(edit("/Users/test/chat3/notes.txt"))
    );
    // The event name is matched in any case, and `PostToolUse` repeats the tool input.
    assert_eq!(
        parse("posttooluse", &write),
        Some(edit("/Users/test/chat/notes.txt"))
    );

    // A relative path is taken from the session's folder.
    let mut relative = write.clone();
    relative["tool_input"]["file_path"] = Value::from("src/main.rs");
    assert_eq!(
        parse("PreToolUse", &relative),
        Some(edit("/Users/test/chat/src/main.rs"))
    );

    // A tool that is not an edit has no edited file.
    assert_eq!(
        parse("PreToolUse", &fixture("pre_tool_use_helper_bash.json")),
        Some(helper_tool())
    );
}

#[test]
fn parse_reads_notification_kinds() {
    assert_eq!(
        parse("Notification", &fixture("notification_permission.json")),
        Some(HookEvent::NeedsInput)
    );
    assert_eq!(
        parse("PermissionRequest", &fixture("permission_request.json")),
        Some(HookEvent::NeedsInput)
    );
    // Waiting for the next prompt comes after the turn has ended, which is done, not blocked.
    assert_eq!(
        parse("Notification", &fixture("notification_idle.json")),
        None
    );
    let mut other = fixture("notification_permission.json");
    other["notification_type"] = Value::from("auth_success");
    assert_eq!(parse("Notification", &other), None);
}

#[test]
fn parse_reads_helpers_and_turn_ends() {
    assert_eq!(
        parse("PostToolUse", &fixture("post_tool_use_agent.json")),
        Some(HookEvent::Tool {
            edited: None,
            helper_started: true,
            from_helper: false,
        })
    );
    assert_eq!(
        parse("Stop", &fixture("stop_helper_running.json")),
        Some(HookEvent::Stop {
            helper_running: true
        })
    );
    // Only a running helper holds done back, not a background shell command.
    assert_eq!(parse("Stop", &fixture("stop_empty.json")), Some(stop()));
    assert_eq!(
        parse("Stop", &fixture("stop_shell_running.json")),
        Some(stop())
    );
    assert_eq!(
        parse("SubagentStop", &fixture("subagent_stop.json")),
        Some(HookEvent::HelperStop)
    );
    // A stop that carries a helper's id is that helper's, so it never ends the turn.
    let mut helper_stop = fixture("stop_empty.json");
    helper_stop["agent_id"] = Value::from("a8842aa35560b0d93");
    assert_eq!(parse("Stop", &helper_stop), Some(HookEvent::HelperStop));

    assert_eq!(
        parse("SessionStart", &fixture("session_start.json")),
        Some(HookEvent::Start {
            chat: "22222222-2222-4222-8222-222222222222".to_string()
        })
    );
    assert_eq!(
        parse("UserPromptSubmit", &fixture("user_prompt_submit.json")),
        Some(HookEvent::Prompt)
    );
    assert_eq!(
        parse("SessionEnd", &fixture("session_end.json")),
        Some(HookEvent::End)
    );
    assert_eq!(parse("SubagentStart", &fixture("subagent_stop.json")), None);
    assert_eq!(parse("Stop", &Value::Null), Some(stop()));
}
