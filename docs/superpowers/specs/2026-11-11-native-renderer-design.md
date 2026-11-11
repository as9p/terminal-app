# Native terminal engine: design

Date: 2026-11-11. Status: approved by Alex on 2026-11-11. Stage A (the engine and the plain terminal on Mac) is built and passed its speed targets on 2026-11-11; Alex's hand check of the window is still open. Stages B and C are not built. The results are in `DESIGN.md` under "Stage A results".

Lines marked **(Alex)** are what Alex said. Lines marked **(my call)** are decisions made for Alex, who can reject any of them. This document adds to `DESIGN.md` at the repo root; where the two differ on how terminals are drawn, this one wins once approved.

## Why

`DESIGN.md` says a Rust renderer is built only if xterm.js misses the speed targets. It has missed them, and the cause is the web view, not our code.

Same-sitting measurements on Alex's Mac, 2026-11-11 (median key-to-screen delay, output speed for 30 MB of coloured text):

| | Typing | Output |
|---|---|---|
| Today's app, WebGL drawing | 20.3 ms | 22 to 30 MB/s |
| Today's app, plain DOM drawing | 21.6 ms | 68 to 85 MB/s |
| kitty (that sitting) | 11.3 ms | about 80 MB/s |

A throwaway prototype (Rust, GPU drawing, real shell, web sidebar in the same window) measured in its own sitting:

| | Typing median | 95th percentile | Output |
|---|---|---|---|
| Prototype, no sidebar | 14.1 ms | 25.3 ms | 86 to 101 MB/s |
| Prototype, web sidebar beside it | 14.2 ms | 25.3 ms | 101 to 105 MB/s |
| kitty (that sitting) | 14.1 ms | 22.2 ms | 70 to 77 MB/s |

The prototype had no scrollback, so its output number flatters it. Its typing number is a fair comparison. Alex tried it by hand: the keyboard went to the terminal unless the sidebar text box was focused, and it felt faster.

## What Alex asked for

- Replace the terminal drawing with a native Rust GPU renderer. Keep the Rust host. **(Alex)**
- A plain terminal that is "literally just a terminal", and the sidebar and everything else built on top of that same thing. The plain one is what gets compared with other terminals, since they are plain terminals too. **(Alex)**
- It should feel like a regular terminal; the only benefit you notice is the speed. **(Alex)**
- The real workload is Claude Code, many sessions at once, in long sittings, across many projects, and it has to stay seamless. **(Alex)**
- The exact same thing on the Windows PC. **(Alex)**
- From one computer, click into the other computer's terminals as if they were local, over Tailscale. This belongs to the sidebar layer, with the native terminal as the base. **(Alex)** It is already designed in `DESIGN.md` ("Architecture", build order step 5) and this document does not change it.
- The phone is not part of this. Phone use goes through Claude's Remote Control. **(Alex)**
- After a sidebar rename, Enter saves the name and the keyboard goes straight back to the terminal. **(Alex)**
- If it is good, become known for the fastest terminal. **(Alex)**

## Goals

1. Typing in the plain terminal is as fast as the fastest terminal on the same computer, measured in the same sitting.
2. The full app types as fast as the plain terminal. The sidebar, other panes, other tabs and busy agents cost the focused terminal nothing you can measure.
3. One engine. The plain terminal and the full app run the same engine code, so the measured thing is the used thing.
4. It runs Claude Code correctly: what Claude draws looks right and does not flicker.
5. Mac and Windows from the same code.
6. The speed claim can be rerun by anyone with a published script.

## Non-goals for now

- A native sidebar. The sidebar stays a web page. It cost nothing in the prototype and keeps the reviewed sidebar work. **(Alex chose terminal only)**
- Running the engine on a phone or in a browser. **(Alex)**
- Pictures inside the terminal, search within scrollback, font ligatures, and typing with a composing pop-up for languages that need one. They come later and must not be blocked by the design. **(my call)**
- Linux. Nothing should rule it out, but nothing is built or tested for it. **(my call)**
- Screen reader support for the terminal area. **(my call)**
- Changing the host's job or its protocol beyond what a stage below names.

## Anti-goals: it failed if

