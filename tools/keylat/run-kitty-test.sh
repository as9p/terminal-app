#!/bin/zsh
K=/Users/x/projects/terminal-app/tools/keylat
if ! pgrep -f runner.command > /dev/null; then
  aerospace list-windows --focused --format '%{window-id}' > $K/focus.tmp
  open -g -a Terminal $K/runner.command
  /bin/sleep 2
  aerospace focus --window-id $(cat $K/focus.tmp)
fi
rm -f $K/kitty.out $K/job.log
cp $K/test-kitty.sh $K/job.tmp && mv $K/job.tmp $K/job.sh
echo "Test running. Hands off the keyboard for about 30 seconds."
for i in {1..120}; do grep -q job-finished $K/job.log 2>/dev/null && break; /bin/sleep 1; done
cat $K/kitty.out 2>/dev/null || cat $K/job.log
