# Stage 1 implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task by task. Steps use checkbox (`- [ ]`) syntax. After each task, tick its boxes and add a short handoff under "Handoffs".

**Goal:** A terminal Alex can use every day on the Mac: tabs and splits with kitty's look and keys, sessions that survive closing the window, restore after a reboot, and a host that also passes its tests on Windows.

**Architecture:** A Rust host owns the terminals and serves them over a WebSocket on loopback. A Tauri client draws them with xterm.js and talks to the host through that same WebSocket, so the phone page can reuse the code later. One thread per session handles output and client attach in order, which keeps a reattaching client from losing or repeating bytes.

**Tech stack:** Rust workspace, `portable-pty` 0.9, `vt100` 0.18, `tokio` plus `tokio-tungstenite` 0.30, `libproc` 0.15 on Mac, Tauri 2 (not the 3 alpha), TypeScript, Vite, `@xterm/xterm` 6 and `@xterm/addon-webgl` 0.19, pnpm.

**Spec:** `DESIGN.md`. Read it with this plan.

## Global constraints

- MIT license. Keep Orca's notice on anything copied from it. Never copy from kitty, cmux or AGPL projects.
- The host never depends on xterm.js. It sends plain terminal bytes.
- The host only listens on 127.0.0.1 in stage 1.
- The app never creates a worktree unless the user explicitly asks for one.
- Speed targets: at least 40 MB/s with a visible tab through the host, and typing and backspace medians within 5 ms of kitty, on a release build.
- Any `tools/keylat` run takes focus for about a minute, so get a fresh go from Alex before each one.
- Any new tab UI goes through `ui-review-page`. No tab or split UI code before Alex says go.
- `DESIGN.md` holds private details. The GitHub repo stays private until Alex decides otherwise and that file is cleaned.
- Run `grep -c -i -F -f ~/.claude-sync/block-words.txt` over new text files before a push. Never print a matching word.
- No `gh auth switch`. Use `GH_TOKEN=$(gh auth token --user as9p) gh ...`. Never touch any github.com/as9pa URL.
- No em-dashes or emoji in code comments, docs or commits. Sentence case for UI labels.

## Review focus

Each of these has its test in the task that owns the code.

1. Host started from inside a Claude session or a Finder launch: it must not leak `CLAUDECODE`, `CLAUDE_CONFIG_DIR` or `KITTY_*` into tabs, and `claude` must still be on the PATH in a tab. (Task 3)
2. A web page in Alex's browser tries to open the local WebSocket: it must be refused, even though it runs on the same computer. (Task 5)
3. A reboot hangs up every shell before the host stops: tabs must stay restorable, while typing `exit` or closing a tab removes them. (Task 11)
4. A restored Claude tab whose chat never got a first message: `--resume` would fail, so it must start fresh with the same id. (Task 11)
5. A program floods output (`yes`) while the client falls behind: host memory stays bounded, other tabs still respond, and Ctrl+C works within a second. (Task 4)

---

## Task 1: Risky tests that need no second account

**Files:** Create `docs/risky-tests.md`. Modify `DESIGN.md` (the lines these tests settle).

- [x] **Step 1:** Resume under a different path on the Mac. In a scratch folder run `claude -p --model haiku "Remember the word plum"` with `CLAUDECODE` unset, note the session id, move the folder, then try `claude -p --resume <id> "What word?"` from the new path: first as is, then after copying the transcript into the new path's folder under `~/.claude/projects/`. Record which works.
- [x] **Step 2:** Find whether Remote Control sessions can be listed. Read `claude remote-control --help` and `claude --help`, and look for a matching file under `~/.claude`. Read only. Don't start a server. Record the answer, with the exact command or file, or "no list found".
- [x] **Step 3:** Write results to `docs/risky-tests.md` and update the "Not tested" lines in `DESIGN.md`.
- [x] **Step 4:** Commit: `docs: record resume path and Remote Control list tests`.

Mac to Windows resume waits for Task 8, when a PC session is involved.

## Task 2: Risky tests that need a second account (waits on Alex)

**Files:** Modify `docs/risky-tests.md`, `DESIGN.md`. Waiting on Alex doesn't block the other tasks.

