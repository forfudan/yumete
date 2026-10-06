//! Code inside a fence, coloured by its own grammar (#420).
//!
//! **Only the fence is parsed.** A manuscript is never handed to tree-sitter:
//! its markup is line-local and cached per paragraph, and a parser that wants
//! the whole document would fight that. A fence is different — it is a closed
//! run of lines that says, on its first line, what language it is written in —
//! so it is parsed on its own, and the answer is handed back line by line in
//! the same [`Span`] shape every other colour on the page already travels in.
//!
//! **Seven grammars ship** (measured 2026-09-16, release + LTO): json, toml,
//! html, css, yaml, javascript and python, 1.34 MB between them. A fence in any
//! other language, or none, keeps the one colour a fence has always had.

use std::sync::OnceLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor, Tree};

use crate::markdown::{Kind, Span};

/// What a run of code is, as far as its colour goes.
///
/// Fewer than a grammar's own capture names on purpose — `@function.method`
/// and `@function.builtin` are one colour in every theme worth copying — and
/// **every character of a parsed fence is one of these**: what no capture
/// names is [`Token::Plain`], drawn in the page's own ink.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Token {
    /// A name nothing more is known about: a variable, an argument.
    Plain,
    /// `def`, `return`, `import`, `@media`.
    Keyword,
    /// `self`, `this` — a name the language gives you.
    Builtin,
    /// A function or method, defined or called.
    Function,
    /// A type, a class, a constructor.
    Type,
    /// A field, an object's key, a CSS property.
    Property,
    /// An HTML tag.
    Tag,
    /// An HTML attribute.
    Attribute,
    /// A string.
    String,
    /// An escape, and the `{…}` that interpolates into a string.
    Escape,
    /// A number, `true`, `None`, a named constant.
    Constant,
    /// A comment.
    Comment,
    /// `+`, `=`, `=>`.
    Operator,
    /// Brackets, commas, colons.
    Punctuation,
}

/// A language this build can parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Css,
    Go,
    Html,
    JavaScript,
    Json,
    Python,
    Rust,
    Toml,
    Yaml,
}

impl Language {
    /// Every language, for `:view-code`'s report and the tests.
    pub const ALL: [Language; 9] = [
        Language::Css,
        Language::Go,
        Language::Html,
        Language::JavaScript,
        Language::Json,
        Language::Python,
        Language::Rust,
        Language::Toml,
        Language::Yaml,
    ];

    /// The language a fence's info string names — ```` ```py ````,
    /// ```` ``` python title="x" ````, ```` ```{.yaml} ```` — with the common
    /// spellings each one goes by. `None` for anything this build cannot parse.
    pub fn from_info(info: &str) -> Option<Language> {
        let word: String = info
            .trim()
            .trim_start_matches(['{', '.'])
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+'))
            .collect::<String>()
            .to_ascii_lowercase();
        Some(match word.as_str() {
            "css" => Language::Css,
            "go" | "golang" => Language::Go,
            "html" | "htm" | "xhtml" => Language::Html,
            "javascript" | "js" | "mjs" | "cjs" | "jsx" | "node" => Language::JavaScript,
            "json" | "jsonc" | "json5" | "geojson" => Language::Json,
            "python" | "py" | "python3" | "py3" => Language::Python,
            "rust" | "rs" => Language::Rust,
            "toml" => Language::Toml,
            "yaml" | "yml" => Language::Yaml,
            _ => return None,
        })
    }

    /// The language a file's extension says it is in. Narrower than
    /// [`Self::from_info`]: a file called `node` is not JavaScript.
    pub fn from_extension(extension: &str) -> Option<Language> {
        Some(match extension {
            "css" => Language::Css,
            "go" => Language::Go,
            "html" | "htm" | "xhtml" => Language::Html,
            "js" | "mjs" | "cjs" | "jsx" => Language::JavaScript,
            "json" | "jsonc" | "json5" | "geojson" => Language::Json,
            "py" | "pyw" => Language::Python,
            "rs" => Language::Rust,
            "toml" => Language::Toml,
            "yaml" | "yml" => Language::Yaml,
            _ => return None,
        })
    }

