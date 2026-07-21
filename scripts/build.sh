#!/usr/bin/env bash
#
# Build yumete in release mode and place the compiled binary at the repository
# root as ./yumete (gitignored). This is the artifact intended for release via
# a Homebrew tap.
#
# Usage: scripts/build.sh
set -euo pipefail

# Repository root (this script lives in <root>/scripts).
cd "$(dirname "$0")/.."

cargo build --release
cp target/release/yumete ./yumete

# On macOS, `strip = true` invalidates the linker's ad-hoc code signature, and
# AMFI then kills the binary on launch (SIGKILL). Re-sign it ad-hoc.
if [[ "$(uname)" == "Darwin" ]]; then
  codesign --sign - --force ./yumete
fi

echo "Built ./yumete ($(./yumete --version))"
