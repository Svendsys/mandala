#!/usr/bin/env bash
# Apply or check SPDX-License-Identifier headers on all .rs files.
#
# Usage:
#   ./scripts/add_license_headers.sh           # add headers to files missing them
#   ./scripts/add_license_headers.sh --check   # exit nonzero if any are missing
#
# Idempotent: skips files that already carry the marker.

set -euo pipefail

HEADER='// SPDX-License-Identifier: MPL-2.0'
MARKER='SPDX-License-Identifier'
MODE="apply"

if [ "${1:-}" = "--check" ]; then
    MODE="check"
elif [ -n "${1:-}" ]; then
    echo "Unknown argument: $1" >&2
    echo "Usage: $0 [--check]" >&2
    exit 2
fi

mapfile -t files < <(find . \
    -type d \( -name target -o -name dist -o -name .git -o -name node_modules \) -prune \
    -o -type f -name '*.rs' -print)

missing=()

for file in "${files[@]}"; do
    if ! grep -q "$MARKER" "$file"; then
        missing+=("$file")
    fi
done

if [ ${#missing[@]} -eq 0 ]; then
    echo "All ${#files[@]} .rs file(s) carry the SPDX header."
    exit 0
fi

if [ "$MODE" = "check" ]; then
    echo "Missing SPDX-License-Identifier in ${#missing[@]} file(s):" >&2
    printf '  %s\n' "${missing[@]}" >&2
    echo "" >&2
    echo "Run: ./scripts/add_license_headers.sh" >&2
    exit 1
fi

# MODE=apply
for file in "${missing[@]}"; do
    tmp=$(mktemp)
    printf '%s\n\n' "$HEADER" > "$tmp"
    cat "$file" >> "$tmp"
    mv "$tmp" "$file"
done

echo "Added header to ${#missing[@]} file(s); ${#files[@]} total."
