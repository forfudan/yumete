//! **Talking to a language server — the wire, not the process** (#53/#54,
//! L2, 2026-09-20).
//!
//! [`crate::problem`] holds what a server *said*; this holds **how it is
//! said**. Everything in this module is a pure function over bytes and
//! strings: frame a message, take a message off a buffer, build the four
//! requests that matter, read a notification. Nothing here spawns anything,
//! opens anything, or blocks — the process and the thread belong to the front
//! end, which already owns the event loop.
//!
//! **That split is what makes this testable at all.** A protocol bug found
//! against a live `rust-analyzer` costs a spawn, a handshake and a guess about
//! whose fault it was; found here it costs a string literal.
//!
//! ## Why there is no `tower-lsp`, and no async
//!
//! 2026-09-20 定下的三條，寫在 `development.md` §5.4：**不要 async、不要
//! tokio、不要 `tower-lsp`**，新依賴只有 `serde_json`。The editor's event loop
//! is a `mpsc::Receiver` with a timeout, and it has carried every background
//! job this program has (the typesetter, git, the IME) without one. An async
//! runtime here would be a second way for things to happen, which is the
//! expensive kind of dependency.
//!
//! ## The one trap
//!
//! Warning: **LSP counts characters in UTF-16 code units.** Not bytes, not
//! characters — the unit of a language nobody here writes in. For ASCII the
//! three agree, which is exactly why it goes unnoticed until a Chinese comment
//! or a `𝄞` puts every mark on a line in the wrong place. So nothing in this
//! module quietly calls a UTF-16 number a column: [`crate::problem::Problem`]
//! carries `utf16_column` under that name, and turning it into a character
//! offset is [`crate::problem::char_column`], which needs the line's text and
//! is therefore done where the text is.

use crate::problem::{Problem, Severity};
use std::path::{Path, PathBuf};

/// Wrap a JSON body in the header a language server reads.
///
/// Warning: **The length is in bytes, not characters** — a message carrying a
/// Chinese identifier is longer than it looks, and a server given the
/// character count waits forever for the rest of a message that already
/// arrived.
pub fn frame(body: &str) -> Vec<u8> {
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

/// Take one complete message off the front of `buf`, if one is there.
///
/// `None` means 「not yet」, never 「broken」: a socket hands over whatever has
/// arrived, and half a message is the normal state of affairs. The caller reads
/// more and asks again.
///
/// Warning: **Other headers are allowed and must be skipped.** `Content-Type` is in
/// the specification, and a reader that insisted the first header was the
/// length would desynchronise on the first server that sends one — after which
/// every later message is garbage, and the symptom is 「diagnostics stopped
/// halfway through」.
pub fn take_frame(buf: &mut Vec<u8>) -> Option<String> {
    let end = find(buf, b"\r\n\r\n")?;
    let head = std::str::from_utf8(&buf[..end]).ok()?;
    let length: usize = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim().eq_ignore_ascii_case("content-length").then(|| value.trim().parse().ok())?
        })
        // A header block with no length is not a message this can ever
        // recover from; dropping it is the only way to get back in step.
        .unwrap_or(0);
    let from = end + 4;
    if buf.len() < from + length {
        return None;
    }
    let body = String::from_utf8(buf[from..from + length].to_vec()).ok();
    buf.drain(..from + length);
    body
}

/// Where `needle` starts in `hay`.
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

// ---- What we say ----------------------------------------------------------

/// The `initialize` request — the first thing any server is sent.
///
/// **What is claimed here is what the server will send.** The capabilities are
/// deliberately thin: this step wants `publishDiagnostics` and nothing else,
/// and a server told we can render markdown hovers will spend work making
/// them.
///
/// Warning: **`completionItem.snippetSupport` is claimed `false` on purpose**
/// (#53 ④). A snippet is not text — it is `counted(${1:text})`, a form with
/// holes in it, and an editor that does not fill the holes has to put those
/// six characters into the reader's file. Saying so is what makes
/// `rust-analyzer` send `counted` instead, which is exactly what this step can
/// insert. Warning: **Claiming it and then not expanding is the bug**, and it is a
/// quiet one: it only shows up on functions.
///
/// Warning: **`synchronization.didSave` is not a nicety.** A server that runs a real
/// compiler (rust-analyzer's `cargo check`) only re-runs it when the file is
/// saved, and a client that never claims to send saves is never sent one.
/// Without this line, an error stays on the screen after the line that caused
/// it is deleted — 「我把錯的行刪了，錯誤信息還在」（2026-09-21）。
/// Warning: **`contentFormat` 是一張按偏好排的單子，頭一個就是要的那一個。**
/// 從前這裏寫 `["plaintext","markdown"]`，於是 pylsp 照單取頭一個，回的是
/// `kind: "plaintext"` 的生 docstring——Python 的 docstring 慣例是
/// reStructuredText，`Main API` 底下那一行 `======` 就這麼原樣畫了出來
/// （2026-09-30 報的）。排 `markdown` 在前，pylsp 就走 `docstring-to-markdown`
/// 把 reST/numpydoc 譯成 Markdown 再送（譯不動的原樣退回，不會更差）。
pub fn initialize(id: i64, root: &Path) -> String {
    let root = uri_of(root);
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"initialize","params":{{"processId":{pid},"rootUri":{root},"capabilities":{{"window":{{"workDoneProgress":true}},"textDocument":{{"publishDiagnostics":{{"relatedInformation":false}},"synchronization":{{"didSave":true}},"hover":{{"contentFormat":["markdown","plaintext"]}},"completion":{{"completionItem":{{"snippetSupport":false}}}},"signatureHelp":{{"signatureInformation":{{"documentationFormat":["plaintext"]}}}}}}}}}}}}"#,
        pid = std::process::id(),
        root = json_string(&root),
    )
}

/// The `initialized` notification — 「go ahead」, sent once the answer to
/// [`initialize`] is in. Warning: A server that never gets it may never start work.
pub fn initialized() -> String {
    r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#.to_string()
}

/// `textDocument/didOpen` — this file, this language, this text.
pub fn did_open(path: &Path, language: &str, version: i64, text: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":{uri},"languageId":{language},"version":{version},"text":{text}}}}}}}"#,
        uri = json_string(&uri_of(path)),
        language = json_string(language),
        text = json_string(text),
    )
}

/// `textDocument/didChange`, **whole file** (`TextDocumentSyncKind.Full`).
///
/// Warning: **The whole file, on purpose.** Incremental sync is a second model of
/// what the document is, kept in step with this one by hand — and when it
/// drifts the server answers about text nobody has, which looks exactly like a
/// server bug. A megabyte of Rust is a millisecond of JSON; the moment that
/// stops being true is the moment to write the incremental path, and not
/// before.
pub fn did_change(path: &Path, version: i64, text: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didChange","params":{{"textDocument":{{"uri":{uri},"version":{version}}},"contentChanges":[{{"text":{text}}}]}}}}"#,
        uri = json_string(&uri_of(path)),
        text = json_string(text),
    )
}

/// `textDocument/definition` — 「where is this thing written?」 (#53 ②).
///
/// Warning: **The column is in UTF-16 code units**, like every position on this
/// wire. [`crate::problem::utf16_column`] turns a character offset into one,
/// and it needs the line's text to do it.
pub fn definition(id: i64, path: &Path, line: usize, utf16_column: usize) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"textDocument/definition","params":{{"textDocument":{{"uri":{uri}}},"position":{{"line":{line},"character":{utf16_column}}}}}}}"#,
        uri = json_string(&uri_of(path)),
    )
}

/// `textDocument/hover` — 「這是什麽？」 (#53 ③).
///
/// The column is UTF-16, like every position on this wire.
pub fn hover(id: i64, path: &Path, line: usize, utf16_column: usize) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"textDocument/hover","params":{{"textDocument":{{"uri":{uri}}},"position":{{"line":{line},"character":{utf16_column}}}}}}}"#,
        uri = json_string(&uri_of(path)),
    )
}

