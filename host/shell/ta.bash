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
