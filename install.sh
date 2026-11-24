#!/bin/sh

die() {
    printf 'clashpt installer: %s\n' "$*" >&2
    exit 1
}

trusted_path() {
    # Every component must be owned by this user or root. Sticky shared parents
    # (such as /tmp) protect an owned child against replacement by another UID.
    checked=$1
    first=yes
    while :; do
        [ ! -L "$checked" ] || die "symbolic link in installation path: $checked"
        owner=$(stat -c %u -- "$checked") || die "cannot inspect $checked"
        [ "$owner" = 0 ] || [ "$owner" = "$(id -u)" ] || die "untrusted owner: $checked"
        mode=$(stat -c %a -- "$checked")
        if [ $((0$mode & 022)) -ne 0 ]; then
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
Usage: sh install.sh [--prefix DIR] [--version TAG | --from-build FILE] [--gh-proxy URL]

Install or update ClashPT, Mihomo, and Bash/Zsh integration on Linux.
  --prefix DIR       Install binaries in DIR/bin (default: ~/.local/bin).
  --version TAG      Download a specific ClashPT release (default: latest).
  --from-build FILE  Install a locally compiled binary from a source checkout.
  --gh-proxy URL     Fetch GitHub downloads through this HTTPS gateway.

CLASHPT_RELEASE_URL overrides the release download directory.
GH_PROXY supplies the gateway unless overridden by --gh-proxy.
EOF
}

download() {
    command -v curl >/dev/null 2>&1 || die 'curl is required for downloads'
    case "$1" in
        https://github.com/*|https://raw.githubusercontent.com/*)
            if [ -n "$gh_proxy" ]; then
                # Reject redirects so the client never follows a GitHub URL directly.
                curl -q --noproxy '*' -fsSL --max-redirs 0 --retry 3 --output "$2" -- "$gh_proxy/$1" ||
                    die "could not download $1 through $gh_proxy"
                return
            fi ;;
    esac
    curl -fsSL --retry 3 --output "$2" -- "$1" || die "could not download $1"
}

check_sha256() {
    command -v sha256sum >/dev/null 2>&1 || die 'sha256sum is required to verify downloads'
    checksum=$(sha256sum < "$2") || die "could not read $2"
    [ "${checksum%% *}" = "$1" ] || die "SHA-256 mismatch for ${2##*/}"
}

quote_shell() {
    escaped=$(printf '%s' "$1" | sed "s/'/'\\\\''/g")
    printf "'%s'" "$escaped"
}

