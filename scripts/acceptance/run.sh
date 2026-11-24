#!/bin/sh
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
repo=$(CDPATH= cd -- "$script_dir/../.." && pwd -P)
default_cli=$repo/target/release/clashpt
[ -x "$default_cli" ] || default_cli=$repo/target/debug/clashpt
binary=${1:-$default_cli}
core=${2:-$(command -v mihomo || :)}
if [ -z "${CLASHPT_ACCEPTANCE_PROOT:-}" ] && ! command -v proot >/dev/null 2>&1 &&
    command -v apt-get >/dev/null 2>&1 && command -v dpkg-deb >/dev/null 2>&1; then
    packages=$(mktemp -d)
    trap 'rm -rf -- "$packages"' EXIT
    if (cd "$packages" && apt-get -o Acquire::Retries=0 -o Acquire::http::Timeout=8 download proot libtalloc2 >/dev/null 2>&1 &&
        mkdir contents && for pkg in ./*.deb; do dpkg-deb -x "$pkg" contents || exit 1; done); then
        CLASHPT_ACCEPTANCE_PROOT=$packages/contents/usr/bin/proot
        for library in "$packages"/contents/usr/lib/*/libtalloc.so.2; do
            CLASHPT_ACCEPTANCE_PROOT_LIB=${library%/*}
        done
        export CLASHPT_ACCEPTANCE_PROOT CLASHPT_ACCEPTANCE_PROOT_LIB
    fi
fi
python3 -B "$script_dir/acceptance.py" "$repo" "$binary" "$core"
