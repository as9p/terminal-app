# Terminal app design

Working name only. Written 2026-11-08. Status: Alex approved on 2026-11-08, and stage 1 is being built (see `PLAN.md`). Later stages can still change.

Updated 2026-11-09 from Alex's review of Orca's settings (`orca-settings-notes.md`), a study of cmux and a survey of other terminals. Those changes are listed under "What changed on 2026-11-09" at the end and wait for Alex's approval.

Labels: **(Alex)** means Alex decided it. **(my call)** means Claude decided it and Alex can overrule it. Unlabeled lines are facts found by testing or reading code.

## Why this exists

Alex runs many Claude Code sessions and likes Orca's idea of grouping them, but Orca feels bloated. Sessions must attach to a project, everything lands in one "projects" workspace, it creates worktrees Alex never asked for, and its sidebar and right-click menus are cluttered. kitty is Alex's favorite terminal but has none of these session features.

This app should look like Alex's kitty config, feel like cmux, and do the useful things Orca does, with the extra features out of sight until asked for. Alex used cmux on an older laptop and liked it a lot: "calm super easy to nav". **(Alex, 2026-11-09)** Alex uses it alone for now and would open source it. It may grow into Alex's own terminal or harness one day.

## Goals

- Feels like kitty in look, keys and speed, and is as optimized as possible. **(Alex)**
- Calm and easy to navigate, like cmux. By default the window shows the sidebar and the terminal and nothing else. **(Alex)**
- Typing focus is always in the terminal. Alex never clicks back into it. **(Alex)**
- New tab or new Claude opens instantly as a default session in the sidebar, with no project needed. A plain terminal is never locked to a project. **(Alex)**
- A folder becomes a group in one action, and git is picked up from the folder with nothing to connect. **(Alex)**
- Sessions group into projects, and the same projects appear on every computer. **(Alex)**
- A session moves itself to the project it starts working in. **(Alex)**
- Works from a Mac laptop, a Windows PC and a phone over Tailscale. **(Alex)**
- Multiple Claude accounts with automatic switching at the usage limit. Alex calls this of utmost importance. **(Alex)**
- Alex always hears about it when a session is stopped and waiting on them, and never hears a sound for a helper agent finishing. **(Alex)**
- One session per group can be the master agent, which drives the others and cleans up after them. **(Alex)**
- Runs other agent tools and local models, not just Claude, and any session can message any other. Alex said "maybe" on local models. **(Alex)**

## Non-goals for now

- Company servers, accounts or a relay of this app's own. Everything the app itself runs stays on Alex's tailnet. **(Alex)** Claude's own Remote Control is the one exception, because it goes through Anthropic's service and Alex asked for it (see "Remote Control").
- Cloud computers, Jira, Linear, GitLab, voice dictation and telemetry. **(my call)**
- A chat view, a task board, clock-based automations, per-project settings pages, a floating or pop-out terminal, a phone emulator, browser extensions, Orca-style artifact publishing and a pet. **(Alex said no to most of these in the settings review, the rest are my call)**
- A built-in remote desktop. Alex has RustDesk for that. **(Alex)**
- Custom account switching flows. Alex called this rambling and only wants the default flow. **(Alex)**

## Anti-goals: it failed if

- It ever creates a worktree without being asked. **(Alex)**
- A floating icon can't be removed, or `[primary]` or `main` labels appear in the sidebar. **(Alex)**
- Rename takes more than one simple action. **(Alex)**
- Seeing your projects and sessions needs a pop-out panel the way Orca's workspaces do. Alex calls Orca's version bad. **(Alex)**
- It feels like an Electron app that happens to contain a terminal.
- Alex has to click the terminal to type again after using the sidebar, a menu or a rename. **(Alex)**
- A title bar, window buttons, a top tab bar, a bottom status bar or a right-hand panel shows up. **(Alex)**
- A helper agent finishing plays the sound or lights the marker. **(Alex)**
- A spinner or any other constant motion runs in the sidebar. **(Alex)**
- Another device resizes a terminal Alex is typing in. **(Alex)**
- A keyboard shortcut puts up a popup, toast or overlay, the way Orca shows one for zoom. Alex finds these very annoying. A shortcut just does its job. **(Alex)**

## Devices and where things run

- There is no home server. The Windows PC is the main host. **(Alex)**
- New tab opens on whichever device you're on. **(Alex)**
- On the phone, a new tab is a real tab on the PC, in the same sidebar. The phone is a screen into the PC's terminal. **(Alex)**
- The phone matters less than it did. Claude's Remote Control covers everyday phone use, and Alex wants the phone mainly to fix things when something breaks (see "Phone"). **(Alex, 2026-11-09)**
- Running work on the PC may die if the PC drops. The laptop can take over manually by reopening the chat. **(Alex)**
- brawlfarm, bh, Minecraft and Android work stay tied to the PC. **(Alex)**
- A Mac-only tab is an ordinary private terminal. Other devices can't see or reach it. **(Alex)** Normal Mac tabs are visible to other devices while the Mac is awake. Mac-only is opt-in. **(my call)**

### Staying up and fixing it from away

Alex's PC crashes now and then, and Orca once closed on it while Alex was out, with no way to bring it back from the laptop. **(Alex)**

- A closed or crashed window never ends sessions, because the host owns them.
- The host is a background service that starts with the computer and restarts itself after a crash. **(my call)**
- On Windows, starting Alex's shells from a service before anyone has logged in may not be possible without storing a password. If it isn't, the host starts at login and Windows automatic login covers the gap. Test this before promising it.
- When a host is down, its computer shows as down in the sidebar on the other computers, with a restart action. The restart goes over SSH across Tailscale, so the SSH server has to be turned on once on that computer. **(my call)**
- An agent on the PC can operate PC apps while Alex drives it from the Mac (see "Computer control"). It works only while the PC is on, awake and logged in. Alex accepted that limit. **(Alex)**
- The whole desktop with Alex's own mouse and keyboard is RustDesk's job. It is not built in. **(Alex)**
- Outside this app, told to Alex on 2026-11-09: set the BIOS to power on after a power loss, keep Windows' automatic restart after a crash on, set Tailscale to run unattended, install RustDesk as a service, and put the PC on a smart plug for hard freezes.

### Phone

