//! The sidebar, the pickers, and the clipboard (#94).
//!
//! One column down the side showing files, buffers, the outline or the
//! dictionary; the pickers that open over it; and the copy/paste that the
//! front end has to be asked for, because a terminal cannot read the
//! clipboard by itself.

use super::*;

impl Editor {
    // ---- The side panels (Feature #94, #293) -------------------------------

    /// **Which slot a panel lives in — the one place that decides it** (#293).
    ///
    /// One answer per panel, because a reader may want the outline across from
    /// the tree, or the 字典 stacked under it. ⚠️ **`Tab` then walks only the
    /// views that share a slot** ([`Editor::cycle_view`]): the motion belongs
    /// to the column, not to the list of views.
    pub fn side_of(&self, panel: crate::sidebar::Panel) -> crate::sidebar::Side {
        self.sides[panel as usize]
    }

    /// Put that panel on that side.
    ///
    /// Whatever is already open moves with it, because a panel that stayed
    /// where the old setting put it would make the setting a lie until the
    /// next restart. A slot that was busy hands what it held back to the slot
    /// this one just left, so nothing is silently closed.
    pub fn set_side(&mut self, panel: crate::sidebar::Panel, side: crate::sidebar::Side) {
        use crate::sidebar::{Layer, View};
        let was = self.side_of(panel);
        self.sides[panel as usize] = side;
        if was == side {
            return;
        }
        // A resident view that is showing goes across; a transient one has
        // nothing to carry, since it is worked out afresh every frame.
        match View::ALL.into_iter().find(|&v| crate::sidebar::Panel::from(v) == panel) {
            Some(view) if self.showing(view) == Some(was) => {
                let moving = self.panels[was as usize].take();
                let focused = self.panel_focus == Some((was, Layer::Top));
                let displaced = self.panels[side as usize].take();
                self.panels[side as usize] = moving;
                self.panels[was as usize] = displaced;
                if focused {
                    self.panel_focus = Some((side, Layer::Top));
                }
            }
            Some(_) => {}
            None if self.panel_focus == Some((was, Layer::Bottom)) => {
                self.panel_focus = Some((side, Layer::Bottom));
            }
            None => {}
        }
        self.refresh_sidebar();
    }

    /// Which slot a view opens in.
    pub(super) fn side_for(&self, view: crate::sidebar::View) -> crate::sidebar::Side {
        self.side_of(view.into())
    }

    /// `Tab` in a slot: the next view **that lives in this slot**, wrapping.
    ///
    /// A slot with one view in it has nowhere to go, and says so rather than
    /// looking broken.
    pub(super) fn cycle_view(&mut self, side: crate::sidebar::Side, back: bool) {
        let Some(here) = self.panel(side).map(|p| p.view()) else {
            return;
        };
        let mine: Vec<crate::sidebar::View> = crate::sidebar::View::ALL
            .into_iter()
            .filter(|&v| self.side_for(v) == side)
            .collect();
        if mine.len() < 2 {
            self.status = say!("sidebar.only-view-on-this-side");
            return;
        }
        let at = mine.iter().position(|&v| v == here).unwrap_or(0);
        let n = mine.len();
        let next = mine[match back {
            true => (at + n - 1) % n,
            false => (at + 1) % n,
        }];
        if let Some(panel) = self.panel_mut(side) {
            panel.show(next);
        }
        self.refresh_sidebar();
    }

    /// The panel in that slot, for the front end to draw.
    pub fn panel(&self, side: crate::sidebar::Side) -> Option<&crate::sidebar::Sidebar> {
        self.panels[side as usize].as_ref()
    }

    /// The same, to be moved about in.
    pub(super) fn panel_mut(
        &mut self,
        side: crate::sidebar::Side,
    ) -> Option<&mut crate::sidebar::Sidebar> {
        self.panels[side as usize].as_mut()
    }

    /// **What the bottom of that slot is showing** — worked out afresh, never
    /// stored (#293).
    ///
    /// **Order is the whole rule** when both live on this side: asking about a
    /// character is a thing a reader just did; what the cursor is standing in
    /// has been true all along. The question wins while it is live, and when
    /// it stops being live the row underneath simply shows again — nobody
    /// remembered it, and nobody put it back.
    pub fn transient(&self, side: crate::sidebar::Side) -> Option<crate::sidebar::Transient> {
        use crate::sidebar::{Panel, Transient};
        if self.side_of(Panel::Dictionary) == side && self.dictionary_live() {
            return Some(Transient::Dictionary);
        }
        if self.side_of(Panel::Detail) == side && self.detail_is_a_panel() {
            return Some(Transient::Detail);
        }
        None
    }

    /// **Whether the 字典 question is still being asked** (#215, #293).
    ///
    /// It is, while the cursor has not moved off the character it was asked
    /// about — or while the keys are in the panel, where the cursor cannot
    /// move at all, so a long answer can be read to the end.
    ///
    /// ⚠️ **Reads the stored focus, not [`Editor::panel_focus`]**: that one
    /// asks whether the layer is showing, which asks this, which would ask it
    /// again. The stored field is the right one anyway — the question is
    /// 「were the keys put here」, not 「is there something here to look at」.
    fn dictionary_live(&self) -> bool {
        if self.dictionary.is_none() {
            return false;
        }
        let reading = self.panel_focus
            == Some((
                self.side_of(crate::sidebar::Panel::Dictionary),
                crate::sidebar::Layer::Bottom,
            ));
        reading || self.dictionary_anchor == Some(self.cursor)
    }

    /// The rows of the bottom layer, when it is one that is drawn as a list.
    pub fn transient_rows(&self, side: crate::sidebar::Side) -> Vec<crate::sidebar::Row> {
        match self.transient(side) {
            Some(crate::sidebar::Transient::Dictionary) => self.dictionary_rows(),
            _ => Vec::new(),
        }
    }

