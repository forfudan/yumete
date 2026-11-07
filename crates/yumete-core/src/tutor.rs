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

/// The lesson, in Chinese, as written: Simplified, in mainland usage
/// (2026-11-07). Written as a manuscript rather than as a reference: each
/// section is a paragraph to read and a line to do something to, in the order
/// a writer meets them. The reader is expected to break it — that is the point
/// of a copy.
pub const LESSON_ZHS: &str = include_str!("tutor_zhs.md");

/// The same lesson in Traditional, made from [`LESSON_ZHS`] at build time the
/// way the messages' Traditional is (`build.rs`: s2t, the 大陸通規 glyph
/// table, the term fixes).
pub const LESSON: &str = include_str!(concat!(env!("OUT_DIR"), "/tutor_zht.md"));

/// **The lesson, in English** (2026-10-29), for a reader whose interface is
/// English: the same ten steps, practised on English prose, with the Chinese
/// input method and the vertical page kept to one section each.
pub const LESSON_EN: &str = r#"# The yumete lesson

This is **your own file**. Everything you do here changes it and nothing else.
To start over: `:tutor` again opens a fresh copy.

To leave: `:q`. To keep your changes, `:w` first; the file already has a name.

────────────────────────────────────────────────────────────

## 1. Two modes, one Esc

You are in **NORMAL** now: the letters on the keyboard are commands.
Press `i` for **INSERT**, where keys type text. `Esc` goes back.

  ⇒ Move to the line below, press `i`, type a few words, press `Esc`.

    This line is yours to change.

Broke something? Press `u`, and again. **`u` undoes**, `U` redoes.
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

`v` turns on Select mode: moves grow the selection. `;` shrinks it to one
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

`/` opens a search; type and press Enter. `n` the next one, `N` the previous one.

  ⇒ Press `/`, type `snow`, Enter, then `n` a few times.

With something selected, **`g/` asks where else it is** — no retyping.

  ⇒ Select a `snow` above and press `g/`, then `n` for the next one.

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
`:q` with unsaved changes stops you. To discard them: `:q!`.

What you edit is backed up every few seconds; after a crash
yumete asks next time whether to `:recover` it.

`空格 f` finds a file, `空格 e` browses, `空格 b` switches files,
`:sidebar explorer` opens the sidebar. `:count` says how long the file is
(or the selection, when there is one).

────────────────────────────────────────────────────────────

## 10. Next

    `:help`            the keys you will use
    `:help commands`   every : command
    `::`               search the commands by what they do

Stop halfway through a key sequence and the corner of the screen tells you what can
come next; `g`, `m` and Space open a table of them.

That is the lesson. This file is yours: change anything.
"#;

/// The lesson in the interface's language: English for English, Chinese
/// otherwise.
pub fn lesson() -> &'static str {
    match crate::messages::language() {
        crate::messages::Language::English => LESSON_EN,
        crate::messages::Language::Simplified => LESSON_ZHS,
        crate::messages::Language::Traditional => LESSON,
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
            "`gg`", "`ge`", "`30G`", "`g/`", "`n`", "`:w`", "`:q`", "`:vertical`",
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
            "`gg`", "`ge`", "`30G`", "`g/`", "`n`", "`:w`", "`:q`", "`:vertical`",
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
