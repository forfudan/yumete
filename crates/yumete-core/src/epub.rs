//! **`:export epub`**: the HTML export as a book a reader app
//! opens — Apple Books, Thorium, a Kobo — vertical and turning from right to
//! left when the editor is.
//!
//! The text and the stylesheet are the HTML page's ([`crate::export`]): the
//! same Latin set down the page, corner brackets, vertical tables, figures and
//! drawn formulas. What is the book's own is the packaging, EPUB 3 at its
//! smallest:
//!
//! - `mimetype`, first and stored, as the format requires, so a program can
//!   tell what the zip is from its first bytes;
//! - `META-INF/container.xml`, which points at the package;
//! - `OEBPS/content.opf`, the package: title, language, an identifier made
//!   from the text, the time it was made, and — for a vertical book —
//!   `page-progression-direction="rtl"`, which is what makes a reader turn
//!   pages the way a Chinese book turns;
//! - `OEBPS/nav.xhtml`, the table of contents, from the headings, each
//!   chapter with the headings under it;
//! - `OEBPS/text-1.xhtml`, `text-2.xhtml`…, a page for each chapter
//!   ([`chapters`]), and `OEBPS/style.css`, the stylesheet they share;
//! - `OEBPS/images/`, each drawn formula and each picture, a figure's or one
//!   in a line, as a file of its own: a data URL is not a resource a package
//!   can list, and a book cannot reach outside itself for a picture.
//!
//! Everything is stored, not deflated: the formulas are PNGs already, a
//! chapter is a few tens of kilobytes, and a zip writer with no compressor is
//! the dozen lines of [`zip`] rather than a dependency.

use std::fmt::Write as _;

use crate::export::Style;

/// The book's text as one page of XHTML, and the formula pictures it
/// refers to, each with the name it has in the book.
pub(crate) fn chapter(text: &str, style: &Style) -> (String, Vec<(String, Vec<u8>)>) {
    let parts = parts(text, style);
    (page(parts.lang, &parts.title, &parts.body), parts.images)
}

/// The whole EPUB, as the bytes of the file.
pub fn book(text: &str, style: &Style) -> Vec<u8> {
    let Parts { lang, title, body, css, images, headings } = parts(text, style);
    let id = identifier(text, &title);
    let (documents, chapter_level) = chapters(&body);
    let name = |n: usize| format!("text-{}.xhtml", n + 1);
    // Where each `id` is, so a link to it from another chapter names that
    // chapter's file.
    let mut home: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (n, document) in documents.iter().enumerate() {
        for found in ID.captures_iter(document) {
            home.entry(found[1].to_string()).or_insert(n);
        }
    }
    let mut manifest = String::from("<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n");
    let mut spine = String::new();
    for n in 0..documents.len() {
        let _ = writeln!(manifest, "<item id=\"t{}\" href=\"{}\" media-type=\"application/xhtml+xml\"/>", n + 1, name(n));
        let _ = writeln!(spine, "<itemref idref=\"t{}\"/>", n + 1);
    }
    manifest.push_str("<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>\n");
    for (n, (name, _)) in images.iter().enumerate() {
        let kind = media_type(name).unwrap_or("image/png");
        let _ = writeln!(manifest, "<item id=\"f{}\" href=\"{name}\" media-type=\"{kind}\"/>", n + 1);
    }
    // **Right to left exactly when the page is vertical**, as
    // the stylesheet sets it: a vertical page read as English set its lines
    // from the right and turned its pages from the left.
    let (direction, mode) = match style.settings.vertical {
        true => (" page-progression-direction=\"rtl\"", "\n<meta name=\"primary-writing-mode\" content=\"vertical-rl\"/>"),
        false => ("", ""),
    };
    let opf = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"id\" xml:lang=\"{lang}\">\n\
         <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n\
         <dc:identifier id=\"id\">urn:uuid:{id}</dc:identifier>\n\
         <dc:title>{title}</dc:title>\n\
         <dc:language>{lang}</dc:language>\n\
         <meta property=\"dcterms:modified\">{modified}</meta>{mode}\n\
         </metadata>\n\
         <manifest>\n{manifest}</manifest>\n\
         <spine{direction}>\n{spine}</spine>\n\
         </package>\n",
        title = xml_text(&title),
        modified = utc_now(),
    );
    // The contents: every chapter and the two levels of headings under it,
    // and never less than down to `<h3>`, each pointing into its own page.
    let deepest = (chapter_level + 2).max(3);
    let entries: Vec<Entry> = headings
        .iter()
        .filter(|(level, _, _)| *level <= deepest)
        .map(|(level, anchor, words)| Entry {
            level: *level,
            href: format!("{}#{anchor}", name(home.get(anchor).copied().unwrap_or(0))),
            words: if words.is_empty() { anchor.clone() } else { words.clone() },
            under: Vec::new(),
        })
        .collect();
    let mut toc = String::new();
    contents(&nested(entries), &mut toc);
    if toc.is_empty() {
        let _ = writeln!(toc, "<li><a href=\"{}\">{}</a></li>", name(0), xml_text(&title));
    }
    let nav = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" lang=\"{lang}\" xml:lang=\"{lang}\">\n\
         <head><meta charset=\"utf-8\"/><title>{title}</title><link rel=\"stylesheet\" href=\"style.css\"/></head>\n\
         <body>\n<nav epub:type=\"toc\" id=\"toc\">\n<ol>\n{toc}</ol>\n</nav>\n</body>\n</html>\n",
        title = xml_text(&title),
    );
    let container = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n\
         <rootfiles>\n<rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/>\n</rootfiles>\n\
         </container>\n";
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("mimetype".to_string(), b"application/epub+zip".to_vec()),
        ("META-INF/container.xml".to_string(), container.as_bytes().to_vec()),
        ("OEBPS/content.opf".to_string(), opf.into_bytes()),
        ("OEBPS/nav.xhtml".to_string(), nav.into_bytes()),
    ];
    for (n, document) in documents.iter().enumerate() {
        // A link to a place in another chapter names its file; one to a
        // place in this chapter, or to nowhere, is left as it is.
        let document = HREF.replace_all(document, |found: &regex::Captures| match home.get(&found[1]) {
            Some(&at) if at != n => format!("href=\"{}#{}\"", name(at), &found[1]),
            _ => found[0].to_string(),
        });
        // Each page is titled by its first heading, or by the book.
        let named = headings
            .iter()
            .find(|(_, anchor, words)| home.get(anchor) == Some(&n) && !words.is_empty())
            .map_or(title.as_str(), |(_, _, words)| words.as_str());
        files.push((format!("OEBPS/{}", name(n)), page(lang, named, &document).into_bytes()));
    }
    files.push(("OEBPS/style.css".to_string(), css.into_bytes()));
    for (name, png) in images {
        files.push((format!("OEBPS/{name}"), png));
    }
    zip(&files)
}

/// An `id` in a page, and a link to a place in it.
static ID: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r#"\sid="([^"]*)""#).expect("a pattern"));
static HREF: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r##"href="#([^"]*)""##).expect("a pattern"));

/// One heading in the table of contents, and the headings under it.
struct Entry {
    level: usize,
    href: String,
    words: String,
    under: Vec<Entry>,
}

/// The headings as a tree: each under the last heading above it of a
/// higher level, however many levels it skips.
fn nested(entries: Vec<Entry>) -> Vec<Entry> {
    fn put(list: &mut Vec<Entry>, entry: Entry) {
        match list.last_mut() {
            Some(last) if last.level < entry.level => put(&mut last.under, entry),
            _ => list.push(entry),
        }
    }
    let mut tree = Vec::new();
    for entry in entries {
        put(&mut tree, entry);
    }
    tree
}

/// The tree of headings as the `<li>`s of the navigation document's list.
fn contents(entries: &[Entry], out: &mut String) {
    for entry in entries {
        let _ = write!(out, "<li><a href=\"{}\">{}</a>", entry.href, xml_text(&entry.words));
        if !entry.under.is_empty() {
            out.push_str("\n<ol>\n");
            contents(&entry.under, out);
            out.push_str("</ol>\n");
        }
        out.push_str("</li>\n");
    }
}

