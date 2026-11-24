#!/bin/sh

die() {
    printf 'clashpt uninstaller: %s\n' "$*" >&2
    exit 1
}

trusted_path() {
    # Ordinary user installs may use shared writable directories.
    checked=$1
    first=yes
    while :; do
        [ ! -L "$checked" ] || die "symbolic link in installation path: $checked"
        owner=$(stat -c %u -- "$checked") || die "cannot inspect $checked"
        [ "$owner" = 0 ] || [ "$owner" = "$(id -u)" ] || die "untrusted owner: $checked"
        mode=$(stat -c %a -- "$checked")
        if { [ ! -d "$checked" ] || [ "$(id -u)" = 0 ]; } && [ $((0$mode & 022)) -ne 0 ]; then
            [ "$first" = no ] && [ -d "$checked" ] && [ $((0$mode & 01000)) -ne 0 ] ||
                die "writable installation path: $checked"
        fi
        [ "$checked" != / ] || break
        checked=$(dirname -- "$checked")
        first=no
    done
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
    settings_dir=$(realpath -ms -- "$settings_dir")
    prefix=$(realpath -ms -- "$prefix")
    [ "$prefix" != / ] || die 'the installation prefix cannot be /'
    if [ -n "$data_dir" ]; then
        data_dir=$(realpath -ms -- "$data_dir")
        [ "$data_dir" != / ] || die 'the custom runtime directory cannot be /'
    fi
    bindir=$prefix/bin
    share=$prefix/share/clashpt

    [ -d "$prefix" ] && [ "$(realpath -e -- "$prefix")" = "$prefix" ] || die 'unknown installation prefix'
    trusted_path "$prefix"
    exec 9<"$prefix"
    flock -x 9
    receipt=$share/install.receipt
    for dir in "$bindir" "$prefix/share" "$share" "$share/shell"; do
        [ ! -L "$dir" ] || die "refusing symbolic link: $dir"
    done
    [ -f "$receipt" ] && [ ! -L "$receipt" ] && [ "$(sed -n '1p' "$receipt")" = "$prefix" ] ||
        die "invalid or mismatched installation receipt: $receipt"
    trusted_path "$receipt"
    trusted_path "$bindir"

    printf 'ClashPT files to remove:\n  %s\n  %s\n  %s\n' "$bindir/clashpt" "$share" "$settings_dir"
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

    owned() {
        [ -f "$1" ] && [ ! -L "$1" ] || return 1
        sum=$(sha256sum < "$1") || return 1
        grep -Fxq -- "${sum%% *}  $2" "$receipt"
    }
    for directory in "$settings_dir" "$data_dir"; do
        [ -n "$directory" ] && [ -d "$directory" ] || continue
        [ "$(realpath -e -- "$directory")" = "$directory" ] || die 'refusing aliased runtime directory'
        if [ ! -f "$directory/.clashpt-runtime" ]; then
            if [ "$directory" = "$settings_dir" ] && [ -e "$directory/process.json" ]; then
                die "stop the unregistered instance in $directory before uninstalling"
            fi
            continue
        fi
        owned "$bindir/clashpt" bin/clashpt || die 'cannot clean state with an unverified manager'
        trusted_path "$bindir/clashpt"
        trusted_path "$directory"
        "$bindir/clashpt" --data-dir "$directory" uninstall-runtime
    done
    if [ "$system" = yes ]; then clean_system; fi
    for rel in bin/clashpt bin/mihomo share/clashpt/shell/clashpt.bash share/clashpt/shell/clashpt.zsh; do
        if [ "$rel" = bin/mihomo ] && [ "$remove_mihomo" = no ]; then continue; fi
        if owned "$prefix/$rel" "$rel"; then rm -- "$prefix/$rel"; fi
    done
    rm -rf -- "$settings_dir" "$share"
    rmdir --ignore-fail-on-non-empty -- "$prefix/share"
    printf 'Uninstall complete.\n'
}

main "$@"