- A keystroke or a byte of terminal output ever passes through the web view on its way between the keyboard, the shell and the screen.
- There are two engines, or the plain terminal and the app draw with different code.
- Sidebar activity, another pane's output or the number of open tabs changes the typing delay of the focused terminal.
- Alex has to click the terminal to type again after using the sidebar, a menu or a rename. (Already an anti-goal in `DESIGN.md`; restated because two kinds of view now share the window.)
- The app draws frames while nothing on screen is changing.
- Memory keeps growing over a long day with many sessions.
- Code is copied from kitty, cmux, Zed or any other GPL or AGPL project. Ideas are fine. (`DESIGN.md`, "Open source and licensing".)
- Anything in `DESIGN.md`'s anti-goal list comes back (title bar, popups on shortcuts, another device resizing a terminal, and so on).

## The shape

Three layers, each a separate Rust crate or program. **(my call on the split; the idea of base plus add-on is Alex's)**

1. **Engine.** A library. It is given terminal bytes and a rectangle to draw in, and it gives back the bytes to send when a key is pressed or the mouse is used. It holds the screen state (grid, scrollback, selection, cursor) and draws it on the GPU. It knows nothing about shells, hosts, sockets, sidebars, tabs or windows. Several engine views can live in one window and share one GPU device and one glyph texture.
   - The number of rows and columns is set by whoever owns the view, never worked out by the engine on its own. The engine only answers "how many rows and columns fit in this rectangle". **(my call)**
   - A view can draw a grid that does not match its rectangle: the grid is anchored top left, the rest is filled with the background colour, and anything that does not fit is clipped. This one rule serves the resize behaviour (the grid stays put while the rectangle is still changing) and `DESIGN.md`'s rule that a terminal shown on a second device is letterboxed and never resized by it. Scaling a grid to fit is not built now. **(my call)**
2. **Plain terminal.** A small program: one window, one engine view filling it, one shell started directly on this computer with no host in between. It has scrollback, selection, copy, paste and text zoom, and nothing else: no tabs, no splits, no sidebar, no settings screen. It exists to be measured and to prove the engine. It uses the built-in theme and font and reads no kitty config. **(my call that it has no tabs or splits; Alex has not said whether they would use it day to day)**
3. **The app.** Today's product, rebuilt around the engine: the web sidebar on the left, engine views for the panes on the right, connected to the host on this computer and, later, to hosts on other computers. Everything in `DESIGN.md` about tabs, splits, groups, focus, keys, zoom, accounts and devices still applies to this layer unchanged.

### What moves where

- **Out of the web view, into Rust:** drawing, terminal parsing, the keyboard path, shortcuts, pane layout and splits, the connection that carries terminal bytes to and from the host. **(my call)**
- **Stays in the web view:** the sidebar's rows, groups, drag and drop, rename, markers and its styles. **(my call)**
- **One owner of state.** The Rust side owns the truth (which sessions exist, which tab is showing, which pane has focus). The sidebar shows what it is told and reports what the user did (clicked a row, renamed, dragged). It never talks to the host itself. **(my call)** This avoids two halves of the app disagreeing, and it is what lets a shortcut work with no detour through the web page.
- **The window is no longer a Tauri app.** It uses the windowing library and the web view library that Tauri itself is built on (winit and wry), which is the combination the prototype measured. Tauri's partial-window web view is still marked unstable and nobody has shown it working beside GPU drawing. **(my call)** What Tauri gave us has to be replaced: building the Mac app bundle and Windows installer, starting the bundled host, the menu, clipboard, remembering window size and position, and dragging and resizing a window with no title bar.
- **The host does not change in stage A, and its protocol does not change in stage B** unless measurement shows the local connection adds delay (see targets).

### Keyboard focus between the two kinds of view

- The terminal has the keyboard by default. Clicking a sidebar row, dragging, or using a menu never takes it away.
- A sidebar text field (rename, and any later field) takes the keyboard only while it is being typed in. Enter saves and returns the keyboard to the terminal. **(Alex)** Escape cancels and returns it, and so does clicking outside the field. **(my call)**
- Shortcuts work wherever the keyboard is, including while a sidebar field is focused. **(my call)**

## Day one: what "a regular terminal for Claude Code" means

**(my call on the list; Alex said "regular terminal" and "I just use Claude Code really")**

- Scrollback of 20,000 lines per terminal, as today, scrolled with wheel or trackpad.
- Mouse selection by character, word (double click) and line (triple click), copy, paste, and paste of many lines as one paste (bracketed paste).
- Full colour, bold, italic, underline styles, the kitty theme colours in `client/src/theme.ts`, the same font, size and line height as today (JetBrainsMono Nerd Font Mono, 14 px, 1.1), with fallback fonts for symbols and colour emoji, and box and block characters that join with no gaps.
- No flicker while a full-screen program redraws (synchronized output, terminal mode 2026).
- Wide characters and combined characters land in the right cells.
- Shift+Enter makes a new line in Claude's prompt; the Mac line keys already built keep working; Option behaves as today.
- Programs that use the mouse (scrolling inside Claude Code, clicking) get mouse events.
- Links open with Cmd+click (Ctrl+click on Windows), both plain URLs and links a program marks itself.
- Universal text zoom as in `DESIGN.md`. Resizing behaves like the current fix: nothing is refitted while the size is still changing, then one refit when it settles.
- Reconnecting to a session shows its screen and scrollback from the host's copy, as today.
- A terminal that is not visible does no drawing work.

## How it is built: decisions and what is left open

- **Screen state and parsing:** use an existing permissively licensed Rust terminal library, not a new parser. The leading candidate is `alacritty_terminal` (Apache-2.0), which Alacritty and Zed's terminal use and which is believed to cover scrollback, rewrapping on resize, selection, mouse modes, bracketed paste, marked links and synchronized output. This has not been verified against the day-one list. **(my call)**
- **The first task of stage A is a go or no-go check of that library**, done with small tests and no drawing code. It passes if the library handles every item in the day-one list that depends on screen state (scrollback of 20,000 lines, rewrap on resize, selection, mouse modes, bracketed paste, synchronized output, links a program marks itself, wide and combined characters, the key reporting Claude Code needs for Shift+Enter), and if 40 terminals with full scrollback stay within a memory figure the check itself reports and the plan then fixes. A small gap is closed by a patch kept in this repo, with its licence notice. A large gap means trying, in this order, `termwiz` (MIT, from WezTerm) and then Ghostty's terminal library (only if it builds on Windows). Switching library after drawing code exists is the expensive mistake this gate prevents. **(my call)**
- **Reconnecting:** the host keeps its own `vt100` screen copy and its protocol as they are. Attach already carries the client's rows and columns, and the host replays its copy as ordinary terminal bytes at that size (`host/src/protocol.rs`, `host/src/screen.rs`). The engine is set to the size it sent in attach, is fed the replay like any other output, and is resized afterwards if needed. The engine and the host never have to agree on wrapping, because the engine only ever sees bytes.
- **App to host connection:** the existing WebSocket with its token, opened from Rust instead of from the page. The host already accepts a connection with no Origin header as long as the token is right (`host/src/server.rs`, `origin_passes`), so the host needs no change for this. A different local transport is considered only if the app target below is missed and measurement blames the connection. **(my call)**
- **Keys with two kinds of view:** Rust sees every key first. The app's shortcuts (the "Keys" table in `DESIGN.md`) are handled there wherever the keyboard is. While a sidebar text field is focused, everything else goes to the web view, including Cmd+A, Cmd+C, Cmd+V, Cmd+X and Cmd+Z, so the field edits like any text field. Keys are fixed as in that table; reading key settings from a kitty config is not part of this. **(my call)**
- **Window layout:** the sidebar is a child web view at the left edge, as in the prototype, not a transparent page over the terminals. Anything that later has to appear over the terminal area is drawn by Rust or shown in its own short-lived web view; that is decided when the first such feature is built. **(my call)**
- **Drawing:** `wgpu` (Metal on Mac, DirectX 12 on Windows). Every glyph is drawn once into a shared texture and a frame is a small number of draw calls. Only rows that changed are rebuilt. The prototype used a general text library and re-laid out changed rows; the real engine may need its own cell-grid drawing to hold the output target with scrollback on. Left to the builder, held to the targets.
- **Frame timing:** draw when something changed (a key echo arrived, output arrived, the window changed), never on a timer. Reading shell or host output happens off the drawing thread. Show a frame with the least queueing the system allows. Whether to wait for the screen's refresh was decided by measurement in stage A: waiting (`fifo`) stays the default. Not waiting (`immediate`) typed 13.8 ms against 14.9 ms in the first sitting and 13.5 ms against 14.1 ms in the second, which clears the 0.5 ms bar set for it, but it printed 17 to 30 percent slower (104.3 against 125.0 MB/s, then 75.3 against 107.5 MB/s) and fell below kitty's output speed in the second sitting, and its typing gain is no bigger than the spread between the default's own rounds (13.6 to 14.9 ms). So the default is the mode that passes every target and cannot tear; `immediate` stays available through `TERM_PLAIN_PRESENT` for measuring. A no-wait mode that limits how often it draws during heavy output is the untried next step. On Windows, leave room to bypass `wgpu` for presenting if it cannot reach the lowest delay.
- **Left to the builder:** crate names and file layout, the message format between Rust and the sidebar page, the packaging tool, how the frameless window is dragged and resized, and how the sidebar code is adapted to receive state instead of fetching it.

## Speed targets

Measured with the project's typing and output tests, always in turns within one sitting, on release builds. Numbers from different sittings are never compared. **(my call)**

- **Plain terminal, Mac:** typing and backspace medians no more than 1 ms behind the fastest of kitty, Ghostty and Alacritty; 95th percentile no more than 2 ms behind; output at least equal to kitty with 20,000 lines of scrollback on.
- **The app, Mac:** typing median and 95th percentile within 1 ms of the plain terminal, with the sidebar showing, 20 tabs open and another pane printing output. If the local host connection is what costs more than that, the transport between app and local host is changed before anything else is built on it.
- **Windows:** the same two rules against Windows Terminal, Alacritty and WezTerm. This needs a Windows version of the typing test, which does not exist yet.
- **Memory:** an idle app with 40 sessions stays flat over an hour; the plan sets the number after the first measurement.
- **The public claim** waits for all of the above plus one outside check: a slow-motion phone video (240 frames a second) of key press to screen, which is the method others have published with.

## Stages

Each stage ends with something Alex can use and a speed sitting. Each gets its own plan. **(my call)**

- **Stage A: engine and plain terminal on Mac.** Ends when the plain terminal passes the day-one list and its Mac targets. The app is untouched, so Alex keeps using today's app.
- **Stage B: the app on the engine, Mac.** The new window, the sidebar as a pure view, panes and splits, shortcuts, the host connection. Ends when it does everything today's client does, passes the app targets, and Alex has used it for real work. Then the xterm.js terminal code and Tauri are removed.
- **Stage C: Windows.** The plain terminal first, then the app, plus the Windows typing test. Needs a way to build and measure on the PC (a session on the PC, or SSH from the Mac).

Until stage B ends, the current app stays the daily driver and gets no more speed work. Work in `DESIGN.md`'s build order that lives in the host or the sidebar (accounts, Tailscale attach, alerts) is not blocked by this and carries over.

## Testing

- The engine is tested without a window: feed it bytes, check the grid, the selection and the bytes it sends back for keys and mouse.
- Drawing is checked by rendering to an off-screen image and comparing with stored images for a small set of screens (colours, box characters, emoji, wide characters, cursor, selection).
- A recorded Claude Code session is replayed through the engine as a regression check for correctness and for flicker.
- Every stage's last step is a speed sitting, and a slowdown against the previous sitting's ratio to kitty blocks the stage.

## Consequences for `DESIGN.md`

After approval, `DESIGN.md` gets a dated "What changed" entry that points here and updates three lines of "Architecture": the clients are no longer Tauri apps with xterm.js, the native path was triggered by measurement, and the phone web app (last in the build order) would keep a web terminal of its own because the engine does not run in a browser.

## Risks

- **The screen library falls short** on something Claude Code needs. Guard: verify first, before any drawing code.
- **Losing Tauri costs more than expected** (packaging, frameless window behaviour, the bundled host). Guard: stage B starts with a walking skeleton that is packaged and launched like the real app before features are ported.
- **The host hop adds delay** the prototype never had, since it ran its shell directly. Guard: the app target is measured as soon as typing works through the host.
- **Windows presents frames later than Mac** through `wgpu`. Guard: the design allows presenting directly; measured at the start of stage C.
- **Fonts**: fallback, emoji and Nerd Font symbols are where terminal renderers look wrong. Guard: the image tests above, and Alex's eyes on a Claude Code screen at the end of stage A.
- **Two things to keep in step for a while**: today's client and the new app both exist until stage B ends. Guard: no features are added to today's terminal code during that time.
