//! **What changed, by word** (Feature #235).
//!
//! Every diff a writer can reach is a *line* diff, and a Chinese paragraph is
//! one line. Move one 的 in a 500-字 段落 and `git diff` paints the whole
//! paragraph red and then green again: the answer is technically true and
//! completely useless, because the one thing the writer wanted to see — which
//! word moved — is buried in five hundred characters of noise.
//!
//! So the unit here is the 詞, not the line. The tokens are whatever the
//! editor's own [`yumete_cjk::Segmenter`] says the words are, which is the same
//! segmentation `w` steps over and the overlay draws, so the diff agrees with
//! what the reader already sees. A line break is a token like any other, which
//! is what lets a split paragraph read as one insertion rather than as two
//! rewritten lines.
//!
//! The algorithm is Myers's O(ND) — the same one `git` uses — over that token
//! stream, after the shared head and tail are trimmed off. `D` is the number of
//! word-level edits, which for an afternoon's revision is tens; a manuscript
//! that was replaced wholesale hits [`MAX_D`] and gets an honest 「差得太遠」
//! instead of an hour of work.

/// How many word-level edits are still worth showing one at a time. Past this
/// the two texts are not two drafts of the same thing, and the memory the
/// trace costs (`O(D²)`) stops being worth spending.
pub const MAX_D: usize = 1200;

/// One token's fate in the edit script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op<'a> {
    /// In both, unchanged.
    Same(&'a str),
    /// In the old text only.
    Gone(&'a str),
    /// In the new text only.
    New(&'a str),
}

/// One line of the new text that differs from the old, rendered the way
/// `wdiff` renders one: `[-什麼走了-]` and `{+什麼來了+}` inline, everything
/// else as it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineChange {
    /// 0-based line in the **new** text. A line that only lost text is
    /// reported at the line it was deleted from.
    pub line: usize,
    /// The line with the two markers in it.
    pub marked: String,
}

/// What was removed, wrapped so it reads as removed.
const GONE: (&str, &str) = ("[-", "-]");
/// What was added.
const NEW: (&str, &str) = ("{+", "+}");
/// A line break, where one was added or taken away. Drawn, because an
/// invisible change is the one a writer cannot check.
const BREAK: &str = "⏎";

/// **One row of one draft** — half of a 「before and after」 pair (2026-10-16).
///
/// 原話：「It first list the paragraph before and after, than mark the words
/// changed in brighter colors.」
///
/// **一份，三處畫**：`空格 x` 的浮窗、`空格 X` 的邊欄、`:git-diff` 的報告。三
/// 處從前各算各的，於是行號、記號、底色的響度和折不折行四樣都不一樣（作者
/// 2026-10-16 逐條報來）。所以算在這裏，畫在那三處，差別只剩「怎麽畫」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftRow {
    /// 1-based line number **in its own draft** — the old file's for a `gone`
    /// row, the new file's for the other.
    ///
    /// 兩個號都留着，因為它們本來就不是一個數：一段往下挪過四行，`537` 與
    /// `533` 之差正是那四行。跳轉不照這個號走，照「這一處改動現在在哪」走
    /// （`gf`，2026-10-16 定，同 magit／fugitive 那一族）。
    pub line: usize,
    /// The old draft's row (`-`) rather than the new one's (`+`).
    pub gone: bool,
    /// 這一行的字，不帶行號也不帶記號——那兩樣各處自己加。
    pub text: String,
    /// 這一行裏改掉的那幾段，相對 [`Self::text`] 的**字節**區間。
    ///
    /// 連着的同命 token 併成一段：`[-很早-][-很早-]` 讀起來是兩處改動。
    pub marks: Vec<std::ops::Range<usize>>,
}

