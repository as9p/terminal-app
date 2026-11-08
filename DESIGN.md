# Terminal app design

Working name only. Written 2026-11-08. Status: Alex approved on 2026-11-08, and stage 1 is being built (see `PLAN.md`). Later stages can still change.

Labels: **(Alex)** means Alex decided it. **(my call)** means Claude decided it and Alex can overrule it. Unlabeled lines are facts found by testing or reading code.

## Why this exists

Alex runs many Claude Code sessions and likes Orca's idea of grouping them, but Orca feels bloated. Sessions must attach to a project, everything lands in one "projects" workspace, it creates worktrees Alex never asked for, and its sidebar and right-click menus are cluttered. kitty is Alex's favorite terminal but has none of these session features.

This app should feel like kitty and do the useful things Orca does, with the extra features out of sight until asked for. Alex uses it alone for now and would open source it. It may grow into Alex's own terminal or harness one day.

## Goals

- Feels like kitty in look, keys and speed, and is as optimized as possible. **(Alex)**
- New tab or new Claude opens instantly as a default session in the sidebar, with no project needed. **(Alex)**
- Sessions group into projects, and the same projects appear on every computer. **(Alex)**
- A session moves itself to the project it starts working in. **(Alex)**
- Works from a Mac laptop, a Windows PC and a phone over Tailscale. **(Alex)**
- Multiple Claude accounts with automatic switching at the usage limit. Alex calls this of utmost importance. **(Alex)**
- Alex always hears about it when a session is stopped and waiting on them. **(Alex)**
- Runs other agent tools and local models, not just Claude, and any session can message any other. Alex said "maybe" on local models. **(Alex)**

## Non-goals for now

- Company servers, accounts or a relay of this app's own. Everything the app itself runs stays on Alex's tailnet. **(Alex)** Claude's own Remote Control is the one exception, because it goes through Anthropic's service and Alex asked for it (see "Remote Control").
- Cloud computers, Jira, Linear, GitLab, voice dictation and telemetry. **(my call)**
- Custom account switching flows. Alex called this rambling and only wants the default flow. **(Alex)**

## Anti-goals: it failed if

- It ever creates a worktree without being asked. **(Alex)**
- A floating icon can't be removed, or `[primary]` or `main` labels appear in the sidebar. **(Alex)**
- Rename takes more than one simple action. **(Alex)**
- Seeing your projects and sessions needs a pop-out panel the way Orca's workspaces do. Alex calls Orca's version bad. **(Alex)**
- It feels like an Electron app that happens to contain a terminal.

## Devices and where things run

- There is no home server. The Windows PC is the main host. **(Alex)**
- New tab opens on whichever device you're on. **(Alex)**
- On the phone, a new tab is a real tab on the PC, in the same sidebar. The phone is a screen into the PC's terminal. **(Alex)**
- Running work on the PC may die if the PC drops. The laptop can take over manually by reopening the chat. **(Alex)**
- brawlfarm, bh, Minecraft and Android work stay tied to the PC. **(Alex)**
- A Mac-only tab is an ordinary private terminal. Other devices can't see or reach it. **(Alex)** Normal Mac tabs are visible to other devices while the Mac is awake. Mac-only is opt-in. **(my call)**

## Architecture

**(my call)** unless marked.