/// **The book cut into chapters**, a page of XHTML each, so a reader opens
/// a long book at the chapter it is in rather than laying out the whole of
/// it; and the heading level the chapters start at.
///
/// A chapter starts at each heading of the highest level the text uses at
/// its top level, not one inside a quotation or a list. One heading of that
/// level first in the text, with lower ones after it, is the book's title
/// and not a chapter: `# 书名` then `## 第一章`… is cut at each `##`, the
/// title going with what is before the first chapter. What is before the
/// first chapter is a page of its own, when there is anything.
///
/// **A note stays with the chapter that first cites it**, in a list of the
/// chapter's own notes at its end, numbered as in the whole book; a note
/// cited nowhere goes with the last chapter.
fn chapters(body: &str) -> (Vec<String>, usize) {
    let (text, notes) = notes_apart(body);
    let headings = top_level_headings(text);
    let Some(top) = headings.iter().map(|(_, level)| *level).min() else { return (vec![body.to_string()], 1) };
    let below = headings.iter().map(|(_, level)| *level).filter(|level| *level > top).min();
    let titled = headings[0].1 == top && headings.iter().filter(|(_, level)| *level == top).count() == 1;
    let level = match below {
        Some(below) if titled => below,
        _ => top,
    };
    let mut cuts: Vec<usize> = headings.iter().filter(|(_, l)| *l <= level).map(|(at, _)| *at).collect();
    cuts.insert(0, 0);
    cuts.push(text.len());
    let mut documents: Vec<String> =
        cuts.windows(2).map(|w| text[w[0]..w[1]].to_string()).filter(|page| !page.trim().is_empty()).collect();
    if documents.is_empty() {
        documents.push(String::new());
    }
    let Some(notes) = notes else { return (documents, level) };
    let mut kept: Vec<Vec<(usize, &str)>> = vec![Vec::new(); documents.len()];
    for (n, note) in notes {
        let cited = format!("href=\"#fn-{n}\"");
        let at = documents.iter().position(|page| page.contains(&cited)).unwrap_or(documents.len() - 1);
        kept[at].push((n, note));
    }
    for (page, notes) in documents.iter_mut().zip(kept) {
        if notes.is_empty() {
            continue;
        }
        page.push_str("<section class=\"footnotes\">\n<hr/>\n<ol>\n");
        for (n, note) in notes {
            let _ = writeln!(page, "<li id=\"fn-{n}\" value=\"{n}\">{note}</li>");
        }
        page.push_str("</ol>\n</section>\n");
    }
    (documents, level)
}

/// The page's text and, apart, its notes as [`crate::export::html_body`]
/// lists them at its end: each note's number and its words. `None` when
/// the text ends in no such list, and the text is then the whole page.
fn notes_apart(body: &str) -> (&str, Option<Vec<(usize, &str)>>) {
    const OPEN: &str = "<section class=\"footnotes\">\n<hr/>\n<ol>\n";
    const SHUT: &str = "</ol>\n</section>";
    let whole = body.trim_end();
    let found = body.rfind(OPEN).filter(|&at| at == 0 || body[..at].ends_with('\n'));
    let Some(at) = found.filter(|_| whole.ends_with(SHUT)) else { return (body, None) };
    let mut rest = &whole[at + OPEN.len()..whole.len() - SHUT.len()];
    let mut notes = Vec::new();
    while !rest.is_empty() {
        let Some((n, after)) = rest.strip_prefix("<li id=\"fn-").and_then(|r| r.split_once("\">")) else { return (body, None) };
        let Ok(n) = n.parse::<usize>() else { return (body, None) };
        let Some(end) = closing_li(after) else { return (body, None) };
        notes.push((n, &after[..end]));
        let after = &after[end + "</li>".len()..];
        rest = after.strip_prefix('\n').unwrap_or(after);
    }
    (&body[..at], Some(notes))
}

/// Where the `</li>` is that closes a list item whose words start `text`,
/// past any list item inside it.
fn closing_li(text: &str) -> Option<usize> {
    let mut depth = 1usize;
    for (k, _) in text.match_indices("li") {
        let open = text[..k].ends_with('<') && text[k + 2..].starts_with(['>', ' ']);
        let shut = text[..k].ends_with("</") && text[k + 2..].starts_with('>');
        if open {
            depth += 1;
        } else if shut {
            depth -= 1;
            if depth == 0 {
                return Some(k - 2);
            }
        }
    }
    None
}

/// Where each heading at the top level of the page starts, a line of its
/// own, and its level.
fn top_level_headings(body: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut at = 0;
    while let Some(found) = body[at..].find('<') {
        let from = at + found;
        if body[from..].starts_with("<!--") {
            at = body[from..].find("-->").map_or(body.len(), |end| from + end + 3);
            continue;
        }
        let Some(tag) = read_tag(&body[from..]) else {
            at = from + 1;
            continue;
        };
        if tag.closing {
            depth = depth.saturating_sub(1);
        } else if !tag.closed {
            let level = tag.name.strip_prefix('h').and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=6).contains(n));
            if let Some(level) = level.filter(|_| depth == 0 && (from == 0 || body[..from].ends_with('\n'))) {
                out.push((from, level));
            }
            depth += 1;
        }
        at = from + tag.len;
    }
    out
}

/// What a book is made of before it is packed.
struct Parts {
    lang: &'static str,
    title: String,
    /// The text, as what goes inside a page's `<body>`.
    body: String,
    css: String,
    images: Vec<(String, Vec<u8>)>,
    /// Each heading's level, anchor and words, for the table of contents.
    headings: Vec<(usize, String, String)>,
}

fn parts(text: &str, style: &Style) -> Parts {
    let text = crate::markdown::strip_comments(text);
    let lang = crate::export::language_of(&text);
    let css = crate::export::stylesheet(style, lang, true);
    let body = crate::export::html_body(&text, style, lang);
    let body = pictures_in(&body, style.base.as_deref());
    let (body, images) = pictures_out(&body);
    let body = links_kept_in(&body);
    let (body, mut headings) = anchored(&well_formed(&xhtml(&body)));
    // A heading inside a quotation or a list is the quotation's, not one
    // of the book's: not in its contents, and not its title.
    let outline: std::collections::HashSet<String> = top_level_headings(&body)
        .iter()
        .filter_map(|(at, _)| read_tag(&body[*at..])?.attributes.into_iter().find(|(name, _)| name == "id")?.1)
        .collect();
    headings.retain(|(_, anchor, _)| outline.contains(anchor));
    let title = headings
        .iter()
        .find(|(_, _, words)| !words.is_empty())
        .map(|(_, _, words)| words.clone())
        .unwrap_or_else(|| style.title.rsplit_once('.').map_or(style.title.clone(), |(stem, _)| stem.to_string()));
    Parts { lang, title, body, css, images, headings }
}

/// A page of the book: `body` in the XHTML around it, titled `title`.
fn page(lang: &str, title: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" lang=\"{lang}\" xml:lang=\"{lang}\">\n\
         <head><meta charset=\"utf-8\"/><title>{}</title><link rel=\"stylesheet\" href=\"style.css\"/></head>\n\
         <body>\n{body}</body>\n</html>\n",
        xml_text(title),
    )
}

/// The picture types a book may hold, by the extension a file is named
/// with.
const PICTURES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("svg", "image/svg+xml"),
];

/// The media type of a picture named `name`, by its extension.
fn media_type(name: &str) -> Option<&'static str> {
    let (_, extension) = name.rsplit_once('.')?;
    PICTURES.iter().find(|(e, _)| extension.eq_ignore_ascii_case(e)).map(|(_, kind)| *kind)
}

/// **Each picture taken into the book**, a figure's and
/// one in a line: read from beside the source, its path
/// percent-decoded as the preview reads it (`my%20pic.png`), and written
/// into the page as a `data:` URL, for [`pictures_out`] to file. A picture
/// that cannot be read — not there, on the web, of a type a book does not
/// hold, or a buffer never saved — is left out: a figure keeps its caption,
/// a picture in a line its text, since a book that points outside itself
/// does not open in a reader that checks.
fn pictures_in(body: &str, base: Option<&std::path::Path>) -> String {
    let body = written_pictures_in(body, base);
    let body = body.as_str();
    const TAGS: [&str; 2] = ["<img class=\"figure\" src=\"", "<img class=\"inline\" src=\""];
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some((at, open)) = TAGS.iter().filter_map(|p| rest.find(p).map(|at| (at, *p))).min() {
        out.push_str(&rest[..at]);
        let after = &rest[at + open.len()..];
        let (Some(end), Some(close)) = (after.find('"'), after.find('>')) else {
            out.push_str(&rest[at..]);
            return out;
        };
        let src = after[..end].replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
        let src = crate::markdown::percent_decoded(&src);
        let read = base
            .filter(|_| !src.contains("://"))
            .and_then(|base| Some((media_type(&src)?, std::fs::read(base.join(&src)).ok()?)));
        if let Some((kind, bytes)) = &read {
            let _ = write!(out, "{open}data:{kind};base64,{}{}", crate::preview::base64(bytes), &after[end..=close]);
        }
        let tag = &after[..=close];
        rest = &after[close + 1..];
        match (read.is_some(), open == TAGS[0]) {
            (true, _) => {}
            // The line a figure's picture stood on goes with it.
            (false, true) => rest = rest.strip_prefix('\n').unwrap_or(rest),
            // A picture in a line leaves its words, as the preview shows
            // them when it has no picture.
            (false, false) => out.push_str(&alt_of(tag)),
        }
    }
    out.push_str(rest);
    out
}

