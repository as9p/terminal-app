#!/bin/bash
# Prints a reproducible burst of mixed text (plain, colored, long wrapped lines,
# box drawing and CJK). Usage: ./gen-output.sh [size in MB, default 100]
#        ./gen-output.sh --prepare [MB]   builds the file and prints its checksum
# The data is cached and then printed with cat, so `time` measures the terminal
# and not this generator. The two OSC 7777 markers drive the in-page throughput
# readout. Terminals that do not know that sequence ignore it, kitty included.
set -euo pipefail
export LC_ALL=C
dir="$(cd "$(dirname "$0")" && pwd)"
prepare=0
if [ "${1:-}" = "--prepare" ]; then prepare=1; shift; fi
mb="${1:-100}"
file="$dir/.cache/burst-$mb.txt"

if [ ! -f "$file" ]; then
  mkdir -p "$dir/.cache"
  block="$dir/.cache/block.txt"
  if [ ! -f "$block" ]; then
    # One MiB block from a fixed seed, repeated to the requested size.
    awk 'BEGIN {
      srand(42); bytes = 0; n = 0
      words = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor"
      nw = split(words, w, " ")
      while (bytes < 1048576) {
        kind = n % 5; line = ""
        if (kind == 0) { for (i = 0; i < 12; i++) line = line w[int(rand() * nw) + 1] " " }
        else if (kind == 1) { for (i = 0; i < 10; i++) line = line sprintf("\033[38;5;%dm%s\033[0m ", int(rand() * 256), w[int(rand() * nw) + 1]) }
        else if (kind == 2) { for (i = 0; i < 60; i++) line = line sprintf("%08x", int(rand() * 2147483647)) }
        else if (kind == 3) { for (i = 0; i < 20; i++) line = line "\342\224\200\342\224\202" }
        else { for (i = 0; i < 20; i++) line = line "\346\274\242\345\255\227 " }
        print line; bytes += length(line) + 1; n++
      }
    }' > "$block"
  fi
  for ((i = 0; i < mb; i++)); do cat "$block"; done > "$file"
fi

if [ "$prepare" = 1 ]; then
  shasum -a 256 "$file"
  wc -c "$file"
  exit 0
fi

printf '\033]7777;burst-start\007'
cat "$file"
printf '\033]7777;burst-end\007'
