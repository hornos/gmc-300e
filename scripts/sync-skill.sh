#!/usr/bin/env bash
# Keep the bundled OpenCode skill copies identical to gmc300e.sh.
# Usage: scripts/sync-skill.sh [--check]
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
SRC="$ROOT/gmc300e.sh"
INSTALLED="$HOME/.config/opencode/skills/gmc-300e"
TARGETS=("$ROOT/skill/scripts")
[[ -d "$INSTALLED" ]] && TARGETS+=("$INSTALLED/scripts")

case ${1:-} in
    --check)
        for dir in "${TARGETS[@]}"; do
            cmp "$SRC" "$dir/gmc300e.sh"
            printf 'identical: %s\n' "$dir/gmc300e.sh"
        done
        exit 0
        ;;
    '') ;;
    *)
        printf 'usage: %s [--check]\n' "$0" >&2
        exit 2
        ;;
esac

for dir in "${TARGETS[@]}"; do
    mkdir -p -- "$dir"
    cp -- "$SRC" "$dir/gmc300e.sh"
    chmod +x "$dir/gmc300e.sh"
    cmp "$SRC" "$dir/gmc300e.sh"
    printf 'synced: %s\n' "$dir/gmc300e.sh"
done

if [[ -d "$INSTALLED" ]]; then
    cp -- "$ROOT/skill/SKILL.md" "$INSTALLED/SKILL.md"
    mkdir -p -- "$INSTALLED/references"
    cp -- "$ROOT/skill/references/"*.md "$INSTALLED/references/"
    printf 'synced skill instructions and references to %s\n' "$INSTALLED"
fi