- Each computer runs a small host process written in Rust. It owns the terminals, keeps them alive when the window closes, and keeps a copy of each screen so a client can reconnect. A Rust renderer is built only if xterm.js misses the speed targets below, and the host stays the same either way.
- Clients attach to any host over Tailscale. Only devices on Alex's tailnet can connect. **(Alex)**
- Mac and Windows clients are Tauri apps with xterm.js using WebGL. The phone is a web page using the same terminal code.
- The host runs on Mac and Windows from the first build, because the PC is the main host. Any terminal-process layer must handle both Unix terminals and Windows ConPTY. The Mac client comes first and the Windows client follows once the host and tabs work.
- Hosts accept connections only over Tailscale, and each new device pairs once to get its own token, so being on the tailnet isn't enough by itself.
- The host serves the phone page over HTTPS using a Tailscale certificate. The phone's Face ID lock is a passkey, which requires that HTTPS.
- If real keypress testing shows it can't match kitty's feel, build a native Mac client and leave the phone on the web. The host stays the same.
- The host never depends on xterm.js. It sends plain terminal bytes and knows nothing about the client's drawing code, so the drawing can be replaced on one platform without touching the host.
- Reuse Orca's daemon design, including screen snapshots, as a reference. Rewrite the sidebar, panes, persistence, startup and byte path. Skip its relay and cloud code.

### Speed evidence

30 MB of colored text, three times each, on Alex's Mac, measured by how fast the terminal finished processing it.

| Terminal | MB/s |
|---|---|
| kitty, `scrollback_pager_history_size` off | 70 |
| Test build, xterm.js | 47 |
| kitty, Alex's config | 39 to 49 |
| Test build, ghostty-web | 31 |
| Orca, hidden tab | 25 |
| macOS Terminal | 18 |
| Orca, visible tab | 9.7 |

Test build memory was about 500 MB from a debug build with one tab. Orca used 2.2 to 2.6 GB with all of Alex's sessions open. The test build has no sidebar or host process, so the real app will lose some speed.

Typing latency, 60 presses of a key in each terminal running `cat`, measured from key press to the first changed screen frame at 120 fps on Alex's Mac:

| Terminal | Fastest | Median | 95th percentile | Slowest | Backspace median |
|---|---|---|---|---|---|
| macOS Terminal | 9.5 ms | 15.8 ms | 22.3 ms | 28.8 ms | 16.1 ms |
| Orca, visible tab | 9.1 ms | 17.8 ms | 26.0 ms | 31.7 ms | 17.2 ms |
| kitty, Alex's config | 11.0 ms | 19.0 ms | 24.9 ms | 80.9 ms | 16.7 ms |
| Test build, xterm.js | 12.5 ms | 21.4 ms | 34.6 ms | 41.5 ms | 22.9 ms |

Terminal, Orca and kitty are effectively tied: the median gap is 3.2 ms and one screen frame is about 8 ms. In this test xterm.js in Electron does not lose to native terminals on typing.

The test build passes the latency target on its typing median, 2.4 ms behind kitty, so the native Mac client path is not triggered. Both were watched across the whole window, so kitty's 19.0 ms is the fair comparison, not its 17.4 ms spot-only rerun. Two weaker results remain:

- Backspace median is 6.2 ms behind kitty, and the 95th percentile is 9.7 ms behind. Both gaps are about one screen frame or less.
- 11 of 60 presses took over 30 ms, against 1 for kitty and Orca and 0 for Terminal. The cause isn't known.

The test build is a debug build that sends every key through a Node server and a WebSocket, which the Rust host would replace. Measure the real app again as a release build.

Limits of this test:

- Each terminal had one run, so small gaps may not repeat.

- It's a homemade tool using macOS screen capture, not cross-checked against a camera or another tool. Trust the comparison more than the absolute numbers.
- It measures `cat` echoing a key, not typing into Claude Code, which draws its own input box.
- Orca ran at its normal 1728x1084 with its sessions open, the test build ran at 1728x1084, and kitty and Terminal ran at about 1200x800.
- ghostty-web has no typing number, because the tool's calibration fails on it. Calibration also fails on xterm.js, but its run doesn't need calibration because its screen doesn't change when idle.
- Orca's window changes about 13 times per second on its own, so its test watched only the 18x36 pixel spot where the character appears. kitty and Terminal were watched across the whole window. Rerunning kitty with the spot-only method gave a median of 17.4 ms against 19.0 ms, so the method difference is small, and that kitty rerun is level with Orca's 17.8 ms.

