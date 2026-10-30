//! Table editing — Feature #118.
//!
//! A CSV is a text file, and yumete edits text files; what it is not is a
//! *grid*. Twenty-eight columns of 拆分 read as one run-on line, the columns
//! do not line up, and moving to "the fourth field of this row" means counting
//! commas by eye. This module is the grid: a view over the same text, in which
//! the unit of movement is the cell.
//!
//! **The text is the truth.** Cells are never a parallel copy of the document
//! that has to be written back; they are ranges into the line, computed when a
//! row is looked at and thrown away after. Editing a cell edits the buffer at
//! that range, so everything the editor already knows how to do — undo, the
//! IME, autosave, recovery — keeps working, and a byte nobody touched goes out
//! exactly as it came in.
//!
//! **The table's own knowledge lives in a schema, not here.** Which columns a
//! file has, what they are called, which of them is a foreign key into another
//! row: all of it comes from a TOML file next to the data. Nothing in this
//! module knows what 拆分 is.
//!
//! ## The comma
//!
//! The first table this serves has no quoting at all: a cell is what lies
//! between two commas, and that is the whole rule. Parsing is `split(',')` and
//! writing is a no-op, which is why a hand-edited 8 MB file survives a round
//! trip byte for byte. The price is that **no cell may contain the delimiter**
//! — so the editor refuses to type one rather than letting a file quietly
//! shift every column right of the damage. `quoting = "minimal"` is left
//! deliberately unimplemented: it would make every read slower to buy a
//! freedom this data does not want.

use std::path::{Path, PathBuf};

use serde::Deserialize;

pub mod layer;

/// How a table's columns are told apart (Feature #157).
///
/// Three answers, because the right one depends on the table and not on the
/// editor: twenty-eight columns of one character read as a grid and want a
/// rule between them; six wide ones read as a page, where a rule between every
/// column is noise between the words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rules {
    /// Nothing between them — the columns are told apart by their alignment,
    /// which is how a printed table does it.
    Off,
    /// A band: every column sits a shade off the page, and the page shows in
    /// the seam between them.
    Colour,
    /// A drawn line, in one of three strokes.
    Line(Stroke),
}

impl Default for Rules {
    /// A dashed line. A drawn rule is what makes twenty-eight one-character
    /// columns readable as a grid, and *dashed* is the one that does it
    /// without becoming the loudest thing on the page — the eye takes it as a
    /// boundary rather than as a column of its own.
    fn default() -> Rules {
        Rules::Line(Stroke::Dash)
    }
}

/// Which line is drawn between two columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stroke {
    Solid,
    Dash,
    Double,
}

impl Stroke {
    /// The character it is drawn with.
    pub fn glyph(self) -> &'static str {
        match self {
            Stroke::Solid => "│",
            Stroke::Dash => "┆",
            Stroke::Double => "║",
        }
    }
}

impl Rules {
    /// Read a config value or the words of a command: `off`, `color`, `line`,
    /// `line dash`, `line double`.
    pub fn parse(value: &str) -> Option<Rules> {
        let mut words = value.split_whitespace();
        let rules = match words.next()? {
            "off" | "none" | "無" | "无" => Rules::Off,
            "colour" | "color" | "底色" => Rules::Colour,
            "line" | "線" | "线" => match words.next() {
                None | Some("solid") => Rules::Line(Stroke::Solid),
                Some("dash" | "dashed") => Rules::Line(Stroke::Dash),
                Some("double") => Rules::Line(Stroke::Double),
                Some(_) => return None,
            },
            "solid" => Rules::Line(Stroke::Solid),
            "dash" | "dashed" => Rules::Line(Stroke::Dash),
            "double" => Rules::Line(Stroke::Double),
            _ => return None,
        };
        Some(rules)
    }

    /// How it is written in a config file, and said on the status line.
    pub fn name(self) -> &'static str {
        match self {
            Rules::Off => "off",
            Rules::Colour => "color",
            Rules::Line(Stroke::Solid) => "line",
            Rules::Line(Stroke::Dash) => "line dash",
            Rules::Line(Stroke::Double) => "line double",
        }
    }
}

/// What a column holds. Generic on purpose — the first table's twenty-eight
/// columns happen to be all strings, and the next one will not be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub enum Kind {
    #[default]
    String,
    Int,
    Float,
    Bool,
}

/// One column of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// Its name in the header row, and the name a `compute` refers to it by.
    pub name: String,
    /// What to show in the header, when that is not the name itself.
    pub label: Option<String>,
    /// What it holds.
    pub kind: Kind,
    /// Whether the grid draws it and movement stops in it.
    ///
    /// It is still **read and written**: a hidden column is a column the file
    /// has and this reader is not interested in. Two of the 拆分表's
    /// twenty-eight are empty in all 123,380 rows and cost eight cells each
    /// across the whole page.
    pub hidden: bool,
}

