//! **Word files to Markdown** (`ye --import-word`): a `.docx`, or a `.doc`
//! made a `.docx` first, read by pandoc and written beside it as `.md`.
//!
//! pandoc's docx reader does the reading. What is added here is what it
//! leaves out or guesses at:
//!
//! - **the file's properties** (`docProps/core.xml`: title, author, created,
//!   modified) as front matter, with `source`, the file's own name — only
//!   the keys that have values. pandoc does not read them;
//! - **no guesses**: pandoc reads a paragraph indented in Word as a
//!   quotation, and takes paragraphs in the Title, Subtitle, Author, Date and
//!   Abstract styles opening a document out of the text into its metadata.
//!   [`FILTER`] puts both back: an indented paragraph is a paragraph, and
//!   those paragraphs stay where they were written, as paragraphs. A heading
//!   is a paragraph in a real Heading style and nothing else — pandoc reads
//!   no bold line or large type as one, and each Word paragraph stays one;
//! - **readings**: pandoc drops Word's phonetic guide, base and all. Each
//!   one is written into the text, between marks no writer types, before
//!   pandoc reads it, and comes out in the brace form, `{漢字|hàn zì}`;
//! - **a `.doc`** (Word 97–2003) is made a `.docx` by LibreOffice or by
//!   macOS's `textutil`, whichever is there ([`to_docx`]);
//! - **a folder**: every `.doc` and `.docx` in it and under it, each `.md`
//!   beside its source or under `--out` in the same tree. Nothing is ever
//!   written over without `--overwrite`, and a source is only read.

use std::path::{Path, PathBuf};

/// What one import is told.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Where the Markdown goes: a folder the tree is mirrored under, or —
    /// one file named — a `.md` file.
    pub out: Option<PathBuf>,
    /// Whether a `.md` already there may be written over.
    pub overwrite: bool,
}

/// The `.md` each source is written to, in the order they are found: a
/// file named is itself, a folder every `.doc` and `.docx` under it.
/// A source two of which would write the same `.md` is refused, all of them.
pub fn plan(paths: &[PathBuf], options: &Options) -> Vec<(PathBuf, Result<PathBuf, String>)> {
    let mut found: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut refused: Vec<(PathBuf, Result<PathBuf, String>)> = Vec::new();
    let single = paths.len() == 1 && paths[0].is_file();
    for path in paths {
        if path.is_dir() {
            let mut files = Vec::new();
            walk(path, &mut files);
            for file in files {
                let relative = file.strip_prefix(path).unwrap_or(&file).with_extension("md");
                let target = match &options.out {
                    Some(out) => out.join(relative),
                    None => file.with_extension("md"),
                };
                found.push((file, target));
            }
        } else if path.is_file() && is_word(path) {
            let target = match &options.out {
                Some(out) if single && is_markdown(out) && !out.is_dir() => out.clone(),
                Some(out) => out.join(Path::new(path.file_name().unwrap_or_default()).with_extension("md")),
                None => path.with_extension("md"),
            };
            found.push((path.clone(), target));
        } else if path.is_file() {
            refused.push((path.clone(), Err(crate::say!("import-word.not-word"))));
        } else {
            refused.push((path.clone(), Err(crate::say!("import-word.not-there"))));
        }
    }
    let mut out = refused;
    for (k, (source, target)) in found.iter().enumerate() {
        let shared = found.iter().enumerate().find(|(j, (_, t))| *j != k && t == target);
        out.push(match shared {
            Some((_, (other, _))) => {
                (source.clone(), Err(crate::say!("import-word.same-target", other.display(), target.display())))
            }
            None => (source.clone(), Ok(target.clone())),
        });
    }
    out
}

/// Every `.doc` and `.docx` under `dir`, in name order: no hidden file or
/// folder, and none of the `~$…` files Word keeps while a document is open.
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if name.starts_with('.') || name.starts_with("~$") {
            continue;
        }
        if path.is_dir() {
            walk(&path, out);
        } else if is_word(&path) {
            out.push(path);
        }
    }
}

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase()
}

