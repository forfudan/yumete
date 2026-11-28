//! **Headings rearranged and numbered**: the text-only half of
//! `:sort-sections` and `:heading-numbering`. Which lines are headings is the
//! outline's answer, handed in; what is decided here is where each line goes
//! and what each heading's number reads.

use crate::numerals::Counting;

/// A heading as the outline found it: the line its title is on, and its
/// level (1 for `#` or `=`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Head {
    pub line: usize,
    pub level: usize,
}

/// Which levels a sort reorders (`--levels 2`, `--levels 2-3`); every level
/// when `None`.
pub type Levels = Option<(usize, usize)>;

/// **Lines `from..to` with their sections reordered.** `order[i]` is the
/// line that goes `i` lines below `from`; `sorted` counts the sections whose
/// siblings were compared, `moved` the ones that changed place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arranged {
    pub order: Vec<usize>,
    pub sorted: usize,
    pub moved: usize,
}

struct Node {
    head: usize,
    start: usize,
    end: usize,
    children: Vec<Node>,
}

/// Sections of the heads `lo..hi`, all inside lines `..end`.
fn tree(heads: &[Head], lo: usize, hi: usize, end: usize) -> Vec<Node> {
    let mut out = Vec::new();
    let mut i = lo;
    while i < hi {
        let level = heads[i].level;
        let next = (i + 1..hi).find(|&k| heads[k].level <= level).unwrap_or(hi);
        let stop = heads.get(next).filter(|_| next < hi).map_or(end, |h| h.line);
        out.push(Node { head: i, start: heads[i].line, end: stop, children: tree(heads, i + 1, next, stop) });
        i = next;
    }
    out
}

/// **A section moves with its body and every section under it; it never
/// leaves its parent.** The lines above the first heading in `from..to` are
/// nobody's section and stay first. Siblings are ordered when any of them is
/// at a level `levels` names, by `keys` (one per head), equal keys keeping
/// their order; a level left out stays put while the levels under it are
/// still ordered.
pub fn arrange<K: Ord>(from: usize, to: usize, heads: &[Head], keys: &[K], levels: Levels, reverse: bool) -> Arranged {
    let mut nodes = tree(heads, 0, heads.len(), to);
    let mut out = Arranged { order: Vec::with_capacity(to - from), sorted: 0, moved: 0 };
    let first = nodes.first().map_or(to, |n| n.start);
    out.order.extend(from..first);
    let wanted = |level: usize| levels.is_none_or(|(lo, hi)| (lo..=hi).contains(&level));
    fn place<K: Ord>(nodes: &mut [Node], heads: &[Head], keys: &[K], wanted: &dyn Fn(usize) -> bool, reverse: bool, out: &mut Arranged) {
        if nodes.iter().any(|n| wanted(heads[n.head].level)) {
            let before: Vec<usize> = nodes.iter().map(|n| n.head).collect();
            nodes.sort_by(|a, b| match reverse {
                false => keys[a.head].cmp(&keys[b.head]),
                true => keys[b.head].cmp(&keys[a.head]),
            });
            out.sorted += nodes.len();
            out.moved += nodes.iter().zip(&before).filter(|(n, &b)| n.head != b).count();
        }
        for node in nodes.iter_mut() {
            let body_end = node.children.first().map_or(node.end, |c| c.start);
            out.order.extend(node.start..body_end);
            place(&mut node.children, heads, keys, wanted, reverse, out);
        }
    }
    place(&mut nodes, heads, keys, &wanted, reverse, &mut out);
    out
}

/// Each heading's chain of ancestor titles, ending in its own, sorted: two
/// texts whose sections only changed order have the same chains.
pub fn chains(titles: &[(usize, String)]) -> Vec<Vec<String>> {
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut out = Vec::new();
    for (level, title) in titles {
        while stack.last().is_some_and(|(l, _)| l >= level) {
            stack.pop();
        }
        stack.push((*level, title.clone()));
        out.push(stack.iter().map(|(_, t)| t.clone()).collect());
    }
    out.sort();
    out
}

