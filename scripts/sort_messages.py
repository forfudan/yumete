#!/usr/bin/env python3
"""把 messages.toml 裏排錯位置的那幾則搬回去。

⚠️ 2026-09-22 第三次被同一條測試攔下來（`the_table_is_in_order`）。靠腦子算
字母序是算不對的——`ui.looking-it-up` 該排在 `ui.no-screenshot-command` 之前
還是之後，人一眼看不出，機器一秒鐘。**往後加文案就跑這個。**
"""
import sys, pathlib, re
p = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "crates/yumete-core/messages.toml")
s = p.read_text()
head_end = s.index("[[message]]")
head, body = s[:head_end], s[head_end:]
blocks = re.split(r"(?=\[\[message\]\])", body)
blocks = [b for b in blocks if b.strip()]
def key_of(b):
    m = re.search(r'^key = "([^"]+)"', b, re.M)
    return m.group(1) if m else ""
before = [key_of(b) for b in blocks]
order = sorted(blocks, key=key_of)
after = [key_of(b) for b in order]
if before == after:
    print("已經是有序的")
    sys.exit(0)
moved = [k for k, j in zip(before, after) if k != j]
p.write_text(head + "".join(order))
print(f"搬了 {len(blocks)} 則裏的若干條；最早出現分歧的是：{moved[:3]}")
