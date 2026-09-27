#!/usr/bin/env python3
"""把一份發布說明畫成一張長圖，好轉進聊天羣。

    python3 scripts/release_card.py 0.3.0
    python3 scripts/release_card.py 0.3.0 --notes 草稿.md --out /tmp/card.png

發布說明從 GitHub 那一頁取（`gh release view`），也可以拿 `--notes` 指一個本地檔。
卡片上半是一幀**現拍的** yumete 畫面——`yumete --shot --html` 交出來的就是帶顏色的
HTML，原樣嵌進去即可，不必先存成圖片再套進來。

⚠️ **沒有 markdown 庫，也不裝一個。** 發布說明只用到列表、粗體、行內代碼、鏈接、代碼
塊這幾樣，而一個只認這幾樣的解析器是三十行的事（同 `Editor::outline`：標題就是行首那
幾個井號，不需要 parser）。真要用到別的語法，改這裏比裝一個依賴便宜。

要 Chrome：`/Applications/Google Chrome.app`。
"""
import argparse
import html
import pathlib
import re
import unicodedata
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent.parent
CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"

# **字體**（2026-09-26 定的，原話：「用 wenkai GB 來渲染這個卡片以及截圖中的文字」）。
#
# ⚠️ **截圖那一塊必須是中文剛好兩倍寬的等寬體**，否則每一行的格子對不齊——終端畫面
# 是按「一個漢字兩格」排的，字體不守這條，畫面就散。量過（Chrome，100px）：
#
#   | 字體 | A | 中 | 比 |
#   | --- | --- | --- | --- |
#   | LXGW WenKai Mono GB | 50.0 | 100.0 | **2.000** ✓ |
#   | LXGW WenKai GB | 69.0 | 100.0 | 1.449（比例，正文用） |
#   | Sarasa Mono SC | 72.2 | 100.0 | 1.385 ← **沒裝**，回落到襯線 |
#   | Menlo | 60.2 | 100.0 | 1.661 |
#
# ⚠️ **`document.fonts.check()` 會撒謊**：上面四個它全回 `true`，包括根本沒裝的
# Sarasa。要判斷一個字體在不在，**量「中」和「A」的寬度比**，別問那個 API。
MONO = '"LXGW WenKai Mono GB", "Sarasa Mono SC", Menlo, monospace'
SANS = '"LXGW WenKai GB", "PingFang SC", "Hiragino Sans GB", sans-serif'

# 墨香那幾個色，和編輯器裏量出來的一樣（`development.md` 的梯子那一節）。
PAPER = "#181A1D"
CHROME_RUNG = "#2B2C2E"
TEXT = "#D2CEC4"
GOLD = "#D8C99A"
QUIET = "#8A8883"


def shell(args, **kw):
    return subprocess.run(args, check=True, capture_output=True, text=True, **kw).stdout


def inline(text):
    """`代碼`、**粗**、[字](網址) —— 其餘原樣轉義。

    ⚠️ **先轉義再套標籤**，不能反過來：正文裏一個 `<` 會把後面整段吃掉。
    """
    out = html.escape(text)
    # ⚠️ **雙反引號先收，而且收完要藏起來。** ``  `s  `` 是「行內代碼裏本身有一個
    # 反引號」的寫法。讓單反引號那一條先跑，它會從中間切開；而**就算雙的先跑**，
    # 換出來的 `<code> `s </code>` 裏還留着那個反引號，單的那一條照樣會再吃一遍，
    # 出來是 `<code> <code>s </code> …`。所以先換成一個占位符，等別的規則都跑完再
    # 放回去——占位符裏沒有反引號、星號、方括號，誰也碰不到它。
    holes = []

    def stash(m):
        body = m.group(1)
        # CommonMark：兩頭各有一個空格就各去一個（`` `s `` 寫的是 「`s」）。
        if len(body) > 1 and body[0] == " " and body[-1] == " " and body.strip():
            body = body[1:-1]
        holes.append(body)
        return f"\x00{len(holes) - 1}\x00"

    out = re.sub(r"``(.+?)``", stash, out)
    out = re.sub(r"`([^`]+)`", r'<code>\1</code>', out)
    out = re.sub(r"\*\*([^*]+)\*\*", r"<b>\1</b>", out)
    out = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", r'<a href="\2">\1</a>', out)
    out = re.sub(r"\x00(\d+)\x00", lambda m: f"<code>{holes[int(m.group(1))]}</code>", out)
    return out


