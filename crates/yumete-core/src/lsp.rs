//! **Talking to a language server — the wire, not the process** (#53／#54,
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
//! ⚠️ **LSP counts characters in UTF-16 code units.** Not bytes, not
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
/// ⚠️ **The length is in bytes, not characters** — a message carrying a
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
/// ⚠️ **Other headers are allowed and must be skipped.** `Content-Type` is in
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
/// ⚠️ **`completionItem.snippetSupport` is claimed `false` on purpose**
/// (#53 ④). A snippet is not text — it is `counted(${1:text})`, a form with
/// holes in it, and an editor that does not fill the holes has to put those
/// six characters into the reader's file. Saying so is what makes
/// `rust-analyzer` send `counted` instead, which is exactly what this step can
/// insert. ⚠️ **Claiming it and then not expanding is the bug**, and it is a
/// quiet one: it only shows up on functions.
///
/// ⚠️ **`synchronization.didSave` is not a nicety.** A server that runs a real
/// compiler (rust-analyzer's `cargo check`) only re-runs it when the file is
/// saved, and a client that never claims to send saves is never sent one.
/// Without this line, an error stays on the screen after the line that caused
/// it is deleted — 「我把錯的行刪了，錯誤信息還在」（2026-09-21）。
pub fn initialize(id: i64, root: &Path) -> String {
    let root = uri_of(root);
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"initialize","params":{{"processId":{pid},"rootUri":{root},"capabilities":{{"textDocument":{{"publishDiagnostics":{{"relatedInformation":false}},"synchronization":{{"didSave":true}},"hover":{{"contentFormat":["plaintext","markdown"]}},"completion":{{"completionItem":{{"snippetSupport":false}}}}}}}}}}}}"#,
        pid = std::process::id(),
        root = json_string(&root),
    )
}

/// The `initialized` notification — 「go ahead」, sent once the answer to
/// [`initialize`] is in. ⚠️ A server that never gets it may never start work.
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
/// ⚠️ **The whole file, on purpose.** Incremental sync is a second model of
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
/// ⚠️ **The column is in UTF-16 code units**, like every position on this
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

/// `textDocument/completion` — 「接下來能打什麽？」 (#53 ④).
pub fn completion(id: i64, path: &Path, line: usize, utf16_column: usize) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"textDocument/completion","params":{{"textDocument":{{"uri":{uri}}},"position":{{"line":{line},"character":{utf16_column}}}}}}}"#,
        uri = json_string(&uri_of(path)),
    )
}

/// `textDocument/didSave` — 「這一份落盤了」。
///
/// ⚠️ **This is what re-runs the compiler.** rust-analyzer's own analysis
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
    /// ⚠️ In **UTF-16 code units**, as it arrived; see
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
    /// /create`, `client/registerCapability`). ⚠️ **An unanswered request can
    /// stall a server** — `rust-analyzer` waits on its own registration — so
    /// the id comes back out to be answered with an empty result.
    Asked { id: i64 },
    /// **The answer to something we asked**, with the id that says which.
    ///
    /// ⚠️ **Typed, not raw JSON.** Only the front end knows which id it sent
    /// for what, so the id comes back untouched — but the *shape* of the
    /// answer is this crate's business, and a `serde_json::Value` crossing the
    /// boundary would make it everybody's.
    /// ⚠️ **Both readings, because the reader cannot know which was asked.**
    /// An answer carries an id and no method — nothing else in it says whether
    /// it came back from `definition` or from `hover`. Only the front end knows
    /// what it sent that id for, so both readings are offered and it takes the
    /// one it is waiting for: a definition fills `places` and leaves `told`
    /// empty; a hover does the opposite; a server that answered `null` fills
    /// neither, which is 「nothing to say」 in both languages.
    Answer { id: i64, places: Vec<Place>, told: Option<String>, offers: Vec<Offer> },
    /// Anything else: progress, logs, an answer nobody is waiting for.
    Nothing,
}

