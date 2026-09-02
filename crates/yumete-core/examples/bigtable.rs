use std::time::Instant;
use yumete_core::TextStore;
use yumete_core::editor::Editor;
use yumete_core::input::Key;

fn main() {
    let dir = std::env::temp_dir().join("yumete-bigtable");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("division.csv");
    if !path.exists() {
        let mut text = String::with_capacity(9_000_000);
        text.push_str("char,ids_y,ids_g,ids_t,ids_h,block,unicode");
        for i in 7..28 {
            text.push_str(&format!(",col{i}"));
        }
        text.push('\n');
        for i in 0..123_380u32 {
            let c = char::from_u32(0x4E00 + (i % 20000)).unwrap();
            text.push_str(&format!(
                "{c},⿰⿱木目金,⿰木⿱目金,⿰木目,⿰木目,CJK,U+{:04X}",
                0x4E00 + (i % 20000)
            ));
            for _ in 7..28 {
                text.push_str(",亠");
            }
            text.push('\n');
        }
        std::fs::write(&path, &text).unwrap();
    }
    println!("{} bytes", std::fs::metadata(&path).unwrap().len());

    let t = Instant::now();
    let mut ed = Editor::new();
    ed.open_file(&path).unwrap();
    println!("open_file             {:?}", t.elapsed());
    println!("lines                 {}", ed.current_buffer().line_count());

    ed.set_wrap_width(200);
    ed.set_page(50, 200);

    let t = Instant::now();
    for _ in 0..100 {
        ed.on_key(Key::Char('j'));
    }
    println!("100 × j               {:?}", t.elapsed());

    let t = Instant::now();
    ed.execute("120000").ok();
    println!("jump to line 120000   {:?}", t.elapsed());

    let t = Instant::now();
    ed.on_key(Key::Char('i'));
    for _ in 0..20 {
        ed.on_key(Key::Char('X'));
    }
    ed.on_key(Key::Esc);
    println!("20 characters typed   {:?}", t.elapsed());

    // What the renderer asks for: the rows of one screenful.
    let t = Instant::now();
    for _ in 0..100 {
        let rope = ed.current_buffer().rope();
        let anchor = yumete_core::wrap::Anchor::default();
        let m = yumete_core::wrap::Measure::plain(200);
        let n = yumete_core::wrap::rows_from(rope, anchor, m, 50).len();
        std::hint::black_box(n);
    }
    println!("100 × a screen of rows {:?}", t.elapsed());

    let out = dir.join("out.csv");
    let t = Instant::now();
    ed.execute(&format!("w {}", out.display())).unwrap();
    println!("save                  {:?}", t.elapsed());
}
