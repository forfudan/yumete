//! The editor's side of 雙鏈 (2026-10-30): when the project's links are read,
//! the panel that shows them two ways, `gr` on a Markdown file, and the one
//! resolver `[[name]]` goes through everywhere.

use super::*;
use crate::command::LinksCommand;
use crate::links::{Index, Look};
use std::collections::BTreeSet;

/// Fold keys the links panel remembers — sections by name, nodes by path.
const IN: &str = "in";
const OUT: &str = "out";
const DANGLING: &str = "dangling";
const MENTIONS: &str = "mentions";

/// A row of the panel is at most this many neighbours before 「…and N more」.
const MOST_NEIGHBOURS: usize = 12;
/// How much of a line the panel shows around a link.
const AROUND: usize = 18;

impl Editor {
    /// The project's link index, read now if it has not been. A key that
    /// wants an answer (`gd`, `gr`, `[[` completion) waits for it; the panel
    /// asks [`Self::start_links_index`] instead and draws 「reading…」.
    pub(super) fn links_index(&mut self) -> &Index {
        if self.links.is_none() {
            self.collect_links_index();
        }
        if self.links.is_none() {
            let root = self.root();
            let texts = read_project(&root);
            self.links = Some(Index::build(&root, texts));
            self.links_building = None;
        }
        self.links.as_ref().expect("read just above")
    }