impl Column {
    /// What the header shows for it.
    pub fn heading(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Schema {
    /// The file names this schema is for.
    pub files: Vec<String>,
    /// The column that names a row — what the detail panel titles it by, and
    /// where a link lands.
    pub key: Option<String>,
    /// What separates two cells.
    pub delimiter: char,
    /// Whether the first line names the columns.
    pub header: bool,
    pub columns: Vec<Column>,
}

/// The file a table's description lives in, beside the data: a Frictionless
/// Data Package (2026-10-30, as decided). One per folder, listing each table
/// file as a resource with its `schema` (fields, `primaryKey`) and `dialect`
/// (delimiter, header). Nothing of yumete's own goes in it.
pub const DATAPACKAGE: &str = "datapackage.json";

impl Schema {
    /// This file's description out of a `datapackage.json`: the resource whose
    /// `path` is `file`, or `None` when the package does not list it.
    ///
    /// Read loosely, as the standard asks: a package another tool wrote may
    /// carry fields this editor has no use for, and they are left alone.
    /// Only what would misread the file is an error — a delimiter that is not
    /// one character, a `primaryKey` that names no field.
    pub fn from_datapackage(text: &str, file: &str) -> Result<Option<Schema>, String> {
        let package: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("{DATAPACKAGE}: {e}"))?;
        let Some(resources) = package.get("resources").and_then(|r| r.as_array()) else {
            return Ok(None);
        };
        let Some(resource) = resources.iter().find(|r| {
            r.get("path").and_then(|p| p.as_str()).is_some_and(|p| {
                p == file || Path::new(p).file_name().is_some_and(|n| n == file)
            })
        }) else {
            return Ok(None);
        };
        let dialect = resource.get("dialect");
        let delimiter = match dialect.and_then(|d| d.get("delimiter")).and_then(|d| d.as_str()) {
            None => ',',
            Some(d) if d.chars().count() == 1 && !"\n\r".contains(d) => d.chars().next().unwrap(),
            Some(d) if "\n\r".contains(d) => {
                return Err("a line break already separates rows, not cells".to_string())
            }
            Some(d) => return Err(format!("delimiter must be one character, not '{d}'")),
        };
        let header = dialect
            .and_then(|d| d.get("header"))
            .and_then(|h| h.as_bool())
            .unwrap_or(true);
        let fields = resource
            .get("schema")
            .and_then(|s| s.get("fields"))
            .and_then(|f| f.as_array());
        let columns: Vec<Column> = fields
            .map(|fields| {
                fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| {
                        let name = f
                            .get("name")
                            .and_then(|n| n.as_str())
                            .filter(|n| !n.trim().is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("{}", i + 1));
                        Column {
                            name,
                            label: f.get("title").and_then(|t| t.as_str()).map(str::to_string),
                            kind: match f.get("type").and_then(|t| t.as_str()) {
                                Some("integer") => Kind::Int,
                                Some("number") => Kind::Float,
                                Some("boolean") => Kind::Bool,
                                _ => Kind::String,
                            },
                            hidden: false,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if columns.is_empty() {
            return Err(format!("{file}: a table with no fields"));
        }
        // `primaryKey` is a field name or a list of them; a row here is named
        // by one field, so the first names the row.
        let key = match resource.get("schema").and_then(|s| s.get("primaryKey")) {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(k)) => Some(k.clone()),
            Some(serde_json::Value::Array(list)) => list.first().and_then(|k| k.as_str()).map(str::to_string),
            Some(other) => return Err(format!("primaryKey must name a field, not {other}")),
        };
        if let Some(key) = &key {
            if !columns.iter().any(|c| &c.name == key) {
                return Err(format!("primaryKey = '{key}' is not a field"));
            }
        }
        Ok(Some(Schema {
            files: vec![file.to_string()],
            key,
            delimiter,
            header,
            columns,
        }))
    }

    pub fn numbered(columns: usize, delimiter: char) -> Schema {
        Schema {
            files: Vec::new(),
            key: None,
            delimiter,
            header: false,
            columns: (0..columns)
                .map(|i| Column {
                    name: format!("{}", i + 1),
                    label: None,
                    kind: Kind::String,
                    hidden: false,
                })
                .collect(),
        }
    }

    /// Make a bare schema out of a file's own header row.
    ///
    /// What `yumete -t` falls back on: every CSV already says what its columns
    /// are on its first line, so a file nobody has written a schema for can
    /// still be read as a grid. It gets the names and nothing else — no
    /// labels, no computed fields, no links, because those are knowledge about
    /// the data that only a person has.
    pub fn from_header(line: &str, delimiter: char) -> Schema {
        let columns: Vec<Column> = cells(line, delimiter)
            .into_iter()
            .enumerate()
            .map(|(i, span)| {
                let name = cell_text(line, span);
                Column {
                    // A header cell that is blank still needs a name, or two of
                    // them would be the same column.
                    name: if name.trim().is_empty() {
                        format!("{}", i + 1)
                    } else {
                        name
                    },
                    label: None,
                    kind: Kind::String,
                    hidden: false,
                }
            })
            .collect();
        Schema {
            files: Vec::new(),
            key: None,
            delimiter,
            header: true,
            columns,
        }
    }

    /// Whether this schema is for a file with this name.
    pub fn covers(&self, path: &Path) -> bool {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        self.files.iter().any(|f| f == name)
    }

    /// Which column has this name.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }

    /// Whether column `i` is drawn and stopped in.
    ///
    /// A column the schema does not know about — a ragged row's extra field —
    /// is always shown: hiding damage is not what hiding is for.
    pub fn shows(&self, i: usize) -> bool {
        self.columns.get(i).is_none_or(|c| !c.hidden)
    }

    /// Whether any column is hidden at all, so the common case costs nothing.
    pub fn hides_anything(&self) -> bool {
        self.columns.iter().any(|c| c.hidden)
    }
}

/// `codepoint(char)`, `block(char)`, `range_label(char, cjk_blocks)`.
pub fn schema_for(path: &Path) -> Option<(PathBuf, Schema)> {
    schema_for_reporting(path).0
}

/// The same, and what went wrong with the schemas that did not work.
///
/// A schema with a typo in it used to be dropped on the floor: the file opened
/// with the twenty-eight labels, both computed fields and the whole link
/// silently missing, and the only clue was that the status line said 「照首行」
/// where it should have said the schema's name. The parser has a good message
/// for every one of these; this is how it reaches a person.
pub fn schema_for_reporting(path: &Path) -> (Option<(PathBuf, Schema)>, Vec<String>) {
    let mut problems = Vec::new();
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return (None, problems);
    };
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let package = dir.join(DATAPACKAGE);
    let found = match std::fs::read_to_string(&package) {
        Ok(text) => match Schema::from_datapackage(&text, name) {
            Ok(Some(schema)) => Some((package, schema)),
            Ok(None) => None,
            Err(why) => {
                problems.push(why);
                None
            }
        },
        Err(_) => None,
    };
    // **The old format is no longer read, and says so once** (2026-10-30):
    // a `.yumete/tables/*.toml` up the tree that names this file would be
    // silently ignored otherwise, and its writer would see the header row
    // and wonder where their column names went.
    if found.is_none() {
        let mut up = Some(dir);
        while let Some(d) = up {
            if let Ok(entries) = std::fs::read_dir(d.join(".yumete").join("tables")) {
                for entry in entries.flatten() {
                    let file = entry.path();
                    if file.extension().is_some_and(|e| e == "toml")
                        && std::fs::read_to_string(&file).is_ok_and(|t| t.contains(name))
                    {
                        problems.push(format!(
                            "{}: the .yumete/tables schema is no longer read; write a {DATAPACKAGE} beside the data with :table-schema write",
                            file.display()
                        ));
                    }
                }
            }
            up = d.parent();
        }
    }
    (found, problems)
}

/// The `datapackage.json` for this folder with `file` described as `schema`
/// reads it: the package as it stands with this one resource replaced or
/// added, or a new package when there is none. Pretty-printed, two spaces.
///
/// Only what differs from the standard's defaults is written into the
/// dialect, so a comma-separated file with a header says nothing about
/// either. The package's `name` and `$schema` are what other tools require
/// of one; they are written when missing and never changed when present.
pub fn datapackage_with(existing: Option<&str>, folder: &str, file: &str, schema: &Schema) -> Result<String, String> {
    use serde_json::{json, Map, Value};
    let mut package: Value = match existing {
        Some(text) => serde_json::from_str(text).map_err(|e| format!("{DATAPACKAGE}: {e}"))?,
        None => json!({}),
    };
    if !package.is_object() {
        return Err(format!("{DATAPACKAGE}: not an object"));
    }
    let slug = |text: &str| -> String {
        let mut out: String = text
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        while out.contains("--") {
            out = out.replace("--", "-");
        }
        let out = out.trim_matches('-').to_string();
        match out.is_empty() {
            true => "table".to_string(),
            false => out,
        }
    };
    let object = package.as_object_mut().expect("an object");
    object
        .entry("$schema")
        .or_insert_with(|| json!("https://datapackage.org/profiles/2.0/datapackage.json"));
    object.entry("name").or_insert_with(|| json!(slug(folder)));
    let mut fields = Vec::new();
    for column in &schema.columns {
        let mut field = Map::new();
        field.insert("name".into(), json!(column.name));
        if let Some(label) = &column.label {
            field.insert("title".into(), json!(label));
        }
        let kind = match column.kind {
            Kind::String => None,
            Kind::Int => Some("integer"),
            Kind::Float => Some("number"),
            Kind::Bool => Some("boolean"),
        };
        if let Some(kind) = kind {
            field.insert("type".into(), json!(kind));
        }
        fields.push(Value::Object(field));
    }
    let mut table = Map::new();
    table.insert("fields".into(), Value::Array(fields));
    if let Some(key) = &schema.key {
        table.insert("primaryKey".into(), json!(key));
    }
    let mut dialect = Map::new();
    if schema.delimiter != ',' {
        dialect.insert("delimiter".into(), json!(schema.delimiter.to_string()));
    }
    if !schema.header {
        dialect.insert("header".into(), json!(false));
    }
    let stem = Path::new(file).file_stem().and_then(|s| s.to_str()).unwrap_or(file);
    let mut resource = Map::new();
    resource.insert("name".into(), json!(slug(stem)));
    resource.insert("path".into(), json!(file));
    if !dialect.is_empty() {
        resource.insert("dialect".into(), Value::Object(dialect));
    }
    resource.insert("schema".into(), Value::Object(table));
    let resource = Value::Object(resource);
    let resources = object
        .entry("resources")
        .or_insert_with(|| json!([]));
    let Some(list) = resources.as_array_mut() else {
        return Err(format!("{DATAPACKAGE}: resources is not a list"));
    };
    match list.iter().position(|r| r.get("path").and_then(|p| p.as_str()) == Some(file)) {
        Some(at) => list[at] = resource,
        None => list.push(resource),
    }
    serde_json::to_string_pretty(&package)
        .map(|t| t + "\n")
        .map_err(|e| e.to_string())
}

/// Where the delimiters that actually separate fields stand, in characters
/// from the line's start, and whether the line ends **inside** a quoted field.
///
/// One pass, and the only place the quoting rule is written down. Everything
/// that has to agree about where a cell begins reads it from here: the cell
/// spans, the record that runs on, and the grid the reader sees. They used to
/// disagree — the grid counted every comma in the line, so 「"Smith, John"」
/// was drawn as two cells in a file the parser read as one (#391).
fn scan(line: &str, delimiter: char) -> (Vec<usize>, bool) {
    let line = line.trim_end_matches(['\n', '\r']);
    let mut walls = Vec::new();
    // Where a field may **open** with a quote: at the start of the line or
    // just after a delimiter, spaces allowed before it because real files have
    // them. A quote anywhere else — `he said "hi"` — is a character like any
    // other, and a writer who needed to protect a delimiter would have quoted
    // the whole field.
    let mut fresh = true;
    let mut inside = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' if inside => match chars.get(i + 1) {
                // `""` inside a quoted field is one quote, not the end of it
                // (RFC 4180). Stepping over both is the whole of reading it.
                Some('"') => i += 1,
                _ => inside = false,
            },
            '"' if fresh => {
                inside = true;
                fresh = false;
            }
            _ if c == delimiter && !inside => {
                walls.push(i);
                fresh = true;
            }
            ' ' | '\t' if fresh => {}
            _ => fresh = false,
        }
        i += 1;
    }
    (walls, inside)
}

