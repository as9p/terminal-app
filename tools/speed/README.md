# Speed tests

`run-sitting.sh <name>` compares the release app with kitty and saves everything under `tools/keylat/results/<name>/`, with the totals in `summary.txt`. It takes the keyboard and opens windows for about 4 minutes, so it needs a fresh go from Alex each time. Build first with `pnpm -C client tauri build`.

## What it runs

- **Typing:** 60 presses and 60 backspaces into `cat`, three runs each, app and kitty in turns. `tools/keylat` reads the time from the key event to the frame showing the change.
- **Output speed:** 30 MB from `gen-output.sh`, three runs each, window showing. This is the same mixed text as the earlier tables in `DESIGN.md`: plain, colored, long wrapped lines, box drawing and CJK. The app's window is made the focused window before each burst.

## Options

Set these before `run-sitting.sh`. They travel to the runner in its job line, and the top of `summary.txt` says what was used.

- `TA_RENDERER=webgl|dom` picks the app's renderer. Unset, the app keeps WebGL.
- `TA_SPEED_TABS=<n>` measures the app with n tabs in its sidebar. Once the app shows its own first tab, `add-tabs.py` asks the test host for the rest. They start in a folder named `speed-idle`, where `throughput-shell.sh` runs `cat` instead of a burst and `cat-shell.sh` runs `cat` anyway, so they sit idle. The app keeps showing the first tab, the measured one. Unset or 1 is one tab, as before. For example `TA_SPEED_TABS=20 zsh tools/speed/run-sitting.sh tabs-20`.

## How to read the numbers

- Compare numbers within one sitting, never across sittings. Everything else running on the Mac moves them.
- `throughput-shell.sh` stops the clock when the terminal answers a cursor position question sent after the burst, which counts everything still queued in the terminal. The older tables in `DESIGN.md` were timed another way, so they are not comparable to these runs.
- MB means the generator's 1 MiB blocks, so MB/s is 30 divided by the seconds. 30 blocks are 31.5 million bytes.
- The app's window floats at 1280x800. kitty runs at about the same size with Alex's config.
- A run marked `ignore this run` had its window hidden, because WebKit stops drawing hidden windows.
- Each app output run also says whether another `terminal-app` was running (its pids, or `none`) and whether the test window was the focused window at the start and end of the burst. WebKit slows a window that is covered or not in front, and another copy of the app shares the GPU with the test one, so a slow run with `end=no` or another copy open is suspect.
- When the app closes, the scripts wait up to 3 seconds for the test host to exit before removing its data folder. If anything is still left, the run's output lists it before it is removed.

## Comparing the plain terminal with other terminals

`TA_JOB=plain-compare.sh zsh tools/speed/run-sitting.sh <name>` times the plain terminal against kitty, Ghostty and Alacritty in one sitting and writes `tools/keylat/results/<name>/summary.txt`. It takes the keyboard and opens windows for about 12 minutes, so it needs a fresh go from Alex each time. Build first with `cargo build --release -p term-plain`. Then `zsh tools/speed/plain-targets.sh tools/keylat/results/<name>/summary.txt` checks the stage's rules, one `PASS` or `FAIL` line each, and exits 1 on any `FAIL`. A second argument checks another plain label, for example `plain-immediate`.

What gets installed and where the scripts look:

- **Plain terminal:** `target/release/paw`, run twice: `plain-fifo` waits for the display before each frame, `plain-immediate` does not (`TERM_PLAIN_PRESENT`). Its path is written only in `tools/keylat/bin-window.sh`.
- **kitty:** on the PATH or in `/Applications/kitty.app`, started with `--config NONE` and `-o` overrides.
- **Ghostty:** `/Applications/Ghostty.app`, started with `--config-default-files=false` and `configs/ghostty.conf`.
- **Alacritty:** `/Applications/Alacritty.app`, then the PATH, then `~/.cargo/bin/alacritty`, started with `--config-file configs/alacritty.toml`.
- **Warp:** `/Applications/Warp.app`. It has no way for a script to start a program in it, so the summary says `warp: skipped (cannot start a program in it)`. Any Warp line would end with `(own settings)`, because Warp keeps its own font and size, and Warp never decides a rule.

A target that is not installed gets a `<label>: not installed` line and the sitting goes on. Every target opens at 1280 by 800 points with font size 14 and 20,000 lines of scrollback, and no script reads or writes any terminal's own config folder. Ghostty and Alacritty size their first window in grid cells, so their size is the nearest whole number of cells; the `window WxH` line in each `<label>-<round>.out` shows the real size.

How to read the summary:

- `<label> typing:` and `<label> backspace:` lines cover all three rounds of 60 presses into `cat`, then one line per round and keylat's `missed=` lines, as in the app's sitting.
- `<label> <MB> <seconds> <MB/s>` lines are output runs, timed by `throughput-shell.sh` as above, then `<label> peak memory: <MB> MB`, the largest resident size of the terminal's process sampled about every 200 ms during its runs.
- `idle frames in 5 s: <n>` is how many frames `plain-fifo` presented in 5 seconds with nothing changing on its screen, and `plain memory idle: <MB> MB` is its resident size at the end of that wait.
- `plain-targets.sh` passes the plain terminal when its typing and backspace medians are at most 1.0 ms above the lowest of kitty, Ghostty and Alacritty, its p95s at most 2.0 ms above the lowest p95, the median of its output runs at least kitty's, and its idle frames at most 1. It also prints `ratio to kitty` and, when Warp was measured, a `beside warp` line. Memory is reported but gates nothing.
- As with the app, numbers from different sittings are never compared, only numbers from the same sitting.

`zsh tools/speed/plain-targets.sh tools/speed/configs/summary-fixture-pass.txt` and the same with `summary-fixture-fail.txt` check the rules themselves without opening anything.

## Safety

The app runs on ports 47398 and 47399 with a temporary data folder, so it cannot reach the real host on 47211. Only a process named `term-host` on those ports gets stopped.
