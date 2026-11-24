#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=$project_dir/data/release

die() {
    printf 'clashpt packager: %s\n' "$*" >&2
    exit 1
}

case "$#" in
    0)
        if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
            PATH=$HOME/.cargo/bin:$PATH
            export PATH
        fi
        command -v cargo >/dev/null 2>&1 || die 'cargo is required to build the release binaries'
        cargo zigbuild --manifest-path "$project_dir/Cargo.toml" --target-dir "$project_dir/target" \
            --release --locked --target x86_64-unknown-linux-musl --target aarch64-unknown-linux-musl
        amd64_binary=$project_dir/target/x86_64-unknown-linux-musl/release/clashpt
        arm64_binary=$project_dir/target/aarch64-unknown-linux-musl/release/clashpt ;;
    2) amd64_binary=$1; arm64_binary=$2 ;;
    *) die 'usage: package_release.sh [X86_64_BINARY AARCH64_BINARY]' ;;
esac

validate_binary() {
    [ -x "$1" ] || die "missing executable $1"
    machine=$(LC_ALL=C readelf -h -- "$1" | awk -F ': *' '/Machine:/ {print $2}')
    [ "$machine" = "$2" ] || die "$1 must be a $2 binary"
    headers=$(LC_ALL=C readelf -l -- "$1") || die "could not inspect $1"
    if printf '%s\n' "$headers" | grep -q 'INTERP'; then
        die "$1 must run without a dynamic linker"
    fi
}

command -v readelf >/dev/null 2>&1 || die 'readelf is required to verify release binaries'
validate_binary "$amd64_binary" 'Advanced Micro Devices X86-64'
validate_binary "$arm64_binary" 'AArch64'

mkdir -p -- "$output"
work=$(mktemp -d "$output/.package.XXXXXX")
trap 'rm -rf -- "$work"' EXIT
trap 'exit 1' HUP INT TERM
mkdir -- "$work/package"
install -m 644 -- "$project_dir/scripts/clashpt.zsh" "$project_dir/scripts/clashpt.bash" "$work/package/"
install -m 644 -- "$project_dir/scripts/mihomo-release.tsv" "$work/package/mihomo-release.tsv"

package() {
    install -m 755 -- "$2" "$work/package/clashpt"
    tar -czf "$work/clashpt-linux-$1.tar.gz" -C "$work/package" clashpt clashpt.zsh clashpt.bash mihomo-release.tsv
}

package_with_mihomo() {
    entry=$(awk -v arch="$1" '
        $1 == arch && NF == 4 {print $2, $3, $4; count++}
        END {if (count != 1) exit 1}
    ' "$work/package/mihomo-release.tsv") || die "no unique Mihomo release for $1"
    IFS=' ' read -r version asset checksum <<EOF
$entry
EOF
    upstream=https://github.com/MetaCubeX/mihomo
    curl -fsSL --retry 3 -o "$work/$asset" -- "$upstream/releases/download/$version/$asset"
    printf '%s  %s\n' "$checksum" "$work/$asset" | sha256sum -c -
    gzip -dc -- "$work/$asset" > "$work/package/mihomo"
    chmod 755 "$work/package/mihomo"
    validate_binary "$work/package/mihomo" "$2"

    source_archive=mihomo-source-$version.tar.gz
    if [ ! -f "$work/package/$source_archive" ]; then
        curl -fsSL --retry 3 -o "$work/package/$source_archive" -- "$upstream/archive/refs/tags/$version.tar.gz"
        tar -xOzf "$work/package/$source_archive" "mihomo-${version#v}/LICENSE" > "$work/package/MIHOMO-LICENSE"
        test -s "$work/package/MIHOMO-LICENSE"
    fi

    tar -czf "$work/clashpt-linux-$1-with-mihomo.tar.gz" -C "$work/package" \
        clashpt mihomo clashpt.zsh clashpt.bash MIHOMO-LICENSE "$source_archive"
}

command -v curl >/dev/null 2>&1 || die 'curl is required to download Mihomo and its source'
package x86_64 "$amd64_binary"
package_with_mihomo x86_64 'Advanced Micro Devices X86-64'
package aarch64 "$arm64_binary"
package_with_mihomo aarch64 'AArch64'
install -m 755 -- "$project_dir/install.sh" "$work/install.sh"
(
    cd -- "$work"
    sha256sum clashpt-linux-x86_64.tar.gz clashpt-linux-aarch64.tar.gz > SHA256SUMS
    sha256sum clashpt-linux-x86_64-with-mihomo.tar.gz clashpt-linux-aarch64-with-mihomo.tar.gz > SHA256SUMS-with-mihomo
)
mv -- "$work/clashpt-linux-x86_64.tar.gz" "$work/clashpt-linux-aarch64.tar.gz" \
    "$work/clashpt-linux-x86_64-with-mihomo.tar.gz" "$work/clashpt-linux-aarch64-with-mihomo.tar.gz" \
    "$work/install.sh" "$work/SHA256SUMS" "$work/SHA256SUMS-with-mihomo" "$output/"
printf 'Release files prepared in %s\n' "$output"