    /// Its name, as a fence would spell it.
    pub fn name(self) -> &'static str {
        match self {
            Language::Css => "css",
            Language::Go => "go",
            Language::Html => "html",
            Language::JavaScript => "javascript",
            Language::Json => "json",
            Language::Python => "python",
            Language::Rust => "rust",
            Language::Toml => "toml",
            Language::Yaml => "yaml",
        }
    }

    fn grammar(self) -> tree_sitter::Language {
        match self {
            Language::Css => tree_sitter_css::LANGUAGE.into(),
            Language::Go => tree_sitter_go::LANGUAGE.into(),
            Language::Html => tree_sitter_html::LANGUAGE.into(),
            Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Language::Json => tree_sitter_json::LANGUAGE.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
            Language::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
            Language::Yaml => tree_sitter_yaml::LANGUAGE.into(),
        }
    }

    fn highlights(self) -> &'static str {
        match self {
            Language::Css => tree_sitter_css::HIGHLIGHTS_QUERY,
            Language::Go => tree_sitter_go::HIGHLIGHTS_QUERY,
            Language::Html => tree_sitter_html::HIGHLIGHTS_QUERY,
            Language::JavaScript => tree_sitter_javascript::HIGHLIGHT_QUERY,
            Language::Json => tree_sitter_json::HIGHLIGHTS_QUERY,
            Language::Python => tree_sitter_python::HIGHLIGHTS_QUERY,
            Language::Rust => tree_sitter_rust::HIGHLIGHTS_QUERY,
            Language::Toml => tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            Language::Yaml => tree_sitter_yaml::HIGHLIGHTS_QUERY,
        }
    }

    /// The compiled query and what each of its captures means. Compiled once
    /// per process: a query is parsed from its source text, and that is not
    /// something to do per frame.
    /// **語法樹裏那些「一個定義」的查詢**（`]f`／`mi f` 那一族，2026-10-06）。
    ///
    /// Warning: **用語法本身帶的 `TAGS_QUERY`，不抄 helix 的 `textobjects.scm`。**
    /// helix 把文本對象寫成自己 runtime 裏的查詢檔，而那些檔是 **MPL-2.0**，這個
    /// 倉是 Apache-2.0——照抄要先定授權怎麼辦。`tags.scm` 是**語法 crate 自己
    /// 帶的**（我們本來就依賴它，和 `HIGHLIGHTS_QUERY` 同一個來源、同一份授權），
    /// 而它捕獲的 `@definition.function`／`@definition.class` 包的正是整個定義，
    /// 名字另有 `@name`。九種語言裏七種帶，css 與 html 不帶。
    ///
    /// 它給不出的：參數、註釋、測試——那幾種只在 helix 的 textobjects 裏有。
    fn tags(self) -> Option<&'static str> {
        Some(match self {
            Language::Go => tree_sitter_go::TAGS_QUERY,
            Language::JavaScript => tree_sitter_javascript::TAGS_QUERY,
            Language::Python => tree_sitter_python::TAGS_QUERY,
            Language::Rust => tree_sitter_rust::TAGS_QUERY,
            // 這幾種沒有定義可言，語法 crate 也不帶 tags。
            Language::Css | Language::Html | Language::Json | Language::Toml | Language::Yaml => {
                return None
            }
        })
    }

    /// **我們自己寫的那份文本對象查詢**（2026-10-06 定：「自己写比较好，不要
    /// 抄」）。
    ///
    /// Warning: **不抄 helix 的 `textobjects.scm`**——那些檔是 MPL-2.0，這個倉是
    /// Apache-2.0（見 [`Language::tags`]）。自己寫還有一層好處：helix 那份要照顧
    /// 幾十種語言和它自己的鍵，我們只要這四種語言、兩種對象。
    ///
    /// ⚠️ **代價是上游改語法我們自己盯**：下面這些節點名是寫死的，`tree-sitter-*`
    /// 升一版把某個節點改了名，查詢就**靜悄悄地不匹配**——不報錯、不編譯失敗，
    /// `mi a` 就是沒反應。所以每一種語言各釘一格測試
    /// （`our_textobject_queries_still_match_these_grammars`）。
    ///
    /// 節點名是從語法自己的 `src/node-types.json` 裏讀出來的，不是記的。
    fn objects(self) -> Option<&'static str> {
        Some(match self {
            Language::Python => {
                "(parameters (_) @parameter)\n                 (lambda_parameters (_) @parameter)\n                 (comment) @comment\n"
            }
            Language::Rust => {
                "(parameters (_) @parameter)\n                 (closure_parameters (_) @parameter)\n                 (type_parameters (_) @parameter)\n                 (line_comment) @comment\n                 (block_comment) @comment\n"
            }
            Language::Go => {
                "(parameter_list (_) @parameter)\n                 (type_parameter_list (_) @parameter)\n                 (comment) @comment\n"
            }
            Language::JavaScript => {
                "(formal_parameters (_) @parameter)\n                 (comment) @comment\n"
            }
            Language::Css | Language::Html | Language::Json | Language::Toml | Language::Yaml => {
                return None
            }
        })
    }

    /// 編好的那份，連着每一格捕獲算哪一種對象。
    fn object_query(self) -> Option<&'static Objects> {
        static CELLS: [OnceLock<Option<Objects>>; 9] = [const { OnceLock::new() }; 9];
        let at = Language::ALL.iter().position(|&l| l == self)?;
        CELLS[at]
            .get_or_init(|| {
                let query = Query::new(&self.grammar(), self.objects()?).ok()?;
                let kinds = query
                    .capture_names()
                    .iter()
                    .map(|n| match *n {
                        "parameter" => Some(Object::Parameter),
                        "comment" => Some(Object::Comment),
                        _ => None,
                    })
                    .collect();
                Some((query, kinds))
            })
            .as_ref()
    }

    /// 編好的 tags 查詢，連着每一格捕獲算哪一種定義。
    fn defines(self) -> Option<&'static Defines> {
        static CELLS: [OnceLock<Option<Defines>>; 9] = [const { OnceLock::new() }; 9];
        let at = Language::ALL.iter().position(|&l| l == self)?;
        CELLS[at]
            .get_or_init(|| {
                let query = Query::new(&self.grammar(), self.tags()?).ok()?;
                let kinds = query.capture_names().iter().map(|n| define_of(n)).collect();
                Some((query, kinds))
            })
            .as_ref()
    }

    fn query(self) -> Option<&'static Compiled> {
        static CELLS: [OnceLock<Option<Compiled>>; 9] = [const { OnceLock::new() }; 9];
        let at = Language::ALL.iter().position(|&l| l == self)?;
        CELLS[at]
            .get_or_init(|| {
                let query = Query::new(&self.grammar(), self.highlights()).ok()?;
                let tokens = query.capture_names().iter().map(|n| token_of(n)).collect();
                Some((query, tokens))
            })
            .as_ref()
    }
}