- [ ] **Step 1:** Alex signs in to a second account in a test folder: `CLAUDE_CONFIG_DIR=~/.claude-test-b claude`, then `/login`. Use a personal account, no work data.
- [ ] **Step 2:** Create a chat on the main account with a haiku prompt in a scratch folder. Link `~/.claude-test-b/projects` to `~/.claude/projects`, then resume that chat under the second account and confirm it remembers the word. If that fails, try copying the chat file, as `DESIGN.md` describes.
- [ ] **Step 3:** Start two `claude remote-control` servers side by side from a scratch folder, one per account, for about a minute, then stop both. Alex checks the Claude app shows both. Confirm no server is left running.
- [ ] **Step 4:** Record results, update `DESIGN.md`, commit: `docs: record account resume and two server tests`.

## Task 3: Terminal sessions in the host

**Files:** Create `LICENSE` (MIT), `Cargo.toml` (workspace), `host/Cargo.toml`, `host/src/{main,lib,pty,env,session}.rs`, `host/tests/session.rs`.

**Produces:**
- `Session::spawn(spec: SpawnSpec, size: Size) -> Result<SessionHandle>`
- `SpawnSpec { kind: Kind, cwd: PathBuf, uuid: Uuid }`, `Kind::Shell`, `Kind::Claude { chat: Uuid, resume: bool }`
- `SessionHandle::{write(&[u8]), resize(Size), kill(), attach(ClientTx, Size), detach(ClientId), ack(ClientId, u64), pid()}`
- `clean_env(from: impl Iterator<Item=(String,String)>) -> Vec<(String,String)>`

Each session has a reader thread feeding a bounded channel and one actor thread. The actor forwards bytes to attached clients first, then feeds the screen copy. Attach, detach and resize go through the actor, so attach happens at one exact point in the output.

- [x] **Step 1:** Write failing tests in `host/tests/session.rs`:
  - `echo_round_trip`: spawn `cat` (Windows: `cmd /c more`), write `hello\n`, read it back within 2 s.
  - `resize_reaches_program`: resize to 100x40, `stty size` prints `40 100` (Unix only).
  - `exit_reports_clean_or_signal`: `exit` reports clean; a `SIGHUP` kill reports a signal (Unix only).
  - `clean_env_strips`: `CLAUDECODE`, `CLAUDE_CONFIG_DIR`, `CLAUDE_CODE_*`, `KITTY_*`, `ORCA_*` removed; `TERM=xterm-256color`, `COLORTERM=truecolor` set.
  - `login_shell_finds_path`: a spawn with a bare environment still gets the PATH from the login shell (Unix only).
- [x] **Step 2:** Run `cargo test -p term-host --test session`. Expected: fails to compile.
- [x] **Step 3:** Implement. Shell: `$SHELL -l`, using `SHELL` from the host's environment so the speed tests can swap it. Windows: `COMSPEC`. Claude on Unix: `$SHELL -lic 'exec claude --session-id <uuid>'`, or `--resume <uuid>` when `resume` is true. Claude on Windows: `cmd /c claude ...`.
- [x] **Step 4:** Run the tests. Expected: pass.
- [x] **Step 5:** Commit: `feat(host): spawn and run terminal sessions`.

## Task 4: Screen copy and attach

**Files:** Create `host/src/screen.rs`, `host/tests/screen.rs`, `host/benches/parse.rs`, `tools/speed/gen-output.sh` (from the test build). Modify `host/src/session.rs`.

**Produces:** `Screen::new(size, scrollback: usize)`, `feed(&[u8])`, `resize(Size)`, `snapshot() -> Vec<u8>`. Scrollback is 20000 lines to match Alex's kitty.

- [x] **Step 1:** Clone Orca into the scratchpad and read how its daemon makes snapshots, reference only. Write three lines in the handoff on what to borrow. Copy no code without its notice.
- [x] **Step 2:** Write failing tests:
  - `snapshot_round_trip`: feed colors, wide characters, cursor moves and 500 scrolled lines, feed the snapshot into a fresh `Screen`, and assert the same formatted contents, scrollback and cursor.
  - `alt_screen_round_trip`: the same while a full-screen program is active, with modes like mouse and bracketed paste kept.
  - `attach_mid_stream_no_loss_no_dup`: while a session prints numbered lines, attach; the snapshot plus later bytes contain every number once, in order.
  - `flood_stays_bounded`: a client that never acks stops forwarding at 4 MiB unacked, the program is throttled, a second session keeps echoing, and `kill()` works.
  - `stale_client_dropped`: a client with no ack for 5 s is dropped, and the throttle lifts.
