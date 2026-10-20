//! Pane layout math: weighted rows/columns, draggable dividers and directional focus.
use super::*;

/// Smallest share a pane may be dragged down to.
pub(super) const MIN_SHARE: f32 = 0.12;
/// Space between panes, which is also the grab area of a divider.
pub(super) const GAP: f32 = 6.;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// Row and column shares of the visible layout; each vector sums to 1.
#[derive(Clone, Debug, Default)]
pub(super) struct Weights {
    pub rows: Vec<f32>,
    pub cols: Vec<f32>,
}
impl Weights {
    /// Keep the vectors at the right length, resetting to equal shares when the shape changed.
    pub fn fit(&mut self, rows: usize, cols: usize) {
        for (w, n) in [(&mut self.rows, rows), (&mut self.cols, cols)] {
            if w.len() != n {
                *w = vec![1. / n.max(1) as f32; n.max(1)];
            }
        }
    }
    pub fn equalize(&mut self) {
        self.rows.clear();
        self.cols.clear();
    }
}
/// Move the boundary between entry `i` and `i + 1` by `delta` (a share), keeping both usable.
pub(super) fn drag(w: &mut [f32], i: usize, delta: f32) {
    if i + 1 >= w.len() {
        return;
    }
    let pair = w[i] + w[i + 1];
    let first = (w[i] + delta).clamp(MIN_SHARE.min(pair / 2.), (pair - MIN_SHARE).max(pair / 2.));
    w[i] = first;
    w[i + 1] = pair - first;
}
/// `(start, length)` of each entry along an axis of `total` pixels with `gap` between entries.
pub(super) fn spans(total: f32, gap: f32, w: &[f32]) -> Vec<(f32, f32)> {
    let usable = (total - gap * w.len().saturating_sub(1) as f32).max(0.);
    let mut at = 0.;
    w.iter()
        .map(|share| {
            let len = usable * share;
            let span = (at, len);
            at += len + gap;
            span
        })
        .collect()
}

pub(super) struct Divider {
    pub rect: Rect,
    /// A vertical line between columns (dragged horizontally) rather than between rows.
    pub between_columns: bool,
    /// Boundary index within the row/column weights.
    pub index: usize,
    /// Pixels spanned by the whole axis, to turn a drag distance into a share.
    pub axis_len: f32,
    /// Row this divider sits in (column dividers only), to keep ids unique.
    pub row: usize,
}
pub(super) struct Arranged {
    pub rects: Vec<Rect>,
    pub dividers: Vec<Divider>,
}
/// Place `n` panes in a `(rows, cols)` grid inside `area`. A lone pane in a short last row spans it.
pub(super) fn arrange(area: Rect, dims: (usize, usize), n: usize, w: &Weights) -> Arranged {
    let (rows, cols) = (dims.0.max(1), dims.1.max(1));
    let col_spans = spans(area.width(), GAP, &w.cols);
    let row_spans = spans(area.height(), GAP, &w.rows);
    let mut rects = Vec::with_capacity(n);
    let mut dividers = Vec::new();
    for i in 0..n {
        let (r, c) = (i / cols, i % cols);
        let in_row = cols.min(n - r * cols);
        let (y, h) = row_spans[r.min(row_spans.len() - 1)];
        let (x, wd) = if in_row == 1 && cols > 1 {
            (0., area.width())
        } else {
            col_spans[c.min(col_spans.len() - 1)]
        };
        rects.push(Rect::from_min_size(
            area.min + Vec2::new(x, y),
            Vec2::new(wd, h),
        ));
    }
    for r in 0..rows {
        if cols.min(n.saturating_sub(r * cols)) < cols {
            continue;
        }
        let (y, h) = row_spans[r.min(row_spans.len() - 1)];
        for (c, &(x, wd)) in col_spans.iter().enumerate().take(cols.saturating_sub(1)) {
            dividers.push(Divider {
                rect: Rect::from_min_size(area.min + Vec2::new(x + wd, y), Vec2::new(GAP, h)),
                between_columns: true,
                index: c,
                axis_len: area.width(),
                row: r,
            });
        }
    }
    for (r, &(y, h)) in row_spans.iter().enumerate().take(rows.saturating_sub(1)) {
        dividers.push(Divider {
            rect: Rect::from_min_size(
                area.min + Vec2::new(0., y + h),
                Vec2::new(area.width(), GAP),
            ),
            between_columns: false,
            index: r,
            axis_len: area.height(),
            row: 0,
        });
    }
    Arranged { rects, dividers }
}

