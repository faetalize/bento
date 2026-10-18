//! Cell grid for bento's tiling layout.
//!
//! The workspace is an infinite horizontal strip of near-square cells, a fixed number of rows
//! tall. Every tiled window covers a rectangle of whole cells. Windows are independent of each
//! other: moving or resizing one pushes the others out of the way, and empty cells are allowed.
//! Columns of cells that no window covers collapse, so the strip never has empty gaps all the way
//! from top to bottom.
//!
//! This module is pure cell arithmetic with no compositor state, so it can be tested in isolation.

// Wired into the layout in a later phase.
#![allow(dead_code)]

use smithay::utils::{Logical, Point, Rectangle, Size};

/// Default number of rows in the grid.
///
/// 12 divides evenly into halves, thirds, quarters and sixths.
pub const DEFAULT_ROWS: i32 = 12;

/// Direction for a grid operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    fn delta(self) -> (i32, i32) {
        match self {
            Direction::Left => (-1, 0),
            Direction::Right => (1, 0),
            Direction::Up => (0, -1),
            Direction::Down => (0, 1),
        }
    }

    fn opposite(self) -> Self {
        match self {
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
            Direction::Up => Direction::Down,
            Direction::Down => Direction::Up,
        }
    }

    fn is_horizontal(self) -> bool {
        matches!(self, Direction::Left | Direction::Right)
    }
}

/// Rectangle of whole cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Area {
    /// Leftmost column.
    pub col: i32,
    /// Topmost row.
    pub row: i32,
    /// Width in columns.
    pub cols: i32,
    /// Height in rows.
    pub rows: i32,
}

impl Area {
    pub fn new(col: i32, row: i32, cols: i32, rows: i32) -> Self {
        Self {
            col,
            row,
            cols,
            rows,
        }
    }

    /// One past the rightmost column.
    pub fn right(&self) -> i32 {
        self.col + self.cols
    }

    /// One past the bottom row.
    pub fn bottom(&self) -> i32 {
        self.row + self.rows
    }

    pub fn overlaps(&self, other: &Area) -> bool {
        self.col < other.right()
            && other.col < self.right()
            && self.row < other.bottom()
            && other.row < self.bottom()
    }

    fn shifted(self, dx: i32, dy: i32) -> Self {
        Self {
            col: self.col + dx,
            row: self.row + dy,
            ..self
        }
    }

    /// Span perpendicular to `dir`: rows for horizontal directions, columns for vertical ones.
    fn cross_span(&self, dir: Direction) -> (i32, i32) {
        if dir.is_horizontal() {
            (self.row, self.bottom())
        } else {
            (self.col, self.right())
        }
    }
}

/// Windows placed on the cell grid, keyed by `K`.
#[derive(Debug, Clone)]
pub struct Grid<K> {
    /// Number of rows; always positive.
    rows: i32,
    /// Windows and their areas, in insertion order.
    items: Vec<(K, Area)>,
}

impl<K: Clone + PartialEq> Grid<K> {
    pub fn new(rows: i32) -> Self {
        assert!(rows > 0);
        Self {
            rows,
            items: Vec::new(),
        }
    }

    pub fn rows(&self) -> i32 {
        self.rows
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Width of the strip in columns.
    pub fn width(&self) -> i32 {
        self.items.iter().map(|(_, a)| a.right()).max().unwrap_or(0)
    }

    pub fn area(&self, id: &K) -> Option<Area> {
        self.idx(id).map(|idx| self.items[idx].1)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, Area)> + '_ {
        self.items.iter().map(|(k, a)| (k, *a))
    }

    fn idx(&self, id: &K) -> Option<usize> {
        self.items.iter().position(|(k, _)| k == id)
    }

    /// Clamps an area to a valid size and to the rows of the grid.
    fn clamp_area(&self, mut area: Area) -> Area {
        area.cols = area.cols.max(1);
        area.rows = area.rows.clamp(1, self.rows);
        area.row = area.row.clamp(0, self.rows - area.rows);
        area
    }

    /// Inserts a window at `area`, pushing overlapping windows to the right.
    pub fn insert(&mut self, id: K, area: Area) {
        debug_assert!(self.idx(&id).is_none(), "window is already in the grid");

        let area = self.clamp_area(area);
        self.items.push((id, area));
        let idx = self.items.len() - 1;
        self.settle(&[idx], Direction::Right);
        self.normalize();
    }

