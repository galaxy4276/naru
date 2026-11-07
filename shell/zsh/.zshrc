ZDOTDIR=$NARU_USER_ZDOTDIR
[[ -f $ZDOTDIR/.zshrc ]] && builtin source $ZDOTDIR/.zshrc
ZDOTDIR=$_naru_zdotdir
builtin source $_naru_zdotdir/naru.zsh
