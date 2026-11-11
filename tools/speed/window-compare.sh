#!/bin/zsh
# Output speed only, for two builds of the app in turns, so anything drifting on the Mac hits
# both. It types nothing, but it opens windows and takes focus, so it runs only through
# run-sitting.sh: TA_JOB=window-compare.sh TA_APP_B=<other build's binary> run-sitting.sh <name>
# TA_APP_A=<binary> swaps build A too; unset, A is the release build in target/.
# Usage: window-compare.sh <results folder>

here=${0:A:h}
out=$1
mkdir -p $out
[ -x "${TA_APP_B:-}" ] || { echo "TA_APP_B is not a build of the app" > $out/summary.txt; exit 1; }

{
  for round in 1 2 3; do
    echo "A webgl: $(TA_APP=${TA_APP_A:-} $here/throughput.sh app 1 2>&1)"
    echo "B webgl: $(TA_APP=$TA_APP_B $here/throughput.sh app 1 2>&1)"
    echo "A dom:   $(TA_APP=${TA_APP_A:-} TA_RENDERER=dom $here/throughput.sh app 1 2>&1)"
  done
  echo "kitty:   $($here/throughput.sh kitty 1 2>&1)"
  echo "A is ${TA_APP_A:-the release build in target/}, B is $TA_APP_B"
} > $out/summary.txt
