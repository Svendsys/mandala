#!/usr/bin/env bash
# Check that `vendor/cosmic-text` is exactly the published cosmic-text
# 0.19.0 plus `vendor/cosmic-text/mandala.patch`, and that cosmic-text's
# own test suite passes against the patched sources (V1-B14, #304;
# vendor/cosmic-text/PATCHES.md).
#
# Usage:
#   scripts/check_vendored_cosmic_text.sh
#
# It needs the network (it fetches the crate from crates.io), so it is
# deliberately not part of `./test.sh`, which stays offline; CI's
# `vendored-cosmic-text` job runs it (.github/workflows/test.yml).
#
# Steps:
#   1. Fetch cosmic-text-0.19.0.crate and check its sha256 against the
#      checksum crates.io published, the one the lockfile recorded before
#      the patch replaced the registry source with a path.
#   2. Unpack it, apply mandala.patch to its Cargo.toml and src/, and
#      compare the result with vendor/cosmic-text file for file. The
#      vendored directory may hold only what the crate ships that is
#      vendored (Cargo.toml, src/, README.md, LICENSE-MIT,
#      LICENSE-APACHE) and the two files that describe the patch
#      (PATCHES.md, mandala.patch).
#   3. Put the patched src/ in the unpacked crate, which keeps its own
#      manifest, tests and fonts, and run its library and integration
#      tests with its own lockfile, every one of them. `LANG` is set to
#      `en_US.UTF-8` when the caller has none: `test_fonts_load_time`
#      reads the host locale through sys-locale and panics where none is
#      set (a CI runner, a container), patched or not.
#
# Exit codes: 0 everything holds, 1 a check failed.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VENDORED="$ROOT/vendor/cosmic-text"
VERSION="0.19.0"
SHA256="be17b688510d934ce13f48a2beba700e11583e281e0fda99c22bb256a14eda73"
URL="https://static.crates.io/crates/cosmic-text/cosmic-text-$VERSION.crate"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "== fetch cosmic-text $VERSION =="
curl --fail --silent --show-error --location "$URL" -o "$WORK/crate.tar.gz"
echo "$SHA256  $WORK/crate.tar.gz" | sha256sum --check --status \
  || { echo "FAILED: the fetched crate's sha256 is not $SHA256" >&2; exit 1; }
tar -xzf "$WORK/crate.tar.gz" -C "$WORK"
PUBLISHED="$WORK/cosmic-text-$VERSION"

echo "== published + mandala.patch == vendor/cosmic-text =="
PATCHED="$WORK/patched"
mkdir -p "$PATCHED"
cp -r "$PUBLISHED/src" "$PUBLISHED/Cargo.toml" "$PUBLISHED/README.md" \
  "$PUBLISHED/LICENSE-MIT" "$PUBLISHED/LICENSE-APACHE" "$PATCHED/"
(cd "$PATCHED" && patch --strip=1 --silent --forward < "$VENDORED/mandala.patch") \
  || { echo "FAILED: mandala.patch does not apply to the published crate" >&2; exit 1; }
cp "$VENDORED/PATCHES.md" "$VENDORED/mandala.patch" "$PATCHED/"
diff --recursive "$PATCHED" "$VENDORED" \
  || { echo "FAILED: vendor/cosmic-text is not the published crate plus mandala.patch" >&2; exit 1; }

echo "== cosmic-text's own tests, against the patched src/ =="
rm -rf "$PUBLISHED/src"
cp -r "$VENDORED/src" "$PUBLISHED/src"
# From the repository root, so the toolchain rust-toolchain.toml pins
# builds it, as it builds the app.
cd "$ROOT"
LANG="${LANG:-en_US.UTF-8}" cargo test --locked --manifest-path "$PUBLISHED/Cargo.toml" --lib --tests \
  || { echo "FAILED: cosmic-text's own tests fail against the patched src/" >&2; exit 1; }

echo "vendor/cosmic-text is cosmic-text $VERSION plus mandala.patch, and its tests pass."
