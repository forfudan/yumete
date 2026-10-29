// Shared by the core and config integration tests, so splitting the manual does
// not remove any of the checks against its real contents.
fn manual_text() -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/en");
    let summary = std::fs::read_to_string(root.join("SUMMARY.md")).expect("manual contents");
    let mut text = String::new();
    for line in summary.lines() {
        if let Some((_, tail)) = line.split_once("](manual/") {
            let name = tail.split(')').next().unwrap();
            text.push_str(&std::fs::read_to_string(root.join("manual").join(name)).expect("manual chapter"));
            text.push('\n');
        }
    }
    for name in ["help.md", "mode-keys.md"] {
        text.push_str(&std::fs::read_to_string(root.join(name)).expect("help guide"));
        text.push('\n');
    }
    text
}