/// **A picture the writer wrote as HTML**, `<img src="a.png" alt="…">`,
/// taken into the book as a picture written in Markdown is (a picture
/// pointing out of the book is RSC-007), its other attributes
/// kept; one that cannot be read leaves its `alt`. The page's own pictures,
/// a figure's, one in a line and a drawn formula, are [`pictures_in`]'s and
/// [`pictures_out`]'s.
fn written_pictures_in(body: &str, base: Option<&std::path::Path>) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(at) = rest.find("<img") {
        out.push_str(&rest[..at]);
        let tag = read_tag(&rest[at..]).filter(|tag| tag.name == "img" && !tag.closing);
        let Some(tag) = tag else {
            out.push_str("<img");
            rest = &rest[at + 4..];
            continue;
        };
        let written = &rest[at..at + tag.len];
        rest = &rest[at + tag.len..];
        let value = |name: &str| tag.attributes.iter().find(|(n, _)| n == name).and_then(|(_, v)| v.clone());
        let src = value("src").unwrap_or_default();
        let ours = value("class").is_some_and(|c| matches!(c.as_str(), "figure" | "inline") || c.starts_with("math"));
        if ours || src.starts_with("data:") {
            out.push_str(written);
            continue;
        }
        let path = crate::markdown::percent_decoded(&src.replace("&amp;", "&"));
        let read = base
            .filter(|_| !path.contains("://") && !path.is_empty())
            .and_then(|base| Some((media_type(&path)?, std::fs::read(base.join(&path)).ok()?)));
        match read {
            Some((kind, bytes)) => {
                let data = format!("data:{kind};base64,{}", crate::preview::base64(&bytes));
                let attributes: Vec<(String, Option<String>)> = tag
                    .attributes
                    .iter()
                    .map(|(n, v)| match n.as_str() {
                        "src" => (n.clone(), Some(data.clone())),
                        _ => (n.clone(), v.clone()),
                    })
                    .collect();
                let has_alt = attributes.iter().any(|(n, _)| n == "alt");
                let mut kept = Tag { attributes, ..tag };
                if !has_alt {
                    kept.attributes.push(("alt".to_string(), Some(String::new())));
                }
                out.push_str(&kept.xml(true).unwrap_or_default());
            }
            None => out.push_str(&value("alt").map(|alt| xml_text(&crate::export::plain_words(&alt))).unwrap_or_default()),
        }
    }
    out.push_str(rest);
    out
}

/// The `alt` of an `<img>` tag, as it was written into the tag: escaped
/// already, and so text for the page as it is.
fn alt_of(tag: &str) -> String {
    tag.split_once(" alt=\"").and_then(|(_, after)| after.split_once('"')).map(|(alt, _)| alt.to_string()).unwrap_or_default()
}

/// Each `data:` picture — a drawn formula, a figure taken in — written out
/// into a file of its own, and the page pointing at it.
///
/// **One file for each picture**, however many times it is shown: a
/// picture whose bytes are already filed points at that file.
fn pictures_out(body: &str) -> (String, Vec<(String, Vec<u8>)>) {
    const DATA: &str = "src=\"data:";
    let mut out = String::with_capacity(body.len());
    let mut images: Vec<(String, Vec<u8>)> = Vec::new();
    let mut filed: std::collections::HashMap<Vec<u8>, usize> = std::collections::HashMap::new();
    let mut rest = body;
    while let Some(at) = rest.find(DATA) {
        out.push_str(&rest[..at]);
        let after = &rest[at + DATA.len()..];
        let Some(end) = after.find('"') else {
            out.push_str(&rest[at..]);
            rest = "";
            break;
        };
        let url = &after[..end];
        let picture = url.split_once(";base64,").and_then(|(kind, data)| {
            let (extension, _) = PICTURES.iter().find(|(_, k)| *k == kind)?;
            Some((*extension, crate::preview::decode_base64(data)?))
        });
        match picture {
            Some((extension, bytes)) => {
                let at = match filed.get(&bytes) {
                    Some(&at) => at,
                    None => {
                        images.push((format!("images/f{}.{extension}", images.len() + 1), bytes.clone()));
                        filed.insert(bytes, images.len() - 1);
                        images.len() - 1
                    }
                };
                let _ = write!(out, "src=\"{}\"", images[at].0);
            }
            None => out.push_str(&rest[at..at + DATA.len() + end + 1]),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    (out, images)
}

/// **Links a book can follow**, and the rest as their words (a link to a
/// file the book does not hold, `[x](x.html)`, is one epubcheck refuses,
/// RSC-007): a link to the web or to a place in the page
/// stays a link; one to a file beside the manuscript, which the book does
/// not hold, keeps its words, the target in their `title`. So does one in
/// any scheme but `http`, `https` and `mailto` (a `file:` link is
/// RSC-030), which a reader cannot follow from inside a book.
fn links_kept_in(body: &str) -> String {
    const OPEN: &str = "<a href=\"";
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(at) = rest.find(OPEN) {
        out.push_str(&rest[..at]);
        let after = &rest[at + OPEN.len()..];
        let (Some(end), Some(close)) = (after.find('"'), after.find('>')) else {
            out.push_str(&rest[at..]);
            return out;
        };
        let href = &after[..end];
        let scheme = href.split_once(':').is_some_and(|(s, _)| ["http", "https", "mailto"].iter().any(|k| s.eq_ignore_ascii_case(k)));
        if scheme || href.starts_with('#') || href.starts_with("//") {
            out.push_str(&rest[at..at + OPEN.len() + close + 1]);
            rest = &after[close + 1..];
            continue;
        }
        let words = &after[close + 1..];
        let Some(shut) = words.find("</a>") else {
            out.push_str(&rest[at..]);
            return out;
        };
        let _ = write!(out, "<span class=\"link\" title=\"{href}\">{}</span>", &words[..shut]);
        rest = &words[shut + "</a>".len()..];
    }
    out.push_str(rest);
    out
}

/// HTML made XML: the void elements closed (`<br>` as `<br/>`), the one
/// named entity HTML has that XML lacks, `&nbsp;`, written as a number, and
/// no character XML refuses ([`crate::export::legal_text`]). Inline HTML the
/// writer typed passes through as it did into the HTML page, so a tag left
/// open there is still open here.
fn xhtml(body: &str) -> String {
    use std::sync::LazyLock;
    static VOID: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"<(area|br|col|embed|hr|img|input|source|track|wbr)\b([^>]*?)\s*/?>").expect("a pattern")
    });
    let body = crate::export::legal_text(body);
    VOID.replace_all(&body, "<${1}${2}/>").replace("&nbsp;", "&#160;")
}

/// Every heading with the `id` the page gave it from its words
/// (`export::heading_ids`), and the list of them with their level and words
/// for the table of contents.
fn anchored(body: &str) -> (String, Vec<(usize, String, String)>) {
    let mut headings = Vec::new();
    for line in body.lines() {
        let level = line.strip_prefix("<h").and_then(|rest| rest.bytes().next()).filter(|d| (b'1'..=b'6').contains(d));
        let Some(level) = level.map(|d| usize::from(d - b'0')) else { continue };
        let Some(rest) = line[3..].strip_prefix(" id=\"") else { continue };
        let Some((anchor, rest)) = rest.split_once("\">") else { continue };
        let close = format!("</{}", &line[1..3]);
        let inner = rest.find(&close).map(|end| &rest[..end]).unwrap_or_default();
        // **A heading that is only a picture is named by the picture**:
        // `# ![第一章](title.png)` has no words, and an empty entry in the
        // table of contents, or an empty title, is what epubcheck refuses
        // (RSC-005). Its `alt`; a heading with neither is given its anchor in
        // the table of contents, and does not title the book.
        let words = Some(crate::export::plain_words(inner))
            .filter(|w| !w.trim().is_empty())
            .unwrap_or_else(|| alt_words(inner));
        headings.push((level, anchor.to_string(), words.trim().to_string()));
    }
    (body.to_string(), headings)
}