main() {
    set -eu
    umask 022
    prefix=${HOME:?HOME is required}/.local
    version=latest
    from_build=
    gh_proxy=${GH_PROXY:-}
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --prefix|--version|--from-build)
                [ "$#" -ge 2 ] && [ -n "$2" ] || die "$1 requires a value"
                case "$1" in
                    --prefix) prefix=$2 ;;
                    --version) version=$2 ;;
                    --from-build) from_build=$2 ;;
                esac
                shift 2 ;;
            --gh-proxy)
                [ "$#" -ge 2 ] || die '--gh-proxy requires a value'
                gh_proxy=$2
                shift 2 ;;
            --gh-proxy=*) gh_proxy=${1#*=}; shift ;;
            -h|--help) usage; return ;;
            *) die "unknown option $1" ;;
        esac
    done
    [ -z "$from_build" ] || [ "$version" = latest ] || die 'choose --version or --from-build'
    case "$prefix" in /*) ;; *) prefix=$PWD/$prefix ;; esac
    case "$version" in *[!a-zA-Z0-9._-]*|.|..) die 'invalid release tag' ;; esac
    gh_proxy=${gh_proxy%/}
    case "$gh_proxy" in ''|https://?*) ;; *) die '--gh-proxy requires an HTTPS URL' ;; esac

    [ "$(uname -s)" = Linux ] || die 'only Linux is supported'
    case "$(uname -m)" in
        x86_64) arch=x86_64 ;;
        aarch64|arm64) arch=aarch64 ;;
        *) die "unsupported architecture $(uname -m)" ;;
    esac

    bindir=$prefix/bin
    shelldir=$prefix/share/clashpt/shell
    work=$(mktemp -d) || die 'could not create a temporary directory'
    bin_stage=
    shell_stage=
    trap 'rm -rf -- "$work" "${bin_stage:-$work}" "${shell_stage:-$work}"' EXIT
    trap 'exit 1' HUP INT TERM

    if [ -n "$from_build" ]; then
        case "$from_build" in /*) binary=$from_build ;; *) binary=$PWD/$from_build ;; esac
        script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd) || die 'could not find the source checkout'
        zsh_script=$script_dir/scripts/clashpt.zsh
        bash_script=$script_dir/scripts/clashpt.bash
        mihomo_manifest=$script_dir/scripts/mihomo-release.tsv
        [ -x "$binary" ] && [ -f "$zsh_script" ] && [ -f "$bash_script" ] && [ -f "$mihomo_manifest" ] ||
            die 'run install.sh from a source checkout with --from-build PATH_TO_BINARY'
    else
        releases=https://github.com/lizuojun04/clashpt/releases
        if [ "$version" = latest ]; then
            release_url=$releases/latest/download
        else
            release_url=$releases/download/$version
        fi
        release_url=${CLASHPT_RELEASE_URL:-$release_url}
        archive=clashpt-linux-$arch.tar.gz
        download "${release_url%/}/SHA256SUMS" "$work/SHA256SUMS"
        expected=$(awk -v name="$archive" '$2 == name || $2 == "*" name {print $1; count++} END {if (count != 1) exit 1}' "$work/SHA256SUMS") ||
            die "SHA256SUMS must contain one checksum for $archive"
        download "${release_url%/}/$archive" "$work/$archive"
        check_sha256 "$expected" "$work/$archive"

        binary=$work/clashpt
        zsh_script=$work/clashpt.zsh
        bash_script=$work/clashpt.bash
        mihomo_manifest=$work/mihomo-release.tsv
        for member in clashpt clashpt.zsh clashpt.bash mihomo-release.tsv; do
            tar -xOzf "$work/$archive" "$member" > "$work/$member" || die "could not read $member from $archive"
            [ -s "$work/$member" ] || die "$member is empty"
        done
        chmod 755 "$binary"
    fi
    release=$("$binary" --version) || die 'ClashPT could not run on this machine'

    mihomo_entry=$(awk -v arch="$arch" '
        $1 == arch && NF == 4 && $2 ~ /^v[0-9][a-zA-Z0-9._-]*$/ && $3 ~ /^mihomo-[a-zA-Z0-9._-]+\.gz$/ {
            print $2, $3, $4; count++
        }
        END {if (count != 1) exit 1}
    ' "$mihomo_manifest") || die "no unique Mihomo release for $arch"
    IFS=' ' read -r mihomo_version mihomo_asset mihomo_checksum <<EOF
$mihomo_entry
EOF
    mihomo=$work/mihomo
    download "https://github.com/MetaCubeX/mihomo/releases/download/$mihomo_version/$mihomo_asset" "$work/$mihomo_asset"
    check_sha256 "$mihomo_checksum" "$work/$mihomo_asset"
    gzip -dc -- "$work/$mihomo_asset" > "$mihomo" || die 'could not decompress Mihomo'
    chmod 755 "$mihomo"
    "$mihomo" -v >/dev/null || die 'the downloaded Mihomo could not run on this machine'

    mkdir -p -- "$prefix"
    prefix=$(realpath -e -- "$prefix")
    trusted_path "$prefix"
    exec 9<"$prefix"
    flock -x 9
    bindir=$prefix/bin
    share=$prefix/share/clashpt
    shelldir=$share/shell
    for dir in "$bindir" "$prefix/share" "$share" "$shelldir"; do
        [ ! -L "$dir" ] || die "refusing symbolic link: $dir"
    done
    mkdir -p -- "$bindir" "$shelldir"
    for dir in "$bindir" "$shelldir"; do trusted_path "$dir"; done
    receipt=$share/install.receipt
    if [ -e "$receipt" ] || [ -L "$receipt" ]; then
        trusted_path "$receipt"
        [ ! -L "$receipt" ] && [ "$(sed -n '1p' "$receipt")" = "$prefix" ] ||
            die 'invalid installation receipt'
    fi
    for rel in bin/clashpt bin/mihomo share/clashpt/shell/clashpt.bash share/clashpt/shell/clashpt.zsh; do
        dest=$prefix/$rel
        if [ -e "$dest" ] || [ -L "$dest" ]; then
            [ -f "$dest" ] && [ ! -L "$dest" ] && [ -f "$receipt" ] ||
                die "existing file has no installation receipt: $dest"
            sum=$(sha256sum < "$dest")
            grep -Fxq -- "${sum%% *}  $rel" "$receipt" || die "existing file was replaced: $dest"
        fi
    done
    bin_stage=$(mktemp -d "$bindir/.clashpt-install.XXXXXX")
    shell_stage=$(mktemp -d "$shelldir/.clashpt-install.XXXXXX")
    install -m 755 -- "$binary" "$bin_stage/clashpt"
    install -m 755 -- "$mihomo" "$bin_stage/mihomo"
    install -m 644 -- "$zsh_script" "$bash_script" "$shell_stage/"
    case "${XDG_CONFIG_HOME:-}" in /*) settings=$XDG_CONFIG_HOME/clashpt/clashpt.yaml ;; *) settings=$HOME/.config/clashpt/clashpt.yaml ;; esac
    own_settings=no
    if [ ! -e "$settings" ] && [ ! -L "$settings" ]; then own_settings=yes
    elif [ -f "$receipt" ] && [ -f "$settings" ]; then
        sum=$(sha256sum < "$settings")
        if grep -Fxq -- "${sum%% *}  $settings" "$receipt"; then own_settings=yes; fi
    fi
    "$bin_stage/clashpt" install-settings || die 'could not install ClashPT settings'
    {
        printf '%s\n' "$prefix"
        for name in clashpt mihomo; do
            sum=$(sha256sum < "$bin_stage/$name")
            printf '%s  bin/%s\n' "${sum%% *}" "$name"
        done
        for name in clashpt.zsh clashpt.bash; do
            sum=$(sha256sum < "$shell_stage/$name")
            printf '%s  share/clashpt/shell/%s\n' "${sum%% *}" "$name"
        done
        if [ "$own_settings" = yes ]; then sha256sum -- "$settings"; fi
    } > "$shell_stage/receipt"
    cp -- "$shell_stage/receipt" "$shell_stage/pending"
    if [ -f "$receipt" ]; then tail -n +2 "$receipt" >> "$shell_stage/pending"; fi
    mv -fT -- "$shell_stage/pending" "$receipt"
    mv -fT -- "$bin_stage/mihomo" "$bindir/mihomo"
    mv -fT -- "$bin_stage/clashpt" "$bindir/clashpt"
    mv -fT -- "$shell_stage/clashpt.zsh" "$shelldir/clashpt.zsh"
    mv -fT -- "$shell_stage/clashpt.bash" "$shelldir/clashpt.bash"
    mv -fT -- "$shell_stage/receipt" "$receipt"
    printf 'Installed Mihomo %s at %s\n' "$mihomo_version" "$bindir/mihomo"
    printf 'Installed %s at %s\n' "$release" "$bindir/clashpt"

    printf '\nAdd to ~/.bashrc or ~/.zshrc for shell integration:\n'
    printf '  export PATH=%s:"$PATH"\n' "$(quote_shell "$bindir")"
    printf '  Bash: source %s\n' "$(quote_shell "$shelldir/clashpt.bash")"
    printf '  Zsh:  source %s\n' "$(quote_shell "$shelldir/clashpt.zsh")"
}

main "$@"