/// Where the field separators of a line stand, in characters from its start.
///
/// A delimiter inside a quoted field is a character in that field, not a wall,
/// so this is what the grid is drawn on. See [`scan`].
pub fn walls(line: &str, delimiter: char) -> Vec<usize> {
    scan(line, delimiter).0
}

/// Where each cell of a line starts and ends, in characters from the line's
/// start.
///
/// The spans **include** the quotes a field is wrapped in, because they are in
/// the file and the writer is looking at the file; [`cell_text`] is the other
/// end of that.
pub fn cells(line: &str, delimiter: char) -> Vec<(usize, usize)> {
    let end = line.trim_end_matches(['\n', '\r']).chars().count();
    let mut out = Vec::new();
    let mut start = 0;
    for wall in walls(line, delimiter) {
        out.push((start, wall));
        start = wall + 1;
    }
    out.push((start, end));
    out
}

/// Whether this line ends **inside** a quoted field — a record that runs on
/// into the next line (#311).
///
/// The one shape of RFC 4180 this editor does not read, and it is not the
/// splitter that cannot: the grid is a line and a record is a line, all the
/// way up through the cursor, the cell spans and the row operations. A field
/// holding a line break is a record that is two lines, and nothing about
/// reading the delimiter more cleverly would change that.
///
/// So it is recognised and said rather than half-read. Left to itself the
/// symptom is only that the rows disagree about how many fields they have,
/// and the file 「is not a grid」 — true, and no help at all.
pub fn field_runs_on(line: &str, delimiter: char) -> bool {
    scan(line, delimiter).1
}

