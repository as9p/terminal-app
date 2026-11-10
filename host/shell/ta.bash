# Written by the terminal app, and run by bash in place of ~/.bashrc. It loads the same files
# a login shell would, then reports the current folder to the app each time a prompt is drawn,
# as ESC ] 7 ; file://<host><percent-encoded path> BEL.

if [ -f /etc/profile ]; then
  . /etc/profile
fi

for _ta_profile in ~/.bash_profile ~/.bash_login ~/.profile; do
  if [ -f "$_ta_profile" ]; then
    . "$_ta_profile"
    break
  fi
done
unset _ta_profile

_ta_report_cwd() {
  # Kept and handed back, so a command after this one still sees the last status.
  local last=$?
  # Byte by byte, so a non-ASCII name is encoded as its UTF-8 bytes.
  local LC_ALL=C
  local path=$PWD encoded='' char code i
  for ((i = 0; i < ${#path}; i++)); do
    char=${path:i:1}
    case $char in
      [A-Za-z0-9/._~-]) encoded+=$char ;;
      *)
        # Masked, because bash 3.2 reads a byte over 127 as a negative number.
        printf -v code '%d' "'$char"
        printf -v char '%%%02X' $((code & 255))
        encoded+=$char
        ;;
    esac
  done
  printf '\e]7;file://%s%s\a' "$HOSTNAME" "$encoded"
  return $last
}

PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND
}_ta_report_cwd"

# Claude's subcommands, from `claude --help`. A call that starts with one gets nothing added.
_ta_claude_subcommands=(agents attach auth auto-mode doctor gateway import install logs mcp
  plugin plugins purge respawn rm setup-token stop kill ultrareview update upgrade)

# Runs the real claude with the app's hooks file, then each line of the app's flags file that
# was not typed already, then the typed arguments. Both files are read at each call, so a
# changed setting applies in open tabs. `function` keeps a user alias from breaking this.
function claude {
  local added=() line typed
  for line in "${_ta_claude_subcommands[@]}"; do
    if [ "$line" = "$1" ]; then
      command claude "$@"
      return
    fi
  done
  if [ -z "$TA_SESSION" ]; then
    command claude "$@"
    return
  fi
  if [ -f "$TA_DATA/claude-hooks.json" ]; then
    added=(--settings "$TA_DATA/claude-hooks.json")
  fi
  if [ -f "$TA_DATA/claude-args" ]; then
    while IFS= read -r line || [ -n "$line" ]; do
      [ -n "$line" ] || continue
      for typed in "$@"; do
        [ "$typed" = "$line" ] && continue 2
      done
      added+=("$line")
    done < "$TA_DATA/claude-args"
  fi
  command claude ${added[@]+"${added[@]}"} "$@"
}