def render(md):
    """那幾樣語法 → HTML。段落、`---`、代碼塊、列表（含一層縮進）、標題。"""
    out, lines, i = [], md.split("\n"), 0
    stack = 0  # 開着幾層 <ul>

    def close(to=0):
        nonlocal stack
        while stack > to:
            out.append("</ul>")
            stack -= 1

    while i < len(lines):
        line = lines[i].rstrip()
        if line.startswith("```"):
            close()
            i += 1
            code = []
            while i < len(lines) and not lines[i].startswith("```"):
                code.append(html.escape(lines[i]))
                i += 1
            out.append("<pre class=sh>" + "\n".join(code) + "</pre>")
        elif line.strip() == "---":
            close()
            out.append("<hr>")
        elif re.match(r"^#{1,4} ", line):
            close()
            n = len(line) - len(line.lstrip("#"))
            out.append(f"<h{n}>{inline(line[n:].strip())}</h{n}>")
        elif re.match(r"^(\s*)[-*] ", line):
            deep = 1 if line.startswith(("  -", "  *")) else 0
            while stack < deep + 1:
                out.append("<ul>")
                stack += 1
            close(deep + 1)
            out.append("<li>" + inline(line.lstrip().lstrip("-*").strip()) + "</li>")
        elif not line.strip():
            close()
        else:
            close()
            out.append("<p>" + inline(line.strip()) + "</p>")
        i += 1
    close()
    return "\n".join(out)


# 一格多寬（像素）。截圖那一塊的字號乘以這個比例——漢字兩格、西文一格。
CELL = 7.5
LEAD = 1.5  # 行高倍數


def cells(ch, nxt=""):
    """這個字佔幾格——**按終端的算法，不按字體的意見**。

    UAX #11：只有 East Asian **Wide** 和 **Fullwidth** 算兩格，組合符與控制字符算零，
    其餘一格。

    ⚠️ **Ambiguous 算一格，不算兩格。** `─ │ · – → ±` 這一族在 UAX #11 裏是
    「看終端」，而 yumete 出的那一幀按一格排——量出來的：一條 `─────` 的分隔線
    24 個字符正好 24 格，按兩格算會多出 24 格（2026-09-26 自檢報的就是這一行）。

    ⚠️ **變體選擇符本身零格，可它把前一個字撐成兩格。** `⚠️` 是兩個 char
    （U+26A0 ＋ U+FE0F）：`⚠` 單獨是一格，跟上 VS16 就成了 emoji 呈現，終端給兩格
    （`unicode-width` 的 `width_cjk` 就是這麼算的）。所以要往後看一個字。

    ⚠️ **2026-09-26 這一條漏了兩次。** 第一次是整條沒想到；第二次是自檢只查了
    「超出」沒查「不足」，於是短一格的行靜悄悄地過——畫出來就是那一行的金框往前
    縮了一格，而一眼就被看出來了。**自檢要查兩頭。**

    ⚠️ **這支函數是在複述 yumete 的算法**（`yumete_cjk::char_width`），兩邊有可能
    走散。所以 `pin` 每一行都對一次總格數，對不上就當場喊出來——複述不可靠，自檢
    可靠。
    """
    if ch in "\ufe0f\ufe0e" or unicodedata.combining(ch):
        return 0
    if unicodedata.category(ch) in ("Cc", "Cf"):
        return 0
    if nxt == "\ufe0f":
        return 2
    return 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1


# 框綫字。**鎖進格子之後它們會斷成虛綫**——字形本身不占滿一格，而真終端會把它拉滿。
# 2026-09-19 踩過一次：看圖的原話是「可以不用虛綫的吧」，而編輯器畫的一直是實綫。
# 橫的往寬裏拉一點，竪的往高裏拉一截。
ACROSS = "─━┄┅┈┉╌╍┬┴┼┭┮┯┰┱┲┵┶┷┸┹┺┻"
DOWN = "│┃┆┇┊┋╎╏├┤┼╭╮╰╯┌┐└┘┝┞┟┠┡┢┥┦┧┨┩┪"


