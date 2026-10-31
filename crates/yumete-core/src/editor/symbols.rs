//! **helix's four pickers on `空格 s` `S` `d` `D`** (2026-10-31): this file's
//! symbols, the project's symbols, this file's diagnostics, the project's.
//!
//! Measured against helix 25.07.1 with rust-analyzer: the symbol picker's
//! columns are kind, name and container, the project one adds the path, and
//! is asked again as the query changes; the diagnostics pickers' are
//! severity, source, code and message, the project one with the path before
//! the message. helix refuses the symbol pickers where no language server
//! answers; here they fall back to what yumete reads itself (decided
//! 2026-10-31): this file's outline — headings, or tree-sitter's definitions
//! in code — and the headings of every Markdown and Typst file in the
//! project.

use super::*;

/// What a symbol picker asked a language server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolAsk {
    /// `textDocument/documentSymbol` for this file.
    Document(PathBuf),
    /// `workspace/symbol` with this query.
    Workspace(String),
}

/// Which symbol picker an answer is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SymbolWant {
    /// This buffer's (its id), at this path.
    Document(u64, PathBuf),
    Project,
}

/// One heading: its line, its level, its title.
type Heading = (usize, usize, String);

/// Rows of cells as one string each, every column but the last padded to
/// its widest cell (capped, so one long name does not push the rest off).
fn columns(rows: &[Vec<String>]) -> Vec<String> {
    const WIDEST: usize = 36;
    let n = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut wide = vec![0usize; n];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            wide[i] = wide[i].max(yumete_cjk::str_width(cell).min(WIDEST));
        }
    }
    rows.iter()
        .map(|row| {
            let last = row.len().saturating_sub(1);
            let mut out = String::new();
            for (i, cell) in row.iter().enumerate() {
                out.push_str(cell);
                if i < last {
                    let pad = wide[i].saturating_sub(yumete_cjk::str_width(cell));
                    out.push_str(&" ".repeat(pad + 1));
                }
            }
            out.trim_end().to_string()
        })
        .collect()
}

/// The headings of a file read from disk: ATX headings (`#` in Markdown,
/// `=` in Typst), with fenced code and a front-matter block skipped. Plainer
/// than the open file's outline, which knows every block; enough to find a
/// chapter by its title.
fn headings_in(text: &str, mark: char) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut fenced = false;
    let mut front = false;
    for (n, line) in text.lines().enumerate() {
        let trimmed = line.trim_end();
        if n == 0 && trimmed == "---" {
            front = true;
            continue;
        }
        if front {
            front = trimmed != "---";
            continue;
        }
        let bare = trimmed.trim_start();
        if bare.starts_with("```") || bare.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let Some(depth) = crate::markdown::heading_marks(trimmed, mark) else { continue };
        let title = trimmed[depth..].trim();
        if !title.is_empty() {
            out.push((n, depth, title.to_string()));
        }
    }
    out
}

/// The heading each heading sits under: for every entry, the title of the
/// nearest one before it with a smaller level.
fn containers(entries: &[(usize, usize, String)]) -> Vec<String> {
    let mut open: Vec<(usize, String)> = Vec::new();
    entries
        .iter()
        .map(|(_, level, title)| {
            while open.last().is_some_and(|(l, _)| l >= level) {
                open.pop();
            }
            let above = open.last().map(|(_, t)| t.clone()).unwrap_or_default();
            open.push((*level, title.clone()));
            above
        })
        .collect()
}

impl Editor {
    /// **`空格 s`, 符號挑選器**: what this file defines. The outline is in it at
    /// once; a language server's answer, when one comes, takes its place.
    pub(super) fn open_symbol_picker(&mut self) {
        let mut picker = crate::picker::Picker::new(&say!("picker.symbols"), Vec::new());
        picker.verbatim = true;
        if !self.show_the_picker(picker) {
            return;
        }
        self.fill_with_outline();
        if let Some(path) = self.current_buffer().path().map(Path::to_path_buf) {
            self.symbols_wanted = Some(SymbolWant::Document(self.current_buffer().id(), path.clone()));
            self.symbols_query = Some(SymbolAsk::Document(path));
        }
    }

    /// **`空格 S`, 項目符號挑選器**: what the project defines, asked of the
    /// server as the query changes; the project's headings where no server
    /// serves this file.
    pub(super) fn open_project_symbol_picker(&mut self) {
        let mut picker = crate::picker::Picker::new(&say!("picker.project-symbols"), Vec::new());
        picker.verbatim = true;
        picker.live = true;
        if !self.show_the_picker(picker) {
            return;
        }
        self.symbols_wanted = Some(SymbolWant::Project);
        self.symbols_query = Some(SymbolAsk::Workspace(String::new()));
    }

