# Source this file from .zshrc to apply ClashPT proxy settings in the current shell.
typeset -g _clashpt_release_binary="${${(%):-%x}:A:h:h}/target/release/clashpt"

function _clashpt_exec {
    if (( $+commands[clashpt] )); then
        command clashpt "$@"
    elif [[ -x "$_clashpt_release_binary" ]]; then
        "$_clashpt_release_binary" "$@"
    else
        print -u2 'clashpt: executable not found in PATH or target/release'
        return 127
    fi
}

function _clashpt_set_proxy {
    local shell_code
    shell_code=$(_clashpt_exec "$@" --shell) || return
    builtin eval "$shell_code"
}

function _clashpt_apply_proxy {
    _clashpt_set_proxy "$@" || return
    _clashpt_exec "$@" --applied
}

function _clashpt_proxy_matches {
    [[ -n "$1" ]] || return 1
    # Compare in a subshell so checking ownership leaves the current shell untouched.
    (
        local -a previous=("${HTTP_PROXY-}" "${HTTPS_PROXY-}" "${ALL_PROXY-}"
            "${http_proxy-}" "${https_proxy-}" "${all_proxy-}")
        builtin eval "$1" || return
        [[ "${HTTP_PROXY-}" == "${previous[1]}" && "${HTTPS_PROXY-}" == "${previous[2]}"
            && "${ALL_PROXY-}" == "${previous[3]}" && "${http_proxy-}" == "${previous[4]}"
            && "${https_proxy-}" == "${previous[5]}" && "${all_proxy-}" == "${previous[6]}" ]]
    )
}

function _clashpt_sync_proxy {
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

function clashpt {
    emulate -L zsh
    local -a args=("$@")
    local -a options=()
    local subcommand='' action='' previous=''
    local -i index

    for (( index = 1; index <= ${#args}; index++ )); do
        case "${args[index]}" in
            --data-dir|--mihomo)
                options+=("${args[index]}")
                (( index++ ))
                if (( index <= ${#args} )); then
                    options+=("${args[index]}")
                fi
                ;;
            --system|--data-dir=*|--mihomo=*) options+=("${args[index]}") ;;
            --config) (( index++ )) ;;
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

function clash {
    clashpt "$@"
}