/// **How many lines the record starting here spans** (2026-10-20 作者定).
///
/// `lines` is this record's first line and the ones after it, each **with** its
/// line break — `rope.lines_at(first)`, in other words. The answer is 1 for
/// almost every record, and more only where a quoted field holds a line break.
///
/// Warning: **A record that never closes its quote is one line, not all of them.**
/// A stray `"` in a 123,380-line file would otherwise swallow the rest of the
/// table into one record, and every row below it would vanish from the grid.
/// `:table-check` is the place that complains about an unclosed quote; this one
/// gives up and lets the line stand alone, so the damage stops at the line that
/// has it.
pub fn record_lines<I, S>(lines: I, delimiter: char) -> usize
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut held = String::new();
    for (taken, line) in lines.into_iter().take(RECORD_LINES_MOST).enumerate() {
        held.push_str(line.as_ref());
        if !scan(&held, delimiter).1 {
            return taken + 1;
        }
    }
    // Never closed: the line stands alone, so the damage stops at the line
    // that has the stray quote.
    1
}

/// How far [`record_lines`] looks for the quote that closes a record.
///
/// A bound for **cost**, not for correctness: without one, a single stray `"`
/// in a 123,380-line 拆分表 makes every row query scan to the end of the file,
/// which is the whole table per row. A field holding more than this many lines
/// is not a field somebody wrote, it is a quote nobody closed — and that is
/// what `:table-check` is for.
pub const RECORD_LINES_MOST: usize = 1000;

