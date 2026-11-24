#!/bin/sh

die() {
    printf 'clashpt uninstaller: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Usage: sh uninstall.sh [-y|--yes] [--prefix DIR] [--data-dir DIR] [--system]

Remove ClashPT binaries, shell integration, settings, and saved profiles.
  -y, --yes       Remove ClashPT and its installed Mihomo binary without prompting.
  --prefix DIR    Installation prefix (default: ~/.local).
  --data-dir DIR  Also clean a custom runtime directory (default: CLASHPT_DATA_DIR).
  --system        Also remove the systemd deployment and its system data.

Without -y, choose to remove ClashPT only, remove both, or cancel.
EOF
}

stop_user_service() {
    [ -f "$1/process.json" ] || return 0
    [ -n "$clashpt_binary" ] || die "cannot stop the Mihomo process recorded in $1 without ClashPT"
    if ! "$clashpt_binary" --data-dir "$1" stop >/dev/null && [ -f "$1/process.json" ]; then
        die "could not stop the Mihomo process recorded in $1"
    fi
}

clean_custom_data() {
    directory=$1
    if [ ! -f "$directory/profile.json" ] && [ ! -f "$directory/process.json" ] &&
        [ ! -f "$directory/operation.lock" ]; then
        if [ -d "$directory/core" ]; then
            rmdir -- "$directory/core" 2>/dev/null || :
        fi
    else
        for profile in "$directory"/profile-*; do
            [ -f "$profile/config.yaml" ] || [ -f "$profile/source.yaml" ] || continue
            rm -rf -- "$profile"
        done
        rm -rf -- "$directory/core"
        rm -f -- "$directory/mihomo.log" "$directory/validate.log" \
            "$directory/profile.json" "$directory/process.json" "$directory/operation.lock"
    fi
    if [ -d "$directory" ] && [ ! -L "$directory" ]; then
        rmdir --ignore-fail-on-non-empty -- "$directory"
    fi
}

clean_system() {
    # Keep the installed Mihomo copy when the user selects ClashPT only.
    if [ "$(id -u)" -eq 0 ]; then
        set -- /bin/sh
    elif [ "$assume_yes" = yes ]; then
        set -- sudo -n -- /bin/sh
    else
        set -- sudo -- /bin/sh
    fi
    "$@" -eu -c '
        unit_dir=/etc/systemd/system
        if [ -d /run/systemd/system ]; then
            if [ -f "$unit_dir/clashpt.service" ] || [ -L "$unit_dir/multi-user.target.wants/clashpt.service" ]; then
                /usr/bin/systemctl disable --now clashpt.service
            fi
            if [ -f "$unit_dir/clashpt-api.service" ]; then
                /usr/bin/systemctl stop clashpt-api.service
            fi
        fi
        rm -f -- "$unit_dir/clashpt.service" "$unit_dir/clashpt-api.service" \
            "$unit_dir/multi-user.target.wants/clashpt.service"
        rm -rf -- /var/lib/clashpt /var/lib/private/clashpt /run/clashpt-core
        if [ "$1" = yes ]; then
            rm -rf -- /etc/clashpt
        elif [ -d /etc/clashpt ]; then
            rm -f -- /etc/clashpt/clashpt /etc/clashpt/config.yaml /etc/clashpt/client.json \
                /etc/clashpt/server.json /etc/clashpt/apply.lock
            rmdir --ignore-fail-on-non-empty -- /etc/clashpt
        fi
        if [ -d /run/systemd/system ]; then
            /usr/bin/systemctl daemon-reload
        fi
    ' clashpt-system-uninstall "$remove_mihomo" || die 'system uninstall failed (administrator access required)'
}

main() {
    set -eu
    prefix=${HOME:?HOME is required}/.local
    data_dir=${CLASHPT_DATA_DIR:-}
    assume_yes=no
    system=no

    while [ "$#" -gt 0 ]; do
        case "$1" in
            -y|--yes) assume_yes=yes; shift ;;
            --system) system=yes; shift ;;
            --prefix|--data-dir)
                [ "$#" -ge 2 ] && [ -n "$2" ] || die "$1 requires a directory"
                case "$1" in --prefix) prefix=$2 ;; --data-dir) data_dir=$2 ;; esac
                shift 2 ;;
            -h|--help) usage; return ;;
            *) die "unknown option $1" ;;
        esac
    done

    case "${XDG_CONFIG_HOME:-}" in /*) settings_dir=$XDG_CONFIG_HOME/clashpt ;; *) settings_dir=$HOME/.config/clashpt ;; esac
    settings_dir=$(realpath -ms -- "$settings_dir")
    legacy_dir=$(realpath -ms -- "$HOME/.local/state/clashpt")
    prefix=$(realpath -ms -- "$prefix")
    [ "$prefix" != / ] || die 'the installation prefix cannot be /'
    if [ -n "$data_dir" ]; then
        data_dir=$(realpath -ms -- "$data_dir")
        [ "$data_dir" != / ] || die 'the custom runtime directory cannot be /'
    fi
    bindir=$prefix/bin
    share=$prefix/share/clashpt

    printf 'ClashPT files to remove:\n  %s\n  %s\n  %s\n' "$bindir/clashpt" "$share" "$settings_dir"
    [ ! -e "$legacy_dir" ] || printf '  %s\n' "$legacy_dir"
    [ -z "$data_dir" ] || [ "$data_dir" = "$settings_dir" ] || [ "$data_dir" = "$legacy_dir" ] ||
        printf '  ClashPT data in %s\n' "$data_dir"
    [ "$system" = no ] || printf '  ClashPT systemd units and system data\n'
    printf 'Mihomo binary to remove when selected:\n  %s\n' "$bindir/mihomo"
    [ "$system" = no ] || printf '  /etc/clashpt/mihomo\n'

    remove_mihomo=yes
    if [ "$assume_yes" = no ]; then
        printf '%s' 'Remove both [y], ClashPT only [c], or cancel [N]? '
        IFS= read -r answer || answer=
        case "$answer" in
            y|Y|yes|YES) ;;
            c|C) remove_mihomo=no ;;
            *) printf 'Uninstall cancelled.\n'; return ;;
        esac
    fi

    if [ -x "$bindir/clashpt" ]; then
        clashpt_binary=$bindir/clashpt
    else
        clashpt_binary=$(command -v clashpt) || clashpt_binary=
    fi
    stop_user_service "$settings_dir"
    stop_user_service "$legacy_dir"
    if [ "$system" = yes ]; then
        clean_system
    fi
    if [ -n "$data_dir" ] && [ "$data_dir" != "$settings_dir" ] && [ "$data_dir" != "$legacy_dir" ]; then
        stop_user_service "$data_dir"
        clean_custom_data "$data_dir"
    fi
    rm -f -- "$bindir/clashpt"
    if [ "$remove_mihomo" = yes ]; then
        rm -f -- "$bindir/mihomo"
    fi
    rm -rf -- "$share" "$settings_dir" "$legacy_dir"
    printf 'Uninstall complete.\n'
}

main "$@"
