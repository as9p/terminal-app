# Sourced by test-bin.sh and the tools/speed scripts that compare the plain terminal with other
# terminals. Opens one target with a test program running in it, at 1280 by 800 points, font
# size 14 and 20,000 lines of scrollback. The settings come from command-line overrides or the
# files in tools/speed/configs, never from a terminal's own config folder.

REPO=${${(%):-%x}:A:h:h:h}
CONFIGS=$REPO/tools/speed/configs
# The plain terminal. Its path is written only here, so renaming the binary changes one line.
PLAIN_BIN=$REPO/target/release/snowflake
# Every target, in the order a sitting measures them.
BIN_LABELS=(plain-fifo plain-immediate kitty ghostty alacritty warp)

# Usage: first_program <paths...>. Prints the first one that can run.
first_program() {
  local p
  for p in "$@"; do
    [ -n "$p" ] && [ -x "$p" ] && { echo "$p"; return 0; }
  done
  return 1
}

# Usage: bin_program <label>. Prints the program behind a target, or nothing when it is not
# installed.
bin_program() {
  case "$1" in
    plain-fifo|plain-immediate) first_program "$PLAIN_BIN" ;;
    kitty) first_program "$(whence -p kitty)" /Applications/kitty.app/Contents/MacOS/kitty ;;
    ghostty) first_program /Applications/Ghostty.app/Contents/MacOS/ghostty ;;
    alacritty)
      first_program /Applications/Alacritty.app/Contents/MacOS/alacritty "$(whence -p alacritty)" \
        "$HOME/.cargo/bin/alacritty"
      ;;
    warp) first_program /Applications/Warp.app/Contents/MacOS/stable ;;
  esac
}

# Usage: bin_skip_reason <label>. Prints why a target cannot be measured, or nothing.
bin_skip_reason() {
  case "$1" in
    plain-fifo|plain-immediate|kitty|ghostty|alacritty|warp) ;;
    *) echo "not a target"; return ;;
  esac
  [ -n "$(bin_program "$1")" ] || { echo "not installed"; return; }
  # Warp has no scripting dictionary, unlike Terminal in test-terminal.sh, and its warp:// links
  # open a folder, not a program. So nothing here can start the test program in it.
  [ "$1" = warp ] && echo "skipped (cannot start a program in it)"
}

# Usage: bin_note <label>. Warp keeps its own font and size, so every line about it says so.
bin_note() {
  [ "$1" = warp ] && echo " (own settings)"
}

# Usage: carries_mark <pid>. True when that process's own command line holds BIN_MARK, the text
# open_bin put in it. A pid that ended and was reused by any other program never holds it.
carries_mark() {
  [ -n "${BIN_MARK:-}" ] && [[ "$(ps -ww -o args= -p "$1" 2> /dev/null)" == *"$BIN_MARK"* ]]
}

# Usage: launched_pid <process name> <pids running before, space separated with spaces around>.
# Prints the pid of a new process with that name whose command line holds BIN_MARK. Without the
# mark, a kitty or Ghostty the owner opens during the wait could be taken for the test one.
launched_pid() {
  local i c
  for i in {1..40}; do
    for c in $(pgrep -x "$1"); do
      [[ "$2" == *" $c "* ]] && continue
      carries_mark "$c" && { echo "$c"; return 0; }
    done
    /bin/sleep 0.25
  done
  return 1
}

# Usage: open_bin <label> <program run inside the terminal>. Sets BIN_PID and BIN_WINDOW. With
# BIN_STATS set to a file path, the plain terminal writes its frame count there.
open_bin() {
  local label=$1 program=$2 reason found before i title
  BIN_LABEL=$label
  BIN_PID=""
  BIN_WINDOW=""
  BIN_MARK=""
  reason=$(bin_skip_reason "$label")
  [ -z "$reason" ] || { echo "$label: $reason"; return 1; }
  found=$(bin_program "$label")
  # A title unique to this launch. It goes on the command line, which is how the pid is proved.
  title="speed test $label t$$-$RANDOM$RANDOM"
  case "$label" in
    plain-fifo|plain-immediate)
      local stats=()
      [ -n "${BIN_STATS:-}" ] && stats=("TERM_PLAIN_STATS=$BIN_STATS")
      # The plain terminal has no title flag, and $! is already its exact pid, so its mark is
      # its own command line.
      BIN_MARK="$found -- $program"
      # env replaces itself with the terminal, so BIN_PID is the terminal's own pid.
      env "TERM_PLAIN_PRESENT=${label#plain-}" "${stats[@]}" "$found" -- "$program" > /dev/null 2>&1 &
      BIN_PID=$!
      ;;
    kitty)
      BIN_MARK=$title
      before=" $(pgrep -x kitty | tr '\n' ' ')"
      # A new instance, so the overrides apply and nothing is shared with a kitty already open.
      open -na kitty --args --config NONE --title "$title" -o 'font_size=14' \
        -o 'remember_window_size=no' -o 'initial_window_width=1280' -o 'initial_window_height=800' \
        -o 'scrollback_lines=20000' -o 'confirm_os_window_close=0' -o 'update_check_interval=0' \
        -o 'macos_quit_when_last_window_closed=yes' "$program"
      BIN_PID=$(launched_pid kitty "$before")
      ;;
    ghostty)
      BIN_MARK=$title
      before=" $(pgrep -x ghostty | tr '\n' ' ')"
      # Ghostty does not support starting its binary from the command line on macOS, only open.
      # -e also makes it quit when the program ends.
      open -na Ghostty --args --config-default-files=false "--config-file=$CONFIGS/ghostty.conf" \
        "--title=$title" -e "$program"
      BIN_PID=$(launched_pid ghostty "$before")
      ;;
    alacritty)
      BIN_MARK=$title
      "$found" --config-file "$CONFIGS/alacritty.toml" --title "$title" -e "$program" \
        > /dev/null 2>&1 &
      BIN_PID=$!
      ;;
  esac
  [ -n "$BIN_PID" ] || { echo "$label: no new process carrying this run's mark ($BIN_MARK)"; return 1; }
  for i in {1..40}; do
    BIN_WINDOW=$(aerospace list-windows --monitor all --pid "$BIN_PID" --format '%{window-id}' | head -1)
    [ -n "$BIN_WINDOW" ] && break
    /bin/sleep 0.25
  done
  if [ -z "$BIN_WINDOW" ]; then
    echo "$label: no window"
    close_bin
    return 1
  fi
  # AeroSpace tiles new windows, and a floated window keeps the size the target opened at.
  aerospace layout floating --window-id "$BIN_WINDOW"
}

# Ends only the process open_bin started, by its pid, and waits up to 3 seconds for it to go so
# the next target never overlaps it. Safe to call twice: the second call does nothing.
close_bin() {
  local i
  [ -n "${BIN_PID:-}" ] || return 0
  # A process that already ended can have its pid reused, so it is signalled only while its
  # command line still holds this launch's mark.
  if ! carries_mark "$BIN_PID"; then
    BIN_PID=""
    BIN_WINDOW=""
    return 0
  fi
  kill "$BIN_PID" 2> /dev/null
  for i in {1..30}; do
    kill -0 "$BIN_PID" 2> /dev/null || break
    /bin/sleep 0.1
  done
  kill -0 "$BIN_PID" 2> /dev/null && echo "$BIN_LABEL: pid $BIN_PID was still running after 3 seconds"
  BIN_PID=""
  BIN_WINDOW=""
}
