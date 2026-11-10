//! Code inside a fence, coloured by its own grammar (#420).
//!
//! **Only the fence is parsed.** A manuscript is never handed to tree-sitter:
//! its markup is line-local and cached per paragraph, and a parser that wants
//! the whole document would fight that. A fence is different — it is a closed
//! run of lines that says, on its first line, what language it is written in —
//! so it is parsed on its own, and the answer is handed back line by line in
//! the same [`Span`] shape every other colour on the page already travels in.
//!
//! **Grammars ship for the languages [`Language`] lists**; a fence in any
//! other language, or none, keeps a fence's one colour.

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
    /// A function the language gives you — `print`, `len` (One Dark colours
    /// it apart from the rest, so the token has to say it).
    BuiltinFunction,
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
    /// A named constant, `None`.
    Constant,
    /// A number or a boolean (Zed colours them apart from a named constant;
    /// this editor's own themes do not).
    Number,
    /// A function's parameter (same reason).
    Parameter,
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
    C,
    /// C++.
    Cpp,
    Ada,
    /// Shell scripts: bash, sh, zsh.
    Bash,
    Css,
    /// GAS（`.s`）——與 [`Language::Nasm`] 共用一套通用匯編語法，分家分的是註釋記號。
    Gas,
    Go,
    Html,
    Java,
    JavaScript,
    Json,
    /// Lua.
    Lua,
    /// Mojo（`.mojo` `.🔥`）——語法是我們自己的，見
    /// <https://github.com/forfudan/tree-sitter-mojo>。
    Mojo,
    /// NASM（`.asm` `.S` `.nasm`）——見 [`Language::Gas`]。
    Nasm,
    Pascal,
    Python,
    R,
    Rust,
    Toml,
    /// TypeScript, and TSX, which is TypeScript with JSX in it.
    Tsx,
    TypeScript,
    /// Typst — a raw block tagged ```` ```typ ````, and the
    /// colours of a Typst manuscript itself. Never a file's syntax: a `.typ`
    /// is [`crate::syntax::Syntax::Typst`], whose prose the block scanner
    /// reads. The grammar is ours:
    /// <https://github.com/forfudan/tree-sitter-typst>.
    Typst,
    Yaml,
}

