# Sourced by test-app.sh and tools/speed/throughput.sh. Opens the release app on a test port
# with its own data folder, so a test can never reach the real host on 47211.

REPO=${${(%):-%x}:A:h:h:h}
APP=$REPO/target/release/bundle/macos/terminal-app.app/Contents/MacOS/terminal-app

# Only a process with the host's exact name gets stopped, whatever else holds the port.
test_host_pids() {
  local pid
  for pid in $(lsof -ti tcp:$1 -sTCP:LISTEN); do
    [ "$(ps -o comm= -p $pid | xargs basename)" = term-host ] && echo $pid
  done
}

# Usage: open_app <port> <shell program>. Sets APP_PID, APP_WINDOW and APP_DATA.
open_app() {
  APP_PORT=$1
  [ -x "$APP" ] || { echo "no release build at $APP"; return 1; }
  if lsof -ti tcp:$APP_PORT -sTCP:LISTEN > /dev/null; then
    echo "port $APP_PORT is already in use"
    return 1
  fi
  APP_DATA=$(mktemp -d -t term-speed)
  # A speed run picks the renderer, webgl or dom, with TA_RENDERER. Unset, the app keeps WebGL.
  local renderer=()
  if [ -n "${TA_RENDERER:-}" ]; then
    renderer=(TA_RENDERER=$TA_RENDERER)
  fi
  # env replaces itself with the app, so APP_PID is still the app's.
  SHELL=$2 TERM_APP_DATA=$APP_DATA TERM_HOST_PORT=$APP_PORT env "${renderer[@]}" "$APP" > /dev/null 2>&1 &
  APP_PID=$!
  APP_WINDOW=""
  for i in {1..40}; do
    APP_WINDOW=$(aerospace list-windows --monitor all --pid $APP_PID --format '%{window-id}' | head -1)
    [ -n "$APP_WINDOW" ] && break
    /bin/sleep 0.25
  done
  if [ -z "$APP_WINDOW" ]; then
    echo "the app opened no window"
    close_app
    return 1
  fi
  # AeroSpace tiles new windows, and a floated window has the same size on every run.
  aerospace layout floating --window-id $APP_WINDOW
}

# The host outlives the app by design, so stopping the app alone would leave it running.
close_app() {
  kill $APP_PID 2> /dev/null
  /bin/sleep 0.5
  local pids=$(test_host_pids $APP_PORT)
  [ -n "$pids" ] && kill ${=pids}
  rm -rf "$APP_DATA"
}