    /// Whether the detail is the sort that wants a **panel** rather than the
    /// floating note (#294).
    ///
    /// A row has twenty-eight fields and is a tall thing wherever it is
    /// written; a footnote is one short paragraph, and taking a slot off the
    /// page for it would be paying the wrong price — it floats over the
    /// writing instead, near the cursor.
    fn detail_is_a_panel(&self) -> bool {
        self.detail_visible()
            && (self.detail_shows_a_row()
                || self.table.as_ref().is_some_and(|view| view.takes_the_pane()))
    }

    /// Whether that layer of that slot has anything in it to look at.
    pub fn layer_showing(&self, side: crate::sidebar::Side, layer: crate::sidebar::Layer) -> bool {
        match layer {
            crate::sidebar::Layer::Top => self.panel(side).is_some(),
            crate::sidebar::Layer::Bottom => self.transient(side).is_some(),
        }
    }

    /// Whether that layer is a place the keys can be — a seat on `C-w`'s ring.
    ///
    /// Not the same question as [`Editor::layer_showing`]: a panel can be
    /// worth reading and still be no place to stand
    /// ([`crate::sidebar::Transient::takes_keys`]).
    fn layer_takes_keys(&self, side: crate::sidebar::Side, layer: crate::sidebar::Layer) -> bool {
        match layer {
            crate::sidebar::Layer::Top => self.panel(side).is_some(),
            crate::sidebar::Layer::Bottom => {
                self.transient(side).is_some_and(|kind| kind.takes_keys())
            }
        }
    }

    /// Which slot and layer the keys are in, if any.
    ///
    /// `None` when they are in the text — and also when what the focus names
    /// has since gone away, so a stale focus can never be reported as a live
    /// one. That second half is what lets the bottom layer be derived: it
    /// vanishes when the cursor moves off, and the keys fall back to the text
    /// without anybody having to put them there.
    pub fn panel_focus(&self) -> Option<(crate::sidebar::Side, crate::sidebar::Layer)> {
        let (side, layer) = self.panel_focus?;
        self.layer_showing(side, layer).then_some((side, layer))
    }


    /// Which slot is showing that view, if either is.
    pub(super) fn showing(&self, view: crate::sidebar::View) -> Option<crate::sidebar::Side> {
        crate::sidebar::Side::BOTH
            .into_iter()
            .find(|&side| self.panel(side).is_some_and(|p| p.view() == view))
    }

    /// What `Space e` and `Space o` do — one rule for both, so neither is the
    /// odd one out.
    ///
    /// A key that names a view answers three different intentions depending on
    /// what is already showing, and all three are what a reader means by
    /// pressing it:
    ///
    /// - nowhere → open it in its own slot, with the keys.
    /// - showing, without the keys → take the keys back.
    /// - showing, with the keys → put it away. Pressing the same key twice
    ///   undoes it, which is the one thing every toggle must do.
    /// - its slot busy with **another** view → switch that slot to this view
    ///   and take the keys. The key means "show me the outline", not "toggle
    ///   the panel".
    pub(super) fn show_sidebar(&mut self, view: crate::sidebar::View) {
        use crate::sidebar::Layer;
        if let Some(side) = self.showing(view) {
            match self.panel_focus() == Some((side, Layer::Top)) {
                true => self.close_panel(side),
                false => self.focus_layer(side, Layer::Top),
            }
            return;
        }
        let side = self.side_for(view);
        match self.panel_mut(side) {
            Some(panel) => {
                panel.show(view);
                self.panel_focus = Some((side, Layer::Top));
                self.refresh_sidebar();
            }
            None => {
                let root = self.project_root();
                self.open_sidebar_showing(&root, view);
            }
        }
    }

    /// Put that slot's resident panel away, and the keys back in the text if
    /// they were in it.
    pub(super) fn close_panel(&mut self, side: crate::sidebar::Side) {
        self.panels[side as usize] = None;
        if self.panel_focus.is_some_and(|(at, layer)| {
            at == side && layer == crate::sidebar::Layer::Top
        }) {
            self.panel_focus = None;
        }
    }

    /// **The keys every panel answers, wherever it sits.**
    ///
    /// ⚠️ 左欄、右欄、下層的臨時面板是**同一個組件擺在不同位置**，所以這一組鍵
    /// 必須是同一份。分成三份各自維護的代價已經付過一次：常駐面板和搜索面板都
    /// 有 `q`，臨時層漏了，於是 `空格 d` 打開的字典**關不掉**——`q`、`Esc`、`j`
    /// 全被那一句 `_ => {}` 吃掉，唯一的出路是 `C-w` 再移動光標，兩步，而且提示
    /// 行一個字都沒說。
    ///
    /// Tried **last**, so a panel's own meaning for a key still wins: the
    /// search panel spends `Space` on a switch when the keys are on one.
    /// Answers whether it took the key.
    ///
    /// `Esc` is deliberately **not** here — see `on_sidebar_key`: a panel with
    /// a field in it spends `Esc` on leaving Insert, and one press too many
    /// would then put the panel away. `q` is the door, and the hint row says
    /// so in every panel.
    pub(super) fn panel_key_in_common(
        &mut self,
        key: Key,
        side: crate::sidebar::Side,
        layer: crate::sidebar::Layer,
    ) -> bool {
        match key {
            Key::Ctrl('w') => self.cycle_region(),
            Key::Char('q') => match layer {
                crate::sidebar::Layer::Top => self.close_panel(side),
                crate::sidebar::Layer::Bottom => self.close_transient(side),
            },
            // **`:` opens the command line from in here too.** It used to be
            // swallowed, so a reader with the keys in a panel had no way to
            // run a command at all — and `:sidebar-show-left` is a command
            // *about* the panel you are standing in, which nobody could have
            // reached. The focus stays where it is while the line is typed, so
            // 「this one」 still means this one.
            Key::Char(':') => {
                self.mode = Mode::Command;
                self.command_line.clear();
                self.command_caret = 0;
            }
            // Space still opens the menu, so `Space e` closes the sidebar from
            // inside it exactly as it opened it.
            Key::Char(' ') => self.pending = Pending::Space,
            _ => return false,
        }
        true
    }

