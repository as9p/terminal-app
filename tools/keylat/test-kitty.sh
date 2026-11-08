PREV=$(aerospace list-windows --focused --format '%{window-id}')
BEFORE=" $(pgrep -x kitty | tr '\n' ' ')"
open -na kitty --args --title 'keylat test' cat
P=""; for i in {1..40}; do for c in $(pgrep -x kitty); do [[ "$BEFORE" == *" $c "* ]] || P=$c; done; [ -n "$P" ] && break; /bin/sleep 0.25; done
[ -n "$P" ] || { echo "no new kitty"; exit 1; }
W=""; for i in {1..40}; do W=$(aerospace list-windows --monitor all --pid $P --format '%{window-id}' | head -1); [ -n "$W" ] && break; /bin/sleep 0.25; done
[ -n "$W" ] || { echo "no kitty window"; kill $P; exit 1; }
aerospace layout floating --window-id $W
aerospace focus --window-id $W
/bin/sleep 0.5
/Users/x/projects/terminal-app/tools/keylat/keylat $P 60 /Users/x/projects/terminal-app/tools/keylat/kitty-raw.csv > /Users/x/projects/terminal-app/tools/keylat/kitty.out 2>&1
kill $P
[ -n "$PREV" ] && aerospace focus --window-id $PREV
