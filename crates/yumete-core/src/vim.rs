//! **vim's grammar** — the second reader of [`crate::motion`] (B3, 2026-09-20).
//!
//! The vim preset used to be a **translation table**: `x` was played as `;D`,
//! and an operator waited by remembering which keys to press next. A table can
//! give a vim hand `dw` and `dd`; it cannot give it `d$`, `df,`, `di(` — and it
//! cannot give it vim's *special cases* at all, because those are not keys.
//! Measured against a conformance corpus, six of twenty everyday presses were
//! wrong, and two of them lost text the hand did not mean to lose (`dw` at the
//! end of a line took the newline **and the next line's indent**).
//!
//! What is here instead is vim's own model, in vim's own words:
//!
//! - a **motion** names a place ([`crate::motion::Motion`]);
//! - an **operator** takes the text between here and there;
//! - and whether 「there」 is inside what it takes is the motion's **class** —
//!   `:h exclusive`, `:h inclusive`. That one word is the whole of what the
//!   translation table could not say.
//!
//! Warning: **The same key can be two motions.** 2026-09-20: 「vim 的 `w` 獨立
//! 的時候是跳轉，在命令中是選詞。」 Standalone, `w` asks
//! [`crate::motion::Reading::Caret`]; under an operator it also asks for the
//! caret's target and the operator then takes everything up to it, *excluding*
//! it. Both readings come from one motion — see [`crate::motion::Reading`].

use crate::motion::{Grain, Motion};

/// How far an operator reaches when it is given this motion — vim's own
/// classification (`:h exclusive`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// The character the motion landed on is **not** taken: `dw`, `d0`, `d{`.
    Exclusive,
    /// It is: `de`, `df,`, `d$`.
    Inclusive,
    /// Whole lines, however few characters the motion crossed: `dj`, `dG`.
    Linewise,
    /// The motion knows both its ends already: `diw`, `di(`.
    Object,
}

/// One entry of vim's motion table: which motion, and how far a verb reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub motion: Motion,
    pub reach: Reach,
    /// Whether the key still has to be told a character (`f,`, `i(`).
    pub asks: bool,
}

const fn step(motion: Motion, reach: Reach) -> Step {
    Step { motion, reach, asks: false }
}

const fn asking(motion: Motion, reach: Reach) -> Step {
    Step { motion, reach, asks: true }
}