impl Language {
    /// Every language, for `:view-code`'s report and the tests.
    pub const ALL: [Language; 23] = [
        Language::Ada,
        Language::Bash,
        Language::C,
        Language::Cpp,
        Language::Css,
        Language::Gas,
        Language::Go,
        Language::Html,
        Language::Java,
        Language::JavaScript,
        Language::Json,
        Language::Lua,
        Language::Mojo,
        Language::Nasm,
        Language::Pascal,
        Language::Python,
        Language::R,
        Language::Rust,
        Language::Toml,
        Language::Tsx,
        Language::TypeScript,
        Language::Typst,
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
            "ada" => Language::Ada,
            "asm" | "nasm" => Language::Nasm,
            "bash" | "sh" | "shell" | "zsh" | "ksh" => Language::Bash,
            "c" => Language::C,
            "cpp" | "c++" | "cxx" | "cc" | "hpp" => Language::Cpp,
            "css" => Language::Css,
            "gas" => Language::Gas,
            "go" | "golang" => Language::Go,
            "html" | "htm" | "xhtml" => Language::Html,
            "java" | "jav" => Language::Java,
            "javascript" | "js" | "mjs" | "cjs" | "jsx" | "node" => Language::JavaScript,
            "json" | "jsonc" | "json5" | "geojson" => Language::Json,
            "lua" => Language::Lua,
            "mojo" => Language::Mojo,
            "pascal" | "delphi" | "pas" => Language::Pascal,
            "python" | "py" | "python3" | "py3" => Language::Python,
            "r" => Language::R,
            "rust" | "rs" => Language::Rust,
            "toml" => Language::Toml,
            "typescript" | "ts" => Language::TypeScript,
            "tsx" => Language::Tsx,
            // Typst's own tags: `typ` for markup, `typc` for code. The grammar
            // has no entry point for code, so a `typc` block is read as
            // markup, which its upstream README says reads the same for one
            // expression.
            "typst" | "typ" | "typc" => Language::Typst,
            "yaml" | "yml" => Language::Yaml,
            _ => return None,
        })
    }

    /// The language a file's extension says it is in. Narrower than
    /// [`Self::from_info`]: a file called `node` is not JavaScript.
    ///
    /// Warning: **收的是原樣的副檔名，轉小寫在這一支裏面**。幾乎
    /// 所有副檔名不分大小寫，但**匯編分**：`.s` 是 GAS（註釋 `#`），`.S` 是要
    /// 走預處理的那一種，照 helix 歸 NASM（註釋 `;`）。轉小寫放在呼叫端的話，
    /// 這一層永遠再也分不出兩者——所以先看原樣的，再轉小寫查其餘。
    pub fn from_extension(extension: &str) -> Option<Language> {
        // **大小寫真的算數的就這兩個**，實打實地寫在前面。
        match extension {
            "s" => return Some(Language::Gas),
            "S" => return Some(Language::Nasm),
            _ => {}
        }
        let extension = extension.to_ascii_lowercase();
        Some(match extension.as_str() {
            "adb" | "ads" | "ada" => Language::Ada,
            "asm" | "nasm" => Language::Nasm,
            // **`.h` 走 C**（helix 也是這麼分的）。
            //
            // 一份 C++ 的頭文件於是也走 C 的語法：絕大部分構造判得對，`template`／
            // `class` 那幾個詞判不對。代價說明白了還是比「一個顏色都没有」好——而
            // `.cpp`／`.ts` 老實地一個都不認（没帶那兩種語法）。
            "c" | "h" => Language::C,
            // helix's list, less the ones C or another tool has first.
            "cc" | "hh" | "c++" | "cpp" | "hpp" | "ipp" | "tpp" | "cxx" | "hxx" | "ixx" | "txx"
            | "ino" | "cu" | "cuh" | "cppm" | "h++" | "ii" | "inl" => Language::Cpp,
            "sh" | "bash" | "ash" | "dash" | "ksh" | "mksh" | "zsh" | "zshenv" | "zlogin"
            | "zlogout" | "zprofile" | "zshrc" | "eclass" | "ebuild" | "bazelrc" | "bashrc"
            | "bash_profile" | "bash_login" | "bash_logout" => Language::Bash,
            "css" => Language::Css,
            "go" => Language::Go,
            "html" | "htm" | "xhtml" => Language::Html,
            "java" | "jav" | "pde" => Language::Java,
            "js" | "mjs" | "cjs" | "jsx" => Language::JavaScript,
            "json" | "jsonc" | "json5" | "geojson" => Language::Json,
            "lua" | "rockspec" => Language::Lua,
            // **第二個是真的副檔名**：Mojo 收 `.mojo` 和 `.🔥`（上游自己這麼定的）。
            "mojo" | "🔥" => Language::Mojo,
            "pas" | "pp" | "dpr" | "dpk" | "lpr" => Language::Pascal,
            "py" | "pyw" => Language::Python,
            "r" => Language::R,
            "rs" => Language::Rust,
            "toml" => Language::Toml,
            "ts" | "mts" | "cts" => Language::TypeScript,
            "tsx" => Language::Tsx,
            "yaml" | "yml" => Language::Yaml,
            _ => return None,
        })
    }

    /// Its name, as a fence would spell it.
    pub fn name(self) -> &'static str {
        match self {
            Language::Ada => "ada",
            Language::Bash => "bash",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::Css => "css",
            Language::Gas => "gas",
            Language::Go => "go",
            Language::Html => "html",
            Language::Java => "java",
            Language::JavaScript => "javascript",
            Language::Json => "json",
            Language::Lua => "lua",
            Language::Mojo => "mojo",
            Language::Nasm => "nasm",
            Language::Pascal => "pascal",
            Language::Python => "python",
            Language::R => "r",
            Language::Rust => "rust",
            Language::Toml => "toml",
            Language::Tsx => "tsx",
            Language::TypeScript => "typescript",
            Language::Typst => "typst",
            Language::Yaml => "yaml",
        }
    }

    fn grammar(self) -> tree_sitter::Language {
        match self {
            Language::Ada => tree_sitter_ada::LANGUAGE.into(),
            Language::Bash => tree_sitter_bash::LANGUAGE.into(),
            Language::C => tree_sitter_c::LANGUAGE.into(),
            Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Language::Css => tree_sitter_css::LANGUAGE.into(),
            // **一套語法兩門語言**：`tree-sitter-asm` 是通用匯編語法，三種行
            // 註釋（`#` `//` `;`）全認；分成 GAS 與 NASM 是為了 `空格 c` 寫得出
            // 對的那一個——寫錯一邊是真的編不過。helix 也分兩門（它用兩套語法）。
            Language::Gas | Language::Nasm => tree_sitter_asm::LANGUAGE.into(),
            Language::Go => tree_sitter_go::LANGUAGE.into(),
            Language::Html => tree_sitter_html::LANGUAGE.into(),
            Language::Java => tree_sitter_java::LANGUAGE.into(),
            Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Language::Json => tree_sitter_json::LANGUAGE.into(),
            Language::Lua => tree_sitter_lua::LANGUAGE.into(),
            Language::Mojo => tree_sitter_mojo::LANGUAGE.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::Pascal => tree_sitter_pascal::LANGUAGE.into(),
            Language::R => tree_sitter_r::LANGUAGE.into(),
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
            Language::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
            Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Typst => tree_sitter_typst::LANGUAGE.into(),
            Language::Yaml => tree_sitter_yaml::LANGUAGE.into(),
        }
    }

    /// **一條追加的規則，讓調用看起來像調用**。
    ///
    /// 要的是「同一個構造在十六種語言裏同一個顏色」。「照 helix，改成第一條規則贏」
    /// 那個辦法的前提查不實：
    ///
    /// | | `@function.method` 在第幾行 | 兜底的 `@property` | 最後一條贏 | 第一條贏 |
    /// | --- | --- | --- | --- | --- |
    /// | Go | 12 | 25 | 調用與字段同色 ✗ | 分得開 ✓ |
    /// | JavaScript | 19／27 | 9 | 分得開 ✓ | 同色 ✗ |
    ///
    /// 兩家查詢寫的順序正好相反，所以**翻比較器只是把錯處從一邊搬到另一邊**（實測
    /// 過：Go 對了，JS 的 `o.run()` 從藍變成金）。helix 兩邊都對，是因為**它自己策過
    /// 那些查詢檔**——它的 Go 查詢裏根本没有那條兜底的 `@property`。
    ///
    /// Warning: **而 helix 的查詢檔抄不得**：MPL-2.0，這個倉是 Apache-2.0（同
    /// `objects()` 那一條）。所以這裏走第三條路：**自己追加一條**。最後一條贏，追加
    /// 的就贏，而另外十五種語言一個像素都不動。
    fn extra_highlights(self) -> &'static str {
        match self {
            // `fmt.Println(1)` 的 `Println` 是一個 `field_identifier`，而 Go 的查詢
            // 先捕成 `@function.method`、後又被兜底的 `(field_identifier) @property`
            // 蓋掉。把調用那一種再捕一次，擺在最後。
            Language::Go => {
                "\n(call_expression function: (selector_expression                  field: (field_identifier) @function.method))\n"
            }
            // `os.getcwd()` 的 `getcwd` 同理：Python 的查詢把它兜底成了 `@attribute`。
            Language::Python => {
                "\n(call (attribute attribute: (identifier) @function.method))\n"
            }
            // **What the page already said, kept**. The
            // `#` before code takes the colour of what follows it — Typst's
            // own highlighter does this (`highlight_hash`), and so did the
            // scanner this grammar replaced; `#name` is drawn as a call, as that
            // scanner drew it (Typst gives it a colour of its own, which no theme
            // here has); the `*` and `_` of strong and
            // emphasis are the markers 所見即所得 hides; a `$` is the maths', and a
            // raw block's tag is the raw block's, as Typst draws it. In maths,
            // as Typst's highlighter has it: a number is text, and `^` `_` `/` `'` `√` `&` are
            // operators.
            //
            // **And the rest as typst-syntax 0.15's `highlight` has it**,
            // where nothing above says otherwise. Typst
            // gives what it calls interpolated — a name after `#`, and a
            // symbol in maths — a colour of its own, the same blue as a call
            // in its own theme; this page draws it as a call, as `#name`
            // above. So: `#` before a field access, a string, a number or
            // `true` takes the colour of what follows, and the names of a
            // field access after `#` are drawn as `#name` is (`#it.body`,
            // the `calc` of `#calc.pow(…)`, three deep; `it.body` inside a
            // code block is not interpolated, and keeps its plain names);
            // `none`, `auto`, `true` and `false` are keywords; in maths `pi`,
            // `sum` and `alpha` are drawn as a call, and `+`, `=`, `⌈` and a
            // bracket are text — only the brackets Typst takes away, round a
            // fraction's part, a script or a root's radicand, are
            // punctuation.
            Language::Typst => {
                "
(code \"#\" @keyword [(let_binding) (set_rule) (show_rule) (module_import)
  (module_include) (conditional) (for_loop) (while_loop) (contextual)
  (return_statement) (break_statement) (continue_statement)])
(code \"#\" @function (call))
(code \"#\" @function (identifier) @function)
(code \"#\" @function . (field_access))
(code (field_access object: (identifier) @function))
(code (field_access field: (identifier) @function))
(code (field_access object: (field_access object: (identifier) @function)))
(code (field_access object: (field_access field: (identifier) @function)))
(code (field_access object: (field_access object: (field_access object: (identifier) @function))))
(code (field_access object: (field_access object: (field_access field: (identifier) @function))))
(code (call function: (field_access object: (identifier) @function)))
(code (call function: (field_access object: (field_access object: (identifier) @function))))
(code (call function: (field_access object: (field_access field: (identifier) @function))))
[(bool) (none) (auto)] @keyword
(code \"#\" @string . (string))
(code \"#\" @keyword . [(bool) (none) (auto)])
(code \"#\" @constant.numeric . [(int) (float) (numeric)])
(strong \"*\" @markup.marker)
(emph \"_\" @markup.marker)
(equation \"$\" @markup.math)
(raw_block (raw_lang) @markup.raw.block)
(math_number) @variable
(math_ident) @function
(math_symbol) @variable
(math_delimited [\"(\" \")\"] @variable)
(math_delimiter) @variable
(math_frac numerator: (math_delimited open: \"(\" @punctuation.bracket close: \")\" @punctuation.bracket))
(math_frac denominator: (math_delimited open: \"(\" @punctuation.bracket close: \")\" @punctuation.bracket))
(math_attach top: (math_delimited open: \"(\" @punctuation.bracket close: \")\" @punctuation.bracket))
(math_attach bottom: (math_delimited open: \"(\" @punctuation.bracket close: \")\" @punctuation.bracket))
(math_root radicand: (math_delimited open: \"(\" @punctuation.bracket close: \")\" @punctuation.bracket))
(math_attach [\"_\" \"^\"] @markup.math.operator)
(math_frac \"/\" @markup.math.operator)
[(primes) (root_sign) (align_point)] @markup.math.operator
(math_shorthand) @constant.character.escape
"
            }
            _ => "",
        }
    }

    fn highlights(self) -> &'static str {
        match self {
            // **Ada 與 Pascal 這兩份是我們自己寫的**：最安全。
            //
            // Pascal 那一份帶 `@function`／`@type`：只認關鍵詞的話，
            // `function Add(A: Integer): Integer;` 裏 `Add`／`A`／`Integer` 全是素色。節點名是從語法自己的 `src/node-types.json` 讀出來的
            // ——那是**生成的 API 描述**，不是那份 `.scm`。
            //
            // 兩個 crate 包裏有 `queries/highlights.scm` 這個**檔**，可
            // `bindings/rust/lib.rs` 裏那一行 `pub const HIGHLIGHTS_QUERY` 是**注掉的**，
            // 跨 crate 的 `include_str!` 做不到。抄那兩份 `.scm` 要帶它們的授權聲明，
            // 自己寫没這一層事——同 `objects()`／`indents()` 那一條規矩。
            //
            // ⚠ **節點名是從語法自己的 `src/node-types.json` 生成的，不是記的。**
            // 一個名字寫錯，`Query::new` 整條失敗，那一門語言**一個字都不上色**
            // 而且不報錯——`every_shipped_grammar_compiles_its_own_query` 就是描這一條的。
            // ⚠ **`r##"` 不是 `r#"`**：gnatprep 那四個詞裏的 `"#` 會把 `r#"` 当場收掉。
            Language::Ada => r##"
                    (comment) @comment
                    [(string_literal) (character_literal)] @string
                    (numeric_literal) @number
                    (procedure_specification name: (identifier) @function)
                    (function_specification name: (identifier) @function)
                    (procedure_call_statement name: (identifier) @function)
                    (function_call name: (identifier) @function)
                    [
                      "#else" "#elsif" "#end" "#if" "Class" "abort" "abs"
                      "abstract" "accept" "access" "aliased" "all" "and" "array"
                      "at" "begin" "body" "case" "constant" "declare" "delay"
                      "delta" "digits" "do" "else" "elsif" "end" "entry"
                      "exception" "exit" "for" "function" "generic" "goto" "if"
                      "in" "interface" "is" "limited" "loop" "mod" "new"
                      "not" "null" "of" "or" "others" "out" "overriding"
                      "package" "parallel" "pragma" "private" "procedure" "protected" "raise"
                      "range" "record" "rem" "renames" "requeue" "return" "reverse"
                      "select" "separate" "some" "subtype" "synchronized" "tagged" "task"
                      "terminate" "then" "type" "until" "use" "when" "while"
                      "with" "xor"
                    ] @keyword
                    [
                      "&" "*" "**" "+" "-" "." ".." "/" "/="
                      ":=" "<" "<<" "<=" "<>" "=" "=>" ">" ">="
                      ">>" "|"
                    ] @operator
"##,
            // **Mojo 那一份也是自己寫的**——它的語法是我們自己的
            // （`forfudan/tree-sitter-mojo`），所以根本没有別人的 `.scm` 可抄，
            // 也没有授權那一層事。節點名與詞表是從它自己的
            // `src/node-types.json` 讀出來的。
            //
            // `__generator_type`／`__match`／`__mlir_region` 這幾個帶雙下劃線的
            // 是 Mojo 真有的關鍵詞，不是筆誤。
            Language::Mojo => r#"
                    (comment) @comment
                    [(string) (t_string) (concatenated_string)] @string
                    [(integer) (float)] @number
                    [(true) (false) (none)] @constant
                    (function_definition name: (identifier) @function)
                    (struct_definition name: (identifier) @type)
                    (trait_definition name: (identifier) @type)
                    (enum_definition name: (identifier) @type)
                    (enum_case name: (identifier) @constant)
                    (extension_definition name: (identifier) @type)
                    (conformance (identifier) @type)
                    (parameter type: (identifier) @type)
                    (assignment type: (identifier) @type)
                    (decorator) @attribute
                    (keyword_argument name: (identifier) @property)
                    (attribute attribute: (identifier) @property)
                    (dot_member attribute: (identifier) @property)
                    (call function: (identifier) @function)
                    ; Warning: 這兩條要擺在 @property 後面——最後一條贏（同
                    ; extra_highlights 那一節），不然 x.f() 的 f 被兜底成字段。
                    ; 查詢語言的註釋是分號，不是 //（寫錯整條查詢不編譯）。
                    (call function: (attribute attribute: (identifier) @function))
                    (call function: (dot_member attribute: (identifier) @function))
                    [
                      "__async" "__await" "__enum" "__extension"
                      "__generator_type" "__match" "__mlir_region"
                      "abi" "alias" "and" "as" "assert" "async" "await"
                      "borrowed" "capturing" "case" "comptime" "def" "deinit"
                      "del" "elif" "else" "except" "finally" "fn" "for" "from"
                      "global" "if" "imm" "import" "in" "inout" "is" "lambda"
                      "match" "mut" "nonlocal" "not" "or" "out" "owned" "raise" "raises"
                      "read" "ref" "register_passable" "return" "struct" "thin"
                      "trait" "try" "var" "where" "while" "with"
                    ] @keyword
                    [
                      "!=" "%" "%=" "&" "&=" "*" "**" "**=" "*=" "+" "+="
                      "-" "-=" "->" "/" "//" "//=" "/=" ":=" "<" "<<" "<<="
                      "<=" "=" "==" ">" ">=" ">>" ">>=" "@=" "^" "^=" "|" "|=" "~"
                    ] @operator
"#,
            Language::Pascal => r#"
                    (declProc name: (identifier) @function)
                    (declProp name: (identifier) @function)
                    (declType name: (identifier) @type)
                    (typeref) @type
                    (comment) @comment
                    [(literalString) (literalChar)] @string
                    (literalNumber) @number
                    [(kTrue) (kFalse) (kNil)] @constant
                    [
                      (kAbsolute) (kAbstract) (kAlias) (kAnd) (kArray) (kAs)
                      (kAsm) (kAssembler) (kBegin) (kCase) (kCdecl) (kClass)
                      (kConst) (kConstref) (kConstructor) (kCppdecl) (kCvar) (kDefault)
                      (kDelayed) (kDeprecated) (kDestructor) (kDispId) (kDispInterface) (kDo)
                      (kDownto) (kDynamic) (kElse) (kEnd) (kExcept) (kExperimental)
                      (kExport) (kExports) (kExternal) (kFar) (kFile) (kFinalization)
                      (kFinally) (kFor) (kForward) (kFunction) (kGeneric) (kGoto)
                      (kHardfloat) (kHelper) (kIf) (kImplementation) (kImplements) (kIn)
                      (kIndex) (kInherited) (kInitialization) (kInline) (kInterface) (kInterrupt)
                      (kIocheck) (kIs) (kLabel) (kLibrary) (kLocal) (kMessage)
                      (kMs_abi_cdecl) (kMs_abi_default) (kMwpascal) (kName) (kNear) (kNodefault)
                      (kNoreturn) (kNostackframe) (kNot) (kObjccategory) (kObjcclass) (kObjcprotocol)
                      (kObject) (kOf) (kOn) (kOperator) (kOptional) (kOr)
                      (kOut) (kOverload) (kOverride) (kPacked) (kPascal) (kPlatform)
                      (kPrivate) (kProcedure) (kProgram) (kProperty) (kProtected) (kPublic)
                      (kPublished) (kRaise) (kRead) (kRecord) (kReference) (kRegister)
                      (kReintroduce) (kRepeat) (kRequired) (kResourcestring) (kSafecall) (kSaveregisters)
                      (kSealed) (kSet) (kSoftfloat) (kSpecialize) (kStatic) (kStdcall)
                      (kStored) (kStrict) (kString) (kSysv_abi_cdecl) (kSysv_abi_default) (kThen)
                      (kThreadvar) (kTo) (kTry) (kType) (kUnimplemented) (kUnit)
                      (kUntil) (kUses) (kVar) (kVarargs) (kVectorcall) (kVirtual)
                      (kWhile) (kWinapi) (kWith) (kWrite) (kXor)
                    ] @keyword
                    [
                      (kAdd) (kAssign) (kAssignAdd) (kAssignDiv) (kAssignMul) (kAssignSub)
                      (kAt) (kDiv) (kDot) (kEndDot) (kEq) (kFdiv)
                      (kGt) (kGte) (kHat) (kLt) (kLte) (kMod)
                      (kMul) (kNeq) (kShl) (kShr) (kSub)
                    ] @operator
"#,
            // Warning: **C 那一個叫 `HIGHLIGHT_QUERY`**，單數——同一族 crate 兩種拼法。
            Language::C => tree_sitter_c::HIGHLIGHT_QUERY,
            // **Written to sit on top of another's**: C++'s
            // queries extend C's and TypeScript's extend JavaScript's, as the
            // grammars' own READMEs say — so each is read with its base.
            Language::Cpp => inherited(&CPP, tree_sitter_c::HIGHLIGHT_QUERY, tree_sitter_cpp::HIGHLIGHT_QUERY),
            Language::TypeScript | Language::Tsx => inherited(
                &TYPESCRIPT,
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
            ),
            Language::Bash => tree_sitter_bash::HIGHLIGHT_QUERY,
            Language::Lua => tree_sitter_lua::HIGHLIGHTS_QUERY,
            Language::Css => tree_sitter_css::HIGHLIGHTS_QUERY,
            Language::Gas | Language::Nasm => tree_sitter_asm::HIGHLIGHTS_QUERY,
            Language::Go => tree_sitter_go::HIGHLIGHTS_QUERY,
            Language::Html => tree_sitter_html::HIGHLIGHTS_QUERY,
            Language::Java => tree_sitter_java::HIGHLIGHTS_QUERY,
            Language::JavaScript => tree_sitter_javascript::HIGHLIGHT_QUERY,
            Language::Json => tree_sitter_json::HIGHLIGHTS_QUERY,
            Language::Python => tree_sitter_python::HIGHLIGHTS_QUERY,
            Language::R => tree_sitter_r::HIGHLIGHTS_QUERY,
            Language::Rust => tree_sitter_rust::HIGHLIGHTS_QUERY,
            Language::Toml => tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            // Ours, so read as it ships; its capture names are helix's, which
            // [`typst_paint`] reads.
            Language::Typst => tree_sitter_typst::HIGHLIGHTS_QUERY,
            Language::Yaml => tree_sitter_yaml::HIGHLIGHTS_QUERY,
        }
    }

    /// The compiled query and what each of its captures means. Compiled once
    /// per process: a query is parsed from its source text, and that is not
    /// something to do per frame.
    /// **語法樹裏那些「一個定義」的查詢**（`]f`/`mi f` 那一族）。
    ///
    /// Warning: **用語法本身帶的 `TAGS_QUERY`，不抄 helix 的 `textobjects.scm`。**
    /// helix 把文本對象寫成自己 runtime 裏的查詢檔，而那些檔是 **MPL-2.0**，這個
    /// 倉是 Apache-2.0——照抄要先定授權怎麼辦。`tags.scm` 是**語法 crate 自己帶
    /// 的**（我們本來就依賴它，和 `HIGHLIGHTS_QUERY` 同一個來源、同一份授權），
    /// 而它捕獲的 `@definition.function`/`@definition.class` 包的正是整個定義，
    /// 名字另有 `@name`。九種語言裏七種帶，css 與 html 不帶。
    ///
    /// 它給不出的：參數、註釋、測試——那幾種只在 helix 的 textobjects 裏有。
    fn tags(self) -> Option<&'static str> {
        Some(match self {
            Language::C => tree_sitter_c::TAGS_QUERY,
            Language::Cpp => tree_sitter_cpp::TAGS_QUERY,
            Language::TypeScript | Language::Tsx => tree_sitter_typescript::TAGS_QUERY,
            Language::Lua => tree_sitter_lua::TAGS_QUERY,
            Language::Go => tree_sitter_go::TAGS_QUERY,
            Language::Java => tree_sitter_java::TAGS_QUERY,
            Language::JavaScript => tree_sitter_javascript::TAGS_QUERY,
            Language::Python => tree_sitter_python::TAGS_QUERY,
            // **這一份的 `tags.scm` 是我們自己寫的**——語法是我們自己的，
            // 所以没有別人的可用，也没有授權那一層事。函數、結構、特徵、
            // 擴展四種；`alias` 捕不到，那是語法那一頭的事（見那個檔的註釋）。
            Language::Mojo => tree_sitter_mojo::TAGS_QUERY,
            // Headings and `#let` bindings, ours too.
            Language::Typst => tree_sitter_typst::TAGS_QUERY,
            Language::R => tree_sitter_r::TAGS_QUERY,
            Language::Rust => tree_sitter_rust::TAGS_QUERY,
            // 這幾種没有定義可言，語法 crate 也不帶 tags。
            Language::Css
            | Language::Html
            | Language::Json
            | Language::Toml
            | Language::Yaml
            | Language::Gas
            | Language::Nasm
            // 這兩門的 crate 同樣没把 `tags.scm` 導出來。
            | Language::Ada
            | Language::Pascal
            | Language::Bash => return None,
        })
    }

    /// **我們自己寫的那份文本對象查詢**，不抄。
    ///
    /// Warning: **不抄 helix 的 `textobjects.scm`**——那些檔是 MPL-2.0，這個倉是
    /// Apache-2.0（見 [`Language::tags`]）。自己寫還有一層好處：helix 那份要照顧
    /// 幾十種語言和它自己的鍵，我們只要這八種語言、兩種對象。
    ///
    /// ⚠ **代價是上游改語法我們自己盯**：下面這些節點名是寫死的，`tree-sitter-*`
    /// 升一版把某個節點改了名，查詢就**靜悄悄地不匹配**——不報錯、不編譯失敗，
    /// `mi a` 就是没反應。所以每一種語言各釘一格測試
    /// （`our_textobject_queries_still_match_these_grammars`）。
    ///
    /// 節點名是從語法自己的 `src/node-types.json` 裏讀出來的，不是記的。
    fn objects(self) -> Option<&'static str> {
        // **`@test` / `@entry` / `@element`** (helix's `T` `e` `x`):
        // a test is told by its name or its attribute — `#[test]`, `def test_…`,
        // `func Test…`, `it(…)`/`test(…)`/`describe(…)`, `@Test` — and an
        // entry is one key with its value, or one field. Node and field names
        // read from each grammar's `node-types.json`, as the rest.
        Some(match self {
            Language::Python => {
                "(parameters (_) @parameter)
                 (lambda_parameters (_) @parameter)
                 (comment) @comment
                 ((function_definition name: (identifier) @_name) @test
                  (#match? @_name \"^test\"))
                 (dictionary (pair) @entry)
"
            }
            Language::Rust => {
                "(parameters (_) @parameter)
                 (closure_parameters (_) @parameter)
                 (type_parameters (_) @parameter)
                 (line_comment) @comment
                 (block_comment) @comment
                 ((attribute_item (attribute (identifier) @_attr)) . (function_item) @test
                  (#eq? @_attr \"test\"))
                 ((attribute_item (attribute (scoped_identifier name: (identifier) @_attr))) .
                  (function_item) @test (#eq? @_attr \"test\"))
                 (field_declaration) @entry
                 (field_initializer) @entry
                 (shorthand_field_initializer) @entry
"
            }
            Language::Go => {
                "(parameter_list (_) @parameter)
                 (type_parameter_list (_) @parameter)
                 (comment) @comment
                 ((function_declaration name: (identifier) @_name) @test
                  (#match? @_name \"^(Test|Benchmark|Example|Fuzz)\"))
                 (keyed_element) @entry
                 (field_declaration) @entry
"
            }
            Language::JavaScript => {
                "(formal_parameters (_) @parameter)
                 (comment) @comment
                 ((call_expression function: (identifier) @_name) @test
                  (#match? @_name \"^(it|test|describe)$\"))
                 (object (pair) @entry)
"
            }
            Language::C | Language::Cpp => {
                "(parameter_list (_) @parameter)
                 (comment) @comment
"
            }
            Language::TypeScript | Language::Tsx => {
                "(formal_parameters (_) @parameter)
                 (comment) @comment
                 ((call_expression function: (identifier) @_name) @test
                  (#match? @_name \"^(it|test|describe)$\"))
                 (object (pair) @entry)
"
            }
            Language::Bash => "(comment) @comment\n",
            Language::Lua => {
                "(parameters (_) @parameter)
                 (comment) @comment
                 (table_constructor (field) @entry)
"
            }
            Language::Java => {
                "(formal_parameters (_) @parameter)
                 (type_parameters (_) @parameter)
                 (inferred_parameters (_) @parameter)
                 (line_comment) @comment
                 (block_comment) @comment
                 ((method_declaration (modifiers (marker_annotation name: (identifier) @_name))) @test
                  (#eq? @_name \"Test\"))
"
            }
            // Warning: **R 這一門不許寫 `(_)`**：它把逗號也做成了一個**有名字**的節點
            // （`node-types.json` 裏 `parameters` 的子節點類型就只列着 `comma`），
            // 於是 `(a, b)` 用 `(_)` 數出來是三個。寫明 `(parameter)`。
            Language::R => {
                "(parameters (parameter) @parameter)
                 (comment) @comment
"
            }
            // Mojo 的 `parameters` 只收 `parameter` 一種子節點（`node-types.json`
            // 查過），所以寫得明白，不必像 R 那樣躲逗號。
            Language::Mojo => {
                "(parameters (parameter) @parameter)
                 (comment) @comment
"
            }
            Language::Html => {
                "(element) @element
                 (script_element) @element
                 (style_element) @element
                 (comment) @comment
"
            }
            Language::Json => "(pair) @entry\n",
            Language::Typst => {
                "(line_comment) @comment
                 (block_comment) @comment
"
            }
            Language::Toml => {
                "(pair) @entry
                 (comment) @comment
"
            }
            Language::Yaml => {
                "(block_mapping_pair) @entry
                 (flow_pair) @entry
                 (comment) @comment
"
            }
            Language::Css
            // 匯編裏没有「參數」這種東西。
            | Language::Gas
            | Language::Nasm
            | Language::Ada
            | Language::Pascal => return None,
        })
    }

    /// 編好的那份，連着每一格捕獲算哪一種對象。
    fn object_query(self) -> Option<&'static Objects> {
        static CELLS: [OnceLock<Option<Objects>>; Language::ALL.len()] = [const { OnceLock::new() }; Language::ALL.len()];
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
                        "test" => Some(Object::Test),
                        "entry" => Some(Object::Entry),
                        "element" => Some(Object::Element),
                        _ => None,
                    })
                    .collect();
                Some((query, kinds))
            })
            .as_ref()
    }

    /// 編好的 tags 查詢，連着每一格捕獲算哪一種定義。
    fn defines(self) -> Option<&'static Defines> {
        static CELLS: [OnceLock<Option<Defines>>; Language::ALL.len()] = [const { OnceLock::new() }; Language::ALL.len()];
        let at = Language::ALL.iter().position(|&l| l == self)?;
        CELLS[at]
            .get_or_init(|| {
                let query = Query::new(&self.grammar(), self.tags()?).ok()?;
                let kinds = query.capture_names().iter().map(|n| define_of(n)).collect();
                Some((query, kinds))
            })
            .as_ref()
    }

    /// **哪些節點開一級縮進，哪些把自己退回去**（第一期）。
    ///
    /// 問題是「行尾是一個 `(`，下一行該不該再縮一級」。查過三家——
    ///
    /// | | 縮嗎 |
    /// | --- | --- |
    /// | vim `autoindent` | 不縮，只照抄上一行的空白 |
    /// | vim `smartindent` | 不縮：它只認行尾的 `{`（`options.txt`：「After a line ending in '{'」） |
    /// | vim `cindent` | 縮，默認 `shiftwidth × 2`（`cinoptions` 的 `(N`） |
    /// | helix | **縮一級，`)` 退一級**（`indents.scm`：`arguments`/`parameters`/`call_expression` 是 `@indent`，`")"` 是 `@outdent`） |
    ///
    /// 定的是 helix 那一套（用語法樹，不用字符啓發式）。
    ///
    /// Warning: **這是我們自己寫的查詢，不是抄 helix 的檔。** 兩個理由：① 語法包只帶
    /// `highlights`/`injections`/`tags`，`indents.scm` 是 helix 自己的檔，而 helix 是
    /// MPL-2.0，這個倉是 Apache-2.0；② 第一期只用得上「哪些節點類型」這一層，而那
    /// 是語法本身的事實（`call_expression` 就叫這個名字），不是誰的文章。
    ///
    /// Warning: **第一期只有 `@indent` 和 `@outdent`。** helix 那一支還認 `@align`、
    /// `@extend`、`#not-same-line?` 那幾個謂詞，以及「賦值右邊」「`.await` 鏈」這些
    /// 專門的形狀——1502 行。那些留給下一期，見 `local/development.md` §5.117。
    fn indents(self) -> &'static str {
        match self {
            // 括號那三族 ＋ 塊。`token_tree` 是宏裏面，照樣要縮。
            Language::Rust => {
                r#"[(block) (arguments) (parameters) (declaration_list)
                    (field_declaration_list) (field_initializer_list)
                    (enum_variant_list) (call_expression) (token_tree)
                    (array_expression) (tuple_expression) (use_list)
                    (match_block)] @indent
                   ["}" "]" ")"] @outdent"#
            }
            Language::Go => {
                r#"[(block) (argument_list) (parameter_list) (literal_value)
                    (expression_case) (type_declaration) (const_declaration)
                    (var_declaration)] @indent
                   ["}" "]" ")"] @outdent"#
            }
            Language::JavaScript => {
                r#"[(statement_block) (arguments) (formal_parameters) (object)
                    (array) (class_body) (switch_body)
                    (parenthesized_expression)] @indent
                   ["}" "]" ")"] @outdent"#
            }
            Language::Python => {
                r#"[(block) (argument_list) (parameters) (list) (dictionary)
                    (set) (tuple) (parenthesized_expression)] @indent
                   ["}" "]" ")"] @outdent"#
            }
            // Mojo 和 Python 同形（冒號 ＋ 縮進），節點名是它自己的：括起來那一個
            // 叫 `parenthesised_expression`（英式拼法），形參表另有 `type_parameters`。
            // **`match` opens a level** (Mojo 1.2): its `case` lines are children
            // of the statement, not of a block, and the 1.2 sources indent them
            // in 316 of 449 statements (the rest are in three test files).
            Language::Mojo => {
                r#"[(block) (match_statement) (argument_list) (parameters) (type_parameters)
                    (list) (dictionary) (set) (tuple) (parenthesised_expression)] @indent
                   ["}" "]" ")"] @outdent"#
            }
            // Java 的塊、三種體、兩種參數表、陣列初始化、`switch` 體與註解的參數表。
            Language::Java => {
                r#"[(block) (class_body) (interface_body) (enum_body)
                    (constructor_body) (argument_list) (formal_parameters)
                    (type_parameters) (array_initializer) (switch_block)
                    (annotation_argument_list)] @indent
                   ["}" "]" ")"] @outdent"#
            }
            // C：`{}` 那一族在這門語言裏叫 `compound_statement`，另有結構體、
            // 枚舉、初始化串。
            Language::C => {
                r#"[(compound_statement) (field_declaration_list) (enumerator_list)
                    (initializer_list) (argument_list) (parameter_list)
                    (declaration_list)] @indent
                   ["}" "]" ")"] @outdent"#
            }
            // C++ is C's list and its own templates and classes.
            Language::Cpp => {
                r#"[(compound_statement) (field_declaration_list) (enumerator_list)
                    (initializer_list) (argument_list) (parameter_list)
                    (declaration_list) (template_argument_list)
                    (template_parameter_list)] @indent
                   ["}" "]" ")"] @outdent"#
            }
            // TypeScript is JavaScript's list and its types.
            Language::TypeScript | Language::Tsx => {
                r#"[(statement_block) (arguments) (formal_parameters) (object)
                    (array) (class_body) (switch_body) (parenthesized_expression)
                    (interface_body) (object_type) (enum_body) (type_arguments)
                    (type_parameters)] @indent
                   ["}" "]" ")"] @outdent"#
            }
            // A shell block ends on a word: `fi`, `done`, `esac`.
            Language::Bash => {
                r#"[(compound_statement) (do_group) (if_statement) (case_statement)
                    (case_item) (subshell) (array)] @indent
                   ["}" ")" "done" "fi" "esac" "else" "elif"] @outdent"#
            }
            // Lua's too: `end`, and `until` for a `repeat`.
            Language::Lua => {
                r#"[(function_declaration) (function_definition) (if_statement)
                    (for_statement) (while_statement) (repeat_statement)
                    (do_statement) (table_constructor) (arguments) (parameters)] @indent
                   ["end" "}" ")" "until" "else" "elseif"] @outdent"#
            }
            // R 只有三處：`{}`、呼叫的實參、函數的形參。
            Language::R => {
                r#"[(braced_expression) (arguments) (parameters)] @indent
                   ["}" ")"] @outdent"#
            }
            Language::Json => r#"[(object) (array)] @indent
                                 ["}" "]"] @outdent"#,
            Language::Css => r#"[(block)] @indent
                                ["}"] @outdent"#,
            Language::Html => "[(element)] @indent",
            // TOML 與 YAML 的縮進是**語法本身**（一個鍵一行、靠縮進分層），
            // 多縮一級是錯的——這兩種不給查詢。
            // 匯編没有塊：標籤下面那一段縮不縮是寫稿子的人自己的事。
            Language::Toml
            | Language::Yaml
            | Language::Gas
            | Language::Nasm
            // A Typst manuscript is prose; its code is indented by hand.
            | Language::Typst
            // Ada 與 Pascal 的塊是 `begin`／`end` 這種詞，不是括號——第一期那一套
            // （`@indent` 配 `["}" "]" ")"]` 退一級）在這裏不是同一件事。留給下一期。
            | Language::Ada
            | Language::Pascal => "",
        }
    }

    /// 編譯過的那一份，和 [`Language::query`] 同一套緩存。
    fn indent_query(self) -> Option<&'static Query> {
        static CELLS: [OnceLock<Option<Query>>; Language::ALL.len()] = [const { OnceLock::new() }; Language::ALL.len()];
        let at = Language::ALL.iter().position(|&l| l == self)?;
        let text = self.indents();
        if text.is_empty() {
            return None;
        }
        CELLS[at].get_or_init(|| Query::new(&self.grammar(), text).ok()).as_ref()
    }

    fn query(self) -> Option<&'static Compiled> {
        static CELLS: [OnceLock<Option<Compiled>>; Language::ALL.len()] = [const { OnceLock::new() }; Language::ALL.len()];
        let at = Language::ALL.iter().position(|&l| l == self)?;
        CELLS[at]
            .get_or_init(|| {
                let source = format!("{}{}", self.highlights(), self.extra_highlights());
                let query = Query::new(&self.grammar(), &source).ok()?;
                let tokens = query
                    .capture_names()
                    .iter()
                    .map(|n| match self {
                        Language::Typst => typst_paint(n),
                        _ => token_of(n).map(Kind::Token),
                    })
                    .collect();
                Some((query, tokens))
            })
            .as_ref()
    }
}

/// **一個定義是哪一種**（`]f`/`]c`）。
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

/// **What the syntax tree can name for `mi`** — `mi a`/`mi c`, and `mi T`/`mi e`/`mi x`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Object {
    /// 一個參數（`mi a`）。
    Parameter,
    /// 一段註釋（`mi c`）。
    Comment,
    /// A test function (`mi T`, helix's `T`).
    Test,
    /// One key with its value, or one field (`mi e`, helix's `e`).
    Entry,
    /// An HTML element (`mi x`, helix's `x`).
    Element,
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
/// whatever else names them. A token for code; for Typst's markup the run it
/// is, a heading or an emphasis.
type Paint = Option<Kind>;

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
fn token_of(capture: &str) -> Option<Token> {
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
        ("function", "builtin") => Token::BuiltinFunction,
        ("function", _) => Token::Function,
        ("type" | "constructor" | "label", _) => Token::Type,
        ("tag", _) => Token::Tag,
        ("attribute", _) => Token::Attribute,
        ("property", _) => Token::Property,
        ("number" | "boolean", _) => Token::Number,
        ("constant", _) => Token::Constant,
        ("variable", "builtin") => Token::Builtin,
        ("variable", "parameter") => Token::Parameter,
        ("variable" | "embedded", _) => Token::Plain,
        ("operator", _) => Token::Operator,
        // Python's f-string braces, JavaScript's `${`: where a string stops
        // being a string, which is what an escape says too.
        ("punctuation", "special") => Token::Escape,
        ("punctuation", _) => Token::Punctuation,
        _ => return None,
    })
}

/// **What a Typst capture is, in this page's colours**.
///
/// The grammar names its captures as helix does (`constant.numeric.integer`,
/// `variable.other.member`, `markup.heading.2`), which [`token_of`] reads
/// for the code; markup becomes the run the block scanner would have called
/// it, so a heading is drawn as a heading, in its level's colour.
fn typst_paint(capture: &str) -> Paint {
    let kind = |token| Some(Kind::Token(token));
    match capture {
        "markup.heading.marker" => Some(Kind::HeadingMark),
        "markup.bold" => Some(Kind::Strong),
        "markup.italic" => Some(Kind::Emphasis),
        "markup.marker" => Some(Kind::Marker),
        "markup.math" => Some(Kind::Math),
        // Typst draws its maths operators in its escapes' colour (`typ-math-op`
        // and `typ-escape` are one teal in its own theme).
        "markup.math.operator" => kind(Token::Escape),
        "markup.link.url" => Some(Kind::Link),
        "markup.link.label" => Some(Kind::Ref),
        "label" => Some(Kind::Label),
        "punctuation.special" => kind(Token::Punctuation),
        _ if capture.starts_with("markup.heading") => Some(Kind::Heading),
        _ if capture.starts_with("markup.raw") => Some(Kind::Code),
        // A list's marker has its own colour on the page (`list_mark`).
        _ if capture.starts_with("markup.list") => None,
        _ if capture.starts_with("constant.numeric") => kind(Token::Number),
        // Typst's highlighter calls an escape, a shorthand and a line break
        // one thing (`Tag::Escape`).
        _ if capture.starts_with("constant.character") => kind(Token::Escape),
        _ => token_of(capture).map(Kind::Token),
    }
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

/// **這一個位置開着幾級縮進**（第一期）。
///
/// 算法照 helix 的 `indent.rs`，取它最頂上那一層：從 `byte` 那個節點往上走，每遇
/// 到一個 `@indent` 的祖先就加一級——**而且那個祖先要是在前面某一行開的**。同一
/// 行上開了又關的（`foo(1)` 整個在一行裏）不算，不然打完一行正常的調用，下一行就
/// 莫名其妙縮進去了。
///
/// Warning: **一行只算一級。** `foo(bar(` 兩個括號都在這一行開着，helix 那一支也只給
/// 一級——縮進說的是「層次」，不是「括號數」。
///
/// `closing` 是**新那一行開頭第一個非空白字符**：它要是 `}`/`]`/`)` 這種
/// `@outdent`，就退一級。這一半讓 `}` 自己回到和 `{` 那一行齊平。
pub fn open_levels(
    language: Language,
    source: &str,
    tree: &Tree,
    byte: usize,
    before_line: usize,
    closing: Option<char>,
) -> usize {
    let Some(query) = language.indent_query() else { return 0 };
    let Some(indent) = query.capture_index_for_name("indent") else { return 0 };
    let outdent = query.capture_index_for_name("outdent");
    let byte = byte.min(source.len());
    // 哪些節點是 `@indent`，哪些字是 `@outdent`。節點按 **id** 記——下面是順着
    // 祖先鏈往上走，而不是拿範圍去套。
    let mut opens: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut shutters: Vec<String> = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut hits = cursor.matches(query, tree.root_node(), source.as_bytes());
    {
        use tree_sitter::StreamingIterator as _;
        while let Some(m) = {
            hits.next();
            hits.get()
        } {
            for capture in m.captures {
                if Some(capture.index) == outdent {
                    if let Ok(text) = capture.node.utf8_text(source.as_bytes()) {
                        shutters.push(text.to_string());
                    }
                } else if capture.index == indent {
                    opens.insert(capture.node.id());
                }
            }
        }
    }
    // Warning: **順着祖先鏈走，不許拿範圍去套**（寫的時候當場撞的兩個）。
    // ① 没閉合的塊（`fn f() {` 正打到一半，而這正是按 Enter 的那一刻）**結束在
    // 檔尾**，所以 `to > byte` 是假的，一級都數不出來；② 改成 `to >= byte` 又把
    // 「光標剛好停在 `}` 後面」那一種錯數成在塊裏。祖先鏈兩種都答得對：没閉合的
    // 塊仍然罩着這個位置，而閉合完了的不再罩着。
    // **Python and Mojo are asked at the line's last character**: a block
    // there has no closing token, so at the end of its last line it has
    // already ended, and `o` there would fall back a level. A bracket closed
    // on that character is not counted below.
    let blocks_by_indent = matches!(language, Language::Python | Language::Mojo);
    let probe = match blocks_by_indent && byte > 0 && !source[..byte].ends_with('\n') {
        true => source[..byte].char_indices().next_back().map_or(byte, |(i, _)| i),
        false => byte,
    };
    let at = tree
        .root_node()
        .descendant_for_byte_range(probe, probe)
        .unwrap_or_else(|| tree.root_node());
    // **按「開在第幾行」數，一行只算一級**：`foo(bar(` 兩個括號開在同一行，helix
    // 那一支也只給一級——縮進說的是層次，不是括號數。`call_expression` 和它的
    // `arguments` 也是這麼合成一級的。
    //
    // **`before_line` 是「正在算哪一行」**——只數開在它**之前**的那些。這一格分得
    // 出兩種問法：
    //
    // - 按 Enter 要算的是**新開的那一行**，所以傳光標那一行 ＋ 1：`go(` 這種「這一
    //   行剛開的括號」算數。
    // - 拿上一行當基準的時候傳**那一行自己**：`call_expression` 就開在那一行的第一
    //   個字上，算進去的話兩頭一樣多，差就成了零——那正是這個功能整個不生效的樣子。
    let mut lines: Vec<usize> = Vec::new();
    let mut node = Some(at);
    while let Some(one) = node {
        let closed_here = blocks_by_indent
            && one.end_byte() <= byte
            && source[..one.end_byte()].ends_with([')', ']', '}']);
        if opens.contains(&one.id()) && !closed_here {
            let mut line = source[..one.start_byte()].matches('\n').count();
            // **A block of Python or Mojo is opened by its header**: the node
            // starts at its first statement, a line below the `def …:` that
            // opens it, and that first line is already indented by it.
            if matches!(language, Language::Python | Language::Mojo) && one.kind() == "block" {
                line = line.saturating_sub(1);
            }
            if line < before_line && !lines.contains(&line) {
                lines.push(line);
            }
        }
        // At the end of the header itself the block is not round the place
        // yet: it is the statement's child that starts after it.
        if matches!(language, Language::Python | Language::Mojo) {
            let mut walk = one.walk();
            for child in one.children(&mut walk) {
                if child.kind() == "block" && child.start_byte() >= byte && opens.contains(&child.id()) {
                    let line = source[..child.start_byte()].matches('\n').count().saturating_sub(1);
                    if line < before_line && !lines.contains(&line) {
                        lines.push(line);
                    }
                }
            }
        }
        node = one.parent();
    }
    let mut level = lines.len();
    // 新那一行開頭是個 `@outdent` 的字：退一級，它自己回到和開的那一行齊平。
    // 查的是**字**不是節點——那一行還没進語法樹，它正要被打出來。
    if let Some(c) = closing {
        if shutters.iter().any(|t| t.starts_with(c)) {
            level = level.saturating_sub(1);
        }
    }
    level
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
    // **A block at a time, then a byte at a time**: comparing
    // slices is a `memcmp`, comparing bytes in a loop is not, and on a 1.6 MB
    // book the loop was 0.7 ms of every keystroke.
    const BLOCK: usize = 4096;
    let shorter = a.len().min(b.len());
    let mut head = 0;
    while head + BLOCK <= shorter && a[head..head + BLOCK] == b[head..head + BLOCK] {
        head += BLOCK;
    }
    head += a[head..shorter].iter().zip(&b[head..shorter]).take_while(|(x, y)| x == y).count();
    // Warning: **數的是字節，而一個字的中間也可以對得上**。「的名字」删成「的字」：名 是 `E5 90 8D`、
    // 字 是 `E5 AD 97`，兩個都以 `E5` 開頭，於是前綴數到 4 個字節——正落在那個
    // 字裏面。不退回字的邊界的話，下面 `point` 的 `&text[..byte]` 當場 panic，而就算不 panic，
    // 交給 tree-sitter 的也是一個不存在的位置。
    //
    // **退到字的邊界上**。前綴那一段兩邊字節相同，所以邊界也相同；退一步只會讓
    // 重解析的範圍**大**一點，而上面那一段說過，大一點的答案一樣對。
    while head > 0 && !(was.is_char_boundary(head) && now.is_char_boundary(head)) {
        head -= 1;
    }
    // 從後面數，兩頭不許越過已經對上的那一段。
    let most = a.len().min(b.len()) - head;
    let mut tail = 0;
    while tail + BLOCK <= most && a[a.len() - tail - BLOCK..a.len() - tail] == b[b.len() - tail - BLOCK..b.len() - tail] {
        tail += BLOCK;
    }
    tail += (tail..most)
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

/// **解析一份源碼，樹交出去**（#423）。
///
/// `was` 是上一棵樹（`Tree::edit` 過的）：給了它就走增量，一個字的改動七百多微
/// 秒，從頭解析要五十毫秒。
///
/// Warning: **樹要有人存着。** `highlight` 每叫一次解析一次的話，滾一屏也付一次
/// 全份解析——量過 20,368 行的 `lib.rs` 是 50 毫秒。
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

    let found = captured(query, tokens, source, tree, from..to);
    paint_the_rows(found, lines, &starts, rows, &mut out);
    out
}

/// Every capture of `query` that meets `bytes`, as (start, end, pattern,
/// kind) in bytes of `source`.
fn captured(
    query: &Query,
    kinds: &[Paint],
    source: &str,
    tree: &Tree,
    bytes: std::ops::Range<usize>,
) -> Vec<(usize, usize, usize, Kind)> {
    let mut found = Vec::new();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(bytes);
    let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
    while let Some(m) = matches.next() {
        for capture in m.captures {
            if let Some(Some(kind)) = kinds.get(capture.index as usize) {
                let node = capture.node;
                found.push((node.start_byte(), node.end_byte(), m.pattern_index, *kind));
            }
        }
    }
    found
}

/// **A Typst manuscript's colours, one line at a time, nested**.
///
/// [`paint`] flattens, one token a byte, which is right for code and wrong
/// for markup: a `*strong*` word in a heading is both, and `*_both_*` is
/// what the theme's bold-italic colour is for. So each capture stays a run
/// of its own, outer before inner, and the page lays them on in that order.
/// Where two patterns name the same node the last one wins, as in [`paint`].
///
/// **A raw block is coloured by the language its tag names** — the grammar's
/// injection query finds the tag and the text, and the text is parsed on its
/// own, as a fence is.
///
/// `starts` says where each line of `source` begins ([`line_starts`]): the
/// lines are read where they lie, never copied out one by one.
pub fn paint_typst(
    source: &str,
    tree: &Tree,
    starts: &[usize],
    rows: std::ops::Range<usize>,
) -> Vec<Vec<Span>> {
    let rows = rows.start.min(starts.len())..rows.end.min(starts.len());
    let mut out = vec![Vec::new(); rows.len()];
    let Some((query, kinds)) = Language::Typst.query() else {
        return out;
    };
    if rows.is_empty() {
        return out;
    }
    let line_of = |n: usize| line_in(source, starts, n);
    let from = starts[rows.start];
    let to = starts[rows.end - 1] + line_of(rows.end - 1).len();
    let mut found = captured(query, kinds, source, tree, from..to);
    // The last pattern that names a node wins: sort the later ones first and
    // keep the first of each node.
    found.retain(|f| f.0 < f.1);
    found.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)));
    found.dedup_by_key(|f| (f.0, f.1));

    let mut next = 0usize;
    let mut live: Vec<(usize, usize, Kind)> = Vec::new();
    for n in rows.clone() {
        let line = line_of(n);
        let (head, tail) = (starts[n], starts[n] + line.len());
        while next < found.len() && found[next].0 < tail.max(head + 1) {
            let (start, end, _, kind) = found[next];
            if end > head {
                live.push((start, end, kind));
            }
            next += 1;
        }
        live.retain(|&(_, end, _)| end > head);
        if line.is_empty() {
            continue;
        }
        // Byte to char, for this line.
        let mut char_at = vec![0usize; line.len() + 1];
        for (count, (byte, c)) in line.char_indices().enumerate() {
            for slot in &mut char_at[byte..byte + c.len_utf8()] {
                *slot = count;
            }
        }
        char_at[line.len()] = line.chars().count();
        let row = &mut out[n - rows.start];
        for &(start, end, kind) in &live {
            if start >= tail {
                continue;
            }
            let (a, b) = (char_at[start.max(head) - head], char_at[end.min(tail) - head]);
            if a < b {
                row.push(Span { start: a, end: b, kind, depth: 0, construct: usize::MAX - start });
            }
        }
    }
    inject(source, tree, starts, rows, from..to, &mut out);
    out
}

/// **Whether `tree` ends inside something it could not close** — an error or
/// a missing token that runs to the last byte, as an unclosed `[`, raw block
/// or comment leaves it. A stray `)` mid-paragraph is not.
///
/// Warning: **an unclosed `/*` is no error to tree-sitter-typst**: it is a `block_comment` that runs to the end, in a tree with
/// no error in it. So a comment that ends the source is read for its `*/`s,
/// nested as Typst nests them.
pub fn open_at_end(tree: &Tree, source: &str) -> bool {
    let len = source.len();
    let root = tree.root_node();
    if len == 0 {
        return false;
    }
    let mut node = root.descendant_for_byte_range(len - 1, len);
    while let Some(here) = node {
        if here.kind() == "block_comment" {
            if !block_comment_closed(&source.as_bytes()[here.byte_range()]) {
                return true;
            }
            break;
        }
        node = here.parent();
    }
    if !root.has_error() {
        return false;
    }
    let last = root.child(root.child_count().saturating_sub(1));
    if last.is_some_and(|n| n.is_error() || n.is_missing() || (n.has_error() && n.end_byte() + 1 >= len)) {
        return true;
    }
    let mut node = root.descendant_for_byte_range(len - 1, len);
    while let Some(here) = node {
        if here.is_error() || here.is_missing() {
            return true;
        }
        node = here.parent();
    }
    false
}

/// Whether a Typst block comment's text closes every `/*` in it, its own
/// first: the pairs taken as Typst's lexer takes them, left to right.
fn block_comment_closed(text: &[u8]) -> bool {
    if !text.starts_with(b"/*") {
        return true;
    }
    let (mut deep, mut at) = (1usize, 2usize);
    while at + 1 < text.len() {
        match (text[at], text[at + 1]) {
            (b'/', b'*') => deep += 1,
            (b'*', b'/') => deep -= 1,
            _ => {
                at += 1;
                continue;
            }
        }
        at += 2;
        if deep == 0 {
            return true;
        }
    }
    false
}

/// **Where each line of `source` begins**, in bytes: one entry a line, as the
/// rope counts them — a source ending in a newline has an empty last line.
pub fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = Vec::with_capacity(source.len() / 40 + 1);
    starts.push(0);
    starts.extend(source.match_indices('\n').map(|(at, _)| at + 1));
    starts
}

/// Line `n` of `source`, without its line break.
fn line_in<'s>(source: &'s str, starts: &[usize], n: usize) -> &'s str {
    let end = starts.get(n + 1).map_or(source.len(), |&next| next);
    let line = &source[starts[n].min(end)..end];
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

/// **Compile `language`'s queries on a thread of their own**, once a process:
/// a query is built from its source text, and Typst's takes
/// milliseconds — time the first page need not wait for if the file was
/// opened a moment before it is drawn. Whoever asks for the query meanwhile
/// waits for that thread rather than compiling it twice.
pub fn warm(language: Language) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STARTED: [AtomicBool; Language::ALL.len()] = [const { AtomicBool::new(false) }; Language::ALL.len()];
    let Some(at) = Language::ALL.iter().position(|&l| l == language) else {
        return;
    };
    if STARTED[at].swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(move || {
        let _ = language.query();
        if language == Language::Typst {
            let _ = injections();
        }
    });
}

/// The grammar's injection query, compiled once, with the indices of its
/// language and content captures.
fn injections() -> Option<&'static (Query, u32, u32)> {
    static CELL: OnceLock<Option<(Query, u32, u32)>> = OnceLock::new();
    CELL.get_or_init(|| {
        let query = Query::new(&Language::Typst.grammar(), tree_sitter_typst::INJECTIONS_QUERY).ok()?;
        let language = query.capture_index_for_name("injection.language")?;
        let content = query.capture_index_for_name("injection.content")?;
        Some((query, language, content))
    })
    .as_ref()
}

/// Each raw block in `bytes` whose tag names a language this build can parse,
/// coloured by that grammar on top of the block's own runs.
///
/// Warning: **only the raw blocks** — the same query injects `comment` into
/// every comment, which helix reads for `TODO:`; the page finds those itself
/// (`comment_tags`).
fn inject(
    source: &str,
    tree: &Tree,
    starts: &[usize],
    rows: std::ops::Range<usize>,
    bytes: std::ops::Range<usize>,
    out: &mut [Vec<Span>],
) {
    let Some((query, language, content)) = injections() else {
        return;
    };
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(bytes);
    let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
    while let Some(m) = matches.next() {
        let named = |index: u32| m.captures.iter().find(|c| c.index == index).map(|c| c.node);
        let (Some(tag), Some(body)) = (named(*language), named(*content)) else {
            continue;
        };
        let Some(spoken) = tag.utf8_text(source.as_bytes()).ok().and_then(Language::from_info) else {
            continue;
        };
        let text = &source[body.start_byte()..body.end_byte()];
        let pieces: Vec<String> = text.split('\n').map(str::to_string).collect();
        let first = body.start_position().row;
        // The body starts after the tag, partway along its line.
        let indent = starts
            .get(first)
            .map_or(0, |&head| source[head..body.start_byte()].chars().count());
        for (i, runs) in highlight(spoken, &pieces).into_iter().enumerate() {
            let row = first + i;
            if !rows.contains(&row) || row >= starts.len() {
                continue;
            }
            let shift = if i == 0 { indent } else { 0 };
            out[row - rows.start].extend(runs.into_iter().map(|s| Span {
                start: s.start + shift,
                end: s.end + shift,
                ..s
            }));
        }
    }
}

fn paint_the_rows(
    mut found: Vec<(usize, usize, usize, Kind)>,
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
        .filter(|f| f.3 == Kind::Token(Token::Property))
        .map(|f| (f.0, f.1))
        .collect();
    found.retain(|f| !(f.3 == Kind::Token(Token::String) && named.contains(&(f.0, f.1))));
    found.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));

    // Warning: **A window over the captures, not a pass over them**.
    // Walking the whole of `found` for every line and skipping what does not
    // overlap is O(lines × captures), and a capture per few bytes means both
    // grow together: measured 600 lines 10 ms, 2400 lines 20 ms, **4800 lines
    // 60 ms** — 「the colours just stop」.
    //
    // `found` is sorted by where a capture starts and the lines are walked in
    // order, so one index is enough: everything that can open on this line has
    // start < `to`, and what is finished (end ≤ `from`) comes out. `live`
    // keeps `found`'s order, which **is** the painting order.
    // Warning: **跨過窗口上緣的那幾個要先收進來**，一條橫跨半個檔的塊註釋就是
    // 那一種。`found` 按起點排過，而查詢只交回**與這一窗相交**的捕獲，所以這
    // 一趟只走 `found`，不走檔子的前半。
    //
    // Warning: **別「從第 0 行掃起」**（量出來的）。那樣走的是
    // `O(窗口在第幾行)`：同一份 `lib.rs`，第 9,000 行那一塊 2.7 毫秒，第
    // 15,000 行那一塊 6.4 毫秒——愈往下愈慢，而那正是讀長檔的人待的地方。
    let mut next = 0usize;
    let mut live: Vec<(usize, usize, Kind)> = Vec::new();
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
        let mut paint: Vec<Kind> = vec![Kind::Token(Token::Plain); line.len()];
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
        let mut run: Option<(usize, Kind)> = None;
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

fn span(start: usize, end: usize, kind: Kind) -> Span {
    Span {
        start,
        end,
        kind,
        // 代碼裏没有嵌套的標記，全在頂層。
        depth: 0,
        // Every run its own construct: none of this is markup, so none of it
        // is ever hidden or opened together.
        construct: usize::MAX - start,
    }
}

/// The C++ and TypeScript highlight queries, each read on top of the one it
/// extends (see [`Language::highlights`]).
static CPP: OnceLock<String> = OnceLock::new();
static TYPESCRIPT: OnceLock<String> = OnceLock::new();

/// `base` and then `own`, joined once and kept.
fn inherited(cell: &'static OnceLock<String>, base: &str, own: &str) -> &'static str {
    cell.get_or_init(|| format!("{base}\n{own}")).as_str()
}

/// How loud a tag in a comment is: helix's four levels
/// (`runtime/queries/comment/highlights.scm`), drawn in the diagnostics'
/// colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagLevel {
    Hint,
    Info,
    Warning,
    Error,
}

/// A tag found in a comment: the name's chars, how loud it is, and the
/// `(user)` after it if there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentTag {
    pub name: std::ops::Range<usize>,
    pub level: TagLevel,
    pub user: Option<std::ops::Range<usize>>,
}

/// **The tags in a comment's text**: `TODO:`, `FIXME(ann):`,
/// a bare `NOTE`, as helix marks them through its comment grammar, with its
/// word lists. A name is a whole word in capitals, matched exactly; a
/// `(user)` before the colon is helix's `@constant`. helix's issue numbers,
/// mentions and links are left out.
pub fn comment_tags(text: &[char]) -> Vec<CommentTag> {
    const HINT: &[&str] = &["HINT", "MARK", "PASSED", "STUB", "MOCK", "TIP"];
    const INFO: &[&str] = &[
        "INFO", "NOTE", "TODO", "TO-DO", "PERF", "OPTIMIZE", "PERFORMANCE", "QUESTION", "ASK", "REVIEW", "PR", "CR",
    ];
    const WARNING: &[&str] = &["HACK", "WARN", "WARNING", "TEST", "TEMP"];
    const ERROR: &[&str] = &[
        "BUG", "FIXME", "ISSUE", "XXX", "FIX", "SAFETY", "FIXIT", "FAILED", "DEBUG", "INVARIANT", "COMPLIANCE",
        "PANIC", "SECURITY",
    ];
    let wordy = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let mut out = Vec::new();
    let mut at = 0;
    while at < text.len() {
        if !wordy(text[at]) || (at > 0 && wordy(text[at - 1])) {
            at += 1;
            continue;
        }
        let end = (at..text.len()).find(|&i| !wordy(text[i])).unwrap_or(text.len());
        let word: String = text[at..end].iter().collect();
        let level = [(HINT, TagLevel::Hint), (INFO, TagLevel::Info), (WARNING, TagLevel::Warning), (ERROR, TagLevel::Error)]
            .into_iter()
            .find(|(names, _)| names.contains(&word.as_str()))
            .map(|(_, level)| level);
        if let Some(level) = level {
            let user = (text.get(end) == Some(&'('))
                .then(|| (end + 1..text.len()).find(|&i| text[i] == ')' || text[i].is_whitespace()))
                .flatten()
                .filter(|&close| text[close] == ')' && close > end + 1 && text.get(close + 1) == Some(&':'))
                .map(|close| end..close + 1);
            out.push(CommentTag { name: at..end, level, user });
        }
        at = end;
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_comment_s_tags_are_helix_s() {
        let tags = |text: &str| -> Vec<(String, TagLevel, Option<String>)> {
            let chars: Vec<char> = text.chars().collect();
            comment_tags(&chars)
                .into_iter()
                .map(|t| {
                    let take = |r: std::ops::Range<usize>| chars[r].iter().collect::<String>();
                    (take(t.name), t.level, t.user.map(take))
                })
                .collect()
        };
        assert_eq!(tags("// TODO: 換行"), vec![("TODO".into(), TagLevel::Info, None)]);
        assert_eq!(tags("// FIXME(ann): later"), vec![("FIXME".into(), TagLevel::Error, Some("(ann)".into()))]);
        assert_eq!(tags("# HACK, WARN and NOTE"), vec![
            ("HACK".into(), TagLevel::Warning, None),
            ("WARN".into(), TagLevel::Warning, None),
            ("NOTE".into(), TagLevel::Info, None),
        ]);
        assert_eq!(tags("/* TIP */"), vec![("TIP".into(), TagLevel::Hint, None)]);
        assert!(tags("// todo: TODOS NOTED XTODO Note:").is_empty(), "whole words in capitals only");
        assert_eq!(tags("// TO-DO"), vec![("TO-DO".into(), TagLevel::Info, None)]);
        assert_eq!(tags("// BUG(a b): no"), vec![("BUG".into(), TagLevel::Error, None)], "a user has no space");
    }

    use super::*;

    /// **自己寫的那份查詢，每一種語言各釘一格**。
    ///
    /// ⚠ 這一格就是「自己寫不抄 helix」的全部代價。查詢裏的節點名是寫死的，
    /// `tree-sitter-*` 升一版把某個節點改了名，查詢**靜悄悄地不匹配**——不報錯、
    /// 不編譯失敗，`mi a` 就是没反應。所以這裏逐種語言餵一段真代碼，數它找到
    /// 幾個。**看見這一格紅，先去看那一版的 `src/node-types.json`。**
    /// **The test, entry and element queries still match** (helix's `T` `e`
    /// `x`): one sample per language, counting what each finds.
    /// A node or field renamed upstream makes a query quietly match nothing,
    /// and this is the only thing that would say so.
    #[test]
    fn the_test_entry_and_element_queries_match_their_grammars() {
        let count = |language: Language, source: &str, want: Object| -> usize {
            let tree = parse(language, source, None).expect("parses");
            objects(language, source, &tree, want).len()
        };
        let cases: &[(Language, &str, Object, usize)] = &[
            (Language::Rust, "#[test]\nfn a() {}\nfn b() {}\n#[tokio::test]\nasync fn c() {}\n", Object::Test, 2),
            (Language::Rust, "struct S { a: u8, b: u8 }\nfn f() { S { a: 1, b }; }\n", Object::Entry, 4),
            (Language::Python, "def test_a():\n    pass\ndef b():\n    pass\n", Object::Test, 1),
            (Language::Python, "d = {'a': 1, 'b': 2}\n", Object::Entry, 2),
            (Language::Go, "package p\nfunc TestA(t *T) {}\nfunc b() {}\n", Object::Test, 1),
            (Language::Go, "package p\nvar x = S{A: 1, B: 2}\n", Object::Entry, 2),
            (Language::JavaScript, "describe('a', () => { it('b', () => {}); });\nfoo(1);\n", Object::Test, 2),
            (Language::JavaScript, "const o = { a: 1, b: 2 };\n", Object::Entry, 2),
            (Language::Java, "class A { @Test void a() {} void b() {} }\n", Object::Test, 1),
            (Language::Json, "{\"a\": 1, \"b\": {\"c\": 2}}\n", Object::Entry, 3),
            (Language::Toml, "a = 1\n[t]\nb = 2\n", Object::Entry, 2),
            (Language::Yaml, "a: 1\nb:\n  c: 2\n", Object::Entry, 3),
            (Language::Html, "<p>今天<b>下雨</b></p>\n", Object::Element, 2),
        ];
        for &(language, source, want, expected) in cases {
            assert_eq!(count(language, source, want), expected, "{language:?} {want:?} in {source:?}");
        }
    }

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
            (
                Language::C,
                "// 一句註釋\nint f(int a, int b) { return a; }\n",
                2,
                1,
            ),
            (
                Language::Java,
                "// 一句註釋\nclass A { int f(int a, int b) { return a; } }\n",
                2,
                1,
            ),
            (
                Language::R,
                "# 一句註釋\nf <- function(a, b) { a }\n",
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

    /// **行尾一個 `(`，下一行開着一級**。
    ///
    /// 查過的三家寫在 [`Language::indents`] 上：vim 的 `autoindent` 不縮、
    /// `smartindent` 只認 `{`、`cindent` 和 helix 都縮。這裏照 helix。
    ///
    /// Warning: **數的是「開着幾級」，不是「這一行該縮多少」**。那一步在編輯器那一頭：
    /// 拿這個數和**上一行**的數相減，差多少就在上一行的真實縮進上加多少——helix 的
    /// `Hybrid` 就是這麼做的，它自己的註釋寫着理由：「incomplete queries, incomplete
    /// source code & differing indentation styles」。
    #[test]
    fn an_open_bracket_opens_a_level() {
        // 問的是「新開的那一行」，所以門檻是這個位置那一行 ＋ 1——同 Enter 那一刻。
        let at = |source: &str, byte: usize, closing: Option<char>| {
            let tree = parse(Language::Rust, source, None).expect("解析得了");
            let line = source[..byte.min(source.len())].matches('\n').count();
            open_levels(Language::Rust, source, &tree, byte, line + 1, closing)
        };
        // 塊裏頭：一級。
        let source = "fn f() {\n    let x = 1;\n}\n";
        let in_block = source.find("    let").expect("在") + 4;
        assert_eq!(at(source, in_block, None), 1);
        // 塊外頭：零級。
        assert_eq!(at(source, 0, None), 0);

        // **行尾一個 `(`，而括號還開着：兩級**（塊一級、調用一級）。
        let source = "fn f() {\n    go(\n        1,\n    );\n}\n";
        let after_paren = source.find("go(").expect("在") + 3;
        assert_eq!(at(source, after_paren, None), 2, "塊一級、括號一級");

        // Warning: **同一行開了又關的不算**——`go(1);` 整個在這一行裏，下一行只該回到塊
        // 那一級。
        let source = "fn f() {\n    go(1);\n}\n";
        let after_call = source.find("go(1);").expect("在") + 6;
        assert_eq!(at(source, after_call, None), 1);

        // 新那一行開頭是 `}`：退一級，它自己回到和 `{` 齊平。
        let source = "fn f() {\n    let x = 1;\n}\n";
        assert_eq!(at(source, in_block, Some('}')), 0);

        // Warning: **半截的源碼問不出東西來**（量過）：`fn f() {` 自己一份檔，
        // tree-sitter 給的是 `(source_file (ERROR …))`——**一個 block 節點都没有**，
        // 所以這一支答 0。那一刻靠的是上一行的真實縮進（見上面那一段）。
        assert_eq!(at("fn f() {\n", 9, None), 0, "半截的源碼：答不出來，不是答錯");
    }

    /// **每一門的縮進查詢都編得起來，而且真數得出級**。查詢編不起來是 `None`，
    /// **不報錯**——節點名寫錯了只是
    /// 「那一門永遠不縮」，没人會知道。
    #[test]
    fn every_indent_query_opens_a_level_where_a_bracket_opens_one() {
        let cases = [
            (Language::C, "int f(void) {\n    g(\n    );\n}\n", "g("),
            (Language::Java, "class A { void f() {\n    g(\n    ); } }\n", "g("),
            (Language::R, "f <- function() {\n  g(\n  )\n}\n", "g("),
            (Language::Go, "func f() {\n\tg(\n\t)\n}\n", "g("),
            (Language::Rust, "fn f() {\n    g(\n    );\n}\n", "g("),
            (Language::JavaScript, "function f() {\n  g(\n  )\n}\n", "g("),
            (Language::Python, "def f():\n    g(\n    )\n", "g("),
            (Language::Cpp, "template <class T> void f() {\n    g(\n    );\n}\n", "g("),
            (Language::TypeScript, "function f(): void {\n  g(\n  )\n}\n", "g("),
            (Language::Tsx, "function f() {\n  g(\n  )\n}\n", "g("),
            (Language::Bash, "if true; then\n  echo\nfi\n", "then"),
            (Language::Lua, "function f()\n  g(\n  )\nend\n", "g("),
            (Language::Mojo, "def f():\n    g(\n    )\n", "g("),
        ];
        for (language, source, after) in cases {
            assert!(language.indent_query().is_some(), "{} 的查詢編得起來", language.name());
            let tree = parse(language, source, None).expect("parses");
            let byte = source.find(after).expect("在") + after.len();
            let line = source[..byte].matches('\n').count();
            assert!(
                open_levels(language, source, &tree, byte, line + 1, None) >= 1,
                "{} ：行尾一個 `(` 至少開着一級",
                language.name(),
            );
        }
        // Mojo 1.2: a `case` line sits a level under `match`,
        // which is itself a level in from the `def`.
        let source = "def f(x: Int):\n    match x:\n        case 1:\n            pass\n";
        let tree = parse(Language::Mojo, source, None).expect("parses");
        let byte = source.find("match x:").expect("在") + "match x:".len();
        assert_eq!(open_levels(Language::Mojo, source, &tree, byte, 2, None), 2);
    }

    /// **`.s` 與 `.S` 不是同一門語言**。
    ///
    /// 分家分的是 `空格 c` 寫哪一個記號。其餘副檔名照舊不分大小寫。
    #[test]
    fn the_one_extension_whose_case_decides_which_language_it_is() {
        use crate::comment::{marks, Marks};
        use crate::syntax::Syntax;
        assert_eq!(Language::from_extension("s"), Some(Language::Gas));
        assert_eq!(Language::from_extension("S"), Some(Language::Nasm));
        assert_eq!(Language::from_extension("asm"), Some(Language::Nasm));
        // 別的副檔名大寫小寫都是同一門。
        assert_eq!(Language::from_extension("RS"), Some(Language::Rust));
        assert_eq!(Language::from_extension("R"), Some(Language::R));
        assert_eq!(Language::from_extension("Go"), Some(Language::Go));
        // 而分家是為了這一格：
        assert_eq!(marks(Syntax::Code(Language::Gas)).line, Some("#"));
        assert_eq!(marks(Syntax::Code(Language::Nasm)).line, Some(";"));
        assert_eq!(
            marks(Syntax::Code(Language::Nasm)),
            Marks { line: Some(";"), block: None },
            "NASM 没有塊註釋"
        );
    }

    /// **縮進是語法本身的那兩種不給查詢**（TOML／YAML）。
    #[test]
    fn where_indentation_is_the_syntax_nothing_is_added() {
        assert_eq!(Language::Toml.indents(), "");
        assert_eq!(Language::Yaml.indents(), "");
    }

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

    /// Warning: **說出編不過的那一句話**，不只印語言的名字——那是最没用的一半：手寫的查詢（Ada、Pascal）和追加的那幾條
    /// （[`Language::extra_highlights`]）都是人寫的，寫壞的時候要知道壞在哪一個節點
    /// 名上（`(declArg type: (typeref) …)` 裏那一格收的是 `type`，只說「pascal」
    /// 是找不到的）。
    #[test]
    fn every_shipped_grammar_compiles_its_own_query() {
        for language in Language::ALL {
            let source = format!("{}{}", language.highlights(), language.extra_highlights());
            if let Err(why) = tree_sitter::Query::new(&language.grammar(), &source) {
                panic!("{}: {why}", language.name());
            }
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
            // `rust` 認得出來；「這個 build 認不出來」的例子要用一種真的認不出來的。
            ("rust", Some(Language::Rust)),
            ("rs", Some(Language::Rust)),
            ("ts", Some(Language::TypeScript)),
            ("tsx", Some(Language::Tsx)),
            ("c++", Some(Language::Cpp)),
            ("sh", Some(Language::Bash)),
            ("lua", Some(Language::Lua)),
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
        assert_eq!((one.start, one.kind), (26, Kind::Token(Token::Number)), "{:?}", got[2]);
    }

    /// **我們自己寫的那兩份查詢真的會匹配**。
    ///
    /// Warning: **「編得起來」與「匹配得上」是兩件事。**`Query::new` 只檢查節點名
    /// 存不存在；寫對了名字卻搭錯了形狀，查詢照樣編得過、一個字也不上色。
    #[test]
    fn the_three_queries_we_wrote_ourselves_really_paint() {
        let got = highlight(
            Language::Ada,
            &lines("procedure Hello is\n   -- 打個招呼\nbegin\n   Put_Line (\"你好\");\nend Hello;"),
        );
        assert_eq!(at(&got, 0, 0), Some(Token::Keyword), "procedure：{:?}", got[0]);
        assert_eq!(at(&got, 0, 10), Some(Token::Function), "子程序的名字：{:?}", got[0]);
        assert_eq!(at(&got, 1, 3), Some(Token::Comment), "{:?}", got[1]);
        assert_eq!(at(&got, 2, 0), Some(Token::Keyword), "begin：{:?}", got[2]);
        assert_eq!(at(&got, 3, 14), Some(Token::String), "{:?}", got[3]);

        let got = highlight(
            Language::Pascal,
            &lines("program Hello;\n// 打個招呼\nbegin\n  WriteLn('你好');\nend."),
        );
        assert_eq!(at(&got, 0, 0), Some(Token::Keyword), "program：{:?}", got[0]);
        assert_eq!(at(&got, 1, 0), Some(Token::Comment), "{:?}", got[1]);
        assert_eq!(at(&got, 2, 0), Some(Token::Keyword), "begin：{:?}", got[2]);
        assert_eq!(at(&got, 3, 10), Some(Token::String), "{:?}", got[3]);

        // Mojo。`fn`／`struct` 是關鍵詞，名字是函數與類型，`#` 起
        // 的那一行是註釋，三引號那一種**不是**——它是字符串。
        let got = highlight(
            Language::Mojo,
            &lines(
                "# 打個招呼\nstruct Greeter:\n    fn hello(self, name: String) -> None:\n        print(\"你好\")",
            ),
        );
        assert_eq!(at(&got, 0, 0), Some(Token::Comment), "{:?}", got[0]);
        assert_eq!(at(&got, 1, 0), Some(Token::Keyword), "struct：{:?}", got[1]);
        assert_eq!(at(&got, 1, 7), Some(Token::Type), "結構的名字：{:?}", got[1]);
        assert_eq!(at(&got, 2, 4), Some(Token::Keyword), "fn：{:?}", got[2]);
        assert_eq!(at(&got, 2, 7), Some(Token::Function), "函數的名字：{:?}", got[2]);
        assert_eq!(at(&got, 2, 25), Some(Token::Type), "參數的類型：{:?}", got[2]);
        assert_eq!(at(&got, 3, 8), Some(Token::Function), "print：{:?}", got[3]);
        assert_eq!(at(&got, 3, 15), Some(Token::String), "{:?}", got[3]);

        // Mojo 1.2: bare `match`, `__enum` and its cases.
        let got = highlight(
            Language::Mojo,
            &lines("__enum Shape:\n    case circle(Float64)\ndef f(s: Shape):\n    match s:\n        case circle(r):\n            pass"),
        );
        assert_eq!(at(&got, 0, 0), Some(Token::Keyword), "__enum：{:?}", got[0]);
        assert_eq!(at(&got, 0, 7), Some(Token::Type), "the enum's name：{:?}", got[0]);
        assert_eq!(at(&got, 3, 4), Some(Token::Keyword), "match：{:?}", got[3]);
        assert_eq!(at(&got, 4, 8), Some(Token::Keyword), "case：{:?}", got[4]);
    }

    /// The two token kinds the borrowed themes added come out of
    /// real grammars, not only out of tests that build them by hand: a theme
    /// that colours a built-in or a parameter needs the grammar to say which.
    #[test]
    fn a_builtin_and_a_parameter_are_told_from_their_neighbours() {
        let got = highlight(Language::Python, &lines("print(x)"));
        assert_eq!(at(&got, 0, 0), Some(Token::BuiltinFunction), "{:?}", got[0]);
        let got = highlight(Language::Rust, &lines("fn f(a: u8) {}"));
        assert_eq!(at(&got, 0, 5), Some(Token::Parameter), "{:?}", got[0]);
    }

    #[test]
    fn a_json_key_is_a_name_and_its_value_is_a_string() {
        let got = highlight(Language::Json, &lines("{\"名\": \"值\", \"n\": 3}"));
        assert_eq!(at(&got, 0, 1), Some(Token::Property), "{:?}", got[0]);
        assert_eq!(at(&got, 0, 6), Some(Token::String), "{:?}", got[0]);
        assert_eq!(at(&got, 0, 16), Some(Token::Number), "{:?}", got[0]);
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
    /// **掐頭去尾不許切進一個字裏**（按 `d` 删到「名」當場崩的那一
    /// 下，`code.rs:451` 的 `&text[..byte]`）。
    ///
    /// 漢字在 UTF-8 裏三個字節，開頭常常一樣：名 `E5 90 8D`、字 `E5 AD 97`。
    /// 所以「相同的字節數」會停在一個字的中間，而那不是一個位置。
    #[test]
    fn an_edit_never_starts_or_ends_inside_a_character() {
        // 「的名字」删成「的字」，前綴數到 4 個字節。
        let edit = what_changed("的名字", "的字").expect("they differ");
        assert!("的名字".is_char_boundary(edit.start_byte), "start");
        assert!("的名字".is_char_boundary(edit.old_end_byte), "old end");
        assert!("的字".is_char_boundary(edit.new_end_byte), "new end");

        // 同一族：尾巴那一頭也會落在字裏（髮 `E9 AB BC`、髫 `E9 AB AB`）。
        let edit = what_changed("一髮絲", "一髫絲").expect("they differ");
        assert!("一髮絲".is_char_boundary(edit.start_byte));
        assert!("一髮絲".is_char_boundary(edit.old_end_byte));
        assert!("一髫絲".is_char_boundary(edit.new_end_byte));

        // 整份稿子逐字删一遍，一次都不許切錯——按住 `d` 就是這麼走的。
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

    /// The kind of the innermost run over char `at` of row `row` — the one
    /// the page lays on last.
    fn typst_kind_at(painted: &[Vec<Span>], lines: &[String], row: usize, needle: &str) -> Option<Kind> {
        let byte = lines[row].find(needle).unwrap_or_else(|| panic!("{needle:?} on row {row}"));
        let at = lines[row][..byte].chars().count();
        painted[row].iter().rfind(|s| s.start <= at && at < s.end).map(|s| s.kind)
    }

    /// **A Typst manuscript is coloured by its grammar**: code
    /// as code, markup as the runs the page already draws, the `#` in the
    /// colour of what follows it, and a raw block in its tag's language.
    #[test]
    fn typst_paints_its_code_and_its_markup() {
        let text = concat!(
            "= 第一章 *重* <ch1>\n",
            "#let price = 42 // 每斤\n",
            "#set page(paper: \"a5\", margin: 2cm)\n",
            "那年*冬天 _很_ 冷*，見 @ch1 與 $x^2 + pi$。#emph[雪] #title\n",
            "```rust\n",
            "fn main() { let s = \"字\"; }\n",
            "```\n",
            "#show heading: it => it.body\n",
        );
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let source = joined(&lines);
        let tree = parse(Language::Typst, &source, None).expect("parses");
        let painted = paint_typst(&source, &tree, &line_starts(&source), 0..lines.len());
        let at = |row: usize, needle: &str| typst_kind_at(&painted, &lines, row, needle);
        let token = |t: Token| Some(Kind::Token(t));
        assert_eq!(at(0, "="), Some(Kind::HeadingMark));
        assert_eq!(at(0, "第一章"), Some(Kind::Heading));
        assert_eq!(at(0, "重"), Some(Kind::Strong), "strong inside the heading");
        assert!(painted[0].iter().any(|s| s.kind == Kind::Heading && s.start == 0), "and the heading under it");
        assert_eq!(at(0, "<ch1>"), Some(Kind::Label));
        assert_eq!(at(1, "#let"), token(Token::Keyword), "the `#` takes its keyword's colour");
        assert_eq!(at(1, "let"), token(Token::Keyword));
        assert_eq!(at(1, "price"), token(Token::Plain), "a value, not a function");
        assert_eq!(at(1, "42"), token(Token::Number));
        assert_eq!(at(1, "// 每斤"), token(Token::Comment));
        assert_eq!(at(2, "page"), token(Token::Function));
        assert_eq!(at(2, "paper"), token(Token::Parameter));
        assert_eq!(at(2, "\"a5\""), token(Token::String));
        assert_eq!(at(2, "2cm"), token(Token::Number));
        assert_eq!(at(3, "*冬"), Some(Kind::Marker), "strong's star is a marker");
        assert_eq!(at(3, "冬天"), Some(Kind::Strong));
        assert_eq!(at(3, "很"), Some(Kind::Emphasis), "emphasis nests inside strong");
        assert_eq!(at(3, "@ch1"), Some(Kind::Ref));
        assert_eq!(at(3, "$x"), Some(Kind::Math));
        assert_eq!(at(3, "2 +"), token(Token::Plain), "a number in maths is text, as Typst draws it");
        assert_eq!(at(3, "^"), token(Token::Escape), "and `^` an operator");
        assert_eq!(at(3, "pi"), token(Token::Function), "a symbol is interpolated, drawn as a call");
        assert_eq!(at(3, "#emph"), token(Token::Function), "a call's `#` is the call's");
        assert_eq!(at(3, "#title"), token(Token::Function), "and so is a name's");
        assert_eq!(at(3, "年"), None, "prose is the page's own ink");
        assert_eq!(at(4, "rust"), Some(Kind::Code), "the tag is the raw block's");
        assert_eq!(at(5, "fn"), token(Token::Keyword), "the block is Rust");
        assert_eq!(at(5, "main"), token(Token::Function));
        assert_eq!(at(5, "\"字\""), token(Token::String));
        assert_eq!(at(7, "heading"), token(Token::Function), "a show rule's selector");
        assert_eq!(at(7, "body"), token(Token::Plain), "a field in its object's colour");
    }

    /// **Where Typst's own highlighter is followed** (typst-syntax 0.15,
    /// `highlight`; review), its interpolated colour drawn as a
    /// call's: the `#` before a field access, a string, a number or a
    /// keyword value takes what follows; the names of a field access after
    /// `#` are interpolated; `none`, `auto`, `true` and `false` are
    /// keywords; in maths a symbol is interpolated, `+`, `=` and brackets
    /// are text, and only the brackets Typst takes away are punctuation.
    #[test]
    fn typst_is_coloured_as_its_own_highlighter_colours_it() {
        let text = concat!(
            "#show heading: it => [#it.body]\n",
            "#calc.pow(2, 3) #\"s\" #true #none #auto #12pt\n",
            "#{ let a = none; let b = it.body; a }\n",
            "$x^2 + y_(i=0) = (a+b)/2 + n(n+1) + sqrt(c) + pi + sum$\n",
            "#f(x: true, y: auto)\n",
        );
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let source = joined(&lines);
        let tree = parse(Language::Typst, &source, None).expect("parses");
        let painted = paint_typst(&source, &tree, &line_starts(&source), 0..lines.len());
        let at = |row: usize, needle: &str| typst_kind_at(&painted, &lines, row, needle);
        let token = |t: Token| Some(Kind::Token(t));
        for (row, needle, want) in [
            (0, "#it", Token::Function),
            (0, "it.body]", Token::Function),
            (0, "body]", Token::Function),
            (0, ".body]", Token::Punctuation),
            (1, "#calc", Token::Function),
            (1, "calc", Token::Function),
            (1, "pow", Token::Function),
            (1, "#\"s\"", Token::String),
            (1, "#true", Token::Keyword),
            (1, "true", Token::Keyword),
            (1, "#none", Token::Keyword),
            (1, "#auto", Token::Keyword),
            (1, "#12pt", Token::Number),
            (2, "none;", Token::Keyword),
            (2, "it.body;", Token::Plain),
            (2, "body;", Token::Plain),
            (3, "^", Token::Escape),
            (3, "+ y", Token::Plain),
            (3, "= (", Token::Plain),
            (3, "(i=0)", Token::Punctuation),
            (3, "(a+b)", Token::Punctuation),
            (3, "/2", Token::Escape),
            (3, "(n+1)", Token::Plain),
            (3, "sqrt", Token::Function),
            (3, "(c)", Token::Punctuation),
            (3, "pi", Token::Function),
            (3, "sum", Token::Function),
            (4, "true,", Token::Keyword),
            (4, "auto)", Token::Keyword),
        ] {
            assert_eq!(at(row, needle), token(want), "{needle:?} on row {row}");
        }
    }

    /// The window the page asks for paints what the whole file would, and an
    /// edited tree what a fresh one would — Typst's runs nest, so this is
    /// asked of them apart from [`a_window_paints_exactly_what_the_whole_file_would`].
    #[test]
    fn a_typst_window_and_an_edited_tree_paint_what_the_whole_would() {
        let mut text = String::new();
        for n in 0..300 {
            text.push_str(&format!(
                "= 第{n}章\n\n那年*冬天*很冷，#emph[雪]下得早。$a_{n}$\n\n#let x{n} = (a: 1, b: \"字\")\n/* 一段\n註 */\n"
            ));
        }
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let source = joined(&lines);
        let tree = parse(Language::Typst, &source, None).expect("parses");
        let starts = line_starts(&source);
        let whole = paint_typst(&source, &tree, &starts, 0..lines.len());
        for from in [0usize, 5, 127, 128, 1000, 2047] {
            let rows = from..(from + 128).min(lines.len());
            let win = paint_typst(&source, &tree, &starts, rows.clone());
            assert_eq!(win, whole[rows.clone()].to_vec(), "the window from row {from}");
        }
        // Make a word strong in the middle, as a keystroke would.
        let mut after_lines = lines.clone();
        let row = 1402;
        after_lines[row] = after_lines[row].replacen("很冷", "*很冷*", 1);
        let after = joined(&after_lines);
        let edit = what_changed(&source, &after).expect("it changed");
        let mut edited = tree.clone();
        edited.edit(&edit);
        let again = parse(Language::Typst, &after, Some(&edited)).expect("reparses");
        let fresh = parse(Language::Typst, &after, None).expect("parses");
        let rows = 1280..1408;
        let after_starts = line_starts(&after);
        assert_eq!(
            paint_typst(&after, &again, &after_starts, rows.clone()),
            paint_typst(&after, &fresh, &after_starts, rows),
        );
        let painted = paint_typst(&after, &again, &after_starts, row..row + 1);
        assert!(painted[0].iter().any(|s| s.kind == Kind::Strong && s.end - s.start == 4), "{:?}", painted[0]);
    }

    /// **What the grammar costs on a long book** — a stopwatch, not a test.
    ///
    /// ```text
    /// YUMETE_TYPST_BENCH=book.typ cargo test -p yumete-core --release --lib \
    ///     what_typst_parsing_costs -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn what_typst_parsing_costs() {
        use std::time::Instant;
        let Ok(file) = std::env::var("YUMETE_TYPST_BENCH") else {
            return;
        };
        let text = std::fs::read_to_string(file).unwrap();
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let source = joined(&lines);
        let began = Instant::now();
        let tree = parse(Language::Typst, &source, None).unwrap();
        println!("\nwhole parse {:?} ({} bytes, {} lines)", began.elapsed(), source.len(), lines.len());
        let middle = lines.len() / 2;
        for round in ["first, with the queries compiled", "again"] {
            let began = Instant::now();
            let _ = paint_typst(&source, &tree, &line_starts(&source), middle..middle + 128);
            println!("paint 128 rows ({round}) {:?}", began.elapsed());
        }
        for (share, typed) in [(0.1, "字"), (0.5, "字"), (0.9, "字"), (0.5, "#emph[字]")] {
            let at = (source.len() as f64 * share) as usize;
            let at = (at..source.len()).find(|&i| source.is_char_boundary(i)).unwrap();
            let mut after = source.clone();
            after.insert_str(at, typed);
            let began = Instant::now();
            let edit = what_changed(&source, &after).unwrap();
            let diffed = began.elapsed();
            let mut edited = tree.clone();
            edited.edit(&edit);
            let began = Instant::now();
            let _ = parse(Language::Typst, &after, Some(&edited)).unwrap();
            println!("{typed:?} at {share}: found by diffing {diffed:?}, reparsed {:?}", began.elapsed());
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
            assert_ne!(after, before, "{what}：這一改没改動任何東西");

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

    /// 一模一樣的兩份，`what_changed` 要說「没改」——不然每一幀白解析一次。
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
        // Warning: **出廠那個路徑要從倉根算起，不是從當前目録**：
        // `cargo test` 跑在**這個 crate 的目録**裏，相對路徑於是找不到檔，一跑就
        // `NotFound`。帶 `#[ignore]` 的不進閘，所以没人發現。
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