/// `textDocument/signatureHelp` —— **這個括號裏該填什麼**（2026-10-08）。
///
/// 報的原話：「In insert mode, when I type `(`, I expect that the function doc can
/// appear without triggering, so that I can understand the function better.」
///
/// Warning: **它和 hover 是兩個請求。** hover 答的是「光標底下這個名字是什麼」，這一支
/// 答的是「你正在填的這一次調用，簽名長什麼樣、填到第幾個參數了」——位置一樣，問題
/// 不一樣。helix 的 `auto-signature-help` 問的就是它。
///
/// `documentationFormat` 只報 `plaintext`：這一則畫在一行上（見
/// [`Signature`]），Markdown 在那裏沒有地方施展。
pub fn signature_help(id: i64, path: &Path, line: usize, utf16_column: usize) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"textDocument/signatureHelp","params":{{"textDocument":{{"uri":{uri}}},"position":{{"line":{line},"character":{utf16_column}}}}}}}"#,
        uri = json_string(&uri_of(path)),
    )
}

/// `textDocument/completion` — 「接下來能打什麽？」 (#53 ④).
pub fn completion(id: i64, path: &Path, line: usize, utf16_column: usize) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"textDocument/completion","params":{{"textDocument":{{"uri":{uri}}},"position":{{"line":{line},"character":{utf16_column}}}}}}}"#,
        uri = json_string(&uri_of(path)),
    )
}

/// `textDocument/didSave` — 「這一份落盤了」。
///
/// Warning: **This is what re-runs the compiler.** rust-analyzer's own analysis
/// follows every `didChange`, but its `cargo check` — where 「cannot find
/// value ... in this scope」 comes from, the ones marked `(rustc)` — waits for
/// a save. No save, no new answer, and the old one stays on a line that is not
/// there any more.
///
/// The text is not sent: the server already has it from [`did_change`], and a
/// server that wants it back says so in `save.includeText`, which nothing here
/// claims.
pub fn did_save(path: &Path) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didSave","params":{{"textDocument":{{"uri":{uri}}}}}}}"#,
        uri = json_string(&uri_of(path)),
    )
}

/// `textDocument/didClose` — stop watching this one.
pub fn did_close(path: &Path) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didClose","params":{{"textDocument":{{"uri":{uri}}}}}}}"#,
        uri = json_string(&uri_of(path)),
    )
}

/// `shutdown`, then `exit` — the polite way out.
pub fn shutdown(id: i64) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"shutdown","params":null}}"#)
}

/// The notification that follows [`shutdown`].
pub fn exit() -> String {
    r#"{"jsonrpc":"2.0","method":"exit","params":null}"#.to_string()
}

// ---- What we hear ---------------------------------------------------------

/// One place a server pointed at — a file and a position in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub path: PathBuf,
    pub line: usize,
    /// Warning: In **UTF-16 code units**, as it arrived; see
    /// [`crate::problem::Problem::utf16_column`].
    pub utf16_column: usize,
}

/// What a message from the server turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// The answer to [`initialize`]: the handshake is done and
    /// [`initialized`] must go back before the server will work.
    Ready,
    /// `textDocument/publishDiagnostics` — everything now true about one file.
    Said { path: PathBuf, said: Vec<Problem> },
    /// A request *from* the server that wants an answer (`window/workDoneProgress
    /// /create`, `client/registerCapability`). Warning: **An unanswered request can
    /// stall a server** — `rust-analyzer` waits on its own registration — so
    /// the id comes back out to be answered with an empty result.
    ///
    /// Warning: **號碼照 JSON 原樣帶着，不是 `i64`**（2026-10-07 審出來的）。協議說
    /// `id` 可以是數字**也可以是字串**，而從前這裏是 `as_i64()`：字串號碼的請求
    /// 讀成了 [`Notice::Nothing`]，於是**永遠沒人答**，而那正是這一格存在的理由
    /// ——等自己註冊回音的服務器就停在那裏。`"3"` 原樣回 `"3"`，數字原樣回數字。
    Asked { id: String },
    /// **The answer to something we asked**, with the id that says which.
    ///
    /// Warning: **Typed, not raw JSON.** Only the front end knows which id it sent
    /// for what, so the id comes back untouched — but the *shape* of the
    /// answer is this crate's business, and a `serde_json::Value` crossing the
    /// boundary would make it everybody's.
    /// Warning: **Both readings, because the reader cannot know which was asked.**
    /// An answer carries an id and no method — nothing else in it says whether
    /// it came back from `definition` or from `hover`. Only the front end knows
    /// what it sent that id for, so both readings are offered and it takes the
    /// one it is waiting for: a definition fills `places` and leaves `told`
    /// empty; a hover does the opposite; a server that answered `null` fills
    /// neither, which is 「nothing to say」 in both languages.
    Answer {
        id: i64,
        places: Vec<Place>,
        told: Option<Told>,
        offers: Vec<Offer>,
        /// 簽名那一問的答案（2026-10-08）。同上：讀出來擺着，前端拿它要的那一格。
        signature: Option<Signature>,
    },
    /// **服務器說它在忙，或者忙完了**（`$/progress`，2026-10-01）。
    ///
    /// `token` 是那一件活自己的號碼，`begin` 開一件、`end` 關一件（`report`
    /// 不算開也不算關）。轉圈那八個點數的就是**還開着幾件**。
    ///
    /// Warning: **一定要宣告 `window.workDoneProgress`，不然一條都收不到**
    /// （2026-10-01 量的：宣告之前三十秒零條；宣告之後起一次 rust-analyzer 是
    /// 221 條，`begin` 十五次 `end` 十五次，兩秒四收場）。
    Working { token: String, begin: bool, end: bool },
    /// Anything else: logs, an answer nobody is waiting for.
    Nothing,
}

/// **正在填的那一次調用，簽名長什麼樣**（`textDocument/signatureHelp`，2026-10-08）。
///
/// Warning: **只留一行。** 服務器可以給好幾個重載（`signatures`），這一頭取它說的那個
/// 「正在用的」（`activeSignature`，不說就是第一個）——一行浮在光標旁邊，是打字當口
/// 看得完的全部。別的重載要看，那是 `空格 k` 的事。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// 那一行本身，例如 `fn push(&mut self, value: T)`。
    pub label: String,
    /// **正在填第幾個參數**，以及它在 `label` 裏的哪一段（字符下標，不是字節）。
    ///
    /// `None` ＝ 服務器沒說，或者說的是一個這一頭對不上的形狀。畫的那一頭拿它把
    /// 那一段加重——「填到哪了」正是這一則存在的理由。
    pub active: Option<(usize, usize)>,
}

/// **One thing the server says could come next** (#53 ④).
///
/// Warning: **`label` is what is shown and `insert` is what is typed, and they are
/// not the same string.** A method comes back labelled `count()` and inserted
/// as `count`; a field of a struct is labelled `text: &str` and inserted as
/// `text`. Showing the insert text makes the list useless to read; inserting
/// the label puts the type annotation into the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// What the list shows.
    pub label: String,
    /// What goes into the file.
    pub insert: String,
    /// 「method」「field」「keyword」… — the server's own number, kept as a
    /// number because the names are this crate's business only when they are
    /// drawn. `0` for an item that did not say.
    pub kind: i64,
    /// The type, the signature, the module it comes from — one short line the
    /// server offers *about* the item. `None` when it offered none.
    pub detail: Option<String>,
    /// **How much of what is already typed this replaces** — which line, and
    /// the two UTF-16 code-unit columns on it.
    ///
    /// Warning: **The line is carried, not assumed** (2026-10-07). It used to read
    /// `start.character` and `end.character` and drop both line numbers, and
    /// the caller then applied those two columns to **whatever line the caret
    /// was on**. A server whose range is one line up — a multi-line edit, or
    /// an item computed against a stale copy — silently cut an arbitrary span
    /// out of the line being typed. A range that spans two lines is refused
    /// here rather than flattened: this field's whole contract is 「how much of
    /// *this* word」, and nothing in the caller can honour more.
    ///
    /// Warning: **The server decides this, not the editor.** `self.co` completing to
    /// `count` replaces `co` — three characters back from the caret, or two,
    /// or none, depending on what the server thinks the word is. Guessing it
    /// with the editor's own idea of a word is right for `co` and wrong for
    /// `a.b`, `#[der`, `'lifet` and every language whose words are not this
    /// language's words. `None` when the server sent no edit range, and then
    /// the caller replaces nothing.
    pub replacing: Option<Replacing>,
}

/// The span one completion item overwrites: a line, and two columns on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Replacing {
    /// Which line the server meant, counted from zero as LSP counts.
    pub line: usize,
    /// Where the span starts and ends on it, in **UTF-16 code units**.
    pub from: usize,
    pub to: usize,
}