/// The `alt` words of the pictures in a run of markup, one space between.
fn alt_words(markup: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    for (at, _) in markup.match_indices("<img") {
        let tag = &markup[at..markup[at..].find('>').map_or(markup.len(), |end| at + end)];
        if let Some(alt) = tag.split_once(" alt=\"").and_then(|(_, rest)| rest.split_once('"')).map(|(alt, _)| alt) {
            words.push(crate::export::plain_words(alt));
        }
    }
    words.join(" ")
}

/// **Inline HTML the writer typed, made XML** (a `<span>` left open makes
/// the page one a reader refuses, RSC-016). A
/// browser closes what is left open and an XML reader does not, so here a
/// tag that is closed in its place stays a tag; an attribute with no value
/// (`<details open>`) or with an unquoted one is written out in full, and an
/// `&` in a value that starts no entity is escaped; a tag left open, a
/// closing tag with nothing to close and a tag XML cannot read go into the
/// page as the words they were typed as. The page's own markup is closed
/// where it opens, so only the writer's is ever touched.
///
/// **And where HTML allows it**: what is typed stands in a line of text, a
/// paragraph's or a heading's, where only phrasing goes (`<span>`, `<b>`,
/// `<kbd>`…); a block typed there (`<div>`, `<details>`) is words too, and
/// a closing tag never closes past the paragraph it stands in.
fn well_formed(body: &str) -> String {
    const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"];
    // What may stand in a line of text (HTML's phrasing content, and a
    // ruby's reading), and what holds nothing else.
    const PHRASING: &[&str] = &[
        "a", "abbr", "area", "audio", "b", "bdi", "bdo", "br", "button", "canvas", "cite", "code", "data", "datalist", "del", "dfn", "em",
        "embed", "i", "iframe", "img", "input", "ins", "kbd", "label", "map", "mark", "meter", "object", "output", "picture", "progress",
        "q", "ruby", "rt", "rp", "s", "samp", "select", "slot", "small", "span", "strong", "sub", "sup", "template", "textarea", "time",
        "u", "var", "video", "wbr",
    ];
    const LINES: &[&str] = &["p", "h1", "h2", "h3", "h4", "h5", "h6", "pre"];
    let in_a_line = |open: &[(String, usize)]| open.iter().any(|(n, _)| PHRASING.contains(&n.as_str()) || LINES.contains(&n.as_str()));
    /// What becomes of one tag.
    enum Fate {
        Keep,
        Words,
        Written(String),
    }
    let mut tags: Vec<(usize, usize, Fate)> = Vec::new();
    // The tags open, innermost last: their name and their place in `tags`.
    let mut open: Vec<(String, usize)> = Vec::new();
    let mut at = 0;
    while let Some(found) = body[at..].find('<') {
        let from = at + found;
        if body[from..].starts_with("<!--") {
            match body[from..].find("-->") {
                Some(end) => at = from + end + 3,
                None => {
                    tags.push((from, from + 1, Fate::Words));
                    at = from + 1;
                }
            }
            continue;
        }
        let Some(tag) = read_tag(&body[from..]) else {
            tags.push((from, from + 1, Fate::Words));
            at = from + 1;
            continue;
        };
        let to = from + tag.len;
        let name = tag.name.clone();
        let void = VOID.contains(&name.as_str());
        let fate = match tag.xml(void) {
            Some(written) if written == body[from..to] => Fate::Keep,
            Some(written) => Fate::Written(written),
            None => Fate::Words,
        };
        // A closing tag looks no further out than the line it stands in.
        let reach = open.iter().rposition(|(n, _)| LINES.contains(&n.as_str()) && *n != name).map_or(0, |k| k + 1);
        if !tag.closing && in_a_line(&open) && !PHRASING.contains(&name.as_str()) {
            tags.push((from, to, Fate::Words));
            at = to;
            continue;
        }
        match (tag.closing, void || tag.closed) {
            (true, _) if void => tags.push((from, to, Fate::Words)),
            (true, _) => match open[reach..].iter().rposition(|(n, _)| *n == name).map(|k| k + reach) {
                Some(k) => {
                    // What was opened inside it and never closed is words.
                    for (_, place) in open.drain(k..).skip(1) {
                        tags[place].2 = Fate::Words;
                    }
                    tags.push((from, to, fate));
                }
                None => tags.push((from, to, Fate::Words)),
            },
            (false, true) => tags.push((from, to, fate)),
            (false, false) => {
                open.push((name, tags.len()));
                tags.push((from, to, fate));
            }
        }
        at = to;
    }
    for (_, place) in open {
        tags[place].2 = Fate::Words;
    }
    let mut out = String::with_capacity(body.len() + 16);
    let mut at = 0;
    for (from, to, fate) in tags {
        out.push_str(&body[at..from]);
        match fate {
            Fate::Keep => out.push_str(&body[from..to]),
            Fate::Written(written) => out.push_str(&written),
            Fate::Words => out.push_str(&words_text(&body[from..to])),
        }
        at = to;
    }
    out.push_str(&body[at..]);
    out
}

/// One tag as [`read_tag`] reads it.
struct Tag {
    closing: bool,
    /// Its name, in small letters.
    name: String,
    /// Each attribute and its value, `None` for a bare one.
    attributes: Vec<(String, Option<String>)>,
    /// Written `<br/>`.
    closed: bool,
    /// How long it is, in bytes.
    len: usize,
}

impl Tag {
    /// The tag as XML: a bare attribute given its own name as its value,
    /// every value in double quotes, an `&` that starts no entity escaped,
    /// a void element closed. `None` when an attribute is there twice.
    fn xml(&self, void: bool) -> Option<String> {
        if self.closing {
            return Some(format!("</{}>", self.name));
        }
        let mut out = format!("<{}", self.name);
        let mut seen: Vec<&str> = Vec::new();
        // **`align` is written as the style it means**: HTML still draws
        // `<p align="center">`, and an EPUB's XHTML has no such attribute
        // (epubcheck, RSC-005).
        let align = self
            .attributes
            .iter()
            .find(|(name, _)| name == "align")
            .and_then(|(_, v)| v.as_deref())
            .map(|v| v.trim().to_ascii_lowercase())
            .filter(|v| matches!(v.as_str(), "left" | "right" | "center" | "justify"))
            .map(|v| format!("text-align: {v}"));
        let mut styled = false;
        for (name, value) in &self.attributes {
            if seen.contains(&name.as_str()) {
                return None;
            }
            seen.push(name);
            if name == "align" {
                continue;
            }
            let value = value.as_deref().unwrap_or(name);
            let value = match (name.as_str(), &align) {
                ("style", Some(align)) => {
                    styled = true;
                    format!("{}; {align}", value.trim_end().trim_end_matches(';'))
                }
                _ => value.to_string(),
            };
            let _ = write!(out, " {name}=\"{}\"", attribute_text(&value));
        }
        if let Some(align) = align.filter(|_| !styled) {
            let _ = write!(out, " style=\"{align}\"");
        }
        out.push_str(if void || self.closed { "/>" } else { ">" });
        Some(out)
    }
}

/// An attribute's value for a double-quoted XML attribute: `<` and `"`
/// escaped, and an `&` that starts no entity XML knows.
fn attribute_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for (at, c) in value.char_indices() {
        match c {
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '&' if !entity_at(&value[at..]) => out.push_str("&amp;"),
            c => out.push(c),
        }
    }
    out
}

/// A tag that goes into the page as the words it was typed as: as
/// [`attribute_text`], and its `>` escaped too: in a line of text, as
/// against an attribute, `]]>` is a thing XML refuses, so a
/// `<div title="]]>">` typed in a paragraph would make the page ill-formed.
fn words_text(tag: &str) -> String {
    attribute_text(tag).replace('>', "&gt;")
}

/// Whether `text` starts with an entity XML reads: one of its five, or the
/// number of a character XML allows — not `&#0;`, nor any other control
/// but tab and the two line ends, which make the page ill-formed wherever
/// they stand.
fn entity_at(text: &str) -> bool {
    let Some(end) = text.find(';') else { return false };
    let name = &text[1..end];
    let number = match name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
        Some(hex) if !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit()) => u32::from_str_radix(hex, 16).ok(),
        Some(_) => return false,
        None => match name.strip_prefix('#') {
            Some(n) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => n.parse::<u32>().ok(),
            Some(_) => return false,
            None => return matches!(name, "amp" | "lt" | "gt" | "quot" | "apos"),
        },
    };
    number.and_then(char::from_u32).is_some_and(|c| matches!(c, '\t' | '\n' | '\r') || (c >= ' ' && !matches!(c, '\u{FFFE}' | '\u{FFFF}')))
}

