<!-- help: 70-簡繁-conversion-convert -->

# 簡繁 conversion: `:convert`

`:convert s t` turns the whole manuscript from 簡體 into 繁體, `:convert t s` the other
way.

**yumete does not do this itself.** 发 is 髮 in 头发 and 發 in 发现; 干 is 乾, or 幹, or
still 干; 里 is 里 or 裏 depending on whether it is the 里 of a road or the 裏 of inside. No
table of characters can answer that — it takes a dictionary and segmentation, and that
dictionary, [OpenCC](https://github.com/BYVoid/OpenCC), has been doing the work properly
for fifteen years. So `:convert` goes and runs `opencc`: writing another one would only
produce a worse OpenCC, and a manuscript has to trust it.

With opencc not installed it says in one line how to install it and changes nothing.
`:convert-opencc-install` will run `brew install opencc` for you on macOS; on other
systems it only prints the command, because that needs sudo, and a full-screen editor is
not the place to be asked for your password. Once it is installed, opencc has to be on
`PATH` for `:convert` to find it — `:convert-opencc` says whether it is there, where, and
which version.

Seven codes, the first five of them opencc's own configuration names:

| | |
| --- | --- |
| `s` | 簡體 — 说 为 内 吴 里 发 台 |
| `t` | opencc's plain 繁體 — 說 爲 內 吳 裏 髮 臺 |
| `tw` | 臺灣正體 — 說 為 內 吳 裡 髮 臺 |
| `hk` | 香港繁體 — 説 為 內 吳 裏 髮 台 |
| `jp` | 日本新字体 — 説 為 内 呉 裏 髪 台 |
| `c` | **大陸通規繁體** — 説 爲 内 吴 裏 髮 臺 |
| `g` | **古籍通規繁體** — 説 爲 内 吳 裏 髮 臺 |

`:convert` with no arguments lists these seven and where each one can go. opencc has no
route between every two of them — `jp` only comes back to 繁體 — and what is listed is
what it can do.

**`c` and `g` are added here.** OpenCC's 繁體 is the 港臺 glyph shapes, while a mainland
publisher setting a 繁體 book uses the 通規 shapes, and 宇浩's own data is written to 通規
as well. What differs is **the glyph, not the word**:
<!-- verbatim -->説 對 說、内 對 內、吴 對 吳<!-- verbatim -->, a few dozen characters, one for
one. Something that small ought to be data, so it is a table — `:convert s c` has
opencc run `s2t` first, then sweeps the glyphs with that table. The table is taken
from [GujiCC](https://github.com/forFudan/GujiCC).

Starting from `c` works too: that table is walked **backwards** first, back to glyphs
opencc knows, and then handed over. So `:convert c tw` is 「通規 → 繁體 → 臺灣正體」. A few
characters will not go back —
<!-- verbatim -->通規把 蝨 併進 虱、把 嶽 併進 岳<!-- verbatim -->, and a 虱 has no one place it
came from — and those are left as they stand rather than guessed at.

**The bang changes words, not characters.** A few of opencc's configurations carry a
`p` (`s2twp`, `tw2sp`) and convert the vocabulary along with it: 内存 becomes 記憶體, 鼠標
becomes 滑鼠. That is a different thing from everything above — it changes the 字 count,
and nothing reverses it — so it has to be asked for: `:convert! s tw`. For the pairs with
no `p` configuration, the bang is refused outright.

**To take it back, `u`.** One conversion is one undo, and the whole manuscript comes
back together. **Never use the reverse direction as an undo**: `s t` and then
`空格 t s` does not give you the manuscript you had, it gives you what it looks like
after the round trip.

---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