    /// The question for the language server, taken once.
    pub fn take_symbols_query(&mut self) -> Option<SymbolAsk> {
        self.symbols_query.take()
    }

    /// **A live picker's query changed**: ask again with it.
    pub(super) fn ask_symbols_again(&mut self, query: &str) {
        if self.symbols_wanted == Some(SymbolWant::Project) {
            self.symbols_query = Some(SymbolAsk::Workspace(query.to_string()));
        }
    }

    /// **No language server serves this file**: what yumete reads itself.
    pub fn symbols_without_a_server(&mut self, ask: SymbolAsk) {
        match ask {
            // The outline is already in the picker.
            SymbolAsk::Document(_) => {}
            SymbolAsk::Workspace(_) => {
                if let Some(picker) = self.symbols_picker_mut(&say!("picker.project-symbols")) {
                    picker.live = false;
                }
                let items = self.project_headings();
                if let Some(picker) = self.symbols_picker_mut(&say!("picker.project-symbols")) {
                    picker.replace_items(items);
                }
            }
        }
    }

    /// **A server's symbols**, into the picker that asked, if it is still
    /// open. An empty answer for this file leaves the outline standing.
    pub fn show_symbols(&mut self, symbols: Vec<crate::lsp::Symbol>) {
        let Some(want) = self.symbols_wanted.clone() else { return };
        let root = self.project_root();
        let (title, items) = match want {
            SymbolWant::Document(id, path) => {
                if self.current_buffer().id() != id || symbols.is_empty() {
                    return;
                }
                let rows: Vec<Vec<String>> = symbols
                    .iter()
                    .map(|s| vec![s.kind.to_string(), s.name.clone(), s.container.clone().unwrap_or_default()])
                    .collect();
                let items = columns(&rows)
                    .into_iter()
                    .zip(&symbols)
                    .map(|(label, s)| self.place_item(label, &path, s.line, s.utf16_column))
                    .collect();
                (say!("picker.symbols"), items)
            }
            SymbolWant::Project => {
                let rows: Vec<Vec<String>> = symbols
                    .iter()
                    .map(|s| {
                        let path = s.path.as_deref().map(|p| p.strip_prefix(&root).unwrap_or(p).display().to_string());
                        vec![s.kind.to_string(), s.name.clone(), s.container.clone().unwrap_or_default(), path.unwrap_or_default()]
                    })
                    .collect();
                let items = columns(&rows)
                    .into_iter()
                    .zip(&symbols)
                    .filter_map(|(label, s)| Some(self.place_item(label, s.path.as_deref()?, s.line, s.utf16_column)))
                    .collect();
                (say!("picker.project-symbols"), items)
            }
        };
        if let Some(picker) = self.symbols_picker_mut(&title) {
            picker.replace_items(items);
        }
    }

    /// The open picker, when it is the one called `title`.
    fn symbols_picker_mut(&mut self, title: &str) -> Option<&mut crate::picker::Picker> {
        self.picker.as_mut().filter(|p| p.title == title)
    }

    /// A row that goes to `line` of `path` at a UTF-16 column, as a server
    /// counts it.
    fn place_item(&self, label: String, path: &Path, line: usize, utf16: usize) -> crate::picker::Item {
        let text = self.line_of_file(path, line).unwrap_or_default();
        let column = crate::problem::char_column(&text, utf16);
        crate::picker::Item::Place(label, path.to_path_buf(), line, column)
    }

    /// This file's outline as the symbol picker's rows: the heading marks as
    /// the kind (`##`, `==`), or nothing for code's definitions, and the
    /// heading each sits under.
    fn fill_with_outline(&mut self) {
        let entries = self.outline();
        let mark = match self.current_buffer().syntax() {
            crate::syntax::Syntax::Markdown => Some('#'),
            crate::syntax::Syntax::Typst => Some('='),
            _ => None,
        };
        let above = containers(&entries);
        let rows: Vec<Vec<String>> = entries
            .iter()
            .zip(&above)
            .map(|((_, level, title), up)| {
                let kind = mark.map(|m| m.to_string().repeat(*level)).unwrap_or_default();
                vec![kind, title.clone(), up.clone()]
            })
            .collect();
        let path = self.current_buffer().path().map(Path::to_path_buf).unwrap_or_default();
        let items = columns(&rows)
            .into_iter()
            .zip(&entries)
            .map(|(label, (line, _, _))| crate::picker::Item::Place(label, path.clone(), *line, 0))
            .collect();
        if let Some(picker) = self.symbols_picker_mut(&say!("picker.symbols")) {
            picker.replace_items(items);
        }
    }