/// **一個定義是哪一種**（`]f`／`]c`，2026-10-06）。
///
/// `tags.scm` 的捕獲名分得比這細（`definition.method`、`definition.interface`、
/// `definition.module`…）；這裏只收兩種，因為鍵只有兩個，而「方法」在讀稿子的人
/// 眼裏就是一個函數。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Define {
    /// 函數、方法、巨集——按 `]f` 走的。
    Function,
    /// 類、結構、枚舉、介面、模組、類型——按 `]c` 走的。
    Class,
}

fn define_of(capture: &str) -> Option<Define> {
    match capture {
        "definition.function" | "definition.method" | "definition.macro" => Some(Define::Function),
        "definition.class" | "definition.interface" | "definition.module" | "definition.type" => {
            Some(Define::Class)
        }
        _ => None,
    }
}

/// **語法樹認得的那兩種小東西**（`mi a`／`mi c`，2026-10-06）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Object {
    /// 一個參數（`mi a`）。
    Parameter,
    /// 一段註釋（`mi c`）。
    Comment,
}

/// **這份正文裏每一個 `want`**，按起點排好，位置是**字節**。
pub fn objects(
    language: Language,
    source: &str,
    tree: &Tree,
    want: Object,
) -> Vec<(usize, usize)> {
    let Some((query, kinds)) = language.object_query() else {
        return Vec::new();
    };
    let mut cursor = QueryCursor::new();
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
    while let Some(one) = streaming_iterator::StreamingIterator::next(&mut matches) {
        for capture in one.captures {
            if kinds.get(capture.index as usize).copied().flatten() != Some(want) {
                continue;
            }
            out.push((capture.node.start_byte(), capture.node.end_byte()));
        }
    }
    out.sort_by_key(|&(from, to)| (from, std::cmp::Reverse(to)));
    out.dedup();
    out
}

/// **這份正文裏每一個定義**，按起點排好，位置是**字節**。
///
/// 同一個節點可能被兩條規則捕獲（rust 的 `function_item` 既在 `declaration_list`
/// 裏又在頂層），所以去重。
pub fn definitions(language: Language, source: &str, tree: &Tree) -> Vec<(usize, usize, Define)> {
    let Some((query, kinds)) = language.defines() else {
        return Vec::new();
    };
    let mut cursor = QueryCursor::new();
    let mut out: Vec<(usize, usize, Define)> = Vec::new();
    let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
    while let Some(one) = streaming_iterator::StreamingIterator::next(&mut matches) {
        for capture in one.captures {
            let Some(kind) = kinds.get(capture.index as usize).copied().flatten() else {
                continue;
            };
            let node = capture.node;
            out.push((node.start_byte(), node.end_byte(), kind));
        }
    }
    out.sort_by_key(|&(from, to, _)| (from, std::cmp::Reverse(to)));
    out.dedup_by_key(|&mut (from, to, _)| (from, to));
    out
}

/// What a capture does to the characters it covers: `None` leaves them to
/// whatever else names them.
type Paint = Option<Token>;

/// 一套語法編好的查詢，連着它每一格捕獲上什麼色——按捕獲的次序。
///
/// 兩者是一個答案：捕獲是靠它在查詢裏的下標認的，換一份查詢配同一張
/// `Vec<Paint>`，上的就是別人的色。
type Compiled = (Query, Vec<Paint>);

/// 同一件事的 tags 那一份：編好的查詢，連着每一格捕獲算哪一種定義。
type Defines = (Query, Vec<Option<Define>>);

/// 我們自己那份文本對象查詢，同一個形狀。
type Objects = (Query, Vec<Option<Object>>);

/// What a grammar's capture name is, in this page's colours.
fn token_of(capture: &str) -> Paint {
    let mut parts = capture.split('.');
    let head = parts.next().unwrap_or(capture);
    let second = parts.next().unwrap_or("");
    Some(match (head, second) {
        ("comment", _) => Token::Comment,
        // A JSON object's key is `@string.special.key`: a name, not a value.
        ("string", "special") if capture.ends_with(".key") => Token::Property,
        ("string", _) => Token::String,
        ("escape", _) => Token::Escape,
        ("keyword" | "import" | "media" | "charset" | "keyframes" | "namespace" | "supports", _) => {
            Token::Keyword
        }
        ("function", _) => Token::Function,
        ("type" | "constructor" | "label", _) => Token::Type,
        ("tag", _) => Token::Tag,
        ("attribute", _) => Token::Attribute,
        ("property", _) => Token::Property,
        ("number" | "boolean" | "constant", _) => Token::Constant,
        ("variable", "builtin") => Token::Builtin,
        ("variable" | "embedded", _) => Token::Plain,
        ("operator", _) => Token::Operator,
        // Python's f-string braces, JavaScript's `${`: where a string stops
        // being a string, which is what an escape says too.
        ("punctuation", "special") => Token::Escape,
        ("punctuation", _) => Token::Punctuation,
        _ => return None,
    })
}

