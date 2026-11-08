//! Layout, line map, incremental and viewport tests.

use super::*;
use crate::layout::Item;
use crate::parse::BlockKind;

fn options(width: u32) -> Options {
    Options { width, scale: 1.0, theme: Theme::light(), base_dir: PathBuf::from(".") }
}

fn preview(text: &str, width: u32) -> Preview {
    let mut p = Preview::new(options(width), Box::new(NoFormulas));
    p.set_text(text);
    p
}

/// A formula renderer that draws every formula as a box `w` × `h`.
struct Boxes;

impl Formulas for Boxes {
    fn draw(&self, tex: &str, _: bool, _: [u8; 3], size: f32) -> Result<Formula, String> {
        let (w, h) = ((tex.len() as f32 * size * 0.5).max(4.0) as u32, (size * 1.2) as u32);
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
        Ok(Formula { png, baseline: h as f32 * 0.8 })
    }
}

impl crate::layout::Text {
    fn layout_text(&self) -> &str {
        &self.string
    }
}

#[test]
fn the_words_on_screen_can_be_read_back() {
    let p = preview(LONG, 600);
    let top = p.visible_text(0.0, 60.0);
    assert!(top.contains("標題"), "{top}");
    assert!(!top.contains("最後一段"), "{top}");
    assert!(p.visible_text(0.0, p.size().1).contains("最後一段"));
    assert!(p.content_width() <= 600.0);
}

/// The visual lines of every text item in `laid`, as strings.
fn visual_lines(laid: &Laid) -> Vec<String> {
    let mut out = Vec::new();
    for item in &laid.items {
        if let Item::Text { text, .. } = item {
            let all = text.layout_text();
            for line in text.layout.lines() {
                out.push(all[line.text_range()].trim_end_matches('\n').to_string());
            }
        }
    }
    out
}

const LONG: &str = "# 標題\n\n第一段，那年冬天，雪下得很大。The quick brown fox jumps over the lazy dog.\n\n\
- 一\n- 二\n  - 三\n\n| 城市 | 简介 |\n| --- | --- |\n| 杭州 | 西湖边上的城市，夏天很热，冬天湿冷，春秋两季最适合出游 |\n\n\
```rust\nfn main() {}\nlet x = 1;\n```\n\n> 引用\n> 第二行\n\n最後一段。\n";

#[test]
fn the_line_map_runs_down_the_page() {
    let p = preview(LONG, 600);
    let lines = LONG.lines().count();
    let mut last = -1.0f32;
    for line in 0..lines {
        let y = p.y_for_line(line);
        assert!(y >= last, "line {line} at {y}, above line {} at {last}", line.saturating_sub(1));
        last = y;
    }
    // A line that begins a block comes back from its height.
    for line in [0, 2, 4, 8, 12, 17, 20] {
        assert_eq!(p.line_for_y(p.y_for_line(line) + 1.0), line, "line {line}");
    }
    assert!(p.size().1 > p.y_for_line(20), "the page is taller than its last line's top");
}

#[test]
fn a_paragraph_s_source_lines_are_its_visual_lines() {
    let text = "甲\n乙\n丙\n";
    let p = preview(text, 600);
    // Chinese lines stay broken, so each source line is a visual line.
    let (y0, y1, y2) = (p.y_for_line(0), p.y_for_line(1), p.y_for_line(2));
    assert!(y0 < y1 && y1 < y2, "{y0} {y1} {y2}");
}

#[test]
fn chinese_breaks_by_its_rules() {
    let words = "断行测试：这一行故意写得很长很长很长很长很长很长很长，看看“，”和“。”会不会被放到行首。「引号里的话」也不应该被拆开！（括号里的话）也一样；书名号《红楼梦》、句号。逗号，顿号、分号；冒号：问号？叹号！";
    let text = format!("{words}{words}{words}\n");
    for width in [180, 230, 260, 333, 410] {
        let mut p = preview(&text, width);
        let laid = p.placed[0].laid.clone();
        let lines = visual_lines(&laid);
        assert!(lines.len() > 3, "{width}: {lines:?}");
        for l in &lines {
            let first = l.chars().next().unwrap();
            let last = l.chars().last().unwrap();
            assert!(!"，。、；：！？」）》".contains(first), "{width}: a line opens with {first:?}: {l}");
            assert!(!"「（《".contains(last), "{width}: a line ends with {last:?}: {l}");
        }
        let _ = p.set_width(width + 1);
    }
}

