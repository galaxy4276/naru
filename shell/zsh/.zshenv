# Naru keeps ZDOTDIR on this folder until .zlogin so .zshrc can add its hooks after the user's.
_naru_zdotdir=$ZDOTDIR
# A screen replays the pane's output at its own width; zsh's partial-line mark, drawn for the
# width the shell had then, would be left behind as a stray `%`. Set before the user's files so
# their own mark still wins.
PROMPT_EOL_MARK=''
ZDOTDIR=${NARU_USER_ZDOTDIR:-$HOME}
[[ -f $ZDOTDIR/.zshenv ]] && builtin source $ZDOTDIR/.zshenv
NARU_USER_ZDOTDIR=$ZDOTDIR
ZDOTDIR=$_naru_zdotdir
