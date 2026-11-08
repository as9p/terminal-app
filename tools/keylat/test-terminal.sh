K=/Users/x/projects/terminal-app/tools/keylat
PREV=$(aerospace list-windows --focused --format '%{window-id}')
TTY=$(osascript -e 'tell application "Terminal"' -e 'set t to do script "exec cat"' -e 'tty of t' -e 'end tell')
[ -n "$TTY" ] || { echo "no terminal window"; exit 1; }
W=$(osascript -e 'tell application "Terminal"' -e 'repeat with w in windows' -e "if tty of tab 1 of w is \"$TTY\" then return id of w" -e 'end repeat' -e 'end tell')
P=$(pgrep -ax Terminal | head -1)
echo "tty=$TTY window=$W pid=$P"
[ -n "$W" ] && [ -n "$P" ] || { echo "lookup failed"; pkill -t ${TTY#/dev/} -x cat; exit 1; }
/bin/sleep 1.5
aerospace layout floating --window-id $W
aerospace focus --window-id $W
/bin/sleep 0.5
$K/keylat $P 60 $K/terminal-raw.csv $W > $K/terminal.out 2>&1
pkill -KILL -t ${TTY#/dev/} -x cat; pkill -KILL -t ${TTY#/dev/} -x zsh; pkill -KILL -t ${TTY#/dev/} -x -- -zsh
/bin/sleep 0.7
osascript -e "tell application \"Terminal\" to close window id $W saving no"
[ -n "$PREV" ] && aerospace focus --window-id $PREV
