# Sidebar and groups implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task by task (implementer on opus, task and final reviewers on sonnet). Steps use checkbox (`- [ ]`) syntax. After each task, tick its boxes and add a short handoff under "Handoffs".

**Goal:** The Mac app becomes the window the design describes: a sidebar of vertical tabs and folder-based groups beside the terminal, splits, the focus rule, the four session states, and `claude` typed by hand picking up Alex's flags.

**Architecture:** The host owns one tree (groups, tabs, split shapes, names, order) and pushes it to every client, so a later plan can sync it between computers and drive it from `ta` without moving it. The host starts each shell with a small hook that reports the folder at every prompt through an escape sequence in the output stream, which costs no extra process per prompt. Claude Code hooks report session events through a tiny `term-host hook` command over a local socket, and the host turns them into states.

**Tech stack:** What stage 1 uses (Rust, `portable-pty`, `vt100`, `tokio`, Tauri 2, TypeScript, Vite, xterm.js with WebGL, pnpm), plus `vitest` and `jsdom` for client logic tests and the `interprocess` crate for the local socket (Unix socket on Mac, named pipe on Windows). Take the newest stable version of each with `cargo add` and `pnpm add -D`.

**Spec:** `DESIGN.md`, sections "Window, sidebar and groups", "Text zoom", "Keys", "Alerts" (the "your move" rule only) and "Plan impacts". Read it with this plan.

**How this fits `PLAN.md` (stage 1):**

- This plan replaces `PLAN.md` Tasks 9 and 10. The design dropped the top tab bar, so they cannot be built as written.
- It builds on `PLAN.md` Task 11 (restore). That work exists uncommitted in `/Users/x/projects/terminal-app/.claude/worktrees/stage-1` (`host/src/state.rs`, `host/tests/state.rs` and edits to `server.rs`, `main.rs`, `lib.rs`, `Cargo.toml`). It must pass its tests and be committed before Task 3 here starts. Task 1 and Task 2 here can start now.
- `PLAN.md` Task 7 (first speed check) still gates UI code, as before. `PLAN.md` Task 12 (speed pass) runs after Task 7 of this plan, with the sidebar showing.

**Not in this plan (each has its own later plan):** anything with a second computer (device filter, offline rows, project list sync, Cmd+Shift+N, `ta new --on`), accounts, the `ta` command line beyond `hook`, the master agent and `claude master`, sounds and system notifications, the board, the pull request line in a group header, helper and team child rows, the dim "maybe waiting" marker for agents without hooks, the usage line's numbers, the settings window, the Windows service and the Windows title bar setting.

## Global constraints

