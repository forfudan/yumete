//! **The keys a citation can name** (2026-10-29): `[@` in Markdown (Pandoc's
//! syntax) and `@` in Typst finish from the bibliography beside the document —
//! BibTeX's `@article{key,` and hayagriva's top-level `key:` — with each
//! entry's title beside its key.
//!
//! Which files: the ones the document names (`#bibliography("refs.bib")` in
//! Typst, `bibliography:` in Markdown's front matter), and every `.bib` in the
//! document's own folder. A YAML file is read only when it is named, since
//! most YAML is not a bibliography.

/// The entries of a BibTeX file: each key, and its title when it has one.
pub fn bibtex_entries(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find('@') {
        rest = &rest[at + 1..];
        let kind: String = rest.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        // `@comment`, `@string` and `@preamble` hold no entry.
        if kind.is_empty() || matches!(kind.to_ascii_lowercase().as_str(), "comment" | "string" | "preamble") {
            continue;
        }
        let after = rest[kind.len()..].trim_start();
        let Some(body) = after.strip_prefix(['{', '(']) else { continue };
        let Some(comma) = body.find(',') else { continue };
        let key = body[..comma].trim();
        if key.is_empty() || key.contains(char::is_whitespace) {
            continue;
        }
        // The title, looked for up to the next entry.
        let entry = &body[comma..body[comma..].find("\n@").map_or(body.len(), |n| comma + n)];
        out.push((key.to_string(), bibtex_title(entry).unwrap_or_default()));
    }
    out
}