/// Read one message.
///
/// **Never fails.** A server is another program, and the only useful response
/// to a message this does not understand is to carry on — a parse error that
/// stopped the editor would make every server's quirk into a crash.
pub fn read(message: &str, initialize_id: i64) -> Notice {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(message) else {
        return Notice::Nothing;
    };
    let method = value.get("method").and_then(|m| m.as_str());
    // 這一頭發出去的號碼一律是數字，所以「回答」那兩檔照數字讀；**服務器自己發
    // 來的請求**那一檔不是這一頭取的名字，字串也認（見 [`Notice::Asked`]）。
    let asked = match value.get("id") {
        Some(serde_json::Value::Number(n)) => Some(n.to_string()),
        Some(serde_json::Value::String(text)) => Some(serde_json::Value::String(text.clone()).to_string()),
        _ => None,
    };
    let id = value.get("id").and_then(|i| i.as_i64());
    match (method, id) {
        (None, Some(id)) if id == initialize_id => Notice::Ready,
        (Some("textDocument/publishDiagnostics"), _) => said(&value),
        (Some("$/progress"), _) => working(&value),
        // A request from the server: it has both a method *and* an id.
        (Some(_), Some(id)) => Notice::Asked { id: asked.unwrap_or_else(|| id.to_string()) },
        // 號碼是字串的請求——同上一檔，只是號碼不是數字。
        (Some(_), None) if asked.is_some() => Notice::Asked { id: asked.unwrap_or_default() },
        // An answer to something we sent: an id and no method.
        (None, Some(id)) => Notice::Answer {
            id,
            places: places(value.get("result")),
            told: told(value.get("result")),
            offers: offers(value.get("result")),
            signature: signature(value.get("result")),
        },
        _ => Notice::Nothing,
    }
}

/// `$/progress` — 一件活開了還是完了。
///
/// `token` 可以是字串也可以是數字（協議兩種都許），這裏一律當字串記。看不懂的
/// 形狀回 [`Notice::Nothing`]——一條進度消息讀不出來，不是停下來的理由。
fn working(value: &serde_json::Value) -> Notice {
    let Some(params) = value.get("params") else { return Notice::Nothing };
    let token = match params.get("token") {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => return Notice::Nothing,
    };
    let kind = params.get("value").and_then(|v| v.get("kind")).and_then(|k| k.as_str());
    match kind {
        Some("begin") => Notice::Working { token, begin: true, end: false },
        Some("end") => Notice::Working { token, begin: false, end: true },
        // `report` 只是說「還在忙」，開着的那一件還開着。
        Some("report") => Notice::Working { token, begin: false, end: false },
        _ => Notice::Nothing,
    }
}

/// The places in a `textDocument/definition` answer.
///
/// Warning: **Three shapes, all of them legal.** The specification lets a server
/// answer with one `Location`, an array of them, or an array of
/// `LocationLink` (which spells the range `targetSelectionRange` instead) —
/// and servers really do differ: `rust-analyzer` sends `LocationLink`, others
/// send a bare `Location`. Reading only one shape works until the day somebody
/// uses the other server.
fn places(result: Option<&serde_json::Value>) -> Vec<Place> {
    let one = |value: &serde_json::Value| -> Option<Place> {
        // `LocationLink` names the file `targetUri` and the spot
        // `targetSelectionRange`; `Location` calls them `uri` and `range`.
        let uri = value.get("uri").or_else(|| value.get("targetUri"))?.as_str()?;
        let range = value
            .get("range")
            .or_else(|| value.get("targetSelectionRange"))
            .or_else(|| value.get("targetRange"))?;
        let at = range.get("start")?;
        Some(Place {
            path: path_of(uri)?,
            line: at.get("line")?.as_u64()? as usize,
            utf16_column: at.get("character").and_then(|c| c.as_u64()).unwrap_or(0) as usize,
        })
    };
    match result {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::Array(many)) => many.iter().filter_map(one).collect(),
        Some(value) => one(value).into_iter().collect(),
    }
}

/// What a `textDocument/hover` answer says, as plain text (#53 ③).
///
/// Warning: **Three shapes again, and one of them is deprecated but still sent.**
/// `contents` is a `MarkupContent` (`{kind, value}`), a `MarkedString` (a bare
/// string, or `{language, value}`), or an array of `MarkedString`.
///
/// The value is Markdown, **and it stays Markdown** — this editor sets
/// Markdown for a living, and the float draws it with the same inks the page
/// does (2026-09-21 問：「它為什麼不能渲染 markdown 呢？我覺得完全可以呀。」).
/// Only the block shapes a one-paragraph float cannot hold are rewritten, and
/// each is rewritten into the Markdown that *means the same thing inline*:
/// a fenced block becomes one `` `code` `` span a line, and a `---` rule
/// becomes the blank line it was standing in for.
fn told(result: Option<&serde_json::Value>) -> Option<Told> {
    // `(text, markdown)`. Warning: **`kind` is the server's own word for what it
    // sent** (2026-09-30) — see [`Told::markdown`].
    fn one(value: &serde_json::Value) -> Option<(String, bool)> {
        match value {
            // A bare `MarkedString`: the specification says Markdown.
            serde_json::Value::String(text) => Some((text.clone(), true)),
            // Both `MarkupContent` and the old `{language, value}` keep the
            // text under `value`. The old shape is a code block, so Markdown;
            // the new one says which it is.
            _ => {
                let text = value.get("value")?.as_str()?.to_string();
                let plain = value.get("kind").and_then(|k| k.as_str()) == Some("plaintext");
                Some((text, !plain))
            }
        }
    }
    let contents = result?.get("contents")?;
    let (raw, markdown) = match contents {
        serde_json::Value::Array(many) => {
            let parts: Vec<(String, bool)> = many.iter().filter_map(one).collect();
            if parts.is_empty() {
                return None;
            }
            // One array, one answer: it is Markdown unless every part is plain.
            let markdown = parts.iter().any(|(_, md)| *md);
            let text: Vec<&str> = parts.iter().map(|(t, _)| t.as_str()).collect();
            (text.join("\n\n"), markdown)
        }
        value => one(value)?,
    };
    // Warning: **`inline` rewrites Markdown, so plain text must not go through it**
    // — it would eat a line of three dashes and put backticks round a table
    // drawn in ASCII. Plain text is set as it came.
    let text = match markdown {
        true => inline(&raw),
        false => raw.trim_end().to_string(),
    };
    match text.is_empty() {
        true => None,
        false => Some(Told { text, markdown }),
    }
}

/// **What a server said, and whether it said it in Markdown** (2026-09-30).
///
/// Warning: **The `kind` field is not decoration.** This used to read `value` out
/// of a `MarkupContent` and throw the `kind` beside it away, and the float
/// then set every answer with the Markdown inks — so a server that only
/// speaks plain text had its `*` turned into emphasis and its `#` into a
/// heading. `contentFormat` now asks for Markdown first (see [`initialize`]),
/// which is what most servers will then send; this is for the ones that
/// cannot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Told {
    pub text: String,
    /// Whether [`Self::text`] is Markdown and should be set with its inks.
    pub markdown: bool,
}

impl From<String> for Told {
    /// Markdown, which is what every shape but an explicit `plaintext` means.
    fn from(text: String) -> Self {
        Self { text, markdown: true }
    }
}

impl From<&str> for Told {
    fn from(text: &str) -> Self {
        Self::from(text.to_string())
    }
}

/// A hover answer's Markdown, in the shapes a one-paragraph float can set.
/// See [`told`].
fn inline(raw: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut fenced = false;
    for line in raw.lines() {
        let bare = line.trim_end();
        let start = bare.trim_start();
        if start.starts_with("```") || start.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            // **The signature, and it is the most useful line in the answer.**
            // Backticks rather than a fence because the float sets *inline*
            // markup — and `` `fn counted(text: &str) -> usize` `` is the same
            // code in the same ink, in a shape this editor already draws.
            // Warning: A backtick inside the code would close the span early; a line
            // holding one is left bare rather than set wrong.
            out.push(match bare.contains('`') || bare.trim().is_empty() {
                true => bare.to_string(),
                false => format!("`{}`", bare.trim_end()),
            });
            continue;
        }
        match start {
            // A horizontal rule is how rust-analyzer separates the signature
            // from the documentation. One blank line says the same thing.
            "---" | "***" | "___" => out.push(String::new()),
            _ => out.push(bare.to_string()),
        }
    }
    let out: Vec<&str> = out.iter().map(String::as_str).collect();
    // Runs of blank lines collapse, and the ends are trimmed: the fences and
    // rules that were dropped leave holes behind them.
    let mut text = String::new();
    let mut blank = true;
    for line in out {
        if line.is_empty() {
            blank = true;
            continue;
        }
        if !text.is_empty() {
            text.push('\n');
            if blank {
                text.push('\n');
            }
        }
        text.push_str(line);
        blank = false;
    }
    text
}

