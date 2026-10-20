//! Pane trees: panes laid out by splitting any pane side by side or one above the other, as often
//! as you like, instead of the fixed grid. Every divider can be dragged. The tree lives in the
//! desktop client and is keyed by pane id; it follows the daemon's pane list (a closed pane
//! disappears and its neighbour takes the space, a pane someone else opens is split off the focused one).
use super::panes::{drag, spans, Dir, GAP};
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Node {
    Leaf(u64),
    Split {
        /// Children sit side by side (a vertical line between them) rather than stacked.
        row: bool,
        kids: Vec<Node>,
        /// Share of the space for each child; sums to 1.
        w: Vec<f32>,
    },
}

/// A divider between two children of a split, found by walking `path` from the root.
pub(super) struct TreeDivider {
    pub rect: Rect,
    pub between_columns: bool,
    pub path: Vec<usize>,
    pub index: usize,
    pub axis_len: f32,
}

#[derive(Clone, Debug, Default)]
pub(super) struct PaneTree {
    root: Option<Node>,
    /// A pane that was asked for by a split: when it appears it goes next to this pane, in this direction.
    pub pending: Option<(u64, bool)>,
}

impl Node {
    fn collect(&self, out: &mut Vec<u64>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { kids, .. } => kids.iter().for_each(|k| k.collect(out)),
        }
    }
    fn contains(&self, id: u64) -> bool {
        match self {
            Node::Leaf(i) => *i == id,
            Node::Split { kids, .. } => kids.iter().any(|k| k.contains(id)),
        }
    }
}

/// Put `new` next to `target`: a split of the target's own space, or one more sibling when the
/// target's parent already splits in that direction (so repeated splits do not nest needlessly).
fn split_at(node: &mut Node, target: u64, new: u64, row: bool) -> bool {
    match node {
        Node::Leaf(id) if *id == target => {
            *node = Node::Split {
                row,
                kids: vec![Node::Leaf(target), Node::Leaf(new)],
                w: vec![0.5, 0.5],
            };
            true
        }
        Node::Leaf(_) => false,
        Node::Split {
            row: r, kids, w, ..
        } => {
            if *r == row {
                if let Some(i) = kids
                    .iter()
                    .position(|k| matches!(k, Node::Leaf(id) if *id == target))
                {
                    let half = w[i] / 2.;
                    w[i] = half;
                    w.insert(i + 1, half);
                    kids.insert(i + 1, Node::Leaf(new));
                    return true;
                }
            }
            kids.iter_mut().any(|k| split_at(k, target, new, row))
        }
    }
}

/// Take `id` out; its share goes to the others, and a split left with one child becomes that child.
/// Returns whether `node` itself is now empty.
fn remove_from(node: &mut Node, id: u64) -> bool {
    match node {
        Node::Leaf(i) => *i == id,
        Node::Split { kids, w, .. } => {
            let mut i = 0;
            while i < kids.len() {
                if remove_from(&mut kids[i], id) {
                    kids.remove(i);
                    w.remove(i);
                } else {
                    i += 1;
                }
            }
            let sum: f32 = w.iter().sum();
            if sum > 0. {
                w.iter_mut().for_each(|x| *x /= sum);
            }
            match kids.len() {
                0 => true,
                1 => {
                    let only = kids.remove(0);
                    *node = only;
                    false
                }
                _ => false,
            }
        }
    }
}

fn node_at<'a>(node: &'a mut Node, path: &[usize]) -> Option<&'a mut Node> {
    match path.split_first() {
        None => Some(node),
        Some((i, rest)) => match node {
            Node::Split { kids, .. } => node_at(kids.get_mut(*i)?, rest),
            Node::Leaf(_) => None,
        },
    }
}

