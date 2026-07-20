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

echo "Built ./yumete ($(./yumete --version))"