/// **The tag at the start of `text`**, read as HTML reads one: a name,
/// then attributes, each with a value in double or single quotes, a value
/// with no quotes, or none. `None` when it is not a tag.
fn read_tag(text: &str) -> Option<Tag> {
    let bytes = text.as_bytes();
    let mut at = 1;
    let closing = bytes.get(at) == Some(&b'/');
    at += usize::from(closing);
    let name_at = at;
    if !bytes.get(at)?.is_ascii_alphabetic() {
        return None;
    }
    while bytes.get(at).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'-') {
        at += 1;
    }
    let name = text[name_at..at].to_ascii_lowercase();
    let mut attributes = Vec::new();
    let space = |at: &mut usize| {
        while bytes.get(*at).is_some_and(u8::is_ascii_whitespace) {
            *at += 1;
        }
    };
    loop {
        let before = at;
        space(&mut at);
        match bytes.get(at)? {
            b'>' => return Some(Tag { closing, name, attributes, closed: false, len: at + 1 }),
            b'/' if bytes.get(at + 1) == Some(&b'>') && !closing => {
                return Some(Tag { closing, name, attributes, closed: true, len: at + 2 });
            }
            _ if closing || at == before => return None,
            _ => {}
        }
        let attribute_at = at;
        if !bytes[at].is_ascii_alphabetic() && !matches!(bytes[at], b'_' | b':') {
            return None;
        }
        while bytes.get(at).is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-')) {
            at += 1;
        }
        let attribute = text[attribute_at..at].to_ascii_lowercase();
        let mut after = at;
        space(&mut after);
        if bytes.get(after) != Some(&b'=') {
            attributes.push((attribute, None));
            continue;
        }
        at = after + 1;
        space(&mut at);
        let value = match bytes.get(at)? {
            &quote @ (b'"' | b'\'') => {
                let end = at + 1 + text[at + 1..].find(quote as char)?;
                let value = &text[at + 1..end];
                at = end + 1;
                value
            }
            _ => {
                let from = at;
                while bytes.get(at).is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'>' | b'"' | b'\'' | b'<' | b'=' | b'`')) {
                    at += 1;
                }
                if at == from {
                    return None;
                }
                &text[from..at]
            }
        };
        attributes.push((attribute, Some(value.to_string())));
    }
}

/// `&`, `<` and `>` escaped for XML text, and nothing in it XML refuses.
fn xml_text(text: &str) -> String {
    crate::export::legal_text(text).replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// A UUID made from the text and its title, so the same manuscript exported
/// twice is the same book to a reader's library, and an edited one is the
/// same book too only if nothing changed.
fn identifier(text: &str, title: &str) -> String {
    use std::hash::{Hash, Hasher};
    let half = |salt: u8| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        salt.hash(&mut h);
        title.hash(&mut h);
        text.hash(&mut h);
        h.finish()
    };
    let (a, b) = (half(1), half(2));
    // Version 4's and the variant's bits, so it is a well-formed UUID.
    let a = (a & 0xFFFF_FFFF_FFFF_0FFF) | 0x0000_0000_0000_4000;
    let b = (b & 0x3FFF_FFFF_FFFF_FFFF) | 0x8000_0000_0000_0000;
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        a >> 32,
        (a >> 16) & 0xFFFF,
        a & 0xFFFF,
        b >> 48,
        b & 0xFFFF_FFFF_FFFF
    )
}

/// Now, in UTC, as EPUB's `dcterms:modified` wants it: `2026-11-08T01:02:03Z`.
pub(crate) fn utc_now() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    utc(seconds)
}

/// A second since 1970 as a UTC date and time (Howard Hinnant's
/// `civil_from_days`).
fn utc(seconds: i64) -> String {
    let (days, of_day) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    )
}