    /// Read the project on a thread of its own; [`Self::collect_links_index`]
    /// takes the answer when it comes.
    pub(super) fn start_links_index(&mut self) {
        if self.links_building.is_some() {
            return;
        }
        let root = self.root();
        let (say, heard) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let texts = read_project(&root);
            let _ = say.send(Index::build(&root, texts));
        });
        self.links_building = Some(heard);
    }

    /// **Take the index a thread finished**, every frame. `true` when it
    /// landed, so the front end redraws the panel.
    pub fn collect_links_index(&mut self) -> bool {
        // Warm up: a Markdown file on screen with no index yet starts the
        // reading, so `[[` and `gd` find it ready.
        if self.links.is_none()
            && self.links_building.is_none()
            && self.current_file_path().is_some_and(|p| crate::links::is_node(&p))
        {
            self.start_links_index();
            return false;
        }
        let Some(heard) = self.links_building.as_ref() else { return false };
        match heard.try_recv() {
            Ok(index) => {
                self.links = Some(index);
                self.links_building = None;
                self.links_mentions.borrow_mut().take();
                self.refresh_sidebar();
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.links_building = None;
                false
            }
        }
    }

    /// Wait this long, at most, for the thread reading the index.
    fn wait_for_links_index(&mut self, at_most: std::time::Duration) {
        let Some(heard) = self.links_building.as_ref() else { return };
        if let Ok(index) = heard.recv_timeout(at_most) {
            self.links = Some(index);
            self.links_building = None;
            self.links_mentions.borrow_mut().take();
        }
    }

    /// Whether the panel is waiting on a thread.
    pub fn links_reading(&self) -> bool {
        self.links_building.is_some() && self.links.is_none()
    }

    /// **The one resolver**: the file `[[target]]` names, from the current
    /// buffer. `gd`, the panel and completion all ask here, so they agree.
    pub(super) fn resolve_link(&mut self, target: &str) -> Option<PathBuf> {
        let from = self.current_file_path()?;
        self.links_index().resolve(target, &from).map(Path::to_path_buf)
    }

    /// The current buffer's path, made absolute against the working path.
    pub(super) fn current_file_path(&self) -> Option<PathBuf> {
        let here = self.working_dir();
        self.current_buffer().path().map(|p| match p.is_absolute() {
            true => p.to_path_buf(),
            false => here.join(p),
        })
    }

    /// A save: this file's links read again, a new file learnt. Costs one
    /// scan of text already in memory.
    pub(super) fn links_note_saved(&mut self) {
        let Some(path) = self.current_file_path() else { return };
        if !crate::links::is_node(&path) {
            return;
        }
        let text = self.current_buffer().rope().to_string();
        if let Some(index) = self.links.as_mut() {
            index.replace(&path, &text);
            self.links_mentions.borrow_mut().take();
        }
        if self.showing(crate::sidebar::View::Links).is_some() {
            self.refresh_sidebar();
        }
    }

    /// `:links [list|graph|off]`, `:links-export`, `:links-reindex`.
    pub(super) fn links_command(&mut self, what: LinksCommand) -> Result<CommandOutcome, EditorError> {
        match what {
            LinksCommand::Show(look) => {
                if self.links.is_none() {
                    self.start_links_index();
                }
                match look {
                    // Bare `:links` is the sidebar's own dance: open, focus,
                    // close.
                    None => self.show_sidebar(crate::sidebar::View::Links),
                    // A named look: the panel in that look, with the keys —
                    // never closed by asking for it.
                    Some(look) => {
                        self.links_look = look;
                        match self.showing(crate::sidebar::View::Links) {
                            Some(side) => {
                                self.focus_slot(side);
                                self.refresh_sidebar();
                            }
                            None => self.show_sidebar(crate::sidebar::View::Links),
                        }
                    }
                }
            }
            LinksCommand::Off => {
                if let Some(side) = self.showing(crate::sidebar::View::Links) {
                    self.close_panel(side);
                }
            }
            LinksCommand::Export => self.export_links_graph(),
            LinksCommand::Reindex => {
                self.links = None;
                self.links_mentions.borrow_mut().take();
                let n = self.links_index().files.len();
                self.status = say!("links.reindexed", n);
                self.refresh_sidebar();
            }
        }
        Ok(CommandOutcome::Continue)
    }

    /// The whole graph as a page in `.yumete/graph.html`, opened in the
    /// browser the way a web link is.
    fn export_links_graph(&mut self) {
        let root = self.root();
        let title = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let page = self.links_index().html(&title);
        let dir = root.join(".yumete");
        let target = dir.join("graph.html");
        let written = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&target, page));
        match written {
            Ok(()) => {
                self.status = say!("links.exported", target.display());
                self.open_request = Some(target.to_string_lossy().into_owned());
            }
            Err(err) => self.status = say!("links.export-failed", target.display(), err),
        }
    }

    /// `gr` in a Markdown file: what links here, as the references picker.
    /// On a `[[x]]` it is x's backlinks; anywhere else, this file's.
    pub(super) fn show_backlinks(&mut self) {
        let target = match self.link_under_cursor().filter(|l| l.wiki) {
            Some(link) => match self.resolve_link(&link.target) {
                Some(path) => path,
                None => {
                    self.status = say!("link.no-such-file", link.target);
                    return;
                }
            },
            None => match self.current_file_path() {
                Some(path) => path,
                None => {
                    self.status = say!("lsp.buffer-unsaved");
                    return;
                }
            },
        };
        let root = self.root();
        let index = self.links_index();
        let Some(node) = index.index_of(&target) else {
            self.status = say!("links.not-a-node");
            return;
        };
        let items: Vec<crate::picker::Item> = index
            .inbound(node)
            .into_iter()
            .map(|(src, link)| {
                let path = index.files[src].clone();
                let name = path.strip_prefix(&root).unwrap_or(&path).display().to_string();
                let column = link.excerpt[..link.span.start].chars().count();
                let label = format!("{name}:{}  {}", link.line + 1, link.excerpt);
                crate::picker::Item::Place(label, path, link.line, column)
            })
            .collect();
        if items.is_empty() {
            self.status = say!("links.no-backlinks");
            return;
        }
        let picker = crate::picker::Picker::new(&say!("picker.backlinks", items.len()), items);
        self.show_the_picker(picker);
    }

    /// The rows of the links panel, in whichever look it has.
    pub(super) fn links_rows(&mut self) -> Vec<crate::sidebar::Row> {
        let mut keys: Vec<Option<String>> = Vec::new();
        let rows = self.links_rows_inner(&mut keys);
        self.links_row_keys = keys;
        rows
    }

    fn links_rows_inner(&mut self, keys: &mut Vec<Option<String>>) -> Vec<crate::sidebar::Row> {
        let root = self.root();
        let Some(path) = self.current_file_path() else {
            return vec![quiet(say!("lsp.buffer-unsaved"), keys)];
        };
        if !crate::links::is_node(&path) {
            return vec![quiet(say!("links.not-a-node"), keys)];
        }
        if self.links.is_none() {
            self.collect_links_index();
        }
        if self.links.is_none() {
            self.start_links_index();
            // A small project answers within the blink the panel takes to
            // open; a large one shows 「reading…」 and fills in on a later
            // frame. Either way the key thread is held for a tenth of a
            // second at most.
            self.wait_for_links_index(std::time::Duration::from_millis(100));
        }
        if self.links.is_none() {
            return vec![quiet(say!("links.reading"), keys)];
        }
        let look = self.links_look;
        let shut = self.links_shut.clone();
        let mentions = match shut.contains(MENTIONS) {
            true => None,
            false => Some(self.unlinked_mentions(&path)),
        };
        let index = self.links.as_ref().expect("checked above");
        let Some(node) = index.index_of(&path) else {
            return vec![quiet(say!("links.not-a-node"), keys)];
        };
        let spell = |i: usize| spell(&root, &index.files[i]);
        let mut rows = Vec::new();
        match look {
            Look::List => {
                let inbound = index.inbound(node);
                let outbound = index.outbound(node);
                let dangling = index.dangling(node);
                let empty = inbound.is_empty() && outbound.is_empty() && dangling.is_empty();
                for (key, label, links) in [
                    (IN, say!("links.in"), inbound),
                    (OUT, say!("links.out"), outbound),
                ] {
                    let open = !shut.contains(key);
                    rows.push(header(&label, open, Some(links.len()), keys, key));
                    if !open {
                        continue;
                    }
                    for (other, link) in links {
                        rows.push(line_row(&spell(other), index.files[other].clone(), link, keys));
                    }
                }
                if !dangling.is_empty() {
                    let open = !shut.contains(DANGLING);
                    rows.push(header(&say!("links.dangling"), open, Some(dangling.len()), keys, DANGLING));
                    if open {
                        for link in dangling {
                            rows.push(line_row(&link.target, index.files[node].clone(), link, keys));
                        }
                    }
                }
                let open = mentions.is_some();
                let count = mentions.as_ref().map(Vec::len);
                rows.push(header(&say!("links.mentions"), open, count, keys, MENTIONS));
                if let Some(found) = mentions {
                    for (file, line, excerpt, span) in found {
                        let fake = crate::links::Link { line, target: String::new(), excerpt, span };
                        rows.push(line_row(&spell(file), index.files[file].clone(), &fake, keys));
                    }
                }
                if empty {
                    rows.insert(0, quiet(say!("links.none"), keys));
                    keys.rotate_right(1);
                }
            }
            Look::Graph => {
                let neighbours = index.neighbours(node);
                let listed: Vec<usize> = neighbours.iter().map(|n| n.node).collect();
                let (above, below): (Vec<_>, Vec<_>) = neighbours.iter().partition(|n| n.inward);
                let fan = |side: &[&crate::links::Neighbour], rows: &mut Vec<crate::sidebar::Row>, keys: &mut Vec<Option<String>>| {
                    let shown = side.len().min(MOST_NEIGHBOURS);
                    for (i, n) in side.iter().take(MOST_NEIGHBOURS).enumerate() {
                        let arrow = match (n.inward, n.outward) {
                            (true, true) => "↔",
                            (true, false) => "←",
                            _ => "→",
                        };
                        // The fan's rail: in-links hang above the centre and
                        // start at `┌`; out-links hang below and end at `└`.
                        let joint = match (n.inward, i == 0, i + 1 == shown && side.len() == shown) {
                            (true, true, _) => "┌",
                            (false, _, true) => "└",
                            _ => "├",
                        };
                        let own: Vec<usize> = index
                            .neighbours(n.node)
                            .into_iter()
                            .map(|m| m.node)
                            .filter(|&m| m != node && !listed.contains(&m))
                            .collect();
                        // A neighbour's own neighbours — the second hop —
                        // start folded; `l` opens one. Sections work the
                        // other way round, so the set holds 「open」 for a
                        // node and 「shut」 for a section.
                        let key = format!("node:{}", index.files[n.node].display());
                        let open = shut.contains(&key);
                        let mark = match (own.is_empty(), open) {
                            (true, _) => " ",
                            (false, true) => "▾",
                            (false, false) => "▸",
                        };
                        rows.push(crate::sidebar::Row {
                            path: index.files[n.node].clone(),
                            name: format!("{joint}{mark}{} {arrow}", spell(n.node)),
                            depth: 0,
                            is_dir: true,
                            expanded: open,
                            wash: None,
                            quiet: false,
                            marks: Vec::new(),
                            aside: Some(index.degree(n.node).to_string()),
                        });
                        keys.push(Some(key));
                        if open {
                            for &m in own.iter().take(MOST_NEIGHBOURS) {
                                rows.push(crate::sidebar::Row {
                                    path: index.files[m].clone(),
                                    name: format!("│   · {}", spell(m)),
                                    depth: 0,
                                    is_dir: true,
                                    expanded: false,
                                    wash: None,
                                    quiet: false,
                                    marks: Vec::new(),
                                    aside: Some(index.degree(m).to_string()),
                                });
                                keys.push(None);
                            }
                            if own.len() > MOST_NEIGHBOURS {
                                rows.push(quiet(say!("links.more", own.len() - MOST_NEIGHBOURS), keys));
                            }
                        }
                    }
                    if side.len() > MOST_NEIGHBOURS {
                        rows.push(quiet(say!("links.more", side.len() - MOST_NEIGHBOURS), keys));
                    }
                };
                fan(&above, &mut rows, keys);
                rows.push(crate::sidebar::Row {
                    path: PathBuf::new(),
                    name: format!("▣ {}", spell(node)),
                    depth: 0,
                    is_dir: false,
                    expanded: true,
                    wash: None,
                    quiet: false,
                    marks: Vec::new(),
                    aside: Some(neighbours.len().to_string()),
                });
                keys.push(None);
                fan(&below, &mut rows, keys);
                if neighbours.is_empty() {
                    rows.push(quiet(say!("links.none"), keys));
                }
            }
        }
        rows
    }

    /// Lines of other files that write this file's name without brackets —
    /// Obsidian's 「unlinked mentions」. Read when asked for, kept until the
    /// index changes. A name under two characters is not looked for: 「雪」
    /// is in every chapter.
    fn unlinked_mentions(&self, path: &Path) -> Vec<crate::links::Mention> {
        let Some(index) = self.links.as_ref() else { return Vec::new() };
        let Some(node) = index.index_of(path) else { return Vec::new() };
        if let Some((n, g, found)) = self.links_mentions.borrow().as_ref() {
            if *n == node && *g == index.generation {
                return found.clone();
            }
        }
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut found = Vec::new();
        if name.chars().count() >= 2 {
            for (i, file) in index.files.iter().enumerate() {
                if i == node {
                    continue;
                }
                let text = match self.buffers.iter().find(|b| b.path() == Some(file.as_path())) {
                    Some(open) => open.rope().to_string(),
                    None => std::fs::read_to_string(file).unwrap_or_default(),
                };
                for (n, line) in text.lines().enumerate() {
                    let mut from = 0;
                    while let Some(at) = line[from..].find(&name) {
                        let start = from + at;
                        let bracketed = line[..start].ends_with("[[")
                            || line[..start].ends_with('|')
                            || line[..start].ends_with('/');
                        if !bracketed {
                            let trimmed = line.trim();
                            let shift = line.len() - line.trim_start().len();
                            let a = start.saturating_sub(shift).min(trimmed.len());
                            let b = (start + name.len()).saturating_sub(shift).min(trimmed.len());
                            found.push((i, n, trimmed.to_string(), a..b));
                            break;
                        }
                        from = start + name.len();
                    }
                }
            }
        }
        *self.links_mentions.borrow_mut() = Some((node, index.generation, found.clone()));
        found
    }

    /// The fold key of the highlighted links row, if it folds.
    fn links_row_key(&self) -> Option<String> {
        let side = self.showing(crate::sidebar::View::Links)?;
        let panel = self.panel(side)?;
        self.links_row_keys.get(panel.selected())?.clone()
    }

    /// Keys the links panel answers before the list does: `v` flips the
    /// look, `h`/`l` fold and open a section or a neighbour. `true` when
    /// taken.
    pub(super) fn on_links_key(&mut self, key: Key) -> bool {
        if self.showing(crate::sidebar::View::Links).is_none() {
            return false;
        }
        match key {
            Key::Char('v') => {
                self.links_look = match self.links_look {
                    Look::List => Look::Graph,
                    Look::Graph => Look::List,
                };
                self.refresh_sidebar();
                true
            }
            Key::Char('h') | Key::Left => {
                let Some(fold) = self.links_row_key() else { return true };
                match fold.starts_with("node:") {
                    true => self.links_shut.remove(&fold),
                    false => self.links_shut.insert(fold),
                };
                self.refresh_sidebar();
                true
            }
            Key::Char('l') | Key::Right => {
                let Some(fold) = self.links_row_key() else { return false };
                let opened = match fold.starts_with("node:") {
                    true => self.links_shut.insert(fold),
                    false => self.links_shut.remove(&fold),
                };
                if !opened {
                    // Already open: `l` goes on to mean 「take me there」.
                    return false;
                }
                self.refresh_sidebar();
                true
            }
            _ => false,
        }
    }
}