/// The coloured runs of each line of `lines`, parsed as `language` — one entry
/// per line, in char indices, in order and non-overlapping.
pub fn highlight(language: Language, lines: &[String]) -> Vec<Vec<Span>> {
    let source = joined(lines);
    match parse(language, &source, None) {
        Some(tree) => paint(language, &source, &tree, lines, 0..lines.len()),
        None => vec![Vec::new(); lines.len()],
    }
}

/// 一行一行接成一份源碼，每行帶一個換行——`parse` 與 `paint` 讀的是同一份。
pub fn joined(lines: &[String]) -> String {
    let mut source = String::with_capacity(lines.iter().map(|l| l.len() + 1).sum());
    for line in lines {
        source.push_str(line);
        source.push('\n');
    }
    source
}

/// **上一份源碼和這一份差在哪**——回一個 `Tree::edit` 吃得下的改動（#423）。
///
/// Warning: **不必讓編輯器交出「改了什麽」。** 掐頭去尾就看得出來：從前面數到
/// 第一個不同的字節，從後面數到第一個不同的字節，中間那一段就是改動。打一個字
/// 是一次 970 KB 的 memcmp，十分之一毫秒；而換來的是增量解析（0.7 毫秒對 50 毫
/// 秒）。
///
/// Warning: **多光標、粘貼也對。** 幾處一起改的話，掐頭去尾框出來的是**把它們
/// 全包住的那一段**——重解析的範圍大一點，答案一樣對。
///
/// `None` ＝ 兩份一模一樣。
pub fn what_changed(was: &str, now: &str) -> Option<tree_sitter::InputEdit> {
    if was == now {
        return None;
    }
    let (a, b) = (was.as_bytes(), now.as_bytes());
    let mut head = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    // Warning: **數的是字節，而一個字的中間也可以對得上**（2026-10-06 按 `d`
    // 删到「名」當場崩的那一下）。「的名字」删成「的字」：名 是 `E5 90 8D`、
    // 字 是 `E5 AD 97`，兩個都以 `E5` 開頭，於是前綴數到 4 個字節——正落在那個
    // 字裏面。下面 `point` 的 `&text[..byte]` 當場 panic，而就算不 panic，
    // 交給 tree-sitter 的也是一個不存在的位置。
    //
    // **退到字的邊界上**。前綴那一段兩邊字節相同，所以邊界也相同；退一步只會讓
    // 重解析的範圍**大**一點，而上面那一段說過，大一點的答案一樣對。
    while head > 0 && !(was.is_char_boundary(head) && now.is_char_boundary(head)) {
        head -= 1;
    }
    // 從後面數，兩頭不許越過已經對上的那一段。
    let most = a.len().min(b.len()) - head;
    let mut tail = (0..most)
        .take_while(|i| a[a.len() - 1 - i] == b[b.len() - 1 - i])
        .count();
    // 尾巴同理，而這一頭兩邊的位置不同，所以兩邊都要問。
    while tail > 0
        && !(was.is_char_boundary(a.len() - tail) && now.is_char_boundary(b.len() - tail))
    {
        tail -= 1;
    }
    let point = |text: &str, byte: usize| {
        let upto = &text[..byte];
        let row = upto.matches('\n').count();
        let column = byte - upto.rfind('\n').map_or(0, |at| at + 1);
        tree_sitter::Point::new(row, column)
    };
    let start = head;
    let old_end = a.len() - tail;
    let new_end = b.len() - tail;
    Some(tree_sitter::InputEdit {
        start_byte: start,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position: point(was, start),
        old_end_position: point(was, old_end),
        new_end_position: point(now, new_end),
    })
}

/// **解析一份源碼，樹交出去**（#423，2026-09-30）。
///
/// `was` 是上一棵樹（`Tree::edit` 過的）：給了它就走增量，一個字的改動七百多微
/// 秒，從頭解析要五十毫秒。
///
/// Warning: **樹要有人存着。** 從前 `highlight` 每叫一次解析一次，於是滾一屏也
/// 付一次全份解析——量過 20,368 行的 `lib.rs` 是 50 毫秒。
pub fn parse(language: Language, source: &str, was: Option<&Tree>) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&language.grammar()).ok()?;
    parser.parse(source, was)
}

/// **只給 `rows` 那幾行上色**（#423）。
///
/// 樹已經在手上，所以這一支只做查詢，而查詢只掃那幾行覆蓋的字節。量過同一個
/// `lib.rs`：整棵樹查一遍 25 毫秒，只查五十行 0.06 毫秒。
///
/// Warning: **跨過窗口邊緣的那些捕獲照樣算。** 一條橫跨半個檔的塊註釋、一個三
/// 引號字串，起點在窗口上面、終點在窗口下面——`set_byte_range` 收的是**與這一
/// 段相交**的節點，不是「整個裝在裏面」的。這一條有測試釘着
/// （`a_window_paints_exactly_what_the_whole_file_would`）。
pub fn paint(
    language: Language,
    source: &str,
    tree: &Tree,
    lines: &[String],
    rows: std::ops::Range<usize>,
) -> Vec<Vec<Span>> {
    let rows = rows.start.min(lines.len())..rows.end.min(lines.len());
    let mut out = vec![Vec::new(); rows.len()];
    let Some((query, tokens)) = language.query() else {
        return out;
    };
    let mut starts = Vec::with_capacity(lines.len());
    let mut at = 0usize;
    for line in lines {
        starts.push(at);
        at += line.len() + 1;
    }
    if rows.is_empty() {
        return out;
    }
    let from = starts[rows.start];
    let to = starts[rows.end - 1] + lines[rows.end - 1].len();

    // (start, end, pattern, token) in bytes of `source`.
    let mut found: Vec<(usize, usize, usize, Token)> = Vec::new();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(from..to);
    let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
    while let Some(m) = matches.next() {
        for capture in m.captures {
            if let Some(Some(token)) = tokens.get(capture.index as usize) {
                let node = capture.node;
                found.push((node.start_byte(), node.end_byte(), m.pattern_index, *token));
            }
        }
    }
    paint_the_rows(found, lines, &starts, rows, &mut out);
    out
}

