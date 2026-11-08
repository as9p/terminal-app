#!/bin/zsh
# The whole speed comparison, run once by runner.command: typing in the app and kitty, three
# times each and in turns so anything drifting on the Mac hits both, then output speed.
# Usage: sitting.sh <results folder>

here=${0:A:h}
K=/Users/x/projects/terminal-app/tools/keylat
keylat_scripts=$here/../keylat
out=$1
mkdir -p $out

for run in 1 2 3; do
  $keylat_scripts/test-app.sh $out/app-$run.out $out/app-$run-raw.csv
  zsh $keylat_scripts/test-kitty.sh
  mv $K/kitty.out $out/kitty-$run.out
  mv $K/kitty-raw.csv $out/kitty-$run-raw.csv
done

$here/throughput.sh app 3 > $out/throughput.txt 2>&1
$here/throughput.sh kitty 3 >> $out/throughput.txt 2>&1

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
  echo "release build modified $(stat -f '%Sm' $here/../../target/release/bundle/macos/terminal-app.app/Contents/MacOS/terminal-app)"
  for target in app kitty; do
    typing_stats 2 "$target typing" $out/$target-*-raw.csv
    typing_stats 4 "$target backspace" $out/$target-*-raw.csv
    for run in 1 2 3; do
      typing_stats 2 "  run $run typing" $out/$target-$run-raw.csv
    done
    grep -h 'missed=' $out/$target-*.out
  done
  cat $out/throughput.txt
} > $out/summary.txt