- Everyday phone use is Claude's Remote Control, which already exists. Alex said the phone app "doesn't matter" much with it. **(Alex)**
- Alex still wants to fix things from the phone when something breaks. That path must not depend on this app, because the broken thing may be this app's host. It is plain SSH from an existing phone SSH app to the PC over Tailscale, and nothing is built for it. **(my call)**
- This app's own phone client comes last in the build order. It is a home screen web app served by the host: the sessions, a plain shell and the restart action. It needs no Apple developer account, which Alex doesn't have, and it never expires. **(my call. Alex didn't want "a site"; a home screen web app opens like an app, with no browser bars)**
- Pairing is whatever is easiest, since the phone is already on the tailnet. It lives in settings, with no phone button in the window. Terminals keep running when Alex leaves the phone app. **(Alex)**

## Architecture

**(my call)** unless marked.

- Each computer runs a small host process written in Rust. It owns the terminals, keeps them alive when the window closes, and keeps a copy of each screen so a client can reconnect. A Rust renderer is built only if xterm.js misses the speed targets below, and the host stays the same either way.
- A terminal has one size, set by the device Alex last typed in. Other devices showing the same terminal scale or letterbox it and never resize it. Alex hit random resizes on a remote host in Orca. **(my call on the rule)**
- Clients attach to any host over Tailscale. Only devices on Alex's tailnet can connect. **(Alex)**
- Mac and Windows clients are Tauri apps with xterm.js using WebGL. The phone is a web page using the same terminal code. Changed on 2026-11-11: the desktop clients move to a native Rust engine, see "What changed on 2026-11-11".
- The host runs on Mac and Windows from the first build, because the PC is the main host. Any terminal-process layer must handle both Unix terminals and Windows ConPTY. The Mac client comes first and the Windows client follows once the host and tabs work.
- Hosts accept connections only over Tailscale, and each new device pairs once to get its own token, so being on the tailnet isn't enough by itself.
- The host serves the phone page over HTTPS using a Tailscale certificate. The phone's Face ID lock is a passkey, which requires that HTTPS. Both are built together with the phone web app, last in the build order, so the first host has no HTTPS listener. **(my call, 2026-11-09)**
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

Test build memory was about 500 MB from a debug build with one tab. Orca used 2.2 to 2.6 GB with all of Alex's sessions open. The test build has no sidebar or host process, so the real app will lose some speed. For the real app see the 2026-11-10 results below.

Typing latency, 60 presses of a key in each terminal running `cat`, measured from key press to the first changed screen frame at 120 fps on Alex's Mac:

| Terminal | Fastest | Median | 95th percentile | Slowest | Backspace median |
|---|---|---|---|---|---|
| macOS Terminal | 9.5 ms | 15.8 ms | 22.3 ms | 28.8 ms | 16.1 ms |
| Orca, visible tab | 9.1 ms | 17.8 ms | 26.0 ms | 31.7 ms | 17.2 ms |
| kitty, Alex's config | 11.0 ms | 19.0 ms | 24.9 ms | 80.9 ms | 16.7 ms |
| Test build, xterm.js | 12.5 ms | 21.4 ms | 34.6 ms | 41.5 ms | 22.9 ms |

Terminal, Orca and kitty are effectively tied: the median gap is 3.2 ms and one screen frame is about 8 ms. In this test xterm.js in Electron does not lose to native terminals on typing.

The test build passes the latency target on its typing median, 2.4 ms behind kitty, so the native Mac client path is not triggered. This held for the test build only; for the real app see the 2026-11-10 results below. Both were watched across the whole window, so kitty's 19.0 ms is the fair comparison, not its 17.4 ms spot-only rerun. Two weaker results remain:

- Backspace median is 6.2 ms behind kitty, and the 95th percentile is 9.7 ms behind. Both gaps are about one screen frame or less.
- 11 of 60 presses took over 30 ms, against 1 for kitty and Orca and 0 for Terminal. The cause isn't known.

The test build is a debug build that sends every key through a Node server and a WebSocket, which the Rust host would replace. Measure the real app again as a release build. That was done on 2026-11-10; see the results below.

Limits of this test:

- Each terminal had one run, so small gaps may not repeat.

- It's a homemade tool using macOS screen capture, not cross-checked against a camera or another tool. Trust the comparison more than the absolute numbers.
- It measures `cat` echoing a key, not typing into Claude Code, which draws its own input box.
- Orca ran at its normal 1728x1084 with its sessions open, the test build ran at 1728x1084, and kitty and Terminal ran at about 1200x800.
- ghostty-web has no typing number, because the tool's calibration fails on it. Calibration also fails on xterm.js, but its run doesn't need calibration because its screen doesn't change when idle.
- Orca's window changes about 13 times per second on its own, so its test watched only the 18x36 pixel spot where the character appears. kitty and Terminal were watched across the whole window. Rerunning kitty with the spot-only method gave a median of 17.4 ms against 19.0 ms, so the method difference is small, and that kitty rerun is level with Orca's 17.8 ms.

#### Real app through the host (2026-11-10)

First measurements of the real app, going through the Rust host process. All on Alex's MacBook Pro, release build, with `tools/keylat` and `tools/speed`. kitty was measured in the same sitting each time, because kitty's own numbers moved between days. Raw files are under `tools/keylat/results/task-7`, `task-7b` and `task-7b-dom`.

Three sittings:

1. Before the output fix, with WebGL.
2. After the fix, with WebGL.
3. After the fix, with the xterm.js DOM renderer.

Output speed, 30 MB burst, three runs each, in MB/s:

| Sitting | Terminal | Run 1 | Run 2 | Run 3 |
|---|---|---|---|---|
| 1, before the fix, WebGL | App | 28.0 | 35.8 | 36.8 |
| 1, before the fix, WebGL | kitty | 75.9 | 78.1 | 81.6 |
| 2, after the fix, WebGL | App | 78.1 | 71.5 | 77.6 |
| 2, after the fix, WebGL | kitty | 79.5 | 81.8 | 84.8 |
| 3, after the fix, DOM | App | 91.0 | 89.5 | 88.8 |
| 3, after the fix, DOM | kitty | 81.9 | 79.2 | 73.3 |

Typing, with the same method as the test-build table above:

| Sitting | Terminal | Median | 95th percentile | Over 30 ms | Backspace median |
|---|---|---|---|---|---|
| 1, before the fix, WebGL | App | 20.8 ms | 31.9 ms | 11% | 20.8 ms |
| 1, before the fix, WebGL | kitty | 12.8 ms | 20.8 ms | 2% | 12.4 ms |
| 2, after the fix, WebGL | App | 23.7 ms | 36.8 ms | 24% | 24.3 ms |
| 2, after the fix, WebGL | kitty | 14.3 ms | 23.0 ms | 2% | 14.4 ms |
| 3, after the fix, DOM | App | 25.4 ms | 33.9 ms | 26% | 25.6 ms |
| 3, after the fix, DOM | kitty | 14.2 ms | 22.3 ms | 2% | 14.1 ms |

What it means:

- The output target (at least 40 MB/s) is met after the fix. Before it, the app got 28 to 37 MB/s.
- The cause of the first miss: macOS hands the host about 1,000 bytes per terminal read, and each read went out as its own WebSocket message. That was about 31,000 messages per burst, and the web view pays a cost per message.
- The fix: when a read comes back full, the host keeps reading for up to 0.1 ms and sends up to 64 KiB as one message. A short read, such as a key echo, is sent at once. Messages per burst fell to about 1,000.
- The typing target (median within 5 ms of kitty) is missed, by 8.0 ms, 9.4 ms and 11.2 ms in the three sittings.
- The host is not the cause. A key's round trip through the host measures about 0.2 ms headless.
- The renderer is not the cause. WebGL and DOM both miss.
- The app's 20.8 ms in sitting 1 equals the earlier no-host test build (21.4 ms). kitty got faster between test days: 19.0 ms then, 12.8 to 14.3 ms now. That is why the earlier "2.4 ms behind kitty" no longer holds.
- The gap is about one screen frame at 120 Hz (8.3 ms), and it sits in the Mac web view's own drawing pipeline. This is inferred from the shape of the numbers, not measured directly.
- Per the rule below, a typing miss opens the question of replacing the terminal drawing on the Mac, or building a native Mac client. That decision is open and is Alex's. Sidebar work continues in the web client meanwhile, because the web client is also the Windows client. **(my call)**

Known limits of these runs:

- `keylat` resizes kitty to 1200x800 but leaves the app at its own size.
- The app's share of slow presses (11% to 26%) is much higher than kitty's 2%. The cause isn't known yet.

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

### cmux

cmux (cmux.com, github.com/manaflow-ai/cmux) is a macOS-only terminal built on Ghostty's engine, with a vertical tab sidebar for agents. It is the reference for how this app feels. **(Alex)** From its docs, repo and site on 2026-11-09, not run here:

- The window is a sidebar and a terminal. There is no bottom bar, and panes sit in a tight grid with 1 px lines.
- A sidebar row shows a title, the last notification text, and the git branch and folder. It can also show pull request state and listening ports. A compact mode shows one status dot and the title.
- Branch and folder come from shell hooks that run at each prompt, so they update after `cd`.
- "Needs you" is a ring on the pane, a badge on the row and a count in the title bar. One key jumps to the latest unread. Looking at a pane marks it read, and no banner shows for the pane in front.
- Agent events are tagged turn complete, needs permission or idle reminder, and helper agent notifications are off by default.
- Rename is one key, and a name set by hand always wins over automatic titles.
- The browser is a split pane that agents drive from the command line, with cookie import from Chrome, Firefox, Arc and others.
- A command line over a socket lists, opens, splits, sends keys, reads the screen and notifies.
- It reads the user's Ghostty config for fonts and colors.
- Its most upvoted issue says the running and needs-input markers are flaky. That is why a guessed state never shows as a confident one here.
- Its selected row and its needs-you badge are the same blue. This app keeps the two visibly different. **(my call)**
- Not found in its docs: whether a sidebar click returns focus to the terminal, whether rename is inline, and where a default name comes from.
- Four screenshots of its site are in `~/projects/terminal-app-refs/cmux-site/`, outside the repo, for the UI review. They are cmux's images and never go in a public repo.

### Other terminals checked

A web survey on 2026-11-09 looked for an existing terminal that covers Alex's list. From each project's own pages, not tested here:

| Terminal | Feels like a terminal | Windows as the main host | Mac sees PC sessions with no company servers | Phone | Biggest miss |
|---|---|---|---|---|---|
| Herdr | Partly, it runs inside another terminal | Yes, maturity not confirmed | Yes | Yes | No browser, viewer, account switching or master agent |
| cmux | Yes | No, macOS only | Partly | Partly, needs a cmux account | No Windows |
| Orca | Partly | Yes | Yes | Yes | The clutter this app exists to remove |
| Zellij | Partly | Yes | Partly, through its own web server | No | No agent states |
| ccmux | No | Only through WSL2 | Yes | Yes | Early alpha |

Nothing found covers the whole list. Herdr is the closest and runs inside kitty, so it is a cheap way to try the session features before they exist here. Ghostty, WezTerm, Wave and other plain terminals were not checked.

## Window, sidebar and groups

The look is Alex's kitty config. The feel is cmux (see "cmux" above). Everything in this section gets its exact look from one `ui-review-page` review, with real screenshots of cmux, kitty and Orca, before any UI code.

### What is on screen

- By default the window is the sidebar and the terminal, edge to edge. Everything else stays closed until Alex opens it with a key. **(Alex)**
- No title bar and no window buttons, matching `hide_window_decorations titlebar-only` in Alex's kitty config. **(Alex)** Close, minimize and full screen stay as keys and menu items, and dragging an empty part of the sidebar moves the window. **(my call)**
- On Windows a window without a title bar can lose edge resizing and snapping. The PC build must show that both still work, and the title bar is a per-computer setting. **(my call)**
- No top tab bar, no title strip on panes, no bottom status bar and no right-hand panel. **(Alex)**
- The sidebar has no search box, no back and forward buttons, no Tasks, Automations, Mobile or Artifacts buttons, no "Projects" title and no button row. Alex: "we use keyboard commands around here". **(Alex)**
- The sidebar has a faint edge line between it and the terminal and a faint hairline above each group. The Mac sidebar theme is the default and the Terminal theme is the other. **(Alex)**
- Every project group and session sits in the always-visible sidebar, with no pop-out workspace panel. **(Alex)**

### Tabs

- Vertical tabs only. The sidebar is the tab list. **(Alex)**
- Cmd+T opens a plain shell tab at once. It starts in the folder the current tab is in right now and joins that folder's group. If that folder is no group, it joins the current tab's group. With no group at all it starts in the home folder, ungrouped. **(Alex asked for Cmd+T and no lock-in, the folder rule is my call)**
- A new tab can open on another paired computer, so Alex can start a PC terminal from the Mac. **(Alex)** Cmd+T follows the tab Alex is on, so inside a PC group it opens on the PC. A second key, Cmd+Shift+N, opens the new tab on the other computer, in the same project's folder there if that computer has it, otherwise the home folder. With more than two computers it asks which one. `ta new --on <computer>` does the same. **(my call)**
- The sidebar scrolls as one plain list. A group's name scrolls away with its rows and never sticks to the top, which Orca's does. **(Alex)**
- A row is compact by default: the state marker and the name. Two-line rows exist behind one setting, `sidebar_rows`. **(Alex)** A group's header shows its folder, git branch, a session count and, when collapsed, a rolled-up blocked marker. **(Alex)** Longer text, such as what the agent last said, shows only on a session that needs Alex. **(my call, from "far less on screen")** cmux's taller three-line rows are the comparison in the review.
- Rows have no icons unless Alex turns them on, and no `[primary]` or `main` labels. **(Alex)**
- Rows keep the order Alex drags them into. There are no Recent, Repo, Pinned or Active views. New groups start sorted by folder. **(Alex)**
- A group has no arrow. The folder icon in the marker column is the open and closed control. **(Alex)**
- With more than one computer paired, a borderless picker with a menu in the sidebar filters by device: all, Mac or PC. **(Alex)** It is hidden with one computer. **(my call)**
- Helper agents and agent team members are child rows under their session, collapsed by default. A filter shows or hides them. **(Alex)**
- Showing each session's and helper's model in its row is a later add-on, off by default. **(Alex called it an add-on)**
- Rename is inline: one key or a double click, type, Enter. A name Alex set is never overwritten by the terminal title or anything automatic. **(Alex, the never-overwritten rule is from cmux)**
- The right-click menu on a row is short: rename, pin, close, move to group. **(Alex)** Close shows Cmd+Shift+W, matching the keys table. **(my call)**

### Splits

- A split is made the kitty way, or by dragging one existing tab onto another to join them. A pane can be pulled back out into its own tab. **(Alex asked to combine terminals that already exist)**
- Focus follows the mouse between panes, as on Linux. It is a setting, on by default. **(Alex)**
- With no title strip, the pane Alex is typing in stays at full brightness and the others do not dim by default. Dimming is a setting, off by default. **(Alex)** The sidebar row lists its panes beneath it. **(my call)**
- There is no floating or pop-out terminal. **(Alex)**

### Focus

- Typing focus returns to the terminal after every sidebar click, tab switch, drag, menu and rename. Only a text field Alex opened, such as a rename, holds focus, and Enter or Esc gives it back. **(Alex asked for the behavior, the rule is my call)**

### Groups

- A session starts ungrouped, with no project. **(Alex)**
- A folder becomes a group in one action: drag it onto the sidebar, press one key and pick it, or run `ta open .` inside it. A folder with no git is still a group. **(Alex)**
- Git is detected, never connected. The app finds the git root, branch and remote from the folder by itself. **(Alex)**
- A shell tab belongs to the group of the folder it is in. `cd` into a git repo moves the tab to that repo's group, creating the group if needed. `cd` out to a folder that is no group leaves the tab where it is. **(Alex asked that everything in the same folder groups together, the rule is my call)**
- The folder comes from a shell hook that runs at each prompt, as cmux does it. The host loads the hook when it starts the shell, without editing Alex's profile files. zsh, bash and PowerShell are covered. A shell with no hook (cmd, WSL, a shell over SSH) still works as a tab, but it never moves by itself and `claude` gets no added flags there. Loading the hook in PowerShell on the PC is tested in stage 1. **(my call)**
- A group goes away when its last tab is closed or moved out, however it was made. **(Alex)** Its few saved defaults are kept, so opening the folder again brings the group back as it was. **(my call)**
- Dragging a tab by hand pins it, and it stops moving by itself. **(my call)**
- The `cd` rule applies while a shell sits at its prompt. While an agent runs in a tab that is still ungrouped, the tab moves on the agent's first edit instead. A tab that is already in a group never moves because of an edit. Claude Code `PreToolUse` and `PostToolUse` hooks report `Edit` and `Write` calls with `tool_input.file_path`, and the app resolves the file to its git root. It moves on edits, not on reads. **(my call on the rule, Alex asked for the behavior)**
- A project is identified by its git remote, or its folder name if it has none, so the same project matches across computers. **(my call)** On one computer a folder group's key is the folder's whole path, so two folders with the same name stay two groups; matching across computers is not built yet, and when it is it compares the last part of that path. **(my call, 2026-11-10)**
- A group is the project, not the project on one computer. Sessions from every computer sit in the same group, and the device filter narrows it. The order Alex drags things into is one shared order, synced like the project list. **(my call)**
- Every host keeps a full copy of the project list and session groupings and exchanges changes with other hosts when they're both online. If two hosts changed the same item, the newest change wins. There is no central store, so the Mac still shows projects when the PC is off. **(my call)**
- Sessions on an offline computer stay in the sidebar, greyed, with when they were last seen. Claude chats on them offer a manual reopen here, as long as the chat history is available on this computer. Plain terminals only show as offline. **(my call)**
- A group has no settings page. Its few defaults, such as its account and quick commands, are edited from its right-click menu and stored with the project list. **(Alex called per-project settings pages a waste of space, the storage is my call)**
- GitHub: a group's header can show the pull request number and state for its branch, and that is all the GitHub integration does. **(my call)** Alex has more than one GitHub account, so the app holds a main account plus extra ones and uses the one that matches the folder, the way Alex's git config does. **(Alex)** It never runs `gh auth switch`.

### Session states

- Every agent session shows one of four built states: working, blocked (waiting on Alex), done (finished its turn, not yet looked at) and idle. **(Alex, from Herdr)** Two further looks, handoff requested and failed, are recorded in `docs/ui-review.md` and built in a later plan. **(Alex for handoff, my call for failed)** It is one small marker, not text. The working marker is a drawn snowflake that ticks, a 15 degree nudge every second. The other markers hold still, and a marker reshapes into the next one when the state changes. **(Alex)** The marker stays visible however narrow the sidebar gets. **(Alex)**
- Blocked rolls up: a blocked session marks its row, its pane and its group, even when the group is collapsed, and one key jumps to the blocked session. With several blocked, the key goes through them oldest first. **(Alex asked for Herdr's rollup, the order is my call)**
- Done clears when Alex looks at the session. **(my call)**
- Blocked and done both mean "your move". They follow the rule in "Alerts", so a helper agent finishing never sets them. **(Alex)**

### Board

- One key swaps the terminal area for a board of every session across groups and computers. It is a second view of the sidebar's sessions, not a pop-out, and nothing on it is set by hand. **(Alex wants a good version of Orca's workspace board)**
- Sessions sort themselves into four columns: needs you, done and waiting for a look, working, idle. **(Alex)**
- A small ask is answered on its card with one click: allow, deny, or send "go". A full review opens the session or its artifact. **(Alex)**
- Artifact to-do list: the Claude artifacts that are waiting on Alex's answer, each dropping off the list when Alex gives the go. **(Alex)**
- How the to-do list would work **(my call, not tested)**: the host reads each Claude session's transcript for artifacts it published. An artifact counts as waiting while its session is stopped after publishing it, and drops off when that session gets Alex's next message or comment. It covers only sessions inside this app. How a publish and a comment appear in the transcript needs a test before this is promised.
- Artifacts open in a browser pane beside the terminal, signed in through cookie import. The full list is claude.ai's own gallery page in that pane, so no separate viewer is built. **(Alex asked for an artifact viewer like the Claude app, the approach is my call)**

## Claude across devices

- Claude runs on the device you're on and can see and message sessions on the others, edit files and run commands there, with no asking. **(Alex)**
- Claude reaches other computers through the app's own command line tool, which calls the host on that computer. It covers send, read and wait for sessions, plus run command and edit file. The host writes the log, so it can't be skipped. **(my call)**
- Each session keeps a visible log of what it changed on other computers. **(my call)**
- The phone gets a Face ID lock when the phone web app is built. **(my call)**
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
- A setting, "add my flags to `claude`", makes typing `claude`, alone or with other options, start with the flags from the Claude launch preset, so Alex never types them. **(Alex)** It is on for Alex and off by default in the public version. **(my call)**
- `claude master` does the same and opens the group's master session: it starts one with the master's preset, or jumps to the master the group already has. **(Alex asked for the alias, jumping to an existing master is my call)**
- The app's shell hook defines a small `claude` function that adds the preset's flags and handles the word `master`. Flags Alex types win over the preset's. Without the function, Claude Code would read `master` as a prompt, so the function has to be loaded in every shell the app starts, on the Mac and in PowerShell on the PC. **(my call)**
- With no signals, status is a guess from the screen: a known prompt pattern means blocked, quiet output means idle. A guess never shows as a confident blocked or done, and nothing is ever approved because a session looks idle. **(my call, from Herdr's warning)**

**Agents driving the app (Alex, from Herdr):**
- A small command line, `ta` (name open), that talks to the host: list sessions and their states, open a tab or split, start an agent in it, send a prompt, read its screen, and `--wait` until it is done or blocked. The send, read and wait commands are the ones messaging uses below.
- Example: a session splits a pane, starts a reviewer, sends it the diff, and waits for it, with no sleep loop.
- It works from inside any session, and the host knows which session called, so the `From session <name>:` rule and the work-session guard below apply to it too. The host puts a private token in each session's environment, and `ta` sends it over a local socket. **(my call)**

| Feature | Any tool | Needs signals from the tool |
|---|---|---|
| Tabs, sidebar, restore windows, phone access | Yes | |
| Messaging between sessions | Yes | |
| Alert when stopped and waiting | A dim "maybe waiting" marker from a guess, with no sound and no notification | Exact, with sound and notification |
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

## Master agent

Alex likes Orca's main agent that drives other sessions and wants the same thing here, "an auto control panel". The last thing Alex used it for was taking control of the Roblox automation work, so sessions don't get left behind during heavy multitasking. **(Alex)**

- Master is a role for one session per group. Any Claude tab can be made the master in one action, and typing `claude master` starts one. **(Alex asked for the feature, the shape is my call)**
- It works through the `ta` command line (see "Agents driving the app"): open tabs, start agents, send prompts, read screens and wait. An instruction file installed with the app teaches Claude those commands. Orca's `orca orchestration` commands are the reference. **(my call)**
- The master sits at the top of its group, and the group's other tabs hang indented under it. A group with no master is a flat list. **(Alex)**
- Workers report to the master. A worker finishing tells the master and does not alert Alex. A worker that needs an answer only Alex can give still shows blocked on its own row. **(my call)**
- Workers can't start their own workers unless a setting allows it. **(my call, Alex wasn't sure)**
- Claude's own agent teams show the same way: team members are child rows under the lead. **(Alex)**

**Cleanup (Alex):** a worker that is done, or has been inactive too long, has its process ended.

- A finished worker closes as soon as the master has read its result. Alex asked why Orca waits 2 minutes, and nothing here needs that wait. **(my call)**
- A worker with no activity for 30 minutes is closed. The limit is a setting. **(my call)**
- Only workers the master started are ever closed, never a tab Alex opened. **(my call)**
- A worker that is waiting on Alex, or has a command still running, is left alone. **(my call)**
- Closing ends the process only. The chat stays resumable, files are untouched, and Cmd+Shift+T brings the tab back. **(my call)**

## Claude accounts

Each account has its own Claude settings folder, and the app starts Claude in that tab with `CLAUDE_CONFIG_DIR` pointing at it. Anthropic's docs describe this for running several accounts side by side. The account folders link back to the main `~/.claude` so accounts share settings, memory and chat history, as Orca does. Logins are never synced, so each computer signs in once per account.

Launch presets can include an account. Each project can have a default account. **(my call)**

### Automatic switching

Orca appears to have only manual switching, from a text search of its code, not a full read.

**Signals:**
- The status line JSON gives `rate_limits.five_hour` and `rate_limits.seven_day`, each with `used_percentage` and `resets_at`. It appears only for subscribers and only after the session's first reply.
- The `StopFailure` hook with matcher `rate_limit` reports a turn that died. The docs don't define that as the usage limit specifically, so confirm against the percent before switching.
- An account with no running session reports nothing. The app remembers its last numbers, and a passed reset time counts as an empty window. **(my call)**

**Trigger:** at a percent Alex sets, 95 by default, of either window, switch when the current turn ends. **(Alex asked for a switch at a set percent)** If a turn dies first, switch right away and tell Claude to continue. **(my call)**

**Default flow, in order (Alex chose the three sort keys, the details are my call):**
1. Same kind as the project first: work accounts for work projects, personal accounts for everything else.
2. Most room left, where room is the lesser of the 5 hour and 7 day remaining.
3. On a tie, the bigger plan.
4. The other kind of account comes last. Alex said switching can touch those accounts, so crossing is allowed by default.
5. If every account is full, wait for the earliest reset, then continue and notify Alex.

A setting, "work sessions stay on work accounts", blocks crossing for work projects. **(my call)** Plan type and work or personal are set once per account, unless the plan can be read automatically. A project's kind comes from its folder: Alex names the root folders that hold work, and everything else is personal, the same way Alex's git config picks an account. **(my call)**

**Switching:** close Claude, reopen the same chat with `--resume` under the new account, and write one line to the session's log. The sidebar shows the current account only when more than one exists.

After any switch, by hand or automatic, every affected session comes back by itself and carries on. Alex never has to press Enter in each one. **(Alex)**

**Costs and risks:**
- Commands running inside that Claude session die.
- The first message after a switch likely uses more usage, because nothing is cached on the new account. Not tested.
- Reopening a chat under another account is untested. This is the first thing to test and needs two signed-in accounts. If it fails, try copying the chat file into the new account's folder. If that fails too, start a new chat on the new account with a summary of the old one, and tell Alex that switching no longer keeps the full chat. Accounts and automatic switching stay in the plan either way. **(my call)**
- Anthropic's consumer terms (effective Oct 8, 2025) don't mention multiple accounts or usage limits directly, but restrict "automated or non-human means" except through an API key or where explicitly permitted, and "bypassing any of our systems or protective measures". Told to Alex, who chose to proceed.
- If a work session crosses to a personal account, work code and chats go through that personal account.
- Off by default in the public version. **(my call)**

## Alerts

This replaces the separate alert app idea. **(Alex)** No separate app gets built. `~/projects/alert-app` stays as the source of the sounds. **(my call)**

The goal, in Alex's words: "optimalness would be i always respond when something is halted for my input".

**When Alex hears or sees one ("your move") (Alex):**
- A top-level session ended its turn and none of its helper agents are still running. That is done.
- Any session needs an answer from Alex. That is blocked.
- A helper agent finishing never alerts. Neither does an agent team member or a master's worker finishing, because those report to their lead.
- Alex wants the same rule for the bell in today's Claude Code hooks on the Mac and the PC. That is a small separate fix outside this app.

**How (my call):** Claude Code fires `Stop` when the main session ends a turn, `SubagentStop` when a helper ends, and `Notification` when it needs input. A background helper reporting back makes the main session take a short turn, which fires `Stop` again, so the host holds back "done" while any helper of that session is still running. Team members and workers are full sessions that fire their own `Stop`. The host knows which tabs are workers because it started them. Tested 2026-11-10 (`docs/risky-tests.md`, section 3): `Stop` carries `background_tasks`, and the host holds back "done" while any entry has type `subagent` and status `running`. A helper's events carry `agent_id`, the main session's never do. `Notification` alone is a late "blocked" trigger (about 6 s) and nothing clears it before `Stop`, so use `PermissionRequest` to set it and `PostToolUse` to clear it. How a team member's `Stop` is told from its lead's is not tested.

| Kind | When | One-click answers | Reminder |
|---|---|---|---|
| Quick answer | Claude needs a yes or no | Allow, Deny, Open | Again at 3 minutes |
| Review | A plan or merge waits on Alex | Open, remind me later | Once at 30 minutes |
| Done | A session is ready to close | Open | Once at 10 minutes |

Alex accepted the reminder times. The three kinds were Claude's guess, and Allow and Deny came from Alex's round 1 answers.

**Look (Alex, 2026-11-09):** simple, and it fits the terminal. Alex finds Orca's notifications annoying, and dropped the kitten buddy and the alert themes: "remove the cats", "make it fit".
- Inside the app: the marker on the row, the rollup on its group, and that state's sound. No popup inside the window. **(my call)**
- When the app isn't in front: one system notification that opens the session when clicked. None for the session Alex is looking at. **(my call, from cmux)**
- The one-click answers live on the board card and in the system notification. **(my call)**
- Each state can be set to sound, banner, dock badge or nothing, per state. **(Alex)** A sound is aimed at satisfying rather than just polite. **(Alex)**

**Rules:**
- Quiet during the Monday stream, 4:30 to 6:30 PM ET, because OBS could pick up sounds. **(Alex)** Quiet means no sounds and no notifications, while the sidebar still updates. **(my call)** In the public version this is a general quiet hours setting, empty by default. **(my call)** Quiet hours are set per computer, for the one that runs OBS, and the phone is not affected. **(my call)**
- What an alert shows stays on Alex's own devices and never goes in the repo. **(Alex)**

**Limits:**
- Alerts answer and jump only for sessions inside this app. A global Claude hook could also report sessions running in kitty, as a "go look" alert. That would cover the gap until this app replaces kitty day to day. Not tested. **(my call)**
- `Notification` and `Stop` may not be enough to tell a review from a quick answer. Test this before promising three kinds.
- On the phone, an alert shows only while the page is open. Alerting a locked phone isn't designed yet.

## Launch presets

Per-agent defaults for permission mode (manual, auto, bypass), extra arguments and environment, with easy presets such as bypass always on. **(Alex)** Orca's version is the reference: `src/shared/agent-launch/permissions/tui-agent-permissions.ts`.

Bypass is on by default for Claude, and a folder is trusted automatically when the app starts an agent in it. **(Alex)** Automatic trust applies only inside root folders Alex chooses, and both are off by default in the public version. **(my call)**

## Features carried over from Orca

Alex went through Orca's settings on 2026-11-09 (`orca-settings-notes.md`). What stays is below, out of sight until asked for. Anything in Orca that isn't in this document is left out.

- **Viewer pane:** when Claude wants Alex to look at an image, an audio or video file, or a markdown file, it opens in a pane beside the terminal, the way cmux opens its browser. `ta show <file>` opens it. It is view only, with no editor and no file tree. **(Alex)** The pane belongs to its tab, so it never follows Alex into another project, which was Alex's complaint about Orca's file panel. **(my call)** Lift Orca's image viewer, about 12 files.
- **Images Claude works with:** Alex would like to see the screenshots Claude is looking at, but only if it costs almost nothing. They are saved in Claude's transcript files, so the viewer pane can show the latest ones. Build it last and drop it if it grows. There is no chat view. **(Alex)**
- **Browser pane:** the same abilities for agents as Orca's browser, opened as a pane. **(Alex)** It has a handful of settings (home page, cookie import, clear data), because Alex asked why Orca's has so many. Cookie import reads Chrome, Brave and Edge. **(Alex)** Cookies never sync between computers or to the phone. **(my call)** The Mac can show and click a browser pane that runs on the PC. **(Alex)** Build much smaller than Orca's 73k lines. **(my call on the size)**
- **Computer control:** the same as Orca's Computer Use: an agent operates apps on the computer it runs on, including the PC while Alex drives it from the Mac. **(Alex)** Needs native helpers and OS permissions.
- **Restore windows on startup:** comes from the host keeping terminals alive, plus a saved layout. After a reboot, chats reopen with resume. **(Alex)** One limit until the stage 2 hooks: a `claude` Alex typed by hand in a shell tab comes back as a plain shell, because the host only knows the tab as a shell. **(my call)**
- **SSH:** saved hosts open as plain shell tabs, with import from `~/.ssh/config`. Shell only, with no agents over SSH. **(Alex)** Alex hasn't used SSH before, so adding a host is a short guided step with a test button. **(my call)**
- **Usage:** a small usage page for Claude, plus the current account's usage as one dim line inside the sidebar, at its bottom, that Alex can hide. It is not a bar across the window. **(Alex wants the numbers and no bar, the placement is my call)**
- **Quick commands, session search, keep awake while agents run, send, read and wait commands.** **(Alex)**
- **Stop idle agents after a time limit, and continue chats when a usage limit resets.** **(Alex)**
- **Worktrees:** only by explicit request. **(Alex)**
- **Settings:** one short settings window and one config file. Fonts, colors and padding come from Alex's kitty config. The import has to look right, checked with a side-by-side screenshot against kitty, because Orca's import looked wrong to Alex. Ghostty import can come later. GPU drawing is on. Right-click paste is a setting. No menu bar icon, no onboarding checklist and no telemetry. **(Alex)**

**Tasks and automations (my call; Alex asked whether they would be useful):** not now. Orca's Tasks is a drawer of GitHub, Linear and Jira items, and its automations are prompts that run on a clock. Alex doesn't work from an issue board, and the master agent already covers keeping work moving while Alex is away. The one piece worth adding later is a trigger: when something happens, such as a session finishing or a pull request merging, send a set prompt to a session.

## Text zoom

- Zoom is universal, like Alex's kitty config (`change_font_size all`): Cmd+= and Cmd+plus make the text 1 px bigger, Cmd+minus 1 px smaller, Cmd+0 goes back to 14 px. Every terminal in every tab, split and window changes together, and new ones open at the current size. **(Alex)**
- No popup or size readout appears while zooming; the text changes and that is all. **(Alex)**
- The size is saved and survives a restart. It stays between 8 and 40 px. The sidebar and tabs keep their own size, so zooming never reflows them. **(my call)**
- This is text zoom. Cmd+Shift+Enter, which makes one pane fill the tab, is a separate thing and keeps its kitty meaning.

## Keys

kitty's keys stay for everything kitty has. **(Alex)** The keys below are new, and where one differs from kitty's, the one below wins. The table is final after the UI review, as recorded in `docs/ui-review.md` "Keys". **(Alex)** **(my call unless marked)**

| Action | Key |
|---|---|
| New tab | Cmd+T **(Alex)** |
| Close pane | Cmd+W **(Alex)** |
| Close tab | Cmd+Shift+W **(Alex)** |
| New tab on the other computer | Cmd+Shift+N |
| Reopen closed tab | Cmd+Shift+T **(Alex)** |
| Next, previous tab | Cmd+Shift+] and [ **(Alex)** |
| Jump to group 1 to 9 | Cmd+1 to Cmd+9 **(Alex)** |
| Jump to tab 1 to 9 in the group | Ctrl+1 to Ctrl+9 |
| Split right, split down | Cmd+D, Cmd+Shift+D **(Alex)** |
| Move between panes | Cmd+Option+arrows **(Alex)** |
| Fill the tab with one pane | Cmd+Shift+Enter **(Alex)** |
| Show or hide the sidebar | Cmd+B |
| Jump to the oldest blocked session | Cmd+Shift+U |
| Board | Cmd+Shift+B |
| Rename | Cmd+Shift+R |
| Zoom in, out, reset | Cmd+= and Cmd+plus, Cmd+minus, Cmd+0 **(Alex)** |
| Copy, paste | Cmd+C, Cmd+V **(Alex)** |

- No default uses the Option key with a letter or number, because AeroSpace on Alex's Mac owns those. Cmd+Option+arrows for moving between panes is the one exception. **(my call, the exception is Alex's)**
- Jumping to a group lands on the tab Alex last used in it. A group's master, if it has one, is tab 1. **(my call)**
- Reopen closed tab brings back a Claude tab with its chat resumed, and a plain shell as a fresh shell in the same folder. Either way it returns to the same group and the same place, including under its master. **(my call)**
- A key never puts up a popup (see the anti-goals).