def pin(shot, wide):
    """把 `--shot --html` 那一串重排成**一格一個盒子**。

    ⚠️ **不能讓瀏覽器自己排這串 span**，哪怕字體是嚴格兩倍寬的。2026-09-26 量過，
    88×22 那一幀裏有四個字符終端和字體各說各的：

        ·  U+B7    終端 2 格，字體 1 格
        –  U+2013  終端 2 格，字體 1 格
        ⚠  U+26A0  終端 1 格，字體 2 格
        ️   U+FE0F  終端 0 格，字體 0 格（但它跟着的 emoji 佔兩格）

    **換字體修不好這一族**：U+FE0F 在任何字體裏都是零寬，而終端給那個 emoji 兩格。
    每個字鎖進一個 `inline-block` 的盒子，字體就沒有發言權了。

    回報 `(html, 出格的行)`——後者非空就是我這邊的寬度算法和 yumete 的對不上，
    **當場說出來**，別讓它靜靜地歪在圖上。
    """
    rows, off = [], []
    for n, line in enumerate(shot.split("\n")):
        at, out = 0, []
        for m in re.finditer(r'<span style="([^"]*)">(.*?)</span>', line):
            style, text = m.group(1), html.unescape(m.group(2))
            for i, ch in enumerate(text):
                w = cells(ch, text[i + 1] if i + 1 < len(text) else "")
                if w:
                    box = ""
                    if ch in ACROSS:
                        box += ";transform:scaleX(1.12)"
                    if ch in DOWN:
                        box += ";transform:scaleY(1.45)"
                    out.append(
                        f'<i style="{style};left:{at * CELL:.1f}px;'
                        f'width:{w * CELL:.1f}px{box}">{html.escape(ch)}</i>'
                    )
                at += w
        if out:
            rows.append("".join(out))
            # ⚠️ **兩頭都查。** 只查「超出」的話，短一格的行不會報，而它畫出來
            # 就是那一行的邊框往前縮一格——比超出更難看見。
            if at != wide:
                off.append(f"第 {n + 1} 行 {at} 格（該 {wide}）")
    body = "".join(f'<div class=row>{r}</div>' for r in rows)
    return body, off