/// What a `textDocument/completion` answer offers (#53 ④).
///
/// Warning: **Two shapes**: a bare array of items, or a `CompletionList` with them
/// under `items` (and an `isIncomplete` this step does not use — it means
/// 「ask again as they type」, which is a refinement of *when* to ask).
///
/// The list is taken in the order the server sent it. Warning: **Not sorted here**:
/// a server ranks its own offers (`sortText` is what it ranks them by, and it
/// is not always the label), and an editor that re-sorted would be overruling
/// the one party that knows the language.
fn offers(result: Option<&serde_json::Value>) -> Vec<Offer> {
    let items = match result {
        None | Some(serde_json::Value::Null) => return Vec::new(),
        Some(serde_json::Value::Array(many)) => many.clone(),
        Some(value) => match value.get("items").and_then(|i| i.as_array()) {
            Some(items) => items.clone(),
            None => return Vec::new(),
        },
    };
    items.iter().filter_map(one_offer).collect()
}

/// One `CompletionItem`. See [`offers`].
fn one_offer(item: &serde_json::Value) -> Option<Offer> {
    let label = item.get("label")?.as_str()?.to_string();
    // `textEdit` is both what to type and what to replace; `insertText` is
    // only what to type; the label is the fallback the specification names.
    let edit = item
        .get("textEdit")
        .and_then(|e| e.get("newText").and_then(|t| t.as_str()).map(|text| (e, text)));
    let insert = match (edit, item.get("insertText").and_then(|t| t.as_str())) {
        (Some((_, text)), _) => text.to_string(),
        (None, Some(text)) => text.to_string(),
        (None, None) => label.clone(),
    };
    // **`insertTextFormat: 2` 是片段，照字面插進去就是往稿子裏寫 `${1:}`**
    // （2026-10-07 審出來的）。
    //
    // 這一頭報的是 `snippetSupport: false`（見 [`initialize`]），所以守規矩的服務
    // 器不會送片段來——可「不會送」靠的是對方守規矩，而這一條錯的代價是**在人的
    // 檔裏留下幾個他沒打的字**。所以照實問一句，是片段就把洞填平。
    //
    // 不拿 `label` 兜底：rust-analyzer 的 label 是 `counted(…)`，那個省略號比
    // `${1:text}` 還糟。
    let insert = match item.get("insertTextFormat").and_then(|f| f.as_i64()) {
        Some(2) => without_the_holes(&insert),
        _ => insert,
    };
    let replacing = edit.and_then(|(e, _)| {
        // A `TextEdit` has `range`; an `InsertReplaceEdit` has `insert` and
        // `replace` instead, and **replace is the one that means 「the word
        // that is there now」**.
        let range = e.get("range").or_else(|| e.get("replace")).or_else(|| e.get("insert"))?;
        let at = |which: &str, part: &str| -> Option<usize> {
            Some(range.get(which)?.get(part)?.as_u64()? as usize)
        };
        let line = at("start", "line")?;
        // 跨行的範圍這一頭接不住——接住了也只能亂切。寧可當作「沒給範圍」，
        // 那一檔的意思是「什麼都不蓋，只在光標處插入」。
        if at("end", "line")? != line {
            return None;
        }
        Some(Replacing { line, from: at("start", "character")?, to: at("end", "character")? })
    });
    Some(Offer {
        label,
        insert,
        kind: item.get("kind").and_then(|k| k.as_i64()).unwrap_or(0),
        detail: item
            .get("detail")
            .and_then(|d| d.as_str())
            .map(|d| d.replace('\n', " ").trim().to_string())
            .filter(|d| !d.is_empty()),
        replacing,
    })
}

/// **把一段片段裏的洞填平**，留下的是它的字面那一半。
///
/// `counted(${1:text})` → `counted(text)`，`println!("$0")` → `println!("")`。
/// 規矩照 LSP 的片段語法：`$n`／`${n}` 是個光標位置，沒有字，去掉；
/// `${n:默認}` 留那個默認；`${n|甲,乙|}` 留頭一個；`\$`／`\}`／`\\` 是轉義，
/// 脫掉那條反斜線。
///
/// Warning: **這不是片段引擎**——它不給跳轉，只保證插進去的是字，不是語法。真要做
/// 跳轉是另一件事（報 `snippetSupport: true`、存一列洞、`Tab` 走），而那件事沒做
/// 之前，這一支是那條「稿子裏不許出現沒打過的字」的底線。
/// **一段 hover 裏的「正文第一段」**（2026-10-08 定）。
///
/// 簽名回空時那一句回退用的（見 `Editor::show_signature_from_doc`）。三家都不在
/// 插入態冒整段文檔，所以最多一段，後面還有就單排一個 `…`。
///
/// 服務器把簽名與說明排在一起：rust-analyzer 的 hover 是「一對圍欄 ＋ `---`
/// ＋ 正文」。所以跳過圍欄裏的東西與分隔線，第一行真正的散文起算一段。
///
/// `None` ＝整段沒有一句散文（只有簽名）——那就什麼都不畫，寧可不說。
pub fn first_paragraph(told: &str) -> Option<String> {
    let mut fenced = false;
    let mut said: Vec<&str> = Vec::new();
    let mut more = false;
    for line in told.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            // 圍欄開著的時候碰到第二道就是關；收過一段之後碰到圍欄就是「後面還有」。
            if !said.is_empty() {
                more = true;
                break;
            }
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        // **那一對圍欄這一路上早被拆過了**（2026-10-09 補）。
        //
        // Warning: 這一支收的是 [`Told::text`]，而 `Told` 裏那段字已經過了
        // [`inline`]——圍欄一行不剩，裏面那幾行各自裹上了一對反引號。於是上面那道
        // 跳圍欄的閘一次也沒開過，`println!` 的 hover 交出來的是 `` `std::macros` ``
        // 而不是「Prints to the standard output」。整行就是一段行內代碼的，在散文
        // 開始之前一律當簽名跳掉；反引號只有首尾那兩個纔算，不然「`甲` 和 `乙`」
        // 這種真散文也會被當成簽名。
        let only_code = trimmed.len() >= 2
            && trimmed.starts_with('`')
            && trimmed.ends_with('`')
            && trimmed.matches('`').count() == 2;
        if said.is_empty() && only_code {
            continue;
        }
        // `---` 是 rust-analyzer 拿來隔開簽名與說明的，不是正文。
        if trimmed.chars().all(|c| c == '-') && trimmed.len() >= 3 {
            continue;
        }
        if trimmed.is_empty() {
            if said.is_empty() {
                continue;
            }
            // 一段完了。後面還有沒有字，決定要不要那一個 `…`。
            more = told
                .lines()
                .skip_while(|l| !std::ptr::eq(l.as_ptr(), line.as_ptr()))
                .skip(1)
                .any(|l| !l.trim().is_empty());
            break;
        }
        said.push(trimmed);
    }
    if said.is_empty() {
        return None;
    }
    let mut out = said.join("\n");
    if more {
        out.push_str("\n…");
    }
    Some(out)
}