/// **One thing the server says could come next** (#53 ④).
///
/// ⚠️ **`label` is what is shown and `insert` is what is typed, and they are
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
    /// **How much of what is already typed this replaces**, in UTF-16 code
    /// units on the line the caret is on.
    ///
    /// ⚠️ **The server decides this, not the editor.** `self.co` completing to
    /// `count` replaces `co` — three characters back from the caret, or two,
    /// or none, depending on what the server thinks the word is. Guessing it
    /// with the editor's own idea of a word is right for `co` and wrong for
    /// `a.b`, `#[der`, `'lifet` and every language whose words are not this
    /// language's words. `None` when the server sent no edit range, and then
    /// the caller replaces nothing.
    pub replacing: Option<(usize, usize)>,
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
    let id = value.get("id").and_then(|i| i.as_i64());
    match (method, id) {
        (None, Some(id)) if id == initialize_id => Notice::Ready,
        (Some("textDocument/publishDiagnostics"), _) => said(&value),
        // A request from the server: it has both a method *and* an id.
        (Some(_), Some(id)) => Notice::Asked { id },
        // An answer to something we sent: an id and no method.
        (None, Some(id)) => Notice::Answer {
            id,
            places: places(value.get("result")),
            told: told(value.get("result")),
            offers: offers(value.get("result")),
        },
        _ => Notice::Nothing,
    }
}