    /// Shut whichever transient layer this side is showing, and hand the keys
    /// back to the writing.
    ///
    /// ⚠️ **The layer was a trap without this** (2026-09-14). The design is
    /// that a 字典 answer needs no closing — it goes when the cursor leaves the
    /// character it was asked about. True, *until* `空格 d` opens it with the
    /// keys in it: focus alone keeps `dictionary_live()` true, so the cursor
    /// cannot leave, so nothing ends it. `q`, `Esc`, `j`, `d` all fell through
    /// `on_transient_key`'s `_ => {}`, and the only way out anybody could find
    /// was `C-w` and *then* a cursor move — two steps, and the hint line named
    /// neither. The reader who reported it said 「我想砸键盘」.
    pub(super) fn close_transient(&mut self, side: crate::sidebar::Side) {
        match self.transient(side) {
            Some(crate::sidebar::Transient::Dictionary) => {
                self.dictionary = None;
                self.dictionary_query = None;
                self.dictionary_anchor = None;
            }
            Some(crate::sidebar::Transient::Detail) => self.show_detail = Some(false),
            None => return,
        }
        // The keys go back to the writing, not to the panel above: the reader
        // asked to be rid of this, and landing them somewhere they did not ask
        // for is the same surprise one layer up.
        if self.panel_focus == Some((side, crate::sidebar::Layer::Bottom)) {
            self.panel_focus = None;
        }
    }

    /// Give that layer the keys, if it is a place they can be.
    pub(super) fn focus_layer(&mut self, side: crate::sidebar::Side, layer: crate::sidebar::Layer) {
        if !self.layer_takes_keys(side, layer) {
            return;
        }
        // A list keeps its own place; a transient panel is a fresh thing every
        // time it is walked into, so it is read from the top.
        if self.panel_focus != Some((side, layer)) && layer == crate::sidebar::Layer::Bottom {
            self.transient_scroll = 0;
        }
        self.panel_focus = Some((side, layer));
        self.refresh_sidebar();
    }

    /// How far the bottom layer has been scrolled (#293).
    pub fn transient_scroll(&self) -> usize {
        self.transient_scroll
    }

    /// One key in the bottom layer: it is read, not walked into.
    ///
    /// No `q` — there is nothing here anybody opened. No `l`/`Enter` — a
    /// reading is not a place to go. What is left is moving the eye down a
    /// long answer, spelled the way the text and the lists already spell it.
    fn on_transient_key(&mut self, key: Key, side: crate::sidebar::Side) {
        let last = self.transient_len(side).saturating_sub(1);
        let step = |at: usize, by: usize, down: bool| match down {
            true => at.saturating_add(by).min(last),
            false => at.saturating_sub(by),
        };
        match key {
            Key::Char('j') | Key::Down => self.transient_scroll = step(self.transient_scroll, 1, true),
            Key::Char('k') | Key::Up => self.transient_scroll = step(self.transient_scroll, 1, false),
            Key::Char('J') | Key::PageDown => {
                self.transient_scroll = step(self.transient_scroll, Self::PAGE_IN_A_LIST, true)
            }
            Key::Char('K') | Key::PageUp => {
                self.transient_scroll = step(self.transient_scroll, Self::PAGE_IN_A_LIST, false)
            }
            Key::Char('g') | Key::Home => self.transient_scroll = 0,
            Key::Char('G') | Key::End => self.transient_scroll = last,
            other => {
                self.panel_key_in_common(other, side, crate::sidebar::Layer::Bottom);
            }
        }
    }

    /// **`C-w`: hand the keys to the next region** — Feature #293.
    ///
    /// Left panel, the writing, the other work area, right panel, and round
    /// again, skipping whatever is not open. One key, one meaning: 「the next
    /// place the keys can be」. It used to mean two things in two places — the
    /// other pane from the text, back to the text from the panel — and a
    /// second panel is what made that untenable.
    ///
    /// ⚠️ **Not the same key as `空格 w`**, which stays 工作區 and nothing
    /// else: 「nothing open → open one」 is a thing this cannot do without
    /// swallowing it, and a writer with the file tree up would then have no
    /// one key left that splits the page.
    pub(super) fn cycle_region(&mut self) {
        use crate::sidebar::{Layer, Side};
        // `None` is the writing; the ones before it are the left slot's two
        // layers, the ones after it the right slot's, all in screen order.
        let seats = |ed: &Self, side: Side| -> Vec<Option<(Side, Layer)>> {
            Layer::BOTH
                .into_iter()
                .filter(|&layer| ed.layer_takes_keys(side, layer))
                .map(|layer| Some((side, layer)))
                .collect()
        };
        let mut ring = seats(self, Side::Left);
        let panes = 1 + usize::from(self.other_pane().is_some());
        let writing = ring.len();
        ring.extend(std::iter::repeat_n(None, panes));
        ring.extend(seats(self, Side::Right));
        let here = match self.panel_focus() {
            Some(seat) => ring.iter().position(|&r| r == Some(seat)),
            // The live half of the writing: its place in the ring is after
            // whatever the left slot took.
            None => Some(writing + self.live_pane().min(1)),
        };
        let Some(here) = here else { return };
        if ring.len() < 2 {
            return;
        }
        let next = (here + 1) % ring.len();
        match ring[next] {
            Some((side, layer)) => self.focus_layer(side, layer),
            None => {
                self.panel_focus = None;
                // Which half — the ring's index minus the left slot's seats.
                let want = next - writing;
                if panes > 1 && want != self.live_pane().min(1) {
                    self.switch_pane();
                }
            }
        }
    }

