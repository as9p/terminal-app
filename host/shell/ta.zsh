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

# Claude's subcommands, from `claude --help`. A call that starts with one gets nothing added.
_ta_claude_subcommands=(agents attach auth auto-mode doctor gateway import install logs mcp
  plugin plugins purge respawn rm setup-token stop kill ultrareview update upgrade)

# Runs the real claude with the app's hooks file, then each line of the app's flags file that
# was not typed already, then the typed arguments. Both files are read at each call, so a
# changed setting applies in open tabs. `function` keeps a user alias from breaking this.
function claude {
  if [[ -z $TA_SESSION || ${_ta_claude_subcommands[(Ie)$1]} -ne 0 ]]; then
    command claude "$@"
    return
  fi
  local -a added typed
  local line
  typed=("$@")
  if [[ -f $TA_DATA/claude-hooks.json ]]; then
    added=(--settings "$TA_DATA/claude-hooks.json")
  fi
  if [[ -f $TA_DATA/claude-args ]]; then
    while IFS= read -r line || [[ -n $line ]]; do
      if [[ -n $line && ${typed[(Ie)$line]} -eq 0 ]]; then
        added+=("$line")
      fi
    done < "$TA_DATA/claude-args"
  fi
  command claude "${added[@]}" "$@"
}
