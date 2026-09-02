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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jump {
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
    /// what a jump lands on.
    pub key: Option<String>,
    /// What separates two cells.
    pub delimiter: char,
    /// Whether the first line names the columns.
    pub header: bool,
    pub columns: Vec<Column>,
    pub details: Vec<Detail>,
    pub jump: Option<Jump>,
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
            Some(d) if d.chars().count() == 1 => d.chars().next().unwrap(),
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
        let jump = match t.jump {
            None => None,
            Some(j) => {
                for name in j.from.iter().chain(std::iter::once(&j.to)) {
                    if !names.contains(&name.as_str()) {
                        return Err(format!("jump names '{name}', which is not a column"));
                    }
                }
                Some(Jump {
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
                })
                .collect(),
            details,
            jump,
            ranges,
        })
    }

    /// Make a bare schema out of a file's own header row.
    ///
    /// What `yumete -t` falls back on: every CSV already says what its columns
    /// are on its first line, so a file nobody has written a schema for can
    /// still be read as a grid. It gets the names and nothing else — no
    /// labels, no computed fields, no jumps, because those are knowledge about
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
            jump: None,
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
                if let Ok(schema) = Schema::parse(&text) {
                    if schema.covers(path) {
                        return Some((file, schema));
                    }
                }
            }
        }
        dir = d.parent();
    }
    None
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
    for (i, c) in line.chars().enumerate() {
        if c == delimiter {
            out.push((start, i));
            start = i + 1;
        }
    }
    out.push((start, line.chars().count()));
    out
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
    jump: Option<RawJump>,
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
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDetail {
    name: String,
    compute: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJump {
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

[table.jump]
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
            s.jump,
            Some(Jump {
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
             [table.jump]\nfrom = ['c']\nto = 'nope'"
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
        assert!(s.details.is_empty() && s.jump.is_none(), "names only");
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
}