/// `title = {…}` or `title = "…"`, one level of braces taken off.
fn bibtex_title(entry: &str) -> Option<String> {
    let lower = entry.to_ascii_lowercase();
    let mut from = 0;
    let at = loop {
        let at = from + lower[from..].find("title")?;
        let before = lower[..at].chars().next_back();
        let after = lower[at + 5..].trim_start();
        if !before.is_some_and(|c| c.is_ascii_alphanumeric()) && after.starts_with('=') {
            break at;
        }
        from = at + 5;
    };
    let value = entry[at + 5..].trim_start().strip_prefix('=')?.trim_start();
    let (open, close) = match value.chars().next()? {
        '{' => ('{', '}'),
        '"' => ('"', '"'),
        _ => return None,
    };
    let mut depth = 0;
    let mut out = String::new();
    for c in value.chars() {
        match c {
            c if c == open && (open != close || depth == 0) => {
                depth += 1;
                if depth == 1 {
                    continue;
                }
            }
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        if c != '{' && c != '}' {
            out.push(c);
        }
    }
    Some(out.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// The entries of a hayagriva file: each top-level key, and its `title:`.
pub fn hayagriva_entries(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if line.starts_with(|c: char| !c.is_whitespace() && c != '#' && c != '-') {
            if let Some(key) = line.trim_end().strip_suffix(':') {
                if !key.contains([' ', ':']) {
                    out.push((key.to_string(), String::new()));
                }
            }
            continue;
        }
        let trimmed = line.trim_start();
        // The entry's own title: one level in, not a parent's or a series'.
        if let (Some(last), Some(title)) = (out.last_mut(), trimmed.strip_prefix("title:")) {
            let indent = line.len() - trimmed.len();
            if last.1.is_empty() && indent <= 4 {
                last.1 = title.trim().trim_matches(['"', '\'']).to_string();
            }
        }
    }
    out
}

/// The bibliography files `text` names: Typst's `#bibliography(…)` with one
/// path or a list of them, and Markdown's `bibliography:` in front matter, as
/// one path or a YAML list.
pub fn named_bibliographies(text: &str, typst: bool) -> Vec<String> {
    let quoted = |s: &str| -> Vec<String> {
        s.split('"').skip(1).step_by(2).map(str::to_string).filter(|p| !p.is_empty()).collect()
    };
    match typst {
        // The first argument only — one path, or a list of them in its own
        // parentheses; `style: "apa"` after it is not a file.
        true => text
            .match_indices("#bibliography(")
            .flat_map(|(at, call)| {
                let rest = text[at + call.len()..].trim_start();
                let first = match rest.strip_prefix('(') {
                    Some(list) => &list[..list.find(')').unwrap_or(list.len())],
                    None => match rest.strip_prefix('"') {
                        Some(one) => &rest[..one.find('"').map_or(rest.len(), |e| e + 2)],
                        None => "",
                    },
                };
                quoted(first)
            })
            .collect(),
        false => {
            let Some(front) = text.strip_prefix("---\n") else { return Vec::new() };
            let front = &front[..front.find("\n---").unwrap_or(0)];
            let mut out = Vec::new();
            let mut in_list = false;
            for line in front.lines() {
                if let Some(value) = line.strip_prefix("bibliography:") {
                    let value = value.trim().trim_matches(['"', '\'']);
                    match value.is_empty() {
                        true => in_list = true,
                        false => {
                            let list = value.trim_start_matches('[').trim_end_matches(']');
                            out.extend(list.split(',').map(|p| p.trim().trim_matches(['"', '\'']).to_string()));
                        }
                    }
                    continue;
                }
                match (in_list, line.trim_start().strip_prefix("- ")) {
                    (true, Some(item)) => out.push(item.trim().trim_matches(['"', '\'']).to_string()),
                    _ => in_list = false,
                }
            }
            out.retain(|p| !p.is_empty());
            out
        }
    }
}

/// The labels a Typst document sets, `<label>`, so `@label` can name them.
pub fn typst_labels(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (at, _) in text.match_indices('<') {
        let rest = &text[at + 1..];
        let Some(end) = rest.find('>') else { continue };
        let name = &rest[..end];
        let fits = !name.is_empty()
            && name.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
            && name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_');
        if fits && !out.iter().any(|seen: &String| seen == name) {
            out.push(name.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bibtex_keys_and_titles_are_read() {
        let bib = "@article{knuth84,\n  author = {Donald Knuth},\n  title = {Literate {P}rogramming},\n}\n\n@comment{nothing}\n@book{lamport94,\n  title = \"LaTeX: A Document Preparation System\",\n}\n@misc{bare, note={no title}}\n";
        assert_eq!(
            bibtex_entries(bib),
            vec![
                ("knuth84".to_string(), "Literate Programming".to_string()),
                ("lamport94".to_string(), "LaTeX: A Document Preparation System".to_string()),
                ("bare".to_string(), String::new()),
            ]
        );
    }

    #[test]
    fn hayagriva_keys_and_titles_are_read() {
        let yml = "harry:\n  type: Book\n  title: Harry Potter and the Order of the Phoenix\n  parent:\n    title: Series\nelectronic:\n  type: Web\n  title: \"Science: A Thing\"\n";
        assert_eq!(
            hayagriva_entries(yml),
            vec![
                ("harry".to_string(), "Harry Potter and the Order of the Phoenix".to_string()),
                ("electronic".to_string(), "Science: A Thing".to_string()),
            ]
        );
    }

    #[test]
    fn the_named_files_are_found_in_both_syntaxes() {
        assert_eq!(named_bibliographies("= T\n#bibliography(\"refs.bib\")\n", true), vec!["refs.bib"]);
        assert_eq!(
            named_bibliographies("#bibliography((\"a.bib\", \"b.yml\"), style: \"apa\")\n", true),
            vec!["a.bib", "b.yml"],
            "the style is not a file"
        );
        assert_eq!(named_bibliographies("#bibliography(\"refs.bib\", style: \"apa\")\n", true), vec!["refs.bib"]);
        assert_eq!(named_bibliographies("---\ntitle: X\nbibliography: refs.bib\n---\nText\n", false), vec!["refs.bib"]);
        assert_eq!(
            named_bibliographies("---\nbibliography:\n  - a.bib\n  - \"b.json\"\nlang: en\n---\n", false),
            vec!["a.bib", "b.json"]
        );
        assert!(named_bibliographies("No front matter\nbibliography: x.bib\n", false).is_empty());
    }

    #[test]
    fn typst_labels_are_found_once() {
        assert_eq!(typst_labels("= Intro <intro>\n$ x $ <eq:one>\n@intro and <intro> again; a < b > c\n"), vec!["intro", "eq:one"]);
    }
}