fn without_the_holes(snippet: &str) -> String {
    let mut out = String::new();
    let mut chars = snippet.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // 轉義：下一個字原樣留着。
            '\\' => match chars.next() {
                Some(next) => out.push(next),
                None => out.push('\\'),
            },
            '$' => match chars.peek() {
                // `${…}`：數字之後要麼就完了，要麼 `:默認`，要麼 `|甲,乙|`。
                Some('{') => {
                    chars.next();
                    let mut body = String::new();
                    let mut depth = 1usize;
                    while let Some(c) = chars.next() {
                        match c {
                            '\\' => {
                                if let Some(next) = chars.next() {
                                    body.push(next);
                                }
                            }
                            '{' => {
                                depth += 1;
                                body.push(c);
                            }
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                                body.push(c);
                            }
                            _ => body.push(c),
                        }
                    }
                    // `1:默認` 的默認那一半；沒有冒號就整個是個序號，沒有字。
                    match body.split_once(':') {
                        Some((_, text)) => out.push_str(&without_the_holes(text)),
                        None => {
                            if let Some((_, choices)) = body.split_once('|') {
                                out.push_str(
                                    choices.trim_end_matches('|').split(',').next().unwrap_or(""),
                                );
                            }
                        }
                    }
                }
                // `$1`、`$0`：一串數字，去掉。`$` 後面不是數字也不是 `{` 的話，
                // 那個 `$` 就是個普通的錢號（服務器本該轉義，沒轉義就按字面算）。
                Some(c) if c.is_ascii_digit() => {
                    while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                        chars.next();
                    }
                }
                _ => out.push('$'),
            },
            _ => out.push(c),
        }
    }
    out
}

/// **按已經打出來的那半個詞排一遍**（2026-10-08 報的）。
///
/// 報的原話：「I typed "unwrap", the function hint (autocompletion), however, still
/// show the prediction using the original order. … items with exact prefix go first;
/// fuzzy matching is nice-to-have but should go at bottoms.」截圖裏打完 `.unwrap`
/// 之後單子頭三條還是 `is_some_and`／`is_none_or`／`expect`。
///
/// **為什麼服務器不替我們排。** 一張 `isIncomplete: false` 的單子，協議的意思是
/// 「這就是全部，接着打字由**你**自己篩」——rust-analyzer 因此一次給出那個點後面
/// 全部 131 條，按它自己算的相關度排，而「使用者已經打了 `unwrap`」這件事它不再過
/// 問。VS Code、helix 都在這一頭篩。
///
/// Warning: **只重排，不扔。** 配不上的落到最後，不從單子上消失——這一頭認的是 `label`，
/// 而服務器真正拿來篩的是 `filterText`（還沒接）。兩者不一樣的那天，扔掉的就是人要
/// 的那一條；排到後面只是看不見，`C-n` 一路走下去還找得到。
///
/// 分五檔，檔內**保持服務器給的次序**（它那一檔是相關度，比這一頭懂得多）：
///
/// | 檔 | 什麼 |
/// | --- | --- |
/// | 0 | 整個就是它 |
/// | 1 | 前綴，大小寫也對 |
/// | 2 | 前綴，不計大小寫 |
/// | 3 | 字母按順序出現過（模糊） |
/// | 4 | 配不上 |
pub fn rank(items: &mut [Offer], word: &str) {
    if word.is_empty() {
        return;
    }
    let small = word.to_lowercase();
    let tier = |offer: &Offer| -> u8 {
        let label = offer.label.as_str();
        if label == word {
            return 0;
        }
        if label.starts_with(word) {
            return 1;
        }
        let quiet = label.to_lowercase();
        if quiet.starts_with(&small) {
            return 2;
        }
        // 字母按順序出現過就算模糊配得上——`unwrap` 配得上 `unwrap_or_default`，
        // 也配得上 `u_n_w…`。不打分：這一檔本來就排在後面。
        let mut want = small.chars().peekable();
        for c in quiet.chars() {
            if want.peek() == Some(&c) {
                want.next();
            }
        }
        match want.peek().is_none() {
            true => 3,
            false => 4,
        }
    };
    // `sort_by_key` 是穩定排序，所以同一檔裏服務器給的次序一個字都不動。
    items.sort_by_key(tier);
}

/// Read a `SignatureHelp` answer. See [`Signature`].
///
/// Warning: **`activeParameter` 說的是第幾個參數，不是第幾格字。** 哪一段要加重得自己
/// 去 `parameters` 那張表上取：那裏的 `label` 可以是一個字串（在簽名裏找它），也可以
/// 是一對 UTF-16 下標。兩種都認——rust-analyzer 給下標，pylsp 給字串。
///
/// Warning: **參數那一層也有自己的 `activeParameter`**（協議 3.16 起）：簽名自己說的
/// 蓋過頂上那一個。頂上那個是「這一組的默認」。
fn signature(result: Option<&serde_json::Value>) -> Option<Signature> {
    let result = result?;
    let all = result.get("signatures")?.as_array()?;
    let which = result.get("activeSignature").and_then(|n| n.as_u64()).unwrap_or(0) as usize;
    let one = all.get(which).or_else(|| all.first())?;
    let label = one.get("label")?.as_str()?.to_string();
    let active = one
        .get("activeParameter")
        .or_else(|| result.get("activeParameter"))
        .and_then(|n| n.as_u64())
        .map(|n| n as usize);
    let span = active.and_then(|nth| {
        let it = one.get("parameters")?.as_array()?.get(nth)?.get("label")?;
        match it {
            // 一對 UTF-16 下標，直接是簽名裏的那一段。
            serde_json::Value::Array(pair) => {
                let from = pair.first()?.as_u64()? as usize;
                let to = pair.get(1)?.as_u64()? as usize;
                // 服務器數的是 UTF-16 碼元，這一頭數字符。
                Some((
                    crate::problem::char_column(&label, from),
                    crate::problem::char_column(&label, to),
                ))
            }
            // 一個名字：在簽名裏找它。找不到就當沒說——寧可不加重，也不許加重錯
            // 的那一段。
            serde_json::Value::String(name) => {
                let at = label.find(name.as_str())?;
                let from = label[..at].chars().count();
                Some((from, from + name.chars().count()))
            }
            _ => None,
        }
    });
    Some(Signature { label, active: span })
}

/// The empty answer to a request we do not really implement.
///
/// `id` 是**那條請求裏的 JSON 原樣**（`4` 或 `"abc"`），所以回去的號碼和來的那個
/// 逐字節相同——協議要的就是這個。
pub fn empty_answer(id: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"result":null}}"#)
}

/// Turn a `publishDiagnostics` notification into problems.
fn said(value: &serde_json::Value) -> Notice {
    let params = value.get("params");
    let Some(uri) = params.and_then(|p| p.get("uri")).and_then(|u| u.as_str()) else {
        return Notice::Nothing;
    };
    let Some(path) = path_of(uri) else {
        return Notice::Nothing;
    };
    let said = params
        .and_then(|p| p.get("diagnostics"))
        .and_then(|d| d.as_array())
        .map(|list| list.iter().filter_map(one).collect())
        .unwrap_or_default();
    Notice::Said { path, said }
}

/// One entry of the `diagnostics` array.
fn one(entry: &serde_json::Value) -> Option<Problem> {
    let start = entry.get("range")?.get("start")?;
    let message = entry.get("message")?.as_str()?.trim().to_string();
    // Warning: **A missing severity is not a hint, it is an error.** The
    // specification says so (「If omitted it is up to the client to interpret
    // ... as error」), and guessing the other way hides the one kind of
    // complaint a person must see.
    let severity = match entry.get("severity").and_then(|s| s.as_i64()) {
        Some(n) => Severity::from_lsp(n),
        None => Severity::Error,
    };
    Some(Problem {
        line: start.get("line")?.as_u64()? as usize,
        utf16_column: start.get("character").and_then(|c| c.as_u64()).unwrap_or(0) as usize,
        severity,
        message,
        source: entry.get("source").and_then(|s| s.as_str()).map(str::to_string),
    })
}

// ---- URIs -----------------------------------------------------------------

/// `file:///…` for a path.
///
/// Warning: **Only the characters a URI may carry go through unescaped.** A path
/// with a space or a 漢字 in it — and every manuscript in this editor's own
/// tests has 漢字 in it — is rejected by a strict server, and the symptom is a
/// file that simply never gets any diagnostics.
pub fn uri_of(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut out = String::from("file://");
    // Windows paths do not start with a separator; the URI must.
    if !text.starts_with('/') {
        out.push('/');
    }
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char)
            }
            // A Windows drive letter's colon is left alone; so is nothing else.
            b':' => out.push(':'),
            b'\\' => out.push('/'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The path a `file://` URI names, undoing the escaping above.
pub fn path_of(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // `file:///path` on unix, `file:///C:/path` on Windows — the authority is
    // empty either way, so what is left starts at the first `/`.
    let rest = rest.strip_prefix('/').map(|r| format!("/{r}")).unwrap_or_else(|| rest.to_string());
    let mut bytes = Vec::new();
    let mut chars = rest.bytes();
    while let Some(byte) = chars.next() {
        match byte {
            b'%' => {
                let hex: Vec<u8> = chars.by_ref().take(2).collect();
                let hex = std::str::from_utf8(&hex).ok()?;
                bytes.push(u8::from_str_radix(hex, 16).ok()?);
            }
            other => bytes.push(other),
        }
    }
    let text = String::from_utf8(bytes).ok()?;
    // `/C:/x` is a Windows path wearing a URI's leading slash.
    #[cfg(windows)]
    let text = match text.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => text[1..].to_string(),
        _ => text,
    };
    Some(PathBuf::from(text))
}