- [x] **Step 3:** Run them. Expected: fail.
- [x] **Step 4:** Implement on `vt100`, with the actor holding back output while a live client is over 4 MiB unacked.
- [x] **Step 5:** Benchmark 30 MB of colored text with `cargo bench -p term-host --bench parse`. Pass is at least 80 MB/s in release, because the actor parses every byte. Measure memory with 20000 lines full at 200 columns, and report it. If speed fails, swap `Screen`'s insides to `alacritty_terminal` 0.26 (Apache-2.0), keeping its interface, and note why.
- [x] **Step 6:** Run the tests. Expected: pass. Commit: `feat(host): keep a screen copy and attach without losing output`.

## Task 5: WebSocket server

**Files:** Create `host/src/{server,protocol,paths,auth}.rs`, `host/tests/server.rs`. Modify `host/src/main.rs`.

**Produces, protocol version 1:**
- Text frames are JSON. Client: `hello{token,proto}`, `create{uuid,kind,cwd,cols,rows}`, `attach{session,cols,rows}`, `detach{session}`, `resize{session,cols,rows}`, `kill{session}`, `ack{session,bytes}`, `list`, `layout_get`, `layout_set{json}`. Host: `hello_ok{proto,host_version}`, `sessions{items}`, `created{session,uuid}`, `attached{session,snapshot_bytes}`, `title{session,title}`, `exited{session,clean}`, `layout{json}`, `error{code,message}`.
- Binary frames are a 4-byte little-endian session number, then bytes, in both directions.
- Port 47211, or `TERM_HOST_PORT`. Data folder from the `dirs` crate plus `terminal-app`, or `TERM_APP_DATA`. Token file `host-token` holds 32 random bytes as hex, mode 0600.

- [x] **Step 1:** Write failing tests:
  - `refuses_foreign_origin`: `Origin: https://example.com` is refused before the upgrade.
  - `allows_app_origins`: `tauri://localhost` and `http://tauri.localhost` are accepted. `http://localhost:1420` is accepted only in debug builds.
  - `refuses_bad_token`: wrong token closes; no `hello` within 5 s closes; a wrong `proto` returns `error{code:"proto_mismatch"}` and closes.
  - `create_attach_type_echo`: full round trip over a real socket.
  - `sessions_survive_disconnect`: disconnect, reconnect, attach, and the screen and process are still there.
  - `binds_loopback_only`: connecting through a non-loopback address of this computer fails.
  - `second_host_exits`: a second host on the same port exits without touching the first.
- [x] **Step 2:** Run them. Expected: fail.
- [x] **Step 3:** Implement, with `TCP_NODELAY` set on every socket, since a delayed small packet would add typing latency.
- [x] **Step 4:** Run them. Expected: pass. Commit: `feat(host): serve sessions over a local WebSocket`.

## Task 6: One terminal in a Tauri window

**Files:** Create `client/` (`package.json`, `vite.config.ts`, `index.html`, `src/{main,connection,terminal,theme}.ts`, `src-tauri/`). Base the window on `~/projects/term-speed-spike/src-tauri`.

**Produces:** Tauri command `host_connection() -> {port, token}`, which starts the host detached if nothing answers. `Connection` and `TerminalView` classes for the tabs task.

Defaults, from Alex's `~/.config/kitty/kitty.conf`: JetBrainsMono Nerd Font Mono 14, line height 1.1, beam cursor with no blink, padding 8, scrollback 20000, copy on select, left Option as Alt, no bell sound, Shift+Enter sends `\x1b\r`, colors from `current-theme.conf`, hidden title bar.

- [x] **Step 1:** Build it with one shell session. Ack bytes from xterm's write callback.
- [x] **Step 2:** Run `pnpm -C client tsc --noEmit` and `pnpm -C client tauri build`. Expected: both succeed.
- [ ] **Step 3:** Check by hand on the release build: type, run `vim` and `claude`, Cmd+Q, reopen, and the same session returns with its screen. A foreign proto shows a clear message and kills nothing. Stop the host afterward.
- [x] **Step 4:** Commit: `feat(client): show a host session in a Tauri window`.

## Task 7: First speed check through the host

**Files:** Create `tools/keylat/test-app.sh`, `tools/speed/throughput.sh`, `tools/speed/README.md`. Modify `DESIGN.md` speed tables.