/// **A zip of `files`, in that order, every one stored** — the subset of the
/// format an EPUB needs: no compression, no directories, no extras.
fn zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    // 1980-01-01 00:00, the zip format's own zero: the time a reader shows is
    // the package's `dcterms:modified`, not this.
    let (time, date) = (0u16, 0x21u16);
    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        let head = |sig: u32, central: bool| {
            let mut h = Vec::new();
            h.extend_from_slice(&sig.to_le_bytes());
            if central {
                h.extend_from_slice(&20u16.to_le_bytes());
            }
            for v in [20u16, 0, 0, time, date] {
                h.extend_from_slice(&v.to_le_bytes());
            }
            for v in [crc, size, size] {
                h.extend_from_slice(&v.to_le_bytes());
            }
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            if central {
                for v in [0u16, 0, 0] {
                    h.extend_from_slice(&v.to_le_bytes());
                }
                h.extend_from_slice(&0u32.to_le_bytes());
                h.extend_from_slice(&offset.to_le_bytes());
            }
            h.extend_from_slice(name.as_bytes());
            h
        };
        out.extend(head(0x0403_4b50, false));
        out.extend_from_slice(data);
        central.extend(head(0x0201_4b50, true));
    }
    let start = out.len() as u32;
    let count = files.len() as u16;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    for v in [0u16, 0, count, count] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// The zip format's CRC-32 (IEEE, reflected).
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::Format;

    fn style() -> Style {
        Style {
            settings: crate::export::Settings { vertical: true, ruby_typst: false, ..Default::default() },
            title: "第一章.md".to_string(),
            source: crate::syntax::Syntax::Markdown,
            base: None,
            painter: None,
            look: None,
        }
    }

    /// The entries of a stored zip, by walking its local headers.
    fn entries(zip: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut at = 0;
        let u16_at = |at: usize| u16::from_le_bytes([zip[at], zip[at + 1]]) as usize;
        let u32_at = |at: usize| u32::from_le_bytes([zip[at], zip[at + 1], zip[at + 2], zip[at + 3]]);
        while u32_at(at) == 0x0403_4b50 {
            assert_eq!(u16_at(at + 8), 0, "stored");
            let size = u32_at(at + 18) as usize;
            let name_len = u16_at(at + 26);
            let name = String::from_utf8(zip[at + 30..at + 30 + name_len].to_vec()).unwrap();
            let data = zip[at + 30 + name_len..at + 30 + name_len + size].to_vec();
            assert_eq!(crc32(&data), u32_at(at + 14), "{name}");
            out.push((name, data));
            at += 30 + name_len + size;
        }
        assert_eq!(u32_at(at), 0x0201_4b50, "the central directory follows");
        out
    }

    /// Every chapter's page, one after another.
    fn pages(files: &[(String, Vec<u8>)]) -> String {
        let chapters = files.iter().filter(|(name, _)| name.starts_with("OEBPS/text-"));
        chapters.map(|(_, data)| String::from_utf8(data.clone()).unwrap()).collect()
    }

    /// **What a reader that checks will look for**, held of every book the
    /// tests make: each page is in the manifest and the spine, each in the
    /// manifest is in the zip, and every link in a page or in the contents
    /// leads to an `id` in the page it names.
    fn check(files: &[(String, Vec<u8>)]) {
        let read = |name: &str| String::from_utf8(files.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("{name}")).1.clone()).unwrap();
        let opf = read("OEBPS/content.opf");
        let item = regex::Regex::new(r#"<item id="([^"]+)" href="([^"]+)""#).unwrap();
        let manifest: Vec<(String, String)> = item.captures_iter(&opf).map(|c| (c[1].to_string(), c[2].to_string())).collect();
        for (_, href) in &manifest {
            assert!(files.iter().any(|(n, _)| *n == format!("OEBPS/{href}")), "{href} is in the zip");
        }
        let texts: Vec<&(String, Vec<u8>)> = files.iter().filter(|(n, _)| n.starts_with("OEBPS/text-")).collect();
        let spine = regex::Regex::new(r#"<itemref idref="([^"]+)"/>"#).unwrap();
        let order: Vec<String> = spine
            .captures_iter(&opf)
            .map(|c| format!("OEBPS/{}", manifest.iter().find(|(id, _)| *id == c[1]).unwrap().1))
            .collect();
        assert_eq!(order, texts.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(), "the spine reads every page in order");
        let ids = |page: &str| -> Vec<String> { ID.captures_iter(page).map(|c| c[1].to_string()).collect() };
        let link = regex::Regex::new(r##"href="([^"#:]*)(?:#([^"]*))?""##).unwrap();
        let mut seen: Vec<String> = Vec::new();
        for from in texts.iter().map(|(n, _)| n.as_str()).chain(["OEBPS/nav.xhtml"]) {
            let page = read(from);
            let own = ids(&page);
            for id in &own {
                assert!(!seen.contains(id), "{id} is one place in the book");
            }
            seen.extend(own);
            for found in link.captures_iter(&page) {
                let target = match &found[1] {
                    "" => from.to_string(),
                    "style.css" => continue,
                    file => format!("OEBPS/{file}"),
                };
                assert!(texts.iter().any(|(n, _)| *n == target), "{from}: {} names a page of the book", &found[0]);
                if let Some(anchor) = found.get(2) {
                    assert!(ids(&read(&target)).iter().any(|id| id == anchor.as_str()), "{from}: {} leads somewhere", &found[0]);
                }
            }
        }
    }

    #[test]
    fn crc32_is_the_zip_formats() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn a_date_is_read_in_utc() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(1_794_009_600), "2026-11-07T00:00:00Z");
        assert_eq!(utc(951_825_599), "2000-02-29T11:59:59Z");
    }

    /// **The package an EPUB reader looks for**: `mimetype` first and
    /// stored, the container pointing at the package, and a vertical book
    /// turning from right to left.
    #[test]
    fn a_vertical_book_is_packaged_to_turn_right_to_left() {
        let text = "# 第一章\n\n1997年，他说：“走吧。”<br>\n\n## 第2节\n\n完。\n";
        let book = crate::export::export_file(text, Format::Epub, &style());
        assert!(book.starts_with(b"PK\x03\x04"));
        let files = entries(&book);
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            ["mimetype", "META-INF/container.xml", "OEBPS/content.opf", "OEBPS/nav.xhtml", "OEBPS/text-1.xhtml", "OEBPS/text-2.xhtml", "OEBPS/style.css"]
        );
        assert_eq!(files[0].1, b"application/epub+zip");
        assert_eq!(&book[30..38], b"mimetype");
        let read = |n: usize| String::from_utf8(files[n].1.clone()).unwrap();
        assert!(read(1).contains("full-path=\"OEBPS/content.opf\""));
        let opf = read(2);
        assert!(opf.contains("<spine page-progression-direction=\"rtl\">\n<itemref idref=\"t1\"/>\n<itemref idref=\"t2\"/>\n</spine>"), "{opf}");
        assert!(opf.contains("<dc:title>第一章</dc:title>"), "{opf}");
        assert!(opf.contains("<dc:language>zh</dc:language>"), "{opf}");
        assert!(opf.contains("properties=\"nav\""), "{opf}");
        assert!(opf.contains("<meta property=\"dcterms:modified\">"), "{opf}");
        let nav = read(3);
        assert!(nav.contains("<li><a href=\"text-1.xhtml#第一章\">第一章</a>\n<ol>\n<li><a href=\"text-2.xhtml#第2节\">第2节</a></li>\n</ol>\n</li>"), "{nav}");
        let page = read(4);
        assert!(page.contains("<h1 id=\"第一章\">第一章</h1>"), "{page}");
        assert!(page.contains("<span class=\"yoko\">1997</span>年"), "{page}");
        assert!(page.contains("『走吧。』<br/>"), "{page}");
        assert!(read(5).contains("<title>第2节</title>") && read(5).contains("完。"));
        assert!(read(6).contains("-epub-writing-mode: vertical-rl"));
        assert!(!read(6).contains("32em"));
    }

    /// Across the page, no direction is claimed; a formula is a file of its
    /// own, listed in the package.
    #[test]
    fn a_horizontal_book_with_a_formula() {
        fn drawn(_: &str, _: bool, _: bool, _: [u8; 3]) -> Result<crate::export::Formula, String> {
            Ok(crate::export::Formula { png: vec![1, 2, 3], width: 2.0, height: 1.0, depth: 0.2 })
        }
        let mut s = style();
        s.settings.vertical = false;
        s.painter = Some(drawn);
        let book = book("从 $x^2$ 起，到 $y$ 止。\n", &s);
        let files = entries(&book);
        let opf = String::from_utf8(files[2].1.clone()).unwrap();
        assert!(opf.contains("<spine>"), "{opf}");
        assert!(opf.contains("href=\"images/f1.png\" media-type=\"image/png\""), "{opf}");
        // The two formulas drew the same picture: one file.
        assert!(!opf.contains("images/f2.png"), "{opf}");
        // No heading: the title is the file's name.
        assert!(opf.contains("<dc:title>第一章</dc:title>"), "{opf}");
        let page = String::from_utf8(files[4].1.clone()).unwrap();
        assert_eq!(page.matches("src=\"images/f1.png\"/>").count(), 2, "{page}");
        assert_eq!(files[6], ("OEBPS/images/f1.png".to_string(), vec![1, 2, 3]));
        assert_eq!(files.len(), 7);
    }

    /// **A figure's picture goes into the book**, read from
    /// beside the source and listed with its own type; one that cannot be
    /// read is left out and its caption kept.
    #[test]
    fn a_figure_takes_its_picture_into_the_book() {
        let dir = std::env::temp_dir().join(format!("yumete-epub-figure-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jpg"), [9u8, 8, 7]).unwrap();
        let mut s = style();
        s.base = Some(dir.clone());
        let files = entries(&book("看图。\n\n![一棵树](a.jpg)\n\n![不在](gone.png)\n", &s));
        let _ = std::fs::remove_dir_all(&dir);
        let opf = String::from_utf8(files[2].1.clone()).unwrap();
        assert!(opf.contains("href=\"images/f1.jpg\" media-type=\"image/jpeg\""), "{opf}");
        let page = String::from_utf8(files[4].1.clone()).unwrap();
        assert!(page.contains("<img class=\"figure\" src=\"images/f1.jpg\" alt=\"一棵树\"/>"), "{page}");
        assert!(!page.contains("gone.png") && page.contains("<figcaption>不在</figcaption>"), "{page}");
        assert!(files.contains(&("OEBPS/images/f1.jpg".to_string(), vec![9, 8, 7])));
    }

    /// **A vertical book turns from the right whatever its language**:
    /// the stylesheet sets it vertical-rl when the export is
    /// vertical, and the spine says so with it; across, neither does.
    #[test]
    fn a_vertical_book_turns_from_the_right_in_any_language() {
        let files = entries(&book("An English essay about things, set down the page.\n", &style()));
        let opf = String::from_utf8(files[2].1.clone()).unwrap();
        assert!(opf.contains("<dc:language>en</dc:language>"), "{opf}");
        assert!(opf.contains("<spine page-progression-direction=\"rtl\">"), "{opf}");
        assert!(String::from_utf8(files[5].1.clone()).unwrap().contains("writing-mode: vertical-rl"));
        let mut flat = style();
        flat.settings.vertical = false;
        let files = entries(&book("竖排之外的一页。\n", &flat));
        let opf = String::from_utf8(files[2].1.clone()).unwrap();
        assert!(opf.contains("<spine>") && !opf.contains("vertical-rl"), "{opf}");
    }

    /// **A book's page is well-formed XML**: a form feed or
    /// another control character in the text, or in a heading, is not
    /// carried into it.
    #[test]
    fn control_characters_do_not_reach_the_book() {
        let files = entries(&book("# 标题\u{01}一\n\n甲\u{0C}乙\u{1B}丙。\n", &style()));
        for (name, data) in &files {
            let text = String::from_utf8_lossy(data);
            assert!(!text.chars().any(|c| c < ' ' && !matches!(c, '\t' | '\n' | '\r')), "{name}: {text}");
        }
        let page = String::from_utf8(files[4].1.clone()).unwrap();
        assert!(page.contains("甲 乙丙。") && page.contains(">标题一</h1>"), "{page}");
    }

    /// **A picture in a line goes into the book as a figure's does**,
    /// and a path written as a URL writes it is read as the
    /// preview reads it, `%20` a space; a picture in a line that cannot be
    /// read leaves its words.
    #[test]
    fn pictures_in_a_line_and_encoded_paths_go_into_the_book() {
        let dir = std::env::temp_dir().join(format!("yumete-epub-inline-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.png"), [1u8, 2, 3]).unwrap();
        std::fs::write(dir.join("my pic.png"), [4u8, 5, 6]).unwrap();
        let mut s = style();
        s.base = Some(dir.clone());
        let text = "文中有图 ![小图](a.png) 在此，又 ![不在](gone.png) 了。\n\n![有空格的图](my%20pic.png)\n\n![尖括号](<my pic.png>)\n";
        let files = entries(&book(text, &s));
        let _ = std::fs::remove_dir_all(&dir);
        let page = String::from_utf8(files[4].1.clone()).unwrap();
        assert!(page.contains("文中有图 <img class=\"inline\" src=\"images/f1.png\" alt=\"小图\"/> 在此"), "{page}");
        assert!(page.contains("又 不在 了。") && !page.contains("gone.png"), "{page}");
        assert!(page.contains("<img class=\"figure\" src=\"images/f2.png\" alt=\"有空格的图\"/>"), "{page}");
        // The same picture twice is one file in the book.
        assert!(page.contains("<img class=\"figure\" src=\"images/f2.png\" alt=\"尖括号\"/>"), "{page}");
        assert!(files.contains(&("OEBPS/images/f1.png".to_string(), vec![1, 2, 3])));
        assert!(files.contains(&("OEBPS/images/f2.png".to_string(), vec![4, 5, 6])));
        assert!(!files.iter().any(|(name, _)| name.ends_with("f3.png")));
    }

    /// **A link to a file beside the manuscript keeps its words, not its
    /// target** (`[x](x.html)` would make a book epubcheck refuses,
    /// RSC-007): the target goes in the words' `title`; a link to
    /// the web, a mail address or a note stays a link.
    #[test]
    fn a_link_to_a_file_outside_the_book_keeps_its_words() {
        let text = "见[第二章](ch2.html)与[网页](https://example.com)，写信[给我](mailto:a@example.com)，注[^1]。\n\n[^1]: 注文。\n";
        let files = entries(&book(text, &style()));
        let page = String::from_utf8(files[4].1.clone()).unwrap();
        assert!(page.contains("<span class=\"link\" title=\"ch2.html\">第二章</span>"), "{page}");
        assert!(!page.contains("href=\"ch2.html\""), "{page}");
        assert!(page.contains("<a href=\"https://example.com\">") && page.contains("<a href=\"mailto:a@example.com\">"), "{page}");
        assert!(page.contains("href=\"#fn-1\""), "{page}");
    }

    /// **Only the web and mail are followed from a book** (a `file:` link
    /// makes epubcheck refuse it, RSC-030): a link
    /// in any other scheme keeps its words, as one to a file beside the
    /// manuscript does.
    #[test]
    fn a_link_a_book_cannot_follow_keeps_its_words() {
        let text = "见[文件](file:///etc/hosts)与[电话](tel:123)与[网页](HTTPS://example.com)与[协议相对](//example.com/x)。\n";
        let files = entries(&book(text, &style()));
        let page = String::from_utf8(files[4].1.clone()).unwrap();
        assert!(page.contains("<span class=\"link\" title=\"file:///etc/hosts\">文件</span>"), "{page}");
        assert!(page.contains("<span class=\"link\" title=\"tel:123\">电话</span>"), "{page}");
        assert!(page.contains("<a href=\"HTTPS://example.com\">") && page.contains("<a href=\"//example.com/x\">"), "{page}");
    }

    /// **A link to a heading finds it** (`[锚点](#第二章)` pointing at
    /// nothing is RSC-012): each heading's `id` is made from its
    /// words as GitHub makes it, and the table of contents points there.
    #[test]
    fn a_link_to_a_heading_finds_it_and_so_does_the_contents() {
        let text = "# 第一章\n\n见[锚点](#第二章)与[同名](#第一章-1)。\n\n# 第二章\n\n## Hello, World!\n\n# 第一章\n";
        let files = entries(&book(text, &style()));
        let page = pages(&files);
        for id in ["第一章", "第二章", "hello-world", "第一章-1"] {
            assert!(page.contains(&format!(" id=\"{id}\">")), "{id}: {page}");
        }
        assert!(page.contains("<a href=\"text-2.xhtml#第二章\">") && page.contains("<a href=\"text-3.xhtml#第一章-1\">"), "{page}");
        let nav = String::from_utf8(files[3].1.clone()).unwrap();
        assert!(nav.contains("<a href=\"text-2.xhtml#第二章\">第二章</a>"), "{nav}");
        assert!(nav.contains("<a href=\"text-2.xhtml#hello-world\">Hello, World!</a>"), "{nav}");
        assert!(nav.contains("<a href=\"text-3.xhtml#第一章-1\">第一章</a>"), "{nav}");
    }

    /// **The writer's own HTML is made XML** (a `<span>` left open makes
    /// the page one a reader refuses, RSC-016): a tag closed
    /// in its place stays, written out in full; one left open or with
    /// nothing to close, or a block typed in a line of text, is its words.
    #[test]
    fn inline_html_left_open_does_not_break_the_book() {
        let text = "行内 <span>未闭合 与 <b>粗</b> 与 </i> 与 <details open>开</details> 与 <abbr title=a&b>缩</abbr> 与 <span hidden>藏</span>。\n";
        let files = entries(&book(text, &style()));
        let page = String::from_utf8(files[4].1.clone()).unwrap();
        assert!(page.contains("&lt;span&gt;未闭合"), "{page}");
        assert!(page.contains("<b>粗</b>"), "{page}");
        assert!(page.contains("&lt;/i&gt;"), "{page}");
        assert!(page.contains("&lt;details open&gt;开&lt;/details&gt;"), "a block in a line: {page}");
        assert!(page.contains("<span hidden=\"hidden\">藏</span>"), "{page}");
        assert!(page.contains("<abbr title=\"a&amp;b\">缩</abbr>"), "{page}");
        assert!(!page.contains("<span>"), "{page}");
    }

    /// **A picture written as HTML goes into the book** (`<img src="x.png">`
    /// pointing out of it is RSC-007), as one written in
    /// Markdown does; one that is not there leaves its `alt`.
    #[test]
    fn a_picture_written_as_html_goes_into_the_book() {
        let dir = std::env::temp_dir().join(format!("yumete-epub-raw-img-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.png"), [1u8, 2, 3]).unwrap();
        let mut s = style();
        s.settings.vertical = false;
        s.base = Some(dir.clone());
        let text = "图 <img src=\"a.png\" width=40> 与 <img src='gone.png' alt='不在'> 与 <img src=\"a.png\" alt=\"再\"> 了。\n";
        let files = entries(&book(text, &s));
        let _ = std::fs::remove_dir_all(&dir);
        let page = String::from_utf8(files[4].1.clone()).unwrap();
        assert!(page.contains("<img src=\"images/f1.png\" width=\"40\" alt=\"\"/>"), "{page}");
        assert!(page.contains("与 不在 与"), "{page}");
        assert!(page.contains("<img src=\"images/f1.png\" alt=\"再\"/>"), "{page}");
        assert!(!page.contains("gone.png") && !page.contains("\"a.png\""), "{page}");
        assert!(files.contains(&("OEBPS/images/f1.png".to_string(), vec![1, 2, 3])));
        assert_eq!(files.iter().filter(|(name, _)| name.starts_with("OEBPS/images/")).count(), 1);
    }

    /// **What epubcheck refuses, kept out**: an empty entry in the contents
    /// or an empty title from a heading that is only a picture, two
    /// `id="fnref-1"` from a note cited twice, and a page XML will not read
    /// from a tag kept as words with `]]>` in it or a `&#1;` in an
    /// attribute.
    #[test]
    fn a_picture_heading_a_note_cited_twice_and_odd_html_make_a_valid_book() {
        let mut s = style();
        s.settings.vertical = false;
        let text = "# ![第一章](nowhere.png)\n\n甲[^n]乙[^n]丙[^n]。\n\n# ![](nowhere.png)\n\n文 <div title=\"x]]>y\">块</div> 与 <span title=\"&#1;\">字</span> 与 <span title=\"&#x4E00;\">一</span>。\n\n[^n]: 註。\n";
        let files = entries(&book(text, &s));
        let file = |name: &str| String::from_utf8(files.iter().find(|(n, _)| n == name).unwrap().1.clone()).unwrap();
        let (opf, nav, page) = (file("OEBPS/content.opf"), file("OEBPS/nav.xhtml"), pages(&files));
        assert!(opf.contains("<dc:title>第一章</dc:title>"), "{opf}");
        assert!(nav.contains("<title>第一章</title>"), "{nav}");
        assert!(nav.contains(">第一章</a></li>"), "the picture's alt: {nav}");
        assert!(!nav.contains("\"></a>"), "no entry is empty: {nav}");
        for id in ["id=\"fnref-1\"", "id=\"fnref-1-2\"", "id=\"fnref-1-3\""] {
            assert_eq!(page.matches(id).count(), 1, "{id}: {page}");
        }
        assert!(!page.contains("]]>"), "{page}");
        assert!(page.contains("x]]&gt;y"), "{page}");
        assert!(!page.contains("&#1;\""), "{page}");
        assert!(page.contains("title=\"&amp;#1;\""), "{page}");
        assert!(page.contains("title=\"&#x4E00;\""), "a character XML allows stays: {page}");
    }

    /// **An HTML block is in the book too**, not words, and HTML's `align`,
    /// which XHTML has not, is the style it means.
    #[test]
    fn an_html_block_is_in_the_book_and_align_is_a_style() {
        let mut s = style();
        s.settings.vertical = false;
        let text = "<details open><summary>展开</summary>里面</details>\n\n<p align=\"Center\">居中</p>\n\n<div align=right style=\"color: red;\">右</div>\n";
        let files = entries(&book(text, &s));
        let page = String::from_utf8(files[4].1.clone()).unwrap();
        assert!(page.contains("<details open=\"open\"><summary>展开</summary>里面</details>"), "{page}");
        assert!(page.contains("<p style=\"text-align: center\">居中</p>"), "{page}");
        assert!(page.contains("<div style=\"color: red; text-align: right\">右</div>"), "{page}");
    }

    /// **A chapter is a page of its own**: each `#` starts one, what is
    /// before the first is a page too, and a heading in a quotation starts
    /// none. A link to a heading in another chapter names that chapter's
    /// page; a note stays in the chapter that first cites it, numbered as in
    /// the whole book, and a later chapter citing it again points there.
    #[test]
    fn each_chapter_is_a_page_and_links_and_notes_follow() {
        let mut s = style();
        s.settings.vertical = false;
        let text = "前言一段。\n\n# 第一章\n\n正文[^a]，见[第二章](#第二章)与[细节](#细节)。\n\n> # 引文标题\n>\n> 引文。\n\n## 小节一\n\n一[^b]。\n\n# 第二章\n\n再引[^a]，三[^c]。回看[第一章](#第一章)与[小节](#小节一)。\n\n### 细节\n\n末。\n\n[^a]: 注甲。\n[^b]: 注乙。\n[^c]: 注丙。\n";
        let files = entries(&book(text, &s));
        check(&files);
        let read = |name: &str| String::from_utf8(files.iter().find(|(n, _)| n == name).unwrap().1.clone()).unwrap();
        assert_eq!(pages(&files).matches("<?xml").count(), 3, "{}", pages(&files));
        let (front, one, two) = (read("OEBPS/text-1.xhtml"), read("OEBPS/text-2.xhtml"), read("OEBPS/text-3.xhtml"));
        assert!(front.contains("前言一段。") && !front.contains("<h1"), "{front}");
        assert!(front.contains("<title>第一章</title>"), "a page with no heading takes the book's title: {front}");
        assert!(one.contains("<title>第一章</title>") && one.contains("引文标题") && one.contains("小节一"), "{one}");
        assert!(one.contains("<a href=\"text-3.xhtml#第二章\">第二章</a>"), "{one}");
        assert!(one.contains("<a href=\"text-3.xhtml#细节\">细节</a>"), "{one}");
        assert!(two.contains("<a href=\"text-2.xhtml#第一章\">") && two.contains("<a href=\"text-2.xhtml#小节一\">"), "{two}");
        // The notes: 1 and 2 where they are first cited, 3 in its chapter.
        assert!(one.contains("<li id=\"fn-1\" value=\"1\">注甲。</li>\n<li id=\"fn-2\" value=\"2\">注乙。</li>"), "{one}");
        assert!(one.contains("<a href=\"#fn-1\" id=\"fnref-1\">1</a>"), "{one}");
        assert!(two.contains("<a href=\"text-2.xhtml#fn-1\" id=\"fnref-1-2\">1</a>"), "{two}");
        assert!(two.contains("<li id=\"fn-3\" value=\"3\">注丙。</li>") && !two.contains("id=\"fn-1\""), "{two}");
        assert!(!front.contains("footnotes"), "{front}");
        let nav = read("OEBPS/nav.xhtml");
        // The heading in the quotation is the quotation's, not the book's.
        assert!(nav.contains("<li><a href=\"text-2.xhtml#第一章\">第一章</a>\n<ol>\n<li><a href=\"text-2.xhtml#小节一\">小节一</a></li>\n</ol>\n</li>"), "{nav}");
        assert!(!nav.contains("引文标题"), "{nav}");
        assert!(nav.contains("<li><a href=\"text-3.xhtml#第二章\">第二章</a>\n<ol>\n<li><a href=\"text-3.xhtml#细节\">细节</a></li>\n</ol>\n</li>"), "{nav}");
    }

    /// **A vertical book is vertical in every chapter**: one `#` heading
    /// first is the book's title and the chapters are its `##`s; each page
    /// links the stylesheet that sets the page vertical, and ruby, a
    /// picture and a note stay in the chapter they are in.
    #[test]
    fn a_vertical_book_is_cut_at_its_chapters_under_the_title() {
        let dir = std::env::temp_dir().join(format!("yumete-epub-chapters-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.png"), [1u8, 2, 3]).unwrap();
        let mut s = style();
        s.base = Some(dir.clone());
        let text = "# 书名\n\n题记。\n\n## 第一章\n\n<ruby>永<rt>ㄩㄥˇ</rt></ruby>和九年，见[后文](#第二章)。\n\n## 第二章\n\n![一棵树](a.png)\n\n2024年[^n]。\n\n[^n]: 注。\n";
        let files = entries(&book(text, &s));
        let _ = std::fs::remove_dir_all(&dir);
        check(&files);
        let read = |name: &str| String::from_utf8(files.iter().find(|(n, _)| n == name).unwrap().1.clone()).unwrap();
        let opf = read("OEBPS/content.opf");
        assert!(opf.contains("<spine page-progression-direction=\"rtl\">\n<itemref idref=\"t1\"/>\n<itemref idref=\"t2\"/>\n<itemref idref=\"t3\"/>\n</spine>"), "{opf}");
        assert!(opf.contains("<dc:title>书名</dc:title>"), "{opf}");
        let (title, one, two) = (read("OEBPS/text-1.xhtml"), read("OEBPS/text-2.xhtml"), read("OEBPS/text-3.xhtml"));
        for page in [&title, &one, &two] {
            assert!(page.contains("<link rel=\"stylesheet\" href=\"style.css\"/>") && page.contains("xml:lang=\"zh\""), "{page}");
        }
        assert!(title.contains("<h1 id=\"书名\">书名</h1>") && title.contains("题记。") && !title.contains("第一章</h2>"), "{title}");
        assert!(one.contains("<ruby>永<rt>ㄩㄥˇ</rt></ruby>和九年") && one.contains("href=\"text-3.xhtml#第二章\""), "{one}");
        assert!(two.contains("src=\"images/f1.png\"") && two.contains("<span class=\"yoko\">2024</span>年"), "{two}");
        assert!(two.contains("<li id=\"fn-1\" value=\"1\">"), "{two}");
        assert!(read("OEBPS/style.css").contains("-epub-writing-mode: vertical-rl"));
        let nav = read("OEBPS/nav.xhtml");
        assert!(nav.contains("<li><a href=\"text-1.xhtml#书名\">书名</a>\n<ol>\n<li><a href=\"text-2.xhtml#第一章\">第一章</a></li>\n<li><a href=\"text-3.xhtml#第二章\">第二章</a></li>\n</ol>\n</li>"), "{nav}");
    }

    /// **A text of one chapter is one page**, notes and all; so is one with
    /// no heading, and the contents then name the book.
    #[test]
    fn one_chapter_is_one_page() {
        let files = entries(&book("# 唯一\n\n正文[^1]。\n\n[^1]: 注。\n", &style()));
        check(&files);
        assert_eq!(files.iter().filter(|(n, _)| n.starts_with("OEBPS/text-")).count(), 1);
        let page = pages(&files);
        assert!(page.contains("<h1 id=\"唯一\">唯一</h1>") && page.contains("<li id=\"fn-1\" value=\"1\">注。</li>"), "{page}");
        let files = entries(&book("没有标题的一段。\n", &style()));
        check(&files);
        let nav = String::from_utf8(files[3].1.clone()).unwrap();
        assert!(nav.contains("<li><a href=\"text-1.xhtml\">第一章</a></li>"), "{nav}");
        assert_eq!(files[4].0, "OEBPS/text-1.xhtml");
    }
}