/// **Every changed row of both drafts, paired** — what all three views draw.
///
/// Two passes. The first is Myers over **lines**, which is cheap: `D` counted
/// in lines is tens even for an afternoon that rewrote every paragraph, where
/// `D` counted in words ran past [`MAX_D`] and the whole command answered
/// 「差得太遠」 (reported 2026-10-16: 「I cannot see the git diff when the diff
/// is many? That is very inconvienent.」). The second is Myers over the
/// **words** of one paired line, where `D` cannot exceed the line.
///
/// Rows come back in reading order, a pair at a time: the old row then the new
/// one. A line that was only added or only taken away is one row, not two.
///
/// Warning: **一段切成兩段，從前讀成一筆插入，現在讀成一行改寫加一行新增。** 詞流那
/// 一套把換行也當一個詞，所以看得出「這裏多了一個換行」；配行先走就看不出了。
/// 這是這一刀的代價，作者 2026-10-16 選的。
///
/// `old_at`/`new_at` are the 1-based numbers of each side's first line — `1`
/// for a whole file, the hunk's own start for `空格 x`.
///
/// `None` only when the two differ by more than [`MAX_D`] **lines**, which is
/// a different file rather than another draft of this one.
pub fn drafts(
    old: &[&str],
    old_at: usize,
    new: &[&str],
    new_at: usize,
    segment: &dyn Fn(&str) -> Vec<(usize, usize)>,
) -> Option<Vec<DraftRow>> {
    let ops = script(old, new)?;
    let mut out: Vec<DraftRow> = Vec::new();
    let (mut was, mut now) = (old_at, new_at);
    let mut gone: Vec<&str> = Vec::new();
    let mut added: Vec<&str> = Vec::new();

    for op in &ops {
        match *op {
            Op::Same(_) => {
                settle(&mut out, &mut was, &mut now, &mut gone, &mut added, segment);
                was += 1;
                now += 1;
            }
            Op::Gone(t) => gone.push(t),
            Op::New(t) => added.push(t),
        }
    }
    settle(&mut out, &mut was, &mut now, &mut gone, &mut added, segment);
    Some(out)
}

/// One block of 「these old lines became those new ones」, paired off.
///
/// Pairing is positional, which is what a reader expects of a block of
/// rewritten paragraphs: the first old line is the first new one's earlier
/// draft. Whatever is left over at either end was only added or only taken
/// away, and gets one row rather than two.
fn settle(
    out: &mut Vec<DraftRow>,
    was: &mut usize,
    now: &mut usize,
    gone: &mut Vec<&str>,
    added: &mut Vec<&str>,
    segment: &dyn Fn(&str) -> Vec<(usize, usize)>,
) {
    for i in 0..gone.len().max(added.len()) {
        let (old, new) = (gone.get(i).copied(), added.get(i).copied());
        // Both halves in hand: mark each side with its own losses and gains.
        let (lost, got) = match (old, new) {
            (Some(o), Some(w)) => two_drafts(o, w, segment),
            // One side only — the whole row is the change, and saying so a
            // second time word by word is saying it twice.
            _ => (Vec::new(), Vec::new()),
        };
        if let Some(text) = old {
            out.push(DraftRow { line: *was + i, gone: true, text: text.to_string(), marks: lost });
        }
        if let Some(text) = new {
            out.push(DraftRow { line: *now + i, gone: false, text: text.to_string(), marks: got });
        }
    }
    *was += gone.len();
    *now += added.len();
    gone.clear();
    added.clear();
}

/// Two drafts of one line: which byte ranges each side lost and gained.
fn two_drafts(
    old: &str,
    new: &str,
    segment: &dyn Fn(&str) -> Vec<(usize, usize)>,
) -> (Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>) {
    let (a, b) = (tokens(old, segment), tokens(new, segment));
    // One line more than `MAX_D` words from the other is not a redraft of it:
    // the whole row is the change, which the row's own ground already says.
    let Some(ops) = script(&a, &b) else {
        return (Vec::new(), Vec::new());
    };
    let (mut lost, mut got) = (Vec::new(), Vec::new());
    let (mut at_old, mut at_new) = (0usize, 0usize);
    // 連着的同命 token 併成一段。
    let mark = |runs: &mut Vec<std::ops::Range<usize>>, span: std::ops::Range<usize>| {
        match runs.last_mut() {
            Some(last) if last.end == span.start => last.end = span.end,
            _ => runs.push(span),
        }
    };
    for op in &ops {
        match *op {
            Op::Same(t) => {
                at_old += t.len();
                at_new += t.len();
            }
            Op::Gone(t) => {
                mark(&mut lost, at_old..at_old + t.len());
                at_old += t.len();
            }
            Op::New(t) => {
                mark(&mut got, at_new..at_new + t.len());
                at_new += t.len();
            }
        }
    }
    (lost, got)
}

