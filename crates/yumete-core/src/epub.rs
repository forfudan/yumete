//! **`:export epub`** (2026-11-08): the HTML export as a book a reader app
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
//! - `OEBPS/nav.xhtml`, the table of contents, from the headings;
//! - `OEBPS/text.xhtml` and `OEBPS/style.css`, the page;
//! - `OEBPS/images/`, each drawn formula and each picture, a figure's or one
//!   in a line, as a file of its own: a data URL is not a resource a package
//!   can list, and a book cannot reach outside itself for a picture.
//!
//! Everything is stored, not deflated: the formulas are PNGs already, a
//! chapter is a few tens of kilobytes, and a zip writer with no compressor is
//! the dozen lines of [`zip`] rather than a dependency.

use std::fmt::Write as _;

use crate::export::Style;

/// The book's page, as XHTML, and the formula pictures it refers to, each
/// with the name it has in the book.
pub(crate) fn chapter(text: &str, style: &Style) -> (String, Vec<(String, Vec<u8>)>) {
    let parts = parts(text, style);
    (parts.page, parts.images)
}

/// The whole EPUB, as the bytes of the file.
pub fn book(text: &str, style: &Style) -> Vec<u8> {
    let parts = parts(text, style);
    let Parts { lang, title, page, css, images, headings } = parts;
    let id = identifier(text, &title);
    let mut manifest = String::from(
        "<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n\
         <item id=\"text\" href=\"text.xhtml\" media-type=\"application/xhtml+xml\"/>\n\
         <item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>\n",
    );
    for (n, (name, _)) in images.iter().enumerate() {
        let kind = media_type(name).unwrap_or("image/png");
        let _ = writeln!(manifest, "<item id=\"f{}\" href=\"{name}\" media-type=\"{kind}\"/>", n + 1);
    }
    // **Right to left exactly when the page is vertical** (2026-11-09), as
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
         <spine{direction}>\n<itemref idref=\"text\"/>\n</spine>\n\
         </package>\n",
        title = xml_text(&title),
        modified = utc_now(),
    );
    let mut toc = String::new();
    for (anchor, words) in &headings {
        let _ = writeln!(toc, "<li><a href=\"text.xhtml#{anchor}\">{}</a></li>", xml_text(words));
    }
    if toc.is_empty() {
        let _ = writeln!(toc, "<li><a href=\"text.xhtml\">{}</a></li>", xml_text(&title));
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
        ("OEBPS/text.xhtml".to_string(), page.into_bytes()),
        ("OEBPS/style.css".to_string(), css.into_bytes()),
    ];
    for (name, png) in images {
        files.push((format!("OEBPS/{name}"), png));
    }
    zip(&files)
}

/// What a book is made of before it is packed.
struct Parts {
    lang: &'static str,
    title: String,
    page: String,
    css: String,
    images: Vec<(String, Vec<u8>)>,
    /// Each heading's anchor and its words, for the table of contents.
    headings: Vec<(String, String)>,
}

fn parts(text: &str, style: &Style) -> Parts {
    let text = crate::markdown::strip_comments(text);
    let lang = crate::export::language_of(&text);
    let css = crate::export::stylesheet(style, lang, true);
    let body = crate::export::html_body(&text, style, lang);
    let body = pictures_in(&body, style.base.as_deref());
    let (body, images) = pictures_out(&body);
    let body = links_kept_in(&body);
    let (body, headings) = anchored(&xhtml(&body));
    let title = headings
        .first()
        .map(|(_, words)| words.clone())
        .unwrap_or_else(|| style.title.rsplit_once('.').map_or(style.title.clone(), |(stem, _)| stem.to_string()));
    let page = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" lang=\"{lang}\" xml:lang=\"{lang}\">\n\
         <head><meta charset=\"utf-8\"/><title>{}</title><link rel=\"stylesheet\" href=\"style.css\"/></head>\n\
         <body>\n{body}</body>\n</html>\n",
        xml_text(&title),
    );
    Parts { lang, title, page, css, images, headings }
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

