#!/usr/bin/env bash
#
# Build yumete in release mode and place the compiled binary at the repository
# root as ./yumete (gitignored). This is the artifact intended for release via
# a Homebrew tap.
#
# By default it also compiles the Yume IME data tables and installs them into the
# user data directory (~/.local/share/yumete) so CJK input works in the editor.
# Pass --no-data (or set YUMETE_SKIP_DATA=1) to build only the binary.
#
# Usage: scripts/build.sh [--no-data]
set -euo pipefail

# Repository root (this script lives in <root>/scripts).
cd "$(dirname "$0")/.."
YUMETE_ROOT="$(pwd)"

install_data=1
for arg in "$@"; do
  case "$arg" in
    --no-data) install_data=0 ;;
    *) echo "build.sh: unknown option '$arg'" >&2; exit 2 ;;
  esac
done
if [[ "${YUMETE_SKIP_DATA:-0}" == "1" ]]; then
  install_data=0
fi

cargo build --release
cp target/release/yumete ./yumete

# On macOS, `strip = true` invalidates the linker's ad-hoc code signature, and
# AMFI then kills the binary on launch (SIGKILL). Re-sign it ad-hoc.
if [[ "$(uname)" == "Darwin" ]]; then
  codesign --sign - --force ./yumete
fi

echo "Built ./yumete ($(./yumete --version))"

# ---- IME data: compile the Yume tables and install them into the data dir -----
#
# yumete embeds yume-core and loads compiled tables at runtime from
# ~/.local/share/yumete. This reuses the sibling yume repository's data and its
# `yume-compile` tool. Only runs when yumete lives inside the yume workspace.
install_ime_data() {
  local yume_root
  yume_root="$(cd .. && pwd)"
  if [[ ! -f "$yume_root/crates/yume-core/Cargo.toml" ]]; then
    echo "==> IME data: skipped (yume workspace not found at $yume_root)"
    return 0
  fi
  if [[ ! -f "$yume_root/data/ling.txt" ]]; then
    echo "==> IME data: skipped (no source tables in $yume_root/data — run the yume data pipeline first)"
    return 0
  fi

  local dest="${XDG_DATA_HOME:-$HOME/.local/share}/yumete"
  echo "==> IME data: compiling and installing into $dest"

  # Refresh the yume source data (non-fatal if the upstream source is offline).
  if command -v python3 >/dev/null 2>&1; then
    (cd "$yume_root" && python3 scripts/gen_data.py) || \
      echo "   (gen_data.py unavailable; using existing $yume_root/data)"
  fi

  # Build the data compiler in the yume workspace.
  local compiler="$yume_root/target/release/yume-compile"
  (cd "$yume_root" && cargo build --release -p yume-compile)

  # Clear the tables we manage (preserving any user files such as segmentation.txt).
  rm -f "$dest"/{ling,xing,qing,riyue}.ytab \
        "$dest"/pinyin.yflb "$dest"/pinyin.ywtb \
        "$dest"/chaifen.yann "$dest"/chaifen_{xing,qing,riyue}.yann
  rm -rf "$dest/charsets" "$dest/fonts"
  mkdir -p "$dest/charsets"

  # Compile the Lingming + shared tables.
  "$compiler" "$yume_root/data/ling.txt" "$dest/ling.ytab"
  "$compiler" --pinyin "$yume_root/data/pinyin.txt" "$dest/pinyin.yflb"
  "$compiler" --weights "$yume_root/data/pinyin.txt" "$dest/pinyin.ywtb"
  if [[ -f "$yume_root/data/chaifen_ling.txt" ]]; then
    "$compiler" --chaifen "$yume_root/data/chaifen_ling.txt" "$dest/chaifen.yann"
  fi
  for cs in common tonggui harmonic; do
    if [[ -f "$yume_root/data/charsets/$cs.txt" ]]; then
      "$compiler" --charset "$yume_root/data/charsets/$cs.txt" "$dest/charsets/$cs.ycs"
    fi
  done

  # Optional sibling schemes (星陳 / 卿雲 / 日月).
  for id in xing qing riyue; do
    if [[ -f "$yume_root/data/$id.txt" ]]; then
      "$compiler" "$yume_root/data/$id.txt" "$dest/$id.ytab"
    fi
    if [[ -f "$yume_root/data/chaifen_$id.txt" ]]; then
      "$compiler" --chaifen "$yume_root/data/chaifen_$id.txt" "$dest/chaifen_$id.yann"
    fi
  done

  # Bundle the CJK root font, if present.
  if compgen -G "$yume_root/data/fonts/*.ttf" >/dev/null 2>&1; then
    mkdir -p "$dest/fonts"
    cp "$yume_root"/data/fonts/*.ttf "$dest/fonts/" 2>/dev/null || true
  fi

  echo "==> IME data installed ($(ls -1 "$dest"/*.ytab 2>/dev/null | wc -l | tr -d ' ') scheme table(s))"
}

if [[ "$install_data" == "1" ]]; then
  install_ime_data
else
  echo "==> IME data: skipped (--no-data)"
fi

