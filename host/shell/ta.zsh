# Written by the terminal app. Reports the current folder to the app each time a prompt is
# drawn, as ESC ] 7 ; file://<host><percent-encoded path> BEL.

autoload -Uz add-zsh-hook

_ta_report_cwd() {
  # Kept and handed back, so a precmd function after this one still sees the last status.
  local last=$?
  emulate -L zsh
  # Byte by byte, so a non-ASCII name is encoded as its UTF-8 bytes.
  setopt extendedglob nomultibyte
  local encoded=${PWD//(#m)[^A-Za-z0-9\/._~-]/%${(l:2::0:)$(( [##16] #MATCH ))}}
  printf '\e]7;file://%s%s\a' "$HOST" "$encoded"
  return $last
}

add-zsh-hook precmd _ta_report_cwd