    /// Inserts a window of `cols` × `rows` cells right of `after`, or at the end of the strip.
    ///
    /// The window lines up with the top of `after` where it fits.
    pub fn insert_after(&mut self, id: K, after: Option<&K>, cols: i32, rows: i32) {
        let area = match after.and_then(|after| self.area(after)) {
            Some(after) => Area::new(after.right(), after.row, cols, rows),
            None => Area::new(self.width(), 0, cols, rows),
        };
        self.insert(id, area);
    }

    /// Removes a window, leaving its cells empty.
    pub fn remove(&mut self, id: &K) -> Option<Area> {
        let idx = self.idx(id)?;
        let (_, area) = self.items.remove(idx);
        self.normalize();
        Some(area)
    }

    /// Sets a window's area, pushing overlapping windows in `dir`.
    ///
    /// Returns whether any window changed position or size.
    pub fn set_area(&mut self, id: &K, area: Area, dir: Direction) -> bool {
        let Some(idx) = self.idx(id) else {
            return false;
        };

        let before = self.items.clone();
        self.items[idx].1 = self.clamp_area(area);
        self.settle(&[idx], dir);
        self.normalize();
        self.items != before
    }

    /// Resizes a window to `cols` × `rows`, keeping its top-left corner.
    ///
    /// Growing down pushes windows below it down; growing right pushes windows right.
    pub fn resize(&mut self, id: &K, cols: i32, rows: i32) -> bool {
        let Some(idx) = self.idx(id) else {
            return false;
        };

        let before = self.items.clone();

        let area = self.items[idx].1;
        self.items[idx].1 = self.clamp_area(Area { rows, ..area });
        self.settle(&[idx], Direction::Down);

        let area = self.items[idx].1;
        self.items[idx].1 = self.clamp_area(Area { cols, ..area });
        self.settle(&[idx], Direction::Right);

        self.normalize();
        self.items != before
    }

    /// Moves a window by one cell in `dir`.
    ///
    /// If the cells it moves into are occupied, it trades places with the windows there instead:
    /// it jumps past them and they move into the space it left. Returns whether any window
    /// changed position.
    pub fn move_window(&mut self, id: &K, dir: Direction) -> bool {
        let Some(idx) = self.idx(id) else {
            return false;
        };

        let area = self.items[idx].1;
        let (dx, dy) = dir.delta();
        let target = area.shifted(dx, dy);
        if target.row < 0 || target.bottom() > self.rows {
            return false;
        }

        let before = self.items.clone();

        // Blockers all touch the window's edge in `dir`, since they can't overlap the window.
        let blockers: Vec<usize> = (0..self.items.len())
            .filter(|&i| i != idx && self.items[i].1.overlaps(&target))
            .collect();

        if blockers.is_empty() {
            self.items[idx].1 = target;
        } else {
            let blocker_areas = || blockers.iter().map(|&i| self.items[i].1);
            let (window_area, blocker_shift) = match dir {
                Direction::Right => {
                    let end = blocker_areas().map(|a| a.right()).max().unwrap();
                    (
                        Area {
                            col: end - area.cols,
                            ..area
                        },
                        (-area.cols, 0),
                    )
                }
                Direction::Left => {
                    let start = blocker_areas().map(|a| a.col).min().unwrap();
                    (Area { col: start, ..area }, (area.cols, 0))
                }
                Direction::Down => {
                    let end = blocker_areas().map(|a| a.bottom()).max().unwrap();
                    (
                        Area {
                            row: end - area.rows,
                            ..area
                        },
                        (0, -area.rows),
                    )
                }
                Direction::Up => {
                    let start = blocker_areas().map(|a| a.row).min().unwrap();
                    (Area { row: start, ..area }, (0, area.rows))
                }
            };

            self.items[idx].1 = window_area;
            for &i in &blockers {
                let a = self.items[i].1;
                self.items[i].1 = a.shifted(blocker_shift.0, blocker_shift.1);
            }

            // The blockers moved against `dir`, so anything they now overlap gets pushed that
            // way too.
            let mut pinned = blockers;
            pinned.push(idx);
            self.settle(&pinned, dir.opposite());
        }

        self.normalize();
        self.items != before
    }