## Plugins

A small plugin system can add a pane beside the terminal, notifications and access to the built-in browser. **(my call on the shape)**

Uber Eats is the first plugin. **(Alex)**
- Sign in once in the built-in browser, then search and order from that pane.
- The app watches order status and keeps alerting until Alex responds, using the same alerts as sessions.
- Placing an order always stops at the full total with fees and waits for a yes. **(my call)**
- It drives the website, so a redesign can break it, and Uber's terms may not allow automation. Off by default in the public version. **(my call)**

### Claude UI mod

- Alex wants the Claude UI mod they were making folded into this app. **(Alex, 2026-11-09)**
- What it is, from the PC session that owns it (2026-11-09): `msg-ui`, a Claude Code mod that redraws chat rows inside Claude's own terminal screen. Messages from helper agents and other sessions get a colored sender label, agent finish rows get a status mark (done, failed, killed), each reply gets a start marker, folded rows get a hint, and a toggle hides the "allowed by auto mode" note.
- It was built on the PC on 2026-11-10 from Alex's answers on its review page (claude.ai/artifact/TvEphn6iqxqQvp2fBNbSxg), according to that page's comment thread. Not seen running from this session. **(Alex's choices)**: cards for agent and session messages, glyph marks on agent finish rows, teal for agents and violet for other sessions, and a background highlight on Claude's replies like the one Alex's own messages have.
- Hiding the "allowed by auto mode" note turned out not to be possible as a mod, because Claude Code draws that line outside what a mod can change.
- It is a small Claude Code plugin with a hooks module. It lives only on the PC and is not in git, so it needs a home (a repo, or Alex's settings sync) before the Mac or this app can install it.
- It runs inside Claude Code, and Claude Code runs inside this app's terminals, so it works here with nothing to port. This app has no chat view of its own to redraw. **(my call)**
- What "in the terminal" means here **(my call)**: once the mod exists, the app installs it through the same settings folder it already starts Claude with, as a setting that is on for Alex. The sidebar's helper rows use the mod's status marks and sender colors, so a helper looks the same in the chat and in the sidebar. The UI review starts from the mod's glyphs and its teal and violet.
- Changes to the mod happen in its own session on the PC, not as part of this build.

## Settings sync

Keeping Claude settings and knowledge in step across devices is a separate project at `~/projects/claude-sync`. This app keeps its own project list in step (see the sidebar section). Chat history for session moves isn't covered by claude-sync yet, so the move feature carries the chat itself.

## Open source and licensing

- License: MIT. **(my call)**
- Orca is MIT: keep its notice when copying code, don't use its name or logos, and check bundled licenses in the files copied.
- Don't copy from kitty (GPL-3.0), cmux (GPL-3.0-or-later, some BUSL-1.1) or AGPL projects.

## Build order

**(my call)**

1. Prove the risky parts: reopening a chat under another account, resuming under a different path, reading a list of Remote Control sessions, two Remote Control servers under different accounts, the hook signals behind the "your move" rule, and starting the host as a service on Windows.
2. Host process on Mac and Windows, plus the Mac client with a basic sidebar tab list and restore. Measure against the speed targets with `tools/keylat` as soon as typing works through the host, then finish with the speed pass. Steps 1 and 2 are stage 1. The Windows client follows.
3. After the UI review: the full sidebar, groups from folders with the `cd` rule, the focus rule, the four session states with blocked rollup, and typing `claude` with the usual flags.
4. Tailscale attach in both directions: a host on the Mac and a host on the PC, each computer's app showing and opening the other's sessions. Moved ahead of accounts on 2026-11-11. **(Alex: "i want them both to be hosts. tailscale higher than account switch")**
5. Accounts with automatic switching.
6. SSH into the PC and the restart action, then move session and cross-device Claude, then the Remote Control sidebar and silent host.
7. The `ta` command line, the master agent with cleanup, agent team rows, alerts, then the board with the artifact to-do list.
8. Viewer pane, browser pane with cookie import, computer control, presets, other agent tools with messaging between sessions, quick commands, search and SSH hosts.
9. The phone web app, the plugin system and Uber Eats, triggers.

## Left to the builder

File layout, the wire protocol between host and clients, the storage format, and how the Tauri windows are wired. Follow the speed targets and don't add a dependency that hurts startup or typing.

## Plan impacts

`PLAN.md` belongs to the session building stage 1, so these are recorded here for it to pick up:

- No top tab bar is built. The tab list in stage 1 is a basic sidebar.
- The tab review and the sidebar review become one `ui-review-page` review.
- The window has no title bar.
- The host is installed as a service that restarts itself.
- The chat view is dropped, so nothing in stage 1 should prepare for it.
- Cmd+1 to Cmd+9 jump to groups, not tabs.
- The first host has no HTTPS listener and no passkey. Those come with the phone web app.
- Starting a shell must load the app's shell hook in zsh and in PowerShell without editing profile files.
- The board, the master agent, alerts and the other 2026-11-09 additions come after stage 1 and change nothing else in it.

## Leaving Orca

Alex plans to uninstall Orca once this app is better. **(Alex)** When that day comes:

- Remove the Orca commands, skills and rules from Alex's Claude setup.
- Rescue anything under `~/orca/workspaces` first. Those are git worktrees, removed with `git worktree remove`, never `rm -rf`.

## What changed on 2026-11-09

For Alex's approval. Each line points at the section that holds the detail.

- cmux is the reference for feel, kitty stays the reference for look ("Why this exists", "cmux").
- The window is the sidebar and the terminal only: no title bar, top tabs, pane strips, status bar or right panel ("What is on screen").
- Vertical tabs only, compact rows, drag to order, a device filter, no views ("Tabs").
- Join existing tabs into splits, and focus follows the mouse ("Splits").
- Typing focus always returns to the terminal ("Focus").
- A folder becomes a group in one action, and `cd` moves a tab to its repo's group. An ungrouped agent tab still moves on its first edit ("Groups").
- Session markers hold still and follow the "your move" rule ("Session states", "Alerts").
- A board that sorts itself, with one-click answers and an artifact to-do list ("Board").
- The master agent with cleanup ("Master agent").
- Alerts are simple and match the terminal. The kitten buddy and themes are gone ("Alerts").
- The host restarts itself and can be restarted from another computer ("Staying up and fixing it from away").
- The phone app moves last. Remote Control and plain SSH cover the phone until then ("Phone").
- The chat view is dropped. The viewer and the browser are panes beside the terminal ("Features carried over from Orca").
- No task board and no clock automations. Triggers may come later ("Features carried over from Orca").
- New keys, none on the Option key ("Keys").
- A group disappears when its last tab is gone ("Groups").
- A new tab can open on the PC from the Mac ("Tabs").
- The sidebar scrolls as a plain list with no sticky group names ("Tabs").
- The Claude UI mod (`msg-ui`) is installed by the app once it exists, and the sidebar shares its marks and colors ("Claude UI mod").
- A setting adds Alex's flags to a typed `claude`, and `claude master` opens the master ("Other agent tools and local models", "Master agent").

## What changed on 2026-11-10

For Alex's approval. The settled values are in `docs/ui-review.md`.

- The sidebar has a faint edge line and a faint hairline above each group; the Mac theme is the default ("What is on screen").
- No group arrow: the folder icon is the open and closed control ("Tabs").
- A group header shows folder, branch, session count and a rolled-up blocked marker ("Tabs").
- Compact rows by default, two-line rows behind `sidebar_rows`; the device filter is a borderless picker ("Tabs").
- Unfocused panes do not dim by default ("Splits").
- The working marker is a snowflake that ticks; markers reshape on a state change. Handoff requested and failed are recorded looks ("Session states").
- Each state can be set to sound, banner, dock badge or nothing ("Alerts").
- The keys table is final and Cmd+Option+arrows is the one Option exception ("Keys").

## What changed on 2026-11-11

Approved by Alex on 2026-11-11. The full design is in `docs/superpowers/specs/2026-11-11-native-renderer-design.md`.

- The native path is triggered: in same-sitting runs the app typed about 9 ms behind kitty and printed at 22 to 30 MB/s, and a native prototype matched kitty with and without a web sidebar ("Architecture", "Speed evidence").
- Terminals are drawn by a Rust GPU engine. A plain terminal is the base and the benchmark; the app with the sidebar is built around the same engine. The sidebar stays a web page ("Architecture").
- The Mac and Windows clients stop being Tauri apps with xterm.js once the app runs on the engine ("Architecture").
- The phone web app, last in the build order, would keep a web terminal of its own, because the engine does not run in a browser ("Phone", "Build order").
- The host and its protocol do not change.

### Stage A results (the plain terminal, measured 2026-11-11)

The plain terminal is named Paw (Alex, 2026-11-11); its program is `paw`. Snowflake stays the name of the app with the sidebar. The plan and the result files written before the rename still say `snowflake`.

Two sittings on Alex's Mac, each measuring every terminal in turn: 60 keys per round in three rounds, then three runs of 30 MB of coloured text. Same window size (about 1280 by 800 points), font size 14 and 20,000 lines of scrollback for all. The folders are `tools/keylat/results/stage-a-1` and `tools/keylat/results/stage-a-2`. Numbers compare only within one sitting.

Second sitting (`stage-a-2`):

| Terminal | Typing median | Typing p95 | Backspace median | Output median | Peak memory |
|---|---|---|---|---|---|
| Plain terminal (default, `fifo`) | 14.1 ms | 27.5 ms | 13.1 ms | 107.5 MB/s | 200.9 MB |
| Plain terminal (`immediate`) | 13.5 ms | 25.5 ms | 14.0 ms | 75.3 MB/s | 201.4 MB |
| kitty | 15.4 ms | 29.6 ms | 15.5 ms | 75.9 MB/s | 230.4 MB |
| Alacritty 0.17.0 | 14.7 ms | 28.7 ms | 14.6 ms | 78.4 MB/s | 183.9 MB |
| Ghostty 1.4.0 | 19.3 ms | 32.5 ms | 19.3 ms | 129.8 MB/s | 156.2 MB |

First sitting (`stage-a-1`), plain terminal default against the same three: typing median 14.9 ms (kitty 15.9, Alacritty 15.4, Ghostty 19.2), typing p95 26.8 ms (29.2, 30.2, 35.5), output median 125.0 MB/s (95.1, 87.5, 100.7).

- All six stage rules pass in both sittings (`tools/speed/plain-targets.sh`). The plain terminal draws 0 frames in 5 idle seconds and holds 115 MB when idle.
- `ratio to kitty: typing 0.94 output 1.31` (first sitting) and `ratio to kitty: typing 0.92 output 1.42` (second). Later stages must not fall behind this: typing at or below kitty's median, output at or above it, in the same sitting.
- Ghostty prints faster than the plain terminal in the second sitting (129.8 against 107.5 MB/s). On peak memory the plain terminal is third of the four in both sittings: Ghostty and Alacritty both use less, kitty uses more. Output speed and memory are the two places the plain terminal is not first.
- Limits of these runs: four typing rounds out of thirty were cut short when another app took the front (CotEditor twice in the first sitting, Roblox Studio twice in the second); the keys measured before each cut are kept. The thinnest case is the first sitting, where round 1 gave 8 keys for the default mode and none for `immediate`, so that sitting's two plain medians rest on about two rounds each; the second sitting has 158 and 180 keys. Output runs are a quarter of a second long, so single runs swing widely; the rule uses the median of three. Warp was not measured, because nothing lets a script start a program inside it.
