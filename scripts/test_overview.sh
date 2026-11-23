#!/usr/bin/env bash
set -uo pipefail

proxy="${PROXY_URL:-http://127.0.0.1:7891}"
target="https://speed.cloudflare.com"
curl_options=(--noproxy '' --proxy "$proxy" --connect-timeout 5 --max-time 15
    --silent --show-error --fail --output /dev/null)

trap 'exit 130' INT
trap 'exit 143' TERM

download() {
    curl "${curl_options[@]}" --limit-rate "$1" \
        --write-out 'Downloaded %{size_download} B in %{time_total} s\n' \
        "$target/__down?bytes=$2"
}

upload() {
    head -c "$2" /dev/zero | curl "${curl_options[@]}" --limit-rate "$1" \
        --write-out 'Uploaded %{size_upload} B in %{time_total} s\n' \
        --data-binary @- "$target/__up"
}

phase() {
    local label="$1" rate="$2" download_bytes="$3" upload_bytes="$4"
    printf '\n%s (limit: %s KiB/s)\n' "$label" "${rate%k}"
    download "$rate" "$download_bytes"
    upload "$rate" "$upload_bytes"
}

printf 'Generating traffic through %s. Press Ctrl+C to stop.\n' "$proxy"
while true; do
    phase 'Low speed' 24k 65536 49152
    phase 'High speed' 128k 393216 262144
    printf '\nIdle for 4 seconds\n'
    sleep 4
    phase 'Medium speed' 48k 131072 98304
    printf '\nIdle for 3 seconds\n'
    sleep 3
done
