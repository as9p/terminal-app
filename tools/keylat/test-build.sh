K=/Users/x/projects/terminal-app/tools/keylat
S=/Users/x/projects/term-speed-spike
NODE=/opt/homebrew/bin/node
PREV=$(aerospace list-windows --focused --format '%{window-id}')
rm -f $K/build.out
SHELL=$K/cat-shell.sh $NODE $S/server.js --no-open > $K/spike-server.log 2>&1 &
SERVER=$!
for i in {1..40}; do lsof -nP -iTCP:4517 -sTCP:LISTEN >/dev/null 2>&1 && break; /bin/sleep 0.25; done

measure() { # label pid window
  aerospace layout floating --window-id $3
  aerospace focus --window-id $3
  /bin/sleep 0.5
  $K/keylat $2 1 $K/size-$1.csv $3 > $K/size-$1.out 2>&1   # one press, sets the window to 1200x800
  $K/keylat $2 60 $K/$1-raw.csv $3 keep-size-calibrate > $K/$1.out 2>&1
  echo "=== $1" >> $K/build.out; cat $K/$1.out >> $K/build.out
}

for ENGINE in xterm ghostty; do
  SPIKE_URL="http://127.0.0.1:4517/?engine=$ENGINE" $S/src-tauri/target/debug/term-speed-spike > /dev/null 2>&1 &
  A=$!
  W=""; for i in {1..40}; do W=$(aerospace list-windows --monitor all --pid $A --format '%{window-id}' | head -1); [ -n "$W" ] && break; /bin/sleep 0.25; done
  if [ -n "$W" ]; then /bin/sleep 3; measure build-$ENGINE $A $W; else echo "=== build-$ENGINE: no window" >> $K/build.out; fi
  kill $A
  /bin/sleep 1
done

# kitty again with the same spot-only detection, so the methods can be compared directly
BEFORE=" $(pgrep -x kitty | tr '\n' ' ')"
open -na kitty --args --title 'keylat test' cat
P=""; for i in {1..40}; do for c in $(pgrep -x kitty); do [[ "$BEFORE" == *" $c "* ]] || P=$c; done; [ -n "$P" ] && break; /bin/sleep 0.25; done
if [ -n "$P" ]; then
  W=""; for i in {1..40}; do W=$(aerospace list-windows --monitor all --pid $P --format '%{window-id}' | head -1); [ -n "$W" ] && break; /bin/sleep 0.25; done
  [ -n "$W" ] && measure kitty-region $P $W
  kill $P
fi

kill $SERVER
[ -n "$PREV" ] && aerospace focus --window-id $PREV
echo all-done >> $K/build.out
