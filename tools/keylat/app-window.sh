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
  local tabs=${TA_SPEED_TABS:-1}
  if [[ $tabs != <-> ]] || ((tabs < 1)); then
    echo "TA_SPEED_TABS must be a whole number of tabs, not $tabs"
    return 1
  fi
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
  # TA_SPEED_TABS=<n> measures with n tabs in the sidebar. The extra tabs start in speed-idle,
  # where the test's shell program sits idle, and the app keeps showing its own first tab.
  if ((tabs > 1)); then
    mkdir -p $APP_DATA/speed-idle
    if ! python3 $REPO/tools/speed/add-tabs.py $APP_PORT $APP_DATA $tabs $APP_DATA/speed-idle; then
      echo "could not give the app $tabs tabs"
      close_app
      return 1
    fi
  fi
}

# The host outlives the app by design, so stopping the app alone would leave it running.
close_app() {
  local pids pid alive i
  kill $APP_PID 2> /dev/null
  /bin/sleep 0.5
  pids=$(test_host_pids $APP_PORT)
  [ -n "$pids" ] && kill ${=pids}
  # A host still writing its files while rm walks the folder makes rm fail with "Directory not
  # empty", so wait for it to be gone first.
  for i in {1..30}; do
    alive=""
    for pid in ${=pids}; do
      kill -0 $pid 2> /dev/null && alive=$pid
    done
    [ -z "$alive" ] && break
    /bin/sleep 0.1
  done
  [ -n "$alive" ] && echo "the test host $alive was still running after 3 seconds"
  rm -rf "$APP_DATA" 2> /dev/null
  if [ -e "$APP_DATA" ]; then
    # Whatever is still there names what kept writing, so it goes into the run's output.
    echo "the data folder was not empty after the test, left in it:"
    find "$APP_DATA" | sed 's/^/  /'
    /bin/sleep 0.5
    rm -rf "$APP_DATA"
  fi
}