- [x] **Step 1:** Write `test-app.sh` from `test-build2.sh`: a release build, a host on its own port and data folder, and `SHELL` set to `cat-shell.sh`, then 60 presses with `keep-size`.
- [ ] **Step 2:** Ask Alex for a go, then run it through `runner.command` as `run-kitty-test.sh` shows. Rerun kitty in the same sitting, so the comparison is fair, and run each 3 times.
- [ ] **Step 3:** Measure throughput with 30 MB of colored text, three times, visible tab.
- [ ] **Step 4:** Record median, 95th percentile, share over 30 ms, backspace and MB/s in `DESIGN.md`. Missing a target stops this plan's UI work until the cause is known.
- [ ] **Step 5:** Commit: `test: measure typing and output speed through the host`.

## Task 8: Windows host

**Files:** Create `.github/workflows/host.yml`, `docs/windows-test.md`.

- [x] **Step 1:** Create the private repo `as9p/terminal-app` if it's missing, and push. If the permission check refuses, hand Alex the command.
- [x] **Step 2:** Add a workflow running `cargo test -p term-host` on `windows-latest` and `macos-latest` for pushes. Expected: green on both, with the Unix-only tests skipped.
- [ ] **Step 3:** Ask a PC session, by message starting `From session <name>:`, to clone the repo, run the tests, type in a real host session through a small test client, and try Mac to Windows chat resume. Save its reported results in `docs/windows-test.md`, marked as reported, not seen here.
- [x] **Step 4:** Commit: `ci(host): test on Windows and Mac`. Done as c1ec2e3 and the commits after it.

## Task 9: Tab and split review (waits on Alex)

**Files:** None in the repo. Publishes an artifact through `ui-review-page`.

- [ ] **Step 1:** Take real screenshots of Orca and kitty with many tabs. Close test windows after.
- [ ] **Step 2:** Publish the review: tab bar look, many-tab behavior (recommended: title shrinks first, status never hides, minimum width, then sideways scroll), the status marker in its four states (working, blocked, done, idle, from Herdr) with blocked rolling up to the tab and the jump-to-blocked key, split dividers, a new Claude tab key, and a states strip.
- [ ] **Step 3:** Wait for Alex's go. Record the answers in `DESIGN.md`.

## Task 10: Tabs, splits and kitty keys

**Files:** Create `client/src/{tabs,splits,keys,layout}.ts`, `client/src/layout.test.ts`. Modify `client/src/main.ts`, `theme.ts`.

**Produces:** `Layout` as JSON saved through `layout_set`: tabs, each holding a split tree whose leaves are session uuids.

Keys, from Alex's kitty config: Cmd+T new tab in current folder, Cmd+W close pane, Cmd+Shift+W close tab, Cmd+Shift+] and [ next and previous tab, Cmd+1 to 9 go to a tab, Cmd+Shift+T rename tab, Cmd+D vertical split in current folder, Cmd+Shift+D horizontal split, Cmd+Alt+arrows move between panes, Cmd+Shift+Enter zoom pane, Cmd+C and Cmd+V copy and paste. New Claude tab uses the key from the review.

- [ ] **Step 1:** Write failing tests for the layout logic: `close_last_pane_closes_tab`, `close_tab_kills_every_session`, `layout_round_trips_json`, `unknown_session_in_saved_layout_is_dropped`, `resize_rounds_to_whole_cells`.
- [ ] **Step 2:** Run `pnpm -C client vitest run`. Expected: fail.
- [ ] **Step 3:** Implement to match the review. Attach to a session the first time its tab is shown. A tab's title comes from the host's `title` event, and rename overrides it.
- [ ] **Step 4:** Run tests, `tsc` and `tauri build`. Expected: pass. Check every key by hand on the release build, plus 40 open tabs against the rule that status never hides.
- [ ] **Step 5:** Commit: `feat(client): add tabs, splits and kitty keys`.

## Task 11: Restore after reboot

**Files:** Create `host/src/state.rs`, `host/tests/state.rs`. Modify `host/src/{server,session,main}.rs`, `client/src/main.ts`.

**Produces:** `State { sessions: Vec<SavedSession>, layout: Json }`, saved to `state.json` by writing a temp file and renaming, at most every 2 s. `SavedSession { uuid, kind, cwd, title, chat }`.

Current folder: `libproc` `pidcwd` on Mac. On Windows, the folder the session started in, for now. Restored shells start at once. Restored Claude tabs start on first attach, so a reboot doesn't launch every chat together.

