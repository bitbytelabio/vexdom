# bash completion for vexdom. Load it in ~/.bashrc with:
#   eval "$(vexdom completions bash)"
# (bash 3.2, the macOS default, cannot `source <(…)`.)
_vexdom() {
    local cur cword words=() line directive=
    if declare -F _get_comp_words_by_ref >/dev/null 2>&1; then
        _get_comp_words_by_ref -n =: cur words cword
    else
        cur=${COMP_WORDS[COMP_CWORD]}
        words=("${COMP_WORDS[@]}")
        cword=$COMP_CWORD
    fi
    COMPREPLY=()
    while IFS= read -r line; do
        if [[ -z $directive ]]; then
            directive=$line
            continue
        fi
        COMPREPLY+=("${line%%$'\t'*}")
    done < <("${words[0]}" __complete "$((cword - 1))" "${words[@]:1:cword-1}" "$cur" 2>/dev/null)
    case $directive in
        :files | :dirs)
            local kind=-f
            [[ $directive == :dirs ]] && kind=-d
            while IFS= read -r line; do
                COMPREPLY+=("$line")
            done < <(compgen "$kind" -- "$cur")
            if [[ $(type -t compopt) == builtin ]]; then
                compopt -o filenames
            fi
            ;;
        *)
            if declare -F __ltrim_colon_completions >/dev/null 2>&1; then
                __ltrim_colon_completions "$cur"
            fi
            ;;
    esac
}
complete -F _vexdom vexdom
