//! **Windows**: how the editing area of the screen is divided.
//!
//! The screen has up to four *regions*: the editing windows and the two
//! sidebars. This is the windows' half — a binary tree whose leaves are
//! windows and whose inner nodes say how two halves lie: one **beside** the
//! other, or one **below** the other. The words are positions, not
//! directions, on purpose: on a 縱書 page 「vertical」 already names the way
//! the text runs, and helix's 「vertical split」 cuts the screen the other
//! way round from what the word says.
//!
//! Everything here is geometry and bookkeeping over ids. What a window
//! shows — its buffer, its place in it — is the editor's (`Pane`), and what
//! it scrolls to is the front end's. The tree never looks inside a window.
//!
//! The editing areas are windows and
//! the sidebars stay sidebars, with 「region」 the word for either; any number
//! of windows, limited by the room on screen rather than by a count; helix's
//! keys open them, `C-w v` beside and `C-w s` below; a new window shows the
//! buffer of the one it was split from, and from then on each window shows
//! its own.

/// A window's name for its whole life. Never reused, so a stale id is a
/// miss, not somebody else's window.
pub type WindowId = u64;

/// How two halves of a split lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    /// Side by side: the second half to the right of the first — or to its
    /// left when the split was opened from the right, as one opened from a
    /// 縱書 page is. Which of the two is the split's own, kept from when it
    /// was made: switching the layout moves no window.
    Beside,
    /// One above the other: the second half under the first.
    Below,
}

/// A rectangle of cells, in the front end's terms but without its types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Area {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Area {
    pub fn new(x: u16, y: u16, width: u16, height: u16) -> Area {
        Area { x, y, width, height }
    }

    fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Window(WindowId),
    /// `rtl`: a **beside** split laid out from the right, its first half on
    /// the right. Always `false` below, so a column is one column however
    /// its splits were made.
    Split { how: Split, rtl: bool, first: Box<Node>, second: Box<Node> },
}

/// The tree of windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Windows {
    root: Node,
    next: WindowId,
}

/// One window's place on screen, and the rule drawn after it when it has a
/// sibling: a row under it, or a column beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub id: WindowId,
    pub area: Area,
}

/// A rule between two halves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub area: Area,
    pub how: Split,
    /// The window touching the rule on its near side (the last of the first
    /// half) and on its far side (the first of the second half) — a caption
    /// on the rule names whichever of them is being read, not written.
    pub first: WindowId,
    pub second: WindowId,
}

impl Default for Windows {
    fn default() -> Windows {
        Windows { root: Node::Window(1), next: 2 }
    }
}

impl Windows {
    /// One window, id 1.
    pub fn one() -> Windows {
        Windows::default()
    }

