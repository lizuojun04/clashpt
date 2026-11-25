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

clean_system() {
    set -- /usr/local/bin/clashpt system remove
    if [ "$remove_mihomo" = no ]; then set -- "$@" --keep-mihomo; fi
    if [ "$(id -u)" -eq 0 ]; then "$@"
    elif [ "$assume_yes" = yes ]; then sudo -n -- "$@"
    else sudo -- "$@"; fi

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
    legacy_dir=$HOME/.local/state/clashpt
    prefix=$(realpath -m -- "$prefix")
    [ "$prefix" != / ] || die 'the installation prefix cannot be /'
    if [ -n "$data_dir" ]; then
        data_dir=$(realpath -m -- "$data_dir")
        [ "$data_dir" != / ] || die 'the custom runtime directory cannot be /'
    fi
    bindir=$prefix/bin
    share=$prefix/share/clashpt

    if [ -d "$prefix" ]; then
        exec 9<"$prefix"
        flock -x 9
    fi

    printf 'ClashPT files to remove:\n  %s\n  %s\n  %s\n' "$bindir/clashpt" "$share" "$settings_dir"
    [ ! -e "$legacy_dir" ] || printf '  %s\n' "$legacy_dir"
    [ -z "$data_dir" ] || [ "$data_dir" = "$settings_dir" ] ||
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

    manager=$bindir/clashpt
    bundled_manager=$(dirname -- "$0")/clashpt
    if [ -x "$bundled_manager" ]; then manager=$bundled_manager; fi
    for directory in "$settings_dir" "$legacy_dir" "$data_dir"; do
        [ -n "$directory" ] && [ -d "$directory" ] || continue
        if [ -f "$directory/process.json" ]; then
            "$manager" --data-dir "$directory" stop || die "could not stop the recorded instance in $directory"
        fi
    done
    if [ -n "$data_dir" ] && [ -f "$data_dir/profile.json" ] &&
        [ "$data_dir" != "$(realpath -m -- "$settings_dir")" ] &&
        [ "$data_dir" != "$(realpath -m -- "$legacy_dir")" ]; then
        "$manager" --data-dir "$data_dir" uninstall-runtime || die 'could not clean custom data directory'
    fi
    if [ "$system" = yes ]; then clean_system; fi
    rm -f -- "$bindir/clashpt"
    if [ "$remove_mihomo" = yes ]; then rm -f -- "$bindir/mihomo"; fi
    rm -rf -- "$settings_dir" "$legacy_dir" "$share"
    if [ -d "$prefix/share" ]; then rmdir --ignore-fail-on-non-empty -- "$prefix/share"; fi
    printf 'Uninstall complete.\n'
}

main "$@"
