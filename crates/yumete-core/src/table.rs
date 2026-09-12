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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

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

/// A field that is *worked out* from a row rather than stored in it.
///
/// It never enters the grid and is never written back — it exists so the
/// detail panel can answer "what is this character" without the answer having
/// to be a column somebody has to keep up to date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detail {
    pub name: String,
    pub compute: Compute,
}

/// The two-and-a-half functions a computed field may use.
///
/// Not an expression language, and deliberately not extensible: every one of
/// these is a fact about a character that the editor already knows, and a
/// schema that could compute *anything* would be a scripting runtime with the
/// serial numbers filed off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compute {
    /// `codepoint(col)` — `U+4E00`.
    Codepoint(String),
    /// `block(col)` — the Unicode block's own name.
    Block(String),
    /// `range_label(col, ranges)` — a name from the schema's own range table,
    /// so a project may call U+2B740 「CJK-D」 or anything else it likes.
    RangeLabel(String, String),
}

impl Compute {
    /// Which column it reads.
    pub fn column(&self) -> &str {
        match self {
            Compute::Codepoint(c) | Compute::Block(c) | Compute::RangeLabel(c, _) => c,
        }
    }

    /// Work it out for one cell's text.
    pub fn apply(&self, cell: &str, ranges: &HashMap<String, Vec<Range>>) -> String {
        // Every one of these is about a *character*, so an empty cell has no
        // answer and a cell of several characters is asked about its first.
        let Some(c) = cell.chars().next() else {
            return String::new();
        };
        match self {
            Compute::Codepoint(_) => yumete_cjk::blocks::codepoint(c),
            Compute::Block(_) => yumete_cjk::blocks::block_of(c).unwrap_or("—").to_string(),
            Compute::RangeLabel(_, table) => ranges
                .get(table)
                .and_then(|rs| rs.iter().find(|r| r.holds(c)))
                .map(|r| r.name.clone())
                .unwrap_or_default(),
        }
    }
}

/// One named span of the code space, from a schema's `[ranges.*]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Range {
    pub name: String,
    pub first: u32,
    pub last: u32,
}

impl Range {
    fn holds(&self, c: char) -> bool {
        (self.first..=self.last).contains(&(c as u32))
    }
}

/// "The text in this cell names a row of this same table."
///
/// A **link**, and it was called `jump` until it was noticed that a jump is
/// only one of the two directions it is read in. `Enter` follows it forward —
/// 「⿰木目 is made of 木 and 目, and 木 has a row of its own」 — and backwards:
/// *who names this?*, which the manual has always described in the same breath
/// as 「這不是跳轉，是搜索」. One relation, two questions, and the name has to
/// be the relation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// The columns whose contents are keys.
    pub from: Vec<String>,
    /// The column those keys are found in.
    pub to: String,
}

/// Everything the editor needs to read one kind of table.
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
    pub details: Vec<Detail>,
    pub link: Option<Link>,
    pub ranges: HashMap<String, Vec<Range>>,
}