    /// Every window in tree order — first halves before second halves.
    ///
    /// **This is the order `C-w w` walks and the order the front end keeps
    /// its viewports in**, so a split never makes a window change its index
    /// from under its scroll.
    pub fn leaves(&self) -> Vec<WindowId> {
        fn walk(node: &Node, out: &mut Vec<WindowId>) {
            match node {
                Node::Window(id) => out.push(*id),
                Node::Split { first, second, .. } => {
                    walk(first, out);
                    walk(second, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }

    pub fn len(&self) -> usize {
        self.leaves().len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn has(&self, id: WindowId) -> bool {
        self.leaves().contains(&id)
    }

    /// Where `id` stands in [`Self::leaves`].
    pub fn index_of(&self, id: WindowId) -> Option<usize> {
        self.leaves().iter().position(|&w| w == id)
    }

    /// Split window `at` and hand back the new window, which takes the
    /// second half — below, or to the right. `None` when `at` is no window.
    pub fn split(&mut self, at: WindowId, how: Split) -> Option<WindowId> {
        self.split_on(at, how, false, false)
    }

    /// [`Self::split`], the new window going `before` `at` — above it or
    /// to its left, as vim's `:split` and `:vsplit` open one
    /// (`nosplitbelow`, `nosplitright`; measured in nvim). `rtl`
    /// lays a **beside** split out from the right, its first half on the
    /// right, as one opened from a 縱書 page is; the split keeps it.
    pub fn split_on(&mut self, at: WindowId, how: Split, before: bool, rtl: bool) -> Option<WindowId> {
        fn go(node: &mut Node, at: WindowId, how: Split, rtl: bool, fresh: WindowId, before: bool) -> bool {
            match node {
                Node::Window(id) if *id == at => {
                    let (first, second) = match before {
                        true => (fresh, at),
                        false => (at, fresh),
                    };
                    *node = Node::Split {
                        how,
                        rtl,
                        first: Box::new(Node::Window(first)),
                        second: Box::new(Node::Window(second)),
                    };
                    true
                }
                Node::Window(_) => false,
                Node::Split { first, second, .. } => {
                    go(first, at, how, rtl, fresh, before) || go(second, at, how, rtl, fresh, before)
                }
            }
        }
        let fresh = self.next;
        if !go(&mut self.root, at, how, rtl && how == Split::Beside, fresh, before) {
            return None;
        }
        self.next += 1;
        Some(fresh)
    }

    /// Close window `id`; its sibling takes the whole of their split. The
    /// last window cannot be closed: `false`.
    pub fn close(&mut self, id: WindowId) -> bool {
        fn go(node: &mut Node, id: WindowId) -> bool {
            let Node::Split { first, second, .. } = node else { return false };
            if **first == Node::Window(id) {
                *node = (**second).clone();
                return true;
            }
            if **second == Node::Window(id) {
                *node = (**first).clone();
                return true;
            }
            go(first, id) || go(second, id)
        }
        go(&mut self.root, id)
    }

    /// The window that takes `id`'s place when it closes: the next window
    /// of its row or column, or the one before when it is the last — vim's
    /// choice (`nosplitbelow`; measured in nvim). A neighbour
    /// split the other way answers its window touching `id`. `None` for the
    /// last window.
    pub fn heir(&self, id: WindowId) -> Option<WindowId> {
        let row = self.row_nodes(id);
        let at = row.iter().position(|node| **node == Node::Window(id))?;
        match row.get(at + 1) {
            Some(next) => Some(first_leaf(next)),
            None => Some(last_leaf(row.get(at.checked_sub(1)?)?)),
        }
    }

    /// The nodes of the row or column `id` stands in, in order: a run of
    /// splits the same way is one row, as vim's frames and helix's
    /// containers are. Empty when `id` is the only window.
    fn row_nodes(&self, id: WindowId) -> Vec<&Node> {
        fn path<'a>(node: &'a Node, id: WindowId, out: &mut Vec<&'a Node>) -> bool {
            match node {
                Node::Window(w) => *w == id,
                Node::Split { first, second, .. } => {
                    out.push(node);
                    if path(first, id, out) || path(second, id, out) {
                        return true;
                    }
                    out.pop();
                    false
                }
            }
        }
        let mut above = Vec::new();
        if !path(&self.root, id, &mut above) {
            return Vec::new();
        }
        let Some(lie) = above.last().and_then(|node| lie_of(node)) else { return Vec::new() };
        let Some(top) = above.iter().rev().take_while(|node| lie_of(node) == Some(lie)).last() else { return Vec::new() };
        let mut out = Vec::new();
        children(top, lie, &mut out);
        out
    }

    /// The tree written out first-half-first: a split, then its two halves.
    /// What a session keeps; [`Self::split_on`] read along the same order
    /// builds it again.
    pub fn steps(&self) -> Vec<Step> {
        fn walk(node: &Node, out: &mut Vec<Step>) {
            match node {
                Node::Window(id) => out.push(Step::Window(*id)),
                Node::Split { how, rtl, first, second } => {
                    out.push(Step::Split(*how, *rtl));
                    walk(first, out);
                    walk(second, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }

    /// Keep `id` alone. `false` when `id` is no window.
    pub fn only(&mut self, id: WindowId) -> bool {
        if !self.has(id) {
            return false;
        }
        self.root = Node::Window(id);
        true
    }

    /// **helix's `C-w t` (`transpose_view`)**: the split `id` is a half of
    /// turns over the diagonal from the top left, beside to below and below
    /// to beside. **What is on the left goes on top**, and what is on top
    /// to the left, whichever half of the split it is: a beside split laid
    /// out from the right (a session may still hold one) has its halves
    /// trade order as it turns, or its left window would go to the bottom.
    /// `false` when `id` is no half of a split.
    pub fn transpose(&mut self, id: WindowId) -> bool {
        fn go(node: &mut Node, id: WindowId) -> bool {
            let Node::Split { how, rtl, first, second } = node else { return false };
            if matches!(**first, Node::Window(w) if w == id) || matches!(**second, Node::Window(w) if w == id) {
                if *how == Split::Beside && *rtl {
                    std::mem::swap(first, second);
                }
                *how = match how {
                    Split::Beside => Split::Below,
                    Split::Below => Split::Beside,
                };
                *rtl = false;
                return true;
            }
            go(first, id) || go(second, id)
        }
        go(&mut self.root, id)
    }

    /// **helix's `C-w H` `J` `K` `L` (`swap_view_*`)**: two windows trade
    /// places, each keeping its id, so the one that was live still is.
    pub fn swap(&mut self, a: WindowId, b: WindowId) -> bool {
        fn go(node: &mut Node, a: WindowId, b: WindowId) {
            match node {
                Node::Window(w) if *w == a => *w = b,
                Node::Window(w) if *w == b => *w = a,
                Node::Window(_) => {}
                Node::Split { first, second, .. } => {
                    go(first, a, b);
                    go(second, a, b);
                }
            }
        }
        if a == b || !self.has(a) || !self.has(b) {
            return false;
        }
        go(&mut self.root, a, b);
        true
    }

    /// **The row or column `id` stands in**, as vim's `C-w x` `r` `R` read
    /// it: the windows and splits lined up the same way as `id` and its
    /// sibling, in order; `None` marks one that is itself split the other
    /// way. Empty when `id` is the only window.
    pub fn row_of(&self, id: WindowId) -> Vec<Option<WindowId>> {
        self.row_nodes(id)
            .into_iter()
            .map(|node| match node {
                Node::Window(w) => Some(*w),
                Node::Split { .. } => None,
            })
            .collect()
    }

    /// **vim's `C-w x`**: `id` trades places with the next window of its
    /// row (the one before, when it is the last), or with the `nth` (from 1).
    /// Answers the other window; `None` when that is a split or not there.
    pub fn exchange(&mut self, id: WindowId, nth: Option<usize>) -> Option<WindowId> {
        let row = self.row_of(id);
        let at = row.iter().position(|w| *w == Some(id))?;
        let other = match nth {
            Some(n) => *row.get(n.checked_sub(1)?)?,
            None => *row.get(at + 1).or_else(|| at.checked_sub(1).and_then(|i| row.get(i)))?,
        }?;
        self.swap(id, other).then_some(other)
    }

    /// **vim's `C-w r` (`forward`) and `C-w R`**: the windows of `id`'s row
    /// each move one place on, the last coming round to the first. `false`
    /// when one of them is itself split (vim's E443) or there is no row.
    pub fn rotate(&mut self, id: WindowId, forward: bool) -> bool {
        let row = self.row_of(id);
        let Some(row) = row.into_iter().collect::<Option<Vec<_>>>() else { return false };
        if row.len() < 2 {
            return false;
        }
        let mut moved = row.clone();
        match forward {
            true => moved.rotate_right(1),
            false => moved.rotate_left(1),
        }
        fn go(node: &mut Node, from: &[WindowId], to: &[WindowId]) {
            match node {
                Node::Window(w) => {
                    if let Some(i) = from.iter().position(|f| f == w) {
                        *w = to[i];
                    }
                }
                Node::Split { first, second, .. } => {
                    go(first, from, to);
                    go(second, from, to);
                }
            }
        }
        go(&mut self.root, &row, &moved);
        true
    }

    /// **vim's `C-w H` `J` `K` `L`**: `id` leaves its place and takes the
    /// whole of that edge of the screen, the rest keeping the other half.
    /// `rtl` makes the new beside split one laid out from the right, as on
    /// a 縱書 page, where the second half is the left one; the window lands
    /// on the edge asked for either way. `false` when `id` is the only
    /// window.
    pub fn to_edge(&mut self, id: WindowId, edge: Direction, rtl: bool) -> bool {
        if self.len() < 2 || !self.close(id) {
            return false;
        }
        let rest = Box::new(self.root.clone());
        let one = Box::new(Node::Window(id));
        let beside = |first, second| Node::Split { how: Split::Beside, rtl, first, second };
        let below = |first, second| Node::Split { how: Split::Below, rtl: false, first, second };
        self.root = match (edge, rtl) {
            (Direction::Left, false) | (Direction::Right, true) => beside(one, rest),
            (Direction::Right, false) | (Direction::Left, true) => beside(rest, one),
            (Direction::Up, _) => below(one, rest),
            (Direction::Down, _) => below(rest, one),
        };
        true
    }

    /// The windows laid into `area`, with the rules between them. The
    /// windows of a row or column share its room equally less the rules,
    /// as vim's (`equalalways`) and helix's do; what does not divide goes
    /// to the last. A **beside** row opened from the right is laid out
    /// from the right, as a 縱書 page reads — each split by its own order,
    /// never by the page the keys are on now.
    pub fn place(&self, area: Area) -> (Vec<Placed>, Vec<Rule>) {
        fn go(node: &Node, area: Area, out: &mut Vec<Placed>, rules: &mut Vec<Rule>) {
            let Some((how, rtl)) = lie_of(node) else {
                if let Node::Window(id) = node {
                    out.push(Placed { id: *id, area });
                }
                return;
            };
            let mut row = Vec::new();
            children(node, (how, rtl), &mut row);
            let n = row.len() as u16;
            let length = match how {
                Split::Below => area.height,
                Split::Beside => area.width,
            };
            let share = length.saturating_sub(n - 1) / n;
            let mut from = 0u16;
            for (i, child) in row.iter().enumerate() {
                let last = i + 1 == row.len();
                let size = match last {
                    true => length.saturating_sub(from),
                    false => share,
                };
                let (part, rule) = match how {
                    Split::Below => (
                        Area::new(area.x, area.y + from, area.width, size),
                        Area::new(area.x, area.y + from + size, area.width, 1),
                    ),
                    Split::Beside => {
                        let x = match rtl {
                            true => area.x + length.saturating_sub(from + size),
                            false => area.x + from,
                        };
                        let rule_x = match rtl {
                            true => x.saturating_sub(1),
                            false => x + size,
                        };
                        (Area::new(x, area.y, size, area.height), Area::new(rule_x, area.y, 1, area.height))
                    }
                };
                go(child, part, out, rules);
                if !last {
                    rules.push(Rule { area: rule, how, first: last_leaf(child), second: first_leaf(row[i + 1]) });
                }
                from = (from + size + 1).min(length);
            }
        }
        let mut out = Vec::new();
        let mut rules = Vec::new();
        go(&self.root, area, &mut out, &mut rules);
        (out, rules)
    }

    /// The window in `direction` from `id` on a screen laid out as `area`:
    /// the one whose area the step from `id`'s edge lands in, nearest to the
    /// middle of that edge. `None` at the screen's edge.
    pub fn neighbour(&self, id: WindowId, direction: Direction, area: Area) -> Option<WindowId> {
        let (placed, _) = self.place(area);
        let me = placed.iter().find(|p| p.id == id)?.area;
        let (x, y) = match direction {
            Direction::Left => (me.x.checked_sub(2)?, me.y + me.height / 2),
            Direction::Right => (me.x + me.width + 1, me.y + me.height / 2),
            Direction::Up => (me.x + me.width / 2, me.y.checked_sub(2)?),
            Direction::Down => (me.x + me.width / 2, me.y + me.height + 1),
        };
        placed.iter().find(|p| p.id != id && p.area.contains(x, y)).map(|p| p.id)
    }
}

/// How a split lies, and whether from the right: a run of splits that lie
/// alike is one row.
fn lie_of(node: &Node) -> Option<(Split, bool)> {
    match node {
        Node::Split { how, rtl, .. } => Some((*how, *rtl)),
        Node::Window(_) => None,
    }
}

/// The halves under `node` that lie `lie`, a run of such splits read as one
/// row.
fn children<'a>(node: &'a Node, lie: (Split, bool), out: &mut Vec<&'a Node>) {
    match node {
        Node::Split { first, second, .. } if lie_of(node) == Some(lie) => {
            children(first, lie, out);
            children(second, lie, out);
        }
        _ => out.push(node),
    }
}

fn first_leaf(node: &Node) -> WindowId {
    match node {
        Node::Window(id) => *id,
        Node::Split { first, .. } => first_leaf(first),
    }
}

fn last_leaf(node: &Node) -> WindowId {
    match node {
        Node::Window(id) => *id,
        Node::Split { second, .. } => last_leaf(second),
    }
}

/// One node of the tree in [`Windows::steps`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// A split, and whether it is laid out from the right.
    Split(Split, bool),
    Window(WindowId),
}

/// A step from one window to the next on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// vim's `C-w x` `r` `R` `H` `J` `K` `L`, each as nvim did it
    /// (`winlayout()` before and after) on three windows
    /// stacked, and on one of them split beside.
    #[test]
    fn a_row_exchanges_rotates_and_a_window_goes_to_an_edge() {
        let three = || {
            let mut w = Windows::one();
            w.split(1, Split::Below);
            w.split(2, Split::Below);
            w
        };
        assert_eq!(three().row_of(2), [Some(1), Some(2), Some(3)]);
        let mut w = three();
        assert_eq!(w.exchange(1, None), Some(2));
        assert_eq!(w.leaves(), [2, 1, 3]);
        let mut w = three();
        assert_eq!(w.exchange(3, None), Some(2), "the last trades with the one before");
        assert_eq!(w.leaves(), [1, 3, 2]);
        let mut w = three();
        assert_eq!(w.exchange(1, Some(3)), Some(3));
        assert_eq!(w.leaves(), [3, 2, 1]);
        let mut w = three();
        assert!(w.rotate(1, true));
        assert_eq!(w.leaves(), [3, 1, 2]);
        let mut w = three();
        assert!(w.rotate(1, false));
        assert_eq!(w.leaves(), [2, 3, 1]);
        // The middle one split beside: it is no window of the column.
        let mut w = three();
        w.split(2, Split::Beside);
        assert_eq!(w.row_of(1), [Some(1), None, Some(3)]);
        assert!(!w.rotate(1, true), "E443");
        assert_eq!(w.exchange(1, None), None);
        assert_eq!(w.row_of(4), [Some(2), Some(4)]);
        assert_eq!(w.exchange(4, None), Some(2));
        assert_eq!(w.leaves(), [1, 4, 2, 3]);
        // `C-w L`: the whole right edge; `C-w K` the top.
        let mut w = three();
        assert!(w.to_edge(1, Direction::Right, false));
        assert_eq!(
            w.steps(),
            [Step::Split(Split::Beside, false), Step::Split(Split::Below, false), Step::Window(2), Step::Window(3), Step::Window(1)]
        );
        let mut w = three();
        assert!(w.to_edge(2, Direction::Up, false));
        assert_eq!(w.leaves(), [2, 1, 3]);
        assert!(!Windows::one().to_edge(1, Direction::Up, false));
        assert!(Windows::one().row_of(1).is_empty());
    }

    #[test]
    fn a_split_adds_a_second_half_and_a_close_gives_it_back() {
        let mut w = Windows::one();
        assert_eq!(w.leaves(), [1]);
        let two = w.split(1, Split::Below).unwrap();
        assert_eq!(two, 2);
        assert_eq!(w.leaves(), [1, 2]);
        let three = w.split(2, Split::Beside).unwrap();
        assert_eq!(w.leaves(), [1, 2, 3]);
        assert!(w.close(two));
        assert_eq!(w.leaves(), [1, 3], "the sibling takes the split");
        assert!(w.close(three));
        assert!(!w.close(1), "the last window stays");
        assert_eq!(w.split(9, Split::Below), None);
        let four = w.split(1, Split::Below).unwrap();
        assert_eq!(four, 4, "ids are never reused");
        assert!(w.only(1));
        assert_eq!(w.leaves(), [1]);
    }

    /// **Turned over the diagonal**: the left window goes on top and comes
    /// back to the left, for a beside split laid out from the right too.
    #[test]
    fn a_split_turns_with_its_left_window_on_top() {
        let area = Area::new(0, 0, 81, 25);
        let mut w = Windows::one();
        w.split_on(1, Split::Beside, false, true);
        // Laid out from the right: 2 on the left, 1 on the right.
        assert_eq!(w.neighbour(2, Direction::Right, area), Some(1));
        for _ in 0..3 {
            assert!(w.transpose(1));
            assert_eq!(w.neighbour(2, Direction::Down, area), Some(1), "the left one on top");
            assert!(w.transpose(1));
            assert_eq!(w.neighbour(2, Direction::Right, area), Some(1), "and back on the left");
        }
    }

    #[test]
    fn a_split_turns_and_two_windows_trade_places() {
        let mut w = Windows::one();
        w.split(1, Split::Beside);
        w.split(2, Split::Below);
        let area = Area::new(0, 0, 81, 25);
        // 1 | 2 over 3. Turning 3's split puts 2 beside 3.
        assert!(w.transpose(3));
        assert_eq!(w.neighbour(2, Direction::Right, area), Some(3));
        assert!(!Windows::one().transpose(1), "one window has no split to turn");
        // Turned back, 3 is below 2 again.
        assert!(w.transpose(3));
        assert_eq!(w.neighbour(2, Direction::Down, area), Some(3));
        assert!(w.transpose(3));
        // 1 and 3 trade places: 3 is on the left now, and the ids stay.
        assert!(w.swap(1, 3));
        assert_eq!(w.leaves(), [3, 2, 1]);
        assert!(!w.swap(1, 1));
        assert!(!w.swap(1, 9));
    }

    #[test]
    fn placing_matches_the_old_two_halves_and_reads_right_to_left_when_opened_so() {
        let mut w = Windows::one();
        w.split(1, Split::Below);
        let (placed, rules) = w.place(Area::new(0, 0, 80, 24));
        assert_eq!(placed[0].area, Area::new(0, 0, 80, 11));
        assert_eq!(rules[0].area, Area::new(0, 11, 80, 1));
        assert_eq!(placed[1].area, Area::new(0, 12, 80, 12));
        assert_eq!((rules[0].first, rules[0].second), (1, 2));

        let mut w = Windows::one();
        w.split_on(1, Split::Beside, false, true);
        let (placed, rules) = w.place(Area::new(0, 0, 81, 24));
        assert_eq!(placed[0].area, Area::new(41, 0, 40, 24), "縱書: the first half on the right");
        assert_eq!(rules[0].area, Area::new(40, 0, 1, 24));
        assert_eq!(placed[1].area, Area::new(0, 0, 40, 24));
        assert_eq!(w.steps()[0], Step::Split(Split::Beside, true), "the split keeps its order");
        let mut w = Windows::one();
        w.split(1, Split::Beside);
        let (placed, _) = w.place(Area::new(0, 0, 81, 24));
        assert_eq!(placed[0].area, Area::new(0, 0, 40, 24));
        assert_eq!(placed[1].area, Area::new(41, 0, 40, 24));
        // Below never reads from the right: a column is one column.
        let mut w = Windows::one();
        let two = w.split_on(1, Split::Below, false, true).unwrap();
        w.split(two, Split::Below);
        assert_eq!(w.steps()[0], Step::Split(Split::Below, false));
        assert_eq!(w.row_of(1).len(), 3);
    }

    /// Splits laid out each its own way share a screen: a
    /// preview opened to the right of a window whose row was opened from the
    /// right stays to its right, and the row stays as it was.
    #[test]
    fn each_split_keeps_its_own_order() {
        // 2 opened from the right of 1, then 3 to the right of 2.
        let mut w = Windows::one();
        let two = w.split_on(1, Split::Beside, false, true).unwrap();
        let three = w.split_on(two, Split::Beside, false, false).unwrap();
        let area = Area::new(0, 0, 92, 10);
        let (placed, _) = w.place(area);
        let xs: Vec<_> = placed.iter().map(|p| (p.id, p.area.x)).collect();
        assert_eq!(xs, [(1, 47), (two, 0), (three, 23)]);
        assert_eq!(w.neighbour(two, Direction::Right, area), Some(three));
        assert_eq!(w.neighbour(three, Direction::Right, area), Some(1));
        assert_eq!(w.neighbour(1, Direction::Left, area), Some(three));
        assert_eq!(w.neighbour(two, Direction::Left, area), None);
        // Two rows, not one: 3 is no window of 1's row.
        assert_eq!(w.row_of(1), [Some(1), None]);
        assert_eq!(w.row_of(three), [Some(two), Some(three)]);
    }

    /// A row shares its room equally, however its splits were made, as
    /// nvim's and helix's do: three stacked in 40 rows less
    /// two rules, three beside after `C-w L` from a column.
    #[test]
    fn a_row_shares_its_room_equally() {
        let mut w = Windows::one();
        let two = w.split(1, Split::Below).unwrap();
        w.split(two, Split::Below);
        let (placed, rules) = w.place(Area::new(0, 0, 80, 40));
        let heights: Vec<_> = placed.iter().map(|p| p.area.height).collect();
        assert_eq!(heights, [12, 12, 14]);
        assert_eq!(rules.len(), 2);
        assert_eq!(placed[1].area.y, 13);
        assert_eq!(rules[1].area.y, 25);
        // vim's new window goes first: the tree is (3 / 2) / 1.
        let mut w = Windows::one();
        let two = w.split_on(1, Split::Below, true, false).unwrap();
        let three = w.split_on(two, Split::Below, true, false).unwrap();
        assert_eq!(w.leaves(), [three, two, 1]);
        assert_eq!(w.row_of(1), [Some(three), Some(two), Some(1)]);
        assert_eq!(w.heir(two), Some(1), "the middle's goes below");
        assert_eq!(w.heir(1), Some(two), "the last's goes up");
        // Three beside, then the first to the right edge: thirds still.
        let mut w = Windows::one();
        let two = w.split(1, Split::Beside).unwrap();
        w.split(two, Split::Beside);
        assert!(w.to_edge(1, Direction::Right, false));
        let (placed, _) = w.place(Area::new(0, 0, 92, 10));
        let widths: Vec<_> = placed.iter().map(|p| (p.id, p.area.x, p.area.width)).collect();
        assert_eq!(widths, [(2, 0, 30), (3, 31, 30), (1, 62, 30)]);
        // 縱書 lays the row out from the right, `C-w L` included.
        let mut w = Windows::one();
        let two = w.split_on(1, Split::Beside, false, true).unwrap();
        w.split_on(two, Split::Beside, false, true);
        assert!(w.to_edge(1, Direction::Left, true));
        let (placed, rules) = w.place(Area::new(0, 0, 92, 10));
        let widths: Vec<_> = placed.iter().map(|p| (p.id, p.area.x, p.area.width)).collect();
        assert_eq!(widths, [(2, 62, 30), (3, 31, 30), (1, 0, 30)]);
        assert_eq!((rules[0].area.x, rules[1].area.x), (61, 30));
    }

    #[test]
    fn the_heir_is_the_window_that_takes_the_room() {
        // Three stacked, as measured in vim: the middle's room and
        // the keys go below, the top's to the middle, the bottom's up.
        let mut w = Windows::one();
        let two = w.split(1, Split::Below).unwrap();
        let three = w.split(two, Split::Below).unwrap();
        assert_eq!(w.heir(two), Some(three));
        assert_eq!(w.heir(1), Some(two));
        assert_eq!(w.heir(three), Some(two));
        // 1 | (2 / 3): closing 1 goes to the top of the column beside it.
        let mut w = Windows::one();
        let two = w.split(1, Split::Beside).unwrap();
        let three = w.split(two, Split::Below).unwrap();
        assert_eq!(w.heir(1), Some(two));
        assert_eq!(w.heir(two), Some(three));
        assert_eq!(Windows::one().heir(1), None);
    }

    #[test]
    fn a_neighbour_is_found_by_geometry() {
        let mut w = Windows::one();
        let two = w.split(1, Split::Beside).unwrap();
        let three = w.split(two, Split::Below).unwrap();
        let area = Area::new(0, 0, 81, 24);
        // From 1's middle row the step lands in the lower of the two on the
        // right; the cursor's own row will refine this (stage 2).
        assert_eq!(w.neighbour(1, Direction::Right, area), Some(three));
        assert_eq!(w.neighbour(1, Direction::Left, area), None);
        assert_eq!(w.neighbour(two, Direction::Down, area), Some(three));
        assert_eq!(w.neighbour(three, Direction::Up, area), Some(two));
        assert_eq!(w.neighbour(three, Direction::Left, area), Some(1));
        let mut w = Windows::one();
        let two = w.split_on(1, Split::Beside, false, true).unwrap();
        let three = w.split(two, Split::Below).unwrap();
        assert_eq!(w.neighbour(1, Direction::Left, area), Some(three), "縱書 puts the second half on the left");
        assert_eq!(w.neighbour(1, Direction::Right, area), None);
    }
}