/// A field's value, with the quotes a delimited file wraps it in taken off.
///
/// The span [`cells`] hands back **includes** the quotes, because they are in
/// the file and the writer is looking at the file. This is for the other end:
/// handing the value to something that is going to quote it again its own way
/// — an export, a copied column, a schema check.
pub fn unquote(text: &str) -> String {
    let trimmed = text.trim();
    match trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        false => text.to_string(),
        true => trimmed[1..trimmed.len() - 1].replace("\"\"", "\""),
    }
}

/// A value written so that a reader cut by `delimiter` gets it back whole.
///
/// Quoted only when it has to be — a delimiter, a quote or a line break in it
/// — because quoting a value that does not need it changes a file for nothing,
/// and this editor's promise is that it changes what it was asked to.
pub fn quote_for(value: &str, delimiter: char) -> String {
    match value.contains([delimiter, '"', '\n', '\r']) {
        false => value.to_string(),
        true => format!("\"{}\"", value.replace('"', "\"\"")),
    }
}

/// The delimiters worth guessing at, best first.
///
/// **A single space is not among them, and never will be.** A run of spaces
/// separating columns is a real thing — a 碼表 is usually written that way —
/// but *one* space is the character that holds a sentence together, and a rule
/// that splits on it turns every line of prose into a grid. Runs of spaces are
/// a different question with a different answer (#216).
const GUESSES: [char; 3] = ['\t', ',', ';'];

/// The delimiters a **block inside a document** may be cut by (#216).
///
/// The three above and `&`, which is what LaTeX's `tabular` and Typst's
/// `#table` put between cells. `&` is not among [`GUESSES`] because that list
/// is asked of a whole *file*, where a run of lines each holding one `&` is as
/// likely to be prose about HTML entities; asked of the few lines a person is
/// standing in and has just pressed the key about, it is worth guessing.
pub const BLOCK_GUESSES: [char; 4] = ['\t', ',', ';', '&'];

/// The delimiters a **plain-text file** may turn out to be cut by (#380).
///
/// The three above, in that order of confidence: a file cut by tabs usually
/// has spaces inside its fields, so the stronger mark has to be tried first.
///
/// **No space** (2026-10-30, as decided). A shopping list whose lines all
/// hold two spaces was read as a three-column grid, and `:w` then wrote
/// quotes around a cell nobody had quoted: `call mum "tonight and dad"`. A
/// space-separated table is asked for by name (`:table space`, `-t`).
pub const TEXT_GUESSES: [char; 3] = ['\t', ',', ';'];

/// Which character splits these lines into cells, if one plainly does.
///
/// The test is not 「which appears most often」 but 「which appears the **same**
/// number of times in every line, at least once」 — a grid is rectangular, and
/// that is the only property of one visible from the outside. A 、 in a
/// sentence fails it; a comma in a CSV passes it; and a block of prose, where
/// every line holds a different number of 逗號, comes back `None` rather than
/// being cut into ragged cells.
pub fn sniff(lines: &[String]) -> Option<char> {
    sniff_among(lines, &GUESSES)
}

/// The same question, asked of a named set of candidates.
///
/// The set is the only thing that differs between a file and a block in a
/// document (#216) — the *rule* is one rule, and keeping it in one place is
/// what stops the two from drifting into disagreeing about what a grid is.
pub fn sniff_among(lines: &[String], guesses: &[char]) -> Option<char> {
    let rows: Vec<&String> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
    if rows.len() < 2 {
        // One line says nothing about what is regular: a single 「甲,乙」 is as
        // likely a sentence as a row. Two lines that agree is the least
        // evidence worth acting on.
        return None;
    }
    guesses.iter().copied().find(|&c| {
        let counts = outside_quotes(&rows, c);
        let mut counts = counts.into_iter();
        let first = counts.next().unwrap_or(0);
        first > 0 && counts.all(|n| n == first)
    })
}

/// **How many times `c` cuts each record**, counting only outside double
/// quotes, a quoted field running over lines making one record (2026-10-30).
/// Counted raw, `A,"x, y"` held two commas against its neighbours' one, and a
/// real CSV with one quoted comma was "no delimiter in sight".
fn outside_quotes(rows: &[&String], c: char) -> Vec<usize> {
    let mut counts = Vec::new();
    let mut quoted = false;
    let mut n = 0;
    for row in rows {
        for ch in row.chars() {
            match ch {
                '"' => quoted = !quoted,
                ch if ch == c && !quoted => n += 1,
                _ => {}
            }
        }
        if !quoted {
            counts.push(n);
            n = 0;
        }
    }
    if quoted {
        counts.push(n);
    }
    counts
}

