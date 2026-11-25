#!/bin/sh

die() {
    printf 'clashpt installer: %s\n' "$*" >&2
    exit 1
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
    printf 'Downloading %s to %s\n' "${1##*/}" "$2" >&2
    case "$1" in
        https://github.com/*|https://raw.githubusercontent.com/*)
            if [ -n "$gh_proxy" ]; then
                # Reject redirects so the client never follows a GitHub URL directly.
                curl -q --noproxy '*' -fL --progress-bar --connect-timeout 60 --max-redirs 0 --retry 3 --output "$2" -- "$gh_proxy/$1" ||
                    die "could not download $1 through $gh_proxy"
                return
            fi ;;
    esac
    curl -fL --progress-bar --connect-timeout 60 --retry 3 --output "$2" -- "$1" || die "could not download $1"
}

check_sha256() {
    command -v sha256sum >/dev/null 2>&1 || die 'sha256sum is required to verify downloads'
    printf 'Verifying SHA-256 for %s\n' "${2##*/}" >&2
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

    bundle=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
    offline=no
    if [ -f "$0" ] && [ -z "$from_build" ] && [ -f "$bundle/SHA256SUMS" ] && [ -x "$bundle/clashpt" ] && [ -x "$bundle/mihomo" ]; then
        offline=yes
        (cd "$bundle" && sha256sum -c SHA256SUMS) || die 'invalid offline package'
        binary=$bundle/clashpt
        mihomo=$bundle/mihomo
        zsh_script=$bundle/clashpt.zsh
        bash_script=$bundle/clashpt.bash
    elif [ -n "$from_build" ]; then
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

    if [ "$offline" = yes ]; then
        mihomo_version=$("$mihomo" -v | head -n 1)
    else
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
    fi

    mkdir -p -- "$prefix"
    prefix=$(realpath -e -- "$prefix")
    exec 9<"$prefix"
    printf 'Acquiring installation lock on %s\n' "$prefix" >&2
    flock -x 9
    printf 'Installing to %s\n' "$prefix" >&2
    bindir=$prefix/bin
    share=$prefix/share/clashpt
    shelldir=$share/shell
    mkdir -p -- "$bindir" "$shelldir"
    bin_stage=$(mktemp -d "$bindir/.clashpt-install.XXXXXX")
    shell_stage=$(mktemp -d "$shelldir/.clashpt-install.XXXXXX")
    install -m 755 -- "$binary" "$bin_stage/clashpt"
    install -m 755 -- "$mihomo" "$bin_stage/mihomo"
    install -m 644 -- "$zsh_script" "$bash_script" "$shell_stage/"
    "$bin_stage/clashpt" install-settings || die 'could not install ClashPT settings'
    if [ "$offline" = yes ]; then
        mkdir -p -- "$share/geodata"
        for file in "$bundle"/geodata/*; do
            install -m 644 -- "$file" "$shell_stage/geodata.tmp"
            mv -fT -- "$shell_stage/geodata.tmp" "$share/geodata/${file##*/}"
        done
    fi
    mv -fT -- "$bin_stage/mihomo" "$bindir/mihomo"
    mv -fT -- "$bin_stage/clashpt" "$bindir/clashpt"
    mv -fT -- "$shell_stage/clashpt.zsh" "$shelldir/clashpt.zsh"
    mv -fT -- "$shell_stage/clashpt.bash" "$shelldir/clashpt.bash"
    printf 'Installed Mihomo %s at %s\n' "$mihomo_version" "$bindir/mihomo"
    printf 'Installed %s at %s\n' "$release" "$bindir/clashpt"

    printf '\nAdd to ~/.bashrc or ~/.zshrc for shell integration:\n'
    printf '  export PATH=%s:"$PATH"\n' "$(quote_shell "$bindir")"
    printf '  Bash: source %s\n' "$(quote_shell "$shelldir/clashpt.bash")"
    printf '  Zsh:  source %s\n' "$(quote_shell "$shelldir/clashpt.zsh")"
}

main "$@"
