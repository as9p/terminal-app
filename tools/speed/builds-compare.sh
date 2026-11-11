#!/bin/zsh
# Output speed only, for several builds of the app in turns, to find which change cost speed.
# It types nothing, but it opens windows and takes focus, so it runs only through
# run-sitting.sh: TA_JOB=builds-compare.sh TA_BUILDS="<label>=<binary> ..." run-sitting.sh <name>
# Usage: builds-compare.sh <results folder>

here=${0:A:h}
out=$1
mkdir -p $out
[ -n "${TA_BUILDS:-}" ] || { echo "TA_BUILDS is empty" > $out/summary.txt; exit 1; }

{
  for round in 1 2; do
    for build in ${=TA_BUILDS}; do
      echo "${build%%=*}: $(TA_APP=${build#*=} $here/throughput.sh app 1 2>&1)"
    done
  done
} > $out/summary.txt
