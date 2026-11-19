#!/usr/bin/env bash
# Re-download the third-party manuals, protocol and photos used by this repo.
# Usage: scripts/fetch-assets.sh [--check]
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
SUMS="$ROOT/scripts/checksums.sha256"

case ${1:-} in
    --check)
        cd "$ROOT"
        while read -r expected path; do
            [[ -z ${path:-} ]] && continue
            if [[ ! -f "$path" ]]; then
                printf 'SKIP (not committed; run without --check): %s\n' "$path"
                continue
            fi
            if printf '%s  %s\n' "$expected" "$path" | sha256sum --check --status; then
                printf 'OK: %s\n' "$path"
            else
                printf 'FAILED: %s\n' "$path" >&2
                exit 1
            fi
        done < "$SUMS"
        exit 0
        ;;
    '') ;;
    *)
        printf 'usage: %s [--check]\n' "$0" >&2
        exit 2
        ;;
esac

command -v curl >/dev/null || { printf 'error: curl is required\n' >&2; exit 1; }
command -v sha256sum >/dev/null || { printf 'error: sha256sum is required\n' >&2; exit 1; }

download() {
    local url=$1 dest=$2
    mkdir -p -- "$(dirname -- "$dest")"
    printf 'downloading %s\n' "${dest#$ROOT/}"
    curl -fL --retry 3 --retry-delay 1 --silent --show-error -o "$dest" "$url"
}

download 'https://www.gqelectronicsllc.com/GMC_300E_Plus_User_Guide.pdf' \
    "$ROOT/docs/manual/GMC-300E-Plus-User-Guide.pdf"
download 'https://www.gqelectronicsllc.com/download/GMCQuickStartGuide.pdf' \
    "$ROOT/docs/manual/GMC-Quick-Start-Guide.pdf"
download 'https://gqelectronicsllc.com/download/GQ-RFC1201.txt' \
    "$ROOT/docs/protocol/GQ-RFC1201.txt"
download 'https://upload.wikimedia.org/wikipedia/commons/8/82/Zaporizhzhia_geigercounter_IMG_7309_%2853022159432%29.jpg' \
    "$ROOT/assets/gmc-300e-in-use.jpg"
download 'https://upload.wikimedia.org/wikipedia/commons/thumb/0/07/GQ_GMC-300S_on_a_White_Background.tif/lossless-page1-1280px-GQ_GMC-300S_on_a_White_Background.tif.png' \
    "$ROOT/assets/gmc-300s-white.png"

cd "$ROOT"
sha256sum --check "$SUMS"