/// The places in a `textDocument/definition` answer.
///
/// ⚠️ **Three shapes, all of them legal.** The specification lets a server
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
/// ⚠️ **Three shapes again, and one of them is deprecated but still sent.**
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
fn told(result: Option<&serde_json::Value>) -> Option<String> {
    fn one(value: &serde_json::Value) -> Option<String> {
        match value {
            serde_json::Value::String(text) => Some(text.clone()),
            // Both `MarkupContent` and the old `{language, value}` keep the
            // text under `value`.
            _ => Some(value.get("value")?.as_str()?.to_string()),
        }
    }
    let contents = result?.get("contents")?;
    let raw = match contents {
        serde_json::Value::Array(many) => {
            let parts: Vec<String> = many.iter().filter_map(one).collect();
            match parts.is_empty() {
                true => return None,
                false => parts.join("\n\n"),
            }
        }
        value => one(value)?,
    };
    let flat = inline(&raw);
    match flat.is_empty() {
        true => None,
        false => Some(flat),
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
            // ⚠️ A backtick inside the code would close the span early; a line
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
/// ⚠️ **Two shapes**: a bare array of items, or a `CompletionList` with them
/// under `items` (and an `isIncomplete` this step does not use — it means
/// 「ask again as they type」, which is a refinement of *when* to ask).
///
/// The list is taken in the order the server sent it. ⚠️ **Not sorted here**:
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
    let replacing = edit.and_then(|(e, _)| {
        // A `TextEdit` has `range`; an `InsertReplaceEdit` has `insert` and
        // `replace` instead, and **replace is the one that means 「the word
        // that is there now」**.
        let range = e.get("range").or_else(|| e.get("replace")).or_else(|| e.get("insert"))?;
        let from = range.get("start")?.get("character")?.as_u64()? as usize;
        let to = range.get("end")?.get("character")?.as_u64()? as usize;
        Some((from, to))
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

/// The empty answer to a request we do not really implement.
pub fn empty_answer(id: i64) -> String {
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
    // ⚠️ **A missing severity is not a hint, it is an error.** The
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
/// ⚠️ **Only the characters a URI may carry go through unescaped.** A path
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
            // ⚠️ The control characters must be escaped or the JSON is
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
        // ⚠️ 位元組，不是字：那個漢字佔三個。
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

    /// ⚠️ `Content-Type` is in the specification. A reader that assumed the
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

    /// ⚠️ **No severity means an error**, which is what the specification
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
        // ⚠️ **Every other answer comes back with its id** (#53 ②), because
        // only the front end knows which id it sent for what. It used to be
        // dropped here as 「nobody's business」; now `gd` has business.
        assert_eq!(
            read(r#"{"jsonrpc":"2.0","id":7,"result":{}}"#, 1),
            Notice::Answer { id: 7, places: Vec::new(), told: None, offers: Vec::new() }
        );
        // A *request* from the server has a method as well as an id, and must
        // be answered or the server may wait on it forever.
        assert_eq!(
            read(r#"{"jsonrpc":"2.0","id":2,"method":"client/registerCapability"}"#, 1),
            Notice::Asked { id: 2 }
        );
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
        assert_eq!(here(one), [want.clone()]);

        // 一串 Location。
        let many = r#"{"id":2,"result":[{"uri":"file:///a.rs","range":{"start":{"line":3,"character":7},"end":{"line":3,"character":9}}}]}"#;
        assert_eq!(here(many), [want.clone()]);

        // 一串 LocationLink——rust-analyzer 送的這一種。
        let links = r#"{"id":2,"result":[{"targetUri":"file:///a.rs","targetRange":{"start":{"line":1,"character":0},"end":{"line":9,"character":1}},"targetSelectionRange":{"start":{"line":3,"character":7},"end":{"line":3,"character":9}}}]}"#;
        assert_eq!(here(links), [want], "⚠️ 取的是 targetSelectionRange，不是整段");

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
        let said = |m: &str| match read(m, 1) {
            Notice::Answer { told, .. } => told,
            other => panic!("是一條回答：{other:?}"),
        };

        // rust-analyzer 送的這一種：MarkupContent，裏面是 Markdown。
        let ra = r#"{"id":3,"result":{"contents":{"kind":"markdown","value":"```rust\nfn counted(text: &str) -> usize\n```\n\n---\n\n**數**一數有幾個字。"}}}"#;
        assert_eq!(
            said(ra).as_deref(),
            Some("`fn counted(text: &str) -> usize`\n\n**數**一數有幾個字。"),
            "⚠️ 圍欄變行內代碼，`---` 變一個空行，`**粗**` 原樣留着讓編輯器自己畫"
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

        // ⚠️ **問定義的答案不會被讀成 hover，反過來也一樣。**
        let where_ = r#"{"id":2,"result":{"uri":"file:///a.rs","range":{"start":{"line":3,"character":7},"end":{"line":3,"character":9}}}}"#;
        assert_eq!(said(where_), None, "定義的答案裏沒有 contents");
        match read(ra, 1) {
            Notice::Answer { places, .. } => assert!(places.is_empty(), "hover 裏沒有地方"),
            other => panic!("{other:?}"),
        }
    }

    /// 圍欄裏本來就有反引號的那一行不加引號——加了就把那一段提前關掉了。
    #[test]
    fn a_backtick_inside_a_fence_is_left_alone() {
        let raw = "```rust\nlet x = `y`;\nfn f()\n```";
        assert_eq!(inline(raw), "let x = `y`;\n`fn f()`");
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
        assert_eq!(got[0].insert, "count", "⚠️ 真打進檔案的是這個");
        assert_eq!(got[0].kind, 2);
        assert_eq!(got[0].detail.as_deref(), Some("fn() -> usize"));
        assert_eq!(got[0].replacing, None, "没給範圍就什麽都不替換");

        // CompletionList，帶 textEdit——rust-analyzer 送的這一種。
        let list = r#"{"id":4,"result":{"isIncomplete":false,"items":[{"label":"counted","kind":3,"textEdit":{"newText":"counted","range":{"start":{"line":6,"character":17},"end":{"line":6,"character":19}}}}]}}"#;
        let got = offered(list);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].insert, "counted");
        assert_eq!(got[0].replacing, Some((17, 19)), "⚠️ 替換掉已經打出來的那兩個碼元");

        // InsertReplaceEdit：要的是 replace 那一段（「現在那個詞」）。
        let both = r#"{"id":4,"result":[{"label":"x","textEdit":{"newText":"x","insert":{"start":{"line":0,"character":4},"end":{"line":0,"character":5}},"replace":{"start":{"line":0,"character":4},"end":{"line":0,"character":9}}}}]}"#;
        assert_eq!(offered(both)[0].replacing, Some((4, 9)), "replace，不是 insert");

        // 没東西可提是 `null` 或空的，不是錯。
        assert!(offered(r#"{"id":4,"result":null}"#).is_empty());
        assert!(offered(r#"{"id":4,"result":{"items":[]}}"#).is_empty());

        // ⚠️ 三種回答互不誤讀。
        assert!(offered(r#"{"id":3,"result":{"contents":"一句話"}}"#).is_empty(), "hover 裏没有候選");
        match read(bare, 1) {
            Notice::Answer { places, told, .. } => {
                assert!(places.is_empty() && told.is_none(), "補全裏没有地方也没有說明")
            }
            other => panic!("{other:?}"),
        }
    }

    /// ⚠️ 這個編輯器自己的測試檔就叫「第一章.md」。A URI that does not escape
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
        for message in [initialize(1, Path::new("/tmp")), initialized(), shutdown(2), exit(), empty_answer(3)] {
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
}