/// **一張表的寫法**：`|` 表格，或者某個分隔符分開的文本（2026-10-02 定）。
///
/// `csv` 和 `tsv` 不是兩種東西，是 [`Shape::Delimited`] 的兩個分隔符——Pandoc
/// 的叫法也是這樣（`csv`/`tsv`/`pipe_tables`），所以命令上那三個詞是通用詞彙，
/// 不是我們自己造的。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    /// Markdown 的 `|` 表格。
    Pipe,
    /// 分隔符分開的文本——`csv` 是 `,`，`tsv` 是 `\t`。
    Delimited(char),
}

impl Shape {
    /// 命令上那個詞：`pipe`、`csv`、`tsv`，或者任何一個分隔符的名字。
    pub fn named(word: &str) -> Option<Shape> {
        match word {
            "pipe" => Some(Shape::Pipe),
            "csv" => Some(Shape::Delimited(',')),
            "tsv" => Some(Shape::Delimited('\t')),
            _ => delimiter_named(word).map(Shape::Delimited),
        }
    }
}

/// 一個分隔符的名字：`tab`、`space`、`comma`、`semicolon`，或者它自己。
///
/// A delimiter is one character, and most of them can simply be typed. The two
/// that cannot are the tab — which the command line would never see, because
/// `Tab` completes — and the space, which is spelled out for the same reason
/// and is accepted here even though the sniffer will never guess it: a writer
/// who says `space` has looked at their data and decided.
pub fn delimiter_named(word: &str) -> Option<char> {
    match word {
        "tab" | "\\t" => Some('\t'),
        "space" | "\\s" => Some(' '),
        "comma" => Some(','),
        "semicolon" => Some(';'),
        "\" \"" | "' '" => Some(' '),
        // Warning: **一個字母不是分隔符，是打了一半的名字**（2026-10-10 報的）。
        // 上面每一個名字都以字母開頭，所以打 `p` 的人想的是 `pipe`、打 `c` 的人想
        // 的是 `csv`（還是 `comma`？兩個都對，正是不許猜的理由）。從前這裏照單收
        // 下，於是 `:convert-table p` **當場把那張表改寫成 `甲p乙`**，而狀態欄說
        // 「用「p」分欄」，看着像成功。兩個字母早就是拒的（`pi` → 「不認得」），
        // 漏的只有一個字母這一格。
        //
        // 別的單個字符照舊收：`;`、`|`、`#`、`、` 這些真的是分隔符，而且沒有一個
        // 名字叫它們。
        _ => {
            let mut chars = word.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if !c.is_whitespace() && !c.is_ascii_alphabetic() => Some(c),
                _ => None,
            }
        }
    }
}

/// **這幾行是哪一種寫法**——`|` 表格，還是哪個分隔符分開的。
///
/// `None` 是「看不出來」：一行說明不了什麼規律（見 [`sniff_among`]），而中文散文
/// 用的 `，、。` 三個猜測一個都不沾。
pub fn shape_of(lines: &[String]) -> Option<Shape> {
    let rows: Vec<&String> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
    if !rows.is_empty() && rows.iter().all(|l| crate::mdtable::is_row(l)) {
        return Some(Shape::Pipe);
    }
    sniff(lines).map(Shape::Delimited)
}

/// **同一張表，換一種寫法。**
///
/// **轉換不會失敗**（2026-10-03 改）。從前目標的分隔符出現在格子裏就交
/// `Err((行, 欄))`，理由是「不替人加引號」；而按欄位決定加不加引號正是 RFC 4180
/// 的規矩，`:export` 那一邊也早就這麼做了——兩邊從前不一致。見
/// [`crate::mdtable::to_delimited`]。
///
/// Warning: **轉成 `|` 表格會丟掉格子兩邊的空白**，轉到另一個分隔符不會：`|` 表格
/// 存不下那幾個空格（讀的時候兩邊一律去掉），分隔文本存得下。2026-10-02 定的，
/// 理由記在 `mdtable::from_delimited`。
pub fn recast(lines: &[String], from: Shape, to: Shape) -> Vec<String> {
    match (from, to) {
        (Shape::Pipe, Shape::Pipe) => lines.to_vec(),
        (Shape::Pipe, Shape::Delimited(c)) => crate::mdtable::to_delimited(lines, c),
        (Shape::Delimited(c), Shape::Pipe) => crate::mdtable::from_delimited(lines, c),
        (Shape::Delimited(a), Shape::Delimited(b)) if a == b => lines.to_vec(),
        (Shape::Delimited(a), Shape::Delimited(b)) => lines
            .iter()
            .map(|line| {
                cells(line, a)
                    .into_iter()
                    // **讀出值，再按目標的規矩寫回。** 逗號在 `.csv` 裏要引號、在
                    // `.tsv` 裏不要——帶着原來那一套引號過去，下一個程序拿到的是
                    // 一個名字裏有引號的值。
                    .map(|span| quote_for(&unquote(&cell_text(line, span)), b))
                    .collect::<Vec<_>>()
                    .join(&b.to_string())
            })
            .collect(),
    }
}

