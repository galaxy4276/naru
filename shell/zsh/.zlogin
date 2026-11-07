ZDOTDIR=$NARU_USER_ZDOTDIR
[[ -f $ZDOTDIR/.zlogin ]] && builtin source $ZDOTDIR/.zlogin
unset _naru_zdotdir NARU_USER_ZDOTDIR