- [ ] **Step 1:** Write failing tests:
  - `clean_exit_removes`: `exit` or explicit close removes a session from state.
  - `signal_exit_keeps`: `SIGHUP` keeps it. Any exit after the host gets `SIGTERM` keeps it.
  - `restart_restores_shell_in_folder`: stop the host, start it, and the shell is back in its last folder, or home if that folder is gone.
  - `claude_restore_uses_resume`: with a transcript present the command has `--resume <id>`; with none, it has `--session-id <id>`. Use a fake `claude` on the PATH.
  - `claude_restore_is_lazy`: no process starts until attach.
  - `corrupt_state_moved_aside`: a bad file is renamed, and the host still starts.
- [ ] **Step 2:** Run them. Expected: fail.
- [ ] **Step 3:** Implement. Check for a transcript under the config folder's `projects/*/<id>.jsonl`, honoring `CLAUDE_CONFIG_DIR`. Keep window size with `tauri-plugin-window-state`.
- [ ] **Step 4:** Run them. Expected: pass. Check by hand: tabs, splits and a real Claude tab, kill the host and all shells, reopen, and all return. A `claude` typed by hand in a shell tab comes back as a shell until the stage 2 hooks can see it. Record that limit in `DESIGN.md`.
- [ ] **Step 5:** Commit: `feat: restore sessions and layout after a restart`.

## Task 12: Speed pass

**Files:** Modify whatever the measurements point to. Modify `DESIGN.md` speed tables.

- [ ] **Step 1:** Ask Alex for a go, and measure the full app as in Task 7, 3 runs each against kitty, with the sidebar-free tab bar on.
- [ ] **Step 2:** Find the cause of presses over 30 ms using timestamps at keydown, socket send, host read, host send, client receive and draw. Use the `superpowers:systematic-debugging` skill. Report the cause with its evidence.
- [ ] **Step 3:** Fix what the evidence supports, one change at a time, measuring each. Ask Alex for a go before each focus-taking run. Keep only changes that measurably help.
- [ ] **Step 4:** Report whether it matches or beats kitty. If it can't, record the gap and the case for replacing only the Mac drawing, for Alex to decide.
- [ ] **Step 5:** Run all tests and the release build. Remind Alex to turn off the Mac screen recording permissions if testing is over. Commit: `perf: speed pass`.

---

## Open questions for Alex

- Which account is the second one for Task 2. Waiting on Alex, not blocking.
- Whether test runs may unset `CLAUDECODE` through a wrapper script. Something blocked `env -u` in Task 1 and the agent worked around it. The source of the block is unknown. Task 2 needs an answer.
- Task 6 Step 3: Alex's hand check of the release app, including the stray 280x168 window and the small round bubble near the cursor.
- Task 7 Steps 2 and 3: a go for `tools/speed/run-sitting.sh task-7`. It takes the keyboard for about 4 minutes. Tasks 9 and 10 wait on its result.
- Task 8 Step 3: no session on the PC is reachable from this Mac (ListAgents shows only Mac sessions). Alex starts Claude Code on the PC with Remote Control on, or says to skip the PC hand test for now. CI already runs the host tests on Windows.

## Handoffs

(One short note per finished task: what changed, what was verified and how, anything the next task should know.)

**Task 1 (9e3ca5c):** Plain `claude --resume <id>` works from a moved folder on the Mac, and keeps writing to the original transcript. No command lists Remote Control sessions. `~/.claude/sessions/<pid>.json` has a `bridgeSessionId` field, and what fills it is unknown. Verified by reading the diff and report and checking that no test folders or processes remain. Mac to Windows resume is still open for Task 8. The test ran through a wrapper script after `env -u` was blocked (see open questions).

**Task 3 (149b986):** Host sessions work: spawn, write, resize, kill with escalation, attach, detach, exit events. Verified by 22 passing tests, clippy, fmt, and 200 repeat runs of the session tests with no failures. Two macOS flakes fixed in `host/src/pty.rs`: a bounded retry on the kernel's -6 open race, and running libnotify's run-once setup before any fork, because a fork child could die with SIGKILL in `_notify_fork_child`. Which parent thread held that gate is still unknown. Any new code that forks must call `prepare_for_fork` first. Task 4: attach only subscribes so far, snapshot and throttle are yours. Task 8: the Windows parts have never compiled; the startup cursor reply rests on memory of the ConPTY stall, and the Windows echo test runs `more` inside COMSPEC. Task 11: `SpawnSpec.uuid` is unused so far, Claude uses `chat`; any exit after `kill()` now reports not clean, including a shell that answers SIGHUP with a normal exit.