#[test]
fn latin_words_stay_whole() {
    let text = "Typography is the craft of arranging type to make written language legible, readable and appealing.\n";
    let p = preview(text, 220);
    let lines = visual_lines(&p.placed[0].laid);
    let words: Vec<&str> = text.split_whitespace().collect();
    for l in &lines {
        for w in l.split_whitespace() {
            assert!(words.contains(&w), "{w:?} was broken: {lines:?}");
        }
    }
}

#[test]
fn a_short_table_heading_is_never_broken() {
    let text = "| 城市 | 简介 | 人口（万） |\n| --- | --- | ---: |\n| 杭州 | 西湖边上的城市，夏天很热，冬天湿冷，春秋两季最适合出游，茶叶也很有名，西湖边上的城市 | 1252 |\n";
    for width in [300, 420, 600] {
        let p = preview(text, width);
        let laid = &p.placed[0].laid;
        for word in ["城市", "人口（万）", "杭州", "1252"] {
            let found = laid.items.iter().find_map(|it| match it {
                Item::Text { text, .. } if text.layout_text().trim() == word => Some(text.layout.len()),
                _ => None,
            });
            assert_eq!(found, Some(1), "{width}: {word} on one line");
        }
        // The table fits the column.
        assert!(laid.right <= width as f32 + 0.5, "{width}: reaches {}", laid.right);
    }
}

#[test]
fn a_wide_table_is_set_smaller_to_fit() {
    let head = (0..12).map(|k| format!("列{k}名")).collect::<Vec<_>>().join(" | ");
    let rule = ["---"; 12].join(" | ");
    let row = (0..12).map(|k| format!("{}", 10000 + k)).collect::<Vec<_>>().join(" | ");
    let text = format!("| {head} |\n| {rule} |\n| {row} |\n");
    let p = preview(&text, 700);
    assert!(p.placed[0].laid.right <= 700.5, "reaches {}", p.placed[0].laid.right);
}

#[test]
fn an_edit_lays_out_only_the_block_it_touched() {
    let mut text = String::new();
    for k in 0..200 {
        text.push_str(&format!("第 {k} 段，那年冬天，雪下得很大。\n\n"));
    }
    let mut p = Preview::new(options(500), Box::new(NoFormulas));
    let first = p.set_text(&text);
    assert_eq!((first.blocks, first.laid, first.reused), (200, 200, 0));
    let edited = text.replace("第 100 段", "第 100 段，改過");
    let s = p.set_text(&edited);
    assert_eq!((s.laid, s.reused), (1, 199));
    // A line put in at the top moves every block down: none is laid again.
    let s = p.set_text(&format!("\n\n{edited}"));
    assert_eq!((s.laid, s.reused), (0, 200));
    assert_eq!(p.line_for_y(p.y_for_line(2) + 1.0), 2);
    // A new width lays everything out again.
    let s = p.set_width(400);
    assert_eq!(s.laid, 200);
}

#[test]
fn a_viewport_is_the_size_asked_for() {
    let mut p = preview(LONG, 640);
    let (w, h) = p.size();
    assert_eq!(w, 640);
    let pic = p.render(0.0, 300);
    assert_eq!((pic.width, pic.height, pic.rgba.len()), (640, 300, 640 * 300 * 4));
    // The corner is the ground; the page has ink somewhere.
    assert_eq!(&pic.rgba[0..4], &Theme::light().background);
    assert!(pic.rgba.chunks(4).any(|px| px[0] < 100), "no ink drawn");
    // Past the end: all ground.
    let past = p.render(h + 10.0, 50);
    assert!(past.rgba.chunks(4).all(|px| px == Theme::light().background));
    // A PNG of it decodes to the same size.
    let decoded = image::load_from_memory(&pic.png()).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (640, 300));
    // Dark is dark.
    p.set_theme(Theme::dark());
    let dark = p.render(0.0, 10);
    assert_eq!(&dark.rgba[0..4], &Theme::dark().background);
}