/// **這一份報告裏有幾行和那一稿不一樣**——一對算一行。
pub fn changed_lines(rows: &[DraftRow]) -> usize {
    rows.iter()
        .enumerate()
        .filter(|(i, row)| !row.gone || rows.get(i + 1).is_none_or(|next| next.gone))
        .count()
}

/// One row written out with its changed runs wrapped, the way the listing
/// holds them: `[-走了-]` on an old row, `{+來了+}` on a new one.
///
/// The listing is **text** and stays text, so it can be yanked, searched and
/// read by anything that reads a file; the markers are what [`spans`] reads
/// back to paint it (#499).
pub fn marked(row: &DraftRow) -> String {
    let marks = match row.gone {
        true => GONE,
        false => NEW,
    };
    let mut out = String::new();
    let mut at = 0usize;
    for span in &row.marks {
        let (start, end) = (span.start.min(row.text.len()), span.end.min(row.text.len()));
        if start < at || start >= end {
            continue;
        }
        out.push_str(&row.text[at..start]);
        out.push_str(marks.0);
        out.push_str(&row.text[start..end]);
        out.push_str(marks.1);
        at = end;
    }
    out.push_str(&row.text[at..]);
    out
}

/// The coloured runs of one line of a `:diff` listing (#499).
///
/// The listing is **text**, and stays text: `[-走了-]{+來了+}` is what the
/// buffer holds, so it can be yanked, searched and read by anything that reads
/// a file. What changes is how it is *drawn* — the four marker characters come
/// off the page the way `**` does, and what was between them is painted 朱 or
/// 綠. 「红色表示删除，绿色表示新增……对于修改的字加红/绿底色。」
///
/// Everything outside a pair is prose and gets no span, including the
/// `yume.md:120:` the line opens with — it is a place, not a change.
///
/// Warning: **Only reached through [`crate::syntax::Syntax::Diff`]**, which nothing
/// but the listing is given. A manuscript may perfectly well contain `[-`.
pub fn spans(line: &str) -> Vec<crate::markdown::Span> {
    use crate::markdown::{Kind, Span};
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut at = 0usize;
    let mut construct = 0usize;
    // **行號後面那一個 `-`/`+` 說這一整行是哪一稿**（2026-10-16）。`537-` 是
    // 舊稿、`533+` 是新稿，同 `空格 x` 那一扇；號碼靠右對齊，所以記號在前導的
    // 空格與數字之後。記號本身**不**畫出來——同 `[-`，它是報告的腳手架。
    let sigil = chars
        .iter()
        .position(|c| !c.is_ascii_digit() && *c != ' ')
        .filter(|&i| i > 0 || chars.first().is_some_and(|c| *c == '-' || *c == '+'));
    let row = sigil.and_then(|i| match chars.get(i) {
        Some('-') => Some((i, Kind::GoneRow)),
        Some('+') => Some((i, Kind::AddedRow)),
        _ => None,
    });
    if let Some((mark, kind)) = row {
        // Warning: **`-`/`+` 本身是畫出來的，不是記號。** 它和號碼一起，就是
        // `空格 x` 那一扇裏的同一截（2026-10-16 作者更正：「in yumete, `space
        // x` put + and - before the paragraph」）。畫不出來的只有 `[-`／`{+`
        // 那四個——它們是這一支一分鐘前寫下去的腳手架，而顏色說得更好。
        //
        // 底色從行頭鋪到行尾，號碼和記號也在上面：一整行是一稿。
        at = mark + 1;
        // **底色鋪在沒有標記的那幾截上**，不鋪在標記上：改過的詞自己那一層更
        // 響（`short_wash` 對 `washed`），兩層疊在一格上誰贏要看畫的次序，而
        // 不疊就不必問。
        let mut plain = 0;
        let mut walk = at;
        let marked = |i: usize| -> Option<usize> {
            let pair = [(('[', '-'), ('-', ']')), (('{', '+'), ('+', '}'))]
                .into_iter()
                .find(|&(open, _)| chars.get(i) == Some(&open.0) && chars.get(i + 1) == Some(&open.1))?;
            let text = i + 2;
            let end = (text..chars.len().saturating_sub(1))
                .find(|&j| chars[j] == pair.1 .0 && chars[j + 1] == pair.1 .1)?;
            (end > text).then_some(end + 2)
        };
        while walk < chars.len() {
            match marked(walk) {
                Some(after) => {
                    if walk > plain {
                        out.push(Span { start: plain, end: walk, kind, construct, depth: 0 });
                        construct += 1;
                    }
                    walk = after;
                    plain = after;
                }
                None => walk += 1,
            }
        }
        if plain < chars.len() {
            out.push(Span { start: plain, end: chars.len(), kind, construct, depth: 0 });
            construct += 1;
        }
    }
    // A pair is two characters each side, and the inside may not be empty:
    // `line_changes` never writes an empty run, and treating `[--]` as one
    // would put a zero-width span on the page.
    let starts = |i: usize, open: (char, char)| {
        chars.get(i) == Some(&open.0) && chars.get(i + 1) == Some(&open.1)
    };
    while at < chars.len() {
        let pair = [
            (('[', '-'), ('-', ']'), Kind::Gone),
            (('{', '+'), ('+', '}'), Kind::Added),
        ]
        .into_iter()
        .find(|&(open, _, _)| starts(at, open));
        let Some((_, close, kind)) = pair else {
            at += 1;
            continue;
        };
        let text = at + 2;
        let Some(end) = (text..chars.len().saturating_sub(1))
            .find(|&i| chars[i] == close.0 && chars[i + 1] == close.1)
        else {
            at += 1;
            continue;
        };
        if end == text {
            at += 1;
            continue;
        }
        // Opener, the run, closer — one construct, the way `**bold**` is one.
        out.push(Span { start: at, end: text, kind: Kind::Marker, construct, depth: 0 });
        out.push(Span { start: text, end, kind, construct, depth: 0 });
        out.push(Span { start: end, end: end + 2, kind: Kind::Marker, construct, depth: 0 });
        construct += 1;
        at = end + 2;
    }
    out
}