**Task 4 (6dde78d, 0482277, 0a283cc, 6eed1ef, c965361):** Screen copy on vt100, attach through the actor, flood limit. Verified by 27 session tests (five clean repeat runs), 5 screen tests, clippy, fmt, and a mutation check that broke `session.rs` 14 ways: 13 caught. The survivor, "drain not restarted", cannot run on macOS, which ends output when the program exits (seen with a probe). The test for it only bites on Linux.

- Orca, read only: (1) a headless emulator per session serialized on attach, the same shape as `Screen`; (2) its "serialize() omits parser state" comment names a real bug, and `Screen::feed` takes the idea as `held`; (3) its alt screen restore lists modes one by one, including some vt100 never tracks (4h, 1004, ?7l, ?45), the place to start if we track modes ourselves.
- Speed: 105 to 113 MB/s release, pass line 80. Memory: 142.5 MiB at 20000 lines of 200 columns. Short lines: 109 MiB resident, 281 MiB after the first attach, because the snapshot pads a clone and the process keeps that memory after freeing it. Live memory goes back to where it was, and later snapshots reuse the space. Full size snapshot about 45 ms. Task 12 can revisit.
- A snapshot cannot restore: scroll margins, saved cursor, origin mode, tab stops, character sets (`ESC ( 0` line drawing comes back as letters), modes vt100 does not track (autowrap off, focus 1004, 2026), OSC state (title, cwd, hyperlinks), lines cut on narrow then widen. This affects reattach only; the live stream goes to xterm directly.
- Ruling: a kill ends the hold. macOS keeps a killed program in `ps` state `E` while its terminal has unread output, so a pure hold never reports its exit. A client over the limit at kill time loses the final bytes and still gets `Exited`. The cause is inferred from behaviour, not kernel source.
- Task 5: `SessionHandle::write` blocks while the program is not reading, so it must never run on the async runtime or a path shared with other sessions. A dropped stale client sees only a disconnected channel, which means "attach again". Acks are running totals and include the snapshot.
- Task 6: snapshot replay has never run in xterm.js. Check it, with xterm scrollback at 20000 or more.
- Task 8: on Windows the terminal has no end of output at exit, so the drain grace is the main path there, and the hold has never run against ConPTY. These tests are `cfg(unix)`. Give the hold and the drain restart a Windows test.
- Known risk: endless small acks can hold a program indefinitely, following "resets on any ack". Only a local client with the token can do this.

**Task 5 (1ab4c83, ed75a53, 9a9e752):** The host serves sessions over a WebSocket on 127.0.0.1 with origin and token checks. Verified by 63 debug tests, 23 server and 28 session tests in release, clippy, fmt, 40 clean repeat runs, 200 clean runs of `last_resize_wins`, and 7 of 7 break checks caught.

- Added beyond the plan: host frame `detached{session}` (the app answers by attaching again), error code `spawn_failed`, and `SessionHandle::announced()`. `kill`, `detach` and `layout_set` send no reply on success. A stale ack is ignored. A duplicate uuid is allowed. `create`'s `uuid` is also the chat id; resume stays Task 11.
- Readline finding: bash line editing reads the window size and writes it back as it returns to its prompt, so a resize landing between the two is lost with no error. On a bare terminal with no host code, 29 of 800 resizes were undone with line editing on, 0 of 800 with it off, 0 of 800 in zsh. The cause in readline's source is from memory, not checked; the experiment is what implicates line editing. Linux probably has it too, not tested. The host now reads the size back after 50 ms and sets it again, up to 3 times. `list` reports a size only once it has held, so it lags a resize by 50 ms.
- A resize that a program undoes later than 50 ms after the host set it stays undone. That needs a shell stalled between its read and its write, accepted as unlikely.
- Speed through the socket on release, acking client that does not parse: 84 MB/s `yes` short lines, 44 `tr | cat` unbroken, 35 `tr` direct unbroken. Bare terminal with no host: 184, 47, 39. So macOS's terminal driver limits unbroken lines and the host costs about 10% there; on short lines the host is the limit at 84. 84 is an upper bound, since no parsing or drawing is included.
- Task 6: check a 20000-line snapshot through the webview. `Kind::Claude` has never been spawned; the first real one is the hand check.
- Task 8: compile on Windows, confirm `get_size` behaves on ConPTY, and test the drain restart and an input thread stuck after a kill with a background holder. Neither can run on macOS.
- Task 10 and 12: `Connection::answer` handles frames one at a time and `create` and `attach` wait on blocking work, so a slow spawn, snapshot or folder check stalls other tabs on that connection. Measure before changing it.
- Accepted: a client holding the token can grow the writer queue with `list`.

