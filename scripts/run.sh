#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
if ! command -v cargo >/dev/null 2>&1 && [[ -x "$HOME/.cargo/bin/cargo" ]]; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi
if (($# == 0)); then
    set -- tui
fi
exec cargo run --quiet --manifest-path "$project_dir/Cargo.toml" -- "$@"