impl Schema {
    /// Read a schema from TOML.
    pub fn parse(text: &str) -> Result<Schema, String> {
        let raw: RawFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let t = raw.table;
        if t.column.is_empty() {
            return Err("a table with no columns".to_string());
        }
        let delimiter = match t.delimiter.as_deref() {
            None => ',',
            // A line terminator is one character, and it is the one character
            // that cannot separate two cells *of the same row*: rows are
            // already separated by it.
            Some(d) if d.chars().count() == 1 && !"\n\r".contains(d) => {
                d.chars().next().unwrap()
            }
            Some(d) if "\n\r".contains(d) => {
                return Err("a line break already separates rows, not cells".to_string())
            }
            Some(d) => return Err(format!("delimiter must be one character, not '{d}'")),
        };
        // The only quoting this reads is none at all, and saying so out loud
        // beats silently treating `"` as an ordinary character in a file whose
        // author believed otherwise.
        if let Some(q) = t.quoting.as_deref() {
            if q != "none" {
                return Err(format!(
                    "quoting = '{q}' is not supported; this reads unquoted files only"
                ));
            }
        }
        let mut ranges = HashMap::new();
        for (table, named) in raw.ranges {
            let mut list = Vec::new();
            for (name, span) in named {
                if span.len() != 2 || span[0] > span[1] {
                    return Err(format!("range '{name}' is not a pair first..last"));
                }
                list.push(Range {
                    name,
                    first: span[0],
                    last: span[1],
                });
            }
            list.sort_by_key(|r| r.first);
            ranges.insert(table, list);
        }
        let names: Vec<&str> = t.column.iter().map(|c| c.name.as_str()).collect();
        if let Some(key) = &t.key {
            if !names.contains(&key.as_str()) {
                return Err(format!("key = '{key}' is not a column"));
            }
        }
        let mut details = Vec::new();
        for d in &t.detail {
            let compute = parse_compute(&d.compute)?;
            if !names.contains(&compute.column()) {
                return Err(format!(
                    "'{}' computes from '{}', which is not a column",
                    d.name,
                    compute.column()
                ));
            }
            if let Compute::RangeLabel(_, table) = &compute {
                if !ranges.contains_key(table) {
                    return Err(format!("'{}' uses ranges.{table}, which is missing", d.name));
                }
            }
            details.push(Detail {
                name: d.name.clone(),
                compute,
            });
        }
        let link = match t.link {
            None => None,
            Some(j) => {
                for name in j.from.iter().chain(std::iter::once(&j.to)) {
                    if !names.contains(&name.as_str()) {
                        return Err(format!("link names '{name}', which is not a column"));
                    }
                }
                Some(Link {
                    from: j.from,
                    to: j.to,
                })
            }
        };
        Ok(Schema {
            files: t.file.into_vec(),
            key: t.key,
            delimiter,
            header: t.header.unwrap_or(true),
            columns: t
                .column
                .into_iter()
                .map(|c| Column {
                    name: c.name,
                    label: c.label,
                    kind: c.kind.unwrap_or_default(),
                    hidden: c.hidden.unwrap_or(false),
                })
                .collect(),
            details,
            link,
            ranges,
        })
    }

    /// A schema for a grid whose first row is **data**, its columns named by
    /// number (#216).
    ///
    /// A 碼表 has no header — 「字⇥碼」 all the way down — so reading its first
    /// line as the column names loses that line and calls one column 「一」.
    /// The numbers are the names the column-number row already draws (#184),
    /// so nothing new has to be shown for them to be usable.
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
            details: Vec::new(),
            link: None,
            ranges: HashMap::new(),
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
            details: Vec::new(),
            link: None,
            ranges: HashMap::new(),
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
fn parse_compute(text: &str) -> Result<Compute, String> {
    let text = text.trim();
    let (name, rest) = text
        .split_once('(')
        .ok_or_else(|| format!("'{text}' is not a call"))?;
    let args = rest
        .strip_suffix(')')
        .ok_or_else(|| format!("'{text}' is missing its ')'"))?;
    let args: Vec<&str> = args.split(',').map(str::trim).filter(|a| !a.is_empty()).collect();
    match (name.trim(), args.as_slice()) {
        ("codepoint", [c]) => Ok(Compute::Codepoint(c.to_string())),
        ("block", [c]) => Ok(Compute::Block(c.to_string())),
        ("range_label", [c, t]) => Ok(Compute::RangeLabel(c.to_string(), t.to_string())),
        (f @ ("codepoint" | "block" | "range_label"), _) => {
            Err(format!("{f} was given {} arguments", args.len()))
        }
        (f, _) => Err(format!("no such function: {f}")),
    }
}

/// Find the schema for a file, by walking up from the directory it is in.
///
/// Up from the *file*, not from the working directory: a schema is a property
/// of the data, not of the session that happened to open it. Opening the same
/// CSV from anywhere must find the same schema — and with several files open
/// at once from different projects, "the working directory" is not a thing
/// each of them has.
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
    let mut dir = path.parent();
    while let Some(d) = dir {
        let tables = d.join(".yumete").join("tables");
        if let Ok(entries) = std::fs::read_dir(&tables) {
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                .collect();
            // Read in a fixed order, so which schema wins never depends on
            // what the file system felt like returning first.
            files.sort();
            for file in files {
                let Ok(text) = std::fs::read_to_string(&file) else {
                    continue;
                };
                match Schema::parse(&text) {
                    Ok(schema) if schema.covers(path) => return (Some((file, schema)), problems),
                    // A schema that parses but is for other files is not a
                    // problem; one that does not parse is, whoever it is for.
                    Ok(_) => {}
                    Err(why) => problems.push(format!(
                        "{}: {why}",
                        file.file_name().unwrap_or_default().to_string_lossy()
                    )),
                }
            }
        }
        dir = d.parent();
    }
    (None, problems)
}