    /// Show the file tree rooted at `root` and give it the keys.
    pub fn open_sidebar_at(&mut self, root: &Path) {
        self.open_sidebar_showing(root, crate::sidebar::View::Explorer);
    }

    /// Show a panel rooted at `root`, opened on `view`, in that view's slot.
    pub fn open_sidebar_showing(&mut self, root: &Path, view: crate::sidebar::View) {
        let mut sidebar = crate::sidebar::Sidebar::new(root);
        sidebar.show(view);
        // Open on the file being written, so the tree says where you are rather
        // than making you find yourself in it.
        if let Some(path) = self.current_buffer().path() {
            if let Ok(full) = std::fs::canonicalize(path) {
                sidebar.reveal(&full);
            }
        }
        let side = self.side_for(view);
        self.panels[side as usize] = Some(sidebar);
        self.panel_focus = Some((side, crate::sidebar::Layer::Top));
        self.refresh_sidebar();
    }

    /// Fill the sidebar with whatever its current view shows.
    ///
    /// The tree builds its own rows from the file system; the other two are the
    /// editor's own knowledge, so they are pushed in from here.
    /// Refreshed when it is asked for, not on every keystroke.
    ///
    /// Building the outline walks the document, and doing that per key is the
    /// trap this editor has fallen into three times. Headings do not change
    /// while a sentence is being typed, so the views are rebuilt when the
    /// sidebar is opened, focused, switched, or the file under it changes —
    /// every moment a reader is about to look at it.
    pub(super) fn refresh_sidebar(&mut self) {
        for side in crate::sidebar::Side::BOTH {
            self.refresh_panel(side);
        }
    }

    /// Fill one slot with whatever the view in it shows.
    fn refresh_panel(&mut self, side: crate::sidebar::Side) {
        use crate::sidebar::{Row, View};
        let Some(view) = self.panel(side).map(|p| p.view()) else {
            return;
        };
        let rows = match view {
            View::Explorer => {
                if let Some(panel) = self.panel_mut(side) {
                    panel.rebuild();
                }
                return;
            }
            // `depth` carries the index the row stands for — the buffer's, or
            // the line's — since a flat list has no depth to spend.
            View::Buffers => self
                .buffers
                .iter()
                .enumerate()
                .map(|(i, b)| Row {
                    path: b.path().map(Path::to_path_buf).unwrap_or_default(),
                    name: format!(
                        "{}{}",
                        b.display_name(),
                        if b.is_modified() { " +" } else { "" }
                    ),
                    depth: i,
                    is_dir: false,
                    expanded: i == self.current,
                })
                .collect(),
            View::Outline => self.outline_rows(),
            // Its own store, its own shape: a form and a list of hits, not
            // rows of a tree (#419).
            View::Search => return,
            // Drawn from the cursor every frame, not from rows (#287).
            View::Wiki => return,
        };
        if let Some(panel) = self.panel_mut(side) {
            panel.set_rows(rows);
        }
    }

    /// The 大綱's headings, whichever way this document spells them.
    ///
    /// A typst master file is a table of contents and nothing else, so its
    /// outline reaches into the files it includes; everything else reads its
    /// own headings.
    fn outline_headings(&self) -> Vec<crate::sidebar::Heading> {
        if self.current_buffer().syntax() == crate::syntax::Syntax::Typst {
            return self.included_headings();
        }
        self.outline()
            .into_iter()
            .map(|(line, level, title)| crate::sidebar::Heading {
                path: PathBuf::new(),
                line,
                level,
                title,
            })
            .collect()
    }

    /// The 大綱's rows: its headings, with whatever is folded away left out
    /// (#37).
    ///
    /// `is_dir` says the heading has something under it and `expanded` whether
    /// that something is showing — the two fields the tree already spends on
    /// the same question, so the front end draws one mark for both views.
    fn outline_rows(&self) -> Vec<crate::sidebar::Row> {
        let headings = self.outline_headings();
        let folded = |key: &(PathBuf, usize)| {
            self.outline_panel()
                .is_some_and(|panel| panel.is_folded(key))
        };
        let mut rows = Vec::new();
        // The level of the shallowest fold currently hiding rows. Anything
        // deeper than it is inside that fold; the first row that is not ends
        // it, and *that* row is the one asked whether it folds in turn.
        let mut hidden_under: Option<usize> = None;
        for (i, heading) in headings.iter().enumerate() {
            if hidden_under.is_some_and(|level| heading.level > level) {
                continue;
            }
            hidden_under = None;
            let has_children = headings
                .get(i + 1)
                .is_some_and(|next| next.level > heading.level);
            let shut = has_children && folded(&heading.key());
            if shut {
                hidden_under = Some(heading.level);
            }
            rows.push(crate::sidebar::Row {
                path: heading.path.clone(),
                name: format!(
                    "{}{}",
                    "  ".repeat(heading.level.saturating_sub(1)),
                    heading.title
                ),
                depth: heading.line,
                is_dir: has_children,
                expanded: has_children && !shut,
            });
        }
        rows
    }

    /// `h` in the 大綱: fold what the highlight is on, or the heading holding
    /// it (#37).
    ///
    /// **One key for both, as in the tree**: pressing it again and again walks
    /// out of the branch rather than stopping at the first heading that has
    /// nothing to fold. A heading with nothing under it, or one already
    /// folded, has no fold of its own to close, so the one above it closes and
    /// takes the highlight.
    pub(super) fn fold_outline(&mut self) {
        let Some(here) = self.outline_row_key() else {
            return;
        };
        let headings = self.outline_headings();
        let Some(i) = headings.iter().position(|h| h.key() == here) else {
            return;
        };
        let has_children = headings
            .get(i + 1)
            .is_some_and(|next| next.level > headings[i].level);
        let shut = self
            .outline_panel()
            .is_some_and(|panel| panel.is_folded(&here));
        let target = match has_children && !shut {
            true => here,
            // The nearest heading above it that is shallower than it is.
            false => match headings[..i]
                .iter()
                .rposition(|h| h.level < headings[i].level)
            {
                Some(parent) => headings[parent].key(),
                None => return,
            },
        };
        self.set_outline_fold(target, true);
    }

