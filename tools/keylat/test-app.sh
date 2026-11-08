#!/bin/zsh
# One typing test of the release app through its host: 60 presses into `cat`, so no shell
# prompt or history can change the numbers.
# Usage: test-app.sh <result file> <raw csv>
# It takes the keyboard for about 30 seconds, so it runs only through tools/speed/run-sitting.sh.

K=/Users/x/projects/terminal-app/tools/keylat
PORT=47398
source ${0:A:h}/app-window.sh

PREV=$(aerospace list-windows --focused --format '%{window-id}')
open_app $PORT ${0:A:h}/cat-shell.sh || exit 1
aerospace focus --window-id $APP_WINDOW
/bin/sleep 3
$K/keylat $APP_PID 60 $2 $APP_WINDOW keep-size > $1 2>&1
close_app
[ -n "$PREV" ] && aerospace focus --window-id $PREV
