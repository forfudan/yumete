#!/usr/bin/env bash
#
# **把一組有代表性的畫面拍下來，改完再拍一次，逐字節比。**
#
# 給**重構**用的，不是給日常測試用的。`cargo test` 回答「邏輯對不對」，這一支回答
# 另一個問題：**「我剛纔那一堆改動，有沒有動到畫面？」** 三十幾個測試目標全綠只說明
# 沒編錯；一個把光標欄位換成一組選區的重構（§5.13 的 Phase 0）**要的證據是畫面一格
# 沒動**，而那件事測試斷言不出來——它們驗的是各自那一小塊。
#
# 用法：
#
#   scripts/frames.sh snap    改之前：拍一套底片
#   scripts/frames.sh check   改之後：再拍一套，和底片逐字節比
#   scripts/frames.sh list    這一套都拍了些什麼
#
# ⚠️ **底片不進 git**（放在 target/ 底下，`cargo clean` 會一起清掉）。進 git 的話，
# 每一次有意的界面改動都要重新生成一遍，而這個倉界面天天在動——那個維護成本買不到
# 相應的東西。這一支的用法是「改之前 snap、改之後 check」，一次重構用一次。
#
# ⚠️ **拍的是 `--shot` 的文字幀，不是 `--html`。** 顏色歸主題管，而重構要問的是
# 「字還在不在原來那一格」。文字幀對這個問題最靈敏也最好讀——diff 出來一眼看得懂。
set -euo pipefail

cd "$(dirname "$0")/.."

BIN=target/release/yumete
DIR=target/frames
SHOTS=$DIR/shots
GOLDEN=$DIR/golden

# **每一格都是一個真問過的問題。** 加一行的判準：這一幀壞掉的時候，有沒有別的東西
# 會告訴我。會的話就不必加。
#
#   名字|窗口|按鍵
scenes() {
    cat <<'SCENES'
横排-正文|80x20|
横排-選區|80x20|vjjw
横排-插入|80x20|iあ那年冬天
竪排-正文|60x24|:layout vertical\n
竪排-選區|60x24|:layout vertical\nvjj
竪排-表格|60x24|:layout vertical\n:open TABLE\n
表格-文中|80x20|:open TABLE\n
表格-滿版|80x20|:open TABLE\ntt
搜索面板|100x24|:s\n那\n
搜索面板-替換|100x24|:replace\n那\n
跳轉標籤|80x20|gw
跳轉標籤-竪排|60x24|:layout vertical\ngw
大綱|90x24|:open MANUAL\n\{space}o
設置頁|100x30|:settings\n
命令選單|80x20|:
空格選單|80x20|\{space}
幫助|100x30|:help\n
注音|80x20|:open RUBY\n
折行-窄|40x16|
折行-極窄|24x10|
SCENES
}

fixture() {
    case "$1" in
        TABLE) printf '| 名 | 說明 | 數 |\n| --- | --- | --- |\n| 甲 | 第一條 | 12 |\n| 乙 | 第二條，長一些 | 345 |\n| ⚠️ | 有警告 | 6 |\n' ;;
        RUBY)  printf '那<ruby>韋<rt>wéi</rt></ruby>字念什麼。\n她伸手去碰，指尖一涼。\n' ;;
        MANUAL) sed -n '1,60p' docs/manual.md ;;
        *)     printf '那年冬天，雪下得早。山路斷了、她在門口站了很久。\n霜花在窗上結成了葉子的樣子，後來又下了一場，比上次大。\n她伸手去碰，指尖一涼，那朵花就化了。\n\nIt was a dark and stormy night in the old house.\n\n- 買菜\n- 倒垃圾\n- 寫第三章\n' ;;
    esac
}

shoot() {
    local into=$1
    rm -rf "${into}"
    mkdir -p "${into}"
    # ⚠️ **每一幀開一個乾淨的檔**。共用一個檔的話，前一幀 `--keys` 改過的東西會被
    # 下一幀看見，而「上一幀留下什麼」正是這一支要排除的變數。
    local work="${DIR}/work"
    while IFS='|' read -r name size keys; do
        [ -n "${name}" ] || continue
        rm -rf "${work}"
        mkdir -p "${work}"
        local which=PROSE
        case "${keys}" in *TABLE*) which=TABLE ;; *RUBY*) which=RUBY ;; *MANUAL*) which=MANUAL ;; esac
        fixture "${which}" > "${work}/文.md"
        # 場景裏寫的是 `:open TABLE`，那只是說「開哪一份稿子」——真正打開的是上面
        # 鋪好的那一個檔。
        local pressed="${keys//:open TABLE\\n/}"
        pressed="${pressed//:open RUBY\\n/}"
        pressed="${pressed//:open MANUAL\\n/}"
        "${BIN}" --shot="${size}" --keys="${pressed}" "${work}/文.md" \
            > "${into}/${name}.txt" 2>&1 || true
        # ⚠️ **版本行要抹掉**：它每次構建都變（時間戳加 commit），留着的話這一支
        # 永遠報「全都不一樣」。
        sed -i '' 's/^-- yumete .*/-- yumete （版本行抹掉了）/' "${into}/${name}.txt"
    done < <(scenes)
    rm -rf "${work}"
}

case "${1:-}" in
    list)
        scenes | awk -F'|' 'NF{printf "  %-16s %s\n", $1, $2}'
        printf '\n共 %s 幀\n' "$(scenes | grep -c .)"
        ;;
    snap)
        [ -x "${BIN}" ] || { echo "!! 先 cargo build --release -p yumete" >&2; exit 1; }
        shoot "${GOLDEN}"
        printf '拍好底片：%s（%s 幀）\n' "${GOLDEN}" "$(ls "${GOLDEN}" | wc -l | tr -d ' ')"
        ;;
    check)
        [ -d "${GOLDEN}" ] || { echo "!! 還沒有底片，先 scripts/frames.sh snap" >&2; exit 1; }
        [ -x "${BIN}" ] || { echo "!! 先 cargo build --release -p yumete" >&2; exit 1; }
        shoot "${SHOTS}"
        if diff -rq "${GOLDEN}" "${SHOTS}" > /dev/null 2>&1; then
            printf '✅ %s 幀，逐字節一樣\n' "$(ls "${GOLDEN}" | wc -l | tr -d ' ')"
        else
            printf '⚠️  這幾幀變了：\n\n'
            diff -rq "${GOLDEN}" "${SHOTS}" 2>&1 | sed 's/^/    /'
            printf '\n逐幀看：diff %s/<名字>.txt %s/<名字>.txt\n' "${GOLDEN}" "${SHOTS}"
            exit 1
        fi
        ;;
    *)
        sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
        exit 1
        ;;
esac
