//! Cell grid for bento's tiling layout.
//!
//! The workspace is an infinite horizontal strip of near-square cells, a fixed number of rows
//! tall. Every tiled window covers a rectangle of whole cells. Windows are independent of each
//! other: moving or resizing one pushes the others out of the way, and empty cells are allowed.
//! Columns of cells that no window covers collapse, so the strip never has empty gaps all the way
//! from top to bottom. With left gravity, windows also slide left within their rows until they
//! touch another window or column 0, which is the fixed left edge of the strip, like columns
//! close up in niri. Without gravity, the strip goes on to the left too.
//!
//! Positions never depend on where the windows are: column 0 stays put, and nothing gets
//! renumbered to start at the leftmost window.
//!
//! This module is pure cell arithmetic with no compositor state, so it can be tested in isolation.

use std::iter::zip;

use smithay::utils::{Logical, Point, Rectangle, Size};

/// Default number of rows in the grid.
///
/// 12 divides evenly into halves, thirds, quarters and sixths.
pub const DEFAULT_ROWS: i32 = 12;

/// Largest width of a window in columns, which keeps cell arithmetic far from overflowing.
pub const MAX_COLS: i32 = 10_000;

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

    pub fn is_horizontal(self) -> bool {
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
#[derive(Debug, Clone, PartialEq)]
pub struct Grid<K> {
    /// Number of rows; always positive.
    rows: i32,
    /// Whether windows slide left within their rows until they touch another window.
    left_gravity: bool,
    /// Window held in place by the pointer, which gravity doesn't move until it's released.
    pinned: Option<K>,
    /// Windows and their areas, in insertion order.
    items: Vec<(K, Area)>,
}

impl<K: Clone + PartialEq> Grid<K> {
    pub fn new(rows: i32) -> Self {
        assert!(rows > 0);
        Self {
            rows,
            left_gravity: false,
            pinned: None,
            items: Vec::new(),
        }
    }

    pub fn pinned(&self) -> Option<&K> {
        self.pinned.as_ref()
    }

    /// Pins a window in place against gravity, like holding it with the pointer, or releases the
    /// pin with `None` and lets the window fall.
    pub fn set_pinned(&mut self, id: Option<K>) {
        self.pinned = id;
        self.normalize();
    }

    pub fn left_gravity(&self) -> bool {
        self.left_gravity
    }

    /// Turns left gravity on or off. Turning it on slides windows left right away.
    pub fn set_left_gravity(&mut self, left_gravity: bool) {
        self.left_gravity = left_gravity;
        self.normalize();
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
        if self.left_gravity {
            area.col = area.col.max(0);
        }
        area.cols = area.cols.clamp(1, MAX_COLS);
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

    /// Resizes every window to its span from `span`, and scales column positions by `ratio`,
    /// for a grid with different cells.
    ///
    /// Windows keep their order and rows; ones that would overlap the windows before them move
    /// right.
    pub fn rescale(&mut self, ratio: f64, mut span: impl FnMut(&K) -> (i32, i32)) {
        for i in 0..self.items.len() {
            let (cols, rows) = span(&self.items[i].0);
            let area = self.items[i].1;
            let col = (f64::from(area.col) * ratio).round() as i32;
            self.items[i].1 = self.clamp_area(Area::new(col, area.row, cols, rows));
        }

        // Going left to right, every window before the current one is already in its final
        // place.
        let mut order: Vec<usize> = (0..self.items.len()).collect();
        order.sort_by_key(|&i| (self.items[i].1.col, self.items[i].1.row));
        for (n, &i) in order.iter().enumerate() {
            let area = self.items[i].1;
            let col = order[..n]
                .iter()
                .map(|&j| self.items[j].1)
                .filter(|b| b.row < area.bottom() && area.row < b.bottom())
                .map(|b| b.right())
                .fold(area.col, i32::max);
            self.items[i].1.col = col;
        }

        self.normalize();
    }

    /// Removes a window, leaving its cells empty.
    pub fn remove(&mut self, id: &K) -> Option<Area> {
        let idx = self.idx(id)?;
        let (_, area) = self.items.remove(idx);
        if self.pinned.as_ref() == Some(id) {
            self.pinned = None;
        }
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
        self.finish(before)
    }

    /// Sets the areas of several windows at once, adding the ones not in the grid yet, and pushes
    /// other overlapping windows to the right.
    ///
    /// The areas must not overlap each other. Returns whether any window changed position or
    /// size.
    pub fn place(&mut self, areas: Vec<(K, Area)>) -> bool {
        let before = self.items.clone();

        let mut pinned = Vec::with_capacity(areas.len());
        for (id, area) in areas {
            let area = self.clamp_area(area);
            let idx = match self.idx(&id) {
                Some(idx) => {
                    self.items[idx].1 = area;
                    idx
                }
                None => {
                    self.items.push((id, area));
                    self.items.len() - 1
                }
            };
            pinned.push(idx);
        }
        debug_assert!(
            pinned.iter().enumerate().all(|(n, &i)| pinned[..n]
                .iter()
                .all(|&j| !self.items[i].1.overlaps(&self.items[j].1))),
            "placed areas must not overlap"
        );

        self.settle(&pinned, Direction::Right);
        self.finish(before)
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

        self.finish(before)
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

        self.finish(before)
    }

    /// Finishes a change that started from `before`, returning whether anything changed.
    ///
    /// With left gravity, column 0 is the left edge of the grid that windows pack against, so a
    /// change that pushed a window past it doesn't happen at all. Without gravity, the grid goes
    /// on to the left too.
    fn finish(&mut self, before: Vec<(K, Area)>) -> bool {
        if self.left_gravity && self.items.iter().any(|(_, a)| a.col < 0) {
            self.items = before;
            return false;
        }

        self.normalize();

        // Shifting every window by the same number of columns leaves the layout as it was, only
        // somewhere else on the grid, so it isn't a change. Like moving the leftmost window left
        // without gravity: collapsing the empty column it leaves pulls everything else along.
        let shifted_by = |dx: i32| {
            self.items.len() == before.len()
                && zip(&self.items, &before).all(|((k, a), (bk, b))| {
                    k == bk
                        && *a
                            == Area {
                                col: b.col + dx,
                                ..*b
                            }
                })
        };
        let dx = match (self.items.first(), before.first()) {
            (Some((_, a)), Some((_, b))) => a.col - b.col,
            _ => 0,
        };
        if dx != 0 && shifted_by(dx) {
            self.items = before;
            return false;
        }

        self.items != before
    }

    /// Columns of cells where windows start, left to right.
    ///
    /// These number the grid's columns for actions and IPC, like the columns of a strip.
    pub fn column_starts(&self) -> Vec<i32> {
        let mut starts: Vec<i32> = self.items.iter().map(|(_, a)| a.col).collect();
        starts.sort_unstable();
        starts.dedup();
        starts
    }

    /// Windows covering column of cells `col`, top to bottom, like the tiles of a column.
    pub fn stack(&self, col: i32) -> Vec<&K> {
        let mut stack: Vec<_> = self
            .items
            .iter()
            .filter(|(_, a)| a.col <= col && col < a.right())
            .collect();
        stack.sort_by_key(|(_, a)| a.row);
        stack.into_iter().map(|(k, _)| k).collect()
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
        self.find_neighbor(id, dir, anchor, false)
    }

    /// Like [`Self::neighbor`], but only finds windows that share rows (for left and right) or
    /// columns (for up and down) with `id`, the way columns in a strip or tiles in a column
    /// are next to each other.
    pub fn adjacent(&self, id: &K, dir: Direction, anchor: Option<i32>) -> Option<&K> {
        self.find_neighbor(id, dir, anchor, true)
    }

    fn find_neighbor(
        &self,
        id: &K,
        dir: Direction,
        anchor: Option<i32>,
        strict: bool,
    ) -> Option<&K> {
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
                } else if strict {
                    return None;
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
        if self.sweep(pinned, dir) {
            // Make room for the windows that went to the right.
            self.sweep(pinned, Direction::Right);
        }
    }

    /// Pushes windows in `dir` so that none overlap, keeping their order along `dir`.
    ///
    /// Going through the windows in order along `dir`, each one moves only as far as it has to
    /// to clear the windows before it that share rows (or columns, going up or down) with it, and
    /// any pinned window. So a window pushes the next one, which pushes the one after it.
    ///
    /// Returns whether a window was pushed off the top or bottom and went right instead.
    fn sweep(&mut self, pinned: &[usize], dir: Direction) -> bool {
        let is_pinned = |i: usize| pinned.contains(&i);
        let mut order: Vec<usize> = (0..self.items.len()).filter(|&i| !is_pinned(i)).collect();
        order.sort_by_key(|&i| (self.along(i, dir), self.items[i].1.cross_span(dir).0, i));

        let shares_lines = |a: &Area, b: &Area| {
            let ((s1, e1), (s2, e2)) = (a.cross_span(dir), b.cross_span(dir));
            s1 < e2 && s2 < e1
        };
        // Whether `a` reaches past where `b` ends along `dir`, and so has to move.
        let behind = |a: &Area, b: &Area| match dir {
            Direction::Right => a.col < b.right(),
            Direction::Left => a.right() > b.col,
            Direction::Down => a.row < b.bottom(),
            Direction::Up => a.bottom() > b.row,
        };
        // `a` moved just past `b` along `dir`.
        let past = |a: Area, b: &Area| match dir {
            Direction::Right => Area {
                col: b.right(),
                ..a
            },
            Direction::Left => Area {
                col: b.col - a.cols,
                ..a
            },
            Direction::Down => Area {
                row: b.bottom(),
                ..a
            },
            Direction::Up => Area {
                row: b.row - a.rows,
                ..a
            },
        };

        let mut placed: Vec<usize> = Vec::new();
        let mut went_right = false;
        for i in order {
            let orig = self.items[i].1;
            let mut area = orig;
            let mut pusher = None;

            // Stay after the windows that came before in these rows or columns. They only ever
            // moved further along `dir`, so this keeps the order.
            for &j in &placed {
                let b = self.items[j].1;
                if shares_lines(&area, &b) && behind(&area, &b) {
                    area = past(area, &b);
                    pusher = Some(j);
                }
            }

            // Clear the pinned windows, which can be anywhere.
            while let Some(&j) = pinned.iter().find(|&&j| area.overlaps(&self.items[j].1)) {
                area = past(area, &self.items[j].1);
                pusher = Some(j);
            }

            if area.row < 0 || area.bottom() > self.rows {
                // Pushed off the top or bottom: go right of the pusher instead, in the same rows.
                let pusher = self.items[pusher.unwrap()].1;
                self.items[i].1 = Area {
                    col: pusher.right(),
                    ..orig
                };
                went_right = true;
                continue;
            }

            self.items[i].1 = area;
            placed.push(i);
        }
        went_right
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

    /// Collapses columns that no window covers between windows, moving the windows right of them
    /// left. With left gravity, slides windows left within their rows instead.
    fn normalize(&mut self) {
        if self.left_gravity {
            self.slide_left();
            return;
        }

        // A window held by the pointer stays where it is until it's released.
        if self.pinned.is_some() {
            return;
        }

        // Positions don't depend on the windows: the leftmost window stays where it is, and so
        // does any space before it.
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
            a.col -= empty_before[rel];
        }
    }

    /// Slides every window left within its rows until it touches another window or column 0.
    ///
    /// This leaves no fully empty columns either, since every window touches something on its
    /// left all the way to column 0.
    ///
    /// A pinned window doesn't slide, but windows to its left can still push it right, the same
    /// as they would push a window held by the pointer.
    fn slide_left(&mut self) {
        let pinned = self.pinned.as_ref().and_then(|id| self.idx(id));

        // Going left to right, every window to the left of the current one is already in its
        // final place. Windows only move left, except the pinned one, which only moves right, but
        // only as far as the windows before it in the order, so the order holds.
        let mut order: Vec<usize> = (0..self.items.len()).collect();
        order.sort_by_key(|&i| (self.items[i].1.col, self.items[i].1.row));

        for (n, &i) in order.iter().enumerate() {
            let area = self.items[i].1;
            let mut col = order[..n]
                .iter()
                .map(|&j| self.items[j].1)
                .filter(|b| b.row < area.bottom() && area.row < b.bottom())
                .map(|b| b.right())
                .max()
                .unwrap_or(0);
            if Some(i) == pinned {
                col = col.max(area.col);
            }
            self.items[i].1.col = col;
        }
    }

    pub fn verify_invariants(&self) {
        for (i, (k, a)) in self.items.iter().enumerate() {
            assert!(a.cols >= 1 && a.rows >= 1, "empty area: {a:?}");
            assert!(
                a.row >= 0 && a.bottom() <= self.rows,
                "area outside rows: {a:?}"
            );
            assert!(
                !self.left_gravity || a.col >= 0,
                "area before column 0 with left gravity: {a:?}"
            );
            assert!(
                self.items[..i].iter().all(|(other, _)| other != k),
                "duplicate window"
            );
            for (_, b) in &self.items[i + 1..] {
                assert!(!a.overlaps(b), "overlapping areas: {a:?} and {b:?}");
            }
        }

        if let Some(pinned) = &self.pinned {
            assert!(
                self.idx(pinned).is_some(),
                "pinned window must be in the grid"
            );
        }

        // No column is empty between the start of the grid with gravity, or the leftmost window
        // without, and the rightmost window. A pinned window can leave empty columns until it's
        // released.
        let start = if self.left_gravity {
            0
        } else {
            self.items.iter().map(|(_, a)| a.col).min().unwrap_or(0)
        };
        for c in start..self.width() {
            assert!(
                self.pinned.is_some()
                    || self.items.iter().any(|(_, a)| a.col <= c && c < a.right()),
                "empty column {c}"
            );
        }

        if self.left_gravity {
            for (k, a) in &self.items {
                if self.pinned.as_ref() == Some(k) {
                    continue;
                }
                let touches = a.col == 0
                    || self.items.iter().any(|(_, b)| {
                        b.right() == a.col && b.row < a.bottom() && a.row < b.bottom()
                    });
                assert!(touches, "window could slide left: {a:?}");
            }
        }
    }
}

/// A way to arrange windows so that they exactly fill a region of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrangement {
    /// Side by side, all equally wide.
    Columns,
    /// A main window on the left taking `num / den` of the width, the others stacked on the
    /// right.
    MainAndStack { num: i32, den: i32 },
    /// Equal columns, about as many as rows of windows in them, filled top to bottom.
    Tiles,
}

impl Arrangement {
    /// Arrangements for `n` windows, in the order the arrange action cycles through them.
    pub fn for_count(n: usize) -> &'static [Arrangement] {
        use Arrangement::*;
        const HALF: Arrangement = MainAndStack { num: 1, den: 2 };
        match n {
            0 | 1 => &[Columns],
            2 => &[Columns, MainAndStack { num: 2, den: 3 }],
            3 => &[HALF, Columns],
            _ => &[Tiles, HALF, Columns],
        }
    }

    /// Whether the first window gets a special place, so it should be the focused one rather
    /// than the first one from the left.
    pub fn has_main(self) -> bool {
        matches!(self, Arrangement::MainAndStack { .. })
    }

    /// Areas for `n` windows filling `cols` × `rows` cells from column `col`, or `None` if they
    /// don't fit.
    pub fn areas(self, n: i32, col: i32, cols: i32, rows: i32) -> Option<Vec<Area>> {
        if n < 1 {
            return None;
        }

        // Windows in vertical stacks, given as the number of columns and windows in each stack.
        let stacks: Vec<(i32, i32)> = match self {
            Arrangement::Columns => {
                let widths = split_evenly(cols, n)?;
                widths.into_iter().map(|w| (w, 1)).collect()
            }
            Arrangement::MainAndStack { .. } if n == 1 => vec![(cols, 1)],
            Arrangement::MainAndStack { num, den } => {
                if cols < 2 {
                    return None;
                }
                let main = (cols * num / den).clamp(1, cols - 1);
                vec![(main, 1), (cols - main, n - 1)]
            }
            Arrangement::Tiles => {
                let k = (1..=n).find(|k| k * k >= n).unwrap();
                let widths = split_evenly(cols, k)?;
                zip(widths, split_evenly(n, k)?).collect()
            }
        };

        let mut areas = Vec::with_capacity(n as usize);
        let mut x = col;
        for (w, count) in stacks {
            let mut y = 0;
            for h in split_evenly(rows, count)? {
                areas.push(Area::new(x, y, w, h));
                y += h;
            }
            x += w;
        }
        Some(areas)
    }
}

