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
# Accept the folder trust question, ask one short thing, then leave.
print -r -- '4000 \r' >"$keys"
print -r -- '9000 say hi in five words\r' >>"$keys"
print -r -- '30000 /exit\r' >>"$keys"

cargo build --release -p term-plain --manifest-path "$root/Cargo.toml"
# The program refuses a recording path whose folder is missing.
mkdir -p -- "${recording:h}"

code=0
# The final screen goes nowhere: it is the session's own text.
"$root/target/release/snowflake" --headless --cols 120 --rows 40 --timeout 45 \
  --cwd "$folder" --keys "$keys" --record "$recording" -- claude >/dev/null || code=$?

size=0
if [[ -f "$recording" ]]; then
  size="$(wc -c <"$recording" | tr -d ' ')"
fi
if (( code == 124 )); then
  if (( size == 0 )); then
    print -u2 "record-claude: claude did not exit in time and nothing was recorded"
    exit 124
  fi
  print -u2 "record-claude: warning: claude did not exit in time; keeping the recording"
elif (( code != 0 )); then
  print -u2 "record-claude: the session exited with code $code"
  exit "$code"
fi
print -r -- "record-claude: $recording is $size bytes"