/// The text of one cell, given the line and the ranges.
pub fn cell_text(line: &str, span: (usize, usize)) -> String {
    line.chars().take(span.1).skip(span.0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quoted comma is not a delimiter (2026-10-30).
    #[test]
    fn a_quoted_comma_does_not_count_against_the_grid() {
        let lines: Vec<String> = ["name,city", "A,\"x, y\"", "B,\"line1", "line2\""].map(String::from).to_vec();
        assert_eq!(sniff(&lines), Some(','));
    }

    const DIVISION: &str = r#"{
  "name": "division",
  "resources": [
    {
      "name": "golden",
      "path": "yuhao_division_golden_source.csv",
      "dialect": { "delimiter": "," },
      "schema": {
        "fields": [
          { "name": "char", "title": "字" },
          { "name": "ids_y", "title": "宇浩拆分" },
          { "name": "ids_g" },
          { "name": "weight", "type": "integer" }
        ],
        "primaryKey": "char"
      }
    },
    { "name": "other", "path": "sub/other.csv", "schema": { "fields": [ { "name": "a" } ] } }
  ]
}"#;

    #[test]
    fn a_datapackage_says_what_the_columns_are_and_what_they_are_called() {
        let s = Schema::from_datapackage(DIVISION, "yuhao_division_golden_source.csv")
            .unwrap()
            .expect("listed");
        assert_eq!(s.delimiter, ',');
        assert!(s.header);
        assert_eq!(s.columns[0].heading(), "字", "the title, when there is one");
        assert_eq!(s.columns[2].heading(), "ids_g", "the name, when there is not");
        assert_eq!(s.columns[3].kind, Kind::Int, "a Frictionless type");
        assert_eq!(s.index_of("ids_y"), Some(1));
        assert_eq!(s.key.as_deref(), Some("char"), "primaryKey names a row");
        assert!(
            Schema::from_datapackage(DIVISION, "something_else.csv").unwrap().is_none(),
            "a file the package does not list"
        );
        assert!(Schema::from_datapackage(DIVISION, "other.csv").unwrap().is_some(), "by file name");
    }

    #[test]
    fn a_datapackage_that_does_not_add_up_says_so_rather_than_half_working() {
        let bad = |body: &str| Schema::from_datapackage(body, "a.csv").unwrap_err();
        assert!(bad("{").contains("datapackage.json"));
        assert!(bad(r#"{"resources":[{"path":"a.csv","schema":{"fields":[]}}]}"#).contains("no fields"));
        assert!(bad(r#"{"resources":[{"path":"a.csv","dialect":{"delimiter":"::"},"schema":{"fields":[{"name":"c"}]}}]}"#)
            .contains("one character"));
        assert!(bad(r#"{"resources":[{"path":"a.csv","dialect":{"delimiter":"\n"},"schema":{"fields":[{"name":"c"}]}}]}"#)
            .contains("separates rows"));
        assert!(bad(r#"{"resources":[{"path":"a.csv","schema":{"fields":[{"name":"c"}],"primaryKey":"nope"}}]}"#)
            .contains("not a field"));
    }

    #[test]
    fn a_file_with_no_schema_still_has_a_header_to_read() {
        let s = Schema::from_header("char,ids_y,,block\n", ',');
        assert_eq!(
            s.columns.iter().map(|c| c.heading()).collect::<Vec<_>>(),
            vec!["char", "ids_y", "3", "block"],
            "a blank header cell is named by its position, not left nameless"
        );
    }

    #[test]
    fn a_datapackage_is_written_beside_the_data_and_keeps_its_neighbours() {
        let s = Schema::from_header("char\tids_y\n", '\t');
        let fresh = datapackage_with(None, "division", "d.tsv", &s).unwrap();
        let back = Schema::from_datapackage(&fresh, "d.tsv").unwrap().expect("listed");
        assert_eq!(back.delimiter, '\t');
        assert_eq!(back.columns.len(), 2);
        assert!(fresh.contains("\"name\": \"division\"") && fresh.contains("$schema"), "{fresh}");
        // A second file joins the package; the first stays as it was.
        let mut other = Schema::numbered(3, ';');
        other.key = Some("1".to_string());
        let both = datapackage_with(Some(&fresh), "division", "e.csv", &other).unwrap();
        assert!(Schema::from_datapackage(&both, "d.tsv").unwrap().is_some());
        let e = Schema::from_datapackage(&both, "e.csv").unwrap().expect("listed");
        assert!(!e.header && e.delimiter == ';' && e.key.as_deref() == Some("1"));
        // Written again, a file replaces its own resource rather than doubling it.
        let again = datapackage_with(Some(&both), "division", "e.csv", &other).unwrap();
        assert_eq!(again.matches("\"e.csv\"").count(), 1);
    }

    #[test]
    fn a_line_is_split_on_the_delimiter_and_nothing_else() {
        // No quoting: a `"` is an ordinary character, which is exactly what
        // the file it was written from means by it.
        let line = "一,⿰木目,\"quoted\",,末";
        let spans = cells(line, ',');
        assert_eq!(spans.len(), 5);
        assert_eq!(cell_text(line, spans[0]), "一");
        assert_eq!(cell_text(line, spans[1]), "⿰木目");
        assert_eq!(cell_text(line, spans[2]), "\"quoted\"");
        assert_eq!(cell_text(line, spans[3]), "", "an empty cell is a cell");
        assert_eq!(cell_text(line, spans[4]), "末");
        // Counted in characters, not bytes, because that is what a rope indexes
        // by — 「一」 is three bytes and one character.
        assert_eq!(spans[1], (2, 5), "⿰木目 is three characters and nine bytes");

        // The line ending is not part of the last cell.
        assert_eq!(cell_text("甲,乙\r\n", cells("甲,乙\r\n", ',')[1]), "乙");
        // A line with no delimiter at all is one cell, not zero.
        assert_eq!(cells("甲", ',').len(), 1);
        assert_eq!(cells("", ',').len(), 1);
    }

    #[test]
    fn the_schema_is_found_beside_the_file_and_an_old_one_is_named() {
        let root = std::env::temp_dir().join(format!("yumete-schema-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let deep = root.join("assets").join("division");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join(DATAPACKAGE), DIVISION).unwrap();
        let csv = deep.join("yuhao_division_golden_source.csv");
        std::fs::write(&csv, "char,ids_y,ids_g,weight\n一,⿰木目,⿰木目,1\n").unwrap();

        let (found, schema) = schema_for(&csv).expect("found beside the file");
        assert!(found.ends_with(DATAPACKAGE));
        assert_eq!(schema.columns.len(), 4);

        // Beside the data only: a package one folder up does not reach down.
        let nested = deep.join("卷一");
        std::fs::create_dir_all(&nested).unwrap();
        let same = nested.join("yuhao_division_golden_source.csv");
        std::fs::write(&same, "char\n一\n").unwrap();
        assert!(schema_for(&same).is_none());

        let other = deep.join("notes.csv");
        std::fs::write(&other, "a,b\n1,2\n").unwrap();
        assert!(schema_for(&other).is_none());

        // The old format is not read, and the reader is told once why not.
        std::fs::create_dir_all(deep.join(".yumete").join("tables")).unwrap();
        std::fs::write(deep.join(".yumete").join("tables").join("n.toml"), "[table]\nfile = 'notes.csv'\n").unwrap();
        let (found, problems) = schema_for_reporting(&other);
        assert!(found.is_none());
        assert!(problems.iter().any(|p| p.contains("no longer read")), "{problems:?}");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_sniffer_asks_whether_the_lines_agree_not_which_is_commonest() {
        let of = |text: &str| -> Option<char> {
            sniff(&text.lines().map(str::to_string).collect::<Vec<_>>())
        };
        assert_eq!(of("字,讀音\n永,ㄩㄥˇ\n和,ㄏㄜˊ"), Some(','));
        assert_eq!(of("字\t讀音\n永\tㄩㄥˇ"), Some('\t'));
        assert_eq!(of("a;b;c\nd;e;f"), Some(';'));
        // A tab beats a comma that is also regular: a file with both is a TSV
        // whose cells hold 逗號.
        assert_eq!(of("甲,乙\tb\n丙,丁\te"), Some('\t'));

        // Prose. Every line holds a different number of 逗號, so nothing here
        // is a column — and a grid is what would otherwise be made of a novel.
        assert_eq!(
            of("那年冬天，雪下得早。\n他站在門口，看了很久，沒有進去。"),
            None
        );
        // …and that is the rule doing the work, not the fact that 中文 uses
        // 全角 marks the guesses do not include. Lines that disagree in ASCII
        // are refused just the same — three commas then two is not a shape,
        // however commonly the comma turns up.
        assert_eq!(of("a,b,c\nd,e"), None);
        // A single space is never guessed at, however regular it is.
        assert_eq!(of("one two\nthree four"), None);
        // One line is not evidence of a shape.
        assert_eq!(of("字,讀音"), None);
        assert_eq!(of(""), None);
    }

    /// **一個字母是打了一半的名字，不是分隔符**（2026-10-10 報的）。
    ///
    /// `:convert-table p` 從前當場把表改寫成 `甲p乙` 並報「用「p」分欄」。
    #[test]
    fn one_letter_is_half_a_name_and_not_a_delimiter() {
        for half in ["p", "c", "t", "s", "P", "z"] {
            assert_eq!(delimiter_named(half), None, "{half}");
            assert_eq!(Shape::named(half), None, "{half}");
        }
        // 真的分隔符照舊收，名字也照舊。
        assert_eq!(delimiter_named(";"), Some(';'));
        assert_eq!(delimiter_named("、"), Some('、'));
        assert_eq!(delimiter_named("tab"), Some('\t'));
        assert_eq!(Shape::named("csv"), Some(Shape::Delimited(',')));
        assert_eq!(Shape::named("pipe"), Some(Shape::Pipe));
    }
}
