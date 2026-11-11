#!/bin/zsh
# One typing test of any target in bin-window.sh: 60 presses into `cat`, the same test as
# test-app.sh, so no shell prompt or history can change the numbers.
# Usage: test-bin.sh <label> <result file> <raw csv>
# It takes the keyboard for about 30 seconds, so it runs only through tools/speed/run-sitting.sh.

K=/Users/x/projects/terminal-app/tools/keylat
source "${0:A:h}/bin-window.sh"
# An interrupted run still ends its test terminal. After a normal finish BIN_PID is empty, so
# this does nothing.
trap 'close_bin >> "$2" 2>&1' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

PREV=$(aerospace list-windows --focused --format '%{window-id}')
if ! open_bin "$1" "${0:A:h}/cat-shell.sh" > "$2" 2>&1; then
  [ -n "$PREV" ] && aerospace focus --window-id "$PREV"
  exit 1
fi
aerospace focus --window-id "$BIN_WINDOW"
/bin/sleep 3
"$K/keylat" "$BIN_PID" 60 "$3" "$BIN_WINDOW" keep-size >> "$2" 2>&1
# Kept with the run, so a note about a target that would not close is not lost in the job log.
close_bin >> "$2" 2>&1
[ -n "$PREV" ] && aerospace focus --window-id "$PREV"
