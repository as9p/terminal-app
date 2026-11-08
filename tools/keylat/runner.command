#!/bin/zsh
# One Terminal window that runs test jobs, so no new windows open. Terminal is the app with the test permissions.
rm -f /Users/x/projects/terminal-app/tools/keylat/stop
while true; do
  if [ -f /Users/x/projects/terminal-app/tools/keylat/job.sh ]; then
    mv /Users/x/projects/terminal-app/tools/keylat/job.sh /Users/x/projects/terminal-app/tools/keylat/job.running
    zsh /Users/x/projects/terminal-app/tools/keylat/job.running > /Users/x/projects/terminal-app/tools/keylat/job.log 2>&1
    echo job-finished >> /Users/x/projects/terminal-app/tools/keylat/job.log
  fi
  [ -f /Users/x/projects/terminal-app/tools/keylat/stop ] && exit 0
  /bin/sleep 0.5
done