    /// Finds the window to focus when moving from `id` in `dir`.
    ///
    /// `anchor` is the row (for left and right) or column (for up and down) that the user is
    /// travelling along. It's clamped into the window's own span; `None` uses the window's top
    /// row or left column.
    ///
    /// Among the windows past the edge in `dir`, the nearest one wins, so repeatedly moving in
    /// one direction never skips a window. Windows overlapping the anchor break ties, then
    /// windows overlapping the current window's span, then the distance from the anchor.
    pub fn neighbor(&self, id: &K, dir: Direction, anchor: Option<i32>) -> Option<&K> {
        let area = self.area(id)?;
        let (start, end) = area.cross_span(dir);
        let anchor = anchor.unwrap_or(start).clamp(start, end - 1);

        self.items
            .iter()
            .filter(|(k, _)| k != id)
            .filter_map(|(k, a)| {
                let gap = match dir {
                    Direction::Right => a.col - area.right(),
                    Direction::Left => area.col - a.right(),
                    Direction::Down => a.row - area.bottom(),
                    Direction::Up => area.row - a.bottom(),
                };
                if gap < 0 {
                    return None;
                }

                let (s, e) = a.cross_span(dir);
                let tier = if s <= anchor && anchor < e {
                    0
                } else if s < end && start < e {
                    1
                } else {
                    2
                };
                let dist = if anchor < s {
                    s - anchor
                } else if anchor >= e {
                    anchor - e + 1
                } else {
                    0
                };

                Some(((tier == 2, gap, tier, dist, s), k))
            })
            .min_by_key(|(key, _)| *key)
            .map(|(_, k)| k)
    }

    /// Resolves overlaps by pushing windows in `dir`.
    ///
    /// Windows in `pinned` never move, and must not overlap each other. A window pushed off the
    /// top or bottom goes to the right of the window that pushed it instead, keeping its rows.
    fn settle(&mut self, pinned: &[usize], dir: Direction) {
        let n = self.items.len();
        let is_pinned = |i: usize| pinned.contains(&i);
        let mut moved = vec![false; n];
        for &i in pinned {
            moved[i] = true;
        }

        // Every push moves a window strictly further along `dir` (counting a push off the top or
        // bottom as a move right), and positions are bounded, so this terminates. The cap is a
        // safety net.
        let max_steps = 64 + n * n * (self.rows as usize + 1) * 4;
        for _ in 0..max_steps {
            // Among all overlaps involving a window that has moved, resolve the one whose pusher
            // comes first along `dir`, so pushes propagate outwards in order.
            let mut best: Option<((bool, i32, usize), usize, usize)> = None;
            for a in (0..n).filter(|&a| moved[a]) {
                let area = self.items[a].1;
                for b in (0..n).filter(|&b| b != a && area.overlaps(&self.items[b].1)) {
                    // Pinned windows push; otherwise an unmoved window gets pushed; otherwise
                    // the one further along `dir` does.
                    let b_ahead = (self.along(b, dir), b) > (self.along(a, dir), a);
                    let (pusher, pushed) = if is_pinned(b) {
                        (b, a)
                    } else if is_pinned(a) || !moved[b] || b_ahead {
                        (a, b)
                    } else {
                        (b, a)
                    };
                    debug_assert!(!is_pinned(pushed), "pinned windows must not overlap");

                    let key = (!is_pinned(pusher), self.along(pusher, dir), pusher);
                    if best.is_none_or(|(best_key, _, _)| key < best_key) {
                        best = Some((key, pusher, pushed));
                    }
                }
            }

            let Some((_, pusher, pushed)) = best else {
                return;
            };
            self.push(pusher, pushed, dir);
            moved[pushed] = true;
        }

        debug_assert!(false, "grid settle did not converge");
    }

    /// Position of a window along `dir`; larger means further in that direction.
    fn along(&self, idx: usize, dir: Direction) -> i32 {
        let a = self.items[idx].1;
        match dir {
            Direction::Right => a.col,
            Direction::Left => -a.right(),
            Direction::Down => a.row,
            Direction::Up => -a.bottom(),
        }
    }