/// A schema file to start from, written out of what the grid is reading this
/// file as **right now** (#218).
///
/// **The first thing it does is nothing.** The delimiter is the one the
/// sniffer guessed, the header line is the one you kept or turned off (`t H`,
/// #217), and the columns carry the names the first row gave them — so opening
/// the file and reading it back changes not one cell of the grid. Every edit to
/// it is then a correction to something visible, which is a far shorter way in
/// than an empty page and a format to guess at.
///
/// `note` is the comment written at the top, so that the one sentence a person
/// reads first is a message like every other and not a literal in here.
pub fn starting_schema(file: &str, schema: &Schema, note: &str) -> String {
    let mut out = format!("# {note}\n\n[table]\nfile = {}\n", quoted(file));
    // Only what differs from the default is written. A starting schema that
    // spells out every setting reads as a form to fill in; one that names the
    // delimiter *because this file has an unusual one* teaches what the key is
    // for.
    if schema.delimiter != ',' {
        out.push_str(&format!("delimiter = {}\n", quoted(&schema.delimiter.to_string())));
    }
    if !schema.header {
        out.push_str("header = false\n");
    }
    if let Some(key) = &schema.key {
        out.push_str(&format!("key = {}\n", quoted(key)));
    }
    for column in &schema.columns {
        out.push_str(&format!("\n[[table.column]]\nname = {}\n", quoted(&column.name)));
        if let Some(label) = &column.label {
            out.push_str(&format!("label = {}\n", quoted(label)));
        }
        if column.kind != Kind::String {
            out.push_str(&format!("type = \"{:?}\"\n", column.kind));
        }
        if column.hidden {
            out.push_str("hidden = true\n");
        }
    }
    out
}

/// A TOML basic string. The 拆分表's delimiter is a tab, which is exactly the
/// character that cannot be written between two quotes as itself.
fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Where each cell of a line starts and ends, in characters from the line's
/// start.
///
/// `split` and nothing more: no quotes, no escapes, no state. A line of
/// twenty-eight fields costs one pass over it, which is what makes it
/// affordable to do this for the rows on screen and no others.
pub fn cells(line: &str, delimiter: char) -> Vec<(usize, usize)> {
    let line = line.trim_end_matches(['\n', '\r']);
    let mut out = Vec::new();
    let mut start = 0;
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
                out.push((start, i));
                start = i + 1;
                fresh = true;
            }
            ' ' | '\t' if fresh => {}
            _ => fresh = false,
        }
        i += 1;
    }
    out.push((start, chars.len()));
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
    let line = line.trim_end_matches(['\n', '\r']);
    let mut fresh = true;
    let mut inside = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '"' if inside => match chars.get(i + 1) {
                Some('"') => i += 1,
                _ => inside = false,
            },
            '"' if fresh => {
                inside = true;
                fresh = false;
            }
            c if c == delimiter && !inside => fresh = true,
            ' ' | '\t' if fresh => {}
            _ => fresh = false,
        }
        i += 1;
    }
    inside
}

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