/// Every Markdown file of the project with its text, read with the same
/// sieve the search uses — hidden and ignored folders left out.
fn read_project(root: &Path) -> Vec<(PathBuf, String)> {
    let sieve = crate::editor::Sieve { uncapped: true, ..Default::default() };
    let mut texts = Vec::new();
    crate::editor::walk_prose(root, &sieve, &mut |path| {
        if !crate::links::is_node(path) {
            return;
        }
        if let Ok(text) = std::fs::read_to_string(path) {
            texts.push((path.to_path_buf(), text));
        }
    });
    texts
}

/// How the panel names a node: its path from the project root, without the
/// suffix.
fn spell(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).with_extension("").to_string_lossy().replace('\\', "/")
}

fn quiet(text: String, keys: &mut Vec<Option<String>>) -> crate::sidebar::Row {
    keys.push(None);
    crate::sidebar::Row {
        path: PathBuf::new(),
        name: text,
        depth: 0,
        is_dir: false,
        expanded: false,
        wash: None,
        quiet: true,
        marks: Vec::new(),
        aside: None,
    }
}

fn header(label: &str, open: bool, count: Option<usize>, keys: &mut Vec<Option<String>>, key: &str) -> crate::sidebar::Row {
    keys.push(Some(key.to_string()));
    crate::sidebar::Row {
        path: PathBuf::new(),
        name: format!("{} {label}", if open { "▾" } else { "▸" }),
        depth: 0,
        is_dir: true,
        expanded: open,
        wash: None,
        quiet: false,
        marks: Vec::new(),
        aside: count.map(|n| n.to_string()),
    }
}