/// Split `text` into the tokens the diff runs over: the words `segment` finds
/// on each line, plus one `"\n"` between lines.
///
/// `segment` is handed one line at a time, so a segmenter that caches by line
/// (the editor's does) answers most of these for free.
pub fn tokens<'a>(text: &'a str, segment: &dyn Fn(&str) -> Vec<(usize, usize)>) -> Vec<&'a str> {
    let mut out = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push("\n");
        }
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        let mut at = 0usize;
        for (ws, we) in segment(line) {
            if ws >= chars.len() || we > chars.len() || ws >= we {
                continue;
            }
            // Whatever the segmenter skipped over — spaces, 標點 — is a token
            // too, or a moved comma would read as no change at all. One
            // character each: a run of 「……」 that gained one 、 should read as
            // one 、 gained.
            while at < ws {
                let end = chars.get(at + 1).map(|c| c.0).unwrap_or(line.len());
                out.push(&line[chars[at].0..end]);
                at += 1;
            }
            let end = chars.get(we).map(|c| c.0).unwrap_or(line.len());
            out.push(&line[chars[ws].0..end]);
            at = we;
        }
        while at < chars.len() {
            let end = chars.get(at + 1).map(|c| c.0).unwrap_or(line.len());
            out.push(&line[chars[at].0..end]);
            at += 1;
        }
    }
    out
}

