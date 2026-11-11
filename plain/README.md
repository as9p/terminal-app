# Plain terminal

A deliberately plain terminal built on the engine in `engine/`: one window, one shell, nothing else. Its program is called `snowflake`. That is a working name; it will be renamed to Paw later.

It exists to be measured and to prove the engine, so it stays small. A sidebar app is planned on top of the same engine; that app is not part of this program.

## What it is, and what it is not

It opens one window of 1280 by 800 points and runs your shell in it. The shell is `$SHELL -l`, or `/bin/zsh -l` when `SHELL` is empty or unset. It draws with the GPU through the engine, keeps 20,000 lines of scrollback, and uses a built-in colour theme and a built-in font (JetBrainsMono Nerd Font Mono, 14 px to start).

It has none of these: tabs, splits, a sidebar, a settings file or settings screen, a menu, search in the scrollback, more than one window, images in the terminal, or a composing pop-up for input methods. The theme and font are fixed. The bell does nothing. When the shell exits, the window closes and `snowflake` exits with the shell's exit code.

Programs inside it see `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=snowflake` and `TERM_PROGRAM_VERSION` set to the crate version.

## Build and run

```
cargo build --release -p term-plain
target/release/snowflake
```

Command-line flags (`plain/src/args.rs`):

- `-- COMMAND ARGS...` runs that command instead of the shell. Everything after `--` belongs to the command.
- `--cwd DIR` starts the command or shell in `DIR`.
- `--record FILE` writes what the command printed, with times, to `FILE`. The file is created before the command starts, so a bad path fails early.
- `--help`, `-h` and `--version`, `-V` print the usage text or the version and exit.
- `--headless --cols N --rows N` runs with no window, then prints the final screen. It needs both sizes. The tests use this mode.
- `--keys FILE` (headless only) types lines of `<milliseconds since start> <text>` at their times. The text may hold `\r`, `\n`, `\t`, `\\` and `\xNN`.
- `--timeout SECONDS` (headless only) kills the command after that long and exits with code 124.

Exit codes: 2 for a bad command line, 127 when the command cannot start, 124 on a headless timeout.

Environment variables read by the program:

- `SHELL` picks the shell (`plain/src/shell.rs`).
- `TERM_PLAIN_PRESENT` is `fifo` (the default) or `immediate` (`plain/src/window.rs`). `fifo` waits for the display before showing each frame and cannot tear. `immediate` shows frames at once. Any other value prints a warning and uses `fifo`.
- `TERM_PLAIN_STATS` is a file path. After each frame the program rewrites that file with `frames=<count>`. The speed comparison uses it to count frames when nothing changes.

## Keys and mouse

On Mac. Cmd means the Command key.

| Key or action | What it does |
|---|---|
| Cmd+C | Copies the selected text. Does nothing when nothing is selected; it never sends Ctrl+C. |
| Cmd+V | Pastes the clipboard text. Line breaks become carriage returns. If the program turned on bracketed paste, the text is wrapped in the paste markers, and any end marker inside it is removed first so pasted text cannot run as typed commands. |
| Cmd+= or Cmd++ | Zoom in by one pixel of font size (Shift may be held, since `=` and `+` share a key on most layouts). |
| Cmd+- | Zoom out by one pixel. |
| Cmd+0 | Reset the font size to 14 px. |
| Cmd+K | Clears the scrollback and sends Ctrl+L so the shell redraws its prompt. In a full-screen program only the Ctrl+L is sent. |
| Cmd+A | Selects everything, scrollback included. |
| Cmd+Up, Cmd+Down | Scroll the view by one page less one line. Not in a full-screen program. |
| Cmd+Home, Cmd+End | Scroll to the top or the bottom of the scrollback. Not in a full-screen program. |
| Cmd+W, Cmd+Q | Close the window and exit. There is only one window, so both do the same. |
| Shift+Enter | Sends Escape then Return, so a program can tell it from Enter. If the program turned on the kitty keyboard protocol, it sends that protocol's Shift+Enter instead. |
| Left Option | Works as Alt. Right Option types special characters as the layout says. |
| Any other key | Goes to the program, encoded the way xterm does, or the kitty keyboard protocol way when the program asks for it. Typing or pasting scrolls the view back to the bottom. |
| Drag with the left button | Selects text. Releasing copies it to the clipboard at once. A click that selects nothing clears the old selection. |
| Double click, triple click | Selects a word, then a line. Clicks must be on the same cell within 400 ms. |
| Cmd+click on a link | Opens it with `/usr/bin/open`. It works on a link the program marked and on a plain URL in the text. |
| Scroll wheel or trackpad | Scrolls the scrollback, 3 lines per wheel notch. In a full-screen program that asks for it, it sends arrow keys instead. |
| Mouse in a program that asked for the mouse | The program gets clicks, drags and wheel. Hold Shift to select text yourself instead. |