/// Nearest pane to `focus` in direction `dir`, from `(id, x0, y0, x1, y1)` boxes; no wrap-around.
pub(super) fn neighbor(boxes: &[(u64, [f32; 4])], focus: u64, dir: Dir) -> Option<u64> {
    let from = boxes.iter().find(|(id, _)| *id == focus)?.1;
    let center = |b: &[f32; 4]| ((b[0] + b[2]) / 2., (b[1] + b[3]) / 2.);
    let (fx, fy) = center(&from);
    boxes
        .iter()
        .filter(|(id, _)| *id != focus)
        .filter_map(|(id, b)| {
            let (cx, cy) = center(b);
            let (along, across) = match dir {
                Dir::Right => (cx - fx, (cy - fy).abs()),
                Dir::Left => (fx - cx, (cy - fy).abs()),
                Dir::Down => (cy - fy, (cx - fx).abs()),
                Dir::Up => (fy - cy, (cx - fx).abs()),
            };
            (along > 1.).then_some((*id, along + 2. * across))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

impl Desktop {
    /// Move the divider next to the focused pane by `step` of its axis, as Ghostty's resize_split does.
    pub(super) fn resize_focused(&mut self, dir: Dir, step: f32) {
        if let Some(tree) = &mut self.tree {
            tree.nudge(self.focus, dir, step);
            return;
        }
        let ids: Vec<u64> = self.panes.keys().copied().collect();
        let shown = visible_ids(&ids, self.focus, self.layout);
        let Some(at) = shown.iter().position(|id| *id == self.focus) else {
            return;
        };
        let (rows, cols) = self.layout.dims(shown.len());
        let (rows, cols) = (usize::from(rows), usize::from(cols));
        self.weights.fit(rows, cols);
        let (r, c) = (at / cols, at % cols);
        let sign = if matches!(dir, Dir::Right | Dir::Down) {
            1.
        } else {
            -1.
        };
        if matches!(dir, Dir::Left | Dir::Right) && cols > 1 {
            let boundary = if c + 1 < cols { c } else { c - 1 };
            drag(&mut self.weights.cols, boundary, sign * step);
        } else if matches!(dir, Dir::Up | Dir::Down) && rows > 1 {
            let boundary = if r + 1 < rows { r } else { r - 1 };
            drag(&mut self.weights.rows, boundary, sign * step);
        }
    }
    pub(super) fn focus_dir(&mut self, dir: Dir) {
        let boxes: Vec<(u64, [f32; 4])> = self
            .pane_rects
            .iter()
            .map(|(id, r)| (*id, [r.left(), r.top(), r.right(), r.bottom()]))
            .collect();
        if let Some(id) = neighbor(&boxes, self.focus, dir) {
            self.focus = id;
        }
    }
    pub(super) fn cycle_pane(&mut self, back: bool) {
        let ids: Vec<u64> = self.panes.keys().copied().collect();
        if let Some(at) = ids.iter().position(|id| *id == self.focus) {
            let n = ids.len();
            self.focus = ids[if back { (at + n - 1) % n } else { (at + 1) % n }];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sum(w: &[f32]) -> f32 {
        w.iter().sum()
    }
    #[test]
    fn drag_keeps_shares_normalized_and_usable() {
        let mut w = vec![0.5, 0.5];
        drag(&mut w, 0, 0.1);
        assert!((w[0] - 0.6).abs() < 1e-6 && (sum(&w) - 1.).abs() < 1e-6);
        drag(&mut w, 0, 5.);
        assert!((w[1] - MIN_SHARE).abs() < 1e-6);
        drag(&mut w, 0, -5.);
        assert!((w[0] - MIN_SHARE).abs() < 1e-6);
        let mut three = vec![0.4, 0.3, 0.3];
        drag(&mut three, 1, 0.05);
        assert!((sum(&three) - 1.).abs() < 1e-6 && (three[0] - 0.4).abs() < 1e-6);
        drag(&mut three, 2, 0.1);
        assert!((sum(&three) - 1.).abs() < 1e-6);
    }
    #[test]
    fn weights_reset_when_the_shape_changes() {
        let mut w = Weights::default();
        w.fit(2, 2);
        w.cols = vec![0.7, 0.3];
        w.fit(2, 2);
        assert_eq!(w.cols, vec![0.7, 0.3]);
        w.fit(3, 2);
        assert_eq!((w.rows.len(), w.cols.clone()), (3, vec![0.7, 0.3]));
        w.fit(3, 1);
        assert_eq!(w.cols, vec![1.]);
        w.equalize();
        w.fit(1, 1);
        assert_eq!(w.rows, vec![1.]);
    }
    #[test]
    fn spans_leave_room_for_gaps() {
        let s = spans(106., 6., &[0.5, 0.5]);
        assert_eq!(s, vec![(0., 50.), (56., 50.)]);
    }
    #[test]
    fn grid_arrangement_places_panes_and_dividers() {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(206., 106.));
        let mut w = Weights::default();
        w.fit(2, 2);
        let a = arrange(area, (2, 2), 4, &w);
        assert_eq!(a.rects.len(), 4);
        assert_eq!(a.rects[3].min, Pos2::new(106., 56.));
        // one column divider per full row, one row divider
        assert_eq!(a.dividers.iter().filter(|d| d.between_columns).count(), 2);
        assert_eq!(a.dividers.iter().filter(|d| !d.between_columns).count(), 1);
        let mut w3 = Weights::default();
        w3.fit(2, 2);
        let odd = arrange(area, (2, 2), 3, &w3);
        assert_eq!(odd.rects[2].width(), 206.);
        assert_eq!(odd.dividers.iter().filter(|d| d.between_columns).count(), 1);
    }
    #[test]
    fn neighbor_picks_the_nearest_pane_in_a_direction() {
        let boxes = vec![
            (1, [0., 0., 100., 100.]),
            (2, [100., 0., 200., 100.]),
            (3, [0., 100., 100., 200.]),
            (4, [100., 100., 200., 200.]),
        ];
        assert_eq!(neighbor(&boxes, 1, Dir::Right), Some(2));
        assert_eq!(neighbor(&boxes, 1, Dir::Down), Some(3));
        assert_eq!(neighbor(&boxes, 4, Dir::Left), Some(3));
        assert_eq!(neighbor(&boxes, 4, Dir::Up), Some(2));
        assert_eq!(neighbor(&boxes, 1, Dir::Left), None);
        assert_eq!(neighbor(&boxes, 9, Dir::Left), None);
    }
}