/// **What a heading reads as**, for ordering by it: the marks around it
/// gone — closing `#`s, emphasis, a link's address, a Typst label, an HTML
/// comment — and the words left.
pub fn visible_title(title: &str, typst: bool) -> String {
    let mut text = without_comments(title);
    if typst {
        if let Some(open) = text.rfind('<').filter(|&i| text.trim_end().ends_with('>') && !text[i..].contains(' ')) {
            text.truncate(open);
        }
    } else {
        let trimmed = text.trim_end();
        let closing = trimmed.trim_end_matches('#');
        if closing.len() < trimmed.len() && (closing.is_empty() || closing.ends_with([' ', '\t'])) {
            text = closing.to_string();
        }
        text = link_texts(&text);
    }
    text.chars().filter(|c| !matches!(c, '*' | '_' | '`' | '~')).collect::<String>().trim().to_string()
}

fn without_comments(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find("<!--") {
        out.push_str(&rest[..open]);
        match rest[open..].find("-->") {
            Some(close) => rest = &rest[open + close + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// `[words](address)` and `![words](address)` as their words.
fn link_texts(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let bang = chars[i] == '!' && chars.get(i + 1) == Some(&'[');
        let at = if bang { i + 1 } else { i };
        if chars[at] == '[' {
            if let Some(close) = chars[at..].iter().position(|&c| c == ']').map(|p| at + p) {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = chars[close..].iter().position(|&c| c == ')').map(|p| close + p) {
                        out.extend(&chars[at + 1..close]);
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// A toned syllable (`hàn`, `lǜ`, `le`) as its letters and its tone, 1–4 for
/// the marked tones and 5 for the neutral one; `ü` spelled `v`, so `lü`
/// orders after `lu`.
pub fn toned(syllable: &str) -> (String, u8) {
    let mut letters = String::new();
    let mut tone = 5;
    for c in syllable.chars() {
        let (base, t) = match c {
            'ā' => ('a', 1),
            'á' => ('a', 2),
            'ǎ' => ('a', 3),
            'à' => ('a', 4),
            'ē' => ('e', 1),
            'é' => ('e', 2),
            'ě' => ('e', 3),
            'è' => ('e', 4),
            'ī' => ('i', 1),
            'í' => ('i', 2),
            'ǐ' => ('i', 3),
            'ì' => ('i', 4),
            'ō' => ('o', 1),
            'ó' => ('o', 2),
            'ǒ' => ('o', 3),
            'ò' => ('o', 4),
            'ū' => ('u', 1),
            'ú' => ('u', 2),
            'ǔ' => ('u', 3),
            'ù' => ('u', 4),
            'ǖ' => ('v', 1),
            'ǘ' => ('v', 2),
            'ǚ' => ('v', 3),
            'ǜ' => ('v', 4),
            'ü' => ('v', 0),
            'ḿ' => ('m', 2),
            'ń' => ('n', 2),
            'ň' => ('n', 3),
            'ǹ' => ('n', 4),
            c if c.is_ascii_digit() => {
                tone = c as u8 - b'0';
                continue;
            }
            c => (c.to_ascii_lowercase(), 0),
        };
        letters.push(base);
        if t > 0 {
            tone = t;
        }
    }
    (letters, tone)
}

// ── numbering ─────────────────────────────────────────────────────────────

/// One piece of a numbering pattern: text written as it stands, or a place
/// a number goes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(String),
    Count(Counting),
}

/// **A level's numbering pattern**: `第一章 `, `1.1. `, `一、`. Its counting
/// symbols (see [`Counting`]) are replaced by numbers, the rest written as
/// it stands, the separator included. As many symbols as there are, so many
/// levels' numbers it shows, ending in its own: `1.1. ` is the parent's and
/// this one's.
///
/// A Latin letter counts only standing alone, so the `a` in `Chapter 1` is
/// a letter of the word and the `A` of `Appendix A.` is a number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern(Vec<Piece>);

impl Pattern {
    pub fn parse(text: &str) -> Pattern {
        let chars: Vec<char> = text.chars().collect();
        let mut pieces = Vec::new();
        let mut literal = String::new();
        for (i, &c) in chars.iter().enumerate() {
            let alone = !c.is_ascii_alphabetic()
                || (!i.checked_sub(1).is_some_and(|p| chars[p].is_ascii_alphabetic())
                    && !chars.get(i + 1).is_some_and(|n| n.is_ascii_alphabetic()));
            match Counting::of(c).filter(|_| alone) {
                Some(counting) => {
                    if !literal.is_empty() {
                        pieces.push(Piece::Text(std::mem::take(&mut literal)));
                    }
                    pieces.push(Piece::Count(counting));
                }
                None => literal.push(c),
            }
        }
        if !literal.is_empty() {
            pieces.push(Piece::Text(literal));
        }
        Pattern(pieces)
    }

    fn places(&self) -> usize {
        self.0.iter().filter(|p| matches!(p, Piece::Count(_))).count()
    }

    /// The number of a heading whose levels' counts are `counts` (the last
    /// is its own).
    fn write(&self, counts: &[u64], traditional: bool) -> String {
        let places = self.places();
        let mut shown = counts[counts.len().saturating_sub(places)..].iter();
        // Fewer levels above than the pattern shows: the first places say 0.
        let missing = places.saturating_sub(counts.len());
        let mut out = String::new();
        let mut place = 0;
        for piece in &self.0 {
            match piece {
                Piece::Text(text) => out.push_str(text),
                Piece::Count(counting) => {
                    let n = if place < missing { 0 } else { shown.next().copied().unwrap_or(0) };
                    out.push_str(&counting.write(n, traditional));
                    place += 1;
                }
            }
        }
        out
    }

    /// **The length of a number this pattern wrote at the start of `text`**,
    /// if one is there. No count in it may pass `most`: `2024. 回顧` under
    /// `1. ` is a year in a title, not a number a heading was given.
    fn written_at(&self, text: &str, most: u64) -> Option<usize> {
        fn at(pieces: &[Piece], text: &str, most: u64) -> Option<usize> {
            let Some((first, rest)) = pieces.split_first() else { return Some(0) };
            match first {
                Piece::Text(literal) => {
                    let after = text.strip_prefix(literal.as_str())?;
                    at(rest, after, most).map(|n| n + literal.len())
                }
                Piece::Count(counting) => {
                    let run: usize = text.chars().take_while(|&c| counting.spells(c)).map(char::len_utf8).sum();
                    // Shortest first is wrong for `1.1.`, longest first for a
                    // run a literal digit ends: try every length, longest first.
                    let mut ends: Vec<usize> = text[..run].char_indices().map(|(i, c)| i + c.len_utf8()).collect();
                    ends.reverse();
                    ends.into_iter().find_map(|end| {
                        let n = counting.read(&text[..end])?;
                        if n > most {
                            return None;
                        }
                        at(rest, &text[end..], most).map(|m| m + end)
                    })
                }
            }
        }
        at(&self.0, text, most).filter(|&n| n > 0)
    }
}

/// How each level is numbered: `None` is a level left unnumbered (`"-"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Numbering {
    pub levels: Vec<Option<Pattern>>,
    /// 貳參陸 rather than 贰叁陆: the file says `lang: zh-Hant` (or a
    /// Taiwan, Hong Kong or Macau tag).
    pub traditional: bool,
}

impl Numbering {
    /// `1. `, `1.1. `, `1.1.1. ` and so on to the sixth level.
    pub fn default_levels() -> Vec<Option<Pattern>> {
        (1..=6).map(|n| Some(Pattern::parse(&format!("{}. ", vec!["1"; n].join("."))))).collect()
    }

    fn of(&self, level: usize) -> Option<&Pattern> {
        self.levels.get(level.checked_sub(1)?).and_then(Option::as_ref)
    }
}

/// Why a file's numbering could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecError {
    /// `heading-numbering:` is there and is not a list of strings.
    NotAList,
}

/// **The numbering a Markdown file asks for**: `heading-numbering` in the
/// front matter, else in a `<!-- heading-numbering: [...] -->` line above the
/// first heading; the front matter wins when both are there. The value is a
/// list, one pattern a level, `"-"` for a level left unnumbered. `Ok(None)`
/// when neither is written.
pub fn numbering_of(lines: &[String], first_heading: usize) -> Result<Option<Numbering>, SpecError> {
    let front = front_matter(lines);
    let traditional = front.iter().any(|(k, v)| {
        k == "lang" && {
            let tag = v.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
            ["zh-hant", "zh-tw", "zh-hk", "zh-mo"].iter().any(|t| tag == *t || tag.starts_with(&format!("{t}-")))
        }
    });
    let list = match front.iter().find(|(k, _)| k == "heading-numbering") {
        Some((_, value)) => Some(value.clone()),
        None => lines[..first_heading.min(lines.len())].iter().find_map(|line| {
            let inner = line.trim().strip_prefix("<!--")?.strip_suffix("-->")?.trim();
            Some(inner.strip_prefix("heading-numbering:")?.trim().to_string())
        }),
    };
    let Some(list) = list else { return Ok(None) };
    let items = flow_list(&list).ok_or(SpecError::NotAList)?;
    let levels = items.iter().map(|item| (item != "-").then(|| Pattern::parse(item))).collect();
    Ok(Some(Numbering { levels, traditional }))
}

/// The `key: value` lines of a front matter block, a block list under a key
/// joined back into the flow form `[…]`.
fn front_matter(lines: &[String]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return out;
    }
    let mut block: Option<Vec<String>> = None;
    let flush = |out: &mut Vec<(String, String)>, block: &mut Option<Vec<String>>| {
        if let (Some(items), Some(last)) = (block.take(), out.last_mut()) {
            last.1 = format!("[{}]", items.join(", "));
        }
    };
    for line in &lines[1..] {
        let trimmed = line.trim_end();
        if trimmed == "---" || trimmed == "..." {
            break;
        }
        if let Some(item) = trimmed.trim_start().strip_prefix('-').filter(|_| trimmed.starts_with([' ', '-'])) {
            if out.last().is_some_and(|(_, v)| v.is_empty()) || block.is_some() {
                block.get_or_insert_with(Vec::new).push(item.trim().to_string());
                continue;
            }
        }
        flush(&mut out, &mut block);
        if let Some((key, value)) = trimmed.split_once(':').filter(|_| !trimmed.starts_with([' ', '\t', '#'])) {
            out.push((key.trim().to_string(), value.trim().to_string()));
        }
    }
    flush(&mut out, &mut block);
    out
}

/// `["-", "第一章 ", '1. ']` as its strings: double quotes with `\"` and
/// `\\`, single quotes with `''`, or bare words up to a comma.
fn flow_list(text: &str) -> Option<Vec<String>> {
    let inner = text.trim().strip_prefix('[')?.strip_suffix(']')?;
    let chars: Vec<char> = inner.chars().collect();
    let mut items = Vec::new();
    let mut i = 0;
    loop {
        while chars.get(i).is_some_and(|c| c.is_whitespace()) {
            i += 1;
        }
        let Some(&c) = chars.get(i) else { break };
        let mut item = String::new();
        match c {
            '"' => {
                i += 1;
                loop {
                    match *chars.get(i)? {
                        '\\' => {
                            item.push(*chars.get(i + 1)?);
                            i += 2;
                        }
                        '"' => {
                            i += 1;
                            break;
                        }
                        ch => {
                            item.push(ch);
                            i += 1;
                        }
                    }
                }
            }
            '\'' => {
                i += 1;
                loop {
                    match *chars.get(i)? {
                        '\'' if chars.get(i + 1) == Some(&'\'') => {
                            item.push('\'');
                            i += 2;
                        }
                        '\'' => {
                            i += 1;
                            break;
                        }
                        ch => {
                            item.push(ch);
                            i += 1;
                        }
                    }
                }
            }
            _ => {
                while chars.get(i).is_some_and(|&c| c != ',') {
                    item.push(chars[i]);
                    i += 1;
                }
                item = item.trim().to_string();
            }
        }
        items.push(item);
        while chars.get(i).is_some_and(|c| c.is_whitespace()) {
            i += 1;
        }
        match chars.get(i) {
            Some(',') => i += 1,
            None => break,
            Some(_) => return None,
        }
    }
    Some(items)
}

/// A Markdown heading as numbering sees it: the line its title is on, its
/// level, and whether `<!-- omit from toc -->` (or `omit in toc`) leaves it
/// out — at the end of the line or alone on the line above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Numbered {
    pub line: usize,
    pub level: usize,
    pub omitted: bool,
}

/// Whether `line` is or ends in the comment that leaves a heading out.
pub fn omits(line: &str) -> bool {
    let line = line.trim_end();
    ["<!-- omit from toc -->", "<!-- omit in toc -->"].iter().any(|c| line.ends_with(c))
}

/// Where a heading's title starts on its line: after the `#`s and the space
/// that follows them, or after the indent of a setext title.
fn title_start(line: &str) -> usize {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    let marks = rest.len() - rest.trim_start_matches('#').len();
    if marks == 0 {
        return indent;
    }
    let after = &rest[marks..];
    indent + marks + (after.len() - after.trim_start_matches([' ', '\t']).len())
}

/// What numbering does to a file: the lines it rewrites, and the headings
/// it left alone because they would have read two numbers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Renumbered {
    pub lines: Vec<(usize, String)>,
    pub doubled: Vec<usize>,
}

/// **Every heading numbered by `numbering`, or its number taken off**
/// (`on == false`). A number already there that the level's pattern could
/// have written is replaced, never added to; an omitted heading neither
/// takes a number nor uses one up; a level a heading skipped reads 0.
pub fn renumber(lines: &[String], heads: &[Numbered], numbering: &Numbering, on: bool) -> Renumbered {
    // A year is the number a title most often starts with; no file has
    // reached a thousandth heading of one level without having more than
    // a thousand headings.
    let most = (heads.len() as u64).max(999);
    let mut counts = [0u64; 6];
    let mut out = Renumbered::default();
    for head in heads {
        let Some(line) = lines.get(head.line) else { continue };
        let level = head.level.clamp(1, 6);
        let start = title_start(line);
        let title = &line[start..];
        let pattern = numbering.of(level);
        let old = pattern.and_then(|p| p.written_at(title, most)).unwrap_or(0);
        let bare = &title[old..];
        let number = match (on, head.omitted) {
            (true, false) => {
                counts[level - 1] += 1;
                counts[level..].iter_mut().for_each(|c| *c = 0);
                pattern.map(|p| p.write(&counts[..level], numbering.traditional))
            }
            _ => None,
        };
        if number.is_some() && pattern.is_some_and(|p| p.written_at(bare, most).is_some()) {
            out.doubled.push(head.line);
            continue;
        }
        let new = format!("{}{}{}", &line[..start], number.unwrap_or_default(), bare);
        if new != *line {
            out.lines.push((head.line, new));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    /// The heads of a plain `#` text, as the outline would find them.
    fn heads(lines: &[String]) -> Vec<Head> {
        lines
            .iter()
            .enumerate()
            .filter_map(|(line, text)| {
                let level = text.chars().take_while(|&c| c == '#').count();
                (level > 0).then_some(Head { line, level })
            })
            .collect()
    }

    fn sorted(text: &str, levels: Levels, reverse: bool) -> (String, Arranged) {
        let lines = lines(text);
        let heads = heads(&lines);
        let keys: Vec<String> = heads.iter().map(|h| visible_title(lines[h.line].trim_start_matches('#'), false)).collect();
        let arranged = arrange(0, lines.len(), &heads, &keys, levels, reverse);
        let out: Vec<&str> = arranged.order.iter().map(|&l| lines[l].as_str()).collect();
        (out.join("\n"), arranged)
    }

    #[test]
    fn sections_move_with_their_bodies_and_children() {
        let text = "intro\n# b\nbody b\n## y\n## x\n# a\nbody a";
        let (out, arranged) = sorted(text, None, false);
        assert_eq!(out, "intro\n# a\nbody a\n# b\nbody b\n## x\n## y");
        assert_eq!((arranged.sorted, arranged.moved), (4, 4));
    }

    #[test]
    fn a_level_left_out_stays_put_and_its_children_are_still_sorted() {
        let text = "# b\n## d\n## c\n# a";
        let (out, _) = sorted(text, Some((2, 2)), false);
        assert_eq!(out, "# b\n## c\n## d\n# a");
    }

    #[test]
    fn reversed_keeps_equal_titles_in_their_order() {
        let text = "# a\none\n# b\n# a\ntwo";
        let (out, _) = sorted(text, None, true);
        assert_eq!(out, "# b\n# a\none\n# a\ntwo");
    }

    #[test]
    fn chains_see_a_section_that_changed_parents() {
        let before = chains(&[(1, "a".into()), (2, "x".into()), (1, "b".into())]);
        let swapped = chains(&[(1, "b".into()), (1, "a".into()), (2, "x".into())]);
        let moved = chains(&[(1, "a".into()), (1, "b".into()), (2, "x".into())]);
        assert_eq!(before, swapped);
        assert_ne!(before, moved);
    }

    #[test]
    fn the_visible_title_drops_marks() {
        assert_eq!(visible_title("**粗** [鏈](http://x) `碼` ##", false), "粗 鏈 碼");
        assert_eq!(visible_title("標題 <!-- omit from toc -->", false), "標題");
        assert_eq!(visible_title("Typst 標題 <label>", true), "Typst 標題");
        assert_eq!(visible_title("C# 入門", false), "C# 入門");
    }

    #[test]
    fn toned_syllables() {
        assert_eq!(toned("hàn"), ("han".into(), 4));
        assert_eq!(toned("lǜ"), ("lv".into(), 4));
        assert_eq!(toned("le"), ("le".into(), 5));
        assert_eq!(toned("zhong1"), ("zhong".into(), 1));
    }

    fn numbered(lines: &[String]) -> Vec<Numbered> {
        heads(lines)
            .into_iter()
            .map(|h| Numbered {
                line: h.line,
                level: h.level,
                omitted: omits(&lines[h.line]) || h.line.checked_sub(1).is_some_and(|p| omits(&lines[p])),
            })
            .collect()
    }

    fn apply(text: &str, on: bool) -> (String, Vec<usize>) {
        let mut lines = lines(text);
        let first = numbered(&lines).first().map_or(lines.len(), |h| h.line);
        let numbering = numbering_of(&lines, first).unwrap().unwrap_or(Numbering { levels: Numbering::default_levels(), traditional: false });
        let done = renumber(&lines, &numbered(&lines), &numbering, on);
        for (line, text) in done.lines {
            lines[line] = text;
        }
        (lines.join("\n"), done.doubled)
    }

    #[test]
    fn the_default_numbers_every_level() {
        let (out, _) = apply("# a\n## b\n## c\n# d", true);
        assert_eq!(out, "# 1. a\n## 1.1. b\n## 1.2. c\n# 2. d");
    }

    #[test]
    fn a_list_in_the_front_matter_numbers_by_level() {
        let text = "---\nheading-numbering: [\"-\", \"第一章 \", \"1. \", \"1.1. \"]\n---\n# 書\n## 山\n### 夜\n#### 小\n## 城";
        let (out, _) = apply(text, true);
        assert!(out.ends_with("# 書\n## 第一章 山\n### 1. 夜\n#### 1.1. 小\n## 第二章 城"), "{out}");
    }

    #[test]
    fn running_again_replaces_and_off_takes_away() {
        let text = "<!-- heading-numbering: [\"一、\"] -->\n# 甲\n# 乙";
        let (once, _) = apply(text, true);
        assert_eq!(once, "<!-- heading-numbering: [\"一、\"] -->\n# 一、甲\n# 二、乙");
        let swapped = once.replace("# 一、甲\n# 二、乙", "# 二、乙\n# 一、甲");
        assert!(apply(&swapped, true).0.ends_with("# 一、乙\n# 二、甲"));
        assert_eq!(apply(&once, false).0, text);
    }

    #[test]
    fn an_omitted_heading_takes_no_number_and_uses_none_up() {
        let (out, _) = apply("# a\n# 目錄 <!-- omit from toc -->\n<!-- omit in toc -->\n# 附\n# b", true);
        assert_eq!(out, "# 1. a\n# 目錄 <!-- omit from toc -->\n<!-- omit in toc -->\n# 附\n# 2. b");
    }

    #[test]
    fn a_skipped_level_reads_zero_and_a_year_is_not_a_number() {
        let (out, _) = apply("# a\n### b\n# 2024. 回顧", true);
        assert_eq!(out, "# 1. a\n### 1.0.1. b\n# 2. 2024. 回顧");
    }

    #[test]
    fn traditional_forms_follow_lang() {
        let text = "---\nlang: zh-TW\nheading-numbering:\n  - \"第壹章 \"\n---\n# 一\n# 二";
        let (out, _) = apply(text, true);
        assert!(out.ends_with("# 第壹章 一\n# 第貳章 二"), "{out}");
    }

    #[test]
    fn a_heading_that_would_read_two_numbers_is_left_and_told() {
        let (out, doubled) = apply("# 1. 1. a", true);
        assert_eq!(out, "# 1. 1. a");
        assert_eq!(doubled, vec![0]);
    }

    #[test]
    fn a_letter_inside_a_word_is_not_a_number() {
        let (out, _) = apply("<!-- heading-numbering: ['Chapter 1: '] -->\n# a", true);
        assert!(out.ends_with("# Chapter 1: a"), "{out}");
    }

    #[test]
    fn a_value_that_is_not_a_list_is_refused() {
        let lines = lines("---\nheading-numbering: 1.\n---\n# a");
        assert_eq!(numbering_of(&lines, 3), Err(SpecError::NotAList));
    }
}