    /// Every heading in the project's Markdown and Typst files, with the
    /// heading above it and the file.
    fn project_headings(&self) -> Vec<crate::picker::Item> {
        let root = self.project_root();
        let sieve = crate::editor::Sieve { uncapped: true, ..Default::default() };
        let mut found: Vec<(PathBuf, Vec<Heading>)> = Vec::new();
        crate::editor::walk_prose(&root, &sieve, &mut |path| {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let mark = match crate::syntax::from_extension(&name) {
                Some(crate::syntax::Syntax::Markdown) => '#',
                Some(crate::syntax::Syntax::Typst) => '=',
                _ => return,
            };
            if let Ok(text) = std::fs::read_to_string(path) {
                found.push((path.to_path_buf(), headings_in(&text, mark)));
            }
        });
        found.sort_by(|a, b| a.0.cmp(&b.0));
        let mut rows = Vec::new();
        let mut places = Vec::new();
        for (path, entries) in &found {
            let mark = if path.extension().is_some_and(|e| e == "typ") { '=' } else { '#' };
            let shown = path.strip_prefix(&root).unwrap_or(path).display().to_string();
            for ((line, level, title), up) in entries.iter().zip(containers(entries)) {
                rows.push(vec![mark.to_string().repeat(*level), title.clone(), up, shown.clone()]);
                places.push((path.clone(), *line));
            }
        }
        columns(&rows)
            .into_iter()
            .zip(places)
            .map(|(label, (path, line))| crate::picker::Item::Place(label, path, line, 0))
            .collect()
    }

    /// **`空格 d` / `空格 D`, 診斷挑選器 / 項目診斷挑選器**: what the servers
    /// said about this file, or about every file, in helix's columns.
    pub(super) fn open_diagnostics_picker(&mut self, project: bool) {
        use crate::problem::Severity;
        let root = self.project_root();
        let files: Vec<PathBuf> = match project {
            true => {
                let mut all: Vec<PathBuf> = self.problems.files().into_iter().map(|(p, _)| p.to_path_buf()).collect();
                all.sort();
                all
            }
            false => self.current_buffer().path().map(Path::to_path_buf).into_iter().collect(),
        };
        let mut rows = Vec::new();
        let mut places = Vec::new();
        for path in &files {
            let mut said: Vec<crate::problem::Problem> = self.problems.of(path).to_vec();
            // By line, the worst first on a line, as helix lists them.
            said.sort_by_key(|p| (p.line, std::cmp::Reverse(p.severity), p.utf16_column));
            for problem in said {
                let severity = match problem.severity {
                    Severity::Error => "ERROR",
                    Severity::Warn => "WARN",
                    Severity::Note => "INFO",
                    Severity::Hint => "HINT",
                };
                let message = problem.message.lines().next().unwrap_or_default().to_string();
                let mut row = vec![
                    severity.to_string(),
                    problem.source.clone().unwrap_or_default(),
                    problem.code.clone().unwrap_or_default(),
                ];
                if project {
                    row.push(path.strip_prefix(&root).unwrap_or(path).display().to_string());
                }
                row.push(message);
                rows.push(row);
                places.push((path.clone(), problem.line, problem.utf16_column));
            }
        }
        let items: Vec<crate::picker::Item> = columns(&rows)
            .into_iter()
            .zip(places)
            .map(|(label, (path, line, utf16))| self.place_item(label, &path, line, utf16))
            .collect();
        let title = match project {
            true => say!("picker.project-diagnostics"),
            false => say!("picker.diagnostics"),
        };
        let mut picker = crate::picker::Picker::new(&title, items);
        picker.verbatim = true;
        self.show_the_picker(picker);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_on_disk_skip_fences_and_front_matter() {
        let text = "---\ntitle: x\n---\n# 第一章\n\n```\n# not this\n```\n## 雪\n#### deep\n# 第二章\n";
        let got = headings_in(text, '#');
        let titles: Vec<&str> = got.iter().map(|(_, _, t)| t.as_str()).collect();
        assert_eq!(titles, ["第一章", "雪", "deep", "第二章"]);
        assert_eq!(containers(&got), ["", "第一章", "雪", ""]);
    }

    #[test]
    fn columns_pad_every_cell_but_the_last_by_its_width() {
        let rows = vec![
            vec!["##".to_string(), "雪".to_string(), "第一章".to_string()],
            vec!["#".to_string(), "第二章".to_string(), String::new()],
        ];
        assert_eq!(columns(&rows), ["## 雪     第一章", "#  第二章"]);
    }
}
