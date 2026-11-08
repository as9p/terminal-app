# Verify checklist: terminal app

Tests and test launches never touch the real host: set `TERM_APP_DATA` to a scratch folder and `TERM_HOST_PORT` to anything but 47211. Never change window focus or send keystrokes to the focused window from a test.

## Always

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `pnpm -C client build` (runs tsc, then vite)

## When touched

### host/

- `cargo test -p term-host`

### client/ or client/src-tauri/

- `pnpm -C client tauri build`, then confirm the modified time of `target/release/bundle/macos/terminal-app.app/Contents/MacOS/terminal-app` is newer than the change.
- Launch that build on a test port and read the session's size through the host's `list`. A floated window is 1280x800 and gives 147x38.
- AeroSpace tiles new windows on this Mac, so a different size usually means the window is tiled, not that the fit is wrong.

### Speed numbers

Only through `tools/keylat`, run through `runner.command` in Terminal, with a fresh go from Alex each run. Compare numbers within a run, never across runs.

## Known regressions

- Wrong font with no error. WKWebView's canvas cannot see user-installed fonts, and xterm measures and draws through canvas, so without the bundled web font the app draws Courier. To check, add a temporary page probe that sends `term._core._renderService.dimensions` and a picture of the WebGL canvas to a test socket (the canvas needs `new WebglAddon(true)` for the picture). In the picture, `0` must have JetBrains Mono's inner mark and the letters must have no slab serifs. At device pixel ratio 2 and 14 px, `device.cell` is 17 x 39. A height of 35 means Courier metrics, and a width of 16 means the letter spacing was lost.
- A picture of a window that is not showing is stale. AeroSpace moves windows on other workspaces to the screen corner, and WebKit stops animation frames for a hidden window while the buffer keeps advancing. Compare against the buffer before calling it a drawing bug, or draw once by hand with `term._core._renderService._renderRows(0, term.rows - 1)`.
- `sips --cropOffset` crops the wrong region. Scale the whole picture with `sips -Z` instead.