fn arrange_node(
    node: &Node,
    area: Rect,
    path: &mut Vec<usize>,
    rects: &mut Vec<(u64, Rect)>,
    dividers: &mut Vec<TreeDivider>,
) {
    match node {
        Node::Leaf(id) => rects.push((*id, area)),
        Node::Split { row, kids, w } => {
            let axis = if *row { area.width() } else { area.height() };
            let along = spans(axis, GAP, w);
            for (i, (kid, &(start, len))) in kids.iter().zip(&along).enumerate() {
                let rect = if *row {
                    Rect::from_min_size(
                        area.min + Vec2::new(start, 0.),
                        Vec2::new(len, area.height()),
                    )
                } else {
                    Rect::from_min_size(
                        area.min + Vec2::new(0., start),
                        Vec2::new(area.width(), len),
                    )
                };
                path.push(i);
                arrange_node(kid, rect, path, rects, dividers);
                path.pop();
                if i + 1 < kids.len() {
                    let gap = if *row {
                        Rect::from_min_size(
                            area.min + Vec2::new(start + len, 0.),
                            Vec2::new(GAP, area.height()),
                        )
                    } else {
                        Rect::from_min_size(
                            area.min + Vec2::new(0., start + len),
                            Vec2::new(area.width(), GAP),
                        )
                    };
                    dividers.push(TreeDivider {
                        rect: gap,
                        between_columns: *row,
                        path: path.clone(),
                        index: i,
                        axis_len: axis,
                    });
                }
            }
        }
    }
}

fn equalize_node(node: &mut Node) {
    if let Node::Split { kids, w, .. } = node {
        *w = vec![1. / kids.len() as f32; kids.len()];
        kids.iter_mut().for_each(equalize_node);
    }
}

/// The path of child indexes from `node` down to the leaf `id`.
fn path_to(node: &Node, id: u64) -> Option<Vec<usize>> {
    match node {
        Node::Leaf(i) => (*i == id).then(Vec::new),
        Node::Split { kids, .. } => kids.iter().enumerate().find_map(|(i, k)| {
            let mut p = path_to(k, id)?;
            p.insert(0, i);
            Some(p)
        }),
    }
}

impl PaneTree {
    /// A tree holding `ids` side by side.
    pub fn new(ids: &[u64]) -> Self {
        let mut t = Self::default();
        for (i, id) in ids.iter().enumerate() {
            match i {
                0 => t.root = Some(Node::Leaf(*id)),
                _ => t.insert(*id, ids[i - 1], true),
            }
        }
        t
    }
    pub fn ids(&self) -> Vec<u64> {
        let mut out = Vec::new();
        if let Some(r) = &self.root {
            r.collect(&mut out);
        }
        out
    }
    fn insert(&mut self, new: u64, next_to: u64, row: bool) {
        match &mut self.root {
            None => self.root = Some(Node::Leaf(new)),
            Some(root) => {
                let target = if root.contains(next_to) {
                    next_to
                } else {
                    let mut all = Vec::new();
                    root.collect(&mut all);
                    *all.last().expect("a tree with a root has a leaf")
                };
                split_at(root, target, new, row);
            }
        }
    }
    /// Make the tree hold exactly `ids`: drop the ones that are gone, and place new ones next to
    /// `focus` (or where a split asked for them).
    pub fn sync(&mut self, ids: &[u64], focus: u64) {
        for gone in self.ids().into_iter().filter(|i| !ids.contains(i)) {
            if self.root.as_mut().is_some_and(|r| remove_from(r, gone)) {
                self.root = None;
            }
        }
        let have = self.ids();
        for id in ids.iter().filter(|i| !have.contains(i)) {
            let (near, row) = match self.pending.take() {
                Some((near, row)) => (near, row),
                None => (focus, true),
            };
            self.insert(*id, near, row);
        }
    }
    /// Rectangles of the panes and the dividers between them inside `area`.
    pub fn arrange(&self, area: Rect) -> (Vec<(u64, Rect)>, Vec<TreeDivider>) {
        let (mut rects, mut dividers) = (Vec::new(), Vec::new());
        if let Some(root) = &self.root {
            arrange_node(root, area, &mut Vec::new(), &mut rects, &mut dividers);
        }
        (rects, dividers)
    }
    /// Move a divider by `delta` (a share of its split's length).
    pub fn drag(&mut self, path: &[usize], index: usize, delta: f32) {
        if let Some(Node::Split { w, .. }) = self.root.as_mut().and_then(|r| node_at(r, path)) {
            drag(w, index, delta);
        }
    }
    pub fn equalize(&mut self) {
        if let Some(r) = &mut self.root {
            equalize_node(r);
        }
    }
    /// Grow `focus` toward `dir` by `delta` (negative shrinks): moves the nearest divider in that
    /// direction that belongs to a split along that axis.
    pub fn nudge(&mut self, focus: u64, dir: Dir, delta: f32) {
        let want_row = matches!(dir, Dir::Left | Dir::Right);
        let forward = matches!(dir, Dir::Right | Dir::Down);
        let Some(path) = self.root.as_ref().and_then(|r| path_to(r, focus)) else {
            return;
        };
        for depth in (0..path.len()).rev() {
            let Some(Node::Split { row, kids, w }) =
                self.root.as_mut().and_then(|r| node_at(r, &path[..depth]))
            else {
                continue;
            };
            if *row != want_row {
                continue;
            }
            let i = path[depth];
            let boundary = if forward { Some(i) } else { i.checked_sub(1) };
            if let Some(boundary) = boundary.filter(|b| b + 1 < kids.len()) {
                // Moving the boundary toward `dir` grows the side `focus` is on when it is the first
                // of the pair and `dir` is forward (or the second and backward).
                let grow = if forward { delta } else { -delta };
                drag(w, boundary, grow);
                return;
            }
        }
    }
}

