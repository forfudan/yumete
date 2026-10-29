//! Offline help index over the mdBook sources. Identifiers are language-neutral;
//! translation is selected only after resolving a topic in the English index.
use std::{collections::BTreeMap, sync::OnceLock};
include!(concat!(env!("OUT_DIR"), "/help_pages.rs"));

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub page: &'static str,
    pub line: usize,
}
#[derive(Default)]
struct Index {
    topics: BTreeMap<String, Location>,
    options: BTreeMap<String, Location>,
}
fn key(s: &str) -> String {
    let normalized = s
        .replace("<Space>", "Space")
        .replace("<space>", "Space")
        .replace('␣', "Space ")
        .replace("空格", "Space")
        .replace("<Esc>", "Esc")
        .replace("<Backspace>", "Backspace")
        .replace("<Enter>", "Enter")
        .replace("<Tab>", "Tab")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    static NAMED_KEY: OnceLock<regex::Regex> = OnceLock::new();
    NAMED_KEY
        .get_or_init(|| regex::Regex::new(r"<([A-Za-z][A-Za-z0-9^_-]*)>").unwrap())
        .replace_all(&normalized, "$1")
        .into_owned()
}
fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut index = Index::default();
        let code = regex::Regex::new(r"`([^`]+)`").unwrap();
        let assignment = regex::Regex::new(r"^([a-zA-Z_][\w-]*)\s*=").unwrap();
        for &(page, text) in PAGES.iter().filter(|(p, _)| p.starts_with("en/")) {
            let key_page = text.contains("<!-- help-index: keys -->");
            let command_page = text.contains("<!-- help-index: commands -->");
            let option_page = text.contains("<!-- help-index: options -->");
            let mut toml = false;
            let mut fenced = false;
            let mut section = String::new();
            for (line, raw) in text.lines().enumerate() {
                let at = Location { page, line };
                let s = raw.trim();
                if let Some(tags) = s
                    .strip_prefix("<!-- help: ")
                    .and_then(|s| s.strip_suffix(" -->"))
                {
                    for tag in tags.split_whitespace() {
                        let line = text
                            .lines()
                            .enumerate()
                            .skip(line + 1)
                            .find(|(_, s)| !s.trim().is_empty() && !s.trim().starts_with("<!--"))
                            .map(|(line, _)| line)
                            .unwrap_or(line);
                        index
                            .topics
                            .entry(key(tag))
                            .or_insert(Location { page, line });
                    }
                }
                if s.starts_with("```") || s.starts_with("~~~") {
                    fenced = !fenced;
                    toml = fenced && s == "```toml" && option_page;
                    section.clear();
                    continue;
                }
                if toml {
                    if s.starts_with('[') && !s.starts_with("[[") {
                        if let Some(end) = s.find(']') {
                            section = s[1..end].to_string();
                        }
                    } else if !section.is_empty() {
                        if let Some(c) = assignment.captures(s) {
                            let name = format!("{}.{}", section, &c[1]);
                            index.options.entry(name).or_insert(at.clone());
                        }
                    }
                }
                if fenced {
                    continue;
                }
                // Commands are indexed only from their reference table. Mentions
                // elsewhere must never displace the actual reference entry.
                if command_page {
                    if s.starts_with('|') || s.starts_with("- `:") {
                        let cell = if s.starts_with('|') {
                            s.split('|').nth(1).unwrap_or("")
                        } else {
                            s.split('—').next().unwrap_or(s)
                        };
                        for c in code.captures_iter(cell) {
                            if let Some(name) = c[1].strip_prefix(':') {
                                let name = name.split_whitespace().next().unwrap_or("");
                                let canonical = crate::command::COMMANDS
                                    .iter()
                                    .find(|e| e.name == name || e.aliases.contains(&name))
                                    .map(|e| e.name)
                                    .unwrap_or(name);
                                index
                                    .topics
                                    .entry(format!(":{canonical}"))
                                    .or_insert(at.clone());
                            }
                        }
                    }
                }
                // First cells in key reference tables are key declarations;
                // ignore comparison tables, prose examples, and command tables.
                if key_page && s.starts_with('|') {
                    let cell = s.split('|').nth(1).unwrap_or("");
                    for c in code.captures_iter(cell) {
                        let spelling = key(&c[1]);
                        if spelling.starts_with(':') {
                            continue;
                        }
                        let insert = cell.contains("(Insert)");
                        let tokens: Vec<_> = spelling.split_whitespace().collect();
                        let spellings = if tokens.len() > 1 && !spelling.starts_with("Space ") {
                            tokens.iter().map(|s| s.to_string()).collect()
                        } else {
                            vec![spelling]
                        };
                        for spelling in spellings {
                            let tag = if insert {
                                format!("i_{spelling}")
                            } else {
                                spelling
                            };
                            index.topics.entry(tag).or_insert(at.clone());
                        }
                    }
                }
            }
            let relative = page.strip_prefix("en/").unwrap();
            index
                .topics
                .entry(relative.to_string())
                .or_insert(Location { page, line: 0 });
        }
        index
    })
}