- Everything in `PLAN.md` "Global constraints" still holds: MIT, never copy from kitty or cmux (both GPL) or AGPL projects, host listens on 127.0.0.1 only, speed targets, a fresh go from Alex before each `tools/keylat` run, private repo, the block-words grep before a push, `GH_TOKEN=$(gh auth token --user as9p) gh ...`, no em-dashes or emoji, sentence case labels.
- No sidebar, tab or split UI code before Alex says go on the Task 2 review. The built UI must match that review.
- A key never puts up a popup, toast or readout.
- No default key uses Option with a letter or number (AeroSpace owns those on Alex's Mac).
- The hook never edits Alex's shell profile files and never changes what they load.
- Nothing that runs per keypress or per prompt may start a process.
- A name Alex typed is never overwritten by anything automatic.
- The app never creates a git worktree.
- `PROTO` goes from 1 to 2 once, in Task 3. Later tasks in this plan add frames without another bump, because host and app ship together.

## Review focus

Each has its test in the task that owns the code.

1. A folder whose path has spaces, `%`, `#` or non-Latin letters: the tab must land in the right group, not a mangled path. (Task 3)
2. Alex's own zsh setup (oh-my-zsh, a custom `ZDOTDIR`, its own `precmd`): every profile file still loads in the normal order, the prompt looks the same, and `ZDOTDIR` inside the shell is what it would have been without the app. (Task 3)
3. Text that only looks like a folder report (a `cat` of a log, an SSH session to another machine whose shell reports its own folder): the tab must not jump to a group. (Tasks 3 and 4)
4. `cd` into a linked worktree or submodule, where `.git` is a file, or into a repo with no remote: it resolves to a sensible group and never to the filesystem root. (Task 4)
5. The host is stopped or busy while Claude runs: `term-host hook` exits 0 within 300 ms and prints nothing, so Claude is never slowed or shown an error. (Task 8)

---

## Task 1: Prove the hook signals

**Files:** Modify `docs/risky-tests.md`, `DESIGN.md` ("Alerts", the "How" paragraph).

No app code. Use Haiku for the test chats, in a scratch folder under the scratchpad, with `CLAUDECODE` unset through a wrapper script as `PLAN.md` Task 1 did.

- [x] **Step 1:** Read the current Claude Code hooks reference and the CLI reference (context7 or the docs site). Record, with the page each came from: the JSON fields of `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Notification`, `Stop`, `SubagentStop` and `SessionEnd`; whether `claude --settings <file>` adds hooks on top of the user's own settings; the values `Notification` can carry and which mean "waiting for permission" and "waiting for input".
- [x] **Step 2:** Write a settings file whose hooks append one line per event (event name plus the raw JSON) to a log file. Run one chat that: answers a prompt, edits a file, runs a foreground helper agent, runs a background helper agent that finishes after the main turn ends, and hits a permission prompt (default permission mode, an edit). Run it through `claude --settings <that file>`.
- [x] **Step 3:** From the log, write a ruling for each of these in `docs/risky-tests.md`, marked as seen in a real run: (a) the exact rule that says "a helper of this session is still running" at the moment `Stop` fires; (b) how a helper's events are told from the main session's; (c) which field of `PreToolUse` holds the edited file path for `Edit` and `Write`; (d) whether `--settings` worked without touching `~/.claude`; (e) whether the session id in hooks equals the `--session-id` passed. If (a) has no reliable signal, the ruling is: hold "done" for 2 s after `Stop` and cancel it if any event arrives.
- [x] **Step 4:** Confirm no test process or folder remains. Commit: `docs: record Claude hook signals for session states`.

## Task 2: Sidebar, tab and split review (waits on Alex)

**Files:** Create `docs/ui-review.md`. Publishes an artifact through the `ui-review-page` skill, then `emil-design-eng` for polish.

- [ ] **Step 1:** Take real screenshots with Playwright MCP or the screen: Orca's sidebar with several groups, kitty with splits, and the stage 1 app window as "current". Use the cmux pictures in `/Users/x/projects/terminal-app-refs/cmux-site/` as reference only; they stay out of the repo. Pull two or three reference apps from Mobbin.
- [ ] **Step 2:** Publish the review. The "proposed" mockup is working HTML with the real theme from `client/src/theme.ts` and JetBrains Mono. It shows: the window with no title bar; compact rows (marker plus name) against cmux's three-line rows; a group header with folder and branch; collapse arrow; ungrouped tabs; the pane list under a split tab; the dimming of unfocused panes; inline rename; the short right-click menu; drag order and drag-to-join targets; 40 tabs in a narrow sidebar with the marker still showing; the dim usage line at the bottom as an empty placeholder. States strip: working, blocked, done, idle, plus blocked rolled up to a collapsed group, starting from the Claude UI mod's glyphs with teal and violet kept for helpers and other sessions.
- [ ] **Step 3:** Ask only the real open questions: the four markers, row height, sidebar width and whether it resizes, the keys table from `DESIGN.md` "Keys" plus the kitty keys listed in Task 6, and whether Cmd+Option+arrows may stay for moving between panes.
- [ ] **Step 4:** Wait for Alex's go. Write the answers into `docs/ui-review.md` as exact values (colors, sizes in px, glyphs, final keys) and update `DESIGN.md` where an answer changes it. Commit: `docs: record the sidebar review answers`.

## Task 3: Shell hook and live folder

**Files:** Create `host/src/osc.rs`, `host/src/shellinit.rs`, `host/shell/zsh/.zshenv`, `host/shell/ta.zsh`, `host/shell/ta.bash`, `host/shell/ta.ps1`, `host/tests/shellinit.rs`. Modify `host/src/pty.rs` (`argv`), `host/src/session.rs`, `host/src/protocol.rs`, `host/src/server.rs`, `host/src/lib.rs`, `client/src/connection.ts`.

**Interfaces:**
- Consumes: `clean_env`, `Session::spawn_with_env`, `ClientEvent`, `Served` and `state::current_dir` from stage 1.
- Produces:
  - `osc::Scanner::new() -> Scanner`, `Scanner::feed(&mut self, bytes: &[u8]) -> Vec<OscEvent>`, `enum OscEvent { Cwd { host: String, path: PathBuf } }`. It keeps state between calls and never changes the bytes.
  - `shellinit::install(data_dir: &Path) -> io::Result<()>` writes the hook files (embedded with `include_str!`) to `<data_dir>/shell/`, replacing them only when the contents differ.
  - `shellinit::wrap(shell: &Path, data_dir: &Path, env: &mut Vec<(String, String)>) -> Vec<OsString>` returns the argv for an interactive shell and adds what the hook needs to `env`. An unknown shell gets the stage 1 argv and no change.
  - `ClientEvent::Cwd(PathBuf)`, `HostFrame::Cwd { session: u32, cwd: String }`, `SessionItem.cwd: Option<String>`, `PROTO = 2`.
  - Every spawned process gets `TA_SESSION=<session uuid>` and `TA_DATA=<data_dir>`.

The folder report is `ESC ] 7 ; file://<hostname>/<percent-encoded absolute path> BEL` (or `ESC \` as the end), printed by the hook each time a prompt is drawn. On Windows the path part is `/C:/Users/x`.

How each shell is wrapped:

| Shell | Argv | Hook |
|---|---|---|
| zsh | `zsh -l`, with `ZDOTDIR=<data>/shell/zsh` and `TA_USER_ZDOTDIR` set to the old value when there was one | Our `.zshenv` puts `ZDOTDIR` back (or unsets it), sources the user's `.zshenv` if it exists, then sources `ta.zsh`, which adds one `precmd` function with `add-zsh-hook`. |
| bash | `bash --rcfile <data>/shell/ta.bash -i` | `ta.bash` sources `/etc/profile`, then the first that exists of `~/.bash_profile`, `~/.bash_login`, `~/.profile`, then appends the report to `PROMPT_COMMAND`. |
| PowerShell (`pwsh`, `powershell.exe`) | `<shell> -NoExit -Command ". '<data>\shell\ta.ps1'"` | Runs after the user's profile and wraps the existing `prompt` function. |

- [x] **Step 1:** Write failing tests.
  - In `osc.rs`: `finds_cwd_split_across_feeds` (the sequence fed one byte at a time gives one `Cwd`), `decodes_percent_and_unicode` (`file://mac/Users/x/My%20Stuff/%E6%97%A5%E6%9C%AC/50%25%23` gives `/Users/x/My Stuff/日本/50%#`), `accepts_bel_and_st_endings`, `ignores_other_osc_and_oversized` (a title sequence and an unterminated 5000-byte one give nothing), `windows_drive_path` (`file://pc/C:/Users/x` gives `C:\Users\x` under `cfg(windows)`).
  - In `host/tests/shellinit.rs`, Unix only, each with a temp `HOME` and data folder: `zsh_reports_folder_at_prompt` (spawn, write `cd /tmp\n`, a `Cwd` event arrives with the real path of `/tmp`); `zsh_profile_order_is_unchanged` (user files `.zshenv`, `.zprofile`, `.zshrc`, `.zlogin` each append their name to a file; the result is that order, and `echo ${ZDOTDIR-unset}` prints `unset`); `zsh_keeps_a_custom_zdotdir` (start with `ZDOTDIR=$HOME/z`, files there load and `$ZDOTDIR` is `$HOME/z` inside); `zsh_user_precmd_still_runs`; `bash_reports_folder_and_loads_profile`; `unknown_shell_is_untouched` (`/bin/sh` gets argv `[sh, -l]` and no `ZDOTDIR`); `report_from_another_host_is_dropped` (printing a report with hostname `elsewhere` gives no `Cwd`); `report_for_a_missing_folder_is_dropped`.
  - In `host/tests/server.rs`: `cwd_frame_reaches_client`, `list_carries_cwd`, `proto_1_client_is_refused`.
- [x] **Step 2:** Run `cargo test -p term-host --test shellinit` and `cargo test -p term-host osc`. Expected: fail to compile.
- [x] **Step 3:** Implement `osc.rs` and the hook files. The session actor feeds every output chunk to the scanner before the screen. It keeps a `Cwd` only when the host part equals this computer's hostname (compare case-insensitively, and accept an empty host) and the folder exists, then stores it and sends `ClientEvent::Cwd` when it changed.
- [x] **Step 4:** Implement `shellinit`, call `install` at host start and `wrap` from `argv` for `Kind::Shell`. `Kind::Claude` keeps its stage 1 argv. Restore (stage 1 Task 11) uses the stored folder when there is one and falls back to `state::current_dir`.
- [ ] **Step 5:** Add the PowerShell case to the Windows CI job: `powershell_reports_folder_at_prompt` under `cfg(windows)`, using `powershell.exe`. Expected: green in `host.yml` on both systems.
- [x] **Step 6:** Add `cwd` to the TypeScript `HostFrame` union and `SessionItem`, and an `onCwd(session, cwd)` entry in `ConnectionEvents`. Run `pnpm -C client tsc --noEmit`. Expected: pass.
- [ ] **Step 7:** Run all host tests, clippy and fmt. Check by hand on Alex's real zsh: the prompt looks the same, `echo $ZDOTDIR` matches a kitty tab, and opening a tab is not visibly slower. Commit: `feat(host): load a shell hook and track each tab's folder`.

## Task 4: The tree: groups, tabs and their rules

**Files:** Create `host/src/project.rs`, `host/src/tree.rs`, `host/tests/tree.rs`. Modify `host/src/state.rs`, `host/src/server.rs`, `host/src/protocol.rs`, `host/src/lib.rs`, `host/tests/state.rs`.

**Interfaces:**
- Consumes: `ClientEvent::Cwd`, `State`, `SavedSession` and the saver thread.
- Produces:

```rust
// project.rs
pub struct Project { pub key: String, pub root: PathBuf, pub is_git: bool, pub branch: Option<String> }
pub fn resolve(folder: &Path) -> Project;      // never runs the git program
pub fn normalize_remote(url: &str) -> String;  // "github.com/owner/repo", lower case, no ".git"

// tree.rs  (all serde, snake_case)
pub struct Tree { pub groups: Vec<Group>, pub ungrouped: Vec<Tab>, pub closed: Vec<ClosedTab>, pub defaults: BTreeMap<String, GroupDefaults> }
pub struct Group { pub key: String, pub path: PathBuf, pub name: Option<String>, pub branch: Option<String>, pub collapsed: bool, pub tabs: Vec<Tab> }
pub struct Tab { pub id: String, pub name: Option<String>, pub pinned: bool, pub split: Split, pub last_used: u64 }
pub enum Split { Leaf { session: String }, Row { children: Vec<Split>, sizes: Vec<f32> }, Col { children: Vec<Split>, sizes: Vec<f32> } }
pub struct ClosedTab { pub tab: Tab, pub group: Option<String>, pub index: usize, pub panes: Vec<SavedSession> }
pub struct GroupDefaults { pub name: Option<String>, pub collapsed: bool }
pub enum Dir { Right, Down }
pub enum Op {
    OpenFolder { path: PathBuf }, CloseTab { tab: String }, ReopenTab,
    MoveTab { tab: String, group: Option<String>, index: usize }, MoveGroup { group: String, index: usize },
    RenameTab { tab: String, name: Option<String> }, RenameGroup { group: String, name: Option<String> },
    PinTab { tab: String, pinned: bool }, CollapseGroup { group: String, collapsed: bool },
    JoinTabs { tab: String, into: String, dir: Dir }, SplitOut { session: String },
    SetSizes { tab: String, split: Split }, Used { tab: String },
}
impl Tree {
    pub fn add_session(&mut self, session: &str, place: Place, cwd: &Path, kind_is_shell: bool) -> String; // returns tab id
    pub fn on_cwd(&mut self, session: &str, cwd: &Path) -> bool;      // true when the tree changed
    pub fn on_exit(&mut self, session: &str, clean: bool) -> bool;
    pub fn apply(&mut self, op: Op) -> Result<Effects, TreeError>;    // Effects { kill: Vec<String>, spawn: Vec<SavedSession> }
}
pub enum Place { NewTab, SplitOf { session: String, dir: Dir } }
```

- Protocol: `ClientFrame::Create` gains `place: Place` (default `NewTab`). New `ClientFrame::TreeGet` and `ClientFrame::TreeOp { op: Op }`. New `HostFrame::Tree { tree: Tree }`, sent in reply to `TreeGet` and pushed to every connected client after any change. `LayoutGet`, `LayoutSet`, `HostFrame::Layout` and `State.layout` are removed; `State` gains `tree: Tree`. Error codes `no_tab`, `no_group`, `bad_op`.

Rules, from `DESIGN.md` "Groups" and "Tabs" (the choices marked "plan" are made here):

- `resolve` walks up from the folder to the first `.git`. A `.git` folder is the root. A `.git` file (worktree or submodule) is read for its `gitdir:` line, and the remote comes from the main repo's `config` through `commondir`. The key is `remote:<normalize_remote(origin url)>`, or `folder:<root folder name>` with no remote or no git. A walk that reaches the home folder or the filesystem root without finding `.git` gives a non-git project whose root is the folder itself.
- A tab's folder is the folder of its first pane, reading the split left to right, top to bottom. (plan)
- `add_session` with `NewTab`: if `cwd` resolves to an existing group or to a git repo, the tab joins that group, creating it for a git repo. Otherwise it joins the group of the tab named by the last `Used` op, and with no such group it goes to `ungrouped`. New tabs go to the end of their group.
- `on_cwd`: for an unpinned tab, a move into a git repo moves the tab to that repo's group, creating it at the position sorted by folder path among groups. A move to a folder that is no group changes nothing. A pinned tab never moves.
- `MoveTab` by hand sets `pinned`. `PinTab` clears or sets it.
- `OpenFolder` creates a group for any folder, git or not, and returns a `spawn` effect for one shell tab in it, so the group is never empty.
- A group is removed when its last tab leaves or closes. Its `name` and `collapsed` are kept in `defaults` under its key and applied when the group comes back.
- `CloseTab` kills every pane and pushes a `ClosedTab` (keep the newest 20). A tab whose last pane exits cleanly is closed the same way without the push. `ReopenTab` pops the newest one, puts it back at the same group and index (recreating the group if needed), and returns `spawn` effects: a Claude pane as a resume of its chat, a shell pane as a fresh shell in its folder.
- `JoinTabs` puts `tab`'s whole split beside `into`'s split and removes `tab`. `SplitOut` makes a pane its own tab right after its old tab.
- On load, a saved leaf whose session is not in `State.sessions` is dropped, and a tab left with no leaf is dropped.

- [x] **Step 1:** Write failing tests.
  - `project.rs` unit tests with temp folders: `finds_root_from_a_subfolder`, `worktree_git_file_uses_main_repo_remote`, `no_remote_uses_folder_name`, `no_git_never_climbs_to_root` (a folder under temp with no `.git` resolves to itself), `normalize_remote_forms` (`git@github.com:Owner/Repo.git`, `https://github.com/owner/repo`, `ssh://git@github.com/owner/repo.git` all give `github.com/owner/repo`), `branch_from_head`, `detached_head_has_no_branch`.
  - `host/tests/tree.rs`: `cd_into_repo_moves_tab_and_creates_group`, `cd_out_to_plain_folder_stays`, `pinned_tab_never_moves`, `hand_move_pins`, `new_tab_joins_current_group_when_folder_is_no_group`, `new_tab_with_no_group_is_ungrouped`, `open_folder_makes_group_for_non_git`, `last_tab_out_removes_group_and_keeps_defaults`, `group_returns_with_its_name`, `new_group_sorted_by_folder`, `close_tab_kills_every_pane`, `clean_exit_of_last_pane_closes_tab_without_reopen_entry`, `reopen_restores_place_and_resumes_claude`, `closed_list_caps_at_20`, `join_then_split_out_round_trips`, `tree_round_trips_json`, `unknown_session_leaf_is_dropped`, `rename_survives_title_and_cwd_events`.
  - `host/tests/server.rs`: `tree_pushed_to_second_client_after_op`, `create_with_split_place_adds_pane`.
  - `host/tests/state.rs`: replace `layout_survives_restart` with `tree_survives_restart`.
- [x] **Step 2:** Run `cargo test -p term-host --test tree`. Expected: fail to compile.
- [x] **Step 3:** Implement `project.rs`, then `tree.rs` as pure logic with no I/O beyond `resolve`.
- [x] **Step 4:** Wire the server: one `Mutex<Tree>` in `Registry`, `Create` calls `add_session`, the event pump calls `on_cwd` and `on_exit`, `TreeOp` calls `apply` and carries out the effects, every change marks the state dirty for the saver and pushes `HostFrame::Tree`. Refresh each group's `branch` when one of its tabs reports a folder.
- [ ] **Step 5:** Run all host tests, clippy, fmt, and CI on Windows. Expected: pass. Commit: `feat(host): keep the tree of groups and tabs`.

## Task 5: Sidebar, tab switching and the focus rule

Needs Alex's go from Task 2 and `PLAN.md` Task 7.

**Files:** Create `client/src/store.ts`, `client/src/sidebar.ts`, `client/src/panes.ts`, `client/src/focus.ts`, `client/src/settings.ts`, `client/src/styles.css`, `client/src/store.test.ts`, `client/src/focus.test.ts`, `client/vitest.config.ts`. Modify `client/src/main.ts`, `client/src/connection.ts`, `client/index.html`, `client/package.json`, `client/src-tauri/tauri.conf.json`.

**Interfaces:**
- Consumes: `HostFrame::Tree`, `TreeOp`, `Create.place`, `cwd` from Tasks 3 and 4. `TerminalView` from stage 1.
- Produces:
  - `connection.ts`: `treeGet(): Promise<Tree>`, `treeOp(op: Op): void`, `ConnectionEvents.onTree(tree)`, and TypeScript types `Tree`, `Group`, `Tab`, `Split`, `Op`, `Place` matching Task 4 field for field.
  - `store.ts`: `class Store { tree: Tree; current: string | null; setTree(t: Tree): void; select(tab: string): void; rows(): Row[]; subscribe(fn: () => void): () => void }`, where `type Row = { kind: "group", group: Group } | { kind: "tab", tab: Tab, depth: 0 | 1 } | { kind: "pane", session: string, tab: string }`, plus pure helpers `nextCurrent(tree: Tree, closed: string, before: Tree): string | null`, `tabAt(tree, groupIndex, tabIndex)`, `leaves(split: Split): string[]`.
  - `panes.ts`: `class Panes { show(tab: Tab): void; focusCurrent(): void; viewFor(session: string): TerminalView | undefined; all(): TerminalView[] }`. It owns one `TerminalView` per session, created the first time its tab is shown, and lays a tab's split out as nested flex boxes.
  - `focus.ts`: `installFocusRule(root: HTMLElement, panes: Panes): void` and `holdFocus(field: HTMLElement): void`.
  - `settings.ts`: `loadSettings(): Promise<Settings>`, `saveSettings(patch: Partial<Settings>): Promise<void>`, with `type Settings = { fontSize: number; focusFollowsMouse: boolean; sidebarWidth: number; sidebarShown: boolean }`, stored as `settings.json` in the app's data folder through two Tauri commands, `settings_get` and `settings_set`.

- [ ] **Step 1:** Add `vitest` and `jsdom`, a `test` script (`vitest run`) and `vitest.config.ts` with the jsdom environment. Write failing tests.
  - `store.test.ts`: `rows_put_group_then_its_tabs_then_ungrouped`, `collapsed_group_hides_its_tabs`, `split_tab_lists_its_panes_under_it`, `closing_current_tab_selects_the_one_after_it_else_before_it`, `closing_last_tab_of_last_group_selects_null`, `current_survives_a_tree_push_that_moves_it`, `tree_push_that_drops_current_picks_a_neighbor`.
  - `focus.test.ts`: `click_on_sidebar_row_returns_focus_to_terminal`, `open_rename_field_keeps_focus_until_enter_or_escape`, `focus_returns_after_escape`.
- [ ] **Step 2:** Run `pnpm -C client test`. Expected: fail.
- [ ] **Step 3:** Implement `store.ts` and `focus.ts`. The focus rule: after any `mouseup`, `drop`, `contextmenu` close or tree change, if `document.activeElement` is not a field registered through `holdFocus`, call `panes.focusCurrent()`. Remove stage 1's `preventDefault` on mousedown outside `#terminal` and let this rule do the job.
- [ ] **Step 4:** Implement the sidebar and panes to the exact values in `docs/ui-review.md`: one scrolling list with no sticky headers (no `position: sticky` anywhere), the sidebar on the terminal's background with no divider, compact rows, group header with name or folder plus branch, a collapse arrow sending `CollapseGroup`, a click selecting a tab and sending `Used`. Only `transform` and `opacity` transitions. `main.ts` now starts by calling `treeGet`, creating one shell in the home folder with `create` and place `NewTab` when the tree is empty, and showing the tab that was current last time (kept in `localStorage`, guarded with try and catch).
- [ ] **Step 5:** Remove the title bar: set `decorations` to false in `tauri.conf.json`, drop `#bar` and `#drag-strip`, and put `data-tauri-drag-region` on the empty part of the sidebar. Add Close, Minimize and Enter full screen to the app menu with their usual Mac keys.
- [ ] **Step 6:** Run `pnpm -C client test`, `pnpm -C client tsc --noEmit` and `pnpm -C client tauri build`. Expected: pass. Check by hand on the release build: the window drags from the empty sidebar and resizes from every edge, AeroSpace still tiles it, 40 tabs in the narrowest sidebar still show every marker slot, scrolling carries group names away with their rows, and typing works at once after each of: row click, collapse, tab switch, window refocus. Take a real screenshot and compare it with the Task 2 mockup.
- [ ] **Step 7:** Commit: `feat(client): add the sidebar, tab switching and the focus rule`.

## Task 6: Keys, splits, rename and text zoom

**Files:** Create `client/src/keys.ts`, `client/src/zoom.ts`, `client/src/rename.ts`, `client/src/keys.test.ts`, `client/src/zoom.test.ts`. Modify `client/src/panes.ts`, `client/src/sidebar.ts`, `client/src/terminal.ts`, `client/src/main.ts`.

**Interfaces:**
- Consumes: `Store`, `Panes`, `holdFocus`, `Settings`, `treeOp`, `Place`.
- Produces:
  - `keys.ts`: `type Action` (a string union, one per row below), `const DEFAULT_KEYS: Record<Action, string>`, `matchKey(e: KeyboardEvent, keys: Record<Action, string>): Action | null`, `runAction(a: Action, ctx: { store: Store; panes: Panes; connection: Connection }): void`.
  - `zoom.ts`: `nextSize(current: number, step: 1 | -1 | 0): number` (0 means reset to 14, result clamped to 8 to 40), `applyZoom(panes: Panes, size: number): void`, `spacingFor(size: number, measuredCellWidth: number): number`.
  - `terminal.ts`: `TerminalView.setFontSize(px: number): void` and `TerminalView.setDimmed(on: boolean): void`.
  - `rename.ts`: `startRename(row: HTMLElement, current: string, done: (name: string | null) => void): void`.

Keys. The final table is whatever `docs/ui-review.md` records; these are the defaults the review starts from.

| Action | Key | What it sends |
|---|---|---|
| New tab | Cmd+T | `create` shell, `NewTab`, in the current pane's live folder |
| Close pane | Cmd+W | `kill` the pane's session |
| Close tab | Cmd+Shift+W | `CloseTab` |
| Reopen closed tab | Cmd+Shift+T | `ReopenTab` |
| Next, previous tab | Cmd+Shift+] and [ | through `Store.rows()` order, skipping collapsed groups |
| Jump to group 1 to 9 | Cmd+1 to 9 | the tab in that group with the highest `last_used` |
| Jump to tab 1 to 9 in the group | Ctrl+1 to 9 | |
| Split right, split down | Cmd+D, Cmd+Shift+D | `create` shell with `SplitOf` the focused pane |
| Move between panes | Cmd+Option+arrows | pending the review |
| Fill the tab with one pane | Cmd+Shift+Enter | client only |
| Show or hide the sidebar | Cmd+B | `saveSettings` |
| Rename | Cmd+Shift+R | `startRename` on the current row |
| Zoom in, out, reset | Cmd+= and Cmd+plus, Cmd+minus, Cmd+0 | `applyZoom`, `saveSettings` |
| Copy, paste | Cmd+C, Cmd+V | as in stage 1 |

- [ ] **Step 1:** Write failing tests.
  - `keys.test.ts`: `every_action_has_a_default`, `no_default_uses_option_with_a_letter_or_digit`, `no_two_actions_share_a_key`, `cmd_3_jumps_to_last_used_tab_of_third_group`, `ctrl_2_picks_second_tab_in_current_group`, `next_tab_skips_collapsed_group`, `jump_past_the_last_group_does_nothing`.
  - `zoom.test.ts`: `zoom_steps_one_px`, `zoom_clamps_to_8_and_40`, `reset_goes_to_14`, `zoom_applies_to_every_terminal` (a fake `Panes` with three views all get `setFontSize(15)`), `spacing_keeps_whole_cells` (`spacingFor` returns the amount that makes the cell width a whole number of device pixels, 1 at 14 px with the measured width from the stage 1 handoff).
  - `store.test.ts`: `resize_rounds_to_whole_cells`.
- [ ] **Step 2:** Run `pnpm -C client test`. Expected: fail.
- [ ] **Step 3:** Implement keys, zoom and rename. Rename opens on the key or a double click, registers its field with `holdFocus`, sends `RenameTab` or `RenameGroup` on Enter (an empty name sends `null`, which brings the automatic name back) and nothing on Esc. A row shows `name` when set, otherwise the terminal title, otherwise the folder name. New terminals open at the saved size.
- [ ] **Step 4:** Implement splits in `panes.ts`: dividers drag to resize and send `SetSizes` on release; the focused pane is full brightness and the others get `setDimmed(true)`; with `focusFollowsMouse` on, `mouseenter` on a pane focuses it. Drag a sidebar row within its list to reorder (`MoveTab`, `MoveGroup`), onto another tab row to join (`JoinTabs`, right by default, down with Shift held), and drag a pane row out to a gap between rows to pull it out (`SplitOut`). Right-click menu on a row: rename, pin or unpin, close, move to group.
- [ ] **Step 5:** Run tests, `tsc` and `tauri build`. Expected: pass. Check every key by hand on the release build, that no key shows a popup or readout, that zoom changes every pane in every tab and survives a restart, and that the sidebar text does not change size with zoom.
- [ ] **Step 6:** Commit: `feat(client): add keys, splits, rename and text zoom`.

## Task 7: Hand check and stage 1 speed pass

**Files:** Modify `DESIGN.md` (speed tables, any limit found).

- [ ] **Step 1:** Daily-use check with Alex on the release build: open three real projects by `cd`, split, rename, reorder, quit and reopen, reboot and reopen. Everything returns in the same groups, order, names and splits. Record anything that felt wrong and fix it before going on.
- [ ] **Step 2:** Run `PLAN.md` Task 12 (speed pass) now, with the sidebar showing and 20 tabs open. It needs Alex's go for each `tools/keylat` run.
- [ ] **Step 3:** Commit what changed: `docs: record the sidebar hand check and speed pass`.

## Task 8: Claude hooks, session states and the `claude` flags

Needs the rulings from Task 1.

**Files:** Create `host/src/ctl.rs`, `host/src/hookcmd.rs`, `host/src/states.rs`, `host/src/claudecfg.rs`, `host/tests/states.rs`, `host/tests/ctl.rs`. Modify `host/src/main.rs`, `host/src/server.rs`, `host/src/protocol.rs`, `host/src/tree.rs`, `host/src/pty.rs`, `host/shell/ta.zsh`, `host/shell/ta.bash`, `host/shell/ta.ps1`, `host/tests/shellinit.rs`.

**Interfaces:**
- Consumes: `TA_SESSION`, `TA_DATA`, `Tree`, `project::resolve`, the saver.
- Produces:
  - `ctl::serve(data_dir: &Path, on: impl Fn(CtlRequest) + Send + 'static) -> io::Result<()>` listening on the local socket `<data_dir>/ctl.sock` (a named pipe `\\.\pipe\terminal-app-ctl-<user>` on Windows). One request is one JSON line: `CtlRequest { token: String, session: String, event: String, payload: serde_json::Value }`. A request whose token is not that session's is dropped without a reply.
  - Each spawned process also gets `TA_TOKEN`, 32 random bytes in hex, made per session and never written to disk.
  - `term-host hook <event>`: reads the hook JSON on stdin, sends one `CtlRequest`, and exits 0 within 300 ms whatever happens, printing nothing.
  - `claudecfg::write(data_dir: &Path, host_exe: &Path) -> io::Result<PathBuf>` writes `<data_dir>/claude-hooks.json`, a Claude settings file whose hooks run `<host_exe> hook <event>` for `SessionStart`, `UserPromptSubmit`, `PreToolUse` (matcher `Edit|Write` plus the helper tool name from Task 1), `Notification`, `Stop`, `SubagentStop` and `SessionEnd`.
  - `states.rs`:

```rust
pub enum AgentState { Working, Blocked, Done, Idle }
pub enum HookEvent { Start { chat: String }, Prompt, Tool { edited: Option<PathBuf>, helper_started: bool }, NeedsInput, HelperStop, Stop, End }
pub fn parse(event: &str, payload: &serde_json::Value) -> Option<HookEvent>;
pub struct Tracker;
impl Tracker {
    pub fn on_event(&mut self, session: &str, ev: HookEvent, now: Instant) -> Change;
    pub fn tick(&mut self, now: Instant) -> Vec<(String, AgentState)>;   // delayed "done"
    pub fn seen(&mut self, session: &str) -> Change;
    pub fn state(&self, session: &str) -> Option<AgentState>;
    pub fn blocked_oldest_first(&self) -> Vec<String>;
}
pub struct Change { pub state: Option<AgentState>, pub first_edit: Option<PathBuf>, pub chat: Option<String> }
```

  - Protocol: `HostFrame::States { items: Vec<StateItem> }` with `StateItem { session: String, state: Option<String>, since_ms: u64 }`, pushed on every change and after `Hello`. `ClientFrame::Seen { session: String }`. Both use the session uuid, the same id the tree's leaves hold.
  - `Tree::on_first_edit(&mut self, session: &str, file: &Path) -> bool`.
  - `<data_dir>/claude-args`: one argument per line. The host writes it from `settings.json` keys `claude_flags_enabled: bool` (default true) and `claude_flags: Vec<String>` (default `["--dangerously-skip-permissions"]`). When disabled the file is empty.

State rules (from "Session states" and "Alerts"; where Task 1 ruled differently, Task 1 wins and this list is updated first):

- `Prompt` or `Tool` sets Working. `NeedsInput` sets Blocked. `End`, or the tab's process exiting, clears the state.
- `Stop` sets Done only when no helper of that session is running, by the Task 1 rule. Otherwise the session stays Working, and the `Stop` that follows the last helper sets Done.
- A helper finishing (`HelperStop`) never sets Blocked or Done.
- `seen` turns Done into Idle. The client sends `Seen` when a Done session's tab is the current tab and the window is focused, and when it becomes so.
- Blocked stays until the next `Prompt`, `Tool` or `Stop`.
- A session with no hook events has no state and shows no marker.
- `first_edit` is filled the first time a `Tool` event carries an edited file. `Tree::on_first_edit` moves the tab to the file's git repo group only if the tab is in `ungrouped` and not pinned.
- `Start` gives the chat id. The host records that this tab is a Claude tab with that chat, so restore reopens it with `--resume`, which removes the stage 1 limit for a hand-typed `claude`.

The `claude` function in each hook file (zsh, bash, PowerShell):

- It runs the real `claude` with `--settings "$TA_DATA/claude-hooks.json"`, then every line of `$TA_DATA/claude-args`, then Alex's own arguments. The file is read on each call, so the setting takes effect in open tabs.
- It adds nothing when the first argument is one of Claude's subcommands (take the list from `claude --help` at build time and keep it in one variable per hook file), or when `TA_SESSION` is not set.
- It skips any flag from `claude-args` that Alex already typed.
- `Kind::Claude` tabs started by the app get the same `--settings` and `claude-args` flags in `pty.rs`.

- [x] **Step 1:** Write failing tests.
  - `host/tests/states.rs`: `prompt_then_stop_is_done`, `stop_with_helper_running_stays_working`, `stop_after_last_helper_is_done`, `helper_stop_never_alerts`, `needs_input_is_blocked_until_next_prompt`, `seen_turns_done_into_idle`, `seen_does_not_clear_blocked`, `end_clears_state`, `blocked_list_is_oldest_first`, `first_edit_reported_once`, `parse_reads_edit_path` and `parse_reads_notification_kinds` (both using JSON copied from the Task 1 log into `host/tests/fixtures/hooks/`).
  - `host/tests/tree.rs`: `first_edit_moves_ungrouped_tab`, `first_edit_leaves_grouped_tab`, `first_edit_leaves_pinned_tab`.
  - `host/tests/ctl.rs`: `hook_reaches_host` (run the built `term-host hook stop` with the env of a live session and see a `States` frame), `wrong_token_is_ignored`, `hook_exits_zero_fast_with_no_host` (no socket present: exit code 0, empty output, under 300 ms), `hook_exits_zero_fast_when_host_never_answers` (a socket that accepts and never reads).
  - `host/tests/shellinit.rs`, with a fake `claude` on the PATH that prints its arguments: `claude_gets_settings_and_flags`, `claude_subcommand_gets_nothing` (`claude mcp list`), `claude_flag_not_repeated`, `claude_flags_off_still_gets_settings`, `changed_flags_apply_in_an_open_shell`.
  - `host/tests/state.rs`: `hand_typed_claude_restores_with_resume`.
- [x] **Step 2:** Run them. Expected: fail to compile.
- [x] **Step 3:** Implement `states.rs` as pure logic, then `ctl.rs`, `hookcmd.rs` and `claudecfg.rs`, then the server wiring (a 250 ms tick for delayed Done only if Task 1 ruled for the delay) and the hook file functions.
- [ ] **Step 4:** Run all host tests, clippy, fmt, and CI on Windows. Expected: pass.
- [ ] **Step 5:** Check by hand with a real Claude tab on Haiku: type `claude` in a shell tab and confirm bypass is on; ask it to edit a file in another repo from an ungrouped tab and watch the tab move; start a background helper and confirm no Done until it finishes; confirm `~/.claude/settings.json` is byte for byte unchanged.
- [x] **Step 6:** Commit: `feat(host): track session states from Claude hooks and add flags to claude`.

## Task 9: State markers, rollup and the jump key

**Files:** Modify `client/src/connection.ts`, `client/src/store.ts`, `client/src/sidebar.ts`, `client/src/panes.ts`, `client/src/keys.ts`, `client/src/styles.css`, `client/src/store.test.ts`, `client/src/keys.test.ts`.

**Interfaces:**
- Consumes: `HostFrame::States`, `ClientFrame::Seen`, the markers from `docs/ui-review.md`.
- Produces: `ConnectionEvents.onStates(items)`, `Connection.seen(session)`, `Store.setStates(items)`, `Store.stateOf(session): AgentState | null`, `Store.tabState(tab: Tab): AgentState | null`, `Store.groupBlocked(group: Group): boolean`, `Store.nextBlocked(after: string | null): string | null`, and `Action` gains `"jumpBlocked"` on Cmd+Shift+U.

- [ ] **Step 1:** Write failing tests: `tab_state_is_blocked_if_any_pane_is_blocked`, `tab_state_order_is_blocked_then_done_then_working_then_idle`, `collapsed_group_still_reports_blocked`, `jump_goes_oldest_first_and_wraps`, `jump_with_nothing_blocked_does_nothing`, `jump_expands_a_collapsed_group`, `seen_sent_when_done_tab_becomes_current`, `seen_not_sent_while_window_is_unfocused`.
- [ ] **Step 2:** Run `pnpm -C client test`. Expected: fail.
- [ ] **Step 3:** Implement. One still marker per row in a fixed-width slot that never shrinks, the same marker on a split tab's pane rows and on the pane's corner, a group marker when any of its tabs is blocked. A blocked or done row shows one extra dim line with the session's last title; other rows stay one line. No animation on markers and no sound (sounds belong to the alerts plan).
- [ ] **Step 4:** Run tests, `tsc` and `tauri build`. Check by hand with three real Claude tabs: each state shows, blocked rolls up through a collapsed group, Cmd+Shift+U walks the blocked ones oldest first, and looking at a done tab clears it. Compare a screenshot with the Task 2 states strip.
- [ ] **Step 5:** Commit: `feat(client): show session states with blocked rollup`.

## Task 10: The host as a Mac service

**Files:** Create `host/src/service.rs`, `host/tests/service.rs`. Modify `host/src/main.rs`, `client/src-tauri/src/host.rs`.

**Interfaces:**
- Produces: `term-host service install`, `term-host service uninstall`, `term-host service status`, and `service::plist(exe: &Path, data_dir: &Path) -> String`.

- [x] **Step 1:** Write failing tests: `plist_has_keepalive_and_run_at_load` (label `app.terminal.host`, `KeepAlive` true, `RunAtLoad` true, the exe path, logs under the data folder), `plist_escapes_paths_with_spaces_and_ampersands`.
- [x] **Step 2:** Run them. Expected: fail.
- [x] **Step 3:** Implement for macOS: write `~/Library/LaunchAgents/app.terminal.host.plist` and load it with `launchctl bootstrap gui/<uid>`; `uninstall` boots it out and removes the file. The app runs `service install` on first start when the installed plist is missing or points at another exe, and no longer starts a detached host itself when the service is installed. Windows keeps stage 1's detached start; its service is the PC plan's first task.
- [ ] **Step 4:** Check by hand: `kill -9` the host and it is back within 10 s with shells restored; log out and in and it is running before the app opens; `service uninstall` leaves nothing behind.
- [x] **Step 5:** Commit: `feat(host): run as a launchd service that restarts itself`.

---

## Open questions for Alex

- None block Tasks 1 and 2. Task 2 ends with the review that needs Alex's go.

## Handoffs

(One short note per finished task: what changed, what was verified and how, anything the next task should know.)

- Task 1 (0b3c36b): rulings (a) to (f) are in `docs/risky-tests.md` section 3, proven in real interactive runs. For Task 8: hold "done" while `Stop.background_tasks` has a running `subagent` entry; a helper's events carry `agent_id`; "blocked" is set by `PermissionRequest` and cleared by `PostToolUse` with no matcher on events without `agent_id`, plus `Stop` and `UserPromptSubmit`. Permission prompt tests on this Mac need `--permission-mode default`.
- Restore work (fcf34c0): the stage 1 restore task was brought onto this branch and finished, so Task 3 can start. Hand check still owed: shells in different folders and a real Claude tab surviving a host kill, and the window size coming back.
- Task 3 (1eb6d91, 8de64e4): shells load a hook that reports their folder at each prompt; the host stores it, sends `Cwd` on change and on attach, and lists it in `SessionItem.cwd`. Folders are resolved to their real path (`/tmp` arrives as `/private/tmp`), so Task 4 can compare folders as plain strings. A restored shell lists its saved folder at once. Verified by host tests on macOS (zsh and bash in a temp home). Steps 5 and 7 stay open: the PowerShell test and the `ta.ps1` path fix have never run (they need a push so the Windows job runs), and the real zsh hand check is in the final checklist. For Task 8: `TA_SESSION` and `TA_DATA` are set on every spawned process, and the hook files live in `host/shell/`.
- Task 4 (e463286, a9d518b, 1663b6d): the host keeps one tree of groups, tabs and splits (`host/src/tree.rs`), finds each folder's project without running git (`host/src/project.rs`), saves the tree with the state and pushes `HostFrame::Tree` to every client after a change. The protocol is now `PROTO = 3`: `Create.place`, `TreeGet`, `TreeOp`, and the layout frames are gone; the client keeps its last shown session in localStorage. Verified by 135 host tests on macOS, clippy for both targets, fmt, tsc and the client build, plus a review and a scoped re-review. Step 5 stays open: the Windows job needs a push, and a built app has not been launched since the protocol changed (final checklist). Rules for later tasks: the server resolves a project BEFORE taking the tree lock and uses the `*_in` tree calls, so nothing touches the disk under the lock; lock order is watchers before tree, and sessions and tree are never held together; a new pane goes into the tree before `registry.add`. Known small gaps, parked for the final review: two folder reports for one session can apply out of order (wrong group or stale branch until the next report); a pane split in by another window during a close is not reopened; the Windows home check is case-sensitive.
- Task 8 (064ee68, 7972049, 235809e, fix 4957fa3): Claude hooks now report to the host. `term-host hook <event>` sends each event over a local socket at `<data>/ctl/ctl.sock` (folder 0700, socket 0600) with the session's `TA_TOKEN`; `host/src/states.rs` turns events into Working, Blocked, Done or Idle, with a 2 s wait before Done on a 250 ms tick; `Seen` turns Done into Idle. The host sends `HostFrame::States` together with the tree in answer to `tree_get` and after each change. Claude tabs start with `--settings <data>/claude-hooks.json --dangerously-skip-permissions`, and a `claude` function in `ta.zsh`, `ta.bash` and `ta.ps1` adds the same flags to a hand-typed claude and sends `ClaudeExit` through `$TA_HOST` when claude returns, so a crashed claude clears its state and the tab restores as a shell. Verified by 170 host tests on macOS, clippy for both targets, fmt, a review and a scoped re-review. Steps 4 and 5 stay open: the Windows job needs a push, and the hand check with a real Claude tab is in the final checklist. For Task 9: plain shells are always Idle; states arrive keyed by session id; `PostToolUseFailure` is not registered. Known small gaps, parked for the final review: a tiny window in `on_hook` after the session check; bash does not send `ClaudeExit` if the function itself is interrupted; the PowerShell function has never run; a crashed `Kind::Claude` tab keeps a stale chat id.
- Task 10 (963a73f): `term-host service install|uninstall|status` writes `~/Library/LaunchAgents/app.terminal.host.plist` (label `app.terminal.host`, `KeepAlive` and `RunAtLoad` true, logs under the data folder) and loads it with `launchctl bootstrap gui/<uid>`. The release app installs the service on first start, or when the plist is missing, points at another exe or the port is silent, and then no longer starts a detached host; a debug build, an app run from a temporary macOS copy, or a failed install keeps the old detached start. `install` refuses to run while `TERM_HOST_PORT` or `TERM_APP_DATA` is set. Windows prints "not supported yet". Verified by 183 host tests with a fake `launchctl` and a temp home, clippy for both host targets, fmt, client clippy and tests, and a review. Nothing has run against real launchd: Step 4 stays open for the final checklist. Known behaviours, parked for the final review: launchd restarts the host after an intended quit, so a quit action must boot the service out; an in-place app update keeps the old host running until its next exit; an old detached host holding the port makes the service retry every 10 s into a log nothing trims; an app run from a mounted disk image is not guarded; client clippy for Windows cannot run on the Mac.
