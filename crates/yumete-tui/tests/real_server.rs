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
///
/// ⚠️ **`whose` is not decoration.** Two tests call this, and the path used to
/// be the same for both (`yumete-ra-<pid>`) while the first thing it does is
/// `remove_dir_all` — so running them together, which is what the command at
/// the top of this file does, had one pull the file out from under the other:
/// 「這個文件在外面被改過了」. 2026-09-23 審出來的。
fn a_broken_crate(whose: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yumete-ra-{whose}-{}", std::process::id()));
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
    let dir = a_broken_crate("answers");
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

    // ⚠️ **這裏沒有「讀完了」的信號可等。** 從前這一段寫成「等到有診斷為止」，
    // 而這一份是編得過的——一條診斷都不會有，於是它每一趟都燒滿九十秒，那句
    // 「先等它讀完項目」描述的是一條不存在的規則（2026-09-23 審出來的）。真正
    // 頂用的是下面那個「問不到就再問一次」的圈。
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
    let dir = a_broken_crate("saved");
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

/// **`空格 k` 真的問得出「這是什麽」**（#53 ③，2026-09-21）。
///
/// ⚠️ **回答的形狀是那個程序的事。** rust-analyzer 送的是 `MarkupContent`，
/// 裏面是一段 Markdown：一個 ```rust 圍欄裝着簽名，一條 `---`，然後文檔註釋。
/// 假服務器只會送這個測試教它送的東西。
#[test]
#[ignore = "needs rust-analyzer on the machine, and takes seconds"]
fn rust_analyzer_says_what_a_function_is() {
    if which("rust-analyzer").is_none() {
        eprintln!("no rust-analyzer on this machine — nothing to check");
        return;
    }
    let dir = std::env::temp_dir().join(format!("yumete-hover-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"asking\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/main.rs"),
        "/// 數一數有幾個字。\nfn counted(text: &str) -> usize {\n    text.chars().count()\n}\n\nfn main() {\n    println!(\"{}\", counted(\"那年冬天\"));\n}\n",
    )
    .unwrap();

    let mut editor = yumete_core::editor::Editor::new();
    editor.open_file(dir.join("src/main.rs")).unwrap();
    let config = yumete_config::Config {
        lsp: yumete_config::factory_servers(),
        ..Default::default()
    };
    let mut servers = yumete_tui::server::Servers::default();

    // 光標走到那一次調用上（第 6 行，0 起算）。
    editor.execute(":7").unwrap();
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
    assert!(standing_on(&editor).starts_with("counted"), "光標站在 counted 上");

    // ⚠️ **問不到就再問一次。** 服務器要先把整個項目讀完纔答得出來，而在那之前
    // 它答的是「無話可說」——這一份沒有錯誤，所以也沒有診斷可以拿來當「讀完了」
    // 的信號。真用起來也是這樣：讀者按一下沒出來，就再按一下。
    let gave_up = Instant::now() + Duration::from_secs(90);
    while Instant::now() < gave_up && editor.hover_here().is_none() {
        editor.on_key(yumete_core::input::Key::Char(' '));
        editor.on_key(yumete_core::input::Key::Char('k'));
        for _ in 0..10 {
            servers.follow(&editor, &config);
            servers.ask_what(&mut editor, &config);
            servers.collect(&mut editor);
            if editor.hover_here().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let told = editor.hover_here().map(str::to_string);
    servers.stop();
    let _ = std::fs::remove_dir_all(&dir);

    let told = told.expect("服務器說了點什麽（90 秒）");
    assert!(told.contains("counted"), "說的是這個函數：{told:?}");
    assert!(told.contains("usize"), "簽名在裏面：{told:?}");
    assert!(!told.contains("```"), "圍欄換成了行內代碼：{told:?}");
    assert!(told.contains("數一數有幾個字"), "文檔註釋也在：{told:?}");
}

/// **`C-n` 真的問得出「接下來能打什麽」，而且打進去的是對的那個字串**
/// （#53 ④，2026-09-21）。
///
/// ⚠️ **這一條是拿來驗兩件只有真服務器能驗的事：**
/// ① `snippetSupport: false` 真的讓 rust-analyzer 送 `counted` 而不是
///    `counted(${1:text})`——認了 snippet 又不會展開，那六個字符會進使用者的檔；
/// ② `textEdit.range` 蓋掉的正是已經打出來的那幾個字母。
#[test]
#[ignore = "needs rust-analyzer on the machine, and takes seconds"]
fn rust_analyzer_offers_what_comes_next_and_it_goes_in_clean() {
    if which("rust-analyzer").is_none() {
        eprintln!("no rust-analyzer on this machine — nothing to check");
        return;
    }
    let dir = std::env::temp_dir().join(format!("yumete-next-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"next\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    // `coun` 打了一半，等着補成 `counted`。
    std::fs::write(
        dir.join("src/main.rs"),
        "fn counted(text: &str) -> usize {\n    text.chars().count()\n}\n\nfn main() {\n    let n = coun\n}\n",
    )
    .unwrap();

    let mut editor = yumete_core::editor::Editor::new();
    editor.open_file(dir.join("src/main.rs")).unwrap();
    let config = yumete_config::Config {
        lsp: yumete_config::factory_servers(),
        ..Default::default()
    };
    let mut servers = yumete_tui::server::Servers::default();

    // 第 6 行（1 起算）的行尾，插入模式——`coun` 剛打完的樣子。
    editor.execute(":6").unwrap();
    editor.on_key(yumete_core::input::Key::Char('A'));
    assert_eq!(editor.mode(), yumete_core::input::Mode::Insert);

    let gave_up = Instant::now() + Duration::from_secs(120);
    while Instant::now() < gave_up && editor.offers_here().is_none() {
        editor.on_key(yumete_core::input::Key::Ctrl('n'));
        for _ in 0..10 {
            servers.follow(&editor, &config);
            servers.ask_next(&mut editor, &config);
            servers.collect(&mut editor);
            if editor.offers_here().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let offered: Vec<String> = editor
        .offers_here()
        .map(|(items, _)| items.iter().map(|o| o.label.clone()).collect())
        .unwrap_or_default();
    assert!(!offered.is_empty(), "服務器提了點什麽（120 秒）");
    assert!(
        offered.iter().any(|label| label.starts_with("counted")),
        "自己寫的那個函數在單子上：{offered:?}"
    );

    // 走到 `counted` 那一條上，按 Tab。
    for _ in 0..offered.len() {
        let picked = editor
            .offers_here()
            .map(|(items, at)| items[at].label.clone())
            .unwrap_or_default();
        if picked.starts_with("counted") {
            break;
        }
        editor.on_key(yumete_core::input::Key::Ctrl('n'));
    }
    editor.on_key(yumete_core::input::Key::Tab);

    let line = {
        let rope = editor.current_buffer().rope();
        rope.line(5).to_string()
    };
    servers.stop();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        line.trim_end(),
        "    let n = counted",
        "⚠️ 打進去的是乾淨的名字：没有 snippet 的 `${{1:…}}`，也没有把 `coun` 留在前面"
    );
}

/// **打着字，單子自己出來**（#53 ④ 的自動那一半，2026-09-21）。
///
/// ⚠️ **這一條驗的是次序。** 補全的問題必須排在 `didChange` 後面——服務器手上要
/// 是上一版正文，它答的就是「上一個字母之前那個詞後面能接什麽」。這裏不按
/// `C-n`，只是打字，然後讓事件循環自己轉。
#[test]
#[ignore = "needs rust-analyzer on the machine, and takes seconds"]
fn typing_alone_brings_the_list_up() {
    if which("rust-analyzer").is_none() {
        eprintln!("no rust-analyzer on this machine — nothing to check");
        return;
    }
    let dir = std::env::temp_dir().join(format!("yumete-auto-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"auto\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/main.rs"),
        "fn counted(text: &str) -> usize {\n    text.chars().count()\n}\n\nfn main() {\n    let n = \n}\n",
    )
    .unwrap();

    let mut editor = yumete_core::editor::Editor::new();
    editor.open_file(dir.join("src/main.rs")).unwrap();
    let config = yumete_config::Config {
        lsp: yumete_config::factory_servers(),
        ..Default::default()
    };
    let mut servers = yumete_tui::server::Servers::default();

    editor.execute(":6").unwrap();
    editor.on_key(yumete_core::input::Key::Char('A'));

    // 打 `coun`，一個字母一個字母地打，中間讓循環轉——**不按 C-n**。
    const WORD: &str = "coun";
    let gave_up = Instant::now() + Duration::from_secs(120);
    let mut typed = false;
    // ⚠️ **`Instant::now().elapsed()` 恆為零。** 從前那句重打的條件寫的是
    // `Instant::now().elapsed().as_secs() % 3 == 0`——從這一納秒到現在，永遠是
    // 0，於是**每一輪**都重打一次整個 `coun` 而只退一個字母，正文成了
    // `coucoucoucou…coun`（2026-09-23 插探針印出來的）。這條測試說的是「只打
    // 四個字母」，而那個情形一次都沒跑到。
    let mut retried = Instant::now();
    while Instant::now() < gave_up && editor.offers_here().is_none() {
        if !typed {
            for c in WORD.chars() {
                editor.on_key(yumete_core::input::Key::Char(c));
            }
            typed = true;
        }
        servers.follow(&editor, &config);
        servers.ask_next(&mut editor, &config);
        servers.collect(&mut editor);
        std::thread::sleep(Duration::from_millis(100));
        // 服務器還在讀項目的時候答的是空的；那就整段退掉重打一次，三秒一輪。
        if editor.offers_here().is_none() && retried.elapsed() >= Duration::from_secs(3) {
            retried = Instant::now();
            for _ in 0..WORD.chars().count() {
                editor.on_key(yumete_core::input::Key::Backspace);
            }
            typed = false;
        }
    }
    let offered: Vec<String> = editor
        .offers_here()
        .map(|(items, _)| items.iter().map(|o| o.label.clone()).collect())
        .unwrap_or_default();
    // 問的是哪一行——重打累積成一長串的時候，這一句是唯一看得出來的地方。
    let line = editor.current_buffer().rope().line(5).to_string();
    servers.stop();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(line.trim_end(), "    let n = coun", "打進去的就是四個字母");
    assert!(!offered.is_empty(), "光是打字就把單子帶出來了（120 秒）");
    assert!(
        offered.iter().any(|label| label.starts_with("counted")),
        "而且是這一份裏的函數：{offered:?}"
    );
}

/// The program, if it is on the PATH.
fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .map(|dir| PathBuf::from(dir).join(program))
        .find(|p| p.is_file())
}
