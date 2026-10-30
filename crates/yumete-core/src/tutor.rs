//! `:tutor` — a lesson you learn by editing it.
//!
//! `vimtutor` and `hx --tutor` are the same good idea: **a copy of a file whose
//! text tells you what to press, and you learn by pressing it on that file.**
//! No dialog, no video, no separate mode — the tutorial *is* a document, and
//! editing it is the lesson.
//!
//! It fits this editor better than it fits either of them, because half of what
//! yumete does can only be taught on Chinese prose: `w` means nothing on `the
//! quick brown fox`, and everything on 「他抬頭看了看那片天」. The IME cannot be
//! described, only typed. 縱書 has to be flipped into, mid-lesson, on the
//! reader's own screen.
//!
//! **It is a real file the reader owns** — written into the data directory, not
//! into a scratch buffer — so `:w` works, `u` is part of lesson one, and every
//! destructive key in it is safe to try.

/// The lesson, in Chinese.
///
/// Written as a manuscript rather than as a reference: each section is a
/// paragraph to read and a line to do something to, in the order a writer meets
/// them. The reader is expected to break it — that is the point of a copy.
pub const LESSON: &str = r#"# 宇夢小課堂

這是一份**你自己的檔案**。你在這裏做的每一件事都改的是它，不是別的稿子。
想從頭來過：`:tutor` 再開一份新的。

想退出：`:q`。捨不得改動就先 `:w`，它已經有名字了。

────────────────────────────────────────────────────────────

## 一、兩種狀態，一個 Esc

現在你在 **NORMAL**：鍵盤上的字母是命令，不是字。
按 `i` 進 **INSERT**，這時打字就是打字。按 `Esc` 回來。

  ⇒ 把光標移到下面這一行，按 `i`，隨便打幾個字，按 `Esc`。

    這一行是給你改的。

打壞了？按 `u`。再按 `u`。**`u` 一直往回走**，`U` 往前走。
這一條先記住，後面所有的練習都有退路。

────────────────────────────────────────────────────────────

## 二、走

`h` `j` `k` `l` 是左下上右。方向鍵也行，但手不用離開。

漢字沒有空格，所以「一個詞」是算出來的：

  ⇒ 光標放到下面這一行的行首，按三次 `w`，看它停在哪裏。

    他抬頭看了看那片天，雪還在下，山路已經看不見了。

`w` 下一個詞首，`b` 上一個詞首，`e` 這個詞的末尾。
`3w` 是走三次——**數字在前面就是次數**。

一行太長的時候：`gh` 行首，`gl` 行尾，`gs` 第一個不是空白的字。
整份稿子：`gg` 開頭，`ge` 結尾，`30G`（或者 `g30g`）去第 30 行。
`J` `K` 翻半頁，`L` `H` 翻整頁。

────────────────────────────────────────────────────────────

## 三、選區——每一次移動都留下一個

這是這個編輯器和別的不一樣的地方：**你先選，再說要拿它怎麼辦**。
按 `w` 就選中了那個詞；這時候按 `d`，刪掉的就是它。

  ⇒ 在下面這一行上：按 `w` 選一個詞，按 `d` 刪掉，按 `u` 還原。

    昨天的雪把院子裏那棵老槐樹壓斷了一根枝。

  ⇒ 同一行：按 `x` 選整行，按 `c` 換掉它——`c` 會直接讓你打字。

`v` 打開延伸模式：再走的時候選區跟着長。`;` 把它收回成一個字。

────────────────────────────────────────────────────────────

## 四、拿選區怎麼辦

    `d` 刪掉      `c` 換掉（順手進 INSERT）
    `y` 複製      `p` 貼在後面
    `>` 縮進      `<` 退回去

`.` 是「重複上一次輸入」：進入輸入的那個鍵和打的字。

  ⇒ 在一個詞前面按 `i`，打「很」，`Esc`；換到另一個詞前面，按 `.`。

────────────────────────────────────────────────────────────

## 五、找

