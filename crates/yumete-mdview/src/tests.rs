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

/// **A table is never cut off**: when even the smallest type cannot fit its
/// longest words, the words break.
#[test]
fn a_table_too_wide_for_any_type_breaks_its_words() {
    let n = 8;
    let head = (0..n).map(|k| format!("Column{k}")).collect::<Vec<_>>().join(" | ");
    let rule = vec!["---"; n].join(" | ");
    let row = (0..n).map(|k| format!("https://example.org/a/very/long/path/{k}")).collect::<Vec<_>>().join(" | ");
    let text = format!("| {head} |\n| {rule} |\n| {row} |\n");
    let p = preview(&text, 500);
    let laid = &p.placed[0].laid;
    assert!(laid.right <= 500.5, "reaches {}", laid.right);
    // The addresses wrap onto several lines rather than run out of the page.
    let tallest = laid.items.iter().filter_map(|it| match it {
        Item::Text { text, .. } => Some(text.layout.len()),
        _ => None,
    });
    assert!(tallest.max().unwrap() > 2);
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
    // A picture alone in its paragraph is a figure.
    let laid = &p.placed[0].laid;
    let pic = laid.items.iter().find_map(|it| match it {
        Item::Pic { r, .. } => Some((r.width() as f32, r.height() as f32)),
        _ => None,
    });
    let (w, h) = pic.expect("the image is a picture");
    assert!(w <= 500.0 && (w / h - 2.0).abs() < 0.05, "{w}×{h}");
    // A missing picture is its words.
    assert!(visual_lines(&p.placed[1].laid).iter().any(|l| l.contains("沒有")));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_picture_changed_on_disk_is_drawn_again() {
    let dir = std::env::temp_dir().join(format!("yumete-mdview-change-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pic.png");
    image::RgbaImage::from_pixel(100, 50, image::Rgba([0, 0, 0, 255])).save(&path).unwrap();
    let mut p = Preview::new(Options { base_dir: dir.clone(), ..options(500) }, Box::new(NoFormulas));
    let text = "![圖](pic.png)\n\n後面\n";
    p.set_text(text);
    let size = |p: &Preview| {
        p.placed[0].laid.items.iter().find_map(|it| match it {
            Item::Pic { r, .. } => Some((r.width() as f32, r.height() as f32)),
            _ => None,
        })
    };
    assert_eq!(size(&p), Some((100.0, 50.0)));
    // A new picture, another size: the same text lays the block out again.
    std::thread::sleep(std::time::Duration::from_millis(20));
    image::RgbaImage::from_pixel(60, 60, image::Rgba([0, 0, 0, 255])).save(&path).unwrap();
    let s = p.set_text(text);
    assert_eq!(s.laid, 1, "only the picture's block");
    assert_eq!(size(&p), Some((60.0, 60.0)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn formulas_kept_are_bounded() {
    let mut text = String::new();
    for k in 0..(crate::layout::FORMULAS_KEPT + 100) {
        text.push_str(&format!("$x_{{{k}}}$ "));
    }
    let mut p = Preview::new(options(800), Box::new(Boxes));
    p.set_text(&text);
    assert_eq!(p.kept().0, crate::layout::FORMULAS_KEPT);
}

/// **Big formulas are kept to a budget in bytes**: each version of one
/// edited key by key is a new picture, and a few are kept, not a thousand.
#[test]
fn big_formulas_kept_are_bounded_in_bytes() {
    let mut p = Preview::new(options(800), Box::new(Boxes));
    for k in 0..60 {
        p.set_text(&format!("$${}{k}$$\n", "x+".repeat(1200)));
    }
    let kept = p.kept().0;
    assert!((1..40).contains(&kept), "{kept}");
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

/// The width of the first text drawn in the last block of `text`.
fn width_of(text: &str) -> f32 {
    let p = preview(text, 600);
    let laid = &p.placed.last().unwrap().laid;
    laid.items
        .iter()
        .find_map(|it| match it {
            Item::Text { text, .. } => Some(text.layout.width()),
            _ => None,
        })
        .unwrap()
}

#[test]
fn adjacent_full_width_marks_give_up_their_blank_halves() {
    // 16 px type: a mark is 16 px, a squeezed pair 24.
    let squeezed = width_of("。「\n");
    let kept = width_of("「。\n");
    assert!((kept - squeezed - 8.0).abs() < 1.5, "kept {kept}, squeezed {squeezed}");
    assert!((width_of("（「\n") - squeezed).abs() < 1.5);
    assert!((width_of("」，\n") - squeezed).abs() < 1.5);
    // In 繁體, 。 sits in the middle of its em and keeps it.
    let tc = width_of("---\nlang: zh-Hant\n---\n\n。「\n");
    assert!((tc - kept).abs() < 1.5, "{tc} vs {kept}");
    // Code is not touched.
    assert!((width_of("`。「`\n") - width_of("`「。`\n")).abs() < 0.5);
}

#[test]
fn a_document_is_set_in_its_own_hand() {
    use crate::fonts::Han;
    assert_eq!(preview("這是一份繁體的說明，爲了測試。\n", 400).han(), Han::Traditional);
    assert_eq!(preview("这是一份简体的说明，为了测试。\n", 400).han(), Han::Simplified);
    // The front matter's `lang` wins over the text.
    assert_eq!(preview("---\nlang: ja\n---\n\n这是简体。\n", 400).han(), Han::Japanese);
    // A span says what its own words are.
    let blocks = parse::parse("中文 <span lang=\"ja\">日本語</span> 中文\n");
    let BlockKind::Paragraph(runs) = &blocks[0].kind else { panic!() };
    let lang_of = |w: &str| {
        runs.iter()
            .find_map(|r| match &r.kind {
                parse::InlineKind::Text { text, marks } if text.contains(w) => Some(marks.lang),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(lang_of("日本語"), Han::Japanese.index() as u8 + 1);
    assert_eq!(lang_of("中文"), 0);
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

/// **Across the page a table has its caption and a picture alone in its
/// paragraph is a figure**, as the vertical page and the export
/// set them: the caption's words over the table, the picture's text under it.
#[test]
fn across_the_page_a_caption_heads_its_table_and_a_figure_has_its_text() {
    let dir = std::env::temp_dir().join(format!("yumete-mdview-fig-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    image::RgbaImage::from_pixel(40, 20, image::Rgba([10, 20, 30, 255])).save(dir.join("a.png")).unwrap();
    let options = crate::Options { width: 800, scale: 1.0, theme: crate::Theme::light(), base_dir: dir.clone() };
    let mut p = crate::Preview::new(options, Box::new(crate::NoFormulas));
    p.set_text("Table: 兩座城\n\n| 城 | 人 |\n| --- | --- |\n| 甲 | 1 |\n\n![一張圖的說明](a.png)\n");
    let words = p.visible_text(0.0, p.size().1);
    assert!(words.contains("兩座城") && !words.contains("Table:"), "{words}");
    assert!(words.find("兩座城").unwrap() < words.find("甲").unwrap(), "the caption over the table: {words}");
    assert!(words.contains("一張圖的說明"), "the figure's text under it: {words}");
    // The caption and its table are one block: a line in either is in it.
    assert_eq!(p.block_at(p.y_for_line(2) + 1.0), Some((0, 4)));
    std::fs::remove_dir_all(&dir).ok();
}

/// **A page set vertically after its text turns quotes by the export's
/// rule too**: the language is read when the page turns, from
/// the text it was given across.
#[test]
fn quotes_follow_the_language_when_the_page_turns_vertical() {
    let mut p = Preview::new(options(800), Box::new(NoFormulas));
    p.set_height(600);
    p.set_text("这一段是中文，引用了“走吧”。\n");
    p.set_vertical(Some(Vertical { latin: yumete_core::export::Latin::Turn, hanging: false }));
    let all: String = p.visible_text(0.0, p.size().1).replace('\n', "");
    assert!(all.contains("『走吧』"), "{all}");
    p.set_vertical(None);
    p.set_text("This essay is mostly written in English words, and it quotes “走吧” once, then goes on.\n");
    p.set_vertical(Some(Vertical { latin: yumete_core::export::Latin::Turn, hanging: false }));
    // A narrow font can break the line inside the quote.
    let all: String = p.visible_text(0.0, p.size().1).replace('\n', "");
    assert!(all.contains("“走吧”") && !all.contains('『'), "{all}");
}

/// **No depth of nesting takes the preview down**: a line of 3000 `>`, a
/// list 600 deep, containers and notes nested 1000 deep are laid out across
/// and down a page on a thread with the preview's own stack, and the words
/// at the bottom are shown.
#[test]
fn deep_nesting_is_drawn_and_never_overflows() {
    let quotes = format!("{} 最深處\n", ">".repeat(3000));
    let list: String = (0..600).map(|k| format!("{}- 第{k}層\n", "  ".repeat(k))).collect();
    let mixed = format!("{}最深處\n", "> - ".repeat(1500));
    let fenced = format!("{}最深處\n{}", "::: tip\n".repeat(1000), ":::\n".repeat(1000));
    let notes: String = std::iter::once("引[^0]\n\n[^0]: 甲\n".to_string()).chain((1..1000).map(|k| format!("  [^{k}]: 乙\n"))).collect();
    for (text, deepest) in [(quotes, "最深處"), (list, "第599層"), (mixed, "最深處"), (fenced, "最深處"), (notes, "乙")] {
        let found = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || {
                let mut p = Preview::new(options(600), Box::new(NoFormulas));
                p.set_height(800);
                p.set_text(&text);
                let across = p.visible_text(0.0, p.size().1);
                p.render(0.0, 400);
                p.set_vertical(Some(Vertical { latin: yumete_core::export::Latin::Turn, hanging: false }));
                p.render(0.0, 400);
                across.contains(deepest)
            })
            .expect("a thread")
            .join()
            .expect("no overflow");
        assert!(found, "{deepest}");
    }
}

/// **A long paragraph is laid out in time it is long**, across and down the
/// page: single line breaks make a whole manuscript one paragraph, and each
/// character's style, each run's reading and each reading's place are found
/// without walking the paragraph again. Here 3000 lines with a reading and
/// a code span on each.
#[test]
fn a_long_paragraph_with_readings_is_laid_out_in_one_pass() {
    let line = "那年冬天<ruby>大雪<rt>dà xuě xuě</rt></ruby>，`code` 下得很大。";
    let text = format!("{}\n", vec![line; 3000].join("\n"));
    for vertical in [false, true] {
        let mut p = Preview::new(options(800), Box::new(NoFormulas));
        p.set_height(800);
        if vertical {
            p.set_vertical(Some(Vertical { latin: yumete_core::export::Latin::Turn, hanging: false }));
        }
        let began = std::time::Instant::now();
        p.set_text(&text);
        p.set_text(&text.replacen("很大", "很小", 1));
        let took = began.elapsed();
        eprintln!("vertical {vertical}: {took:?}");
        assert!(took < std::time::Duration::from_secs(20), "vertical {vertical}: {took:?}");
        let shown = p.visible_text(0.0, p.size().1);
        assert!(shown.contains("那年冬天") && shown.contains("很小"), "{}", &shown[..200.min(shown.len())]);
    }
}