#[test]
fn formulas_are_pictures_on_the_baseline_or_their_tex() {
    let text = "面積 $\\pi r^2$ 公式\n\n$$\nx^2\n$$\n";
    let mut p = Preview::new(options(500), Box::new(Boxes));
    p.set_text(text);
    let laid = &p.placed[0].laid;
    let has_box = laid.items.iter().any(|it| matches!(it, Item::Text { text, .. } if !text.attach.is_empty()));
    assert!(has_box, "the inline formula is a picture");
    assert!(p.placed[1].laid.items.iter().any(|it| matches!(it, Item::Pic { .. })), "the display formula is a picture");
    // Without a renderer, the TeX is set as words.
    let p = preview(text, 500);
    let lines = visual_lines(&p.placed[0].laid);
    assert!(lines.iter().any(|l| l.contains("\\pi r^2")), "{lines:?}");
}

#[test]
fn images_are_drawn_scaled_to_the_column() {
    let dir = std::env::temp_dir().join(format!("yumete-mdview-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    image::RgbaImage::from_pixel(800, 400, image::Rgba([200, 30, 30, 255])).save(dir.join("wide.png")).unwrap();
    let mut p = Preview::new(Options { base_dir: dir.clone(), ..options(500) }, Box::new(NoFormulas));
    p.set_text("![寬圖](wide.png)\n\n![沒有](missing.png)\n");
    let laid = &p.placed[0].laid;
    let pic = laid.items.iter().find_map(|it| match it {
        Item::Text { text, .. } => text.layout.inline_boxes().next().map(|b| (b.width, b.height)),
        _ => None,
    });
    let (w, h) = pic.expect("the image is a box");
    assert!(w <= 500.0 && (w / h - 2.0).abs() < 0.05, "{w}×{h}");
    // A missing picture is its words.
    assert!(visual_lines(&p.placed[1].laid).iter().any(|l| l.contains("沒有")));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn every_block_kind_lays_out_and_draws() {
    let text = "---\ntitle: x\n---\n\n# H\n\npara **b** *i* `c` ==h== ~~s~~ [l](x) <kbd>K</kbd> <ruby>漢<rt>hàn</rt></ruby>[^1]\n\n\
> q\n\n> [!TIP]\n> t\n\n::: warning W\nw\n:::\n\n::: details D\nd\n:::\n\n- [x] a\n- [ ] b\n\n1. o\n\n---\n\n\
    code\n\n<Comp a='1' />\n\n$$\\broken{$$\n\n[^1]: note\n";
    let mut p = preview(text, 480);
    let kinds: Vec<&BlockKind> = p.blocks().iter().map(|b| &b.kind).collect();
    assert!(kinds.iter().any(|k| matches!(k, BlockKind::Footnotes(_))));
    assert!(kinds.iter().any(|k| matches!(k, BlockKind::FrontMatter(_))));
    assert!(kinds.iter().any(|k| matches!(k, BlockKind::Html(_))));
    for placed in &p.placed {
        assert!(placed.laid.height > 0.0, "{:?} has no height", placed.lines);
    }
    let (_, h) = p.size();
    let pic = p.render(0.0, h.ceil() as u32);
    assert_eq!(pic.height, h.ceil() as u32);
}

/// **A preview may be moved to another thread**: the front end can lay out
/// and draw away from the one that reads keys.
#[test]
fn a_preview_can_go_to_another_thread() {
    fn send<T: Send>() {}
    send::<Preview>();
    let mut p = preview("# 題\n\n文。\n", 300);
    let h = std::thread::spawn(move || p.render(0.0, 40).height).join().unwrap();
    assert_eq!(h, 40);
}

#[test]
fn a_footnote_s_lines_are_at_the_end_of_the_page() {
    let text = "甲[^a]。\n\n乙。\n\n[^a]: 注的文字。\n";
    let p = preview(text, 400);
    let note = p.y_for_line(4);
    assert!(note > p.y_for_line(2), "the note is drawn after the text: {note}");
    assert_eq!(p.line_for_y(note + 1.0), 4);
    // The cited text still maps back to its own line.
    assert_eq!(p.line_for_y(p.y_for_line(0) + 1.0), 0);
}

#[test]
fn code_keeps_its_indent_and_its_lines() {
    let text = "```rust\nfn main() {\n\tlet x = 1;\n}\n```\n";
    let p = preview(text, 600);
    let lines = visual_lines(&p.placed[0].laid);
    assert_eq!(lines, ["fn main() {", "    let x = 1;", "}"]);
    // Each code line maps to its own source line.
    let (a, b, c) = (p.y_for_line(1), p.y_for_line(2), p.y_for_line(3));
    assert!(a < b && b < c, "{a} {b} {c}");
    assert_eq!(p.line_for_y(b + 1.0), 2);
}
