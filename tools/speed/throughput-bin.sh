#!/bin/zsh
# throughput.sh for any target in tools/keylat/bin-window.sh: how fast it takes in 30 MB of mixed
# colored text, with its window showing. The burst is the same prepared file, printed by
# throughput-shell.sh with cat in large writes, so the terminal is measured and not a generator.
# It also keeps the largest resident size of the terminal's process seen during the runs.
# Usage: throughput-bin.sh <label> <runs>
# It opens windows on the current workspace, so it runs only through run-sitting.sh.

here=${0:A:h}
source "$here/../keylat/bin-window.sh"
go=$here/.cache/throughput-go
result=$here/.cache/throughput-result
# The largest resident size seen, in KiB.
peak=0
# Set while this run's go file exists, so the exit trap removes only a file this run made.
made_go=""

# An interrupted run still ends its test terminal and removes its own go and result files.
# After a normal finish BIN_PID and made_go are empty, so this does nothing.
trap 'close_bin; [ -z "$made_go" ] || rm -f "$go" "$result"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

"$here/gen-output.sh" --prepare 30 > /dev/null || exit 1

# A hidden window may not draw at all, and a hidden run would measure something else.
on_focused_workspace() {
  aerospace list-windows --workspace focused --format '%{window-id}' | grep -qx "$1"
}

# A terminal can slow a window that is covered or not in front, so every run says whether its
# window was the focused one.
focused_window() {
  [ "$(aerospace list-windows --focused --format '%{window-id}')" = "$1" ] && echo yes || echo no
}

# Keeps the terminal's resident size if it is the largest so far.
sample_memory() {
  local rss=$(ps -o rss= -p "$BIN_PID" 2> /dev/null | tr -d ' ')
  [[ "$rss" == <-> ]] && ((rss > peak)) && peak=$rss
}

one_run() {
  rm -f "$go" "$result"
  local window workspace note="" seen="" i
  workspace=$(aerospace list-workspaces --focused)
  open_bin "$1" "$here/throughput-shell.sh" || return 1
  window=$BIN_WINDOW
  aerospace focus --window-id "$window"
  /bin/sleep 1
  on_focused_workspace "$window" || note=" (window was not showing, ignore this run)"
  seen=" (focused: start=$(focused_window "$window")"
  made_go=yes
  touch "$go"
  for i in {1..3000}; do
    [ -f "$result" ] && break
    # About every 200 ms.
    ((i % 2)) && sample_memory
    /bin/sleep 0.1
  done
  sample_memory
  seen="$seen end=$(focused_window "$window"))"
  # Lets the program in the terminal end, which it holds off so the check above sees the window.
  rm -f "$go"
  [ "$(aerospace list-workspaces --focused)" = "$workspace" ] || note=" (workspace changed, ignore this run)"
  echo "$1 $(cat "$result" 2> /dev/null || echo 'no result')$seen$note$(bin_note "$1")"
  # A clean exit may already have closed the window. This covers a run that failed.
  /bin/sleep 0.5
  close_bin
}

PREV=$(aerospace list-windows --focused --format '%{window-id}')
for run in {1..$2}; do
  one_run "$1"
done
rm -f "$go" "$result"
made_go=""
[ -n "$PREV" ] && aerospace focus --window-id "$PREV"
if ((peak > 0)); then
  # MB here is MiB, like the burst's 1 MiB blocks.
  printf '%s peak memory: %.1f MB%s\n' "$1" "$((peak / 1024.0))" "$(bin_note "$1")"
else
  echo "$1 peak memory: not measured$(bin_note "$1")"
fi