def page(version, body, shot, url):
    SHOT_PX = 15
    ROW = round(SHOT_PX * LEAD, 1)
    return f"""<!doctype html><meta charset="utf-8"><style>
* {{ box-sizing: border-box }}
body {{ margin: 0; background: {PAPER}; width: 1080px;
  font: 19px/1.9 {SANS};
  color: {TEXT}; }}
.card {{ padding: 56px 64px 48px }}
.top {{ display: flex; align-items: baseline; gap: 20px; margin-bottom: 8px }}
.name {{ font-size: 46px; font-weight: 600; color: {GOLD}; letter-spacing: .04em }}
.en {{ font-size: 22px; color: {QUIET}; letter-spacing: .12em }}
.ver {{ margin-left: auto; font-size: 30px; font-weight: 600; color: {PAPER};
  background: {GOLD}; padding: 2px 18px; border-radius: 4px }}
.rule {{ height: 3px; background: {GOLD}; margin: 18px 0 34px }}
.shot {{ margin: 0 0 34px; padding: 18px 20px; background: {PAPER};
  border: 1px solid {CHROME_RUNG}; border-radius: 6px; overflow: hidden }}
/* `--shot --html` 交出來的那一塊，原樣嵌進來 */
.shot .grid {{ font: {SHOT_PX}px/{LEAD} {MONO}; white-space: pre }}
.shot .row {{ position: relative; height: {ROW}px }}
/* ⚠️ **一格一個盒子，位置由列號算**——見 `pin`：字體對寬度沒有發言權。 */
/* `text-align:center` 管「字形比格子窄」那一半：居中留白，不往旁邊靠。
   「比格子寬」那一半由頁尾那段腳本量完壓扁。 */
.shot i {{ position: absolute; top: 0; font-style: normal; text-align: center;
  display: inline-block; white-space: pre }}
p {{ margin: 0 0 14px }}
h1, h2, h3, h4 {{ color: {GOLD}; margin: 30px 0 12px; font-size: 26px }}
ul {{ margin: 0 0 14px; padding-left: 26px }}
ul ul {{ margin: 6px 0 2px }}
li {{ margin: 0 0 10px }}
b {{ color: #FFF6DD; font-weight: 600 }}
code {{ font: .9em {MONO}; color: {GOLD};
  background: {CHROME_RUNG}; padding: 1px 6px; border-radius: 3px }}
a {{ color: #8FC9E8; text-decoration: none }}
pre.sh {{ background: {CHROME_RUNG}; color: {TEXT}; padding: 16px 20px;
  border-radius: 5px; margin: 0 0 18px;
  font: 16px/1.6 {MONO} }}
hr {{ border: 0; border-top: 1px solid {CHROME_RUNG}; margin: 26px 0 }}
.foot {{ margin-top: 34px; padding-top: 20px; border-top: 1px solid {CHROME_RUNG};
  color: {QUIET}; font-size: 17px; display: flex; justify-content: space-between }}
</style>
<div class=card>
  <div class=top>
    <span class=name>宇夢編輯器</span><span class=en>yumete</span>
    <span class=ver>{html.escape(version)}</span>
  </div>
  <div class=rule></div>
  <div class=shot><div class=grid>{shot}</div></div>
  {body}
  <div class=foot><span>{html.escape(url)}</span><span>Apache-2.0</span></div>
</div>
<script>
// **格子是權威，字形迁就格子**（2026-09-26 定的）。每一格已經釘在
// `left = 欄號 × 格寬` 上，所以誰也推不動誰；剩下的是**一格裝不下的字形**——
// 讓它溢出去就會蓋住鄰居。量一量，寬了就橫向壓扁。
//
// ⚠️ **必須在瀏覽器裏量，不能在腳本裏猜。** 同一個字在不同字體、不同回退鏈下寬度
// 不同，而頁面自己知道它實際排成了多寬（`scrollWidth`）。
// ⚠️ 框綫字是反過來的：它們比格子**窄**，要拉滿，不然一列 `─` 是虛綫。那一組在
// `pin` 裏已經給了 `scaleX`／`scaleY`，這裏不碰它們。
// ⚠️ **量的是「步進寬度」，不是墨跡外框。** 兩個都試過（2026-09-26）：
//   * `scrollWidth` 取整——7.5px 的字形報 8，每個半角字都成了「超出」；
//   * `Range.getBoundingClientRect()` 量的是**墨跡**——楷體的筆畫本來就略微溢出
//     字身框（漢字 16 對 15），於是幾乎每個字都被判越界，照它壓會把整幅字壓扁。
// 真正決定「會不會擠到下一格」的是 canvas 的 `measureText().width`。
const pen = document.createElement("canvas").getContext("2d");
// ⚠️ **字體要寫死，別從 `getComputedStyle` 拿。** 那裏回來的帶着行高
// （`15px/22.5px …`），canvas 解析不了，靜靜地回落到一個**比例**字體——量出來
// `M` 是 14.2px 而 `1` 是 8px，於是每一個字都成了「超出」（2026-09-26 踩到）。
pen.font = '{SHOT_PX}px "LXGW WenKai Mono GB", monospace';
for (const cell of document.querySelectorAll(".shot i")) {{
  if (cell.style.transform) continue;          // 框綫字，已經處理過
  const box = parseFloat(cell.style.width);
  const step = pen.measureText(cell.textContent).width;
  if (step > box + 0.05) cell.style.transform = "scaleX(" + (box / step).toFixed(4) + ")";
}}
document.body.dataset.h = document.documentElement.scrollHeight;
</script>
"""


