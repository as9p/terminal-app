# Written by the terminal app. zsh reads this first because the app points ZDOTDIR here.
# It puts ZDOTDIR back as it was, so the rest of the startup files load from their usual
# place, then loads the user's own .zshenv and the hook.

if [[ -n ${TA_USER_ZDOTDIR+set} ]]; then
  ZDOTDIR=$TA_USER_ZDOTDIR
  unset TA_USER_ZDOTDIR
else
  unset ZDOTDIR
fi

if [[ -f ${ZDOTDIR:-$HOME}/.zshenv ]]; then
  source "${ZDOTDIR:-$HOME}/.zshenv"
fi

# Only an interactive shell draws prompts.
if [[ -o interactive && -n $TA_DATA && -f $TA_DATA/shell/ta.zsh ]]; then
  source "$TA_DATA/shell/ta.zsh"
fi
