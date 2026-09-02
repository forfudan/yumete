use std::time::Instant;
use yumete_core::TextStore;
use yumete_core::editor::Editor;
use yumete_core::input::Key;
fn keys(ed: &mut Editor, s: &str) { for c in s.chars() { ed.on_key(Key::Char(c)); } }
fn row(ed: &Editor, l: usize) -> String {
    ed.current_buffer().rope().line(l).to_string().trim_end().chars().take(36).collect()
}
fn at(ed: &Editor) -> String {
    match ed.cell_position() {
        Some((l, c)) => format!("L{} c{} {:?}", l + 1, c, ed.cell_text(l, c)),
        None => "(none)".into(),
    }
}
const DIR: &str = "/private/tmp/claude-501/-Users-ZHU-Programs-yuhao-ime-yumete/bdc09235-d47e-4493-a6ac-9c931ae317ef/scratchpad/work";
fn main() {
    let dir = std::path::PathBuf::from(DIR);
    let mut ed = Editor::new();
    ed.open_file(&dir.join("work.csv")).unwrap();
    ed.set_page(50, 200);
    println!("table? {} status={:?}", ed.table().is_some(), ed.status());

    println!("=== the anchored regex, the only way to find a row ===");
    for pat in ["^木,", "^目,", "^𰻞,"] {
        let t = Instant::now();
        ed.execute("1").unwrap();
        ed.on_key(Key::Char('/'));
        keys(&mut ed, pat);
        ed.on_key(Key::Enter);
        println!("  /{pat}  {:?}  -> {}  status={}", t.elapsed(), at(&ed), ed.status());
    }

    println!("\n=== O and o at the edges ===");
    let l = ed.row_named('鑫').unwrap();
    ed.execute(&format!("{}", l + 1)).unwrap();
    ed.on_key(Key::Char('O'));
    ed.on_key(Key::Esc);
    println!("O   -> new row cells {} : {:?}", ed.row_cells(l).len(), row(&ed, l));
    ed.on_key(Key::Char('u'));
    ed.execute("1").unwrap();
    ed.on_key(Key::Char('O'));
    ed.on_key(Key::Esc);
    println!("O above the header -> {:?} (header now line 2?) row0 {:?}", row(&ed, 0), row(&ed, 1));
    ed.on_key(Key::Char('u'));
    ed.execute("ge").ok();
    println!("last line {} : {:?} ragged {}", ed.current_buffer().line_count(), row(&ed, ed.current_buffer().line_count()-1), ed.row_is_ragged(ed.current_buffer().line_count()-1));
    ed.on_key(Key::Char('o'));
    ed.on_key(Key::Esc);
    println!("o at EOF -> lines {} last {:?}", ed.current_buffer().line_count(), row(&ed, ed.current_buffer().line_count()-1));
    ed.on_key(Key::Char('u'));

    println!("\n=== how many keystrokes to reach column ids_j (index 14) ===");
    ed.execute(&format!("{}", l + 1)).unwrap();
    let t = Instant::now();
    for _ in 0..14 { ed.on_key(Key::Char('l')); }
    println!("  14 × l  {:?}  -> {}", t.elapsed(), at(&ed));
    println!("  14l as a count? ");
    ed.execute(&format!("{}", l + 1)).unwrap();
    keys(&mut ed, "14l");
    println!("  -> {}", at(&ed));

    println!("\n=== :count / :wc on a table ===");
    ed.execute("count").ok();
    println!("  {}", ed.status());

    println!("\n=== 'r' and 'R' in a cell ===");
    ed.execute(&format!("{}", l + 1)).unwrap();
    ed.on_key(Key::Char('r'));
    ed.on_key(Key::Char(','));
    println!("r , -> {} row {:?}", ed.status(), row(&ed, l));
    ed.on_key(Key::Char('r'));
    ed.on_key(Key::Char('金'));
    println!("r 金 -> {} row {:?}", ed.status(), row(&ed, l));
    ed.on_key(Key::Char('u'));

    println!("\n=== undo restores the file byte for byte ===");
    let out = dir.join("out.csv");
    ed.execute(&format!("w {}", out.display())).unwrap();
    println!("identical: {}", std::fs::read(&out).unwrap() == std::fs::read(dir.join("work.csv")).unwrap());
}