/// Whether any field on this line **opens with a quote** — the one shape
/// [`cells`] cannot read (#307).
///
/// `cells` splits and nothing more, which is what makes a grid over an 8 MB
/// file affordable. The price is that `"Smith, John"` is two fields to it, and
/// so an edit to the field *beside* it writes back a row rebuilt from the wrong
/// pieces: `2500,"Smith, John",note` becomes `2500,"Smith,ZZ,note` — the name
/// gone, the file no longer parseable, and nothing said.
///
/// **A field's own quote, not any quote.** `he said "hi"` holds no delimiter
/// and splits correctly; a writer that had to protect a comma would have
/// quoted the whole field, and that is what this looks for — at the start of
/// the line or just after a delimiter, spaces allowed before it because real
/// files have them.
pub fn quoted_field(line: &str, delimiter: char) -> bool {
    let line = line.trim_end_matches(['\n', '\r']);
    let mut fresh = true;
    for c in line.chars() {
        match c {
            _ if c == delimiter => fresh = true,
            '"' if fresh => return true,
            ' ' | '\t' => {}
            _ => fresh = false,
        }
    }
    false
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
/// The three above and a space, in that order of confidence. A space is not
/// among [`GUESSES`] because that list is also asked of a document, where a
/// space is what words are separated by; asked of a whole file whose every
/// line holds the same number of them, it is a column separator. The order
/// matters: a file cut by tabs usually has spaces inside its fields, so the
/// stronger mark has to be tried first.
pub const TEXT_GUESSES: [char; 4] = ['\t', ',', ';', ' '];

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
        let mut counts = rows.iter().map(|l| l.matches(c).count());
        let first = counts.next().unwrap_or(0);
        first > 0 && counts.all(|n| n == first)
    })
}

/// The text of one cell, given the line and the ranges.
pub fn cell_text(line: &str, span: (usize, usize)) -> String {
    line.chars().take(span.1).skip(span.0).collect()
}

// ---- The TOML as written -------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    table: RawTable,
    #[serde(default)]
    ranges: HashMap<String, HashMap<String, Vec<u32>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTable {
    file: OneOrMany,
    #[serde(default)]
    key: Option<String>,
    delimiter: Option<String>,
    quoting: Option<String>,
    header: Option<bool>,
    #[serde(default)]
    column: Vec<RawColumn>,
    #[serde(default)]
    detail: Vec<RawDetail>,
    link: Option<RawLink>,
}

