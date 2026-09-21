//! **One test against a real language server** (#53／#54, 2026-09-20).
//!
//! Everything else about the LSP work is tested without a server — the wire in
//! `yumete_core::lsp`, the rule in `yumete_tui::server` — and that is the right
//! way round: those tests are instant, deterministic, and run on a machine with
//! no toolchain on it.
//!
//! But **the handshake is the one thing a fake cannot check**. Whether
//! `initialize` is shaped the way `rust-analyzer` insists, whether it really
//! will not work until `initialized` arrives, whether it stalls on an
//! unanswered `client/registerCapability` — those are facts about another
//! program, and the only place they can be found out is against that program.
//!
//! ⚠️ **`#[ignore]` on purpose.** It needs `rust-analyzer` on the machine and
//! takes seconds, so it is not part of a normal run:
//!
//! ```text
//! cargo test -p yumete-tui --test real_server -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// A throwaway crate with one deliberate mistake in it.
fn a_broken_crate() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yumete-ra-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"broken\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    // `nowhere` is not a thing, and line 2 (0 起算 line 1) is where it is said.
    std::fs::write(dir.join("src/main.rs"), "fn main() {\n    nowhere();\n}\n").unwrap();
    dir
}

#[test]
#[ignore = "needs rust-analyzer on the machine, and takes seconds"]
fn rust_analyzer_really_answers() {
    if which("rust-analyzer").is_none() {
        eprintln!("no rust-analyzer on this machine — nothing to check");
        return;
    }
    let dir = a_broken_crate();
    // ⚠️ **The project root is worked out from the open file**, and the server
    // is started in it — so the editor has to be looking at the file inside
    // the crate, not at the crate from outside.
    let mut editor = yumete_core::editor::Editor::new();
    editor.open_file(dir.join("src/main.rs")).unwrap();
    let config = yumete_config::Config {
        lsp: yumete_config::factory_servers(),
        ..Default::default()
    };
    let mut servers = yumete_tui::server::Servers::default();

    let gave_up = Instant::now() + Duration::from_secs(90);
    let mut said = 0;
    while Instant::now() < gave_up {
        servers.follow(&editor, &config);
        servers.collect(&mut editor);
        said = editor.problem_count();
        if said > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    servers.stop();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(said > 0, "rust-analyzer 一句話都没說（90 秒）");
    // The mistake is on the second line, and that is where the mark goes.
    assert_eq!(
        editor.problem_on_line(1),
        Some(yumete_core::problem::Severity::Error),
        "那一行是第 2 行"
    );
}

/// **`gd` really jumps** (#53 ②, 2026-09-21).
///
/// ⚠️ **Only a live server can check this one.** The three answer shapes
/// (`Location`, `Location[]`, `LocationLink[]`) are a fact about the program
/// on the other end — `rust-analyzer` sends the third — and a fake would only
/// ever send back whatever this test wrote into it.
#[test]
#[ignore = "needs rust-analyzer on the machine, and takes seconds"]
fn rust_analyzer_says_where_a_function_is_written() {
    if which("rust-analyzer").is_none() {
        eprintln!("no rust-analyzer on this machine — nothing to check");
        return;
    }
    let dir = std::env::temp_dir().join(format!("yumete-gd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"jump\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    // `counted` is written on line 1 (0 起算) and called on line 5.
    std::fs::write(
        dir.join("src/main.rs"),
        "// 一個小程序\nfn counted(text: &str) -> usize {\n    text.chars().count()\n}\n\nfn main() {\n    println!(\"{}\", counted(\"那年冬天\"));\n}\n",
    )
    .unwrap();

    let mut editor = yumete_core::editor::Editor::new();
    editor.open_file(dir.join("src/main.rs")).unwrap();
    let config = yumete_config::Config {
        lsp: yumete_config::factory_servers(),
        ..Default::default()
    };
    let mut servers = yumete_tui::server::Servers::default();

    // 先等它讀完項目——問一個它還不認識的檔，答的是「哪兒都没有」。
    let gave_up = Instant::now() + Duration::from_secs(90);
    while Instant::now() < gave_up && editor.problem_count() == 0 {
        servers.follow(&editor, &config);
        servers.collect(&mut editor);
        std::thread::sleep(Duration::from_millis(200));
    }

    // 光標走到那一次**調用**上（第 6 行，0 起算）：`println!("{}", counted(…)`。
    // 一直按 `l` 直到真站在 `counted` 上，而不是數空格——數出來的那個數字錯了，
    // 測試就在測別的東西。
    editor.execute(":7").unwrap();
    let called = 6;
    assert_eq!(editor.cursor_line(), called, "`:7` 是 1 起算的第 7 行");
    let standing_on = |editor: &yumete_core::editor::Editor| -> String {
        let rope = editor.current_buffer().rope();
        let at = editor.cursor();
        rope.slice(at..(at + 7).min(rope.len_chars())).to_string()
    };
    for _ in 0..60 {
        if standing_on(&editor).starts_with("counted") {
            break;
        }
        editor.on_key(yumete_core::input::Key::Char('l'));
    }
    assert!(
        standing_on(&editor).starts_with("counted"),
        "光標站在 counted 上：{:?}",
        standing_on(&editor)
    );

    editor.on_key(yumete_core::input::Key::Char('g'));
    editor.on_key(yumete_core::input::Key::Char('d'));
    let gave_up = Instant::now() + Duration::from_secs(30);
    while Instant::now() < gave_up && editor.cursor_line() == called {
        servers.follow(&editor, &config);
        servers.ask(&mut editor, &config);
        servers.collect(&mut editor);
        std::thread::sleep(Duration::from_millis(100));
    }
    let landed = editor.cursor_line();
    servers.stop();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(landed, 1, "跳到了 `counted` 寫着的那一行（狀態：{}）", editor.status());
}

/// **Deleting the bad line really takes the mark away** (2026-09-21 報的：
/// 「我把之前的一個錯誤的行刪掉了，但是錯誤信息還在，這個行還是紅色的」)。
///
/// ⚠️ **This one is about `didSave`, and only a live server shows it.**
/// rust-analyzer answers out of two mouths: its own analysis, which follows
/// every `didChange`, and `cargo check`, which runs on `textDocument/didSave`
/// and nothing else. 「cannot find value ... in this scope」 comes out of the
/// second one, so without a save notification it is never taken back — the
/// red stays on a line that is not there any more. A fake server would say
/// whatever this test taught it to say, and prove nothing.
#[test]
#[ignore = "needs rust-analyzer on the machine, and takes seconds"]
fn a_mistake_that_is_deleted_and_saved_stops_being_reported() {
    if which("rust-analyzer").is_none() {
        eprintln!("no rust-analyzer on this machine — nothing to check");
        return;
    }
    let dir = a_broken_crate();
    let mut editor = yumete_core::editor::Editor::new();
    editor.open_file(dir.join("src/main.rs")).unwrap();
    let config = yumete_config::Config {
        lsp: yumete_config::factory_servers(),
        ..Default::default()
    };
    let mut servers = yumete_tui::server::Servers::default();

    let gave_up = Instant::now() + Duration::from_secs(90);
    while Instant::now() < gave_up && editor.problem_count() == 0 {
        servers.follow(&editor, &config);
        servers.collect(&mut editor);
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(editor.problem_count() > 0, "rust-analyzer 一句話都没說（90 秒）");

    // 刪掉出錯那一行（`:2` 是 1 起算的第 2 行；`x` 選整行、`d` 刪掉選區——
    // ⚠️ 默認鍵位是 helix 的，`dd` 在這裏是刪兩個字符，不是刪一行）。
    editor.execute(":2").unwrap();
    editor.on_key(yumete_core::input::Key::Char('x'));
    editor.on_key(yumete_core::input::Key::Char('d'));
    // 剩下的必須是一份編得過的程序——「不含 nowhere」不算數，把那一行
    // 改成別的錯照樣過得去，測試就在測別的東西。
    assert_eq!(
        editor.current_buffer().rope().to_string(),
        "fn main() {\n}\n",
        "那一行整行删掉了"
    );
    editor.execute(":w").unwrap();

    let gave_up = Instant::now() + Duration::from_secs(90);
    while Instant::now() < gave_up && editor.problem_count() > 0 {
        servers.follow(&editor, &config);
        servers.collect(&mut editor);
        std::thread::sleep(Duration::from_millis(200));
    }
    let still = editor.problem_count();
    servers.stop();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(still, 0, "存了之後錯誤要跟着没（90 秒）");
}

/// The program, if it is on the PATH.
fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .map(|dir| PathBuf::from(dir).join(program))
        .find(|p| p.is_file())
}