fn is_word(path: &Path) -> bool {
    matches!(extension(path).as_str(), "doc" | "docx")
}

fn is_markdown(path: &Path) -> bool {
    matches!(extension(path).as_str(), "md" | "markdown")
}

/// Whether pandoc is on the `PATH`.
pub fn pandoc_is_installed() -> bool {
    std::process::Command::new("pandoc")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// **One Word file to `target`**. What comes back on success is said
/// to the writer: the pictures folder, when there were pictures.
pub fn convert(source: &Path, target: &Path, overwrite: bool) -> Result<Option<PathBuf>, String> {
    if target.exists() && !overwrite {
        return Err(crate::say!("import-word.exists", target.display()));
    }
    let scratch = Scratch::new()?;
    let docx = match extension(source).as_str() {
        "doc" => to_docx(source, &scratch.0)?,
        _ => std::fs::read(source).map_err(|e| e.to_string())?,
    };
    let properties = properties(&docx)?;
    let pictures = has_pictures(&docx);
    let stem = target.file_stem().and_then(|s| s.to_str()).unwrap_or("document");
    let media = format!("{stem}.assets");
    let folder = target.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    if pictures && folder.join(&media).exists() && !overwrite {
        return Err(crate::say!("import-word.exists", folder.join(&media).display()));
    }
    let marked = readings_marked(&docx)?;
    let input = scratch.0.join("in.docx");
    std::fs::write(&input, &marked).map_err(|e| e.to_string())?;
    let filter = scratch.0.join("yumete.lua");
    std::fs::write(&filter, FILTER).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let mut pandoc = std::process::Command::new("pandoc");
    pandoc.args(["-f", "docx+styles", "-t", "gfm", "--wrap=none"]).arg(format!("--lua-filter={}", filter.display()));
    if pictures {
        pandoc.arg(format!("--extract-media={media}"));
    }
    let ran = pandoc.arg(&input).current_dir(folder).output().map_err(|_| crate::say!("import-word.no-pandoc"))?;
    if !ran.status.success() {
        return Err(String::from_utf8_lossy(&ran.stderr).trim().to_string());
    }
    let body = readings_restored(&String::from_utf8_lossy(&ran.stdout));
    let name = source.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let text = format!("{}{}", front_matter(&properties, name), body.trim_start_matches('\n'));
    crate::buffer::write_binary_atomically(target, text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(pictures.then(|| folder.join(media)))
}

/// A folder of its own under the system's temporary one, removed with it.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Scratch, String> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("yumete-word-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// **A `.doc` as a `.docx`**: LibreOffice first, which keeps Word's
/// styles — headings, lists, footnotes — and then macOS's `textutil`, which
/// keeps the words, their bold and italic and the file's properties but
/// writes no styles at all, so no heading comes through it.
fn to_docx(source: &Path, scratch: &Path) -> Result<Vec<u8>, String> {
    for office in ["soffice", "libreoffice"] {
        let ran = std::process::Command::new(office)
            .args(["--headless", "--convert-to", "docx", "--outdir"])
            .arg(scratch)
            .arg(source)
            .output();
        let made = scratch.join(Path::new(source.file_name().unwrap_or_default()).with_extension("docx"));
        if ran.is_ok_and(|o| o.status.success()) && made.is_file() {
            return std::fs::read(&made).map_err(|e| e.to_string());
        }
    }
    let out = scratch.join("converted.docx");
    let textutil = std::process::Command::new("textutil")
        .args(["-convert", "docx", "-output"])
        .arg(&out)
        .arg(source)
        .output();
    if textutil.is_ok_and(|o| o.status.success()) && out.is_file() {
        return std::fs::read(&out).map_err(|e| e.to_string());
    }
    Err(crate::say!("import-word.no-doc-converter"))
}

/// One part of the zip, as text.
fn part(docx: &[u8], name: &str) -> Result<Option<String>, String> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(docx)).map_err(|_| crate::say!("import-word.not-a-docx"))?;
    let Ok(mut file) = zip.by_name(name) else { return Ok(None) };
    let mut text = String::new();
    file.read_to_string(&mut text).map_err(|_| crate::say!("import-word.not-a-docx"))?;
    Ok(Some(text))
}