`/` 開搜索，打完按 Enter；`n` 下一處，`N` 上一處。

  ⇒ 按 `/`，打「雪」，Enter，再按幾次 `n`。

選中一段之後，**`g/` 就是「這個詞還在哪裏」**——不用重打一遍。
`g?` 是同一個問題，但答案畫在**另一個工作區**裏，你不用離開現在這裏。

  ⇒ 選中上面某一行裏的「雪」，按 `g?`。看右邊（或下邊）那一半。
  ⇒ 按 `n` 走下一處。看夠了按 `Esc` 關掉那一半；再按 `n`，它自己回來。
  ⇒ 真想過去改它：`空格 w`。

換字：`:s/舊/新/` 換這一行的第一處，`:%s/舊/新/g` 換全篇。

────────────────────────────────────────────────────────────

## 六、打漢字

`:yume off` 把鍵盤整個交還給系統，`:yume on` 拿回來。出廠是拿着的。
拿着的時候，單獨輕按一下 **Shift** 在中/英之間切。

  ⇒ 按 `i` 進 INSERT，在下面這一行後面打幾個字。

    我要在這裏寫一句話：

打不出來？狀態列會說輸入法有沒有裝好；`:yume` 說得更詳細。
`:yume-scheme 靈明` 換方案（靈明、星陳、卿雲、日月、拼音）。

────────────────────────────────────────────────────────────

## 七、把它當成一本書

  ⇒ 按 `:vertical`，Enter。

現在是**竪排**。`h` `l` 換縱（縱是往左疊的），`j` `k` 沿着這一縱走。
`:view-wrap 24` 定一縱多少字，`:view-pages-vertical 2` 把一頁分成上下兩段。
`:view-hanging on` 讓句讀掛到字旁邊去，像排印的書。
按 `:horizontal` 回橫排。

  ⇒ 按 `:indent full`，Enter。段首空了兩格，段間那些空行收起來了——
     檔案沒有變，變的是版面。`:indent basic` 只縮進、空行留着，
     `:indent off` 回去。

────────────────────────────────────────────────────────────

## 八、表格

下面是一張表。把光標放進去，按 `:table-render window`，Enter。

| 字 | 拆分   | 說明     |
| -- | ------ | -------- |
| 木 | 木     | 樹       |
| 相 | ⿰木目 | 看       |
| 杏 | ⿱木口 | 果       |

`h` `j` `k` `l` 預設按**字**走，`T` 換成按格；`Tab` 走下一格。
`i` 進格子打字，`c` 換掉整格，`空格 t r` 加一行，`空格 t d` 刪一行
（`空格 t o` 是退回源碼）。

  ⇒ 光標放到「相」那一行的拆分格上，走到「木」上按 `空格 t /`：
     找哪些字的拆分裏用了「木」。`空格 t ?` 是同一個問題，答案畫在另一個工作區。
     （`空格 t` 是表格的命令組，`g` 是全文的——`gd` `g/` 在表格裏也只認整份稿子。
     `t` 在表格裏和在正文裏一樣，是 vi 的 till。）

  ⇒ `:table-jump 木` 跳到「木」自己那一行，`C-o` 回來。

表頭上面那一行數字是**欄號**：`空格 t 20,20g` 去第 20 行第 20 欄，
`空格 t 1s` 照第一欄排（`1a2d8as` 一次排幾欄，`0s` 照你站的這一欄），
`空格 t 3/` 只在第 3 欄裏找（`2-10?` 是第 2–10 欄）。
`:table-render off` 退出。

────────────────────────────────────────────────────────────

## 九、不弄丟東西

`:w` 存檔，`:q` 退出，`:wq` 存了再退。
改過沒存就 `:q`，它會攔住你。真的不要了：`:q!`。

你正在編輯的東西每隔一會兒就會被抄一份到旁邊；萬一斷電，
下次打開它會問你要不要 `:recover`。

`空格 f` 找檔案，`空格 e` 翻目錄，`空格 b` 換檔案，`:sidebar explorer` 開邊欄。

────────────────────────────────────────────────────────────

