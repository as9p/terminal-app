#!/bin/zsh
# Checks one sitting's summary.txt from plain-compare.sh against the stage's speed rules. Prints
# one PASS or FAIL line per rule and exits 1 on any FAIL. It opens nothing, so it can run anywhere.
# Usage: plain-targets.sh <summary.txt> [plain label, default plain-fifo]
#
# "The others" are kitty, Ghostty and Alacritty. Warp runs on its own settings, so it never
# decides a rule; when the summary has it, a "beside warp" line follows the rules.

[ -f "${1:-}" ] || { echo "usage: plain-targets.sh <summary.txt> [plain label, default plain-fifo]"; exit 2; }

awk -v plain="${2:-plain-fifo}" '
  BEGIN { split("kitty ghostty alacritty", others, " "); fails = 0 }

  # <label> typing: samples=.. median=.. p95=.. over_30ms=..
  /^[a-z0-9-]+ (typing|backspace): samples=/ {
    kind = $2
    sub(/:$/, "", kind)
    for (i = 3; i <= NF; i++) {
      if (split($i, kv, "=") != 2) continue
      if (kv[1] == "median") med[$1, kind] = kv[2] + 0
      if (kv[1] == "p95") p95[$1, kind] = kv[2] + 0
    }
  }

  # <label> <MB> <seconds> <MB/s>, leaving out runs the throughput script said to ignore.
  $2 ~ /^[0-9]+$/ && $3 ~ /^[0-9.]+$/ && $4 ~ /^[0-9.]+$/ && $0 !~ /ignore this run/ {
    speed[$1, ++runs[$1]] = $4 + 0
  }

  /^idle frames in 5 s: / { idle = $6 }

  function verdict(ok, text) {
    if (ok) print "PASS " text
    else { print "FAIL " text; fails++ }
  }

  function output_median(label,   n, i, j, v, t) {
    n = runs[label]
    for (i = 1; i <= n; i++) v[i] = speed[label, i]
    for (i = 2; i <= n; i++)
      for (j = i; j > 1 && v[j - 1] > v[j]; j--) { t = v[j]; v[j] = v[j - 1]; v[j - 1] = t }
    return n % 2 ? v[(n + 1) / 2] : (v[n / 2] + v[n / 2 + 1]) / 2
  }

  # Usage: latency_rule(<rule name>, typing|backspace, median|p95, <ms allowed above the lowest>)
  function latency_rule(name, kind, stat, slack,   k, label, value, mine, best, who) {
    best = ""
    for (k = 1; k <= 3; k++) {
      label = others[k]
      if (stat == "median" && (label, kind) in med) value = med[label, kind]
      else if (stat == "p95" && (label, kind) in p95) value = p95[label, kind]
      else continue
      if (best == "" || value < best) { best = value; who = label }
    }
    if (stat == "median" && (plain, kind) in med) mine = med[plain, kind]
    else if (stat == "p95" && (plain, kind) in p95) mine = p95[plain, kind]
    else { verdict(0, name ": " plain " not measured"); return }
    if (best == "") { verdict(0, name ": none of kitty, ghostty and alacritty measured"); return }
    # The small extra keeps a value exactly at the limit from failing on rounding.
    verdict(mine <= best + slack + 0.000001, sprintf("%s: %s %.1f ms, lowest other %.1f ms (%s), allowed up to %.1f ms", name, plain, mine, best, who, best + slack))
  }

  END {
    latency_rule("typing median", "typing", "median", 1.0)
    latency_rule("backspace median", "backspace", "median", 1.0)
    latency_rule("typing p95", "typing", "p95", 2.0)
    latency_rule("backspace p95", "backspace", "p95", 2.0)

    # The stage defines the output rule against kitty, so without kitty it cannot pass.
    if (!("kitty" in runs)) verdict(0, "output: kitty not measured")
    else if (!(plain in runs)) verdict(0, "output: " plain " not measured")
    else verdict(output_median(plain) >= output_median("kitty"), sprintf("output: %s median %.1f MB/s, kitty median %.1f MB/s", plain, output_median(plain), output_median("kitty")))

    if (idle !~ /^[0-9]+$/) verdict(0, "idle: no frame count in the summary")
    else verdict(idle <= 1, sprintf("idle: %d frames in 5 s, allowed up to 1", idle))

    typing_ratio = "n/a"
    if ((plain, "typing") in med && ("kitty", "typing") in med && med["kitty", "typing"] > 0)
      typing_ratio = sprintf("%.2f", med[plain, "typing"] / med["kitty", "typing"])
    output_ratio = "n/a"
    if (plain in runs && "kitty" in runs && output_median("kitty") > 0)
      output_ratio = sprintf("%.2f", output_median(plain) / output_median("kitty"))
    print "ratio to kitty: typing " typing_ratio " output " output_ratio

    if (("warp", "typing") in med || "warp" in runs) {
      plain_typing = (((plain, "typing") in med) ? sprintf("%.1f", med[plain, "typing"]) : "n/a")
      warp_typing = ((("warp", "typing") in med) ? sprintf("%.1f", med["warp", "typing"]) : "n/a")
      plain_output = ((plain in runs) ? sprintf("%.1f", output_median(plain)) : "n/a")
      warp_output = (("warp" in runs) ? sprintf("%.1f", output_median("warp")) : "n/a")
      print "beside warp: typing " plain_typing " vs " warp_typing ", output " plain_output " vs " warp_output
    }

    exit (fails > 0)
  }
' "$1"
