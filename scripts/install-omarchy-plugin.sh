#!/usr/bin/env bash
# Build the Rust controller, put it on PATH and enable the local Omarchy plugin.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
plugin_id="io.github.hornos.gmc-300e"

cargo build --release --manifest-path "$repo/rust/Cargo.toml"
install -d -m 755 "$HOME/.local/bin"
install -m 755 "$repo/rust/target/release/gmc300e" "$HOME/.local/bin/gmc300e"

if ! command -v omarchy >/dev/null 2>&1; then
  echo "Omarchy CLI not found; Rust binary installed at $HOME/.local/bin/gmc300e" >&2
  exit 0
fi

# This plugin lives in a subdirectory of an existing repository, so install the
# local directory directly. `omarchy plugin add` expects a git URL/repo.
target="$HOME/.config/omarchy/plugins/$plugin_id"
mkdir -p "$(dirname "$target")"
rm -rf "$target"
cp -a "$repo/omarchy-plugin" "$target"

# A freshly copied plugin is only discovered when the running shell reloads.
known="$(omarchy plugin list --json | python3 -c '
import json, sys
plugins = json.load(sys.stdin)
print("1" if any(p.get("id") == "'"$plugin_id"'" for p in plugins) else "0")
')"
if [[ "$known" != "1" ]]; then
  omarchy-restart-shell
fi

omarchy plugin enable "$plugin_id" right >/dev/null 2>&1 || true
omarchy-restart-shell

echo "GMC-300E bar monitor enabled and the shell reloaded."