## 十、接下來

    `:help`            常用的鍵，都在裏面
    `:help chinese`    漢字、標點、注音、輸入法
    `:help vertical`   竪排
    `:help table`      表格
    `:help commands`   所有的 : 命令

按錯了鍵不知道接下來能按什麼？打一半停住，右下角和光標旁邊會告訴你，
`g`、`m`、`空格` 還會彈出一張表。

課上完了。這份檔案是你的，隨便改。
"#;

/// **The lesson, in English** (2026-10-29), for a reader whose interface is
/// English: the same ten steps, practised on English prose, with the Chinese
/// input method and the vertical page kept to one section each.
pub const LESSON_EN: &str = r#"# The yumete lesson

This is **your own file**. Everything you do here changes it and nothing else.
To start over: `:tutor` again opens a fresh copy.

To leave: `:q`. To keep your changes first: `:w` — it already has a name.

────────────────────────────────────────────────────────────

## 1. Two modes, one Esc

You are in **NORMAL** now: the letters on the keyboard are commands.
Press `i` for **INSERT**, where typing is typing. `Esc` comes back.

  ⇒ Move to the line below, press `i`, type a few words, press `Esc`.

    This line is yours to change.

Broke something? Press `u`. Again `u`. **`u` walks back**, `U` walks forward.
Remember this one: every exercise below has a way back.

────────────────────────────────────────────────────────────

## 2. Moving

`h` `j` `k` `l` are left, down, up, right. The arrow keys work too.

  ⇒ Put the cursor at the start of the line below and press `w` three times.

    The snow had not stopped, and the road over the hill was gone.

`w` goes to the next word, `b` to the one before, `e` to the end of this one.
`3w` is three of them — **a number in front is a count**.

On a long line: `gh` its start, `gl` its end, `gs` its first non-blank.
The whole file: `gg` the top, `ge` the end, `30G` (or `g30g`) line 30.
`C-d` `C-u` scroll half a page, `C-f` `C-b` a whole one.

────────────────────────────────────────────────────────────

## 3. Selections — every move leaves one

This is where yumete differs from vim: **you select first, then say what to
do with it**. `w` has selected the word; press `d` now and that word goes.

  ⇒ On the line below: `w` to select a word, `d` to delete it, `u` to bring
     it back.

    Yesterday the snow broke a branch off the old tree in the yard.

  ⇒ Same line: `x` selects the whole line, `c` replaces it and lets you type.

`v` turns on extending: moves grow the selection. `;` shrinks it to one
character.

────────────────────────────────────────────────────────────

## 4. What to do with a selection

    `d` delete      `c` change (and type)
    `y` copy        `p` paste after it
    `>` indent      `<` outdent

`.` types the last insert again: the key that started it and what you typed.

  ⇒ Put the cursor on a word, press `i`, type `very `, press `Esc`. Move to
     another word and press `.`.

Prefer vim's keys? `:keymap vim` — there `.` repeats any change, as vim's does.

────────────────────────────────────────────────────────────

## 5. Finding

`/` opens a search; type and press Enter. `n` the next one, `N` the last one.

  ⇒ Press `/`, type `snow`, Enter, then `n` a few times.

With something selected, **`g/` asks where else it is** — no retyping.
`g?` asks the same, and draws the answer in **the other work area**, so you
stay where you are.

  ⇒ Select a `snow` above and press `g?`. Look at the other half.
  ⇒ `n` goes to the next one. `Esc` closes that half.

Replacing: `:s/old/new/` on this line, `:%s/old/new/g` in the whole file.

────────────────────────────────────────────────────────────

## 6. Chinese, if you write it

yumete has a Chinese input method built in. Writing only English? `:yume off`
hands the keyboard back to your system, and it stays that way. `:yume on`
takes it again; then a lone tap of **Shift** switches between Chinese and
English.

────────────────────────────────────────────────────────────