    /// `l` in the 大綱: open a folded heading. Whether it was folded — if it
    /// was not, the key goes on to mean 「take me there」, as `Enter` does.
    pub(super) fn unfold_outline(&mut self) -> bool {
        let Some(here) = self.outline_row_key() else {
            return false;
        };
        if !self
            .outline_panel()
            .is_some_and(|panel| panel.is_folded(&here))
        {
            return false;
        }
        self.set_outline_fold(here, false);
        true
    }

    /// What the highlighted 大綱 row stands for: the file it is in and the
    /// line it is on, which is what the fold set remembers.
    fn outline_row_key(&self) -> Option<(PathBuf, usize)> {
        let panel = self.outline_panel()?;
        let row = panel.rows().get(panel.selected())?;
        Some((row.path.clone(), row.depth))
    }

    /// The panel showing the 大綱, whichever slot it is in.
    fn outline_panel(&self) -> Option<&crate::sidebar::Sidebar> {
        self.panel(self.showing(crate::sidebar::View::Outline)?)
    }

    /// Fold or open one heading, rebuild the rows, and keep the highlight on
    /// it — folding takes rows away, and a highlight that slid onto whatever
    /// filled the gap would be reading the wrong chapter.
    fn set_outline_fold(&mut self, key: (PathBuf, usize), folded: bool) {
        let Some(side) = self.showing(crate::sidebar::View::Outline) else {
            return;
        };
        if !self.panel_mut(side).is_some_and(|p| p.set_folded(key.clone(), folded)) {
            return;
        }
        self.refresh_sidebar();
        if let Some(panel) = self.panel_mut(side) {
            let at = panel
                .rows()
                .iter()
                .position(|r| (r.path.clone(), r.depth) == key);
            if let Some(at) = at {
                panel.select(at);
            }
        }
    }

