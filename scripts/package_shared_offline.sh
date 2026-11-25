#!/bin/sh
set -eu
project=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project"
export PATH="$HOME/.cargo/bin:$PATH"
target=x86_64-unknown-linux-musl
release=data/release
archive=clashpt-linux-x86_64-with-mihomo.tar.gz
version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' Cargo.toml | head -n 1)
name=clashpt-v$version-shared-local-proxy-linux-x86_64-offline
(cd "$release" && awk -v f="$archive" '$2 == f' SHA256SUMS-with-mihomo | sha256sum -c -)
cargo zigbuild --release --locked --target "$target"
work=$(mktemp -d "$release/.offline.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
bundle=$work/$name
mkdir -p "$bundle/geodata"
mihomo_version=$(awk '$1 == "x86_64" {print $2}' scripts/mihomo-release.tsv)
tar -xzf "$release/$archive" -C "$bundle" mihomo MIHOMO-LICENSE "mihomo-source-$mihomo_version.tar.gz"
install -m 755 "target/$target/release/clashpt" install.sh uninstall.sh "$bundle/"
install -m 644 scripts/clashpt.bash scripts/clashpt.zsh "$bundle/"
for file in geoip.metadb geoip.dat geosite.dat ASN.mmdb; do
    install -m 644 "data/geodata/$file" "$bundle/geodata/"
done
(cd "$bundle" && sha256sum clashpt mihomo *.sh clashpt.bash clashpt.zsh MIHOMO-LICENSE mihomo-source-*.tar.gz geodata/* > SHA256SUMS)
tar -czf "$work/$name.tar.gz" -C "$work" "$name"
mv -fT "$work/$name.tar.gz" "$release/$name.tar.gz"
sha256sum "$release/$name.tar.gz"