## 7. The page

  ⇒ `:view-wrap 60`, Enter: lines wrap at sixty columns.
  ⇒ `:view-focus on`, Enter: everything but this paragraph fades.
     `:view-focus off` brings it back.
  ⇒ `:view-typewriter on`: the line you write stays in the middle of the
     screen.

`:vertical` sets the page vertically, the way a Chinese book is set;
`:horizontal` comes back.

────────────────────────────────────────────────────────────

## 8. Tables

Below is a table. Put the cursor in it and type `:table-render window`, Enter.

| Word  | Count | Note     |
| ----- | ----- | -------- |
| snow  | 4     | weather  |
| road  | 2     | place    |
| tree  | 1     | thing    |

`Tab` goes to the next cell. `i` types in a cell, `c` replaces it,
`空格 t r` adds a row, `空格 t d` deletes one, `空格 t o` shows the source.
`空格 t /` finds what is selected, down the columns (`空格 t 3/` searches
column 3 only). `:table-render off` leaves.

(`空格` is the space bar: `空格 t r` is Space, then `t`, then `r`.)

────────────────────────────────────────────────────────────

## 9. Not losing anything

`:w` saves, `:q` quits, `:wq` does both.
`:q` with changes unsaved stops you. Really mean it: `:q!`.

What you are editing is copied beside it every few seconds; after a crash
the next open asks whether to `:recover` it.

`空格 f` finds a file, `空格 e` browses, `空格 b` switches files,
`:sidebar explorer` opens the sidebar. `:count` says how long the file is
(or the selection, when there is one).

────────────────────────────────────────────────────────────

## 10. Next

    `:help`            the keys you will use
    `:help commands`   every : command
    `::`               search the commands by what they do

Stop halfway through a key and the corner of the screen tells you what can
come next; `g`, `m` and Space open a table of them.

That is the lesson. This file is yours: change anything.
"#;

/// The lesson in the interface's language: English for English, Chinese
/// otherwise.
pub fn lesson() -> &'static str {
    match crate::messages::language() {
        crate::messages::Language::English => LESSON_EN,
        _ => LESSON,
    }
}

/// The lesson's own file name, numbered so a second `:tutor` starts fresh.
pub fn file_name(n: usize) -> String {
    match n {
        0 => "tutor.md".to_string(),
        n => format!("tutor-{}.md", n + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lesson_teaches_the_keys_it_says_it_does() {
        // Every key the lesson names has to be a key that exists: a tutorial
        // that teaches a keystroke the editor does not have is worse than none.
        for key in [
            "`i`", "`Esc`", "`u`", "`w`", "`b`", "`e`", "`x`", "`c`", "`d`", "`y`", "`p`",
            "`gg`", "`ge`", "`30G`", "`g/`", "`g?`", "`n`", "`:w`", "`:q`", "`:vertical`",
            "`:table-render window`", "`gd`", "`空格 t /`", "`:help`", "`空格 f`",
        ] {
            assert!(LESSON.contains(key), "the lesson never mentions {key}");
        }
        // …and it is a document, not a reference: it has to *ask* the reader to
        // do things.
        assert!(LESSON.matches("⇒").count() >= 8, "not enough to do");
    }

    /// The English lesson teaches the same keys, and asks as much.
    #[test]
    fn the_english_lesson_teaches_the_keys_too() {
        for key in [
            "`i`", "`Esc`", "`u`", "`w`", "`b`", "`e`", "`x`", "`c`", "`d`", "`y`", "`p`",
            "`gg`", "`ge`", "`30G`", "`g/`", "`g?`", "`n`", "`:w`", "`:q`", "`:vertical`",
            "`:table-render window`", "`.`", "`空格 t /`", "`:help`", "`空格 f`",
        ] {
            assert!(LESSON_EN.contains(key), "the English lesson never mentions {key}");
        }
        assert!(LESSON_EN.matches("⇒").count() >= 8, "not enough to do");
    }

    #[test]
    fn a_second_lesson_gets_its_own_name() {
        assert_eq!(file_name(0), "tutor.md");
        assert_eq!(file_name(1), "tutor-2.md");
    }
}
