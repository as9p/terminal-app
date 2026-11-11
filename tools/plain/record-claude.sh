#!/usr/bin/env zsh
# Records a short Claude Code session in the windowless program for the engine's replay test.
# The recording can show the account and folder names, so it stays on this computer: it is
# ignored by git and its screen is never printed.
set -euo pipefail

root="${0:A:h:h:h}"
recording="$root/engine/tests/recordings/claude.local.rec"

if ! command -v claude >/dev/null 2>&1; then
  print -u2 "record-claude: claude is not on the PATH"
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT
# Claude Code starts in an empty folder of its own; the keys file sits beside it, not in it.
folder="$work/session"
keys="$work/keys.txt"
mkdir -- "$folder"
# The folder trust question starts on "No, exit", so the keys move down one and confirm, then
# ask one short thing.
print -r -- '4000 \x1b[B' >"$keys"
print -r -- '4600 \r' >>"$keys"
print -r -- '10000 say hi in five words\r' >>"$keys"
# No exit key: leaving would switch back to the empty main screen, and the replay test compares
# the last screen. The time limit ends the session with the answer still showing.

cargo build --release -p term-plain --manifest-path "$root/Cargo.toml"
# The program refuses a recording path whose folder is missing.
mkdir -p -- "${recording:h}"
# An old recording would make the size check and the timeout rule describe another run.
rm -f -- "$recording"

code=0
# The final screen goes nowhere: it is the session's own text.
"$root/target/release/snowflake" --headless --cols 120 --rows 40 --timeout 30 \
  --cwd "$folder" --keys "$keys" --record "$recording" -- claude >/dev/null || code=$?

size=0
if [[ -f "$recording" ]]; then
  size="$(wc -c <"$recording" | tr -d ' ')"
fi
if (( code != 124 )); then
  print -u2 "record-claude: the session ended by itself with code $code, before the time limit"
  exit 1
fi
if (( size == 0 )); then
  print -u2 "record-claude: nothing was recorded"
  exit 1
fi
print -r -- "record-claude: $recording is $size bytes"