Speed targets for the real app **(my call)**: at least 40 MB/s with a visible tab through the host process, and typing and backspace medians within 5 ms of kitty on the same machine, measured on a release build. Missing the latency target triggers the native Mac client path. Also report the 95th percentile and the share of presses over 30 ms, since a good median can hide slow presses.

Peak speed work **(Alex asked for peak Mac speed, the method is my call)**: every release build gets the typing test, so a slowdown shows up right away. Stage 1 ends with a speed pass that finds the cause of the slow presses and tries to match or beat kitty, beyond the 5 ms target. If xterm.js still can't get there, replace only the terminal drawing on the Mac before considering a full native client.

### What Orca's site says

From onorca.dev on 2026-11-08, as Orca's own claims and not tested here:

- It publishes no latency numbers. It uses "the same xterm.js-based terminal VS Code uses" and lists a "GPU-rendered terminal with splits".
- It supports multiple Claude accounts with one-click switching while sessions stay live, and lists 27 supported agents. Agent status comes from hooks. Remote Control support isn't stated.
- It's free under the MIT license.
- Its Relay needs a sign-in and the same account on both ends. LAN pairing doesn't, and `orca serve --pairing-address <ip>` works over Tailscale, with `--mobile-pairing` for a QR code.

### Herdr

Herdr (herdr.dev, github.com/herdrdev/herdr) is a free Rust tool under Apache 2.0 that runs inside a terminal like tmux, on Mac, Linux and Windows. Alex wants its features in this app **(Alex, 2026-11-08)**. From a video about it and its README, not tested here:

- A background server owns the terminals, and closing the window keeps agents running. A reboot ends the programs and brings back the layout. Chats come back only for agents with its integration installed, and other panes come back as a fresh shell in the same folder.
- The sidebar marks each agent as working, blocked, done or idle. Blocked rolls up: one blocked agent marks its tab and its workspace, and one click jumps to it.
- You run `claude` in a normal pane and it notices. It knows 22 agent tools without a wrapper.
- Status comes from reading the screen, so a new kind of prompt can show as idle. `herdr integration install` adds each tool's own signals. Its docs warn that idle is not a yes.
- Agents drive it through a command line and a socket: split a pane, start another agent, send it a prompt, and `herdr agent prompt reviewer "..." --wait` blocks until that agent is done or blocked.
- `herdr machine add <name>` adds another computer over SSH, and its agents show in the same sidebar.
- Apache 2.0 allows copying its code, keeping its license and NOTICE file.

## Sessions, projects and the sidebar

