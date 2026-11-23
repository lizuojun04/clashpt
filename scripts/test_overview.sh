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
        --write-out '下载 %{size_download} B，耗时 %{time_total} s\n' \
        "$target/__down?bytes=$2"
}

upload() {
    head -c "$2" /dev/zero | curl "${curl_options[@]}" --limit-rate "$1" \
        --write-out '上传 %{size_upload} B，耗时 %{time_total} s\n' \
        --data-binary @- "$target/__up"
}

phase() {
    local label="$1" rate="$2" download_bytes="$3" upload_bytes="$4"
    printf '\n%s（限速 %s KiB/s）\n' "$label" "${rate%k}"
    download "$rate" "$download_bytes"
    upload "$rate" "$upload_bytes"
}

printf '通过 %s 生成波动流量，按 Ctrl+C 停止。\n' "$proxy"
while true; do
    phase '低速' 24k 65536 49152
    phase '高速' 128k 393216 262144
    printf '\n空闲 4 秒\n'
    sleep 4
    phase '中速' 48k 131072 98304
    printf '\n空闲 3 秒\n'
    sleep 3
done
