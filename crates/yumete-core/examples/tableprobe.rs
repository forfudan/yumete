use std::time::Instant;
use yumete_core::editor::Editor;
use yumete_core::input::Key;
use yumete_core::TextStore;

fn main() {
    let csv = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let t = Instant::now();
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    println!("open + schema        {:?}", t.elapsed());
    let view = ed.table().expect("read as a grid");
    println!("columns              {}", view.schema.columns.len());
    println!("lines                {}", ed.current_buffer().line_count());

    let t = Instant::now();
    let mut ragged = 0;
    for line in 0..ed.current_buffer().line_count() {
        if ed.row_is_ragged(line) {
            if ragged < 3 {
                println!("  ragged line {}", line + 1);
            }
            ragged += 1;
        }
    }
    println!("ragged rows          {ragged} (scan {:?})", t.elapsed());

    ed.execute("50000").unwrap();
    let t = Instant::now();
    for _ in 0..200 {
        ed.on_key(Key::Char('l'));
    }
    println!("200 × l (cell right) {:?}", t.elapsed());
    let t = Instant::now();
    for _ in 0..200 {
        ed.on_key(Key::Char('j'));
    }
    println!("200 × j (row down)   {:?}", t.elapsed());

    ed.execute("50000").unwrap();
    let t = Instant::now();
    let d = ed.detail().expect("a row");
    println!("detail               {:?}", t.elapsed());
    println!("  title {}  here {}", d.title, d.here);
    for (k, v) in d.rows.iter().filter(|(_, v)| !v.is_empty()).take(8) {
        println!("  {k:12} {v}");
    }

    // Enter on a 拆分 cell: the jump scans the key column, once per component.
    ed.on_key(Key::Char('l'));
    ed.on_key(Key::Char('l'));
    let t = Instant::now();
    let links = ed.detail().unwrap().links;
    println!("links {:?}   in {:?}", links, t.elapsed());
    let t = Instant::now();
    ed.on_key(Key::Enter);
    println!("Enter                {:?}  mode {:?}  status {}", t.elapsed(), ed.mode(), ed.status());
    ed.on_key(Key::Esc);

    // What the renderer actually does on every frame while a writer types.
    ed.execute("50000").unwrap();
    ed.on_key(Key::Char('l'));
    ed.on_key(Key::Char('l'));
    let _ = ed.detail();
    ed.on_key(Key::Char('i'));
    let t = Instant::now();
    for _ in 0..20 {
        ed.on_key(Key::Char('木'));
        let _ = ed.detail_visible();
        let _ = ed.detail();
    }
    println!("20 × (type + 2 panels) {:?}", t.elapsed());
    ed.on_key(Key::Esc);
    // …and in the key column, where a key really can change.
    ed.on_key(Key::Char('0'));
    ed.on_key(Key::Char('i'));
    let t = Instant::now();
    for _ in 0..5 {
        ed.on_key(Key::Char('木'));
        let _ = ed.detail();
    }
    println!("5 × (type in key col)  {:?}", t.elapsed());
}
