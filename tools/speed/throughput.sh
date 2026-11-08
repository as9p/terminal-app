#!/bin/zsh
# Measures how fast a terminal takes in 30 MB of mixed colored text, with its window showing.
# Usage: throughput.sh app|kitty <runs>
# It opens windows on the current workspace, so it runs only through run-sitting.sh.

here=${0:A:h}
PORT=47399
source $here/../keylat/app-window.sh
go=$here/.cache/throughput-go
result=$here/.cache/throughput-result

$here/gen-output.sh --prepare 30 > /dev/null || exit 1

# Usage: open_kitty. Sets KITTY_PID and KITTY_WINDOW.
open_kitty() {
  local before=" $(pgrep -x kitty | tr '\n' ' ')" c
  # Alex's config, at the app's window size and not whatever size kitty remembered.
  open -na kitty --args --title 'speed test' -o remember_window_size=no \
    -o initial_window_width=1280 -o initial_window_height=800 $here/throughput-shell.sh
  KITTY_PID=""
  for i in {1..40}; do
    for c in $(pgrep -x kitty); do
      [[ "$before" == *" $c "* ]] || KITTY_PID=$c
    done
    [ -n "$KITTY_PID" ] && break
    /bin/sleep 0.25
  done
  [ -n "$KITTY_PID" ] || { echo "no new kitty"; return 1; }
  KITTY_WINDOW=""
  for i in {1..40}; do
    KITTY_WINDOW=$(aerospace list-windows --monitor all --pid $KITTY_PID --format '%{window-id}' | head -1)
    [ -n "$KITTY_WINDOW" ] && break
    /bin/sleep 0.25
  done
  [ -n "$KITTY_WINDOW" ] || { echo "no kitty window"; kill $KITTY_PID; return 1; }
  aerospace layout floating --window-id $KITTY_WINDOW
}

# WebKit stops drawing a hidden window, and a hidden run would measure something else.
on_focused_workspace() {
  aerospace list-windows --workspace focused --format '%{window-id}' | grep -qx "$1"
}

one_run() {
  rm -f $go $result
  local window workspace note=""
  workspace=$(aerospace list-workspaces --focused)
  if [ "$1" = app ]; then
    open_app $PORT $here/throughput-shell.sh || return 1
    window=$APP_WINDOW
  else
    open_kitty || return 1
    window=$KITTY_WINDOW
  fi
  /bin/sleep 1
  on_focused_workspace $window || note=" (window was not showing, ignore this run)"
  touch $go
  for i in {1..3000}; do
    [ -f $result ] && break
    /bin/sleep 0.1
  done
  [ "$(aerospace list-workspaces --focused)" = "$workspace" ] || note=" (workspace changed, ignore this run)"
  echo "$1 $(cat $result 2> /dev/null || echo 'no result')$note"
  # A clean exit already closed the window. This covers a run that failed.
  /bin/sleep 0.5
  if [ "$1" = app ]; then
    close_app
  else
    kill $KITTY_PID 2> /dev/null
  fi
}

PREV=$(aerospace list-windows --focused --format '%{window-id}')
for run in {1..$2}; do
  one_run $1
done
rm -f $go $result
[ -n "$PREV" ] && aerospace focus --window-id $PREV
