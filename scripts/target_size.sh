#!/usr/bin/env bash
#
# 說一句 `target/` 有多大，大到該清了就說該清了。**它自己不刪任何東西。**
#
# 為什麼有這支（2026-09-27 量的）：`target/debug/deps` 攢到過 **210 萬個檔、
# 45 G**（`cargo clean` 報的邏輯大小 207 GiB），而那時候「什麼都不用重編」的
# `cargo test` 也要三分半——`rustc` 停在不可中斷的磁盤等待上，CPU 5%。一個目錄
# 裏兩百萬條，APFS 查一個名字都在爬。
#
# 元凶是 `.rcgu.o`（佔 99.3%）：macOS 上 dev 的 `split-debuginfo` 默認
# `unpacked`，調試信息留在每個代碼生成單元的目標檔裏，所以那些 `.o` 必須留着，
# 而檔名帶編譯哈希——重編一次新寫一套，**cargo 對 `target/` 沒有垃圾回收，舊的
# 永遠不刪**。`Cargo.toml` 現在寫死了 `split-debuginfo = "packed"`，那一族歸零。
#
# 所以這支盯兩件事：
#
#  1. **`.rcgu.o` 回來了沒有** —— 回來就是 `packed` 那一行被誰動了，或者有人拿
#     別的 profile 在編。這一查兩頭都便宜：沒有就只掃三百來個檔，有就第一個
#     就撞上。
#  2. **整棵樹多大** —— 過了線就說一句。⚠️ 真的大起來的時候這一步本身就慢，而
#     那個慢正是它要報的那件事。
#
# 為什麼不定時刪：定時器可能正好在要構建之前把緩存清了，而全量重編只要一分多鐘、
# 樹只有一兩 G 的時候留着它幾乎不花錢。該不該清是看數，不是看日子。
#
# 用法：scripts/target_size.sh   （只報告，退出碼永遠是 0）
set -euo pipefail

cd "$(dirname "$0")/.."

if [ ! -d target ]; then
    echo "target/：還沒有，乾淨的。"
    exit 0
fi

# 過了這條線就說一句。1.9 G 是清完重編一輪的大小，20 G 是「攢了很久」。
BIG_GB=20

stray="$(find target -name '*.rcgu.o' -print -quit 2>/dev/null || true)"
if [ -n "${stray}" ]; then
    echo "⚠️  target/ 裏有 .rcgu.o，說明 split-debuginfo 不是 packed 了："
    echo "    ${stray}"
    echo "    看一眼 Cargo.toml 的 [profile.dev]，然後 cargo clean。"
fi

size="$(du -sh target 2>/dev/null | cut -f1)"
gb="$(du -sk target 2>/dev/null | cut -f1)"
gb=$(( gb / 1024 / 1024 ))
echo "target/：${size}"
if [ "${gb}" -ge "${BIG_GB}" ]; then
    echo "⚠️  過 ${BIG_GB} G 了，該清一次——全量重編大約一分半："
    echo "    cargo clean"
fi