/// **Each picture taken into the book** (2026-11-08), a figure's and
/// (2026-11-09) one in a line: read from beside the source, its path
/// percent-decoded as the preview reads it (`my%20pic.png`), and written
/// into the page as a `data:` URL, for [`pictures_out`] to file. A picture
/// that cannot be read — not there, on the web, of a type a book does not
/// hold, or a buffer never saved — is left out: a figure keeps its caption,
/// a picture in a line its text, since a book that points outside itself
/// does not open in a reader that checks.
fn pictures_in(body: &str, base: Option<&std::path::Path>) -> String {
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

/// The `alt` of an `<img>` tag, as it was written into the tag: escaped
/// already, and so text for the page as it is.
fn alt_of(tag: &str) -> String {
    tag.split_once(" alt=\"").and_then(|(_, after)| after.split_once('"')).map(|(alt, _)| alt.to_string()).unwrap_or_default()
}

/// Each `data:` picture — a drawn formula, a figure taken in — written out
/// into a file of its own, and the page pointing at it.
///
/// **One file for each picture** (2026-11-09, reported: a picture shown ten
/// times went into the book ten times): a picture whose bytes are already
/// filed points at that file.
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

/// **Links a book can follow**, and the rest as their words (2026-11-09,
/// reported: `[x](x.html)` made the book one epubcheck refuses, RSC-007,
/// the file not in the book): a link to the web or to a place in the page
/// stays a link; one to a file beside the manuscript, which the book does
/// not hold, keeps its words, the target in their `title`.
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
        let scheme = href.split_once(':').is_some_and(|(s, _)| {
            s.len() > 1 && s.starts_with(|c: char| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
        });
        if scheme || href.starts_with('#') {
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

/// Every `<h1>`–`<h3>` given an `id`, and the list of them with their words
/// for the table of contents.
fn anchored(body: &str) -> (String, Vec<(String, String)>) {
    let mut out = String::with_capacity(body.len() + 64);
    let mut headings = Vec::new();
    let mut rest = body;
    loop {
        let next = ["<h1>", "<h2>", "<h3>"].iter().filter_map(|tag| rest.find(tag).map(|at| (at, *tag))).min();
        let Some((at, tag)) = next else { break };
        out.push_str(&rest[..at]);
        let anchor = format!("h{}", headings.len() + 1);
        let _ = write!(out, "{} id=\"{anchor}\">", &tag[..3]);
        rest = &rest[at + tag.len()..];
        let close = format!("</{}", &tag[1..3]);
        let words = rest.find(&close).map(|end| plain(&rest[..end])).unwrap_or_default();
        headings.push((anchor, words));
    }
    out.push_str(rest);
    (out, headings)
}

/// The words of a run of markup: its tags dropped, and in a `<ruby>` the
/// reading too, so a heading reads as its text in a table of contents. The
/// entities stay, since the result goes back into XML.
fn plain(markup: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut reading = false;
    let mut tag = String::new();
    for c in markup.chars() {
        match (in_tag, c) {
            (false, '<') => {
                in_tag = true;
                tag.clear();
            }
            (true, '>') => {
                in_tag = false;
                let name = tag.trim_start_matches('/');
                if name.starts_with("rt") || name.starts_with("rp") {
                    reading = !tag.starts_with('/');
                }
            }
            (true, c) => tag.push(c),
            (false, c) if !reading => out.push(c),
            _ => {}
        }
    }
    // Back to characters, so `xml_text` does not escape an `&amp;` twice.
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&")
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
fn utc_now() -> String {
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
        assert_eq!(names, ["mimetype", "META-INF/container.xml", "OEBPS/content.opf", "OEBPS/nav.xhtml", "OEBPS/text.xhtml", "OEBPS/style.css"]);
        assert_eq!(files[0].1, b"application/epub+zip");
        assert_eq!(&book[30..38], b"mimetype");
        let read = |n: usize| String::from_utf8(files[n].1.clone()).unwrap();
        assert!(read(1).contains("full-path=\"OEBPS/content.opf\""));
        let opf = read(2);
        assert!(opf.contains("<spine page-progression-direction=\"rtl\">"), "{opf}");
        assert!(opf.contains("<dc:title>第一章</dc:title>"), "{opf}");
        assert!(opf.contains("<dc:language>zh</dc:language>"), "{opf}");
        assert!(opf.contains("properties=\"nav\""), "{opf}");
        assert!(opf.contains("<meta property=\"dcterms:modified\">"), "{opf}");
        let nav = read(3);
        assert!(nav.contains("<a href=\"text.xhtml#h1\">第一章</a>"), "{nav}");
        assert!(nav.contains("<a href=\"text.xhtml#h2\">第2节</a>"), "{nav}");
        let page = read(4);
        assert!(page.contains("<h1 id=\"h1\">第一章</h1>"), "{page}");
        assert!(page.contains("<span class=\"yoko\">1997</span>年"), "{page}");
        assert!(page.contains("『走吧。』<br/>"), "{page}");
        assert!(read(5).contains("-epub-writing-mode: vertical-rl"));
        assert!(!read(5).contains("32em"));
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
        // The two formulas drew the same picture: one file (2026-11-09).
        assert!(!opf.contains("images/f2.png"), "{opf}");
        // No heading: the title is the file's name.
        assert!(opf.contains("<dc:title>第一章</dc:title>"), "{opf}");
        let page = String::from_utf8(files[4].1.clone()).unwrap();
        assert_eq!(page.matches("src=\"images/f1.png\"/>").count(), 2, "{page}");
        assert_eq!(files[6], ("OEBPS/images/f1.png".to_string(), vec![1, 2, 3]));
        assert_eq!(files.len(), 7);
    }

    /// **A figure's picture goes into the book** (2026-11-08), read from
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

    /// **A vertical book turns from the right whatever its language**
    /// (2026-11-09): the stylesheet sets it vertical-rl when the export is
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

    /// **A book's page is well-formed XML** (2026-11-09): a form feed or
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

    /// **A picture in a line goes into the book as a figure's does**
    /// (2026-11-09), and a path written as a URL writes it is read as the
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
        // The same picture twice is one file in the book (2026-11-09).
        assert!(page.contains("<img class=\"figure\" src=\"images/f2.png\" alt=\"尖括号\"/>"), "{page}");
        assert!(files.contains(&("OEBPS/images/f1.png".to_string(), vec![1, 2, 3])));
        assert!(files.contains(&("OEBPS/images/f2.png".to_string(), vec![4, 5, 6])));
        assert!(!files.iter().any(|(name, _)| name.ends_with("f3.png")));
    }

    /// **A link to a file beside the manuscript keeps its words, not its
    /// target** (2026-11-09, reported: `[x](x.html)` made a book epubcheck
    /// refuses, RSC-007): the target goes in the words' `title`; a link to
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
}