    /// The 字典 panel's rows — Feature #215.
    ///
    /// The names are padded to the widest of them so the values line up down a
    /// column, and the padding is counted in **columns** rather than characters
    /// (`拆分` is two characters and four columns wide).
    ///
    /// A row with no value is a heading — the character itself at the top, and
    /// the 陸／臺／港 label above each block when the 拆分表 has more than one
    /// answer. `is_dir` is what the sidebar draws headings with; the flat views
    /// already spend the tree's fields on what they have instead of what a tree
    /// has, and this is that.
    fn dictionary_rows(&self) -> Vec<crate::sidebar::Row> {
        use crate::sidebar::Row;
        let heading = |name: String| Row {
            path: PathBuf::new(),
            name,
            depth: 0,
            is_dir: true,
            expanded: false,
        };
        let Some((ch, answer)) = self.dictionary.as_ref() else {
            return Vec::new();
        };
        let mut rows = vec![heading(ch.to_string())];
        let Some(fields) = answer else {
            return rows;
        };
        if fields.is_empty() {
            rows.push(Row {
                path: PathBuf::new(),
                name: say!("ui.not-in-the-table"),
                depth: 0,
                is_dir: false,
                expanded: false,
            });
            return rows;
        }
        let width = fields
            .iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(name, _)| yumete_cjk::str_width(name))
            .max()
            .unwrap_or(0);
        for (name, value) in fields {
            if value.is_empty() {
                rows.push(heading(name.clone()));
                continue;
            }
            let pad = " ".repeat(width.saturating_sub(yumete_cjk::str_width(name)));
            rows.push(Row {
                path: PathBuf::new(),
                name: format!("{name}{pad}  {value}"),
                depth: 0,
                is_dir: false,
                expanded: false,
            });
        }
        rows
    }

    /// Look this character up in the 拆分表 — `Space d`, and `Tab` on a
    /// candidate (#215).
    ///
    /// The editor does not hold the table: yume does, and only the front end
    /// has it. So the character is parked here and the panel is opened empty;
    /// the answer arrives on the next pass through the loop, one frame later,
    /// which is not long enough for a reader to see the gap.
    pub fn look_up(&mut self, ch: char, focus: bool) {
        self.dictionary_query = Some(ch);
        // Asked, unanswered: what is showing until the answer arrives is the
        // character alone, which is not the same panel as 「查不到」.
        self.dictionary = Some((ch, None));
        // **Where the question was asked from.** The answer stays up while the
        // cursor is still there and goes when it leaves — nothing has to close
        // it, which is the whole of why the bottom layer holds no state
        // (#293). Nothing is opened here either: the panel *is* the question,
        // and `transient()` will draw it because the question is live.
        self.dictionary_anchor = Some(self.cursor);
        self.transient_scroll = 0;
        // Asked from the page, the keys go with the question. Asked while a
        // word is being typed, they must not — the reader is mid-word, and the
        // panel is only there to be glanced at.
        self.panel_focus = focus.then_some((
            self.side_of(crate::sidebar::Panel::Dictionary),
            crate::sidebar::Layer::Bottom,
        ));
        self.refresh_sidebar();
    }

    /// The character `Space d` or `Tab` asked about, for the front end to
    /// answer once (#215).
    pub fn take_dictionary_query(&mut self) -> Option<char> {
        self.dictionary_query.take()
    }

    /// The answer to [`Editor::take_dictionary_query`].
    ///
    /// Dropped if the reader has since asked about a different character —
    /// the answer to last frame's question must not overwrite this frame's.
    pub fn set_dictionary(&mut self, ch: char, fields: Vec<(String, String)>) {
        if self.dictionary.as_ref().is_some_and(|(at, _)| *at != ch) {
            return;
        }
        self.dictionary = Some((ch, Some(fields)));
        self.refresh_sidebar();
    }

    /// The character the 字典 panel is about, and the answer if one has come.
    pub fn dictionary(&self) -> Option<(char, Option<&[(String, String)]>)> {
        self.dictionary
            .as_ref()
            .map(|(ch, answer)| (*ch, answer.as_deref()))
    }

    /// Whether the keys are in either panel.
    pub fn sidebar_focused(&self) -> bool {
        self.panel_focus().is_some()
    }

    /// What the sidebar's keys are, for the status line to say while it has
    /// them.
    ///
    /// A pane that takes the keys has to say how to give them back, in the
    /// place a reader already looks for what is going on.
    pub fn sidebar_keys() -> String {
        say!("hint.sidebar.keys")
    }

    /// How many rows `J`/`K` move in a list — a screenful of a sidebar, near
    /// enough. The sidebar does not know how tall it is drawn (the front end
    /// does), and a list moves by a *fixed* amount for the same reason `J`
    /// moves by half a page in the text: the eye keeps its place.
    const PAGE_IN_A_LIST: usize = 12;

    /// Run one key while the sidebar has the keys.
    ///
    /// The same letters that move in the text move here — `j`/`k` down and up,
    /// `l` into, `h` out of — so there is nothing new to learn; only what they
    /// move through is different.
    pub(super) fn on_sidebar_key(&mut self, key: Key) {
        // The 大綱's fold keys are answered before the borrow below, because
        // they need the whole editor: only it knows how deep each heading sits
        // (#37). `outline_row_key` is `Some` only in that view, with a row
        // under the highlight.
        if self.outline_row_key().is_some() {
            match key {
                // Out of a branch, as `h` is in the tree.
                Key::Char('h') | Key::Left => return self.fold_outline(),
                // Into one. On a folded heading `l` opens it rather than
                // jumping — 「more of this」 is what it already means in the
                // tree. It falls through on any other row, and `Enter` never
                // folds at all: that is the key that goes, and a heading is a
                // place whether or not it is holding others.
                Key::Char('l') | Key::Right if self.unfold_outline() => return,
                _ => {}
            }
        }
        let Some((side, layer)) = self.panel_focus() else {
            self.panel_focus = None;
            return;
        };
        if layer == crate::sidebar::Layer::Bottom {
            return self.on_transient_key(key, side);
        }
        if self.panel(side).map(|p| p.view()) == Some(crate::sidebar::View::Search) {
            return self.on_search_panel_key(key, side);
        }
        let Some(sidebar) = self.panel_mut(side) else {
            return;
        };
        match key {
            Key::Char('j') | Key::Down => sidebar.step(true),
            Key::Char('k') | Key::Up => sidebar.step(false),
            // **A list pages by the same keys the page does.** `J`/`K` are
            // half a page in the text; a 700-chapter outline is the one list
            // where walking it by `j` is not walking, and `PageDown` is not on
            // every keyboard a novelist owns.
            Key::Char('J') | Key::PageDown => {
                for _ in 0..Self::PAGE_IN_A_LIST {
                    sidebar.step(true);
                }
            }
            Key::Char('K') | Key::PageUp => {
                for _ in 0..Self::PAGE_IN_A_LIST {
                    sidebar.step(false);
                }
            }
            // …and the ends, spelled as they are in the text.
            Key::Char('g') | Key::Home => sidebar.go_to_end(false),
            Key::Char('G') | Key::End => sidebar.go_to_end(true),
            // The views are built when they are opened, not on every key, so
            // `R` is how a writer who has just added a file or a chapter says
            // to look again.
            Key::Char('R') => self.refresh_sidebar(),
            // A chapter's whole name does not fit in a column narrow enough to
            // be worth keeping open, so `w` trades the columns for the name
            // and back.
            Key::Char('w') => {
                let wide = sidebar.toggle_width();
                self.status = if wide {
                    say!("sidebar.wide")
                } else {
                    say!("sidebar.narrow")
                };
            }
            Key::Char('h') | Key::Left => sidebar.collapse(),
            Key::Char('l') | Key::Right | Key::Enter => {
                let chosen = sidebar.activate();
                match chosen {
                    Some(crate::sidebar::Chosen::File(path)) => {
                        if let Err(err) = self.open_file(&path) {
                            self.status = say!("buffer.cannot-open", path.display(), err);
                        }
                        // Entering a file means going to write in it.
                        self.panel_focus = None;
                        self.refresh_sidebar();
                    }
                    Some(crate::sidebar::Chosen::Buffer(i)) => {
                        self.show_buffer(i);
                        self.panel_focus = None;
                        self.refresh_sidebar();
                    }
                    Some(crate::sidebar::Chosen::Line(line)) => {
                        self.goto_line(line + 1);
                        self.panel_focus = None;
                    }
                    Some(crate::sidebar::Chosen::FileLine(path, line)) => {
                        match self.open_included_file(&path) {
                            Ok(()) => self.goto_line(line + 1),
                            Err(err) => {
                                self.status = say!("buffer.cannot-open", path.display(), err)
                            }
                        }
                        self.panel_focus = None;
                        self.refresh_sidebar();
                    }
                    None => {}
                }
            }
            // **Tab walks the views that live in *this* slot.** Which ones
            // those are is a setting, so the question belongs to the editor
            // rather than to the panel — with the outline moved across, this
            // slot walks two and the other one walks one (#293).
            Key::Tab => return self.cycle_view(side, false),
            Key::BackTab => return self.cycle_view(side, true),
            // **`Esc` does nothing here** (#293). It is everyone's 「get me
            // out」 key, so it is tempting — but a panel with a field in it
            // spends `Esc` on leaving Insert, and one press too many would
            // then put the panel away. Two doors instead, and both say so in
            // the hint row: `q` closes this slot, `C-w` walks on to the next
            // region and leaves it up. Both live in `panel_key_in_common`,
            // with `:` and `Space`, because every panel owes the reader the
            // same ones.
            other => {
                self.panel_key_in_common(other, side, layer);
            }
        }
    }

    /// **How many fields the bottom layer holds** — what its scrolling is
    /// clamped to (#293).
    ///
    /// ⚠️ **Fields, not drawn lines.** A value too long for the column wraps,
    /// and only the front end knows how wide the column is — so scrolling by
    /// line would have to be clamped by a number the editor cannot work out.
    /// A field is also the better step: it is the thing a reader is looking
    /// for, and one press moves to the next one whether it took one row or
    /// four.
    pub fn transient_len(&self, side: crate::sidebar::Side) -> usize {
        match self.transient(side) {
            Some(crate::sidebar::Transient::Detail) => {
                self.detail().map(|d| d.rows.len()).unwrap_or(0)
            }
            Some(crate::sidebar::Transient::Dictionary) => self.dictionary_rows().len(),
            None => 0,
        }
    }

    /// Open a picker over the files of the project (`Space f`).
    pub(super) fn open_file_picker(&mut self) {
        let root = self.project_root();
        let mut items = Vec::new();
        walk(&root, &mut 0, &mut |path| {
            if items.len() < PICKER_LIMIT {
                let shown = path.strip_prefix(&root).unwrap_or(path);
                items.push(crate::picker::Item::File(shown.display().to_string()));
            }
        });
        if items.is_empty() {
            self.status = say!("picker.no-files-here");
            return;
        }
        self.listing_root = Some(root);
        self.picker = Some(crate::picker::Picker::new(&say!("picker.files"), items));
        self.mode = Mode::Picker;
    }

    /// Open a picker over the buffers already open (`Space b`).
    pub(super) fn open_buffer_picker(&mut self) {
        let items = self
            .buffers
            .iter()
            .enumerate()
            .map(|(i, b)| crate::picker::Item::Buffer(i, b.display_name()))
            .collect();
        self.picker = Some(crate::picker::Picker::new(&say!("picker.buffers"), items));
        self.mode = Mode::Picker;
    }

    /// Whether `Space` is waiting for the key that says what to do — which is
    /// when the which-key menu is drawn.
    pub fn space_pending(&self) -> bool {
        matches!(self.pending, Pending::Space)
    }

    /// The open picker, for the front end to draw.
    pub fn picker(&self) -> Option<&crate::picker::Picker> {
        self.picker.as_ref()
    }

    /// Run one key while a picker is open.
    pub(super) fn on_picker_key(&mut self, key: Key) {
        let Some(picker) = self.picker.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        match key {
            Key::Esc => self.close_picker(),
            // Backspace past the start of the query closes it, the way it
            // leaves the `:` line: the query is the only thing to go back over.
            Key::Backspace => {
                if !picker.backspace() {
                    self.close_picker();
                }
            }
            Key::Down | Key::Tab | Key::Ctrl('n') => picker.step(true),
            Key::Up | Key::BackTab | Key::Ctrl('p') => picker.step(false),
            Key::PageDown => {
                for _ in 0..10 {
                    picker.step(true);
                }
            }
            Key::PageUp => {
                for _ in 0..10 {
                    picker.step(false);
                }
            }
            Key::Enter => {
                let chosen = picker.chosen();
                self.close_picker();
                match chosen {
                    Some(crate::picker::Item::File(path)) => {
                        let full = match &self.listing_root {
                            Some(root) => root.join(&path),
                            None => PathBuf::from(&path),
                        };
                        if let Err(err) = self.open_file(&full) {
                            self.status = say!("buffer.cannot-open", path, err);
                        }
                    }
                    Some(crate::picker::Item::Buffer(i, _)) => self.show_buffer(i),
                    // The picker belongs to whichever key opened it, so
                    // choosing from it lands the way that key lands.
                    Some(crate::picker::Item::Row(line, _)) => {
                        let preview = self.definition_preview;
                        self.land_on_row(line, preview)
                    }
                    Some(crate::picker::Item::Paste(Some(which), _)) => {
                        self.paste_from_menu(which)
                    }
                    // The system clipboard is the front end's to read.
                    Some(crate::picker::Item::Paste(None, _)) => self.clipboard_paste(true),
                    None => self.status = say!("picker.nothing-matched"),
                }
            }
            Key::Char(c) => picker.push(c),
            // A query is typed text, and typed text is edited in the middle.
            Key::Delete => picker.delete(),
            Key::Left => picker.move_caret(crate::picker::Caret::Left),
            Key::Right => picker.move_caret(crate::picker::Caret::Right),
            Key::Home | Key::Ctrl('a') => picker.move_caret(crate::picker::Caret::Start),
            Key::End | Key::Ctrl('e') => picker.move_caret(crate::picker::Caret::End),
            Key::Ctrl('u') => picker.clear_before_caret(),
            _ => {}
        }
    }

    /// Shut the picker and go back to Normal.
    fn close_picker(&mut self) {
        self.picker = None;
        self.mode = Mode::Normal;
    }

    /// Insert text that arrived from outside — the system clipboard, by way of
    /// the terminal's bracketed paste (Feature #108).
    ///
    /// It is *writing*, whatever mode the editor is in. Without this a paste is
    /// a stream of keystrokes, and in Normal mode every character of the pasted
    /// paragraph runs as a command: that is not a paste going wrong so much as
    /// the editor running a macro nobody wrote.
    pub fn paste_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        // **A spreadsheet's clipboard becomes rows** (Feature #226). Excel,
        // Numbers, LibreOffice and a browser table all put tab-separated lines
        // on the clipboard, and until now the cell refused every one of them
        // for holding a tab — the writer got 「格子裏不能有 Tab」 for the one
        // paste a table editor exists to accept. `t p` had already learned to
        // read a block out of the register; this is the same block arriving by
        // the other door, and it lands the same way.
        if (self.mode == Mode::Insert || self.mode == Mode::Normal) && self.table_here() {
            if let Some(grid) = sniff_grid(text.trim_end_matches(['\n', '\r'])) {
                self.paste_grid(grid);
                return;
            }
        }
        // **Judged before anything happens**, in Insert as well as in Normal:
        // the Insert branch used to hand the text to `insert_str`, which
        // silently drops what a cell refuses, and then say 「貼了 8 個字」 about
        // a paste that had not happened. It also spent an undo point on it.
        if self.mode == Mode::Insert || self.mode == Mode::Normal {
            if let Some(why) = self.cell_refuses_text(text) {
                self.status = why;
                return;
            }
        }
        self.snapshot();
        match self.mode {
            // In Insert it lands where the caret is, like anything typed.
            Mode::Insert => self.insert_str(text),
            // In Normal it replaces the selection, which is what `p` over a
            // selection does — and what a writer means by pasting over
            // something they have just picked out.
            Mode::Normal => {
                // **Both halves judged before either runs**, the way `r` and
                // `R` already do it: pasting a comma into a cell used to
                // delete what was selected and *then* refuse the paste, so the
                // cell came back short and the message only talked about the
                // refusal.
                self.delete_selection();
                let at = self.cursor;
                if !self.edit_insert(at, text) {
                    return;
                }
                let rope = self.current_buffer().rope();
                let end = at + text.chars().count();
                let head = motion::prev_grapheme(rope, end).max(at);
                self.anchor = at;
                self.cursor = head;
                self.refresh_goal_column();
            }
            // A prompt takes it as typing, minus the line breaks that would
            // submit it — asked of the mode rather than listed here (#351).
            // The picker is a prompt too, but its query has a store of its
            // own and nothing here reaches it.
            // A panel's field has a store of its own, like the picker's.
            Mode::Field => {
                let text: String = text.chars().filter(|c| !c.is_control()).collect();
                self.type_into_field(&text);
            }
            mode => {
                if mode.types_into_command_line() {
                    for c in text.chars().filter(|c| !c.is_control()) {
                        self.command_line.push(c);
                    }
                    self.completion = None;
                }
            }
        }
        self.status = say!("edit.pasted-characters", text.chars().count());
    }

    /// Put the selection on the system clipboard (`Space y`).
    ///
    /// Through OSC 52, the terminal's own copy escape: it needs no library, and
    /// it is the only way that works over ssh and inside tmux, which is where a
    /// terminal editor is often run from. The terminal may refuse — many do by
    /// default — so this says what it asked for rather than claiming success.
    pub(super) fn copy_to_clipboard(&mut self) {
        let (start, end) = self.selection();
        let text = self.current_buffer().rope().slice(start..end).to_string();
        if text.is_empty() {
            self.status = say!("edit.nothing-selected");
            return;
        }
        // Into the editor's own register too: having copied something, `p` is
        // the next thing a hand reaches for.
        self.store(text.clone());
        let n = text.chars().count();
        self.clipboard_request = Some(text);
        self.status = say!("edit.copied-to-clipboard", n);
    }

    /// Put the cursor at char index `pos`, starting a selection there
    /// (Feature #109).
    pub fn point_at(&mut self, pos: usize) {
        let pos = pos.min(self.current_buffer().char_count());
        self.extend = false;
        self.anchor = pos;
        self.cursor = pos;
        self.refresh_goal_column();
        // **The mouse leaves a guessed block too** (#275). Walking out of one
        // with `j` drops it at the end of `on_key`; clicking out of one never
        // went through `on_key`, so the mode stayed on until the next
        // keystroke — the cursor was in the paragraph and `hjkl` were still
        // walking cells.
        self.forget_a_guessed_table();
        self.find_the_table_here();
    }

    /// Drag the selection's head to char index `pos`, keeping its anchor.
    pub fn drag_to(&mut self, pos: usize) {
        self.cursor = pos.min(self.current_buffer().char_count());
        self.refresh_goal_column();
    }

    /// Take a pending clipboard copy, for the front end to send to the terminal.
    pub fn take_clipboard_request(&mut self) -> Option<String> {
        self.clipboard_request.take()
    }

    /// Ask for the system clipboard, to be pasted after (or before) the
    /// selection once the front end has fetched it.
    pub(super) fn clipboard_paste(&mut self, after: bool) {
        self.clipboard_read = Some(after);
    }

    /// Take a pending clipboard read; `true` means paste after.
    pub fn take_clipboard_read(&mut self) -> Option<bool> {
        self.clipboard_read.take()
    }

    /// Hand over what the system clipboard held, and paste it.
    pub fn provide_clipboard(&mut self, text: &str, after: bool) {
        if text.is_empty() {
            self.status = say!("edit.clipboard-empty");
            return;
        }
        self.snapshot();
        // Whole lines go back as whole lines, and the selection is replaced
        // when there is one — the same rules `p` follows, because this is `p`
        // with the text coming from somewhere else.
        self.store(text.to_string());
        self.paste(after);
    }

    /// Move to the first non-blank character of line `n`, counting from 1 and
    /// clamped to the end of the buffer (`10gg`, `:10`, `:goto 10`).
    pub(super) fn goto_line(&mut self, n: usize) {
        self.remember_jump();
        self.move_to_line(n);
    }

    /// The same, without noting a jump.
    ///
    /// For the callers that have already noted one — a mark, `:table-jump` — where a
    /// second note would be of the place *after* the file switch, and `C-o`
    /// would then take you to the file you had just arrived in.
    pub(super) fn move_to_line(&mut self, n: usize) {
        let rope = self.current_buffer().rope();
        let last = motion::last_line(rope);
        let line = n.saturating_sub(1).min(last);
        let at = rope.line_to_char(line);
        let pos = motion::line_first_non_blank(rope, at);
        self.move_head(pos);
    }
}