    /// Moves `pushed` just past `pusher` in `dir`.
    fn push(&mut self, pusher: usize, pushed: usize, dir: Direction) {
        let p = self.items[pusher].1;
        let mut q = self.items[pushed].1;
        match dir {
            Direction::Right => q.col = p.right(),
            Direction::Left => q.col = p.col - q.cols,
            Direction::Down => {
                if p.bottom() + q.rows <= self.rows {
                    q.row = p.bottom();
                } else {
                    q.col = p.right();
                }
            }
            Direction::Up => {
                if p.row - q.rows >= 0 {
                    q.row = p.row - q.rows;
                } else {
                    q.col = p.right();
                }
            }
        }
        self.items[pushed].1 = q;
    }

    /// Collapses columns that no window covers and moves the strip to start at column 0.
    fn normalize(&mut self) {
        let Some(min) = self.items.iter().map(|(_, a)| a.col).min() else {
            return;
        };
        let width = (self.width() - min) as usize;

        let mut covered = vec![false; width];
        for (_, a) in &self.items {
            for c in a.col..a.right() {
                covered[(c - min) as usize] = true;
            }
        }

        // Number of uncovered columns before each column.
        let mut empty_before = Vec::with_capacity(width);
        let mut count = 0;
        for is_covered in covered {
            empty_before.push(count);
            if !is_covered {
                count += 1;
            }
        }

        // A window covers contiguous columns, so none of its own columns collapse.
        for (_, a) in &mut self.items {
            let rel = (a.col - min) as usize;
            a.col = rel as i32 - empty_before[rel];
        }
    }

    #[cfg(test)]
    pub fn verify_invariants(&self) {
        for (i, (k, a)) in self.items.iter().enumerate() {
            assert!(a.cols >= 1 && a.rows >= 1, "empty area: {a:?}");
            assert!(
                a.row >= 0 && a.bottom() <= self.rows,
                "area outside rows: {a:?}"
            );
            assert!(a.col >= 0, "area before column 0: {a:?}");
            assert!(
                self.items[..i].iter().all(|(other, _)| other != k),
                "duplicate window"
            );
            for (_, b) in &self.items[i + 1..] {
                assert!(!a.overlaps(b), "overlapping areas: {a:?} and {b:?}");
            }
        }

        for c in 0..self.width() {
            assert!(
                self.items.iter().any(|(_, a)| a.col <= c && c < a.right()),
                "empty column {c}"
            );
        }
    }
}

/// Pixel geometry of the cell grid for a given view size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellMetrics {
    /// Cell width in logical pixels.
    pub cell_w: f64,
    /// Cell height in logical pixels.
    pub cell_h: f64,
    /// Gap between cells and around the edges.
    pub gap: f64,
    /// Number of columns that exactly fill the view width.
    pub cols_per_view: i32,
    /// Number of rows that exactly fill the view height.
    pub rows: i32,
}

impl CellMetrics {
    /// Computes cells that fill the view height with `rows` rows and are as close to square as
    /// possible while fitting a whole number of columns in the view width.
    pub fn new(view_size: Size<f64, Logical>, rows: i32, gap: f64) -> Self {
        let rows_f = f64::from(rows);
        let cell_h = ((view_size.h - gap * (rows_f + 1.)) / rows_f).max(1.);

        // n columns take n * cell_w + (n + 1) * gap.
        let n = ((view_size.w - gap) / (cell_h + gap)).round().max(1.);
        let cell_w = ((view_size.w - gap * (n + 1.)) / n).max(1.);

        Self {
            cell_w,
            cell_h,
            gap,
            cols_per_view: n as i32,
            rows,
        }
    }

    /// Rectangle of an area, relative to the top-left corner of the strip.
    pub fn rect(&self, area: Area) -> Rectangle<f64, Logical> {
        let loc = Point::from((
            self.gap + f64::from(area.col) * (self.cell_w + self.gap),
            self.gap + f64::from(area.row) * (self.cell_h + self.gap),
        ));
        let size = Size::from((
            f64::from(area.cols) * self.cell_w + f64::from(area.cols - 1) * self.gap,
            f64::from(area.rows) * self.cell_h + f64::from(area.rows - 1) * self.gap,
        ));
        Rectangle::new(loc, size)
    }

    /// Number of columns closest to `proportion` of the view width; at least one.
    pub fn cols_for_proportion(&self, proportion: f64) -> i32 {
        ((proportion * f64::from(self.cols_per_view)).round() as i32).max(1)
    }