/// `file = "a.csv"` and `file = ["a.csv", "b.csv"]` both read.
#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => vec![s],
            OneOrMany::Many(v) => v,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawColumn {
    name: String,
    label: Option<String>,
    #[serde(rename = "type")]
    kind: Option<Kind>,
    hidden: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDetail {
    name: String,
    compute: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLink {
    from: Vec<String>,
    to: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIVISION: &str = r#"
[table]
file = ["yuhao_division_golden_source.csv", "yuhao_division_pending.csv"]
key = "char"
delimiter = ","
quoting = "none"

[[table.column]]
name = "char"
label = "字"

[[table.column]]
name = "ids_y"
label = "宇浩拆分"

[[table.column]]
name = "ids_g"

[[table.detail]]
name = "unicode"
compute = "codepoint(char)"

[[table.detail]]
name = "block"
compute = "range_label(char, cjk_blocks)"

[table.link]
from = ["ids_y", "ids_g"]
to = "char"

[ranges.cjk_blocks]
"CJK" = [0x4E00, 0x9FFF]
"CJK-D" = [0x2B740, 0x2B81F]
"#;

    #[test]
    fn a_schema_says_what_the_columns_are_and_what_they_are_called() {
        let s = Schema::parse(DIVISION).unwrap();
        assert_eq!(s.files.len(), 2, "golden and pending share one schema");
        assert!(s.covers(Path::new("/x/yuhao_division_pending.csv")));
        assert!(!s.covers(Path::new("/x/something_else.csv")));
        assert_eq!(s.delimiter, ',');
        assert!(s.header);
        assert_eq!(s.columns[0].heading(), "字", "the label, when there is one");
        assert_eq!(s.columns[2].heading(), "ids_g", "the name, when there is not");
        assert_eq!(s.columns[0].kind, Kind::String, "the default");
        assert_eq!(s.index_of("ids_y"), Some(1));
        assert_eq!(s.key.as_deref(), Some("char"), "what names a row");
        assert_eq!(
            s.link,
            Some(Link {
                from: vec!["ids_y".into(), "ids_g".into()],
                to: "char".into()
            })
        );
    }

    #[test]
    fn a_computed_field_is_worked_out_not_stored() {
        let s = Schema::parse(DIVISION).unwrap();
        assert_eq!(s.details[0].compute, Compute::Codepoint("char".into()));
        assert_eq!(s.details[0].compute.apply("一", &s.ranges), "U+4E00");
        // The project's own vocabulary, from the schema's range table.
        assert_eq!(s.details[1].compute.apply("一", &s.ranges), "CJK");
        assert_eq!(s.details[1].compute.apply("\u{2B740}", &s.ranges), "CJK-D");
        assert_eq!(s.details[1].compute.apply("", &s.ranges), "", "an empty cell");
        // …or Unicode's own, when a schema would rather not keep a list.
        let block = Compute::Block("char".into());
        assert_eq!(block.apply("一", &s.ranges), "CJK Unified Ideographs");
    }

    #[test]
    fn a_schema_that_does_not_add_up_says_so_rather_than_half_working() {
        let bad = |body: &str| Schema::parse(body).unwrap_err();
        assert!(bad("[table]\nfile = 'a.csv'").contains("no columns"));
        assert!(bad(
            "[table]\nfile = 'a.csv'\nquoting = 'minimal'\n[[table.column]]\nname = 'c'"
        )
        .contains("not supported"));
        assert!(bad(
            "[table]\nfile = 'a.csv'\ndelimiter = '::'\n[[table.column]]\nname = 'c'"
        )
        .contains("one character"));
        // A computed field that reads a column nobody declared is a typo that
        // would otherwise show up as a silently empty panel.
        assert!(bad(
            "[table]\nfile = 'a.csv'\n[[table.column]]\nname = 'c'\n\
             [[table.detail]]\nname = 'u'\ncompute = 'codepoint(nope)'"
        )
        .contains("not a column"));
        assert!(bad(
            "[table]\nfile = 'a.csv'\n[[table.column]]\nname = 'c'\n\
             [[table.detail]]\nname = 'u'\ncompute = 'shell(c)'"
        )
        .contains("no such function"));
        assert!(bad(
            "[table]\nfile = 'a.csv'\n[[table.column]]\nname = 'c'\n\
             [table.link]\nfrom = ['c']\nto = 'nope'"
        )
        .contains("not a column"));
        // Range tables have to exist before something names one.
        assert!(bad(
            "[table]\nfile = 'a.csv'\n[[table.column]]\nname = 'c'\n\
             [[table.detail]]\nname = 'b'\ncompute = 'range_label(c, blocks)'"
        )
        .contains("missing"));
        assert!(bad("[table]\nfile = 'a.csv'\nkey = 'nope'\n[[table.column]]\nname = 'c'")
            .contains("not a column"));
    }

    #[test]
    fn a_file_with_no_schema_still_has_a_header_to_read() {
        let s = Schema::from_header("char,ids_y,,block\n", ',');
        assert_eq!(
            s.columns.iter().map(|c| c.heading()).collect::<Vec<_>>(),
            vec!["char", "ids_y", "3", "block"],
            "a blank header cell is named by its position, not left nameless"
        );
        assert!(s.details.is_empty() && s.link.is_none(), "names only");
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
    fn the_schema_is_found_by_walking_up_from_the_file() {
        let root = std::env::temp_dir().join(format!("yumete-schema-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let deep = root.join("assets").join("division");
        std::fs::create_dir_all(&deep).unwrap();
        // The schema sits beside the data…
        std::fs::create_dir_all(deep.join(".yumete").join("tables")).unwrap();
        std::fs::write(
            deep.join(".yumete").join("tables").join("division.toml"),
            DIVISION,
        )
        .unwrap();
        let csv = deep.join("yuhao_division_golden_source.csv");
        std::fs::write(&csv, "char,ids_y,ids_g\n一,⿰木目,⿰木目\n").unwrap();

        let (found, schema) = schema_for(&csv).expect("found from the file's own directory");
        assert!(found.ends_with("division.toml"));
        assert_eq!(schema.columns.len(), 3);

        // …and is still found for a file further down, since the walk goes up.
        let nested = deep.join("卷一");
        std::fs::create_dir_all(&nested).unwrap();
        let same = nested.join("yuhao_division_pending.csv");
        std::fs::write(&same, "char\n一\n").unwrap();
        assert!(schema_for(&same).is_some(), "found from a directory below");

        // A file no schema names has none, rather than borrowing the nearest.
        let other = deep.join("notes.csv");
        std::fs::write(&other, "a,b\n1,2\n").unwrap();
        assert!(schema_for(&other).is_none());

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
}
