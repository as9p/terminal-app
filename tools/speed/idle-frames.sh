#!/bin/zsh
# Counts the frames the plain terminal presents while nothing on its screen changes. It should
# draw only when something changed: one stray redraw asked for by the system is fine, a steady
# trickle is not. Also reads the terminal's resident size at the end of the wait.
# Usage: idle-frames.sh
# It opens a window and focuses it, so it runs only through run-sitting.sh (plain-compare.sh).

here=${0:A:h}
source "$here/../keylat/bin-window.sh"
tmp=$(mktemp -d -t plain-idle)
trap 'close_bin > /dev/null; rm -rf "$tmp"' EXIT
BIN_STATS=$tmp/stats

# The terminal rewrites the file in place after every frame, so a read can land between the
# truncate and the write. That read finds no number and is tried again.
frames_now() {
  local i count
  for i in {1..20}; do
    count=$(sed -n 's/^frames=//p' "$BIN_STATS" 2> /dev/null)
    [[ "$count" == <-> ]] && { echo "$count"; return 0; }
    /bin/sleep 0.05
  done
  return 1
}

PREV=$(aerospace list-windows --focused --format '%{window-id}')
open_bin plain-fifo "$here/../keylat/cat-shell.sh" || exit 1
aerospace focus --window-id "$BIN_WINDOW"
/bin/sleep 2
first=$(frames_now)
/bin/sleep 5
second=$(frames_now)
rss=$(ps -o rss= -p "$BIN_PID" 2> /dev/null | tr -d ' ')
close_bin
[ -n "$PREV" ] && aerospace focus --window-id "$PREV"

if [ -n "$first" ] && [ -n "$second" ]; then
  echo "idle frames in 5 s: $((second - first))"
else
  echo "idle frames in 5 s: no frame count (first=${first:-none} second=${second:-none})"
fi
if [[ "$rss" == <-> ]]; then
  # MB here is MiB, like the peak memory lines of throughput-bin.sh.
  printf 'plain memory idle: %.1f MB\n' "$((rss / 1024.0))"
else
  echo "plain memory idle: not measured"
fi
