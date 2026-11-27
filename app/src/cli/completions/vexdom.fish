# fish completion for vexdom. Load it with:
#   vexdom completions fish | source
# or save it as ~/.config/fish/completions/vexdom.fish
function __vexdom_complete
    set -l words (commandline -opc)
    set -l exe $words[1]
    set -e words[1]
    set -l current (commandline -ct)
    set -l out ($exe __complete (count $words) $words "$current" 2>/dev/null)
    set -l directive $out[1]
    set -e out[1]
    switch "$directive"
        case :files
            __fish_complete_path "$current"
        case :dirs
            __fish_complete_directories "$current"
        case '*'
            printf '%s\n' $out
    end
end
complete -c vexdom -e
complete -c vexdom -f -a '(__vexdom_complete)'