**Task 6 (ab72c33, 975e63d, 3dfd3ca, d3fdf8d), Step 3 still waits on Alex's hand check:** The app shows a host session in a Tauri window. Verified on the release build, on test ports: the picture of the WebGL canvas shows JetBrains Mono in all four faces, color, box lines and icons. A host with another protocol shows the version message, and no host was started or killed. A 20038-line snapshot reached the page buffer within 100 ms between two reports, with the right first and last lines. Closing and reopening the app returned the same session with its buffer. Killing the host with the app open showed the lost message, then a new host and new session about 200 ms later. Retries against something that is not the host came 1.1, 2.1, 4.1, 8.1, 16.1 s apart. Not seen yet: typing with real keys, vim, a real `Kind::Claude` spawn, real Cmd+Q, copy on select, ack pacing under load.

- Real bug found: WKWebView's canvas cannot see user-installed fonts, and xterm measures and draws through canvas, so the app drew Courier with no error. The app now ships four woff2 faces plus their license, about 4.3 MB. Windows and other users need no install because of it.
- Cell size: 17 x 39 device pixels at ratio 2. `letterSpacing: 1` is in device pixels and matches kitty's rounding at 14 px, but not at a font size whose natural width is a whole number (15 px gives 18, plus 1). Task 10: work the spacing out from the measured width when font size keys arrive.
- Behind another window or on another AeroSpace workspace, WebKit stops animation frames while the buffer keeps advancing. So acks do not stall, and pictures of a hidden window are stale. Task 12: confirm a hidden window paints at once when shown.
- AeroSpace tiles the window by default, so its size depends on the workspace. 1280x800 applies only when floated.
- Open: an extra 280x168 window at layer 0 from the app's process, which AeroSpace does not list, and a small round bubble near the cursor. Probably macOS's text input indicator, not proven. Both are in Alex's hand check.
- Task 10 and 11: exited sessions pile up in the host list, and `main.ts` start-up and bar code needs sharing out for tabs. Task 8: Windows detach flags in `start_host`, Windows origin.

**Task 8 (c1ec2e3, 028dd70, 7341dec, d0f58a9, 5e6f1f1, 3eabd87), Step 3 waits on a PC session:** The host runs on Windows in CI. `host.yml` tests on windows-latest and macos-latest, and `app.yml` lints and tests the app's Rust side on Windows, because this Mac can't build the app for Windows (tauri-winres needs `llvm-rc`). Verified by CI runs 42527059403 (app), 42533187099 (host) and 42533611436 (host, with the crash exit test seen passing in the Windows log). Windows tests now cover echo, resize read-back (`get_size` reports 100x40 on ConPTY, so no extra resizes), the output hold, a stale client drop, the drain restart at exit, kill while held back, and crash exit codes.

- Measured on the runner: ConPTY passes about 1.42 MB/s with no lines dropped, and a client that never acks goes over 4 MiB in about 3.2 s at 400 columns.
- ConPTY sets a window title at program start, and cmd sets one per command, so Windows sends many more Title events than macOS. Tests skip them, and title events add bytes to a flood.
- Ruling: on Windows an exit code of 0xC0000000 and up (a crash, Ctrl+C) is unclean, any other code clean, matching Unix where only a signal is unclean. A kill is unclean on both through `kill_sent`.
- Fragile: the Windows drain test relies on about 4 KiB of output after the limit fitting in the host's buffers. A Windows build that sends output in much smaller pieces could make it flaky.
- Still not run anywhere: real typing on Windows, the app window on Windows, an input thread stuck after a kill when a background child holds the terminal, and Mac to Windows chat resume. All are Step 3.
- The app starts the host detached on Windows (no console window, home folder as working folder). `http://tauri.localhost` was already an allowed origin.
