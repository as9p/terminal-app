#!/bin/zsh -f
# Runs inside the terminal under test, in place of a shell. Prints the burst and times it until
# the terminal has parsed all of it. The app's host passes -l, which is ignored here.
#
# The end is a cursor position question, not cat finishing. A terminal answers it only after
# everything sent before it, and `cat` finishes early by whatever the terminal still has queued:
# up to 4 MiB in this app, because of the host's flow control.

# The extra tabs of TA_SPEED_TABS start in speed-idle and must not run a burst of their own. cat
# waits without using the CPU and ends by itself when the host closes its terminal.
[[ $PWD == */speed-idle ]] && exec cat

here=${0:A:h}
mb=${SPEED_MB:-30}
result=$here/.cache/throughput-result

zmodload zsh/datetime
# Not raw: a raw terminal would stop turning \n into \r\n, which changes the work being measured.
stty -echo -icanon

# Usage: asked_and_answered <seconds to wait>
asked_and_answered() {
  local deadline=$((EPOCHREALTIME + $1)) char
  print -rn -- $'\e[6n'
  while ((EPOCHREALTIME < deadline)); do
    if read -t 0.1 -k 1 -u 0 char; then
      [[ $char == R ]] && return 0
    fi
  done
  return 1
}

give_up() {
  print -r -- "$1" > $result.tmp
  mv $result.tmp $result
  exit 1
}

# throughput.sh says when the window has its final size.
for i in {1..600}; do
  [ -f $here/.cache/throughput-go ] && break
  /bin/sleep 0.1
done

# A question sent before the app attaches is lost, because the host's screen does not answer
# and its snapshot does not repeat it. So keep asking until the real terminal is listening.
for i in {1..60}; do
  asked_and_answered 0.5 && break
  ((i == 60)) && give_up "no answer before the burst"
done
# Late answers to the earlier questions must not end the timing early.
while read -t 0.3 -k 1 -u 0 char; do :; done

start=$EPOCHREALTIME
cat $here/.cache/burst-$mb.txt
asked_and_answered 300 || give_up "no answer after the burst"
end=$EPOCHREALTIME

printf '%d %.3f %.1f\n' $mb $((end - start)) $((mb / (end - start))) > $result.tmp
mv $result.tmp $result
# The window can close as soon as this ends, and throughput.sh still has to see whether it was
# focused at the end. It takes the go file away once it has looked.
for i in {1..30}; do
  [ -f $here/.cache/throughput-go ] || break
  /bin/sleep 0.1
done
