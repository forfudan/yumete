//! **`ye --grep` from a subdirectory prints paths you can open.**
//!
//! 2026-10-30: run in `proj/sub` with no place named, it searched `sub` but
//! joined each hit to the project root, so `proj/sub/a.txt` was printed as
//! `a.txt` one level up, a file that does not exist. With `.` named it was
//! right, so the two spellings of "here" disagreed.

use std::process::Command;

#[test]
fn hits_from_a_subdirectory_are_relative_to_where_you_stand() {
    let root = std::env::temp_dir().join(format!("yumete-grep-paths-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let sub = root.join("sub");
    std::fs::create_dir_all(root.join(".git")).expect("a project marker");
    std::fs::create_dir_all(&sub).expect("a subdirectory");
    std::fs::write(sub.join("a.txt"), "alpha bravo\n").expect("a file to find");
    for args in [vec!["--grep", "bravo"], vec!["--grep", "bravo", "."]] {
        let out = Command::new(env!("CARGO_BIN_EXE_yumete"))
            .args(&args)
            .current_dir(&sub)
            .output()
            .expect("the binary to run");
        let said = String::from_utf8_lossy(&out.stdout);
        let first = said.lines().next().unwrap_or_default();
        // `path:line:col:text` — split from the right, so a Windows drive
        // letter's colon stays in the path.
        let path = first.rsplitn(4, ':').last().unwrap_or_default();
        assert!(sub.join(path).is_file(), "{args:?} printed {first:?}, which is not a file under sub/");
    }
    let _ = std::fs::remove_dir_all(&root);
}