fn has_pictures(docx: &[u8]) -> bool {
    zip::ZipArchive::new(std::io::Cursor::new(docx)).is_ok_and(|zip| zip.file_names().any(|n| n.starts_with("word/media/")))
}

/// The file's properties that become front matter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Properties {
    pub title: Option<String>,
    pub author: Option<String>,
    pub created: Option<String>,
    pub modified: Option<String>,
}

/// **`docProps/core.xml`**'s title, creator and two dates, each left out
/// when it is empty.
pub fn properties(docx: &[u8]) -> Result<Properties, String> {
    let Some(core) = part(docx, "docProps/core.xml")? else {
        // A zip with no document in it is not a Word file at all.
        return match part(docx, "word/document.xml")? {
            Some(_) => Ok(Properties::default()),
            None => Err(crate::say!("import-word.not-a-docx")),
        };
    };
    let tree = roxmltree::Document::parse(&core).map_err(|_| crate::say!("import-word.not-a-docx"))?;
    let get = |name: &str| {
        tree.descendants()
            .find(|n| n.tag_name().name() == name)
            .and_then(|n| n.text())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
    };
    Ok(Properties { title: get("title"), author: get("creator"), created: get("created"), modified: get("modified") })
}

/// The front matter for `p` and a source called `name`.
pub fn front_matter(p: &Properties, name: &str) -> String {
    let quoted = |v: &str| format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\"").replace(['\n', '\r'], " "));
    let mut out = String::from("---\n");
    for (key, value) in [("title", &p.title), ("author", &p.author)] {
        if let Some(value) = value {
            out.push_str(&format!("{key}: {}\n", quoted(value)));
        }
    }
    for (key, value) in [("created", &p.created), ("modified", &p.modified)] {
        if let Some(value) = value {
            out.push_str(&format!("{key}: {value}\n"));
        }
    }
    if !name.is_empty() {
        out.push_str(&format!("source: {}\n", quoted(name)));
    }
    out.push_str("---\n\n");
    out
}

/// The marks a reading is carried through pandoc in: noncharacters, which
/// Unicode keeps for a program's own use and no writer types.
const BASE: char = '\u{FDD0}';
const READING: char = '\u{FDD1}';
const END: char = '\u{FDD2}';

/// **Each phonetic guide in the text written as its words**,
/// `BASE base READING reading END`, so that pandoc, which drops the guide,
/// carries it through. The parts with none are left as they were.
fn readings_marked(docx: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::{Read, Write};
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(docx)).map_err(|_| crate::say!("import-word.not-a-docx"))?;
    if !zip.file_names().any(|n| n.starts_with("word/") && n.ends_with(".xml")) {
        return Err(crate::say!("import-word.not-a-docx"));
    }
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut changed = false;
    for k in 0..zip.len() {
        let mut file = zip.by_index(k).map_err(|_| crate::say!("import-word.not-a-docx"))?;
        let name = file.name().to_string();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|_| crate::say!("import-word.not-a-docx"))?;
        let marked = match name.starts_with("word/") && name.ends_with(".xml") {
            true => std::str::from_utf8(&bytes).ok().and_then(ruby_as_text),
            false => None,
        };
        if let Some(text) = &marked {
            changed = true;
            bytes = text.clone().into_bytes();
        }
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        out.start_file(name, options).map_err(|e| e.to_string())?;
        out.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    match changed {
        true => Ok(out.finish().map_err(|e| e.to_string())?.into_inner()),
        false => Ok(docx.to_vec()),
    }
}

