#!/bin/zsh
# Starts the speed comparison and waits for it. Only with a fresh go from Alex each time, because
# it takes the keyboard for about 4 minutes.
# Usage: run-sitting.sh <name of the folder under tools/keylat/results>
#
# The job goes through runner.command because Terminal is the app holding the test permissions.

K=/Users/x/projects/terminal-app/tools/keylat
here=${0:A:h}
out=${here:h}/keylat/results/$1

if ! pgrep -f runner.command > /dev/null; then
  aerospace list-windows --focused --format '%{window-id}' > $K/focus.tmp
  open -g -a Terminal $K/runner.command
  /bin/sleep 2
  aerospace focus --window-id $(cat $K/focus.tmp)
fi
rm -f $K/job.log
# The runner is its own process, so the renderer and tab count have to travel in the job line.
# TA_JOB picks another script in this folder to run in place of the whole sitting.
print -r -- "${TA_RENDERER:+TA_RENDERER=$TA_RENDERER }${TA_SPEED_TABS:+TA_SPEED_TABS=$TA_SPEED_TABS }${TA_APP_A:+TA_APP_A=$TA_APP_A }${TA_BUILDS:+TA_BUILDS=${(q)TA_BUILDS} }${TA_APP_B:+TA_APP_B=$TA_APP_B }zsh $here/${TA_JOB:-sitting.sh} $out" > $K/job.tmp
mv $K/job.tmp $K/job.sh
echo "Test running. Hands off the keyboard for about 4 minutes."
for i in {1..600}; do
  grep -q job-finished $K/job.log 2> /dev/null && break
  /bin/sleep 1
done
# Ends the runner's loop, so nothing keeps waiting for jobs once the test is over.
touch $K/stop
cat $K/job.log
cat $out/summary.txt