/// The edit script from `a` to `b`, or `None` when they are more than
/// [`MAX_D`] word-edits apart.
pub fn script<'a>(a: &[&'a str], b: &[&'a str]) -> Option<Vec<Op<'a>>> {
    // The head and tail two drafts share is most of a manuscript, and every
    // token trimmed here is one Myers does not pay for.
    let head = a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count();
    let tail = a[head..]
        .iter()
        .rev()
        .zip(b[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (mid_a, mid_b) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);

    let mut ops: Vec<Op<'a>> = a[..head].iter().map(|t| Op::Same(t)).collect();
    ops.extend(myers(mid_a, mid_b)?);
    ops.extend(a[a.len() - tail..].iter().map(|t| Op::Same(t)));
    Some(ops)
}

/// Myers's greedy O(ND) algorithm, with the trace kept so the script can be
/// walked back out of it.
fn myers<'a>(a: &[&'a str], b: &[&'a str]) -> Option<Vec<Op<'a>>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    // Row `d` of the trace holds `v` for `k` in `-d-1..=d+1`, indexed by
    // `k + d + 1` — the band is all Myers ever reads, and keeping only the band
    // is what makes the trace `O(D²)` instead of `O(D·(n+m))`.
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut v: Vec<isize> = vec![0; 3];
    let ceiling = MAX_D.min((n + m) as usize);

    for d in 0..=ceiling {
        let d = d as isize;
        trace.push(v.clone());
        let mut next = vec![0isize; (2 * d + 3) as usize];
        let get = |row: &Vec<isize>, dd: isize, k: isize| -> isize {
            let i = k + dd + 1;
            if i < 0 || i as usize >= row.len() {
                0
            } else {
                row[i as usize]
            }
        };
        let mut k = -d;
        while k <= d {
            let down = k == -d || (k != d && get(&v, d - 1, k - 1) < get(&v, d - 1, k + 1));
            let mut x = if down {
                get(&v, d - 1, k + 1)
            } else {
                get(&v, d - 1, k - 1) + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            next[(k + d + 1) as usize] = x;
            if x >= n && y >= m {
                trace.push(next);
                return Some(walk_back(&trace, a, b, d as usize));
            }
            k += 2;
        }
        v = next;
    }
    None
}

/// Read the script out of the trace, from the far corner backwards.
fn walk_back<'a>(trace: &[Vec<isize>], a: &[&'a str], b: &[&'a str], d_end: usize) -> Vec<Op<'a>> {
    let mut ops = Vec::new();
    let (mut x, mut y) = (a.len() as isize, b.len() as isize);
    for d in (0..=d_end).rev() {
        let d = d as isize;
        // `trace[d + 1]` is the row step `d` wrote; the previous position comes
        // out of `trace[d]`, the row it read.
        let row = &trace[d as usize];
        let get = |k: isize| -> isize {
            let i = k + d;
            if i < 0 || i as usize >= row.len() {
                0
            } else {
                row[i as usize]
            }
        };
        let k = x - y;
        let down = k == -d || (k != d && get(k - 1) < get(k + 1));
        let prev_k = if down { k + 1 } else { k - 1 };
        let prev_x = get(prev_k);
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            ops.push(Op::Same(a[x as usize]));
        }
        if d > 0 {
            if x == prev_x {
                y -= 1;
                ops.push(Op::New(b[y as usize]));
            } else {
                x -= 1;
                ops.push(Op::Gone(a[x as usize]));
            }
        }
    }
    ops.reverse();
    ops
}

/// The lines of the new text that differ, with the change marked inline.
pub fn line_changes(ops: &[Op]) -> Vec<LineChange> {
    let mut out = Vec::new();
    let mut line = 0usize;
    let mut buf = String::new();
    let mut dirty = false;
    // Adjacent tokens of the same fate are wrapped once, not one bracket per
    // 詞: `[-很早很早-]` and not `[-很早-][-很早-]`.
    let mut run: Option<(bool, String)> = None;

    let flush_run = |run: &mut Option<(bool, String)>, buf: &mut String| {
        if let Some((added, text)) = run.take() {
            let (open, close) = if added { NEW } else { GONE };
            buf.push_str(open);
            buf.push_str(&text);
            buf.push_str(close);
        }
    };

    for op in ops {
        match *op {
            Op::Same(t) => {
                flush_run(&mut run, &mut buf);
                if t == "\n" {
                    if dirty {
                        out.push(LineChange {
                            line,
                            marked: std::mem::take(&mut buf),
                        });
                    }
                    buf.clear();
                    dirty = false;
                    line += 1;
                } else {
                    buf.push_str(t);
                }
            }
            Op::Gone(t) | Op::New(t) => {
                let added = matches!(op, Op::New(_));
                let text = if t == "\n" { BREAK } else { t };
                match &mut run {
                    Some((was, acc)) if *was == added => acc.push_str(text),
                    _ => {
                        flush_run(&mut run, &mut buf);
                        run = Some((added, text.to_string()));
                    }
                }
                dirty = true;
                // A line break that was *added* ends this line of the new text
                // right here; one that was taken away never existed in it.
                if added && t == "\n" {
                    flush_run(&mut run, &mut buf);
                    out.push(LineChange {
                        line,
                        marked: std::mem::take(&mut buf),
                    });
                    buf.clear();
                    dirty = false;
                    line += 1;
                }
            }
        }
    }
    flush_run(&mut run, &mut buf);
    if dirty {
        out.push(LineChange { line, marked: buf });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One 漢字 per word, which is what the editor's own segmenter answers
    /// with no dictionary loaded — and enough to show the grain.
    fn by_character(line: &str) -> Vec<(usize, usize)> {
        (0..line.chars().count()).map(|i| (i, i + 1)).collect()
    }

    fn changed(old: &str, new: &str) -> Vec<String> {
        let seg: &dyn Fn(&str) -> Vec<(usize, usize)> = &by_character;
        let (a, b) = (tokens(old, seg), tokens(new, seg));
        let ops = script(&a, &b).expect("close enough to diff");
        line_changes(&ops)
            .into_iter()
            .map(|c| format!("{}: {}", c.line, c.marked))
            .collect()
    }

    #[test]
    fn one_word_moved_in_a_long_paragraph_is_one_word() {
        // The whole point: a line diff calls this paragraph changed, and says
        // nothing about *what* changed.
        let old = "那年冬天，雪下得很早，鎮上的人都說是好兆頭。";
        let new = "那年冬天，雪下得極早，鎮上的人都說是好兆頭。";
        assert_eq!(
            changed(old, new),
            ["0: 那年冬天，雪下得[-很-]{+極+}早，鎮上的人都說是好兆頭。"]
        );
    }

    #[test]
    fn an_untouched_document_has_nothing_to_report() {
        let text = "第一行\n第二行\n第三行";
        assert!(changed(text, text).is_empty());
    }

    #[test]
    fn only_the_line_that_changed_is_reported() {
        let old = "第一行\n第二行\n第三行";
        let new = "第一行\n第貳行\n第三行";
        assert_eq!(changed(old, new), ["1: 第[-二-]{+貳+}行"]);
    }

    #[test]
    fn adjacent_words_are_wrapped_once() {
        assert_eq!(changed("他很早走了", "他極晚走了"), ["0: 他[-很早-]{+極晚+}走了"]);
    }

    #[test]
    fn a_line_break_that_was_added_is_drawn() {
        let out = changed("上句下句", "上句\n下句");
        assert_eq!(out, ["0: 上句{+⏎+}"]);
    }

    #[test]
    fn a_line_break_that_was_taken_away_is_drawn() {
        let out = changed("上句\n下句", "上句下句");
        assert_eq!(out, ["0: 上句[-⏎-]下句"]);
    }

    #[test]
    fn a_deleted_line_is_reported_where_it_was() {
        assert_eq!(changed("甲\n乙\n丙", "甲\n丙"), ["1: [-乙⏎-]丙"]);
    }

    #[test]
    fn punctuation_is_a_token_too() {
        // The 、 the segmenter walked past is a token like the 詞 around it, so
        // a comma that moved reads as a comma that moved.
        assert_eq!(changed("他說，好。", "他說。"), ["0: 他說[-，好-]。"]);
        assert_eq!(changed("他說。", "他說，好。"), ["0: 他說{+，好+}。"]);
    }

    #[test]
    fn two_texts_with_nothing_in_common_give_up_rather_than_grind() {
        let old: String = std::iter::repeat_n("甲乙丙丁", 2000).collect();
        let new: String = std::iter::repeat_n("戊己庚辛", 2000).collect();
        let seg: &dyn Fn(&str) -> Vec<(usize, usize)> = &by_character;
        assert!(script(&tokens(&old, seg), &tokens(&new, seg)).is_none());
    }
}

#[cfg(test)]
mod spans_tests {
    use super::spans;
    use crate::markdown::Kind;

    fn kinds(line: &str) -> Vec<(Kind, String)> {
        let chars: Vec<char> = line.chars().collect();
        spans(line)
            .into_iter()
            .map(|s| (s.kind, chars[s.start..s.end].iter().collect()))
            .collect()
    }

    /// 一對括號 ＝ 開、內容、關，三段一個 construct（#499）。
    #[test]
    fn a_pair_is_marker_text_marker() {
        assert_eq!(
            kinds("[-很-]"),
            [
                (Kind::Marker, "[-".to_string()),
                (Kind::Gone, "很".to_string()),
                (Kind::Marker, "-]".to_string()),
            ]
        );
        let out = spans("[-很-]");
        assert!(out.iter().all(|s| s.construct == 0), "一對是一個 construct");
    }

    /// 兩對挨着，各是各的。
    #[test]
    fn a_change_is_the_old_run_then_the_new_one() {
        let out = kinds("下[-很-]{+極+}早");
        assert_eq!(
            out.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            [Kind::Marker, Kind::Gone, Kind::Marker, Kind::Marker, Kind::Added, Kind::Marker]
        );
        assert_eq!(out[1].1, "很");
        assert_eq!(out[4].1, "極");
        assert_eq!(spans("下[-很-]{+極+}早")[3].construct, 1, "第二對是第二個 construct");
    }

    /// 行首那個 `a.md:12:` 是個地點，不是改動——不着色。
    #[test]
    fn the_place_the_line_opens_with_is_not_a_change() {
        assert!(kinds("a.md:12: 那年冬天。").is_empty());
    }

    /// Warning: **没有配對就什麼都不是。** 半個括號、空的一對、跨不到底的開頭，都照字面
    /// 留着——畫錯比不畫壞：一段没關上的紅底會一路吃到行尾。
    #[test]
    fn half_a_pair_is_just_characters() {
        assert!(kinds("[-没關上").is_empty());
        assert!(kinds("關上了-]").is_empty());
        assert!(kinds("[--]").is_empty(), "空的一對是零寬，不畫");
        assert!(kinds("{+没關上").is_empty());
    }

    /// 換行畫成 `⏎`，而它也要進着色的那一段（2026-09-15：「留着，带颜色」）。
    ///
    /// 看不見的變動正是最需要畫出來的那一種。
    #[test]
    fn the_break_mark_is_coloured_like_anything_else() {
        assert_eq!(kinds("上句[-⏎-]下句"), [
            (Kind::Marker, "[-".to_string()),
            (Kind::Gone, super::BREAK.to_string()),
            (Kind::Marker, "-]".to_string()),
        ]);
    }
}