fn paint_the_rows(
    mut found: Vec<(usize, usize, usize, Token)>,
    lines: &[String],
    starts: &[usize],
    rows: std::ops::Range<usize>,
    out: &mut [Vec<Span>],
) {
    // Warning: **One exception: a name beats a string on the same node.** JSON's
    // query is the one written the other way round — `@string.special.key`
    // above `(string) @string` — so a key came out the green of its value.
    let named: std::collections::HashSet<(usize, usize)> = found
        .iter()
        .filter(|f| f.3 == Token::Property)
        .map(|f| (f.0, f.1))
        .collect();
    found.retain(|f| !(f.3 == Token::String && named.contains(&(f.0, f.1))));
    found.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));

    // Warning: **A window over the captures, not a pass over them** (2026-09-20).
    // This used to walk the whole of `found` for every line and skip what did
    // not overlap — O(lines × captures), and a capture per few bytes means
    // both grow together: measured 600 lines 10 ms, 2400 lines 20 ms, **4800
    // lines 60 ms**, which is where 「the colours just stop」 came from (the
    // 5000-line cap in `fences.rs` was the bandage).
    //
    // `found` is sorted by where a capture starts and the lines are walked in
    // order, so one index is enough: everything that can open on this line has
    // start < `to`, and what is finished (end ≤ `from`) comes out. `live`
    // keeps `found`'s order, which **is** the painting order.
    // Warning: **跨過窗口上緣的那幾個要先收進來**，一條橫跨半個檔的塊註釋就是
    // 那一種。`found` 按起點排過，而查詢只交回**與這一窗相交**的捕獲，所以這
    // 一趟只走 `found`，不走檔子的前半。
    //
    // Warning: **別改回「從第 0 行掃起」**（2026-09-30 量出來的）。那樣走的是
    // `O(窗口在第幾行)`：同一份 `lib.rs`，第 9,000 行那一塊 2.7 毫秒，第
    // 15,000 行那一塊 6.4 毫秒——愈往下愈慢，而那正是讀長檔的人待的地方。
    let mut next = 0usize;
    let mut live: Vec<(usize, usize, Token)> = Vec::new();
    let head = starts[rows.start];
    while next < found.len() && found[next].0 < head {
        let (start, end, _, token) = found[next];
        if end > head {
            live.push((start, end, token));
        }
        next += 1;
    }
    for n in rows.clone() {
        let line = &lines[n];
        let from = starts[n];
        let to = from + line.len();
        while next < found.len() && found[next].0 < to {
            let (start, end, _, token) = found[next];
            if end > from {
                live.push((start, end, token));
            }
            next += 1;
        }
        live.retain(|&(_, end, _)| end > from);
        // One token per byte of this line, painted in the order above.
        let mut paint: Vec<Token> = vec![Token::Plain; line.len()];
        for &(start, end, token) in &live {
            if start >= to {
                continue;
            }
            for slot in &mut paint[start.max(from) - from..end.min(to) - from] {
                *slot = token;
            }
        }
        // Bytes to runs of chars.
        let spans = &mut out[n - rows.start];
        let mut run: Option<(usize, Token)> = None;
        let mut index = 0;
        for (byte, _) in line.char_indices() {
            let here = paint[byte];
            match (run, here) {
                (Some((_, open)), token) if open == token => {}
                _ => {
                    if let Some((start, token)) = run.take() {
                        spans.push(span(start, index, token));
                    }
                    run = Some((index, here));
                }
            }
            index += 1;
        }
        if let Some((start, token)) = run {
            spans.push(span(start, index, token));
        }
    }
}

fn span(start: usize, end: usize, token: Token) -> Span {
    Span {
        start,
        end,
        kind: Kind::Token(token),
        // 代碼裏沒有嵌套的標記，全在頂層。
        depth: 0,
        // Every run its own construct: none of this is markup, so none of it
        // is ever hidden or opened together.
        construct: usize::MAX - start,
    }
}

#[cfg(test)]
mod tests {