/// `xml` with each `w:ruby` element replaced by a `w:t` of its words, or
/// `None` when it has none.
fn ruby_as_text(xml: &str) -> Option<String> {
    if !xml.contains("ruby") {
        return None;
    }
    let tree = roxmltree::Document::parse(xml).ok()?;
    let words = |node: roxmltree::Node| -> String {
        node.descendants().filter(|n| n.tag_name().name() == "t").filter_map(|n| n.text()).collect()
    };
    let mut spans: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    for ruby in tree.descendants().filter(|n| n.tag_name().name() == "ruby" && n.parent().is_some_and(|p| p.tag_name().name() == "r")) {
        let child = |name: &str| ruby.children().find(|n| n.tag_name().name() == name);
        let (Some(rt), Some(base)) = (child("rt"), child("rubyBase")) else { continue };
        let prefix = ruby.tag_name().namespace().and_then(|ns| ruby.lookup_prefix(ns)).unwrap_or("w");
        let text = format!("{BASE}{}{READING}{}{END}", words(base), words(rt));
        let escaped = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        spans.push((ruby.range(), format!("<{prefix}:t xml:space=\"preserve\">{escaped}</{prefix}:t>")));
    }
    if spans.is_empty() {
        return None;
    }
    let mut out = xml.to_string();
    for (range, text) in spans.into_iter().rev() {
        out.replace_range(range, &text);
    }
    Some(out)
}

/// The marked readings in pandoc's Markdown in the brace form.
fn readings_restored(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(at) = rest.find(BASE) {
        out.push_str(&rest[..at]);
        let after = &rest[at + BASE.len_utf8()..];
        match (after.find(READING), after.find(END)) {
            (Some(r), Some(e)) if r < e => {
                let base = &after[..r];
                let reading = after[r + READING.len_utf8()..e].trim();
                out.push_str(&format!("{{{base}|{reading}}}"));
                rest = &after[e + END.len_utf8()..];
            }
            _ => rest = after,
        }
    }
    out.push_str(rest);
    out.replace([READING, END], "")
}

/// **The Lua filter pandoc reads with**, run in order: the quotations a
/// style made are set aside, the ones an indent made are undone, styles
/// leave no trace in the Markdown, and what pandoc took out of the text
/// into its metadata goes back where it was.
pub const FILTER: &str = r#"
-- A quotation made by a paragraph style (Quote, Block Text, Intense Quote)
-- is set aside, to be told from one pandoc made of an indent.
local styled = {
  Div = function(div)
    if div.attributes['custom-style'] == nil then return nil end
    local out = pandoc.List()
    for _, block in ipairs(div.content) do
      if block.t == 'BlockQuote' then
        out:insert(pandoc.Div(block.content, pandoc.Attr('', {'yumete-quote'})))
      else
        out:insert(block)
      end
    end
    return out
  end,
}

-- A paragraph indented in Word is a paragraph, not a quotation.
local indented = {
  BlockQuote = function(quote) return quote.content end,
}

-- A list whose items are each a paragraph, and the lists under it, is
-- written tight, as Word shows it: no blank line between the items.
local function tight(list)
  for _, item in ipairs(list.content) do
    if #item == 0 or item[1].t ~= 'Para' then return nil end
    for k = 2, #item do
      if item[k].t ~= 'BulletList' and item[k].t ~= 'OrderedList' then return nil end
    end
  end
  for _, item in ipairs(list.content) do
    item[1] = pandoc.Plain(item[1].content)
  end
  return list
end

-- A code block is fenced, with a fence longer than any run of backticks
-- inside it: an indented block is easy to break by editing round it.
local function fenced(code)
  if #code.classes > 0 then return nil end
  local longest = 2
  for run in code.text:gmatch('`+') do
    if #run > longest then longest = #run end
  end
  local fence = string.rep('`', longest + 1)
  return pandoc.RawBlock('markdown', fence .. '\n' .. code.text .. '\n' .. fence)
end

-- A footnote's text starts at its first word: Word puts a space after
-- the number.
local function note(n)
  local first = n.content[1]
  if first ~= nil and (first.t == 'Para' or first.t == 'Plain') then
    while first.content[1] ~= nil and first.content[1].t == 'Space' do first.content:remove(1) end
  end
  return n
end