def shoot(work, out):
    """截一張**剛好那麼高**的圖。

    ⚠️ **`--screenshot` 截的是窗口，不是內容**——`--window-size=1080,4000` 出來的
    是一張下面拖着兩千像素黑地的圖。而這裏沒有 PIL 可以裁（系統 python 沒有，
    2026-09-26 查過）。

    辦法是跑兩遍：`--dump-dom` 印的是**腳本跑完之後**的 DOM，所以讓頁面把自己的高度
    寫進一個屬性，第一遍讀出來，第二遍按它截。多一次 Chrome 啓動，一秒的事。
    """
    high = subprocess.run(
        [CHROME, "--headless", "--disable-gpu", "--virtual-time-budget=2000",
         "--dump-dom", str(work)],
        capture_output=True, text=True,
    ).stdout
    found = re.search(r'data-h="(\d+)"', high)
    tall = int(found.group(1)) if found else 4000
    subprocess.run(
        [CHROME, "--headless", "--disable-gpu", "--hide-scrollbars",
         f"--screenshot={out}", f"--window-size=1080,{tall}", str(work)],
        check=True, capture_output=True,
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("version", help="例如 0.3.0")
    ap.add_argument("--notes", help="本地的發布說明，不給就問 GitHub")
    ap.add_argument("--out", help="出圖放哪，默認 local/figures/release-card-<版本>.png")
    ap.add_argument("--repo", default="forfudan/yumete")
    ap.add_argument("--size", default="88x22", help="截那一幀畫面多大")
    # ⚠️ **這一幀會被畫進卡片發出去，所以它只許出現手冊自己的內容。** 搜索面板
    # 列的是**命中那幾行的原文**——換一個查詢就等於換一批會上圖的句子。
    # **換默認之前把整幅掃一遍**（`--shot` 不帶 `--html` 就是純文本，grep 一下）。
    ap.add_argument(
        "--keys",
        default=":search\\nbiaodian\\e",
        help="截圖前先按哪幾個鍵（`--shot` 的寫法）",
    )
    ap.add_argument("--file", default="docs/manual.md", help="截圖用哪個檔")
    a = ap.parse_args()

    if a.notes:
        notes = pathlib.Path(a.notes).read_text(encoding="utf-8")
    else:
        notes = shell(["gh", "release", "view", f"v{a.version}",
                       "--repo", a.repo, "--json", "body", "--jq", ".body"])

    # 一幀真畫面。⚠️ **用倉根那個 `./yumete`**（`scripts/build.sh` 鏈的就是它），
    # 不是 `target/release/`——見記事本裏「跑的是哪個 yumete」。
    binary = HERE / "yumete"
    if not binary.exists():
        sys.exit(f"找不到 {binary}——先 cargo build --release 並拷過去")
    shot = shell([str(binary), f"--shot={a.size}", f"--keys={a.keys}",
                  "--html", a.file], cwd=HERE)
    # ⚠️ **末尾那一行版本號去掉**：`--shot` 每幀都落一句
    # 「-- yumete 0.3.0-dev.2026…+8494074.dirty」，而發布卡片上寫着 dev 和 dirty
    # 是在說「這不是你下載的那一版」。
    shot = re.sub(r"\n?<span[^>]*>-- yumete [^<]*</span>", "", shot)
    shot, off = pin(shot, int(a.size.split("x")[0]))
    if off:
        print("⚠️ 這幾行寬度對不上，圖上會歪：" + "；".join(off), file=sys.stderr)

    url = f"https://github.com/{a.repo}/releases/tag/v{a.version}"
    # **生成物進 `local/`**（2026-09-26 定的）。那個目錄本來就是為這種東西留的
    # ——`.gitignore` 裏一行 `/local/` 蓋住整棵樹，不必為每一種生成物再加一條規則，
    # 倉根也不會攢下一堆圖。
    out = pathlib.Path(a.out or HERE / "local" / "figures" / f"release-card-{a.version}.png")
    out.parent.mkdir(parents=True, exist_ok=True)
    work = out.with_suffix(".html")
    work.write_text(page(a.version, render(notes), shot, url), encoding="utf-8")

    shoot(work, out)
    work.unlink()
    print(f"{out}  ({out.stat().st_size // 1024} KB)")


if __name__ == "__main__":
    main()
