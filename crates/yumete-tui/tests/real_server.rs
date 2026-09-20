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

/// The program, if it is on the PATH.
fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .map(|dir| PathBuf::from(dir).join(program))
        .find(|p| p.is_file())
}
