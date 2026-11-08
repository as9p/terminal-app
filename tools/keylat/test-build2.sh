K=/Users/x/projects/terminal-app/tools/keylat
S=/Users/x/projects/term-speed-spike
NODE=/opt/homebrew/bin/node
PREV=$(aerospace list-windows --focused --format '%{window-id}')
rm -f $K/build2.out
SHELL=$K/cat-shell.sh $NODE $S/server.js --no-open > $K/spike-server.log 2>&1 &
SERVER=$!
for i in {1..40}; do lsof -nP -iTCP:4517 -sTCP:LISTEN >/dev/null 2>&1 && break; /bin/sleep 0.25; done
open_build() {
  SPIKE_URL="http://127.0.0.1:4517/?engine=$1" $S/src-tauri/target/debug/term-speed-spike > /dev/null 2>&1 &
  A=$!
  W=""; for i in {1..40}; do W=$(aerospace list-windows --monitor all --pid $A --format '%{window-id}' | head -1); [ -n "$W" ] && break; /bin/sleep 0.25; done
  aerospace layout floating --window-id $W
  aerospace focus --window-id $W
  /bin/sleep 3
}
open_build xterm
$K/keylat $A 60 $K/build-xterm-raw.csv $W keep-size > $K/build-xterm.out 2>&1
echo "=== xterm whole window" >> $K/build2.out; cat $K/build-xterm.out >> $K/build2.out
$K/keylat $A 1 $K/diag.csv $W keep-size-calibrate > $K/diag-xterm.out 2>&1
echo "=== xterm calibration" >> $K/build2.out; grep -E 'calibration|echo area|top is' $K/diag-xterm.out >> $K/build2.out
kill $A; /bin/sleep 1
open_build ghostty
$K/keylat $A 1 $K/diag.csv $W keep-size-calibrate > $K/diag-ghostty.out 2>&1
echo "=== ghostty calibration" >> $K/build2.out; grep -E 'calibration|echo area|top is' $K/diag-ghostty.out >> $K/build2.out
kill $A
kill $SERVER
[ -n "$PREV" ] && aerospace focus --window-id $PREV
echo all-done >> $K/build2.out