local tidy = {
  Div = function(div)
    if div.classes:includes('yumete-quote') then return pandoc.BlockQuote(div.content) end
  end,
  Span = function(span)
    if span.classes:includes('mark') then
      local out = pandoc.List({pandoc.RawInline('markdown', '==')})
      out:extend(span.content)
      out:insert(pandoc.RawInline('markdown', '=='))
      return out
    end
    if span.attributes['custom-style'] ~= nil then return span.content end
  end,
  Image = function(image)
    image.attributes = {}
    return image
  end,
  BulletList = tight,
  OrderedList = tight,
  CodeBlock = fenced,
  Note = note,
}

local function as_blocks(value)
  local kind = pandoc.utils.type(value)
  if kind == 'Inlines' then return {pandoc.Para(value)} end
  if kind == 'Blocks' then return value end
  if kind == 'string' then return {pandoc.Para({pandoc.Str(value)})} end
  local out = {}
  if kind == 'List' then
    for _, each in ipairs(value) do
      for _, block in ipairs(as_blocks(each)) do table.insert(out, block) end
    end
  end
  return out
end

-- The Title, Subtitle, Author, Date and Abstract paragraphs pandoc takes out
-- of the text: back where they were, as paragraphs.
local restored = {
  Pandoc = function(doc)
    local front = pandoc.List()
    for _, key in ipairs({'title', 'subtitle', 'author', 'date', 'abstract'}) do
      if doc.meta[key] ~= nil then front:extend(as_blocks(doc.meta[key])) end
    end
    front:extend(doc.blocks)
    return pandoc.Pandoc(front, {})
  end,
}

return {styled, indented, tidy, restored}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readings_go_through_marked_and_come_back_in_braces() {
        let xml = "<w:document xmlns:w=\"w\"><w:body><w:p><w:r><w:ruby><w:rubyPr/><w:rt><w:r><w:t>hàn</w:t></w:r></w:rt>\
                   <w:rubyBase><w:r><w:t>漢</w:t></w:r></w:rubyBase></w:ruby></w:r></w:p></w:body></w:document>";
        let marked = ruby_as_text(xml).expect("a reading");
        assert!(marked.contains("<w:r><w:t xml:space=\"preserve\">\u{FDD0}漢\u{FDD1}hàn\u{FDD2}</w:t></w:r>"), "{marked}");
        assert_eq!(readings_restored("見\u{FDD0}漢\u{FDD1}hàn\u{FDD2}字"), "見{漢|hàn}字");
        assert_eq!(ruby_as_text("<w:document xmlns:w=\"w\"/>"), None);
    }

    #[test]
    fn front_matter_has_only_the_keys_with_values() {
        let p = Properties { title: Some("A \"quoted\" title".into()), author: None, created: Some("2008-01-02T03:04:05Z".into()), modified: None };
        assert_eq!(front_matter(&p, "a.docx"), "---\ntitle: \"A \\\"quoted\\\" title\"\ncreated: 2008-01-02T03:04:05Z\nsource: \"a.docx\"\n---\n\n");
    }

    #[test]
    fn a_folder_is_planned_as_a_mirrored_tree_and_clashes_are_refused() {
        let root = std::env::temp_dir().join(format!("yumete-word-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("in/sub")).unwrap();
        for name in ["in/a.docx", "in/sub/b.DOC", "in/sub/b.docx", "in/~$a.docx", "in/notes.txt", "in/.hidden.docx"] {
            std::fs::write(root.join(name), b"").unwrap();
        }
        let out = root.join("out");
        let planned = plan(&[root.join("in")], &Options { out: Some(out.clone()), overwrite: false });
        let ok: Vec<_> = planned.iter().filter_map(|(s, t)| t.as_ref().ok().map(|t| (s.clone(), t.clone()))).collect();
        assert_eq!(ok, vec![(root.join("in/a.docx"), out.join("a.md"))]);
        assert_eq!(planned.iter().filter(|(_, t)| t.is_err()).count(), 2, "{planned:?}");
        let one = plan(&[root.join("in/a.docx")], &Options { out: Some(root.join("x.md")), overwrite: false });
        assert_eq!(one[0].1.as_ref().unwrap(), &root.join("x.md"));
        let beside = plan(&[root.join("in/a.docx")], &Options::default());
        assert_eq!(beside[0].1.as_ref().unwrap(), &root.join("in/a.md"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
