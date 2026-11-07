# Prompt marks (OSC 133) and the folder (OSC 7) for Naru's daemon.
_naru_precmd() {
  local code=$?
  (( ${+_naru_ran} )) && builtin printf '\e]133;D;%s\a' $code
  unset _naru_ran
  builtin printf '\e]7;file://%s%s\a' $HOST ${PWD//\%/%25}
  builtin printf '\e]133;A\a'
}
_naru_preexec() {
  _naru_ran=1
  builtin printf '\e]133;C\a'
  # Control characters would end the sequence early.
  builtin printf '\e]633;E;%s\a' "${1//[[:cntrl:]]/ }"
}
# The line editor is reading: input written now is not echoed ahead of the prompt.
_naru_line_init() {
  builtin printf '\e]133;B\a'
}
autoload -Uz add-zsh-hook add-zle-hook-widget
add-zsh-hook precmd _naru_precmd
add-zsh-hook preexec _naru_preexec
add-zle-hook-widget line-init _naru_line_init
