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
use tree_sitter::{Parser, Query, QueryCursor};

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
    fn query(self) -> Option<&'static (Query, Vec<Paint>)> {
        static CELLS: [OnceLock<Option<(Query, Vec<Paint>)>>; 9] =
            [const { OnceLock::new() }; 9];
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

/// What a capture does to the characters it covers: `None` leaves them to
/// whatever else names them.
type Paint = Option<Token>;

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
    let mut out = vec![Vec::new(); lines.len()];
    let Some((query, tokens)) = language.query() else {
        return out;
    };
    let mut source = String::new();
    let mut starts = Vec::with_capacity(lines.len());
    for line in lines {
        starts.push(source.len());
        source.push_str(line);
        source.push('\n');
    }
    let mut parser = Parser::new();
    if parser.set_language(&language.grammar()).is_err() {
        return out;
    }
    let Some(tree) = parser.parse(&source, None) else {
        return out;
    };

    // (start, end, pattern, token) in bytes of `source`.
    let mut found: Vec<(usize, usize, usize, Token)> = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
    while let Some(m) = matches.next() {
        for capture in m.captures {
            if let Some(Some(token)) = tokens.get(capture.index as usize) {
                let node = capture.node;
                found.push((node.start_byte(), node.end_byte(), m.pattern_index, *token));
            }
        }
    }
    // **Outer first, then in the order the query lists them**, each painting
    // over the last: an escape inside a string is its own colour, and when two
    // patterns name the same node **the later one wins**. That is how these
    // queries are written — Python opens with `(identifier) @variable` and
    // says `@function` further down; YAML lists `@string` above `@property` —
    // and it is Helix's and Neovim's rule too. (`tree-sitter-highlight` has
    // the opposite one, and read with it every function came out plain.)
    //
    // ⚠️ **One exception: a name beats a string on the same node.** JSON's
    // query is the one written the other way round — `@string.special.key`
    // above `(string) @string` — so a key came out the green of its value.
    let named: std::collections::HashSet<(usize, usize)> = found
        .iter()
        .filter(|f| f.3 == Token::Property)
        .map(|f| (f.0, f.1))
        .collect();
    found.retain(|f| !(f.3 == Token::String && named.contains(&(f.0, f.1))));
    found.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));

    // ⚠️ **A window over the captures, not a pass over them** (2026-09-20).
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
    let mut next = 0usize;
    let mut live: Vec<(usize, usize, Token)> = Vec::new();
    for (n, line) in lines.iter().enumerate() {
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
        let spans = &mut out[n];
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
    out
}

fn span(start: usize, end: usize, token: Token) -> Span {
    Span {
        start,
        end,
        kind: Kind::Token(token),
        // Every run its own construct: none of this is markup, so none of it
        // is ever hidden or opened together.
        construct: usize::MAX - start,
    }
}

#[cfg(test)]
mod tests {
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
}
