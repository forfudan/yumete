//! **Windows**: how the editing area of the screen is divided (2026-10-30).
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
//! Decided 2026-10-30 (the four questions): the editing areas are windows and
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
    /// left on a 縱書 page, where reading runs right to left.
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
    Split { how: Split, first: Box<Node>, second: Box<Node> },
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
    /// second half. `None` when `at` is no window.
    pub fn split(&mut self, at: WindowId, how: Split) -> Option<WindowId> {
        fn go(node: &mut Node, at: WindowId, how: Split, fresh: WindowId) -> bool {
            match node {
                Node::Window(id) if *id == at => {
                    *node = Node::Split {
                        how,
                        first: Box::new(Node::Window(at)),
                        second: Box::new(Node::Window(fresh)),
                    };
                    true
                }
                Node::Window(_) => false,
                Node::Split { first, second, .. } => go(first, at, how, fresh) || go(second, at, how, fresh),
            }
        }
        let fresh = self.next;
        if !go(&mut self.root, at, how, fresh) {
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

    /// Keep `id` alone. `false` when `id` is no window.
    pub fn only(&mut self, id: WindowId) -> bool {
        if !self.has(id) {
            return false;
        }
        self.root = Node::Window(id);
        true
    }

    /// The windows laid into `area`, with the rules between them. A split
    /// gives each half the same room less the rule; `rtl` puts a **beside**
    /// split's second half on the left, as a 縱書 page reads.
    pub fn place(&self, area: Area, rtl: bool) -> (Vec<Placed>, Vec<Rule>) {
        fn go(node: &Node, area: Area, rtl: bool, out: &mut Vec<Placed>, rules: &mut Vec<Rule>) {
            match node {
                Node::Window(id) => out.push(Placed { id: *id, area }),
                Node::Split { how, first, second } => {
                    let (a, rule, b) = match how {
                        Split::Below => {
                            let half = area.height.saturating_sub(1) / 2;
                            (
                                Area::new(area.x, area.y, area.width, half),
                                Area::new(area.x, area.y + half, area.width, 1.min(area.height)),
                                Area::new(area.x, area.y + half + 1, area.width, area.height.saturating_sub(half + 1)),
                            )
                        }
                        Split::Beside => {
                            let half = area.width.saturating_sub(1) / 2;
                            let left = Area::new(area.x, area.y, half, area.height);
                            let rule = Area::new(area.x + half, area.y, 1.min(area.width), area.height);
                            let right = Area::new(area.x + half + 1, area.y, area.width.saturating_sub(half + 1), area.height);
                            match rtl {
                                true => (right, rule, left),
                                false => (left, rule, right),
                            }
                        }
                    };
                    go(first, a, rtl, out, rules);
                    rules.push(Rule { area: rule, how: *how, first: last_leaf(first), second: first_leaf(second) });
                    go(second, b, rtl, out, rules);
                }
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
        let mut out = Vec::new();
        let mut rules = Vec::new();
        go(&self.root, area, rtl, &mut out, &mut rules);
        (out, rules)
    }

    /// The window in `direction` from `id` on a screen laid out as `area`:
    /// the one whose area the step from `id`'s edge lands in, nearest to the
    /// middle of that edge. `None` at the screen's edge.
    pub fn neighbour(&self, id: WindowId, direction: Direction, area: Area, rtl: bool) -> Option<WindowId> {
        let (placed, _) = self.place(area, rtl);
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

    #[test]
    fn placing_matches_the_old_two_halves_and_reads_right_to_left_when_asked() {
        let mut w = Windows::one();
        w.split(1, Split::Below);
        let (placed, rules) = w.place(Area::new(0, 0, 80, 24), false);
        assert_eq!(placed[0].area, Area::new(0, 0, 80, 11));
        assert_eq!(rules[0].area, Area::new(0, 11, 80, 1));
        assert_eq!(placed[1].area, Area::new(0, 12, 80, 12));
        assert_eq!((rules[0].first, rules[0].second), (1, 2));

        let mut w = Windows::one();
        w.split(1, Split::Beside);
        let (placed, rules) = w.place(Area::new(0, 0, 81, 24), true);
        assert_eq!(placed[0].area, Area::new(41, 0, 40, 24), "縱書: the first half on the right");
        assert_eq!(rules[0].area, Area::new(40, 0, 1, 24));
        assert_eq!(placed[1].area, Area::new(0, 0, 40, 24));
        let (placed, _) = w.place(Area::new(0, 0, 81, 24), false);
        assert_eq!(placed[0].area, Area::new(0, 0, 40, 24));
        assert_eq!(placed[1].area, Area::new(41, 0, 40, 24));
    }

    #[test]
    fn a_neighbour_is_found_by_geometry() {
        let mut w = Windows::one();
        let two = w.split(1, Split::Beside).unwrap();
        let three = w.split(two, Split::Below).unwrap();
        let area = Area::new(0, 0, 81, 24);
        // From 1's middle row the step lands in the lower of the two on the
        // right; the cursor's own row will refine this (stage 2).
        assert_eq!(w.neighbour(1, Direction::Right, area, false), Some(three));
        assert_eq!(w.neighbour(1, Direction::Left, area, false), None);
        assert_eq!(w.neighbour(two, Direction::Down, area, false), Some(three));
        assert_eq!(w.neighbour(three, Direction::Up, area, false), Some(two));
        assert_eq!(w.neighbour(three, Direction::Left, area, false), Some(1));
        assert_eq!(w.neighbour(1, Direction::Left, area, true), Some(three), "縱書 puts the second half on the left");
        assert_eq!(w.neighbour(1, Direction::Right, area, true), None);
    }
}