/// Splits `total` cells into `n` spans as equal as possible, larger ones first, or `None` if
/// there are fewer cells than spans.
fn split_evenly(total: i32, n: i32) -> Option<Vec<i32>> {
    (0 < n && n <= total).then(|| {
        (0..n)
            .map(|i| total / n + i32::from(i < total % n))
            .collect()
    })
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
    /// possible while fitting a whole, even number of columns in the view width.
    ///
    /// The column count is even so that two half-width windows fit the view exactly.
    pub fn new(view_size: Size<f64, Logical>, rows: i32, gap: f64) -> Self {
        let rows_f = f64::from(rows);
        let cell_h = ((view_size.h - gap * (rows_f + 1.)) / rows_f).max(1.);

        // n columns take n * cell_w + (n + 1) * gap.
        let square_cols = (view_size.w - gap) / (cell_h + gap);
        let n = ((square_cols / 2.).round() * 2.).max(2.);
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

    /// Number of columns that fit in `proportion` of the view width; at least one.
    ///
    /// Rounds down so that windows sized by complementary proportions fit together.
    pub fn cols_for_proportion(&self, proportion: f64) -> i32 {
        span_for_proportion(proportion, self.cols_per_view).clamp(1, MAX_COLS)
    }

    /// Number of rows that fit in `proportion` of the view height; at least one.
    pub fn rows_for_proportion(&self, proportion: f64) -> i32 {
        span_for_proportion(proportion, self.rows).clamp(1, self.rows)
    }
}

/// Rounds down, allowing for proportions written with limited precision like 0.33333.
fn span_for_proportion(proportion: f64, total: i32) -> i32 {
    (proportion * f64::from(total) + 0.01).floor() as i32
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const R: i32 = DEFAULT_ROWS;

    fn grid(items: &[(u32, Area)]) -> Grid<u32> {
        let grid = Grid {
            rows: R,
            left_gravity: false,
            pinned: None,
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

    fn gravity_grid(items: &[(u32, Area)]) -> Grid<u32> {
        let mut grid = grid(items);
        grid.set_left_gravity(true);
        grid.verify_invariants();
        grid
    }

    #[test]
    fn gravity_closes_holes_left_by_closed_windows() {
        // 1 | 2 | 4
        //   | 3 |
        let mut g = gravity_grid(&[
            (1, Area::new(0, 0, 4, R)),
            (2, Area::new(4, 0, 4, 6)),
            (3, Area::new(4, 6, 4, 6)),
            (4, Area::new(8, 0, 4, 6)),
        ]);

        // Closing 2 lets 4 slide left into its rows; 3 keeps its place.
        g.remove(&2);
        g.verify_invariants();
        assert_eq!(area(&g, 3), Area::new(4, 6, 4, 6));
        assert_eq!(area(&g, 4), Area::new(4, 0, 4, 6));
    }

    #[test]
    fn gravity_keeps_holes_that_nothing_fits() {
        // A full-height window next to a half-height one can't slide into half the space.
        let mut g = gravity_grid(&[
            (1, Area::new(0, 0, 4, 6)),
            (2, Area::new(4, 0, 4, R)),
            (3, Area::new(0, 6, 4, 6)),
        ]);
        g.remove(&3);
        g.verify_invariants();
        assert_eq!(area(&g, 2), Area::new(4, 0, 4, R));
    }

    #[test]
    fn gravity_pulls_neighbors_when_shrinking() {
        let mut g = gravity_grid(&[(1, Area::new(0, 0, 6, R)), (2, Area::new(6, 0, 6, R))]);
        assert!(g.resize(&1, 4, R));
        g.verify_invariants();
        assert_eq!(area(&g, 2), Area::new(4, 0, 6, R));
    }

    #[test]
    fn gravity_makes_horizontal_moves_into_empty_space_do_nothing() {
        // 1 | 2
        // 3 |
        let mut g = gravity_grid(&[
            (1, Area::new(0, 0, 4, 6)),
            (3, Area::new(0, 6, 4, 6)),
            (2, Area::new(4, 0, 4, 6)),
        ]);
        assert!(!g.move_window(&2, Direction::Right));

        // Moving 3 right swaps nothing either, and it slides back.
        assert!(!g.move_window(&3, Direction::Right));
    }

    #[test]
    fn pushes_keep_windows_in_order() {
        // 5 | 6 | 2, with 2 full height, and 3 | 4 at the bottom. 2 comes first in the grid's
        // own order, so that order can't decide who goes first.
        let mut g = gravity_grid(&[
            (2, Area::new(12, 0, 8, R)),
            (3, Area::new(0, 8, 5, 4)),
            (4, Area::new(5, 8, 7, 4)),
            (5, Area::new(0, 0, 8, 5)),
            (6, Area::new(8, 0, 4, 5)),
        ]);

        // 7 lands where 6 is, and is wide enough to reach into 2. It pushes 6, which pushes 2.
        g.insert(7, Area::new(8, 1, 12, 3));
        g.verify_invariants();
        assert_eq!(area(&g, 7), Area::new(8, 1, 12, 3));
        assert_eq!(area(&g, 6), Area::new(20, 0, 4, 5));
        assert_eq!(area(&g, 2), Area::new(24, 0, 8, R));
    }

    #[test]
    fn pinned_window_falls_only_when_released() {
        // 1 | 2
        //   | 2
        let mut g = gravity_grid(&[(1, Area::new(0, 0, 4, 4)), (2, Area::new(4, 0, 4, R))]);
        g.set_pinned(Some(2));

        // Shrinking 2 from the top frees rows next to nothing, but it stays put while held.
        let held = Area::new(4, 4, 4, 8);
        assert!(g.set_area(&2, held, Direction::Up));
        g.verify_invariants();
        assert_eq!(area(&g, 2), held);

        // Letting go drops it under 1.
        g.set_pinned(None);
        g.verify_invariants();
        assert_eq!(area(&g, 2), Area::new(0, 4, 4, 8));
        assert_eq!(area(&g, 1), Area::new(0, 0, 4, 4));
    }

    #[test]
    fn windows_slide_up_to_a_pinned_window() {
        let mut g = gravity_grid(&[
            (1, Area::new(0, 0, 4, R)),
            (2, Area::new(4, 0, 4, R)),
            (3, Area::new(8, 0, 4, R)),
        ]);
        g.set_pinned(Some(2));
        assert!(g.resize(&2, 2, R));
        g.verify_invariants();
        assert_eq!(area(&g, 3), Area::new(6, 0, 4, R));
    }

    #[test]
    fn arrangements_fill_the_region() {
        for n in 1..=8 {
            for &arrangement in Arrangement::for_count(n) {
                let n = n as i32;
                let areas = arrangement.areas(n, 3, 22, 12).unwrap();
                assert_eq!(areas.len(), n as usize);
                for (i, a) in areas.iter().enumerate() {
                    assert!(a.col >= 3 && a.right() <= 25 && a.row >= 0 && a.bottom() <= 12);
                    assert!(areas[..i].iter().all(|b| !a.overlaps(b)), "{arrangement:?}");
                }
                let cells: i32 = areas.iter().map(|a| a.cols * a.rows).sum();
                assert_eq!(cells, 22 * 12, "{arrangement:?} with {n} windows");
            }
        }

        let half = Arrangement::MainAndStack { num: 1, den: 2 };
        assert_eq!(
            half.areas(3, 0, 22, 12).unwrap(),
            [
                Area::new(0, 0, 11, 12),
                Area::new(11, 0, 11, 6),
                Area::new(11, 6, 11, 6)
            ]
        );
        assert_eq!(
            Arrangement::Tiles.areas(5, 0, 22, 12).unwrap(),
            [
                Area::new(0, 0, 8, 6),
                Area::new(0, 6, 8, 6),
                Area::new(8, 0, 7, 6),
                Area::new(8, 6, 7, 6),
                Area::new(15, 0, 7, 12),
            ]
        );
    }

    #[test]
    fn metrics_near_square() {
        let m = CellMetrics::new(Size::from((1920., 1080.)), R, 16.);
        assert_eq!(m.cols_per_view, 22);
        assert!((m.cell_w / m.cell_h - 1.).abs() < 0.05);

        // The cells exactly fill the view.
        let full = m.rect(Area::new(0, 0, m.cols_per_view, R));
        assert!((full.loc.x + full.size.w + m.gap - 1920.).abs() < 1e-6);
        assert!((full.loc.y + full.size.h + m.gap - 1080.).abs() < 1e-6);

        assert_eq!(m.rows_for_proportion(0.5), 6);
        assert_eq!(m.rows_for_proportion(1. / 3.), 4);
        assert_eq!(m.rows_for_proportion(0.25), 3);
        assert_eq!(m.rows_for_proportion(0.33333), 4);

        // Complementary proportions fit in the view.
        assert_eq!(m.cols_for_proportion(0.5), 11);
        assert_eq!(m.cols_for_proportion(0.33333), 7);
        assert_eq!(m.cols_for_proportion(0.66667), 14);
    }

    #[derive(Debug, Clone)]
    enum Op {
        Insert { after: usize, cols: i32, rows: i32 },
        Remove(usize),
        Move(usize, Direction),
        Resize(usize, i32, i32),
        SetArea(usize, Area, Direction),
        Pin(Option<usize>),
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
            1 => prop::option::of(any::<usize>()).prop_map(Op::Pin),
        ]
    }

    proptest! {
        #[test]
        fn random_operations_keep_invariants(
            ops in prop::collection::vec(op(), 1..60),
            left_gravity in any::<bool>(),
        ) {
            let mut g = Grid::new(R);
            g.set_left_gravity(left_gravity);
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
                    Op::Pin(i) => {
                        g.set_pinned(i.and_then(pick));
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