    /// **自己寫的那份查詢，每一種語言各釘一格**（2026-10-06）。
    ///
    /// ⚠️ 這一格就是「自己寫不抄 helix」的全部代價。查詢裏的節點名是寫死的，
    /// `tree-sitter-*` 升一版把某個節點改了名，查詢**靜悄悄地不匹配**——不報錯、
    /// 不編譯失敗，`mi a` 就是沒反應。所以這裏逐種語言餵一段真代碼，數它找到
    /// 幾個。**看見這一格紅，先去看那一版的 `src/node-types.json`。**
    #[test]
    fn our_textobject_queries_still_match_these_grammars() {
        let cases = [
            (
                Language::Python,
                "# 一句註釋\ndef f(a, b=2):\n    return a\n",
                2,
                1,
            ),
            (
                Language::Rust,
                "// 一句註釋\nfn f(a: u8, b: u8) -> u8 { a + b }\n",
                2,
                1,
            ),
            (
                Language::Go,
                "// 一句註釋\nfunc f(a int, b int) int { return a }\n",
                2,
                1,
            ),
            (
                Language::JavaScript,
                "// 一句註釋\nfunction f(a, b) { return a; }\n",
                2,
                1,
            ),
        ];
        for (language, source, parameters, comments) in cases {
            let tree = parse(language, source, None).expect("parses");
            assert_eq!(
                objects(language, source, &tree, Object::Parameter).len(),
                parameters,
                "{language:?} 的參數"
            );
            assert_eq!(
                objects(language, source, &tree, Object::Comment).len(),
                comments,
                "{language:?} 的註釋"
            );
            // 定義那一份走的是語法自己帶的 tags，同樣會被上游改動影響。
            assert_eq!(
                definitions(language, source, &tree)
                    .iter()
                    .filter(|&&(_, _, k)| k == Define::Function)
                    .count(),
                1,
                "{language:?} 的函數"
            );
        }
    }
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    /// The token covering char `at` of `line`, if any.
    fn at(spans: &[Vec<Span>], line: usize, at: usize) -> Option<Token> {
        spans[line].iter().find(|s| s.start <= at && at < s.end).map(|s| match s.kind {
            Kind::Token(t) => t,
            _ => unreachable!(),
        })
    }

    #[test]
    fn every_shipped_grammar_compiles_its_own_query() {
        for language in Language::ALL {
            assert!(language.query().is_some(), "{}", language.name());
        }
    }

    #[test]
    fn a_fence_says_its_language_in_any_of_the_usual_spellings() {
        for (info, want) in [
            ("python", Some(Language::Python)),
            ("py title=\"a.py\"", Some(Language::Python)),
            (" JS", Some(Language::JavaScript)),
            ("{.yaml}", Some(Language::Yaml)),
            ("yml", Some(Language::Yaml)),
            ("jsonc", Some(Language::Json)),
            ("htm", Some(Language::Html)),
            // 2026-09-19 加的兩種（朋友寫 go 和 rust）。`rust` 從前正是這裏
            // 「這個 build 認不出來」的例子——現在換成一種真的認不出來的。
            ("rust", Some(Language::Rust)),
            ("rs", Some(Language::Rust)),
            ("golang", Some(Language::Go)),
            ("haskell", None),
            ("", None),
        ] {
            assert_eq!(Language::from_info(info), want, "{info:?}");
        }
    }

    #[test]
    fn python_comes_out_in_its_parts() {
        let got = highlight(
            Language::Python,
            &lines("def greet(name):\n    # 打個招呼\n    return f\"你好，{name}\" + 1"),
        );
        assert_eq!(at(&got, 0, 0), Some(Token::Keyword), "{:?}", got[0]);
        assert_eq!(at(&got, 0, 4), Some(Token::Function), "{:?}", got[0]);
        // An argument is a name like any other, in the page's own ink.
        assert_eq!(at(&got, 0, 10), Some(Token::Plain), "{:?}", got[0]);
        assert_eq!(at(&got, 1, 6), Some(Token::Comment), "{:?}", got[1]);
        assert_eq!(at(&got, 2, 4), Some(Token::Keyword), "{:?}", got[2]);
        // In chars, not bytes: 你 is three bytes and one column of the line.
        assert_eq!(at(&got, 2, 13), Some(Token::String), "{:?}", got[2]);
        let one = got[2].last().unwrap();
        assert_eq!((one.start, one.kind), (26, Kind::Token(Token::Constant)), "{:?}", got[2]);
    }

    #[test]
    fn a_json_key_is_a_name_and_its_value_is_a_string() {
        let got = highlight(Language::Json, &lines("{\"名\": \"值\", \"n\": 3}"));
        assert_eq!(at(&got, 0, 1), Some(Token::Property), "{:?}", got[0]);
        assert_eq!(at(&got, 0, 6), Some(Token::String), "{:?}", got[0]);
        assert_eq!(at(&got, 0, 16), Some(Token::Constant), "{:?}", got[0]);
    }

    #[test]
    fn a_yaml_key_is_a_name_too() {
        let got = highlight(Language::Yaml, &lines("title: 宇夢\nn: 3"));
        assert_eq!(at(&got, 0, 0), Some(Token::Property), "{:?}", got[0]);
        assert_eq!(at(&got, 0, 7), Some(Token::String), "{:?}", got[0]);
    }

