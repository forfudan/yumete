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
# Finally it points ~/.local/bin/yumete at the binary it just built, so `yumete`
# typed in any directory is this build. Set YUMETE_BIN_DIR to link elsewhere, or
# --no-link to leave the global command alone.
#
# Usage: scripts/build.sh [--no-data] [--no-link]
set -euo pipefail

# Repository root (this script lives in <root>/scripts).
cd "$(dirname "$0")/.."
YUMETE_ROOT="$(pwd)"

install_data=1
link_global=1
for arg in "$@"; do
  case "$arg" in
    --no-data) install_data=0 ;;
    --no-link) link_global=0 ;;
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
# `yume-compile` tool.
#
# **The file list here must match `yume_core::data_manifest`**, which is the one
# place that says what a complete Yume data set is (see yume's
# docs/development.md §4.1.1). `yumete-ime` loads by walking that manifest, so
# anything this script fails to produce is silently skipped at run time and only
# shows up as worse candidates — which is exactly the failure the manifest was
# introduced to stop. When yume adds a data file, add it here too.
#
# This script never writes into the yume tree: it only reads `yume/data` and
# builds `yume-compile`. Refreshing `yume/data` itself (yume's `gen_data.py`) is
# yume's own business and is deliberately not run from here.
install_ime_data() {
  local yume_root="${YUME_ROOT:-$YUMETE_ROOT/../yume}"
  if [[ ! -f "$yume_root/crates/yume-core/Cargo.toml" ]]; then
    echo "==> IME data: skipped (yume workspace not found at $yume_root — set YUME_ROOT)"
    return 0
  fi
  yume_root="$(cd "$yume_root" && pwd)"
  if [[ ! -f "$yume_root/data/ling.txt" ]]; then
    echo "==> IME data: skipped (no source tables in $yume_root/data — run the yume data pipeline first)"
    return 0
  fi

  local dest="${XDG_DATA_HOME:-$HOME/.local/share}/yumete"
  echo "==> IME data: compiling from $yume_root/data into $dest"

  local compiler="$yume_root/target/release/yume-compile"
  (cd "$yume_root" && cargo build --release -p yume-compile)

  # **Two directories, because yume says so.** As of yume's「Split the bundle
  # into data/ and schemes/」, `yume_core::data_manifest` names every shared file
  # under `data/` and every scheme's own under `schemes/` — so that a person who
  # brings their own 方案 replaces one directory and leaves the 字料 alone. The
  # editor loads by walking that manifest, so a file left in the old flat place
  # is not found and the failure is silent: worse candidates, or a scheme that
  # will not switch.
  #
  # Clear the tables we manage — including the names from older layouts, so a
  # data directory built by a previous yumete is not left with files the current
  # yume-core can no longer parse. User files (segmentation.txt) are preserved.
  rm -f "$dest"/{ling,xing,qing,riyue,symbols}.ytab \
        "$dest"/pinyin.yflb "$dest"/lang.{ywtb,ywl,ygram} \
        "$dest"/chaifen.ydiv "$dest"/zigen_{ling,xing,qing,riyue}.yzg \
        "$dest"/words_yuling.ywrd "$dest"/simptrad.txt \
        "$dest"/pinyin.ywtb "$dest"/chaifen{,_xing,_qing,_riyue}.yann
  rm -rf "$dest/charsets" "$dest/fonts" "$dest/data" "$dest/schemes"
  mkdir -p "$dest/data/charsets" "$dest/schemes"
  local shared="$dest/data" schemes="$dest/schemes"

  local d="$yume_root/data"
  for f in ling.txt pinyin.txt lang.txt words_yuling.txt chaifen.txt zigen_ling.txt \
           charsets/common.txt charsets/tonggui.txt charsets/harmonic.txt; do
    if [[ ! -f "$d/$f" ]]; then
      echo "!! required data file missing: $d/$f" >&2
      return 1
    fi
  done

  # 碼表 and the shared 符號表 extracted from it.
  "$compiler" "$d/ling.txt" "$schemes/ling.ytab"
  "$compiler" --symbols "$schemes/ling.ytab" "$shared/symbols.ytab"

  # Multi-character words come from every code table on hand: a word is a fact
  # about the language, not about one 方案.
  local word_sources=()
  for f in "$d/ling.txt" "$d/xing.txt" "$d/qing.txt" "$d/riyue.txt"; do
    [[ -f "$f" ]] && word_sources+=("$f")
  done

  # 音節表, plus synthesised readings for the words only the code tables carry.
  # 詞頻表 and 詞彙表 come from lang.txt, share one cut-off, and must use the same
  # one or the words above the cut fall out of both. These three numbers are
  # yume's tuning (its docs/LANGUAGE-MODEL.md §4.6); yumete follows them.
  "$compiler" --pinyin "$d/pinyin.txt" "$shared/pinyin.yflb" \
    "${word_sources[@]}" --lang "$d/lang.txt" --lang-words 300000
  "$compiler" --weights "$d/lang.txt" "$shared/lang.ywtb" --max-entries 1250000
  "$compiler" --lexicon "$d/lang.txt" "$shared/lang.ywl" \
    "${word_sources[@]}" --max-entries 1250000

  # 全息拆分表 (shared by every scheme) with its 字集 tags, and the 單字白名單.
  local division_tags=("$d/charsets/tonggui.txt:簡")
  for spec in "charsets/tongfan.txt:繁" "charsets/guji.txt:古" \
              "charsets/tai.txt:臺" "charsets/gang.txt:港"; do
    [[ -f "$d/${spec%%:*}" ]] && division_tags+=("$d/${spec%%:*}:${spec##*:}")
  done
  "$compiler" --division "$d/chaifen.txt" "$shared/chaifen.ydiv" "${division_tags[@]}"
  "$compiler" --words "$d/words_yuling.txt" "$shared/words_yuling.ywrd"

  # 字集: slots 0–2 drive the input filters, the rest only list rows in a UI.
  for cs in common tonggui harmonic tongfan guji tai gang; do
    if [[ -f "$d/charsets/$cs.txt" ]]; then
      "$compiler" --charset "$d/charsets/$cs.txt" "$shared/charsets/$cs.ycs"
    fi
  done

  # Optional, copied not compiled: the language model (without it 整句 falls back
  # to plain word frequency) and the 簡繁 table (one annotation column).
  [[ -f "$d/lang.ygram" ]] && cp "$d/lang.ygram" "$shared/lang.ygram"
  [[ -f "$d/simptrad.txt" ]] && cp "$d/simptrad.txt" "$shared/simptrad.txt"

  # 字根表: one per scheme, ~1KB each, and each must be compiled against the very
  # .ydiv above — it indexes into that file's root order.
  "$compiler" --zigen lingming "$d/zigen_ling.txt" "$shared/chaifen.ydiv" "$schemes/zigen_ling.yzg"

  # Optional sibling schemes (星陳 / 卿雲 / 日月).
  compile_scheme() {
    local id="$1" scheme="$2"
    [[ -f "$d/$id.txt" ]] && "$compiler" "$d/$id.txt" "$schemes/$id.ytab"
    [[ -f "$d/zigen_$id.txt" ]] && "$compiler" --zigen "$scheme" "$d/zigen_$id.txt" \
      "$shared/chaifen.ydiv" "$schemes/zigen_$id.yzg"
    return 0
  }
  compile_scheme xing xingchen
  compile_scheme qing qingyun
  compile_scheme riyue riyue

  # The bundled CJK root font, for terminals whose fallback chain lacks 宇浩's
  # PUA roots.
  if compgen -G "$d/fonts/*.ttf" >/dev/null 2>&1; then
    mkdir -p "$shared/fonts"
    cp "$d"/fonts/*.ttf "$shared/fonts/" 2>/dev/null || true
  fi

  echo "==> IME data installed ($(ls -1 "$schemes"/*.ytab 2>/dev/null | wc -l | tr -d ' ') table(s) in $dest)"
}

if [[ "$install_data" == "1" ]]; then
  install_ime_data
else
  echo "==> IME data: skipped (--no-data)"
fi

# ---- The global `yumete`: one symlink, so every build is the one on PATH -----
#
# Testing the editor means opening real manuscripts in whatever directory they
# live in, so `yumete` has to work from anywhere — and it has to be *this*
# build, not one from last week. A symlink is what makes that true without a
# second install step: `./yumete` is rewritten above, and the link already
# points at it, so the global command follows every build for free.
#
# It refuses to touch anything it did not put there. A real file at that path
# is somebody else's install (a tap, a manual copy), and silently replacing a
# binary the user installed on purpose is exactly the kind of thing a build
# script must not do.
link_globally() {
  local bin_dir="${YUMETE_BIN_DIR:-$HOME/.local/bin}"
  local link="$bin_dir/yumete"

  mkdir -p "$bin_dir"
  if [[ -e "$link" && ! -L "$link" ]]; then
    echo "==> global yumete: skipped ($link is a real file, not ours — remove it or set YUMETE_BIN_DIR)"
    return 0
  fi
  ln -sfn "$YUMETE_ROOT/yumete" "$link"
  echo "==> global yumete: $link -> $YUMETE_ROOT/yumete"

  # A link nothing can reach is not an install. `command -v` would find the one
  # we just made even if the directory is absent from PATH — via the shell's
  # own lookup of an absolute path — so ask PATH itself.
  case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) echo "!! $bin_dir is not on PATH — add it, or 'yumete' from another directory is still the old one" >&2 ;;
  esac
}

if [[ "$link_global" == "1" ]]; then
  link_globally
else
  echo "==> global yumete: skipped (--no-link)"
fi