/// Resolve exact identifiers. Ambiguous short options return candidates rather
/// than silently selecting a configuration section.
pub fn lookup(topic: &str) -> Result<Location, Vec<String>> {
    let topic = topic.trim();
    let option = topic.trim_matches('\'');
    if topic.starts_with('\'') {
        if let Some(at) = index().options.get(option) {
            return Ok(at.clone());
        }
        let found: Vec<_> = index()
            .options
            .iter()
            .filter(|(name, _)| name.rsplit('.').next() == Some(option))
            .collect();
        return match found.as_slice() {
            [(_, at)] => Ok((*at).clone()),
            _ => Err(found.iter().map(|(name, _)| format!("'{name}'")).collect()),
        };
    }
    let normalized = key(topic);
    if let Some(at) = index().topics.get(&normalized) {
        return Ok(at.clone());
    }
    let command = topic.strip_prefix(':').unwrap_or(topic);
    if let Some(entry) = crate::command::COMMANDS
        .iter()
        .find(|e| e.name == command || e.aliases.contains(&command))
    {
        if let Some(at) = index().topics.get(&format!(":{}", entry.name)) {
            return Ok(at.clone());
        }
        // The live command declaration still provides help if the prose manual
        // has not acquired a detailed entry yet; the caller renders it.
        return Err(vec![format!(":{}", entry.name)]);
    }
    Err(Vec::new())
}

pub fn page(id: &str, lang: &str) -> Option<(&'static str, &'static str, bool)> {
    let relative = id.strip_prefix("en/").unwrap_or(id);
    let wanted = format!("{lang}/{relative}");
    if let Some(&(id, text)) = PAGES.iter().find(|(id, _)| *id == wanted) {
        return Some((id, text, false));
    }
    let english = format!("en/{relative}");
    PAGES
        .iter()
        .find(|(id, _)| *id == english)
        .map(|&(id, text)| (id, text, lang != "en"))
}

/// Resolve a relative Markdown help link without touching the filesystem.
pub fn link(from: &str, target: &str) -> Option<String> {
    let mut parts: Vec<_> = from.split('/').collect();
    parts.pop();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    let path = parts.join("/");
    let path = path
        .strip_suffix(".html")
        .map(|s| format!("{s}.md"))
        .unwrap_or(path);
    let (lang, relative) = path.split_once('/')?;
    page(relative, lang).map(|_| path)
}

pub fn anchor_line(text: &str, anchor: &str) -> Option<usize> {
    let wanted = anchor.to_lowercase();
    text.lines().enumerate().find_map(|(line, text)| {
        let text = text.trim();
        if text.contains(&format!("id=\"{anchor}\""))
            || text.contains(&format!("name=\"{anchor}\""))
        {
            return Some(line);
        }
        if !text.starts_with('#') {
            return None;
        }
        let heading: String = text
            .trim_start_matches('#')
            .trim()
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '_' || *c == '-')
            .collect();
        (heading.split_whitespace().collect::<Vec<_>>().join("-") == wanted).then_some(line)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_and_aliases() {
        assert_eq!(lookup(":w"), lookup(":write"));
        assert!(lookup(":write").is_ok());
        assert_ne!(lookup("w").unwrap(), lookup(":write").unwrap());
        assert!(lookup("'editor.scrolloff'").is_ok());
        assert_eq!(lookup("'scrolloff'"), lookup("'editor.scrolloff'"));
        assert_eq!(lookup("i_<Esc>"), lookup("i_Esc"));
        assert!(lookup("c_<Backspace>").is_ok());
        assert!(lookup("not-a-help-topic").unwrap_err().is_empty());
    }
    #[test]
    fn translation_fallback_and_embedded_links() {
        let at = lookup("w").unwrap();
        assert!(page(at.page, "zh-Hans").unwrap().2);
        assert!(!page("help.md", "zh-Hans").unwrap().2);
        assert_eq!(
            link("en/manual/06-moving.md", "../help.md"),
            Some("en/help.md".into())
        );
        assert_eq!(link("en/help.md", "../../../etc/passwd"), None);
        assert_eq!(
            link("en/index.md", "../zh-Hans/index.html"),
            Some("zh-Hans/index.md".into())
        );
    }
}

/// Stable strings from the process-wide index, reused by the command menu.
pub fn completions(prefix: &str) -> Vec<&'static str> {
    static TAGS: OnceLock<Vec<String>> = OnceLock::new();
    TAGS.get_or_init(|| {
        let mut tags: Vec<String> = index().topics.keys().cloned().collect();
        tags.extend(index().options.keys().map(|name| format!("'{name}'")));
        for name in index().options.keys() {
            let short = name.rsplit('.').next().unwrap();
            if index()
                .options
                .keys()
                .filter(|name| name.rsplit('.').next() == Some(short))
                .count()
                == 1
            {
                tags.push(format!("'{short}'"));
            }
        }
        for entry in crate::command::COMMANDS {
            tags.push(format!(":{}", entry.name));
            tags.extend(entry.aliases.iter().map(|alias| format!(":{alias}")));
        }
        tags.extend(["chinese", "vertical", "table", "commands"].map(str::to_string));
        tags.sort();
        tags.dedup();
        tags
    })
    .iter()
    .filter(|name| name.starts_with(prefix))
    .map(String::as_str)
    .collect()
}
