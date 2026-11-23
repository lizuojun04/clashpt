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

function _clashpt_apply_proxy {
    local shell_code
    shell_code=$(_clashpt_exec "$@" --shell) || return
    builtin eval "$shell_code" || return
    _clashpt_exec "$@" --applied
}

function clashpt {
    emulate -L zsh
    local -a args=("$@")
    local -a proxy_args=()
    local subcommand='' action=''
    local -i index command_index=0

    for (( index = 1; index <= ${#args}; index++ )); do
        case "${args[index]}" in
            --data-dir|--mihomo) (( index++ )) ;;
            --system|--data-dir=*|--mihomo=*) ;;
            -h|--help|-V|--version|--shell|--applied)
                _clashpt_exec "${args[@]}"
                return
                ;;
            --*) ;;
            *)
                if [[ -z "$subcommand" ]]; then
                    subcommand="${args[index]}"
                    command_index=$index
                elif [[ "$subcommand" == proxy && -z "$action" ]]; then
                    action="${args[index]}"
                fi
                ;;
        esac
    done

    case "$subcommand" in
        start|stop)
            _clashpt_exec "${args[@]}" || return
            if [[ "$subcommand" == start ]]; then
                action=on
            else
                action=off
            fi
            for (( index = 1; index <= ${#args}; index++ )); do
                if (( index != command_index )); then
                    proxy_args+=("${args[index]}")
                fi
            done
            _clashpt_apply_proxy "${proxy_args[@]}" proxy "$action"
            ;;
        proxy)
            if [[ "$action" == on || "$action" == off ]]; then
                _clashpt_apply_proxy "${args[@]}"
            else
                _clashpt_exec "${args[@]}"
            fi
            ;;
        *) _clashpt_exec "${args[@]}" ;;
    esac
}

function clash {
    clashpt "$@"
}
