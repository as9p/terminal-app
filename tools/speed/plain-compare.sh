#!/bin/zsh
# The plain terminal against kitty, Ghostty and Alacritty in one sitting: typing in three rounds,
# each round one test per target in turn so anything drifting on the Mac hits all of them; then
# three output runs per target; then the plain terminal's idle frames. A target that is not
# installed is skipped with a line in the summary. Check the summary with plain-targets.sh.
# It takes the keyboard for about 12 minutes, so it runs only through run-sitting.sh:
# TA_JOB=plain-compare.sh run-sitting.sh <name>
# Usage: plain-compare.sh <results folder>

here=${0:A:h}
keylat_scripts=$here/../keylat
source "$keylat_scripts/bin-window.sh"
out=$1
mkdir -p "$out"
rm -f "$out/throughput.txt" "$out/idle.txt" "$out/notes.txt"

targets=()
skipped=()
for label in "${BIN_LABELS[@]}"; do
  reason=$(bin_skip_reason "$label")
  if [ -n "$reason" ]; then
    skipped+=("$label: $reason")
  else
    targets+=("$label")
  fi
done

for round in 1 2 3; do
  for label in "${targets[@]}"; do
    if ! zsh "$keylat_scripts/test-bin.sh" "$label" "$out/$label-$round.out" "$out/$label-$round-raw.csv"; then
      echo "$label typing round $round failed: $(tr '\n' ' ' < "$out/$label-$round.out" 2> /dev/null)" >> "$out/notes.txt"
    fi
  done
done

for label in "${targets[@]}"; do
  zsh "$here/throughput-bin.sh" "$label" 3 >> "$out/throughput.txt" 2>&1
done

if ((${targets[(Ie)plain-fifo]})); then
  zsh "$here/idle-frames.sh" > "$out/idle.txt" 2>&1
else
  echo "idle frames in 5 s: plain-fifo was not measured" > "$out/idle.txt"
fi

# Usage: typing_stats <csv column> <label> <raw files...>. The same numbers as sitting.sh.
typing_stats() {
  local column=$1 label=$2
  shift 2
  (($# > 0)) || { echo "$label: no samples"; return; }
  awk -F, -v c="$column" 'FNR > 1 && $c >= 0 { print $c }' "$@" | sort -n | awk -v label="$label" '
    { value[NR] = $1; if ($1 > 30) over++ }
    END {
      if (NR == 0) { print label ": no samples"; exit }
      printf "%s: samples=%d median=%.1f p95=%.1f over_30ms=%d (%.0f%%)\n", label, NR,
        value[int((NR + 1) / 2)], value[int(NR * 0.95 + 0.5)], over, 100 * over / NR
    }'
}

# Usage: target_summary <label>. Typing and backspace over all rounds, typing per round, and
# keylat's missed= lines.
target_summary() {
  local raw=("$out/$1"-<1-3>-raw.csv(N)) results=("$out/$1"-<1-3>.out(N)) run
  typing_stats 2 "$1 typing" "${raw[@]}"
  typing_stats 4 "$1 backspace" "${raw[@]}"
  for run in 1 2 3; do
    typing_stats 2 "  run $run typing" "$out/$1-$run-raw.csv"(N)
  done
  ((${#results} > 0)) && grep -h 'missed=' "${results[@]}"
}

{
  echo "plain build modified $(stat -f '%Sm' "$PLAIN_BIN" 2> /dev/null || echo 'unknown, no build')"
  echo "targets: ${targets[*]}"
  for line in "${skipped[@]}"; do
    echo "$line"
  done
  for label in "${targets[@]}"; do
    note=$(bin_note "$label")
    target_summary "$label" | while IFS= read -r line; do
      print -r -- "$line$note"
    done
  done
  [ -f "$out/notes.txt" ] && cat "$out/notes.txt"
  cat "$out/throughput.txt"
  cat "$out/idle.txt"
} > "$out/summary.txt"