/// One link's line: the file, the line number, and the text cut around the
/// brackets, which are marked.
fn line_row(file: &str, path: PathBuf, link: &crate::links::Link, keys: &mut Vec<Option<String>>) -> crate::sidebar::Row {
    keys.push(None);
    let (excerpt, span) = around(&link.excerpt, link.span.clone());
    let head = format!("    {file}:{}  ", link.line + 1);
    let mut marks = Vec::new();
    if !span.is_empty() {
        marks.push(head.len() + span.start..head.len() + span.end);
    }
    crate::sidebar::Row {
        path,
        name: format!("{head}{excerpt}"),
        depth: link.line,
        is_dir: false,
        expanded: false,
        wash: None,
        quiet: false,
        marks,
        aside: None,
    }
}

/// `text` cut to [`AROUND`] characters either side of `span`, with `…` where
/// it was cut; the span moved to match.
fn around(text: &str, span: std::ops::Range<usize>) -> (String, std::ops::Range<usize>) {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let at = |byte: usize| chars.iter().position(|&(b, _)| b >= byte).unwrap_or(chars.len());
    let (a, b) = (at(span.start), at(span.end.min(text.len())));
    let from = a.saturating_sub(AROUND);
    let to = (b + AROUND).min(chars.len());
    let mut out = String::new();
    if from > 0 {
        out.push('…');
    }
    let start = out.len() + chars[from..a].iter().map(|(_, c)| c.len_utf8()).sum::<usize>();
    out.extend(chars[from..to].iter().map(|(_, c)| c));
    let end = start + chars[a..b].iter().map(|(_, c)| c.len_utf8()).sum::<usize>();
    if to < chars.len() {
        out.push('…');
    }
    (out, start..end)
}

/// The fold set: sections in it are shut, neighbours in it are open. It
/// starts with the mentions shut, as Obsidian's does, and no neighbour open.
pub(super) fn folded_at_first() -> BTreeSet<String> {
    BTreeSet::from([MENTIONS.to_string()])
}