A program can also set the clipboard with the terminal's clipboard escape code, and set the window title. An empty title brings back "Snowflake".

## Known limits

- Zoom is not remembered between runs. Nothing is: there is no settings file.
- The cursor never blinks.
- Only `http` and `https` links open on Cmd+click. Links are not underlined when the pointer is over them.
- The window has the system's normal title bar. A frameless window is later work.
- Mac only so far. Links open through `/usr/bin/open`, the shortcuts above are the Mac ones, and Windows is a later stage in the design.
- Selecting text copies it at once, as the current app does.
- The default frame mode is `fifo`. `immediate` is available only through `TERM_PLAIN_PRESENT` and is used for measuring.

## Speed

Measured on 2026-11-11 on a MacBook Pro, every terminal in turn in one sitting, same window size, font size and scrollback. Typing is the time from a key press to the letter on screen (lower is better). Output is how fast 30 MB of coloured text scrolls through (higher is better).

| Terminal | Typing median | Typing p95 | Output median | Peak memory |
|---|---|---|---|---|
| This terminal | 14.1 ms | 27.5 ms | 107.5 MB/s | 200.9 MB |
| kitty | 15.4 ms | 29.6 ms | 75.9 MB/s | 230.4 MB |
| Alacritty 0.17.0 | 14.7 ms | 28.7 ms | 78.4 MB/s | 183.9 MB |
| Ghostty 1.4.0 | 19.3 ms | 32.5 ms | 129.8 MB/s | 156.2 MB |

p95 means 95 keys in 100 were at least this fast. The terminal draws nothing while idle (0 frames in 5 seconds) and holds about 115 MB then. Ghostty prints faster and uses less memory. This terminal had the lowest typing delay in both sittings of the day, but its lead over Alacritty and kitty (about 0.5 to 1.3 ms) is no bigger than the spread between its own rounds, so read it as level with them and clearly ahead of Ghostty. Both sittings, with their limits, are written up in `DESIGN.md` under "Stage A results".

## Tests

```
cargo test -p term-engine
cargo test -p term-plain
```

The engine tests include drawing on the GPU without opening a window. One replay test is skipped unless a local recording of a Claude Code session exists. The plain tests run the built binary in its windowless mode (`--headless`), so they open no window and do not touch the keyboard.

To rerun the speed comparison against kitty, Ghostty and Alacritty, build first, then run from the repository root:

```
cargo build --release -p term-plain
TA_JOB=plain-compare.sh zsh tools/speed/run-sitting.sh <name>
zsh tools/speed/plain-targets.sh tools/keylat/results/<name>/summary.txt
```

Warning: the comparison opens real windows and types into them for about 12 minutes. It takes over the keyboard, so start it only when nobody needs the computer, and leave the keyboard alone until it finishes. If another app comes to the front, the round being measured is cut short. The first command writes the numbers to `tools/keylat/results/<name>/summary.txt`; the second prints one `PASS` or `FAIL` line per rule and exits 1 on any `FAIL`. Compare numbers only within one sitting. Details are in `tools/speed/README.md`.