    /// Number of rows closest to `proportion` of the view height; at least one.
    pub fn rows_for_proportion(&self, proportion: f64) -> i32 {
        ((proportion * f64::from(self.rows)).round() as i32).clamp(1, self.rows)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const R: i32 = DEFAULT_ROWS;

    fn grid(items: &[(u32, Area)]) -> Grid<u32> {
        let grid = Grid {
            rows: R,
            items: items.to_vec(),
        };
        grid.verify_invariants();
        grid
    }

    fn area(g: &Grid<u32>, id: u32) -> Area {
        g.area(&id).unwrap()
    }

    #[test]
    fn insert_after_pushes_right() {
        let mut g = Grid::new(R);
        g.insert_after(1, None, 6, R);
        g.insert_after(2, Some(&1), 6, R);
        g.insert_after(3, Some(&1), 4, R);
        g.verify_invariants();

        assert_eq!(area(&g, 1), Area::new(0, 0, 6, R));
        assert_eq!(area(&g, 3), Area::new(6, 0, 4, R));
        assert_eq!(area(&g, 2), Area::new(10, 0, 6, R));
    }

    #[test]
    fn insert_aligns_with_top_of_previous() {
        let mut g = grid(&[(1, Area::new(0, 6, 6, 6)), (2, Area::new(0, 0, 6, 6))]);
        g.insert_after(3, Some(&1), 6, 6);
        g.verify_invariants();
        assert_eq!(area(&g, 3), Area::new(6, 6, 6, 6));
    }

    #[test]
    fn move_right_swaps_full_height_windows() {
        let mut g = grid(&[(1, Area::new(0, 0, 6, R)), (2, Area::new(6, 0, 6, R))]);
        assert!(g.move_window(&1, Direction::Right));
        g.verify_invariants();
        assert_eq!(area(&g, 2), Area::new(0, 0, 6, R));
        assert_eq!(area(&g, 1), Area::new(6, 0, 6, R));
    }

    #[test]
    fn move_right_swaps_different_widths() {
        let mut g = grid(&[(1, Area::new(0, 0, 6, R)), (2, Area::new(6, 0, 3, R))]);
        assert!(g.move_window(&1, Direction::Right));
        g.verify_invariants();
        assert_eq!(area(&g, 2), Area::new(0, 0, 3, R));
        assert_eq!(area(&g, 1), Area::new(3, 0, 6, R));
    }

    #[test]
    fn move_past_end_does_nothing() {
        let mut g = grid(&[(1, Area::new(0, 0, 6, R)), (2, Area::new(6, 0, 6, R))]);
        assert!(!g.move_window(&2, Direction::Right));
        assert!(!g.move_window(&1, Direction::Left));
        assert!(!g.move_window(&1, Direction::Up));
        assert!(!g.move_window(&1, Direction::Down));
    }

    #[test]
    fn move_into_hole() {
        // 1 | 3
        // 2 | .
        let mut g = grid(&[
            (1, Area::new(0, 0, 6, 6)),
            (2, Area::new(0, 6, 6, 6)),
            (3, Area::new(6, 0, 6, 6)),
        ]);
        assert!(g.move_window(&3, Direction::Down));
        g.verify_invariants();
        assert_eq!(area(&g, 3), Area::new(6, 1, 6, 6));
    }

    #[test]
    fn move_left_out_of_shared_column() {
        // 2 | 1
        // 3 | .
        let mut g = grid(&[
            (2, Area::new(0, 0, 6, 6)),
            (3, Area::new(0, 6, 6, 6)),
            (1, Area::new(6, 0, 6, 6)),
        ]);
        assert!(g.move_window(&3, Direction::Left));
        g.verify_invariants();
        assert_eq!(area(&g, 3), Area::new(0, 6, 6, 6));
        assert_eq!(area(&g, 2), Area::new(1, 0, 6, 6));
        assert_eq!(area(&g, 1), Area::new(7, 0, 6, 6));
    }

    #[test]
    fn vertical_swap() {
        let mut g = grid(&[(1, Area::new(0, 0, 6, 4)), (2, Area::new(0, 4, 6, 8))]);
        assert!(g.move_window(&1, Direction::Down));
        g.verify_invariants();
        assert_eq!(area(&g, 2), Area::new(0, 0, 6, 8));
        assert_eq!(area(&g, 1), Area::new(0, 8, 6, 4));
    }

    #[test]
    fn swap_pushes_windows_inside_the_exchanged_block() {
        // 1 is 2 rows tall; 2 is a full-height window next to it; 3 sits in 1's rows past 2.
        let mut g = grid(&[
            (1, Area::new(0, 0, 4, 2)),
            (2, Area::new(4, 0, 2, R)),
            (3, Area::new(6, 0, 2, 2)),
            (4, Area::new(0, 2, 4, 10)),
        ]);
        assert!(g.move_window(&1, Direction::Right));
        g.verify_invariants();
    }

    #[test]
    fn remove_collapses_empty_columns_only() {
        let mut g = grid(&[
            (1, Area::new(0, 0, 6, R)),
            (2, Area::new(6, 0, 6, 6)),
            (3, Area::new(6, 6, 3, 6)),
            (4, Area::new(12, 0, 6, R)),
        ]);

        // Removing 3 leaves a hole under 2, since 2 still covers those columns.
        g.remove(&3);
        g.verify_invariants();
        assert_eq!(area(&g, 4), Area::new(12, 0, 6, R));

        // Removing 2 empties its columns, which collapse.
        g.remove(&2);
        g.verify_invariants();
        assert_eq!(area(&g, 4), Area::new(6, 0, 6, R));
    }

    #[test]
    fn growing_down_pushes_down() {
        let mut g = grid(&[(1, Area::new(0, 0, 6, 4)), (2, Area::new(0, 4, 6, 4))]);
        assert!(g.resize(&1, 6, 6));
        g.verify_invariants();
        assert_eq!(area(&g, 2), Area::new(0, 6, 6, 4));
    }

    #[test]
    fn pushed_off_the_bottom_goes_right() {
        let mut g = grid(&[(1, Area::new(0, 0, 6, 6)), (2, Area::new(0, 6, 6, 6))]);
        assert!(g.resize(&1, 6, R));
        g.verify_invariants();
        assert_eq!(area(&g, 1), Area::new(0, 0, 6, R));
        assert_eq!(area(&g, 2), Area::new(6, 6, 6, 6));
    }

    #[test]
    fn growing_right_cascades() {
        let mut g = grid(&[
            (1, Area::new(0, 0, 4, R)),
            (2, Area::new(4, 0, 4, 6)),
            (3, Area::new(8, 0, 4, R)),
            (4, Area::new(4, 6, 4, 6)),
        ]);
        assert!(g.resize(&1, 6, R));
        g.verify_invariants();
        assert_eq!(area(&g, 2), Area::new(6, 0, 4, 6));
        assert_eq!(area(&g, 4), Area::new(6, 6, 4, 6));
        assert_eq!(area(&g, 3), Area::new(10, 0, 4, R));
    }

    #[test]
    fn neighbor_zigzag() {
        // AAAAAAAA DDDD
        // BBBB CCCCCCCC
        let g = grid(&[
            (1, Area::new(0, 0, 8, 6)),  // A
            (4, Area::new(8, 0, 4, 6)),  // D
            (2, Area::new(0, 6, 4, 6)),  // B
            (3, Area::new(4, 6, 8, 6)),  // C
            (5, Area::new(12, 0, 6, R)), // E, next full-height window
        ]);
        let n = |id, dir, anchor| g.neighbor(&id, dir, anchor).copied();

        assert_eq!(n(1, Direction::Right, None), Some(4));
        assert_eq!(n(4, Direction::Right, None), Some(5));
        assert_eq!(n(2, Direction::Right, None), Some(3));
        assert_eq!(n(3, Direction::Right, None), Some(5));
        assert_eq!(n(4, Direction::Left, None), Some(1));

        // Down from A depends on which column the user is travelling along.
        assert_eq!(n(1, Direction::Down, Some(1)), Some(2));
        assert_eq!(n(1, Direction::Down, Some(6)), Some(3));

        // Left from E depends on which row the user is travelling along.
        assert_eq!(n(5, Direction::Left, Some(2)), Some(4));
        assert_eq!(n(5, Direction::Left, Some(8)), Some(3));

        assert_eq!(n(1, Direction::Left, None), None);
        assert_eq!(n(1, Direction::Up, None), None);
    }

    #[test]
    fn neighbor_prefers_nearest_over_anchor() {
        // A full-height window with a short window next to it and another one further away at
        // the anchor's height: the adjacent one wins so that nothing is skipped.
        let g = grid(&[
            (1, Area::new(0, 0, 6, R)),
            (2, Area::new(6, 6, 6, 6)),
            (3, Area::new(12, 0, 6, 6)),
        ]);
        assert_eq!(g.neighbor(&1, Direction::Right, Some(2)), Some(&2));
    }

    #[test]
    fn metrics_near_square() {
        let m = CellMetrics::new(Size::from((1920., 1080.)), R, 16.);
        assert_eq!(m.cols_per_view, 21);
        assert!((m.cell_w / m.cell_h - 1.).abs() < 0.05);

        // The cells exactly fill the view.
        let full = m.rect(Area::new(0, 0, m.cols_per_view, R));
        assert!((full.loc.x + full.size.w + m.gap - 1920.).abs() < 1e-6);
        assert!((full.loc.y + full.size.h + m.gap - 1080.).abs() < 1e-6);

        assert_eq!(m.rows_for_proportion(0.5), 6);
        assert_eq!(m.rows_for_proportion(1. / 3.), 4);
        assert_eq!(m.rows_for_proportion(0.25), 3);
    }

    #[derive(Debug, Clone)]
    enum Op {
        Insert { after: usize, cols: i32, rows: i32 },
        Remove(usize),
        Move(usize, Direction),
        Resize(usize, i32, i32),
        SetArea(usize, Area, Direction),
    }

    fn direction() -> impl Strategy<Value = Direction> {
        prop_oneof![
            Just(Direction::Left),
            Just(Direction::Right),
            Just(Direction::Up),
            Just(Direction::Down),
        ]
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            3 => (any::<usize>(), 1..=24i32, 1..=R).prop_map(|(after, cols, rows)| Op::Insert {
                after,
                cols,
                rows
            }),
            1 => any::<usize>().prop_map(Op::Remove),
            4 => (any::<usize>(), direction()).prop_map(|(i, d)| Op::Move(i, d)),
            2 => (any::<usize>(), 1..=24i32, 1..=R).prop_map(|(i, c, r)| Op::Resize(i, c, r)),
            1 => (any::<usize>(), -2..30i32, 0..R, 1..=12i32, 1..=R, direction())
                .prop_map(|(i, col, row, cols, rows, d)| Op::SetArea(
                    i,
                    Area::new(col, row, cols, rows),
                    d
                )),
        ]
    }

