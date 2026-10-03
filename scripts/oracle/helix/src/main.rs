//! **helix 自己的 `movement`，一式一答。**
//!
//! `hxor <檔> <光標位置> <次數> <動作>…`，每一式印一行：
//!
//! ```text
//! w: anchor=0 head=6 span=[0,6) text="alpha " cursor=5
//! ```
//!
//! 為什麼要它：yumete 的選區語義照的是 helix（`Reading::Selection`），而
//! 「照參考實現」這句話只有在**能問得到參考實現**的時候纔是一句話。靠讀 helix 的
//! 源碼推它會怎麼答，推錯過三次。
use helix_core::doc_formatter::TextFormat;
use helix_core::movement::{self, Direction, Movement};
use helix_core::text_annotations::TextAnnotations;
use helix_core::graphemes::next_grapheme_boundary;
use helix_core::{Range, Rope};

fn show(slice: helix_core::RopeSlice, r: Range) -> String {
    let (f, t) = (r.from(), r.to());
    let txt: String = slice.slice(f..t).to_string().replace('\n', "\\n");
    format!(
        "anchor={} head={} span=[{},{}) text={:?} cursor={}",
        r.anchor,
        r.head,
        f,
        t,
        txt,
        r.cursor(slice)
    )
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 4 {
        eprintln!("hxor <file> <pos> <count> <motion>…");
        std::process::exit(2);
    }
    let text = std::fs::read_to_string(&a[0]).unwrap();
    let rope = Rope::from_str(&text);
    let slice = rope.slice(..);
    let pos: usize = a[1].parse().unwrap();
    let count: usize = a[2].parse().unwrap();
    // Warning: **helix 的光標是一格寬的選區，不是一個點。** 用 `Range::point(pos)` 當
    // 起點，邊界上那幾式就答得不一樣（文件開頭的 `B`、行尾的 `e`／`E`／`W`），而
    // 編輯器裏的光標從來不是零寬的。2026-10-03 換成一格寬之後，三個固定裝置上
    // 378 格的分歧從 22 掉到個位數。
    let wide = next_grapheme_boundary(slice, pos);
    let mut r = Range::new(pos, wide);
    let (fmt, mut ann) = (TextFormat::default(), TextAnnotations::default());
    for m in &a[3..] {
        r = match m.as_str() {
            "w" => movement::move_next_word_start(slice, r, count),
            "b" => movement::move_prev_word_start(slice, r, count),
            "e" => movement::move_next_word_end(slice, r, count),
            "W" => movement::move_next_long_word_start(slice, r, count),
            "B" => movement::move_prev_long_word_start(slice, r, count),
            "E" => movement::move_next_long_word_end(slice, r, count),
            "ge" => movement::move_prev_word_end(slice, r, count),
            "h" => movement::move_horizontally(slice, r, Direction::Backward, count, Movement::Move, &fmt, &mut ann),
            "l" => movement::move_horizontally(slice, r, Direction::Forward, count, Movement::Move, &fmt, &mut ann),
            "j" => movement::move_vertically(slice, r, Direction::Forward, count, Movement::Move, &fmt, &mut ann),
            "k" => movement::move_vertically(slice, r, Direction::Backward, count, Movement::Move, &fmt, &mut ann),
            _ => panic!("unknown motion {m}"),
        };
        println!("{m}: {}", show(slice, r));
    }
}