/// **The motion a key names**, given the grain the editor is set to and the
/// character the key was told (for `f`, `t`, `i`, `a`).
///
/// `None` is 「not a motion」, and an operator waiting on one must say so rather
/// than press something else — `d` then `z` does nothing, loudly.
pub fn step_for(typed: &str, grain: Grain, told: Option<char>) -> Option<Step> {
    let g = grain;
    Some(match (typed, told) {
        // ---- Words. `w` is exclusive, `e` inclusive — vim's own pair. -----
        ("w", _) => step(Motion::WordForward(g), Reach::Exclusive),
        ("W", _) => step(Motion::WordForward(Grain::Big), Reach::Exclusive),
        ("b", _) => step(Motion::WordBack(g), Reach::Exclusive),
        ("B", _) => step(Motion::WordBack(Grain::Big), Reach::Exclusive),
        ("e", _) => step(Motion::WordEnd(g), Reach::Inclusive),
        // Warning: **`ge` 在兩套鍵位裏是兩件事**（2026-10-02 作者定）：vim 的是「往
        // 回到上一個詞的末尾」，helix 的是「到檔尾」。這張表只管 vim。
        ("ge", _) => step(Motion::WordEndBack(g), Reach::Inclusive),
        ("gE", _) => step(Motion::WordEndBack(Grain::Big), Reach::Inclusive),
        ("E", _) => step(Motion::WordEnd(Grain::Big), Reach::Inclusive),
        // ---- The line ----------------------------------------------------
        ("$", _) => step(Motion::LineEnd, Reach::Inclusive),
        ("0", _) => step(Motion::LineStart, Reach::Exclusive),
        ("^", _) => step(Motion::LineFirstNonBlank, Reach::Exclusive),
        // ---- Down the page -----------------------------------------------
        // Warning: **`h` and `l` are exclusive**, which is what makes `dl` take the
        // one character under the caret and not two: the motion lands on the
        // next cell and exclusive leaves that cell alone.
        ("l", _) | (" ", _) => step(Motion::Char { forward: true }, Reach::Exclusive),
        ("h", _) => step(Motion::Char { forward: false }, Reach::Exclusive),
        ("j", _) => step(Motion::Line { down: true }, Reach::Linewise),
        ("k", _) => step(Motion::Line { down: false }, Reach::Linewise),
        ("G", _) => step(Motion::FileEnd, Reach::Linewise),
        ("gg", _) => step(Motion::FileStart, Reach::Linewise),
        ("}", _) => step(Motion::Paragraph { forward: true }, Reach::Exclusive),
        ("{", _) => step(Motion::Paragraph { forward: false }, Reach::Exclusive),
        ("H", _) => step(Motion::Sentence { forward: false }, Reach::Exclusive),
        ("L", _) => step(Motion::Sentence { forward: true }, Reach::Exclusive),
        // ---- Find, which is told a character ------------------------------
        //
        // Warning: **`t` is `f` one short**, and that is a property of the motion,
        // not a patch on the span: vim's `t,` lands *before* the comma, so
        // `dt,` leaves it and `df,` takes it.
        ("f", None) | ("F", None) | ("t", None) | ("T", None) => {
            asking(Motion::Find { forward: true, target: ' ', till: false }, Reach::Inclusive)
        }
        ("f", Some(c)) => {
            step(Motion::Find { forward: true, target: c, till: false }, Reach::Inclusive)
        }
        // Warning: **往回的那兩個是排他的**（`:h F`、`:h T`），往前的兩個纔是包含
        // 的。`dT,` 緊貼在逗號後面按，vim 什麼都不做；標成包含的話它會刪掉一格。
        ("F", Some(c)) => {
            step(Motion::Find { forward: false, target: c, till: false }, Reach::Exclusive)
        }
        ("t", Some(c)) => {
            step(Motion::Find { forward: true, target: c, till: true }, Reach::Inclusive)
        }
        ("T", Some(c)) => {
            step(Motion::Find { forward: false, target: c, till: true }, Reach::Exclusive)
        }
        // ---- Text objects, which are their own ends ------------------------
        ("i", None) | ("a", None) => asking(
            Motion::Object { what: crate::motion::Object::Word { coarse: false }, around: false },
            Reach::Object,
        ),
        ("i", Some(c)) => step(object(c, false)?, Reach::Object),
        ("a", Some(c)) => step(object(c, true)?, Reach::Object),
        _ => return None,
    })
}

/// `iw` / `i(` — the object a character names.
fn object(c: char, around: bool) -> Option<Motion> {
    let what = match c {
        // Warning: **`w` 和 `W` 分開了**（2026-09-28）：`iw` 走分詞器，`iW` 是 vim 那個
        // 「一串非空白」。從前兩個折成同一個，於是 `diW` 和 `diw` 一模一樣。
        'w' => crate::motion::Object::Word { coarse: false },
        'W' => crate::motion::Object::Word { coarse: true },
        'p' => crate::motion::Object::Paragraph,
        // `dim`／`dam`：光標所在的那一段 Markdown 標記（2026-09-28）。
        'm' => crate::motion::Object::Markup,
        // `dis`／`das`：光標所在的那一句（2026-09-28）。
        's' => crate::motion::Object::Sentence,
        c => {
            let (open, close) = crate::editor::pair_for(c)?;
            crate::motion::Object::Pair { open, close }
        }
    };
    Some(Motion::Object { what, around })
}

/// Whether more keys could still make a motion (`g` before `gg`).
pub fn more_ahead(typed: &str) -> bool {
    ["gg", "ge", "gE"].iter().any(|k| k.len() > typed.len() && k.starts_with(typed))
}
