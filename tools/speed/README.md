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

## Safety

The app runs on ports 47398 and 47399 with a temporary data folder, so it cannot reach the real host on 47211. Only a process named `term-host` on those ports gets stopped.
