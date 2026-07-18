#!/usr/bin/env bash
#
# Every symbolic icon name referenced in the source must exist in the icon
# theme. A missing one is not an error at runtime — GTK silently substitutes a
# broken-image glyph — so nothing catches it except looking at the app.
#
# Usage: packaging/check-icons.sh

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

missing=0
while IFS= read -r name; do
    if ! find /usr/share/icons ~/.local/share/icons -name "$name.svg" 2>/dev/null | grep -q .; then
        echo "missing icon: $name"
        missing=$((missing + 1))
    fi
done < <(grep -rhoE '"[a-z0-9-]+-symbolic"' "$ROOT/crates" --include="*.rs" | tr -d '"' | sort -u)

if [[ $missing -gt 0 ]]; then
    echo "$missing icon(s) missing from the theme" >&2
    exit 1
fi
echo "all referenced icons present"