/// A string, as JSON writes one.
///
/// Hand-written rather than `serde_json::to_string`, because every call here
/// is building one field of a message whose shape is a literal — and a literal
/// is the clearest possible statement of what goes on the wire.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Warning: The control characters must be escaped or the JSON is
            // invalid — and a file with a stray `\u{1}` in it is a file the
            // server then never hears about.
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_framed_by_its_length_in_bytes() {
        let body = r#"{"a":"漢"}"#;
        let framed = frame(body);
        let text = String::from_utf8(framed.clone()).unwrap();
        assert!(text.starts_with(&format!("Content-Length: {}\r\n\r\n", body.len())));
        // Warning: 位元組，不是字：那個漢字佔三個。
        assert_eq!(body.len(), 11, "three bytes for 漢");
        // …and it reads back whole.
        let mut buf = framed;
        assert_eq!(take_frame(&mut buf).as_deref(), Some(body));
        assert!(buf.is_empty());
    }

    #[test]
    fn half_a_message_is_not_an_error_it_is_not_yet() {
        let body = r#"{"jsonrpc":"2.0"}"#;
        let whole = frame(body);
        let mut buf = whole[..whole.len() - 4].to_vec();
        assert_eq!(take_frame(&mut buf), None, "還没到齊");
        buf.extend_from_slice(&whole[whole.len() - 4..]);
        assert_eq!(take_frame(&mut buf).as_deref(), Some(body), "到齊了就讀得出");
    }

    /// Warning: `Content-Type` is in the specification. A reader that assumed the
    /// length came first would lose step here and never find it again.
    #[test]
    fn another_header_is_stepped_over() {
        let mut buf = b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\nContent-Length: 2\r\n\r\n{}".to_vec();
        assert_eq!(take_frame(&mut buf).as_deref(), Some("{}"));
    }

    #[test]
    fn two_messages_in_one_read_come_out_one_at_a_time() {
        let mut buf = frame("{\"a\":1}");
        buf.extend(frame("{\"b\":2}"));
        assert_eq!(take_frame(&mut buf).as_deref(), Some("{\"a\":1}"));
        assert_eq!(take_frame(&mut buf).as_deref(), Some("{\"b\":2}"));
        assert_eq!(take_frame(&mut buf), None);
    }

    #[test]
    fn a_published_list_becomes_problems() {
        let message = r#"{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{
            "uri":"file:///tmp/a.rs","diagnostics":[
              {"range":{"start":{"line":3,"character":8},"end":{"line":3,"character":9}},
               "severity":1,"source":"rustc","message":"cannot find value `x`"},
              {"range":{"start":{"line":9,"character":0},"end":{"line":9,"character":4}},
               "severity":2,"message":"unused"}
            ]}}"#;
        let Notice::Said { path, said } = read(message, 1) else {
            panic!("a publish is a publish");
        };
        assert_eq!(path, PathBuf::from("/tmp/a.rs"));
        assert_eq!(said.len(), 2);
        assert_eq!(said[0].line, 3);
        assert_eq!(said[0].utf16_column, 8);
        assert_eq!(said[0].severity, Severity::Error);
        assert_eq!(said[0].source.as_deref(), Some("rustc"));
        assert_eq!(said[1].severity, Severity::Warn);
    }

    /// Warning: **No severity means an error**, which is what the specification
    /// says. Reading it as the quietest kind would hide the loudest one.
    #[test]
    fn a_complaint_with_no_severity_is_an_error() {
        let message = r#"{"method":"textDocument/publishDiagnostics","params":{"uri":"file:///a",
            "diagnostics":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"message":"?"}]}}"#;
        let Notice::Said { said, .. } = read(message, 1) else { panic!() };
        assert_eq!(said[0].severity, Severity::Error);
    }

    #[test]
    fn an_empty_list_is_a_file_with_nothing_wrong_not_a_message_to_drop() {
        let message = r#"{"method":"textDocument/publishDiagnostics","params":{"uri":"file:///a","diagnostics":[]}}"#;
        assert_eq!(read(message, 1), Notice::Said { path: PathBuf::from("/a"), said: Vec::new() });
    }

    #[test]
    fn the_answer_to_initialize_is_the_handshake_and_a_server_request_is_not() {
        assert_eq!(read(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#, 1), Notice::Ready);
        // Warning: **Every other answer comes back with its id** (#53 ②), because
        // only the front end knows which id it sent for what. It used to be
        // dropped here as 「nobody's business」; now `gd` has business.
        assert_eq!(
            read(r#"{"jsonrpc":"2.0","id":7,"result":{}}"#, 1),
            Notice::Answer {
                id: 7,
                places: Vec::new(),
                told: None,
                offers: Vec::new(),
                signature: None,
            }
        );
        // A *request* from the server has a method as well as an id, and must
        // be answered or the server may wait on it forever.
        assert_eq!(
            read(r#"{"jsonrpc":"2.0","id":2,"method":"client/registerCapability"}"#, 1),
            Notice::Asked { id: "2".into() }
        );
        // Warning: **號碼也可以是字串**（協議兩種都許，2026-10-07 審出來的）。從前這一
        // 條讀成 `Nothing`：沒人答，而等自己註冊回音的服務器就停在那裏。回去的號
        // 碼要和來的那個逐字節相同，所以引號也帶着。
        assert_eq!(
            read(r#"{"jsonrpc":"2.0","id":"abc","method":"client/registerCapability"}"#, 1),
            Notice::Asked { id: "\"abc\"".into() }
        );
        assert_eq!(empty_answer("\"abc\""), r#"{"jsonrpc":"2.0","id":"abc","result":null}"#);
        // A plain notification wants nothing back.
        assert_eq!(read(r#"{"jsonrpc":"2.0","method":"$/progress"}"#, 1), Notice::Nothing);
    }

    #[test]
    fn nonsense_from_a_server_is_not_a_crash() {
        assert_eq!(read("not json at all", 1), Notice::Nothing);
        assert_eq!(read("", 1), Notice::Nothing);
        assert_eq!(read("[]", 1), Notice::Nothing);
    }

    /// **三種回答都合法，而服務器真的不一樣**（#53 ②）。
    ///
    /// 規範允許一個 `Location`、一串 `Location`、或者一串 `LocationLink`（它把
    /// 範圍叫 `targetSelectionRange`）。`rust-analyzer` 送的是第三種——只認一
    /// 種，能用到換服務器那天爲止。
    #[test]
    fn a_definition_answers_in_three_shapes_and_all_of_them_are_read() {
        let here = |m: &str| match read(m, 1) {
            Notice::Answer { places, .. } => places,
            other => panic!("是一條回答：{other:?}"),
        };
        let want = Place { path: PathBuf::from("/a.rs"), line: 3, utf16_column: 7 };

        // 一個 Location。
        let one = r#"{"id":2,"result":{"uri":"file:///a.rs","range":{"start":{"line":3,"character":7},"end":{"line":3,"character":9}}}}"#;
        assert_eq!(here(one), std::slice::from_ref(&want));

        // 一串 Location。
        let many = r#"{"id":2,"result":[{"uri":"file:///a.rs","range":{"start":{"line":3,"character":7},"end":{"line":3,"character":9}}}]}"#;
        assert_eq!(here(many), std::slice::from_ref(&want));

        // 一串 LocationLink——rust-analyzer 送的這一種。
        let links = r#"{"id":2,"result":[{"targetUri":"file:///a.rs","targetRange":{"start":{"line":1,"character":0},"end":{"line":9,"character":1}},"targetSelectionRange":{"start":{"line":3,"character":7},"end":{"line":3,"character":9}}}]}"#;
        assert_eq!(here(links), [want], "Warning: 取的是 targetSelectionRange，不是整段");

        // 說不出來的時候是 `null`，不是錯。
        assert_eq!(here(r#"{"id":2,"result":null}"#), []);
        assert_eq!(here(r#"{"id":2,"result":[]}"#), []);
    }

    /// **hover 的三種回答，以及 Markdown 是怎麽留下來的**（#53 ③）。
    ///
    /// `contents` 可以是 `MarkupContent`、一個裸字串、`{language, value}`，
    /// 或者它們的一串。圍欄變成**行內代碼**——浮窗畫的是行內標記，而簽名是整條
    /// 回答裏最有用的一行，要留住它的墨色。
    #[test]
    fn a_hover_answers_in_several_shapes_and_stays_markdown() {
        let told_of = |m: &str| match read(m, 1) {
            Notice::Answer { told, .. } => told,
            other => panic!("是一條回答：{other:?}"),
        };
        let said = |m: &str| told_of(m).map(|t| t.text);

        // rust-analyzer 送的這一種：MarkupContent，裏面是 Markdown。
        let ra = r#"{"id":3,"result":{"contents":{"kind":"markdown","value":"```rust\nfn counted(text: &str) -> usize\n```\n\n---\n\n**數**一數有幾個字。"}}}"#;
        assert_eq!(
            said(ra).as_deref(),
            Some("`fn counted(text: &str) -> usize`\n\n**數**一數有幾個字。"),
            "Warning: 圍欄變行內代碼，`---` 變一個空行，`**粗**` 原樣留着讓編輯器自己畫"
        );

        // 裸字串，以及舊的 {language, value}。
        assert_eq!(said(r#"{"id":3,"result":{"contents":"一句話"}}"#).as_deref(), Some("一句話"));
        assert_eq!(
            said(r#"{"id":3,"result":{"contents":{"language":"rust","value":"usize"}}}"#).as_deref(),
            Some("usize")
        );

        // 一串。
        assert_eq!(
            said(r#"{"id":3,"result":{"contents":["甲","乙"]}}"#).as_deref(),
            Some("甲\n\n乙")
        );

        // 無話可說是 `null`，不是錯——空的也算無話可說。
        assert_eq!(said(r#"{"id":3,"result":null}"#), None);
        assert_eq!(said(r#"{"id":3,"result":{"contents":{"kind":"markdown","value":""}}}"#), None);

        // Warning: **問定義的答案不會被讀成 hover，反過來也一樣。**
        let where_ = r#"{"id":2,"result":{"uri":"file:///a.rs","range":{"start":{"line":3,"character":7},"end":{"line":3,"character":9}}}}"#;
        assert_eq!(said(where_), None, "定義的答案裏沒有 contents");
        match read(ra, 1) {
            Notice::Answer { places, .. } => assert!(places.is_empty(), "hover 裏沒有地方"),
            other => panic!("{other:?}"),
        }
    }

    /// **`kind` 說純文本，就不能拿 Markdown 的墨去畫**（2026-09-30 報的：
    /// Python 的 docstring 裏 `Main API` 底下那一行 `======` 原樣畫了出來）。
    ///
    /// 兩件事：`Told::markdown` 要跟着服務器說的走，而純文本**不許過
    /// [`inline`]**——它會把一行三個減號吃掉，把 ASCII 畫的表格每一行包進反
    /// 引號。
    #[test]
    fn a_plaintext_answer_is_not_read_as_markdown() {
        let told_of = |m: &str| match read(m, 1) {
            Notice::Answer { told, .. } => told,
            other => panic!("是一條回答：{other:?}"),
        };

        // pylsp 在 `contentFormat` 把 plaintext 排在前頭時送的就是這一種：
        // 生的 reStructuredText docstring。
        let plain = r#"{"id":3,"result":{"contents":{"kind":"plaintext","value":"Main API\n========\nrun(...): 跑一條命令\n\n---\n\n末尾"}}}"#;
        let told = told_of(plain).expect("有東西");
        assert!(!told.markdown, "服務器說了是純文本");
        assert_eq!(
            told.text, "Main API\n========\nrun(...): 跑一條命令\n\n---\n\n末尾",
            "Warning: 一個字都不許改——`---` 還在，換行還在"
        );

        // 同一段話，`kind` 換成 markdown，就照 Markdown 收拾。
        let marked = plain.replace("plaintext", "markdown");
        let told = told_of(&marked).expect("有東西");
        assert!(told.markdown);
        assert!(!told.text.contains("---"), "`---` 變成了一個空行：{}", told.text);

        // 沒有 `kind` 的那幾種舊形狀一律算 Markdown。
        for shape in [
            r#"{"id":3,"result":{"contents":"一句話"}}"#,
            r#"{"id":3,"result":{"contents":{"language":"rust","value":"usize"}}}"#,
            r#"{"id":3,"result":{"contents":["甲","乙"]}}"#,
        ] {
            assert!(told_of(shape).expect(shape).markdown, "{shape}");
        }

        // 一串裏只要有一段是 Markdown，整條就按 Markdown 畫——混着的時候，
        // 少畫一段標記比把純文本畫錯更輕。
        let mixed = r#"{"id":3,"result":{"contents":[{"kind":"plaintext","value":"甲"},{"kind":"markdown","value":"**乙**"}]}}"#;
        assert!(told_of(mixed).expect("有東西").markdown);
        let all_plain = r#"{"id":3,"result":{"contents":[{"kind":"plaintext","value":"甲"},{"kind":"plaintext","value":"乙"}]}}"#;
        assert!(!told_of(all_plain).expect("有東西").markdown);
    }

    /// 圍欄裏本來就有反引號的那一行不加引號——加了就把那一段提前關掉了。
    #[test]
    fn a_backtick_inside_a_fence_is_left_alone() {
        let raw = "```rust\nlet x = `y`;\nfn f()\n```";
        assert_eq!(inline(raw), "let x = `y`;\n`fn f()`");
    }

    /// **片段不許照字面插進稿子裏**（2026-10-07 審出來的）。
    ///
    /// 這一頭報的是 `snippetSupport: false`，所以這是一道底線：對方不守規矩的時
    /// 候，進稿子的也得是字，不是 `${1:}`。
    #[test]
    fn a_snippet_has_its_holes_filled_in_before_it_is_inserted() {
        let offered = |m: &str| match read(m, 1) {
            Notice::Answer { offers, .. } => offers,
            other => panic!("是一條回答：{other:?}"),
        };
        let snippet = r#"{"id":4,"result":[{"label":"counted(…)","insertText":"counted(${1:text})$0","insertTextFormat":2}]}"#;
        assert_eq!(offered(snippet)[0].insert, "counted(text)");
        // 說是字面（1）就照字面——那一串裏的 `$` 是人家真要的字。
        let plain = r#"{"id":4,"result":[{"label":"x","insertText":"cost$1","insertTextFormat":1}]}"#;
        assert_eq!(offered(plain)[0].insert, "cost$1");
        // 一根光禿禿的序號沒有字；轉義脫一層；`|甲,乙|` 留頭一個。
        assert_eq!(without_the_holes("a${1}b"), "ab");
        assert_eq!(without_the_holes("\\$x"), "$x");
        assert_eq!(without_the_holes("${1|甲,乙|}"), "甲");
        assert_eq!(without_the_holes("${1:${2:裏}}"), "裏", "洞裏還有洞");
    }

    /// **簽名那一答的兩種參數寫法**（2026-10-08）。
    ///
    /// 協議說 `parameters[].label` 可以是一對 UTF-16 下標，也可以是一個名字
    /// ——rust-analyzer 給下標，pylsp 給名字，兩種都要認。
    #[test]
    fn a_signature_says_which_parameter_is_being_filled() {
        let answered = |m: &str| match read(m, 1) {
            Notice::Answer { signature, .. } => signature,
            other => panic!("是一條回答：{other:?}"),
        };
        // 下標那一種：`&mut self` 是第 0 個，`value: T` 是第 1 個。
        let by_index = r#"{"id":4,"result":{"signatures":[{"label":"fn push(&mut self, value: T)","parameters":[{"label":[8,17]},{"label":[19,27]}]}],"activeSignature":0,"activeParameter":1}}"#;
        let one = answered(by_index).expect("有一條");
        assert_eq!(one.label, "fn push(&mut self, value: T)");
        assert_eq!(one.active, Some((19, 27)));
        assert_eq!(&one.label[19..27], "value: T", "那一段正是第二個參數");

        // 名字那一種：在簽名裏找它。
        let by_name = r#"{"id":4,"result":{"signatures":[{"label":"def push(self, value)","parameters":[{"label":"self"},{"label":"value"}]}],"activeParameter":1}}"#;
        let one = answered(by_name).expect("有一條");
        assert_eq!(one.active, Some((15, 20)));
        assert_eq!(&one.label[15..20], "value");

        // Warning: **找不到那個名字就當沒說**——寧可不加重，也不許加重錯的那一段。
        let wrong = r#"{"id":4,"result":{"signatures":[{"label":"def push(self, value)","parameters":[{"label":"nowhere"}]}],"activeParameter":0}}"#;
        assert_eq!(answered(wrong).expect("有一條").active, None);

        // 一條都沒有：沒什麼可畫。
        assert_eq!(answered(r#"{"id":4,"result":{"signatures":[]}}"#), None);
        assert_eq!(answered(r#"{"id":4,"result":null}"#), None);
    }

    /// **補全的兩種回答，以及「顯示的」與「打進去的」不是同一個字串**（#53 ④）。
    #[test]
    fn a_completion_answers_in_two_shapes_and_label_is_not_insert() {
        let offered = |m: &str| match read(m, 1) {
            Notice::Answer { offers, .. } => offers,
            other => panic!("是一條回答：{other:?}"),
        };

        // 裸的一串。
        let bare = r#"{"id":4,"result":[{"label":"count()","insertText":"count","kind":2,"detail":"fn() -> usize"}]}"#;
        let got = offered(bare);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].label, "count()", "單子上看見的");
        assert_eq!(got[0].insert, "count", "Warning: 真打進檔案的是這個");
        assert_eq!(got[0].kind, 2);
        assert_eq!(got[0].detail.as_deref(), Some("fn() -> usize"));
        assert_eq!(got[0].replacing, None, "没給範圍就什麽都不替換");

        // CompletionList，帶 textEdit——rust-analyzer 送的這一種。
        let list = r#"{"id":4,"result":{"isIncomplete":false,"items":[{"label":"counted","kind":3,"textEdit":{"newText":"counted","range":{"start":{"line":6,"character":17},"end":{"line":6,"character":19}}}}]}}"#;
        let got = offered(list);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].insert, "counted");
        assert_eq!(
            got[0].replacing.map(|r| (r.from, r.to)),
            Some((17, 19)),
            "Warning: 替換掉已經打出來的那兩個碼元"
        );

        // InsertReplaceEdit：要的是 replace 那一段（「現在那個詞」）。
        let both = r#"{"id":4,"result":[{"label":"x","textEdit":{"newText":"x","insert":{"start":{"line":0,"character":4},"end":{"line":0,"character":5}},"replace":{"start":{"line":0,"character":4},"end":{"line":0,"character":9}}}}]}"#;
        assert_eq!(
            offered(both)[0].replacing.map(|r| (r.from, r.to)),
            Some((4, 9)),
            "replace，不是 insert"
        );

        // 没東西可提是 `null` 或空的，不是錯。
        assert!(offered(r#"{"id":4,"result":null}"#).is_empty());
        assert!(offered(r#"{"id":4,"result":{"items":[]}}"#).is_empty());

        // Warning: 三種回答互不誤讀。
        assert!(offered(r#"{"id":3,"result":{"contents":"一句話"}}"#).is_empty(), "hover 裏没有候選");
        match read(bare, 1) {
            Notice::Answer { places, told, .. } => {
                assert!(places.is_empty() && told.is_none(), "補全裏没有地方也没有說明")
            }
            other => panic!("{other:?}"),
        }
    }

    /// Warning: 這個編輯器自己的測試檔就叫「第一章.md」。A URI that does not escape
    /// them is a file that silently never gets an answer.
    #[test]
    fn a_path_with_chinese_in_it_survives_the_round_trip() {
        for path in ["/tmp/第一章.md", "/tmp/a b/c.rs", "/tmp/plain.rs", "/tmp/100%/x.go"] {
            let uri = uri_of(Path::new(path));
            assert!(uri.starts_with("file:///"), "{uri}");
            assert!(uri.is_ascii(), "a URI is ASCII or it is not a URI: {uri}");
            assert_eq!(path_of(&uri), Some(PathBuf::from(path)), "{uri}");
        }
    }

    #[test]
    fn what_we_send_is_json_and_says_what_it_means() {
        let text = "let x = \"引\";\n\t1\n";
        let open = did_open(Path::new("/tmp/a.rs"), "rust", 1, text);
        let parsed: serde_json::Value = serde_json::from_str(&open).expect("valid JSON");
        assert_eq!(parsed["method"], "textDocument/didOpen");
        assert_eq!(parsed["params"]["textDocument"]["languageId"], "rust");
        assert_eq!(parsed["params"]["textDocument"]["text"], text, "正文一字不差");
        let change = did_change(Path::new("/tmp/a.rs"), 2, text);
        let parsed: serde_json::Value = serde_json::from_str(&change).expect("valid JSON");
        assert_eq!(parsed["params"]["contentChanges"][0]["text"], text);
        assert_eq!(parsed["params"]["textDocument"]["version"], 2);
        // …and the rest are JSON too.
        for message in [initialize(1, Path::new("/tmp")), initialized(), shutdown(2), exit(), empty_answer("3")] {
            serde_json::from_str::<serde_json::Value>(&message).expect(&message);
        }
    }

    /// A control character in the file must not make the message unparseable.
    #[test]
    fn a_stray_control_character_is_escaped_rather_than_sent_raw() {
        let open = did_open(Path::new("/a.rs"), "rust", 1, "a\u{1}b");
        let parsed: serde_json::Value = serde_json::from_str(&open).expect("valid JSON");
        assert_eq!(parsed["params"]["textDocument"]["text"], "a\u{1}b");
    }

    /// **回退那一段：跳過圍欄與分隔線，只取第一段**（2026-10-08）。
    #[test]
    fn the_first_paragraph_of_a_hover_is_the_prose_not_the_signature() {
        // rust-analyzer 對 `println!` 的 hover，形狀是量出來的。
        let told = "```rust\nstd::macros\n```\n\n```rust\nmacro_rules! println\n```\n\n---\n\nPrints to the standard output, with a newline.\n\nOn all platforms, the newline is the LINE FEED character.\n";
        assert_eq!(
            first_paragraph(told).as_deref(),
            Some("Prints to the standard output, with a newline.\n…"),
            "一段，後面還有就單排一個 …"
        );
        // 最後一段：沒有 `…`。
        let one = "```rust\nfn f()\n```\n\n---\n\nDoes a thing.\n";
        assert_eq!(first_paragraph(one).as_deref(), Some("Does a thing."));
        // 一段可以是好幾行，空行才斷。
        let two = "---\n\nOne line\nand its continuation.\n\nNext paragraph.\n";
        assert_eq!(
            first_paragraph(two).as_deref(),
            Some("One line\nand its continuation.\n…")
        );
        // 只有簽名，沒有散文：什麼都不畫。
        assert_eq!(first_paragraph("```rust\nfn f()\n```\n"), None);

        // **真正遞進來的是過了 `inline` 的那一份**（2026-10-09）。上面那幾格餵的是
        // 服務器原話，而 `Told::text` 早被 `inline` 拆過圍欄了——這幾格走的是產線
        // 上那條路，不走一遍就看不見「跳圍欄」那道閘一次也沒開過。
        assert_eq!(
            first_paragraph(&inline(told)).as_deref(),
            Some("Prints to the standard output, with a newline.\n…"),
            "產線上的形狀：圍欄已經變成行內代碼"
        );
        assert_eq!(first_paragraph(&inline(one)).as_deref(), Some("Does a thing."));
        assert_eq!(first_paragraph(&inline("```rust\nfn f()\n```\n")), None);
        // 真散文裏的行內代碼不許被當成簽名跳掉。
        assert_eq!(
            first_paragraph("`甲` 和 `乙` 都是字。\n").as_deref(),
            Some("`甲` 和 `乙` 都是字。")
        );
    }
}