    #[test]
    fn broken_code_still_colours_what_it_can() {
        let got = highlight(Language::Python, &lines("def (\n# 註\nreturn"));
        assert_eq!(at(&got, 1, 0), Some(Token::Comment), "{:?}", got);
    }
    /// **掐頭去尾不許切進一個字裏**（2026-10-06，按 `d` 删到「名」當場崩的
    /// 那一下，`code.rs:451` 的 `&text[..byte]`）。
    ///
    /// 漢字在 UTF-8 裏三個字節，開頭常常一樣：名 `E5 90 8D`、字 `E5 AD 97`。
    /// 所以「相同的字節數」會停在一個字的中間，而那不是一個位置。
    #[test]
    fn an_edit_never_starts_or_ends_inside_a_character() {
        // 那一下：「的名字」删成「的字」，前綴數到 4 個字節。
        let edit = what_changed("的名字", "的字").expect("they differ");
        assert!("的名字".is_char_boundary(edit.start_byte), "start");
        assert!("的名字".is_char_boundary(edit.old_end_byte), "old end");
        assert!("的字".is_char_boundary(edit.new_end_byte), "new end");

        // 同一族：尾巴那一頭也會落在字裏（髮 `E9 AB BC`、髫 `E9 AB AB`）。
        let edit = what_changed("一髮絲", "一髫絲").expect("they differ");
        assert!("一髮絲".is_char_boundary(edit.start_byte));
        assert!("一髮絲".is_char_boundary(edit.old_end_byte));
        assert!("一髫絲".is_char_boundary(edit.new_end_byte));

        // 整份稿子逐字删一遍，一次都不許切錯——這是按住 `d` 做的事。
        let text = "（2026-09-29 定的名字）違反了約定，所以删掉。\n下一行。\n";
        let chars: Vec<char> = text.chars().collect();
        for n in 0..chars.len() {
            let now: String =
                chars.iter().enumerate().filter(|(i, _)| *i != n).map(|(_, c)| c).collect();
            let Some(edit) = what_changed(text, &now) else { continue };
            assert!(text.is_char_boundary(edit.start_byte), "start at {n}");
            assert!(text.is_char_boundary(edit.old_end_byte), "old end at {n}");
            assert!(now.is_char_boundary(edit.new_end_byte), "new end at {n}");
        }
    }
}

#[cfg(test)]
mod how_long_does_it_take {
    use super::*;
    use std::time::Instant;

    /// **增量解析出來的樹，要和從頭解析的畫出一樣的顏色**（#423）。
    ///
    /// Warning: **釘的是 `what_changed` 框得準不準。** 框錯一個字節，tree-sitter
    /// 會拿錯的舊節點去對新文本，顏色從那裏起全歪——而那種錯**只在改過之後纔
    /// 出現**，開檔看是好的。所以這裏逐種改法試一遍：插一個字、刪一段、換一段、
    /// 在頭上、在尾上、在中文上。
    #[test]
    fn an_edited_tree_paints_what_a_fresh_one_would() {
        let before = "\
fn one() {}
/* 塊註釋
   第二行 */
fn two() -> &'static str { \"一個字串\" }
struct 三 { 甲: u8 }
";
        let edits: &[(&str, &str, &str)] = &[
            ("插一個字", "fn one()", "fn onex()"),
            ("插在頭上", "fn one() {}", "use std::fmt;\nfn one() {}"),
            ("插在尾上", "struct 三 { 甲: u8 }\n", "struct 三 { 甲: u8 }\nfn four() {}\n"),
            ("刪一段", "/* 塊註釋\n   第二行 */\n", ""),
            ("換一段", "\"一個字串\"", "\"換了的字串\""),
            ("動中文", "甲: u8", "乙丙丁: u8"),
            ("把註釋拆開", "/* 塊註釋", "/ * 塊註釋"),
            ("整個清空", "fn one() {}\n", ""),
        ];
        for (what, from, to) in edits {
            let after = before.replacen(from, to, 1);
            assert_ne!(after, before, "{what}：這一改沒改動任何東西");

            let tree = parse(Language::Rust, before, None).expect("解析得了");
            let edit = what_changed(before, &after).expect("看得出改了");
            let mut edited = tree.clone();
            edited.edit(&edit);
            let again =
                parse(Language::Rust, &after, Some(&edited)).expect("增量也解析得了");

            let lines: Vec<String> = after.lines().map(str::to_string).collect();
            let source = joined(&lines);
            let fresh = parse(Language::Rust, &source, None).expect("從頭也解析得了");
            let rows = 0..lines.len();
            assert_eq!(
                paint(Language::Rust, &source, &again, &lines, rows.clone()),
                paint(Language::Rust, &source, &fresh, &lines, rows),
                "{what}：增量解析畫出來的和從頭解析的不一樣"
            );
        }
    }

    /// 一模一樣的兩份，`what_changed` 要說「沒改」——不然每一幀白解析一次。
    #[test]
    fn nothing_changed_is_nothing_to_do() {
        assert!(what_changed("abc", "abc").is_none());
        let one = what_changed("abc", "abXc").expect("看得出");
        assert_eq!((one.start_byte, one.old_end_byte, one.new_end_byte), (2, 2, 3));
        let two = what_changed("abXc", "abc").expect("看得出");
        assert_eq!((two.start_byte, two.old_end_byte, two.new_end_byte), (2, 3, 2));
    }

