#!/bin/zsh
# Typing delay for the app's two renderers and kitty, in turns in one sitting, since numbers
# from different sittings drift by a few milliseconds. Runs only through run-sitting.sh:
# TA_JOB=renderer-compare.sh run-sitting.sh <name>
# Usage: renderer-compare.sh <results folder>

here=${0:A:h}
K=/Users/x/projects/terminal-app/tools/keylat
keylat_scripts=$here/../keylat
out=$1
mkdir -p $out

for run in 1 2 3; do
  TA_RENDERER=webgl $keylat_scripts/test-app.sh $out/webgl-$run.out $out/webgl-$run-raw.csv
  TA_RENDERER=dom $keylat_scripts/test-app.sh $out/dom-$run.out $out/dom-$run-raw.csv
  zsh $keylat_scripts/test-kitty.sh
  mv $K/kitty.out $out/kitty-$run.out
  mv $K/kitty-raw.csv $out/kitty-$run-raw.csv
done

# Usage: typing_stats <csv column> <label> <raw files...>
typing_stats() {
  local column=$1 label=$2
  shift 2
  awk -F, -v c=$column 'FNR > 1 && $c >= 0 { print $c }' "$@" | sort -n | awk -v label="$label" '
    { value[NR] = $1; if ($1 > 30) over++ }
    END {
      if (NR == 0) { print label ": no samples"; exit }
      printf "%s: samples=%d median=%.1f p95=%.1f over_30ms=%d (%.0f%%)\n", label, NR,
        value[int((NR + 1) / 2)], value[int(NR * 0.95 + 0.5)], over, 100 * over / NR
    }'
}

{
  for target in webgl dom kitty; do
    typing_stats 2 "$target typing" $out/$target-*-raw.csv
    typing_stats 4 "$target backspace" $out/$target-*-raw.csv
    for run in 1 2 3; do
      typing_stats 2 "  run $run typing" $out/$target-$run-raw.csv
    done
    grep -h 'missed=' $out/$target-*.out
  done
} > $out/summary.txt