- A session starts in a default group with no project. **(Alex)**
- Smart move: Claude Code `PreToolUse` and `PostToolUse` hooks report `Edit` and `Write` calls with `tool_input.file_path`. The app resolves the file to its git root and moves the session to that project. It moves on the first edit, not on reads. Dragging a session by hand pins it there for good. **(my call on the rule, Alex asked for the behavior)**
- A project is identified by its git remote, or its folder name if it has none, so the same project matches across computers. **(my call)**
- Every host keeps a full copy of the project list and session groupings and exchanges changes with other hosts when they're both online. If two hosts changed the same item, the newest change wins. There is no central store, so the Mac still shows projects when the PC is off. **(my call)**
- Sessions on an offline computer stay in the sidebar, greyed, with when they were last seen. Claude chats on them offer a manual reopen here, as long as the chat history is available on this computer. Plain terminals only show as offline. **(my call)**
- Sidebar: no status clutter, removable repo icons, one-action inline rename, short right-click menus and useful view settings. **(Alex)** Exact look goes through a `ui-review-page` review before any UI code.
- Top right stays nearly empty. **(Alex)**
- Every project group and session sits in the always-visible sidebar, the way Herdr lists them, with no pop-out workspace panel. **(Alex)** Groups collapse in place. **(my call)**
- Every agent session shows one of four states: working, blocked (waiting on Alex), done (finished its turn, not yet looked at) and idle. **(Alex, from Herdr)** It is one small marker, not text, so it fits "no status clutter". The marker's look goes through the same `ui-review-page` review. **(my call)**
- Blocked rolls up: a blocked session marks its tab, its split, and its project group in the sidebar, even when collapsed, and one key or click jumps to the blocked session. With several blocked, the key goes through them oldest first. **(Alex asked for Herdr's rollup, the order is my call)**
- Done clears when Alex looks at the session. **(my call)**
- Top tabs must not collapse the way Orca's do, which Alex calls "kinda unnatural". The status marker on a tab must stay visible no matter how narrow the tab gets, because in Orca it disappears when tabs get small. **(Alex)** What tabs do when many are open goes through the same `ui-review-page` review. Compare Orca's and kitty's behavior there, using real screenshots.

## Claude across devices

- Claude runs on the device you're on and can see and message sessions on the others, edit files and run commands there, with no asking. **(Alex)**
- Claude reaches other computers through the app's own command line tool, which calls the host on that computer. It covers send, read and wait for sessions, plus run command and edit file. The host writes the log, so it can't be skipped. **(my call)**
- Each session keeps a visible log of what it changed on other computers. **(my call)**
- The phone gets a Face ID lock. **(my call)**
- Known risk, already told to Alex: bypass mode plus cross-device access means one bad command can hit every computer. The log shows it afterward but doesn't stop it.

### Move session to Mac or PC

**(Alex asked for the feature, the validity rule is my call.)**

- Available only for a Claude chat, when the other machine is awake, the project exists there or can be pulled, and the project isn't machine-tied. Otherwise the option is greyed with the reason.
- Mid-turn, it waits for the turn to end.
- It carries uncommitted changes and chat history. Running commands don't move.
- Resuming a chat under a different path works on the Mac: `claude --resume <id>` from a moved folder found the chat with no copying, and continued it in the original transcript file (tested 2026-11-08, see `docs/risky-tests.md`). Mac to Windows is still untested, including when the old path's folder is missing on the other machine. Test this before promising it.

## Remote Control

Remote Control is Claude Code's own feature for opening a session on this computer from another device. It is separate from the Tailscale path: the Claude apps can reach these sessions, and it works without this app's client on the other device.

**Wanted (Alex):**
- The sidebar shows every available Remote Control session.
- Several Claude accounts can be logged in at once, so sessions from all of them show together.
- A setting silently runs `claude remote-control` whenever the terminal opens, so any device can open a session with this computer as the host.

**How it would work (my call):**
- The host process starts the Remote Control server, not a visible tab, so it stays out of the way and keeps running when the window closes.
- One server per signed-in account, each with `CLAUDE_CONFIG_DIR` pointing at that account's folder, because a server runs under one login.
- The setting is off by default in the public version, and has a folder to start in, since the server works in one directory.
- The setting exposes the real flags. Alex mentioned "question params", which I took to mean these. `--name` and `--remote-control-session-name-prefix` set the names, `--spawn` picks same directory, worktree or session, `--capacity` sets the limit (default 32), and `--permission-mode` sets the mode.
- `--spawn` defaults to same directory. The app never offers worktree as a default, to respect the no-surprise-worktrees rule.

**Limits, from `claude remote-control --help` on version 2.1.319:**
- The computer has to stay awake and the server has to keep running. The app's keep awake feature would need to cover this.
- It needs a subscription login. A folder Claude hasn't been trusted in needs `claude` run there once first.

**Not tested:**
- Remote Control sessions: no command lists them (checked 2026-11-08, see `docs/risky-tests.md`). `~/.claude/sessions/<pid>.json` has a `bridgeSessionId` field for running local sessions, but whether sessions from a `claude remote-control` server fill it is unknown. Sessions started elsewhere have no known local record, so the sidebar can show only sessions on this computer.
- Running two servers side by side under different accounts.

**Risks:**
- A silent host means anyone signed in to that Claude account can open a session on this computer, so account security becomes computer security. The sidebar shows when a host is running, so it is never truly invisible.
- A work session opened this way goes through whichever account hosts it. Work folders stay out of the silent host unless Alex adds them. **(my call)**

## Other agent tools and local models

**Wanted (Alex):** support for other agent tools (harnesses), maybe local models too, and sessions that can message each other.

**How it would work (my call):**
- An agent tool is one small definition file: its start command, how to resume a chat, its permission flags, and any signals it gives. Adding a tool shouldn't need code changes. Launch presets use the same list.
- Local models need no special support in the app. A preset starts an agent tool that is pointed at a local model server such as Ollama. Which tools work well this way is not tested.
- Every tool works as a plain terminal session from day one. Extra features turn on per tool, depending on what its signals allow.
- Typing `claude` (or any known tool) in a plain shell tab turns it into an agent session, with no wrapper. **(Alex, from Herdr)** The host already reads each terminal's foreground program for titles, so it matches that against the tool list. **(my call)** The app's own Claude hooks are added through the settings folder it starts Claude with, so status is exact for Claude even when typed by hand. **(my call)**
- With no signals, status is a guess from the screen: a known prompt pattern means blocked, quiet output means idle. A guess never shows as a confident blocked or done, and nothing is ever approved because a session looks idle. **(my call, from Herdr's warning)**

**Agents driving the app (Alex, from Herdr):**
- A small command line, `ta` (name open), that talks to the host: list sessions and their states, open a tab or split, start an agent in it, send a prompt, read its screen, and `--wait` until it is done or blocked. The send, read and wait commands are the ones messaging uses below.
- Example: a session splits a pane, starts a reviewer, sends it the diff, and waits for it, with no sleep loop.
- It works from inside any session, and the host knows which session called, so the `From session <name>:` rule and the work-session guard below apply to it too.

| Feature | Any tool | Needs signals from the tool |
|---|---|---|
| Tabs, sidebar, restore windows, phone access | Yes | |
| Messaging between sessions | Yes | |
| Alert when stopped and waiting | Rough guess from quiet output | Exact |
| Smart move to a project | | Yes |
| Reopen a chat, move session | | Yes, needs a resume command |
| Automatic account switching, Remote Control | | Claude only for now |

**Messaging between sessions (my call):**
- It uses the same send, read and wait commands as "Claude across devices", so it reaches sessions on other computers too. Because the host owns every terminal, any tool that can run a command can use them.
- A message arrives as typed text starting with `From session <name>:`, so the receiver can tell it isn't Alex. The host adds that name itself, so a session can't pretend to be another one.
- A message waits until the receiving session is idle, so it doesn't land in the middle of a turn. A tool with no signals counts as idle after its output goes quiet, which is a guess and could still land mid-turn.

**Risks:**
- A message isn't Alex's approval, but a receiving agent in bypass mode may act on it anyway. A weak local model, or one tricked by a web page, could steer a stronger session that has more access.
- Guard: work sessions refuse messages from other sessions unless Alex turns that on, and each message is logged in the sidebar. **(my call)**
- Work code sent to a local model stays on the computer. Work code sent to another company's tool leaves it, so presets for those tools stay out of work projects unless Alex adds them. **(my call)**

## Claude accounts

Each account has its own Claude settings folder, and the app starts Claude in that tab with `CLAUDE_CONFIG_DIR` pointing at it. Anthropic's docs describe this for running several accounts side by side. The account folders link back to the main `~/.claude` so accounts share settings, memory and chat history, as Orca does. Logins are never synced, so each computer signs in once per account.

Launch presets can include an account. Each project can have a default account. **(my call)**

### Automatic switching

Orca appears to have only manual switching, from a text search of its code, not a full read.

**Signals:**
- The status line JSON gives `rate_limits.five_hour` and `rate_limits.seven_day`, each with `used_percentage` and `resets_at`. It appears only for subscribers and only after the session's first reply.
- The `StopFailure` hook with matcher `rate_limit` reports a turn that died. The docs don't define that as the usage limit specifically, so confirm against the percent before switching.
- An account with no running session reports nothing. The app remembers its last numbers, and a passed reset time counts as an empty window. **(my call)**

**Trigger:** at about 95% of either window, switch when the current turn ends. If a turn dies first, switch right away and tell Claude to continue. **(my call)**

**Default flow, in order (Alex chose the three sort keys, the details are my call):**
1. Same kind as the project first: work accounts for work projects, personal accounts for everything else.
2. Most room left, where room is the lesser of the 5 hour and 7 day remaining.
3. On a tie, the bigger plan.
4. The other kind of account comes last. Alex said switching can touch those accounts, so crossing is allowed by default.
5. If every account is full, wait for the earliest reset, then continue and notify Alex.

A setting, "work sessions stay on work accounts", blocks crossing for work projects. **(my call)** Plan type and work or personal are set once per account, unless the plan can be read automatically.

**Switching:** close Claude, reopen the same chat with `--resume` under the new account, and write one line to the session's log. The sidebar shows the current account only when more than one exists.

**Costs and risks:**
- Commands running inside that Claude session die.
- The first message after a switch likely uses more usage, because nothing is cached on the new account. Not tested.
- Reopening a chat under another account is untested. This is the first thing to test and needs two signed-in accounts. If it fails, try copying the chat file into the new account's folder. If that fails too, start a new chat on the new account with a summary of the old one, and tell Alex that switching no longer keeps the full chat. Accounts and automatic switching stay in the plan either way. **(my call)**
- Anthropic's consumer terms (effective Oct 8, 2025) don't mention multiple accounts or usage limits directly, but restrict "automated or non-human means" except through an API key or where explicitly permitted, and "bypassing any of our systems or protective measures". Told to Alex, who chose to proceed.
- If a work session crosses to a personal account, work code and chats go through that personal account.
- Off by default in the public version. **(my call)**

## Alerts

This replaces the separate alert app idea, in the same style as the rest of this app. **(Alex)** No separate app gets built. `~/projects/alert-app` stays as the source of the mockups, sounds and kitten media. **(my call)**

The goal, in Alex's words: "optimalness would be i always respond when something is halted for my input".

An alert built in can do more than a separate app, because the host owns the terminals and already receives Claude's hook signals (`Notification` and `Stop`). It can jump to the right tab, send the answer into the session, and reach whichever device Alex is on, including for a session running on the PC. **(my call)**

| Kind | When | In the alert | Reminder |
|---|---|---|---|
| Quick answer | Claude needs a yes or no | Allow, Deny, Open | Again at 3 minutes |
| Review | A plan or merge waits on Alex | Open, remind me later | Once at 30 minutes |
| Done | A session is ready to close | Open | Once at 10 minutes |

Alex accepted the reminder times. The three kinds were Claude's guess, and Allow and Deny in the alert came from Alex's round 1 answers.

**Look:**
- Soft premium is the default theme. The other three from the review page stay as themes in settings. **(Alex)**
- The alert's shape is a choice in settings. **(Alex)** The default is a line in the sidebar plus a small popup, since Alex left that question open. **(my call)** The final look goes through a `ui-review-page` review, starting from `mockups/round-2/alert-popups-r2.html`.
- A kitten buddy that can wave its paw, offered as its own theme. **(Alex)**
- Each kind has its own sound, aimed at satisfying rather than just polite. **(Alex)**

**Rules:**
- Quiet during the Monday stream, 4:30 to 6:30 PM ET, because OBS could pick up sounds. **(Alex)** Quiet means no sounds and no popups, while the sidebar still updates. **(my call)** In the public version this is a general quiet hours setting, empty by default. **(my call)**
- What an alert shows stays on Alex's own devices and never goes in the repo. **(Alex)**
- Alex's kitten media stays out of the public repo. **(Alex)** A cute image found on Google has the same limit unless its license allows sharing, so the public version needs art it has the rights to. **(my call)**

**Limits:**
- Alerts answer and jump only for sessions inside this app. A global Claude hook could also report sessions running in kitty, as a "go look" alert. That would cover the gap until this app replaces kitty day to day. Not tested. **(my call)**
- `Notification` and `Stop` may not be enough to tell a review from a quick answer. Test this before promising three kinds.
- On the phone, an alert shows only while the page is open. Alerting a locked phone isn't designed yet.

## Launch presets

Per-agent defaults for permission mode (manual, auto, bypass), extra arguments and environment, with easy presets such as bypass always on. **(Alex)** Orca's version is the reference: `src/shared/agent-launch/permissions/tui-agent-permissions.ts`.

## Features carried over from Orca

Alex likes Orca's features **(Alex)**, so all stay in, out of sight until asked for. Alex hasn't reviewed this list yet. **(my call)**

- **Image viewer:** lift Orca's viewer, about 12 files. **(Alex)**
- **Restore windows on startup:** comes from the host keeping terminals alive, plus a saved layout. After a reboot, chats reopen with resume. **(Alex)**
- **Chat view:** Alex asked for this. Start by rendering Claude's transcript files beside the terminal, not Orca's 85k line version. **(my call on the approach)** Images Claude looks at are saved in those files, so they appear here like they do in Remote Control. The terminal itself gets no image support, because Alex worried that could be bloat. **(my call)**
- **Browser control:** Alex asked for the same abilities as Orca. Build much smaller than Orca's 73k lines. **(my call on the size)**
- **Computer control:** Alex asked for the same as Orca. Needs native helpers and OS permissions. **(Alex)**
- **Session search, send/read/wait commands, keep awake while agents run, quick commands.** **(my call)**
- **Worktrees:** only by explicit request. **(Alex)**

## Plugins

A small plugin system can add a side panel, notifications and access to the built-in browser. **(my call on the shape)**

Uber Eats is the first plugin. **(Alex)**
- Sign in once in the built-in browser, then search and order from a side panel.
- The app watches order status and keeps alerting until Alex responds, using the same alerts as sessions.
- Placing an order always stops at the full total with fees and waits for a yes. **(my call)**
- It drives the website, so a redesign can break it, and Uber's terms may not allow automation. Off by default in the public version. **(my call)**

## Settings sync

Keeping Claude settings and knowledge in step across devices is a separate project at `~/projects/claude-sync`. This app keeps its own project list in step (see the sidebar section). Chat history for session moves isn't covered by claude-sync yet, so the move feature carries the chat itself.

## Open source and licensing

- License: MIT. **(my call)**
- Orca is MIT: keep its notice when copying code, don't use its name or logos, and check bundled licenses in the files copied.
- Don't copy from kitty (GPL-3.0), cmux (GPL-3.0-or-later, some BUSL-1.1) or AGPL projects.

## Build order

**(my call)**

1. Prove the risky parts: reopening a chat under another account, resuming under a different path, reading a list of Remote Control sessions, and two Remote Control servers under different accounts.
2. Host process on Mac and Windows, plus the Mac client with tabs and restore. Measure against the speed targets with `tools/keylat` as soon as typing works through the host, then finish with the speed pass. Steps 1 and 2 are stage 1. The Windows client follows.
3. Sidebar and projects with smart move, the four session states with blocked rollup, and noticing `claude` typed in a shell, after a UI review.
4. Accounts with automatic switching, since Alex ranks this highest.
5. Tailscale attach from the PC and phone, then move session and cross-device Claude, then the Remote Control sidebar and silent host.
6. Alerts, after a UI review, then presets, other agent tools with messaging between sessions and the command line agents drive the app with, image viewer and search.
7. Plugin system and Uber Eats, then chat view, browser and computer control.

## Left to the builder

File layout, the wire protocol between host and clients, the storage format, and how the Tauri windows are wired. Follow the speed targets and don't add a dependency that hurts startup or typing.
