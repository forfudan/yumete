//! 界面語言（#494）——`:language` 與 `yumete --lang=`。
//!
//! ⚠️ **自己一個檔，因為語言是個全局。** `messages::set_language` 寫的是一個
//! `AtomicU8`，而 cargo 在同一個測試二進制裏是多執行緒跑的：把它放進
//! `editor/tests.rs` 會讓隔壁幾百條斷言繁體狀態行的測試隨機變紅。整合測試各自
//! 一個行程，這裏改的只影響這個檔。

use yumete_core::messages::{self, Language};
use yumete_core::Editor;

/// 跑完把語言放回去，免得這個檔裏的下一條被上一條帶跑。
struct PutBack(Language);
impl Drop for PutBack {
    fn drop(&mut self) {
        messages::set_language(self.0);
    }
}
fn borrow_the_language() -> PutBack {
    PutBack(messages::language())
}

/// `--help` 說得出口的三個名字，`parse` 都要認。
#[test]
fn the_three_names_the_help_advertises_all_parse() {
    for (name, want) in [
        ("zh", Language::Traditional),
        ("zhs", Language::Simplified),
        ("en", Language::English),
    ] {
        assert_eq!(Language::parse(name), Some(want), "--lang={name}");
    }
    assert_eq!(Language::parse("xx"), None, "認不得的要說認不得");
}

/// ⚠️ **兩件事寫成一條，因為語言是全局。** 同一個測試二進制裏 cargo 是多執行緒
/// 跑的，分成兩條就會互相踩——本來就是為了這個纔把整族搬進這個檔的，分條等於把
/// 同樣的競態搬小一級。
///
/// 一：`:language en` 當場換，而且**回話已經是新語言**——一個讀不懂繁體的讀者打了
/// 這一條，如果確認那句仍是繁體，他就不知道成沒成。
/// 二：分詞那一行的詞表名字也跟着換。從前它是 `yumete-cjk` 裏寫死的
/// `format!("{} 條（內置）")`，而那個 crate 沒有文案表——於是 `--lang=en` 的狀態行
/// 中間卡着四個繁體字。
#[test]
fn everything_the_editor_says_follows_the_language_in_force() {
    let _put_back = borrow_the_language();
    let mut editor = Editor::new();
    messages::set_language(Language::Traditional);

    editor.execute(":language en").expect("認得這條命令");
    assert_eq!(messages::language(), Language::English);
    assert_eq!(editor.status(), "interface language: en");

    // 簡體，以及短名 `:lang`。
    editor.execute(":lang zhs").expect("lang 是 language 的短名");
    assert_eq!(messages::language(), Language::Simplified);
    assert_eq!(editor.status(), "界面语言：zhs");

    // 不帶參數只問不答——語言不動。
    editor.execute(":language").expect("光問也行");
    assert_eq!(messages::language(), Language::Simplified);
    assert_eq!(editor.status(), "界面语言：zhs");

    // 詞表的名字也是說出來的，不是寫死的。
    editor.set_segmenter(Box::new(yumete_cjk::DictionarySegmenter::from_text(
        "宇夢\n那年\n",
        0,
    )));
    for (language, want) in [
        (Language::English, "2 entries (built in)"),
        (Language::Simplified, "2 条（内置）"),
        (Language::Traditional, "2 條（內置）"),
    ] {
        messages::set_language(language);
        assert_eq!(editor.words_in_force(), want, "{language:?}");
    }
}
