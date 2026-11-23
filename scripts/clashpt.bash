# Source this file from .bashrc to apply ClashPT proxy settings in the current shell.
_clashpt_project_dir=$(CDPATH= cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P) || return
_clashpt_release_binary="$_clashpt_project_dir/target/release/clashpt"
unset _clashpt_project_dir

_clashpt_exec() {
    if type -P clashpt >/dev/null; then
        command clashpt "$@"
    elif [[ -x "$_clashpt_release_binary" ]]; then
        "$_clashpt_release_binary" "$@"
    else
        printf '%s\n' 'clashpt: executable not found in PATH or target/release' >&2
        return 127
    fi
}

_clashpt_set_proxy() {
    local shell_code
    shell_code=$(_clashpt_exec "$@" --shell) || return
    builtin eval "$shell_code"
}

_clashpt_apply_proxy() {
    _clashpt_set_proxy "$@" || return
    _clashpt_exec "$@" --applied
}

_clashpt_proxy_matches() (
    [[ -n "$1" ]] || return 1
    # Evaluate in a subshell to leave the current shell's proxy settings untouched.
    local -a previous=("${HTTP_PROXY-}" "${HTTPS_PROXY-}" "${ALL_PROXY-}"
        "${http_proxy-}" "${https_proxy-}" "${all_proxy-}")
    builtin eval "$1" || return
    [[ "${HTTP_PROXY-}" == "${previous[0]}" && "${HTTPS_PROXY-}" == "${previous[1]}"
        && "${ALL_PROXY-}" == "${previous[2]}" && "${http_proxy-}" == "${previous[3]}"
        && "${https_proxy-}" == "${previous[4]}" && "${all_proxy-}" == "${previous[5]}" ]]
)

_clashpt_sync_proxy() {
    local previous="$1" current state
    shift
    if current=$(_clashpt_exec "$@" proxy on --shell 2>/dev/null); then
        if [[ "$current" != "$previous" ]]; then
            builtin eval "$current"
        fi
    else
        state=$(_clashpt_exec "$@" status --json) || return
        if [[ "$state" == *'"proxy":null'* ]]; then
            _clashpt_set_proxy "$@" proxy off
        else
            _clashpt_set_proxy "$@" proxy on
        fi
    fi
}

clashpt() {
    local -a args=("$@") options=()
    local subcommand='' action='' previous='' index

    for (( index = 0; index < ${#args[@]}; index++ )); do
        case "${args[index]}" in
            --data-dir|--mihomo)
                options+=("${args[index]}")
                index=$((index + 1))
                if (( index < ${#args[@]} )); then
                    options+=("${args[index]}")
                fi
                ;;
            --system|--data-dir=*|--mihomo=*) options+=("${args[index]}") ;;
            --config) index=$((index + 1)) ;;
            -h|--help|-V|--version|--shell|--applied)
                _clashpt_exec "${args[@]}"
                return
                ;;
            --) break ;;
            --*) ;;
            *)
                if [[ -z "$subcommand" ]]; then
                    subcommand="${args[index]}"
                elif [[ ( "$subcommand" == proxy || "$subcommand" == profile ) && -z "$action" ]]; then
                    action="${args[index]}"
                fi
                ;;
        esac
    done

    case "$subcommand" in
        start|stop)
            _clashpt_exec "${args[@]}" >/dev/null || return
            if [[ "$subcommand" == start ]]; then
                action=on
            else
                action=off
            fi
            _clashpt_set_proxy "${options[@]}" proxy "$action" || return
            _clashpt_exec "${options[@]}" status
            ;;
        proxy)
            if [[ "$action" == on || "$action" == off ]]; then
                _clashpt_apply_proxy "${args[@]}"
            else
                _clashpt_exec "${args[@]}"
            fi
            ;;
        tui|profile)
            if [[ "$subcommand" == profile && "$action" != use ]]; then
                _clashpt_exec "${args[@]}"
                return
            fi
            previous=$(_clashpt_exec "${options[@]}" proxy on --shell 2>/dev/null) || previous=''
            if ! _clashpt_proxy_matches "$previous"; then
                previous=''
            fi
            _clashpt_exec "${args[@]}" || return
            if [[ -n "$previous" ]]; then
                _clashpt_sync_proxy "$previous" "${options[@]}"
            fi
            ;;
        *) _clashpt_exec "${args[@]}" ;;
    esac
}

clash() {
    clashpt "$@"
}