    /// **一窗畫出來的，要和整份畫出來的那幾行逐字節相同**（#423）。
    ///
    /// Warning: **這一條釘的是「跨過邊緣的捕獲會不會掉」。** 一條橫跨半個檔的
    /// 塊註釋、一個三引號字串，起點在窗口上面、終點在窗口下面——`set_byte_range`
    /// 要是只收「整個裝在裏面」的節點，那幾行就會少掉顏色，而那種錯**只在滾到
    /// 某一屏的時候纔看得見**，逐行對照纔抓得住。
    ///
    /// 所以這裏拿一份真有那幾種東西的源碼，**每一個窗口都對一遍**。
    #[test]
    fn a_window_paints_exactly_what_the_whole_file_would() {
        let text = "\
fn one() {}
/* 一條橫跨好多行的塊註釋
   第二行
   第三行
   第四行 */
fn two() -> &'static str {
    let s = \"一個字串\";
    let long = \"
橫跨幾行的字串
還在裏面
\";
    s
}
// 末尾一行註釋
struct 三 { 甲: u8 }
";
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let whole = highlight(Language::Rust, &lines);
        let source = joined(&lines);
        let tree = parse(Language::Rust, &source, None).expect("解析得了");
        // 每一個起點、每一種窗高都對一遍。
        for from in 0..lines.len() {
            for deep in 1..=lines.len() - from {
                let rows = from..from + deep;
                let window = paint(Language::Rust, &source, &tree, &lines, rows.clone());
                assert_eq!(
                    window,
                    whole[rows.clone()].to_vec(),
                    "第 {from} 行起 {deep} 行，和整份畫的不一樣"
                );
            }
        }
    }

    /// **量一遍：解析多久、查詢多久、只查一窗多久**（#423）。
    ///
    /// 不是斷言，是報數——`cargo test -p yumete-core how_long -- --nocapture`。
    /// Warning: **debug build 的數不能拿去做決定**，tree-sitter 在 debug 下慢一
    /// 個數量級；要真數就 `--release`。
    #[test]
    #[ignore = "報數用的，不是斷言；要跑加 --release --nocapture"]
    fn the_three_numbers_behind_423() {
        // Warning: **出廠那個路徑要從倉根算起，不是從當前目録**（2026-10-01）：
        // `cargo test` 跑在**這個 crate 的目録**裏，相對路徑於是找不到檔，一跑就
        // `NotFound`。帶 `#[ignore]` 的不進閘，所以沒人發現。
        let path = std::env::var("YUMETE_BENCH").unwrap_or_else(|_| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../yumete-tui/src/lib.rs")
                .to_string_lossy()
                .into_owned()
        });
        let text = std::fs::read_to_string(&path).expect("讀得到那個檔");
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let language = Language::Rust;
        let mut source = String::new();
        for line in &lines {
            source.push_str(line);
            source.push('\n');
        }
        println!("\n{path}：{} 行，{} KB", lines.len(), text.len() / 1024);

        let mut parser = Parser::new();
        parser.set_language(&language.grammar()).unwrap();
        let at = Instant::now();
        let tree = parser.parse(&source, None).unwrap();
        println!("  解析整份        {:?}", at.elapsed());

        let (query, _tokens) = language.query().unwrap();
        let count = |range: Option<std::ops::Range<usize>>| {
            let mut cursor = QueryCursor::new();
            if let Some(range) = range {
                cursor.set_byte_range(range);
            }
            let at = Instant::now();
            let mut n = 0usize;
            let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
            while let Some(m) = matches.next() {
                n += m.captures.len();
            }
            (at.elapsed(), n)
        };
        let (whole, n) = count(None);
        println!("  查詢整份        {whole:?}  {n} 處");

        // 一屏大約五十行，取檔子中間那一段。
        let mid = source.len() / 2;
        let window = source[..mid].rfind('\n').unwrap_or(0);
        let upto = source[window..]
            .char_indices()
            .filter(|(_, c)| *c == '\n')
            .nth(50)
            .map_or(source.len(), |(i, _)| window + i);
        let (one, n) = count(Some(window..upto));
        println!("  只查一窗（50 行）{one:?}  {n} 處");

        // **真檔上也對一遍**：合成的那一份小，橫跨邊緣的東西未必夠多。
        let whole = highlight(language, &lines);
        for from in (0..lines.len().saturating_sub(50)).step_by(997) {
            let rows = from..from + 50;
            let win = paint(language, &source, &tree, &lines, rows.clone());
            assert_eq!(win, whole[rows.clone()].to_vec(), "第 {from} 行起那一窗對不上");
        }
        println!("  每 997 行取一窗，和整份逐格相同");

        // paint() 本身要多久，在檔子的不同位置各量一次。
        for at in [500usize, 5_000, 10_000, 19_000] {
            if at + 128 >= lines.len() {
                continue;
            }
            let t = Instant::now();
            let got = paint(language, &source, &tree, &lines, at..at + 128);
            println!("  paint 128 行 @{at:>6}  {:?}  {} 行", t.elapsed(), got.len());
        }

        // **打一個字之後再解析一遍**，兩條路各量一次。
        let mut after = source.clone();
        let at_byte = window;
        after.insert(at_byte, 'x');
        let at = Instant::now();
        let mut fresh = Parser::new();
        fresh.set_language(&language.grammar()).unwrap();
        let _ = fresh.parse(&after, None).unwrap();
        println!("  改一個字，從頭解析 {:?}", at.elapsed());

        let mut edited = tree.clone();
        edited.edit(&tree_sitter::InputEdit {
            start_byte: at_byte,
            old_end_byte: at_byte,
            new_end_byte: at_byte + 1,
            start_position: tree_sitter::Point::new(0, 0),
            old_end_position: tree_sitter::Point::new(0, 0),
            new_end_position: tree_sitter::Point::new(0, 1),
        });
        let at = Instant::now();
        let again = parser.parse(&after, Some(&edited)).unwrap();
        println!("  改一個字，增量解析 {:?}", at.elapsed());
        let _ = again;
        println!();
    }
}

