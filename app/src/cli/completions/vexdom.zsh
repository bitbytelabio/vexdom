#compdef vexdom
# zsh completion for vexdom. Load it in ~/.zshrc, after compinit, with:
#   source <(vexdom completions zsh)
# or save it as _vexdom in a directory on $fpath.
_vexdom() {
  local -a lines candidates
  local line value
  lines=("${(@f)$("${words[1]}" __complete $((CURRENT - 2)) "${(@)words[2,CURRENT]}" 2>/dev/null)}")
  case ${lines[1]} in
    :files) _files ;;
    :dirs) _files -/ ;;
    *)
      for line in "${(@)lines[2,-1]}"; do
        [[ -n $line ]] || continue
        value=${line%%$'\t'*}
        value=${value//:/\\:}
        if [[ $line == *$'\t'* ]]; then
          candidates+=("$value:${line#*$'\t'}")
        else
          candidates+=("$value")
        fi
      done
      (( ${#candidates} )) && _describe -t values vexdom candidates
      ;;
  esac
}
if [[ ${funcstack[1]} == _vexdom ]]; then
  _vexdom "$@"
else
  compdef _vexdom vexdom
fi