    proptest! {
        #[test]
        fn random_operations_keep_invariants(ops in prop::collection::vec(op(), 1..60)) {
            let mut g = Grid::new(R);
            let mut next_id = 0u32;

            for op in ops {
                let ids: Vec<u32> = g.iter().map(|(k, _)| *k).collect();
                let pick = |i: usize| (!ids.is_empty()).then(|| ids[i % ids.len()]);
                let sizes: Vec<(u32, Area)> = g.iter().map(|(k, a)| (*k, a)).collect();

                // The window being operated on, which is the only one allowed to change size.
                let mut target = None;
                match op {
                    Op::Insert { after, cols, rows } => {
                        g.insert_after(next_id, pick(after).as_ref(), cols, rows);
                        next_id += 1;
                    }
                    Op::Remove(i) => {
                        if let Some(id) = pick(i) {
                            g.remove(&id);
                        }
                    }
                    Op::Move(i, dir) => {
                        if let Some(id) = pick(i) {
                            g.move_window(&id, dir);
                        }
                    }
                    Op::Resize(i, cols, rows) => {
                        target = pick(i);
                        if let Some(id) = target {
                            g.resize(&id, cols, rows);
                        }
                    }
                    Op::SetArea(i, area, dir) => {
                        target = pick(i);
                        if let Some(id) = target {
                            g.set_area(&id, area, dir);
                        }
                    }
                }

                g.verify_invariants();

                // Pushing never resizes other windows.
                for (id, old) in sizes {
                    if Some(id) == target {
                        continue;
                    }
                    if let Some(new) = g.area(&id) {
                        prop_assert_eq!((new.cols, new.rows), (old.cols, old.rows));
                    }
                }
            }
        }
    }
}