impl Desktop {
    /// Split the focused pane: start tree mode if needed and ask for a new shell that will land next to it.
    pub(super) fn split_focused(&mut self, row: bool) {
        if self.readonly || !self.online {
            self.toast("Can't split: no writable connection");
            return;
        }
        let ids = self.tiled_ids();
        let focus = self.tiled_focus(&ids);
        let tree = self.tree.get_or_insert_with(|| PaneTree::new(&ids));
        if self.layout == LayoutMode::Single {
            self.layout = LayoutMode::Grid;
        }
        tree.pending = Some((focus, row));
        let cwd = self.focus_cwd();
        self.send(Command::New(cwd));
    }

    /// Draw the panes of the tree and handle its dividers; returns the pane rectangles in draw order.
    pub(super) fn tree_layout(
        &mut self,
        ui: &mut egui::Ui,
        area: Rect,
        ids: &[u64],
    ) -> Vec<(u64, Rect)> {
        let focus = self.tiled_focus(ids);
        let Some(mut tree) = self.tree.take() else {
            return Vec::new();
        };
        tree.sync(ids, focus);
        let t = self.theme.clone();
        let (mut rects, dividers) = tree.arrange(area);
        let mut moved = false;
        for d in &dividers {
            let handle = ui
                .interact(
                    d.rect,
                    egui::Id::new(("teryon-tree-divider", &d.path, d.index)),
                    Sense::drag(),
                )
                .on_hover_cursor(if d.between_columns {
                    egui::CursorIcon::ResizeHorizontal
                } else {
                    egui::CursorIcon::ResizeVertical
                });
            if handle.dragged() {
                let delta = if d.between_columns {
                    handle.drag_delta().x
                } else {
                    handle.drag_delta().y
                } / d.axis_len.max(1.);
                tree.drag(&d.path, d.index, delta);
                moved = true;
            }
            let active = handle.hovered() || handle.dragged();
            if !active && self.cfg.pane_inset > 0. {
                continue;
            }
            let line = if active { t.accent } else { t.border };
            let c = d.rect.center();
            let seg = if d.between_columns {
                [
                    Pos2::new(c.x, d.rect.top()),
                    Pos2::new(c.x, d.rect.bottom()),
                ]
            } else {
                [
                    Pos2::new(d.rect.left(), c.y),
                    Pos2::new(d.rect.right(), c.y),
                ]
            };
            ui.painter()
                .line_segment(seg, Stroke::new(1_f32, color(line)));
        }
        if moved {
            rects = tree.arrange(area).0;
        }
        self.tree = Some(tree);
        rects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(1000., 600.))
    }
    fn rect_of(t: &PaneTree, id: u64) -> Rect {
        t.arrange(area())
            .0
            .into_iter()
            .find(|(i, _)| *i == id)
            .unwrap()
            .1
    }

    #[test]
    fn splits_nest_in_any_direction_and_tile_the_area_without_overlap() {
        let mut t = PaneTree::new(&[1]);
        t.pending = Some((1, true));
        t.sync(&[1, 2], 1);
        t.pending = Some((2, false));
        t.sync(&[1, 2, 3], 2);
        t.pending = Some((3, true));
        t.sync(&[1, 2, 3, 4], 3);
        assert_eq!(t.ids().len(), 4);
        let (rects, dividers) = t.arrange(area());
        assert_eq!(rects.len(), 4);
        // 1 is the left half; 2 above 3 on the right, and 3 is split again side by side with 4.
        let r = |id| rects.iter().find(|(i, _)| *i == id).unwrap().1;
        assert!(r(1).right() < r(2).left() && (r(1).height() - 600.).abs() < 0.5);
        assert!(r(2).bottom() < r(3).top());
        assert!(r(3).right() < r(4).left() && (r(3).height() - r(4).height()).abs() < 0.5);
        for (i, (a, ra)) in rects.iter().enumerate() {
            for (b, rb) in &rects[i + 1..] {
                assert!(!ra.intersects(*rb), "{a} overlaps {b}");
            }
        }
        // Three splits: root (1 | right), right (2 over bottom), bottom (3 | 4).
        assert_eq!(dividers.len(), 3);
        let total: f32 = rects.iter().map(|(_, r)| r.width() * r.height()).sum();
        assert!(total > 600. * 1000. * 0.95, "{total}");
    }

    #[test]
    fn repeated_splits_in_one_direction_stay_flat_with_equal_shares() {
        let mut t = PaneTree::new(&[1]);
        for (n, id) in [2u64, 3, 4].into_iter().enumerate() {
            t.pending = Some((id - 1, true));
            let ids: Vec<u64> = (1..=id).collect();
            t.sync(&ids, id - 1);
            assert_eq!(t.ids().len(), n + 2);
        }
        let Some(Node::Split { kids, .. }) = &t.root else {
            panic!("a split")
        };
        assert_eq!(kids.len(), 4, "no needless nesting");
        t.equalize();
        let widths: Vec<f32> = (1..=4).map(|i| rect_of(&t, i).width()).collect();
        assert!(
            widths.iter().all(|w| (w - widths[0]).abs() < 0.5),
            "{widths:?}"
        );
    }

    #[test]
    fn a_closed_pane_gives_its_space_to_its_neighbours() {
        let mut t = PaneTree::new(&[1, 2, 3]);
        let before = rect_of(&t, 1).width();
        t.sync(&[1, 3], 1);
        assert_eq!(t.ids(), [1, 3]);
        assert!(rect_of(&t, 1).width() > before);
        let (rects, _) = t.arrange(area());
        let covered: f32 = rects.iter().map(|(_, r)| r.width()).sum::<f32>() + GAP;
        assert!((covered - 1000.).abs() < 1., "{covered}");
        // Down to one pane: it fills everything, with no split left over.
        t.sync(&[3], 3);
        assert_eq!(t.root, Some(Node::Leaf(3)));
        assert_eq!(rect_of(&t, 3), area());
        t.sync(&[], 0);
        assert!(t.root.is_none() && t.arrange(area()).0.is_empty());
    }

    #[test]
    fn a_pane_opened_elsewhere_is_split_off_the_focused_one() {
        let mut t = PaneTree::new(&[1, 2]);
        t.sync(&[1, 2, 9], 1);
        let (a, b) = (rect_of(&t, 1), rect_of(&t, 9));
        assert!(a.right() <= b.left() && b.right() <= rect_of(&t, 2).left());
        // A request for a split below 2 wins over the default placement.
        t.pending = Some((2, false));
        t.sync(&[1, 2, 9, 10], 1);
        assert!(rect_of(&t, 2).bottom() < rect_of(&t, 10).top());
        assert!(t.pending.is_none());
    }

    #[test]
    fn dragging_a_divider_resizes_only_its_two_sides_within_limits() {
        let mut t = PaneTree::new(&[1, 2]);
        let (_, dividers) = t.arrange(area());
        let d = &dividers[0];
        let before = rect_of(&t, 1).width();
        t.drag(&d.path, d.index, 0.1);
        assert!((rect_of(&t, 1).width() - before - 0.1 * (1000. - GAP)).abs() < 1.);
        t.drag(&d.path, d.index, 5.);
        assert!(
            rect_of(&t, 2).width() > 100.,
            "the smaller side keeps a usable share"
        );
        t.drag(&[7, 7], 0, 0.1); // a stale path is ignored
    }

    #[test]
    fn nudging_a_pane_moves_the_divider_on_that_side() {
        // 1 | (2 over 3)
        let mut t = PaneTree::new(&[1]);
        t.pending = Some((1, true));
        t.sync(&[1, 2], 1);
        t.pending = Some((2, false));
        t.sync(&[1, 2, 3], 2);
        let (w1, h2) = (rect_of(&t, 1).width(), rect_of(&t, 2).height());
        t.nudge(1, Dir::Right, 0.05);
        assert!(rect_of(&t, 1).width() > w1, "pane 1 grew toward the right");
        t.nudge(2, Dir::Down, 0.05);
        assert!(rect_of(&t, 2).height() > h2, "pane 2 grew downward");
        // Nothing to the left of pane 1, so nothing moves.
        let w1 = rect_of(&t, 1).width();
        t.nudge(1, Dir::Left, 0.05);
        assert_eq!(rect_of(&t, 1).width(), w1);
        // Pane 2 grows to the left by taking space from pane 1.
        t.nudge(2, Dir::Left, 0.05);
        assert!(rect_of(&t, 1).width() < w1);
    }

    fn add_pane(app: &mut Desktop, id: u64) {
        let view = PaneView {
            id,
            title: "sh".into(),
            cwd: "/tmp".into(),
            rows: 10,
            cols: 60,
            revision: 1,
            ..Default::default()
        };
        app.panes
            .insert(id, Pane::new(view, vt100::Parser::new(10, 60, 0)));
    }
    fn frame(app: &mut Desktop) {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200., 800.))),
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| app.frame(ctx));
    }

    #[test]
    fn splitting_in_the_app_builds_a_tree_and_a_layout_action_leaves_it() {
        use crate::desktop::tests::app_with_screen;
        let (mut app, rx) = app_with_screen("");
        app.run_action(ActionKind::SplitRight);
        assert!(matches!(rx.try_recv(), Ok(Command::New(_))));
        assert_eq!(app.tree.as_ref().unwrap().pending, Some((1, true)));
        add_pane(&mut app, 2);
        frame(&mut app);
        let (a, b) = (app.pane_rects[&1], app.pane_rects[&2]);
        assert!(a.right() <= b.left() && (a.top() - b.top()).abs() < 1.);
        // Split the new pane downward.
        app.focus = 2;
        app.run_action(ActionKind::SplitDown);
        // Drawing a frame also sends resizes; the split's request is the new shell.
        assert!(
            std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(c, Command::New(_))),
            "no new pane was requested"
        );
        add_pane(&mut app, 3);
        frame(&mut app);
        assert!(app.pane_rects[&2].bottom() <= app.pane_rects[&3].top());
        assert!((app.pane_rects[&2].left() - app.pane_rects[&3].left()).abs() < 1.);
        assert_eq!(app.pane_rects.len(), 3);
        // Resizing and equalizing work on the tree.
        let w = app.pane_rects[&1].width();
        app.focus = 1;
        app.run_action(ActionKind::Resize(Dir::Right));
        frame(&mut app);
        assert!(app.pane_rects[&1].width() > w);
        app.run_action(ActionKind::Equalize);
        frame(&mut app);
        assert!((app.pane_rects[&1].width() - app.pane_rects[&2].width()).abs() < 1.);
        // A closed pane's space is taken over by its neighbour.
        app.panes.remove(&2);
        frame(&mut app);
        assert_eq!(app.tree.as_ref().unwrap().ids(), [1, 3]);
        // Choosing a grid layout ends tree mode.
        app.run_action(ActionKind::Layout(LayoutMode::Grid));
        assert!(app.tree.is_none());
        // Read-only sessions cannot split.
        while rx.try_recv().is_ok() {}
        app.readonly = true;
        app.run_action(ActionKind::SplitRight);
        assert!(app.tree.is_none() && rx.try_recv().is_err());
    }
}
