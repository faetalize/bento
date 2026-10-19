use std::cmp::{max, min};
use std::iter::{self, zip};
use std::rc::Rc;
use std::time::Duration;

use niri_config::utils::MergeWith as _;
use niri_config::{CenterFocusedColumn, PresetSize, Struts};
use niri_ipc::{ColumnDisplay, SizeChange, WindowLayout};
use ordered_float::NotNan;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Point, Rectangle, Scale, Serial, Size};

use super::closing_window::{ClosingWindow, ClosingWindowRenderElement};
use super::grid::{self, Area, CellMetrics, Grid};
use super::monitor::InsertPosition;
use super::tab_indicator::{TabIndicator, TabIndicatorRenderElement, TabInfo};
use super::tile::{Tile, TileRenderElement, TileRenderSnapshot};
use super::workspace::{InteractiveResize, ResolvedSize};
use super::{ConfigureIntent, HitType, InteractiveResizeData, LayoutElement, Options, RemovedTile};
use crate::animation::{Animation, Clock};
use crate::input::swipe_tracker::SwipeTracker;
use crate::layout::{RenderLayer, SizingMode};
use crate::niri_render_elements;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::xray::XrayPos;
use crate::render_helpers::RenderCtx;
use crate::utils::id::IdCounter;
use crate::utils::transaction::{Transaction, TransactionBlocker};
use crate::utils::{round_logical_in_physical, ResizeEdge};
use crate::window::ResolvedWindowRules;

/// Amount of touchpad movement to scroll the view for the width of one working area.
const VIEW_GESTURE_WORKING_AREA_MOVEMENT: f64 = 1200.;

/// Whether windows are laid out on bento's cell grid rather than in niri's columns.
///
/// On the grid, every window is its own single-tile column, positioned by [`Grid`]. The niri
/// column code paths stay in place behind this flag to keep upstream merges simple.
const GRID_LAYOUT: bool = true;

/// A scrollable-tiling space for windows.
#[derive(Debug)]
pub struct ScrollingSpace<W: LayoutElement> {
    /// Columns of windows on this space.
    columns: Vec<Column<W>>,

    /// Extra per-column data.
    data: Vec<ColumnData>,

    /// Index of the currently active column, if any.
    active_column_idx: usize,

    /// Ongoing interactive resize.
    interactive_resize: Option<InteractiveResize<W>>,

    /// Offset of the view from x = 0 on the grid, which is the fixed left edge of the grid.
    ///
    /// Unlike niri, where this is relative to the active column, it's absolute, and layout
    /// changes never move it: windows that move slide on screen. The view only scrolls to keep
    /// the focused window in view, and for actions about the view itself, like focusing another
    /// window, gestures, centering and fullscreen.
    ///
    /// Any gaps, including left padding from work area left exclusive zone, is handled
    /// with this view offset (rather than added as a constant elsewhere in the code). This allows
    /// for natural handling of fullscreen windows, which must ignore work area padding.
    view_offset: ViewOffset,

    /// Column to activate, rather than a neighbor, when the active column is removed.
    ///
    /// When a new column is created and removed with no focus changes in-between, it is more
    /// natural to go back to the previously-focused column. This variable tracks that.
    activate_prev_column_on_removal: Option<PrevColumn>,

    /// View offset to restore after unfullscreening or unmaximizing, relative to the column.
    view_offset_to_restore: Option<(ColumnId, f64)>,

    /// Windows in the closing animation.
    closing_windows: Vec<ClosingWindow>,

    /// View size for this space.
    view_size: Size<f64, Logical>,

    /// Working area for this space.
    ///
    /// Takes into account layer-shell exclusive zones and niri struts.
    working_area: Rectangle<f64, Logical>,

    /// Working area for this space excluding struts.
    ///
    /// Used for popup unconstraining. Popups can go over struts, but they shouldn't go over
    /// the layer-shell top layer (which renders on top of popups).
    parent_area: Rectangle<f64, Logical>,

    /// Scale of the output the space is on (and rounds its sizes to).
    scale: f64,

    /// Clock for driving animations.
    clock: Clock,

    /// Configurable properties of the layout.
    options: Rc<Options>,

    /// Cell grid positioning every column.
    grid: Grid<ColumnId>,

    /// Pixel geometry of the cell grid.
    cell_metrics: CellMetrics,

    /// Remembered row and column for directional focus.
    focus_anchor: Option<FocusAnchor>,

    /// Grid as it was when the ongoing interactive resize started.
    ///
    /// Resizing applies to this snapshot, so windows pushed aside come back if the resize shrinks
    /// again.
    grid_resize_original: Option<Grid<ColumnId>>,

    /// Where on the grid a dropped window goes when it's added.
    grid_drop_area: Option<(W::Id, Area)>,
}

/// Size of a tiled window on the grid, as the user or a window rule chose it.
///
/// Spans are resolved on the grid the window is on, so the size carries over to grids with
/// different cells.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GridSize {
    /// Width; `None` follows the column width.
    pub cols: Option<GridSpan>,
    /// Height; `None` takes the full height.
    pub rows: Option<GridSpan>,
}

/// Number of cells a window spans along one axis of the grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridSpan {
    /// `n` cells out of `of` cells across the view, as picked on a grid.
    Cells { n: i32, of: i32 },
    /// A preset size.
    Preset(PresetSize),
}

/// Column to go back to when the column opened from it is removed.
#[derive(Debug, Clone, Copy)]
struct PrevColumn {
    /// Column that was opened.
    opened: ColumnId,
    /// Column that was active before it.
    prev: ColumnId,
}

/// Row and column that directional focus is travelling along.
///
/// Only valid while `column` stays active.
#[derive(Debug, Clone, Copy)]
struct FocusAnchor {
    column: ColumnId,
    /// Row for focusing left and right.
    row: Option<i32>,
    /// Column for focusing up and down.
    col: Option<i32>,
}

niri_render_elements! {
    ScrollingSpaceRenderElement<R> => {
        Tile = TileRenderElement<R>,
        ClosingWindow = ClosingWindowRenderElement,
        TabIndicator = TabIndicatorRenderElement,
    }
}

/// Extra per-column data.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ColumnData {
    /// Cached actual column width.
    width: f64,
}

#[derive(Debug)]
pub(super) enum ViewOffset {
    /// The view offset is static.
    Static(f64),
    /// The view offset is animating.
    Animation(Animation),
    /// The view offset is controlled by the ongoing gesture.
    Gesture(ViewGesture),
}

#[derive(Debug)]
pub(super) struct ViewGesture {
    current_view_offset: f64,
    /// Animation for the extra offset to the current position.
    ///
    /// For example, when we need to activate a specific window during a DnD scroll.
    animation: Option<Animation>,
    tracker: SwipeTracker,
    delta_from_tracker: f64,
    // The view offset we'll use if needed for activate_prev_column_on_removal.
    stationary_view_offset: f64,
    /// Whether the gesture is controlled by the touchpad.
    is_touchpad: bool,

    // If this gesture is for drag-and-drop scrolling, this is the last event's unadjusted
    // timestamp.
    dnd_last_event_time: Option<Duration>,
    // Time when the drag-and-drop scroll delta became non-zero, used for debouncing.
    //
    // If `None` then the scroll delta is currently zero.
    dnd_nonzero_start_time: Option<Duration>,
}

static COLUMN_ID_COUNTER: IdCounter = IdCounter::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColumnId(u64);

impl ColumnId {
    fn next() -> ColumnId {
        ColumnId(COLUMN_ID_COUNTER.next())
    }

    pub fn get(self) -> u64 {
        self.0
    }

    pub fn specific(id: u64) -> Self {
        Self(id)
    }
}

#[derive(Debug)]
pub struct Column<W: LayoutElement> {
    /// Tiles in this column.
    ///
    /// Must be non-empty.
    tiles: Vec<Tile<W>>,

    /// Extra per-tile data.
    ///
    /// Must have the same number of elements as `tiles`.
    data: Vec<TileData>,

    /// Index of the currently active tile.
    active_tile_idx: usize,

    /// Desired width of this column.
    ///
    /// If the column is full-width or full-screened, this is the width that should be restored
    /// upon unfullscreening and untoggling full-width.
    width: ColumnWidth,

    /// Currently selected preset width index.
    preset_width_idx: Option<usize>,

    /// Whether this column is full-width.
    is_full_width: bool,

    /// Whether this column is going to be fullscreen.
    ///
    /// This is the compositor-side fullscreen state, so it changes immediately upon
    /// set_fullscreen(). The actual tiles will take some time to respond to the fullscreen request
    /// and become fullscreen.
    ///
    /// Similarly, unsetting fullscreen will change this value to false immediately, and tiles will
    /// take some time to catch up and actually unfullscreen.
    is_pending_fullscreen: bool,

    /// Whether this column is going to be maximized.
    ///
    /// Can be `true` together with `is_pending_fullscreen`, which means that the column is
    /// effectively pending fullscreen, but unfullscreening should go back to maximized state,
    /// rather than normal.
    is_pending_maximized: bool,

    /// How this column displays and arranges windows.
    display_mode: ColumnDisplay,

    /// Tab indicator for the tabbed display mode.
    tab_indicator: TabIndicator,

    /// Animation of the render offset during window swapping.
    move_x_animation: Option<MoveAnimation>,

    /// Animation of a column visually moving vertically.
    move_y_animation: Option<MoveAnimation>,

    /// Latest known view size for this column's workspace.
    view_size: Size<f64, Logical>,

    /// Latest known working area for this column's workspace.
    working_area: Rectangle<f64, Logical>,

    /// Working area for this column's workspace excluding struts.
    ///
    /// Used for maximize-to-edges.
    parent_area: Rectangle<f64, Logical>,

    /// Scale of the output the column is on (and rounds its sizes to).
    scale: f64,

    /// Clock for driving animations.
    clock: Clock,

    /// Configurable properties of the layout.
    options: Rc<Options>,

    /// Unique ID of this column.
    id: ColumnId,

    /// Position and size of this column's tile on the cell grid, set by the scrolling space.
    ///
    /// When set, this decides the tile size and vertical position in the normal sizing mode.
    grid_geometry: Option<GridGeometry>,

    /// Size of this column on the cell grid in (columns, rows).
    grid_span: Option<(i32, i32)>,

    /// Row to go back to on the grid when the column stops being fullscreen or maximized.
    grid_row_to_restore: Option<i32>,
}

/// Where a column's tile sits on the cell grid.
#[derive(Debug, Clone, Copy, PartialEq)]
struct GridGeometry {
    /// Y of the tile relative to the top of the working area.
    y: f64,
    /// Size of the tile.
    size: Size<f64, Logical>,
}

/// Extra per-tile data.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TileData {
    /// Requested height of the window.
    ///
    /// This is window height, not tile height, so it excludes tile decorations.
    height: WindowHeight,

    /// Cached actual size of the tile.
    size: Size<f64, Logical>,

    /// Cached whether the tile is being interactively resized by its left edge.
    interactively_resizing_by_left_edge: bool,
}

/// Width of a column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColumnWidth {
    /// Proportion of the current view width.
    Proportion(f64),
    /// Fixed width in logical pixels.
    Fixed(f64),
}

/// Height of a window in a column.
///
/// Every window but one in a column must be `Auto`-sized so that the total height can add up to
/// the workspace height. Resizing a window converts all other windows to `Auto`, weighted to
/// preserve their visual heights at the moment of the conversion.
///
/// In contrast to column widths, proportional height changes are converted to, and stored as,
/// fixed height right away. With column widths you frequently want e.g. two columns side-by-side
/// with 50% width each, and you want them to remain this way when moving to a differently sized
/// monitor. Windows in a column, however, already auto-size to fill the available height, giving
/// you this behavior. The main reason to set a different window height, then, is when you want
/// something in the window to fit exactly, e.g. to fit 30 lines in a terminal, which corresponds
/// to the `Fixed` variant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowHeight {
    /// Automatically computed *tile* height, distributed across the column according to weights.
    ///
    /// This controls the tile height rather than the window height because it's easier in the auto
    /// height distribution algorithm.
    Auto { weight: f64 },
    /// Fixed *window* height in logical pixels.
    Fixed(f64),
    /// One of the preset heights (tile or window).
    Preset(usize),
}

/// Horizontal direction for an operation.
///
/// As operations often have a symmetrical counterpart, e.g. focus-right/focus-left, methods
/// on `Scrolling` can sometimes be factored using the direction of the operation as a parameter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollDirection {
    Left,
    Right,
}

#[derive(Debug)]
struct MoveAnimation {
    anim: Animation,
    from: f64,
    /// Whether this animation is for moving the tile between workspaces.
    ///
    /// Controls whether the tile is rendered uncropped and above others.
    is_between_workspaces: bool,
}

impl<W: LayoutElement> ScrollingSpace<W> {
    pub fn new(
        view_size: Size<f64, Logical>,
        parent_area: Rectangle<f64, Logical>,
        scale: f64,
        clock: Clock,
        options: Rc<Options>,
    ) -> Self {
        let working_area = compute_working_area(parent_area, scale, options.layout.struts);
        let cell_metrics = compute_cell_metrics(working_area, &options);
        let mut grid = Grid::new(grid::DEFAULT_ROWS);
        grid.set_left_gravity(options.layout.left_gravity);

        Self {
            columns: Vec::new(),
            data: Vec::new(),
            active_column_idx: 0,
            interactive_resize: None,
            view_offset: ViewOffset::Static(0.),
            activate_prev_column_on_removal: None,
            view_offset_to_restore: None,
            closing_windows: Vec::new(),
            view_size,
            working_area,
            parent_area,
            scale,
            clock,
            options,
            grid,
            cell_metrics,
            focus_anchor: None,
            grid_resize_original: None,
            grid_drop_area: None,
        }
    }

    pub fn update_config(
        &mut self,
        view_size: Size<f64, Logical>,
        parent_area: Rectangle<f64, Logical>,
        scale: f64,
        options: Rc<Options>,
    ) {
        let working_area = compute_working_area(parent_area, scale, options.layout.struts);

        // Cell size changes move every window, so keep the view on the active one.
        let active_x = (!self.columns.is_empty()).then(|| self.column_x(self.active_column_idx));
        let prev_cols_per_view = self.cell_metrics.cols_per_view;

        for (column, data) in zip(&mut self.columns, &mut self.data) {
            column.update_config(view_size, working_area, parent_area, scale, options.clone());
            data.update(column);
        }

        self.view_size = view_size;
        self.working_area = working_area;
        self.parent_area = parent_area;
        self.scale = scale;
        self.cell_metrics = compute_cell_metrics(working_area, &options);
        self.options = options;

        // With different cells, windows resolve their sizes to different spans, like a
        // proportional width with more cells across the view.
        let spans: Vec<_> = self
            .columns
            .iter()
            .map(|col| (col.id, self.grid_span_in_mode(col)))
            .collect();
        let resized = spans.iter().any(|(id, (cols, rows))| {
            self.grid
                .area(id)
                .is_some_and(|area| (area.cols, area.rows) != (*cols, *rows))
        });
        let cols_per_view = self.cell_metrics.cols_per_view;
        if resized || cols_per_view != prev_cols_per_view {
            let ratio = f64::from(cols_per_view) / f64::from(prev_cols_per_view);
            self.grid.rescale(ratio, |id| {
                spans.iter().find(|(other, _)| other == id).unwrap().1
            });
        }

        if self.grid.left_gravity() != self.options.layout.left_gravity {
            let prev_positions = self.grid_positions();
            self.grid.set_left_gravity(self.options.layout.left_gravity);
            self.sync_grid(Some(prev_positions), None);
        } else {
            self.sync_grid(None, None);
            if let Some(active_x) = active_x {
                self.view_offset
                    .offset(self.column_x(self.active_column_idx) - active_x);
            }
        }

        // Apply always-center and such right away.
        if !self.columns.is_empty() && !self.view_offset.is_gesture() {
            self.animate_view_offset_to_column(None, self.active_column_idx, None);
        }
    }

    pub fn update_shaders(&mut self) {
        for col in &mut self.columns {
            col.update_shaders();
        }
    }

    pub fn advance_animations(&mut self) {
        if let ViewOffset::Animation(anim) = &self.view_offset {
            if anim.is_done() {
                self.view_offset = ViewOffset::Static(anim.to());
            }
        }

        if let ViewOffset::Gesture(gesture) = &mut self.view_offset {
            // Make sure the last event time doesn't go too much out of date (for
            // workspaces not under cursor), causing sudden jumps.
            //
            // This happens after any dnd_scroll_gesture_scroll() calls (in
            // Layout::advance_animations()), so it doesn't mess up the time delta there.
            if let Some(last_time) = &mut gesture.dnd_last_event_time {
                let now = self.clock.now_unadjusted();
                if *last_time != now {
                    *last_time = now;

                    // If last_time was already == now, then dnd_scroll_gesture_scroll() must've
                    // updated the gesture already. Therefore, when this code runs, the pointer
                    // must be outside the DnD scrolling zone.
                    gesture.dnd_nonzero_start_time = None;
                }
            }

            if let Some(anim) = &mut gesture.animation {
                if anim.is_done() {
                    gesture.animation = None;
                }
            }
        }

        for col in &mut self.columns {
            col.advance_animations();
        }

        self.closing_windows.retain_mut(|closing| {
            closing.advance_animations();
            closing.are_animations_ongoing()
        });
    }

    pub fn are_animations_ongoing(&self) -> bool {
        self.view_offset.is_animation_ongoing()
            || self.columns.iter().any(Column::are_animations_ongoing)
            || !self.closing_windows.is_empty()
    }

    pub fn are_transitions_ongoing(&self) -> bool {
        !self.view_offset.is_static()
            || self.columns.iter().any(Column::are_transitions_ongoing)
            || !self.closing_windows.is_empty()
    }

    pub fn update_render_elements(&mut self, is_active: bool, layer: RenderLayer) {
        let view_pos = Point::from((self.view_pos(), 0.));
        let view_size = self.view_size;
        let active_idx = self.active_column_idx;
        for (col_idx, (col, col_x)) in self.columns_mut().enumerate() {
            // Skip columns belonging to a different render layer.
            if layer.is_normal() == col.is_moving_between_workspaces() {
                continue;
            }

            let is_active = is_active && col_idx == active_idx;
            let col_off = Point::from((col_x, 0.));
            let col_pos = view_pos - col_off - col.render_offset();
            let view_rect = Rectangle::new(col_pos, view_size);
            col.update_render_elements(is_active, view_rect);
        }
    }

    pub fn tiles(&self) -> impl Iterator<Item = &Tile<W>> + '_ {
        self.columns.iter().flat_map(|col| col.tiles.iter())
    }

    pub fn tiles_mut(&mut self) -> impl Iterator<Item = &mut Tile<W>> + '_ {
        self.columns.iter_mut().flat_map(|col| col.tiles.iter_mut())
    }

    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    pub fn active_window(&self) -> Option<&W> {
        if self.columns.is_empty() {
            return None;
        }

        let col = &self.columns[self.active_column_idx];
        Some(col.tiles[col.active_tile_idx].window())
    }

    pub fn active_window_mut(&mut self) -> Option<&mut W> {
        if self.columns.is_empty() {
            return None;
        }

        let col = &mut self.columns[self.active_column_idx];
        Some(col.tiles[col.active_tile_idx].window_mut())
    }

    pub fn active_tile_mut(&mut self) -> Option<&mut Tile<W>> {
        if self.columns.is_empty() {
            return None;
        }

        let col = &mut self.columns[self.active_column_idx];
        Some(&mut col.tiles[col.active_tile_idx])
    }

    pub fn is_active_pending_fullscreen(&self) -> bool {
        if self.columns.is_empty() {
            return false;
        }

        let col = &self.columns[self.active_column_idx];
        col.pending_sizing_mode().is_fullscreen()
    }

    pub fn new_window_toplevel_bounds(&self, rules: &ResolvedWindowRules) -> Size<i32, Logical> {
        let border_config = self.options.layout.border.merged_with(&rules.border);

        let display_mode = rules
            .default_column_display
            .unwrap_or(self.options.layout.default_column_display);
        let will_tab = display_mode == ColumnDisplay::Tabbed;
        let extra_size = if will_tab {
            TabIndicator::new(self.options.layout.tab_indicator).extra_size(1, self.scale)
        } else {
            Size::from((0., 0.))
        };

        compute_toplevel_bounds(
            border_config,
            self.working_area.size,
            extra_size,
            self.options.layout.gaps,
        )
    }

    pub fn new_window_size(
        &self,
        width: Option<PresetSize>,
        height: Option<PresetSize>,
        rules: &ResolvedWindowRules,
    ) -> Size<i32, Logical> {
        let border = self.options.layout.border.merged_with(&rules.border);

        let display_mode = rules
            .default_column_display
            .unwrap_or(self.options.layout.default_column_display);
        let will_tab = display_mode == ColumnDisplay::Tabbed;
        let extra = if will_tab {
            TabIndicator::new(self.options.layout.tab_indicator).extra_size(1, self.scale)
        } else {
            Size::from((0., 0.))
        };

        if GRID_LAYOUT {
            // Open windows at the size of the grid cells they will occupy.
            let borders = if border.off { 0. } else { border.width * 2. };
            let width = width.map_or(0, |preset| {
                let cols = self.grid_cols_for_preset(preset, borders);
                let w = self.grid_geometry(Area::new(0, 0, cols, 1)).size.w - borders - extra.w;
                max(1, w.floor() as i32)
            });
            let rows = height.map_or(self.cell_metrics.rows, |preset| {
                self.grid_rows_for_preset(preset, borders)
            });
            let h = self.grid_geometry(Area::new(0, 0, 1, rows)).size.h - borders - extra.h;
            return Size::from((width, max(h.floor() as i32, 1)));
        }

        let working_size = self.working_area.size;

        let width = if let Some(size) = width {
            let size = match resolve_preset_size(size, &self.options, working_size.w, extra.w) {
                ResolvedSize::Tile(mut size) => {
                    if !border.off {
                        size -= border.width * 2.;
                    }
                    size
                }
                ResolvedSize::Window(size) => size,
            };

            max(1, size.floor() as i32)
        } else {
            0
        };

        let mut full_height = self.working_area.size.h - self.options.layout.gaps * 2.;
        if !border.off {
            full_height -= border.width * 2.;
        }

        let height = if let Some(height) = height {
            let height = match resolve_preset_size(height, &self.options, working_size.h, extra.h) {
                ResolvedSize::Tile(mut size) => {
                    if !border.off {
                        size -= border.width * 2.;
                    }
                    size
                }
                ResolvedSize::Window(size) => size,
            };
            f64::min(height, full_height)
        } else {
            full_height
        };

        Size::from((width, max(height.floor() as i32, 1)))
    }

    pub fn is_centering_focused_column(&self) -> bool {
        self.options.layout.center_focused_column == CenterFocusedColumn::Always
            || (self.options.layout.always_center_single_column && self.is_single_grid_column())
    }

    /// Whether no two windows are side by side, like a single column in niri.
    ///
    /// Windows stacked on top of each other count as a single column even if their widths
    /// differ, as long as some column of cells runs through all of them.
    fn is_single_grid_column(&self) -> bool {
        let left = self.grid.iter().map(|(_, area)| area.col).max();
        let right = self.grid.iter().map(|(_, area)| area.right()).min();
        left.zip(right).is_none_or(|(left, right)| left < right)
    }

    fn compute_new_view_offset_fit(
        &self,
        target_x: Option<f64>,
        col_x: f64,
        width: f64,
        mode: SizingMode,
    ) -> f64 {
        if mode.is_fullscreen() {
            return 0.;
        }

        let (area, padding) = if mode.is_maximized() {
            (self.parent_area, 0.)
        } else {
            (self.working_area, self.options.layout.gaps)
        };

        let target_x = target_x.unwrap_or_else(|| self.target_view_pos());

        let new_offset =
            compute_new_view_offset(target_x + area.loc.x, area.size.w, col_x, width, padding);

        // Non-fullscreen windows are always offset at least by the working area position.
        new_offset - area.loc.x
    }

    fn compute_new_view_offset_centered(
        &self,
        target_x: Option<f64>,
        col_x: f64,
        width: f64,
        mode: SizingMode,
    ) -> f64 {
        if mode.is_fullscreen() {
            return self.compute_new_view_offset_fit(target_x, col_x, width, mode);
        }

        let area = if mode.is_maximized() {
            self.parent_area
        } else {
            self.working_area
        };

        // Columns wider than the view are left-aligned (the fit code can deal with that).
        if area.size.w <= width {
            return self.compute_new_view_offset_fit(target_x, col_x, width, mode);
        }

        -(area.size.w - width) / 2. - area.loc.x
    }

    fn compute_new_view_offset_for_column_fit(&self, target_x: Option<f64>, idx: usize) -> f64 {
        let col = &self.columns[idx];
        self.compute_new_view_offset_fit(
            target_x,
            self.column_x(idx),
            col.width(),
            col.sizing_mode(),
        )
    }

    fn compute_new_view_offset_for_column_centered(
        &self,
        target_x: Option<f64>,
        idx: usize,
    ) -> f64 {
        let col = &self.columns[idx];
        self.compute_new_view_offset_centered(
            target_x,
            self.column_x(idx),
            col.width(),
            col.sizing_mode(),
        )
    }

    fn compute_new_view_offset_for_column(
        &self,
        target_x: Option<f64>,
        idx: usize,
        prev_idx: Option<usize>,
    ) -> f64 {
        if self.is_centering_focused_column() {
            return self.compute_new_view_offset_for_column_centered(target_x, idx);
        }

        match self.options.layout.center_focused_column {
            CenterFocusedColumn::Always => {
                self.compute_new_view_offset_for_column_centered(target_x, idx)
            }
            CenterFocusedColumn::OnOverflow => {
                let Some(prev_idx) = prev_idx else {
                    return self.compute_new_view_offset_for_column_fit(target_x, idx);
                };

                // Activating the same column.
                if prev_idx == idx {
                    return self.compute_new_view_offset_for_column_fit(target_x, idx);
                }

                // Always take the window next to the target on the side we're coming from as the
                // source. Moving up or down within the same columns of cells doesn't need any
                // centering.
                let target_col_x = self.column_x(idx);
                let target_col_width = self.columns[idx].width();
                let target_right = target_col_x + target_col_width;

                let prev_col_x = self.column_x(prev_idx);
                let prev_right = prev_col_x + self.columns[prev_idx].width();
                let dir = if target_right <= prev_col_x {
                    grid::Direction::Right
                } else if prev_right <= target_col_x {
                    grid::Direction::Left
                } else {
                    return self.compute_new_view_offset_for_column_fit(target_x, idx);
                };

                let target_id = self.columns[idx].id;
                let source_idx = self
                    .grid
                    .adjacent(&target_id, dir, None)
                    .and_then(|id| self.columns.iter().position(|col| col.id == *id));
                let Some(source_idx) = source_idx else {
                    return self.compute_new_view_offset_for_column_fit(target_x, idx);
                };

                let source_col_x = self.column_x(source_idx);
                let source_right = source_col_x + self.columns[source_idx].width();

                // NOTE: This logic won't work entirely correctly with small fixed-size maximized
                // windows (they have a different area and padding).
                let total_width = f64::max(target_right, source_right)
                    - f64::min(target_col_x, source_col_x)
                    + self.options.layout.gaps * 2.;

                // If it fits together, do a normal animation, otherwise center the new column.
                if total_width <= self.working_area.size.w {
                    self.compute_new_view_offset_for_column_fit(target_x, idx)
                } else {
                    self.compute_new_view_offset_for_column_centered(target_x, idx)
                }
            }
            CenterFocusedColumn::Never => {
                self.compute_new_view_offset_for_column_fit(target_x, idx)
            }
        }
    }

    fn animate_view_offset(&mut self, idx: usize, new_view_offset: f64) {
        self.animate_view_offset_with_config(
            idx,
            new_view_offset,
            self.options.animations.horizontal_view_movement.0,
        );
    }

    fn animate_view_offset_with_config(
        &mut self,
        idx: usize,
        new_view_offset: f64,
        config: niri_config::Animation,
    ) {
        // The view offset is absolute, while new_view_offset is relative to the column.
        let new_view_offset = self.column_x(idx) + new_view_offset;

        let pixel = 1. / self.scale;

        // If our view offset is already this or animating towards this, we don't need to do
        // anything.
        let to_diff = new_view_offset - self.view_offset.target();
        if to_diff.abs() < pixel {
            // Correct for any inaccuracy.
            self.view_offset.offset(to_diff);
            return;
        }

        match &mut self.view_offset {
            ViewOffset::Gesture(gesture) if gesture.dnd_last_event_time.is_some() => {
                gesture.stationary_view_offset = new_view_offset;

                let current_pos = gesture.current_view_offset - gesture.delta_from_tracker;
                gesture.delta_from_tracker = new_view_offset - current_pos;
                let offset_delta = new_view_offset - gesture.current_view_offset;
                gesture.current_view_offset = new_view_offset;

                gesture.animate_from(-offset_delta, self.clock.clone(), config);
            }
            _ => {
                // FIXME: also compute and use current velocity.
                self.view_offset = ViewOffset::Animation(Animation::new(
                    self.clock.clone(),
                    self.view_offset.current(),
                    new_view_offset,
                    0.,
                    config,
                ));
            }
        }
    }

    fn animate_view_offset_to_column_centered(
        &mut self,
        target_x: Option<f64>,
        idx: usize,
        config: niri_config::Animation,
    ) {
        let new_view_offset = self.compute_new_view_offset_for_column_centered(target_x, idx);
        self.animate_view_offset_with_config(idx, new_view_offset, config);
    }

    fn animate_view_offset_to_column_with_config(
        &mut self,
        target_x: Option<f64>,
        idx: usize,
        prev_idx: Option<usize>,
        config: niri_config::Animation,
    ) {
        let new_view_offset = self.compute_new_view_offset_for_column(target_x, idx, prev_idx);
        self.animate_view_offset_with_config(idx, new_view_offset, config);
    }

    fn animate_view_offset_to_column(
        &mut self,
        target_x: Option<f64>,
        idx: usize,
        prev_idx: Option<usize>,
    ) {
        self.animate_view_offset_to_column_with_config(
            target_x,
            idx,
            prev_idx,
            self.options.animations.horizontal_view_movement.0,
        )
    }

    fn activate_column(&mut self, idx: usize) {
        self.activate_column_with_anim_config(
            idx,
            self.options.animations.horizontal_view_movement.0,
        );
    }

    fn activate_column_with_anim_config(&mut self, idx: usize, config: niri_config::Animation) {
        if self.active_column_idx == idx
            // During a DnD scroll, animate even when activating the same window, for DnD hold.
            && (self.columns.is_empty() || !self.view_offset.is_dnd_scroll())
        {
            return;
        }

        self.animate_view_offset_to_column_with_config(
            None,
            idx,
            Some(self.active_column_idx),
            config,
        );

        if self.active_column_idx != idx {
            self.active_column_idx = idx;

            // A different column was activated; reset the flag.
            self.activate_prev_column_on_removal = None;
            self.view_offset_to_restore = None;
            self.interactive_resize = None;
        }
    }

    pub(super) fn insert_position(
        &self,
        pos: Point<f64, Logical>,
        tile_size: Size<f64, Logical>,
        pointer_ratio: (f64, f64),
    ) -> InsertPosition {
        if GRID_LAYOUT {
            return self.grid_insert_position(pos, tile_size, pointer_ratio);
        }

        if self.columns.is_empty() {
            return InsertPosition::NewColumn(0);
        }

        let x = pos.x + self.view_pos();

        // Aim for the center of the gap.
        let x = x + self.options.layout.gaps / 2.;
        let y = pos.y + self.options.layout.gaps / 2.;

        // Insert position is before the first column.
        if x < 0. {
            return InsertPosition::NewColumn(0);
        }

        // Find the closest gap between columns.
        let (closest_col_idx, col_x) = self
            .column_xs(self.data.iter().copied())
            .enumerate()
            .min_by_key(|(_, col_x)| NotNan::new((col_x - x).abs()).unwrap())
            .unwrap();

        // Find the column containing the position.
        let (col_idx, _) = self
            .column_xs(self.data.iter().copied())
            .enumerate()
            .take_while(|(_, col_x)| *col_x <= x)
            .last()
            .unwrap_or((0, 0.));

        // Insert position is past the last column.
        if col_idx == self.columns.len() {
            return InsertPosition::NewColumn(closest_col_idx);
        }

        // Find the closest gap between tiles.
        let col = &self.columns[col_idx];

        let (closest_tile_idx, tile_y) = if col.display_mode == ColumnDisplay::Tabbed {
            // In tabbed mode, there's only one tile visible, and we want to check its top and
            // bottom.
            let top = col.tile_offsets().nth(col.active_tile_idx).unwrap().y;
            let bottom = top + col.data[col.active_tile_idx].size.h;
            if (top - y).abs() <= (bottom - y).abs() {
                (col.active_tile_idx, top)
            } else {
                (col.active_tile_idx + 1, bottom)
            }
        } else {
            col.tile_offsets()
                .map(|tile_off| tile_off.y)
                .enumerate()
                .min_by_key(|(_, tile_y)| NotNan::new((tile_y - y).abs()).unwrap())
                .unwrap()
        };

        // Return the closest among the vertical and the horizontal gap.
        let vert_dist = (col_x - x).abs();
        let hor_dist = (tile_y - y).abs();
        if vert_dist <= hor_dist {
            InsertPosition::NewColumn(closest_col_idx)
        } else {
            InsertPosition::InColumn(col_idx, closest_tile_idx)
        }
    }

    pub fn add_tile(
        &mut self,
        col_idx: Option<usize>,
        tile: Tile<W>,
        activate: bool,
        width: ColumnWidth,
        is_full_width: bool,
        anim: Option<niri_config::Animation>,
    ) {
        let column = Column::new_with_tile(
            tile,
            self.view_size,
            self.working_area,
            self.parent_area,
            self.scale,
            width,
            is_full_width,
        );

        self.add_column(col_idx, column, activate, anim);
    }

    pub fn add_tile_to_column(
        &mut self,
        col_idx: usize,
        tile_idx: Option<usize>,
        tile: Tile<W>,
        activate: bool,
    ) {
        if GRID_LAYOUT {
            let _ = tile_idx;
            let width = self.columns[col_idx].width;
            self.add_tile(Some(col_idx + 1), tile, activate, width, false, None);
            return;
        }

        let prev_next_x = self.column_x(col_idx + 1);

        let target_column = &mut self.columns[col_idx];
        let tile_idx = tile_idx.unwrap_or(target_column.tiles.len());
        let mut prev_active_tile_idx = target_column.active_tile_idx;

        target_column.add_tile_at(tile_idx, tile);
        self.data[col_idx].update(target_column);

        if tile_idx <= prev_active_tile_idx {
            target_column.active_tile_idx += 1;
            prev_active_tile_idx += 1;
        }

        if activate {
            target_column.activate_idx(tile_idx);
            if self.active_column_idx != col_idx {
                self.activate_column(col_idx);
            }
        }

        let target_column = &mut self.columns[col_idx];
        if target_column.display_mode == ColumnDisplay::Tabbed {
            if target_column.active_tile_idx == tile_idx {
                // Fade out the previously active tile.
                let tile = &mut target_column.tiles[prev_active_tile_idx];
                tile.animate_alpha(1., 0., self.options.animations.window_movement.0);
            } else {
                // Fade out when adding into a tabbed column into the background.
                let tile = &mut target_column.tiles[tile_idx];
                tile.animate_alpha(1., 0., self.options.animations.window_movement.0);
            }
        }

        // Adding a wider window into a column increases its width now (even if the window will
        // shrink later). Move the columns to account for this.
        let offset = self.column_x(col_idx + 1) - prev_next_x;
        if self.active_column_idx <= col_idx {
            for col in &mut self.columns[col_idx + 1..] {
                col.animate_move_x_from(-offset);
            }
        } else {
            for col in &mut self.columns[..=col_idx] {
                col.animate_move_x_from(offset);
            }
        }
    }

    pub fn add_tile_right_of(
        &mut self,
        right_of: &W::Id,
        tile: Tile<W>,
        activate: bool,
        width: ColumnWidth,
        is_full_width: bool,
        anim: Option<niri_config::Animation>,
    ) {
        let right_of_idx = self
            .columns
            .iter()
            .position(|col| col.contains(right_of))
            .unwrap();
        let col_idx = right_of_idx + 1;

        self.add_tile(Some(col_idx), tile, activate, width, is_full_width, anim);
    }

    pub fn add_column(
        &mut self,
        idx: Option<usize>,
        mut column: Column<W>,
        activate: bool,
        anim_config: Option<niri_config::Animation>,
    ) {
        let was_empty = self.columns.is_empty();

        let idx = idx.unwrap_or_else(|| {
            if was_empty {
                0
            } else {
                self.active_column_idx + 1
            }
        });

        column.update_config(
            self.view_size,
            self.working_area,
            self.parent_area,
            self.scale,
            self.options.clone(),
        );

        // Place the column on the grid right of the column before it, pushing others aside.
        let prev_positions = self.grid_positions();
        let (cols, rows) = self.grid_span_for(&column);
        let id = column.id;
        let drop_area = self
            .grid_drop_area
            .take()
            .filter(|(window, _)| column.contains(window));
        if let Some((_, area)) = drop_area {
            self.grid.insert(id, area);
        } else {
            match idx.checked_sub(1).and_then(|i| self.columns.get(i)) {
                Some(after) => self.grid.insert_after(id, Some(&after.id), cols, rows),
                None => self.grid.insert(id, Area::new(0, 0, cols, rows)),
            }
        }

        self.data.insert(idx, ColumnData::new(&column));
        self.columns.insert(idx, column);

        if !was_empty && idx <= self.active_column_idx {
            self.active_column_idx += 1;
        }

        // Animate movement of other columns.
        self.sync_grid(Some(prev_positions), anim_config);
        let mut idx = self.columns.iter().position(|col| col.id == id).unwrap();

        // A window that is already fullscreen or maximized pushes the others aside right away.
        if !self.columns[idx].pending_sizing_mode().is_normal() {
            self.fit_grid_to_sizing_mode(idx);
            idx = self.columns.iter().position(|col| col.id == id).unwrap();
        }

        if activate {
            // If this is the first window on an empty workspace, remove the effect of whatever
            // view_offset was left over and skip the animation.
            if was_empty {
                let col_x = self.column_x(idx);
                self.view_offset = ViewOffset::Static(col_x);
                let offset = self.compute_new_view_offset_for_column(None, idx, None);
                self.view_offset = ViewOffset::Static(col_x + offset);
            }

            // Gravity can put the new column anywhere in the order, so remember the previous one
            // by id.
            let prev = (!was_empty).then(|| PrevColumn {
                opened: id,
                prev: self.columns[self.active_column_idx].id,
            });

            let anim_config =
                anim_config.unwrap_or(self.options.animations.horizontal_view_movement.0);
            self.activate_column_with_anim_config(idx, anim_config);
            self.activate_prev_column_on_removal = prev;
        }
    }

    pub fn remove_tile(&mut self, window: &W::Id, transaction: Transaction) -> RemovedTile<W> {
        let column_idx = self
            .columns
            .iter()
            .position(|col| col.contains(window))
            .unwrap();
        let column = &self.columns[column_idx];

        let tile_idx = column.position(window).unwrap();
        self.remove_tile_by_idx(column_idx, tile_idx, transaction, None)
    }

    pub fn remove_tile_by_idx(
        &mut self,
        column_idx: usize,
        tile_idx: usize,
        transaction: Transaction,
        anim_config: Option<niri_config::Animation>,
    ) -> RemovedTile<W> {
        // If this is the only tile in the column, remove the whole column.
        if self.columns[column_idx].tiles.len() == 1 {
            let mut column = self.remove_column_by_idx(column_idx, anim_config);
            return RemovedTile {
                tile: column.tiles.remove(tile_idx),
                width: column.width,
                is_full_width: column.is_full_width,
                is_floating: false,
            };
        }

        let column = &mut self.columns[column_idx];
        let prev_width = self.data[column_idx].width;

        let movement_config = anim_config.unwrap_or(self.options.animations.window_movement.0);

        // Animate movement of other tiles.
        // FIXME: tiles can move by X too, in a centered or resizing layout with one window smaller
        // than the others.
        let offset_y = column.tile_offset(tile_idx + 1).y - column.tile_offset(tile_idx).y;
        for tile in &mut column.tiles[tile_idx + 1..] {
            tile.animate_move_y_from(offset_y);
        }

        if column.display_mode == ColumnDisplay::Tabbed && tile_idx != column.active_tile_idx {
            // Fade in when removing background tab from a tabbed column.
            let tile = &mut column.tiles[tile_idx];
            tile.animate_alpha(0., 1., movement_config);
        }

        let was_normal = column.sizing_mode().is_normal();

        let tile = column.tiles.remove(tile_idx);
        column.data.remove(tile_idx);

        // If an active column became non-fullscreen after removing the tile, clear the stored
        // unfullscreen offset.
        if column_idx == self.active_column_idx && !was_normal && column.sizing_mode().is_normal() {
            self.view_offset_to_restore = None;
        }

        // If one window is left, reset its weight to 1.
        if column.data.len() == 1 {
            if let WindowHeight::Auto { weight } = &mut column.data[0].height {
                *weight = 1.;
            }
        }

        // Stop interactive resize.
        if let Some(resize) = &self.interactive_resize {
            if tile.window().id() == &resize.window {
                self.interactive_resize = None;
            }
        }

        let tile = RemovedTile {
            tile,
            width: column.width,
            is_full_width: column.is_full_width,
            is_floating: false,
        };

        #[allow(clippy::comparison_chain)] // What do you even want here?
        if tile_idx < column.active_tile_idx {
            // A tile above was removed; preserve the current position.
            column.active_tile_idx -= 1;
        } else if tile_idx == column.active_tile_idx {
            // The active tile was removed, so the active tile index shifted to the next tile.
            if tile_idx == column.tiles.len() {
                // The bottom tile was removed and it was active, update active idx to remain valid.
                column.activate_idx(tile_idx - 1);
            } else {
                // Ensure the newly active tile animates to opaque.
                column.tiles[tile_idx].ensure_alpha_animates_to_1();
            }
        }

        column.update_tile_sizes_with_transaction(true, transaction);
        self.data[column_idx].update(column);
        let offset = prev_width - column.width();

        // Animate movement of the other columns. On the grid, a column keeps its area when a tab
        // leaves it, so nothing else moves.
        if !GRID_LAYOUT {
            if self.active_column_idx <= column_idx {
                for col in &mut self.columns[column_idx + 1..] {
                    col.animate_move_x_from_with_config(offset, movement_config);
                }
            } else {
                for col in &mut self.columns[..=column_idx] {
                    col.animate_move_x_from_with_config(-offset, movement_config);
                }
            }
        }

        tile
    }

    pub fn active_column(&self) -> Option<&Column<W>> {
        if self.columns.is_empty() {
            return None;
        }

        Some(&self.columns[self.active_column_idx])
    }

    pub fn remove_active_column(&mut self) -> Option<Column<W>> {
        if self.columns.is_empty() {
            return None;
        }

        Some(self.remove_column_by_idx(self.active_column_idx, None))
    }

    pub fn remove_column_by_idx(
        &mut self,
        column_idx: usize,
        anim_config: Option<niri_config::Animation>,
    ) -> Column<W> {
        let before = self.grid.clone();
        let prev_positions = self.grid_positions();
        let was_active = column_idx == self.active_column_idx;

        let column = self.columns.remove(column_idx);
        self.data.remove(column_idx);

        // Keep pointing at the same active column so that sync_grid() follows it through the
        // re-sort. If the active column itself was removed, this points at some other column
        // until we pick one below.
        if column_idx < self.active_column_idx {
            self.active_column_idx -= 1;
        }
        self.active_column_idx = min(self.active_column_idx, self.columns.len().saturating_sub(1));

        // Removing leaves a hole on the grid, collapses columns of cells that became empty, and
        // lets gravity pull windows left. The view stays still, so the windows that moved slide
        // into place.
        self.grid.remove(&column.id);
        self.sync_grid(Some(prev_positions), anim_config);

        // Stop interactive resize.
        if let Some(resize) = &self.interactive_resize {
            if column
                .tiles
                .iter()
                .any(|tile| tile.window().id() == &resize.window)
            {
                self.interactive_resize = None;
            }
        }

        let prev = self.activate_prev_column_on_removal.take();

        if self.columns.is_empty() {
            self.view_offset_to_restore = None;
            return column;
        }

        let view_config = anim_config.unwrap_or(self.options.animations.horizontal_view_movement.0);

        if !was_active {
            // Keep going back to the previous column, unless that's the one that was removed.
            self.activate_prev_column_on_removal = prev.filter(|prev| prev.prev != column.id);

            // Gravity may have pulled the active column out of view.
            if !self.view_offset.is_gesture() {
                self.animate_view_offset_to_column_with_config(
                    None,
                    self.active_column_idx,
                    None,
                    view_config,
                );
            }
            return column;
        }

        self.view_offset_to_restore = None;

        // Go back to the column that this one was opened from, if there were no focus changes
        // in-between.
        // The view only scrolls if that window isn't fully in view.
        let prev = prev
            .filter(|prev| prev.opened == column.id)
            .and_then(|prev| self.columns.iter().position(|col| col.id == prev.prev));
        if let Some(idx) = prev {
            self.active_column_idx = idx;
            self.animate_view_offset_to_column_with_config(None, idx, None, view_config);
            return column;
        }

        // Otherwise, go to a neighbor of the removed column.
        let anchor = self
            .focus_anchor
            .filter(|anchor| anchor.column == column.id);
        let idx = self
            .removal_fallback(&before, column.id, anchor)
            .and_then(|id| self.columns.iter().position(|col| col.id == id))
            .unwrap_or(self.active_column_idx);
        self.active_column_idx = idx;
        self.animate_view_offset_to_column_with_config(None, idx, None, view_config);

        column
    }

    /// Column to activate after removing the active `removed` column from `before`.
    ///
    /// Prefers the window below or above, like the tile below or above in a niri column, then
    /// the one to the right or left, like the next or previous column.
    fn removal_fallback(
        &self,
        before: &Grid<ColumnId>,
        removed: ColumnId,
        anchor: Option<FocusAnchor>,
    ) -> Option<ColumnId> {
        use grid::Direction::{Down, Left, Right, Up};

        let row = anchor.and_then(|anchor| anchor.row);
        let col = anchor.and_then(|anchor| anchor.col);
        [(Down, col), (Up, col), (Right, row), (Left, row)]
            .into_iter()
            .find_map(|(dir, anchor)| before.adjacent(&removed, dir, anchor))
            .or_else(|| {
                [(Right, row), (Left, row)]
                    .into_iter()
                    .find_map(|(dir, anchor)| before.neighbor(&removed, dir, anchor))
            })
            .copied()
    }

    pub fn update_window(&mut self, window: &W::Id, serial: Option<Serial>) {
        let (col_idx, column) = self
            .columns
            .iter_mut()
            .enumerate()
            .find(|(_, col)| col.contains(window))
            .unwrap();
        let was_normal = column.sizing_mode().is_normal();
        let prev_origin = column.tiles_origin();

        let (tile_idx, tile) = column
            .tiles
            .iter_mut()
            .enumerate()
            .find(|(_, tile)| tile.window().id() == window)
            .unwrap();

        let resize = tile.window_mut().interactive_resize_data();

        // Do this before calling update_window() so it can get up-to-date info.
        if let Some(serial) = serial {
            tile.window_mut().on_commit(serial);
        }

        let prev_width = self.data[col_idx].width;

        column.update_window(window);
        self.data[col_idx].update(column);
        column.update_tile_sizes(false);

        let offset = prev_width - self.data[col_idx].width;

        // Columns sit on the grid, so unlike in niri, other columns don't move in tandem with
        // the resize.
        let ongoing_resize_anim = self.columns[col_idx].tiles[tile_idx]
            .resize_animation()
            .is_some();

        // When a column goes between fullscreen and non-fullscreen, the tiles origin can change.
        // The change comes from things like ignoring struts and hiding the tab indicator in
        // fullscreen, so both in X and Y directions.
        let column = &mut self.columns[col_idx];
        let new_origin = column.tiles_origin();
        let origin_delta = prev_origin - new_origin;
        if origin_delta != Point::new(0., 0.) {
            for (tile, _pos) in column.tiles_mut() {
                tile.animate_move_from(origin_delta);
            }
        }

        if col_idx == self.active_column_idx {
            // If offset == 0, then don't mess with the view or the gesture. Some clients (Firefox,
            // Chromium, Electron) currently don't commit after the ack of a configure that drops
            // the Resizing state, which can trigger this code path for a while.
            let resize = if offset != 0. && !GRID_LAYOUT {
                resize
            } else {
                None
            };
            if let Some(resize) = resize {
                // Don't bother with the gesture.
                self.view_offset.cancel_gesture();

                // If this is an interactive resize commit of an active window, then we need to
                // either preserve the view offset or adjust it accordingly.
                let centered = self.is_centering_focused_column();

                let width = self.data[col_idx].width;
                let offset = if centered {
                    // FIXME: when view_offset becomes fractional, this can be made additive too.
                    let new_offset =
                        -(self.working_area.size.w - width) / 2. - self.working_area.loc.x;
                    self.column_x(col_idx) + new_offset - self.view_offset.target()
                } else if resize.edges.contains(ResizeEdge::LEFT) {
                    -offset
                } else {
                    0.
                };

                self.view_offset.offset(offset);
            }

            // When the active column goes fullscreen, store the view offset to restore later.
            let is_normal = self.columns[col_idx].sizing_mode().is_normal();
            if was_normal && !is_normal {
                let id = self.columns[col_idx].id;
                let offset = self.view_offset.stationary() - self.column_x(col_idx);
                self.view_offset_to_restore = Some((id, offset));
            }

            // Upon unfullscreening, restore the view offset.
            //
            // In tabbed display mode, there can be multiple tiles in a fullscreen column. They
            // will unfullscreen one by one, and the column width will shrink only when the
            // last tile unfullscreens. This is when we want to restore the view offset,
            // otherwise it will immediately reset back by the animate_view_offset below.
            let unfullscreen_offset = if !was_normal && is_normal {
                // Take the value unconditionally, even if the view is currently frozen by
                // a view gesture. It shouldn't linger around because it's only valid for this
                // particular unfullscreen.
                let id = self.columns[col_idx].id;
                self.view_offset_to_restore
                    .take()
                    .filter(|(restore_id, _)| *restore_id == id)
                    .map(|(_, offset)| offset)
            } else {
                None
            };

            // We might need to move the view to ensure the resized window is still visible. But
            // only do it when the view isn't frozen by an interactive resize or a view gesture.
            if self.interactive_resize.is_none() && !self.view_offset.is_gesture() {
                // Synchronize the horizontal view movement with the resize so that it looks nice.
                // This is especially important for always-centered view.
                let config = if ongoing_resize_anim {
                    self.options.animations.window_resize.anim
                } else {
                    self.options.animations.horizontal_view_movement.0
                };

                // Restore the view offset upon unfullscreening if needed.
                if let Some(prev_offset) = unfullscreen_offset {
                    self.animate_view_offset_with_config(col_idx, prev_offset, config);
                }

                // FIXME: we will want to skip the animation in some cases here to make continuously
                // resizing windows not look janky.
                self.animate_view_offset_to_column_with_config(None, col_idx, None, config);
            }
        }
    }

    pub fn scroll_amount_to_activate(&self, window: &W::Id) -> f64 {
        let column_idx = self
            .columns
            .iter()
            .position(|col| col.contains(window))
            .unwrap();

        if self.active_column_idx == column_idx {
            return 0.;
        }

        // Consider the end of an ongoing animation because that's what compute to fit does too.
        let target_x = self.target_view_pos();
        let new_view_offset = self.compute_new_view_offset_for_column(
            Some(target_x),
            column_idx,
            Some(self.active_column_idx),
        );

        let new_col_x = self.column_x(column_idx);
        let from_view_offset = target_x - new_col_x;

        (from_view_offset - new_view_offset).abs() / self.working_area.size.w
    }

    pub fn activate_window(&mut self, window: &W::Id) -> bool {
        let column_idx = self.columns.iter().position(|col| col.contains(window));
        let Some(column_idx) = column_idx else {
            return false;
        };
        let column = &mut self.columns[column_idx];

        column.activate_window(window);
        self.activate_column(column_idx);

        true
    }

    pub fn start_close_animation_for_window(
        &mut self,
        renderer: &mut GlesRenderer,
        window: &W::Id,
        blocker: TransactionBlocker,
    ) {
        let (tile, mut tile_pos) = self
            .tiles_with_render_positions_mut(false)
            .find(|(tile, _)| tile.window().id() == window)
            .unwrap();

        let Some(snapshot) = tile.take_unmap_snapshot() else {
            return;
        };

        let tile_size = tile.tile_size();

        let (col_idx, tile_idx) = self
            .columns
            .iter()
            .enumerate()
            .find_map(|(col_idx, col)| {
                col.tiles
                    .iter()
                    .position(|tile| tile.window().id() == window)
                    .map(move |tile_idx| (col_idx, tile_idx))
            })
            .unwrap();

        let col = &self.columns[col_idx];

        // Skip closing animation for invisible tiles in a tabbed column.
        if col.display_mode == ColumnDisplay::Tabbed && tile_idx != col.active_tile_idx {
            return;
        }

        // The view doesn't follow the active column, so the window closes where it is.
        tile_pos.x += self.view_pos();

        self.start_close_animation_for_tile(renderer, snapshot, tile_size, tile_pos, blocker);
    }

    fn start_close_animation_for_tile(
        &mut self,
        renderer: &mut GlesRenderer,
        snapshot: TileRenderSnapshot,
        tile_size: Size<f64, Logical>,
        tile_pos: Point<f64, Logical>,
        blocker: TransactionBlocker,
    ) {
        let anim = Animation::new(
            self.clock.clone(),
            0.,
            1.,
            0.,
            self.options.animations.window_close.anim,
        );

        let blocker = if self.options.disable_transactions {
            TransactionBlocker::completed()
        } else {
            blocker
        };

        let scale = Scale::from(self.scale);
        let res = ClosingWindow::new(
            renderer, snapshot, scale, tile_size, tile_pos, blocker, anim,
        );
        match res {
            Ok(closing) => {
                self.closing_windows.push(closing);
            }
            Err(err) => {
                warn!("error creating a closing window animation: {err:?}");
            }
        }
    }

    pub fn start_open_animation(&mut self, id: &W::Id) -> bool {
        self.columns
            .iter_mut()
            .any(|col| col.start_open_animation(id))
    }

    pub fn focus_left(&mut self) -> bool {
        if GRID_LAYOUT {
            return self.focus_grid(grid::Direction::Left);
        }

        if self.active_column_idx == 0 {
            return false;
        }
        self.activate_column(self.active_column_idx - 1);
        true
    }

    pub fn focus_right(&mut self) -> bool {
        if GRID_LAYOUT {
            return self.focus_grid(grid::Direction::Right);
        }

        if self.active_column_idx + 1 >= self.columns.len() {
            return false;
        }

        self.activate_column(self.active_column_idx + 1);
        true
    }

    pub fn focus_column_first(&mut self) {
        if GRID_LAYOUT {
            self.focus_grid_end(false);
            return;
        }

        self.activate_column(0);
    }

    pub fn focus_column_last(&mut self) {
        if GRID_LAYOUT {
            self.focus_grid_end(true);
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        self.activate_column(self.columns.len() - 1);
    }

    pub fn focus_column(&mut self, index: usize) {
        if GRID_LAYOUT {
            if let Some(target) = self.grid_column_window(index) {
                let (_, row, _) = self.grid_focus_lines().unwrap();
                let col = self.grid.area(&target).unwrap().col;
                self.focus_grid_window(target, row, col);
            }
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        self.activate_column(index.saturating_sub(1).min(self.columns.len() - 1));
    }

    pub fn focus_window_in_column(&mut self, index: u8) {
        if GRID_LAYOUT {
            self.focus_grid_in_stack(|stack| usize::from(index).saturating_sub(1).min(stack - 1));
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        self.columns[self.active_column_idx].focus_index(index);
    }

    pub fn focus_down(&mut self) -> bool {
        if GRID_LAYOUT {
            return self.focus_grid_tab(true) || self.focus_grid(grid::Direction::Down);
        }

        if self.columns.is_empty() {
            return false;
        }

        self.columns[self.active_column_idx].focus_down()
    }

    pub fn focus_up(&mut self) -> bool {
        if GRID_LAYOUT {
            return self.focus_grid_tab(false) || self.focus_grid(grid::Direction::Up);
        }

        if self.columns.is_empty() {
            return false;
        }

        self.columns[self.active_column_idx].focus_up()
    }

    /// Focuses the window below that shares columns of cells with the active one, like the tile
    /// below in a niri column, rather than any window further down.
    ///
    /// For actions that do something else when there's no window below.
    pub fn focus_down_adjacent(&mut self) -> bool {
        if GRID_LAYOUT {
            return self.focus_grid_tab(true) || self.focus_grid_in(grid::Direction::Down, true);
        }

        self.focus_down()
    }

    /// Focuses the window above that shares columns of cells with the active one, like the tile
    /// above in a niri column, rather than any window further up.
    ///
    /// For actions that do something else when there's no window above.
    pub fn focus_up_adjacent(&mut self) -> bool {
        if GRID_LAYOUT {
            return self.focus_grid_tab(false) || self.focus_grid_in(grid::Direction::Up, true);
        }

        self.focus_up()
    }

    pub fn focus_down_or_left(&mut self) {
        if GRID_LAYOUT {
            if !self.focus_down_adjacent() {
                self.focus_left();
            }
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let column = &mut self.columns[self.active_column_idx];
        if !column.focus_down() {
            self.focus_left();
        }
    }

    pub fn focus_down_or_right(&mut self) {
        if GRID_LAYOUT {
            if !self.focus_down_adjacent() {
                self.focus_right();
            }
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let column = &mut self.columns[self.active_column_idx];
        if !column.focus_down() {
            self.focus_right();
        }
    }

    pub fn focus_up_or_left(&mut self) {
        if GRID_LAYOUT {
            if !self.focus_up_adjacent() {
                self.focus_left();
            }
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let column = &mut self.columns[self.active_column_idx];
        if !column.focus_up() {
            self.focus_left();
        }
    }

    pub fn focus_up_or_right(&mut self) {
        if GRID_LAYOUT {
            if !self.focus_up_adjacent() {
                self.focus_right();
            }
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let column = &mut self.columns[self.active_column_idx];
        if !column.focus_up() {
            self.focus_right();
        }
    }

    pub fn focus_top(&mut self) {
        if GRID_LAYOUT {
            self.focus_grid_in_stack(|_| 0);
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        self.columns[self.active_column_idx].focus_top()
    }

    pub fn focus_bottom(&mut self) {
        if GRID_LAYOUT {
            self.focus_grid_in_stack(|stack| stack - 1);
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        self.columns[self.active_column_idx].focus_bottom()
    }

    pub fn move_column_to_index(&mut self, index: usize) {
        if GRID_LAYOUT {
            if let Some(target) = self.grid_column_window(index) {
                self.move_grid_to_window(target);
            }
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        self.move_column_to(index.saturating_sub(1).min(self.columns.len() - 1));
    }

    fn move_column_to(&mut self, new_idx: usize) {
        if self.active_column_idx == new_idx {
            return;
        }

        let current_col_x = self.column_x(self.active_column_idx);
        let next_col_x = self.column_x(self.active_column_idx + 1);

        let mut column = self.columns.remove(self.active_column_idx);
        let data = self.data.remove(self.active_column_idx);
        cancel_resize_for_column(&mut self.interactive_resize, &mut column);
        self.columns.insert(new_idx, column);
        self.data.insert(new_idx, data);

        // The column we just moved is offset by the difference between its new and old position.
        let new_col_x = self.column_x(new_idx);
        self.columns[new_idx].animate_move_x_from(current_col_x - new_col_x);

        // All columns in between moved by the width of the column that we just moved.
        let others_x_offset = next_col_x - current_col_x;
        if self.active_column_idx < new_idx {
            for col in &mut self.columns[self.active_column_idx..new_idx] {
                col.animate_move_x_from(others_x_offset);
            }
        } else {
            for col in &mut self.columns[new_idx + 1..=self.active_column_idx] {
                col.animate_move_x_from(-others_x_offset);
            }
        }

        self.activate_column_with_anim_config(new_idx, self.options.animations.window_movement.0);
    }

    pub fn move_left(&mut self) -> bool {
        if GRID_LAYOUT {
            return self.move_grid(grid::Direction::Left);
        }

        if self.active_column_idx == 0 {
            return false;
        }

        self.move_column_to(self.active_column_idx - 1);
        true
    }

    pub fn move_right(&mut self) -> bool {
        if GRID_LAYOUT {
            return self.move_grid(grid::Direction::Right);
        }

        let new_idx = self.active_column_idx + 1;
        if new_idx >= self.columns.len() {
            return false;
        }

        self.move_column_to(new_idx);
        true
    }

    pub fn move_column_to_first(&mut self) {
        if GRID_LAYOUT {
            // Go to the start of the strip in the window's own rows, pushing others right.
            self.move_grid_to_col(|_| (0, grid::Direction::Right));
            return;
        }

        self.move_column_to(0);
    }

    pub fn move_column_to_last(&mut self) {
        if GRID_LAYOUT {
            // Go past the end of the strip.
            let width = self.grid.width();
            self.move_grid_to_col(|_| (width, grid::Direction::Right));
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let new_idx = self.columns.len() - 1;
        self.move_column_to(new_idx);
    }

    pub fn move_down(&mut self) -> bool {
        if GRID_LAYOUT {
            // Tabs reorder within their area, like tiles within a column.
            if let Some(col) = self.columns.get_mut(self.active_column_idx) {
                if col.display_mode == ColumnDisplay::Tabbed && col.tiles.len() > 1 {
                    return col.move_down();
                }
            }
            return self.move_grid(grid::Direction::Down);
        }

        if self.columns.is_empty() {
            return false;
        }

        self.columns[self.active_column_idx].move_down()
    }

    pub fn move_up(&mut self) -> bool {
        if GRID_LAYOUT {
            // Tabs reorder within their area, like tiles within a column.
            if let Some(col) = self.columns.get_mut(self.active_column_idx) {
                if col.display_mode == ColumnDisplay::Tabbed && col.tiles.len() > 1 {
                    return col.move_up();
                }
            }
            return self.move_grid(grid::Direction::Up);
        }

        if self.columns.is_empty() {
            return false;
        }

        self.columns[self.active_column_idx].move_up()
    }

    pub fn consume_or_expel_window_left(&mut self, window: Option<&W::Id>) {
        if GRID_LAYOUT {
            self.consume_or_expel_grid(window, grid::Direction::Left);
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let (source_col_idx, source_tile_idx) = if let Some(window) = window {
            self.columns
                .iter_mut()
                .enumerate()
                .find_map(|(col_idx, col)| {
                    col.tiles
                        .iter()
                        .position(|tile| tile.window().id() == window)
                        .map(|tile_idx| (col_idx, tile_idx))
                })
                .unwrap()
        } else {
            let source_col_idx = self.active_column_idx;
            let source_tile_idx = self.columns[self.active_column_idx].active_tile_idx;
            (source_col_idx, source_tile_idx)
        };

        let source_column = &self.columns[source_col_idx];
        let prev_off = source_column.tile_offset(source_tile_idx);

        let source_tile_was_active = self.active_column_idx == source_col_idx
            && source_column.active_tile_idx == source_tile_idx;

        if source_column.tiles.len() == 1 {
            if source_col_idx == 0 {
                return;
            }

            // Move into adjacent column.
            let target_column_idx = source_col_idx - 1;

            let offset = if self.active_column_idx <= source_col_idx {
                // Tiles to the right animate from the following column.
                self.column_x(source_col_idx) - self.column_x(target_column_idx)
            } else {
                // Tiles to the left animate to preserve their right edge position.
                f64::max(
                    0.,
                    self.data[target_column_idx].width - self.data[source_col_idx].width,
                )
            };
            let mut offset = Point::from((offset, 0.));

            if source_tile_was_active {
                // Make sure the previous (target) column is activated so the animation looks right.
                //
                // However, if it was already going to be activated, leave the offset as is. This
                // improves the workflow that has become common with tabbed columns: open a new
                // window, then immediately consume it left as a new tab.
                self.activate_prev_column_on_removal
                    .get_or_insert(PrevColumn {
                        opened: self.columns[source_col_idx].id,
                        prev: self.columns[target_column_idx].id,
                    });
            }

            offset += self.columns[source_col_idx].render_offset();
            let RemovedTile { tile, .. } = self.remove_tile_by_idx(
                source_col_idx,
                0,
                Transaction::new(),
                Some(self.options.animations.window_movement.0),
            );
            self.add_tile_to_column(target_column_idx, None, tile, source_tile_was_active);

            let target_column = &mut self.columns[target_column_idx];
            offset -= target_column.render_offset();
            offset += prev_off - target_column.tile_offset(target_column.tiles.len() - 1);

            let new_tile = target_column.tiles.last_mut().unwrap();
            new_tile.animate_move_from(offset);
        } else {
            // Move out of column.
            let mut offset = source_column.render_offset();

            let removed =
                self.remove_tile_by_idx(source_col_idx, source_tile_idx, Transaction::new(), None);

            // We're inserting into the source column position.
            let target_column_idx = source_col_idx;

            self.add_tile(
                Some(target_column_idx),
                removed.tile,
                source_tile_was_active,
                removed.width,
                removed.is_full_width,
                Some(self.options.animations.window_movement.0),
            );

            if source_tile_was_active {
                // We added to the left, don't activate even further left on removal.
                self.activate_prev_column_on_removal = None;
            }

            if target_column_idx <= self.active_column_idx {
                // Tiles to the left animate from the following column.
                offset.x += self.column_x(target_column_idx + 1) - self.column_x(target_column_idx);
            }

            let new_col = &mut self.columns[target_column_idx];
            offset += prev_off - new_col.tile_offset(0);
            new_col.tiles[0].animate_move_from(offset);
        }
    }

    pub fn consume_or_expel_window_right(&mut self, window: Option<&W::Id>) {
        if GRID_LAYOUT {
            self.consume_or_expel_grid(window, grid::Direction::Right);
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let (source_col_idx, source_tile_idx) = if let Some(window) = window {
            self.columns
                .iter_mut()
                .enumerate()
                .find_map(|(col_idx, col)| {
                    col.tiles
                        .iter()
                        .position(|tile| tile.window().id() == window)
                        .map(|tile_idx| (col_idx, tile_idx))
                })
                .unwrap()
        } else {
            let source_col_idx = self.active_column_idx;
            let source_tile_idx = self.columns[self.active_column_idx].active_tile_idx;
            (source_col_idx, source_tile_idx)
        };

        let cur_x = self.column_x(source_col_idx);

        let source_column = &self.columns[source_col_idx];
        let mut offset = source_column.render_offset();
        let prev_off = source_column.tile_offset(source_tile_idx);

        let source_tile_was_active = self.active_column_idx == source_col_idx
            && source_column.active_tile_idx == source_tile_idx;

        if source_column.tiles.len() == 1 {
            if source_col_idx + 1 == self.columns.len() {
                return;
            }

            // Move into adjacent column.
            let target_column_idx = source_col_idx;

            offset.x += cur_x - self.column_x(source_col_idx + 1);
            offset -= self.columns[source_col_idx + 1].render_offset();

            if source_tile_was_active {
                // Make sure the target column gets activated.
                self.activate_prev_column_on_removal = None;
            }

            let RemovedTile { tile, .. } = self.remove_tile_by_idx(
                source_col_idx,
                0,
                Transaction::new(),
                Some(self.options.animations.window_movement.0),
            );
            self.add_tile_to_column(target_column_idx, None, tile, source_tile_was_active);

            let target_column = &mut self.columns[target_column_idx];
            offset += prev_off - target_column.tile_offset(target_column.tiles.len() - 1);

            let new_tile = target_column.tiles.last_mut().unwrap();
            new_tile.animate_move_from(offset);
        } else {
            // Move out of column.
            let prev_width = self.data[source_col_idx].width;

            let removed =
                self.remove_tile_by_idx(source_col_idx, source_tile_idx, Transaction::new(), None);

            let target_column_idx = source_col_idx + 1;

            self.add_tile(
                Some(target_column_idx),
                removed.tile,
                source_tile_was_active,
                removed.width,
                removed.is_full_width,
                Some(self.options.animations.window_movement.0),
            );

            offset.x += if self.active_column_idx <= target_column_idx {
                // Tiles to the right animate to the following column.
                cur_x - self.column_x(target_column_idx)
            } else {
                // Tiles to the left animate for a change in width.
                -f64::max(0., prev_width - self.data[target_column_idx].width)
            };

            let new_col = &mut self.columns[target_column_idx];
            offset += prev_off - new_col.tile_offset(0);
            new_col.tiles[0].animate_move_from(offset);
        }
    }

    pub fn consume_into_column(&mut self) {
        if GRID_LAYOUT {
            // Bring the window on the right under the active one.
            let Some((id, row, _)) = self.grid_focus_lines() else {
                return;
            };
            if let Some(&right) = self.grid.adjacent(&id, grid::Direction::Right, Some(row)) {
                self.change_grid(|this| this.consume_grid(right, id));
            }
            return;
        }

        if self.columns.len() < 2 {
            return;
        }

        if self.active_column_idx == self.columns.len() - 1 {
            return;
        }

        let target_column_idx = self.active_column_idx;
        let source_column_idx = self.active_column_idx + 1;

        let mut offset = self.columns[source_column_idx].render_offset();
        offset.x += self.column_x(source_column_idx);
        offset.x -= self.column_x(target_column_idx);
        let prev_off = self.columns[source_column_idx].tile_offset(0);

        let removed = self.remove_tile_by_idx(source_column_idx, 0, Transaction::new(), None);
        self.add_tile_to_column(target_column_idx, None, removed.tile, false);

        let target_column = &mut self.columns[target_column_idx];
        offset += prev_off - target_column.tile_offset(target_column.tiles.len() - 1);
        offset -= target_column.render_offset();

        let new_tile = target_column.tiles.last_mut().unwrap();
        new_tile.animate_move_from(offset);
    }

    pub fn expel_from_column(&mut self) {
        if GRID_LAYOUT {
            if let Some(col) = self.columns.get(self.active_column_idx) {
                let id = col.id;
                let tile_idx = col.active_tile_idx;
                if col.tiles.len() > 1 {
                    self.change_grid(|this| {
                        this.expel_grid_tab(id, tile_idx, grid::Direction::Right)
                    });
                } else {
                    self.change_grid(|this| this.expel_grid(id, grid::Direction::Right));
                }
            }
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let source_col_idx = self.active_column_idx;
        let target_col_idx = self.active_column_idx + 1;
        let cur_x = self.column_x(source_col_idx);

        let source_column = &self.columns[self.active_column_idx];
        if source_column.tiles.len() == 1 {
            return;
        }

        let source_tile_idx = source_column.tiles.len() - 1;

        let mut offset = source_column.render_offset();
        let prev_off = source_column.tile_offset(source_tile_idx);

        let removed =
            self.remove_tile_by_idx(source_col_idx, source_tile_idx, Transaction::new(), None);

        self.add_tile(
            Some(target_col_idx),
            removed.tile,
            false,
            removed.width,
            removed.is_full_width,
            Some(self.options.animations.window_movement.0),
        );

        offset.x += cur_x - self.column_x(target_col_idx);

        let new_col = &mut self.columns[target_col_idx];
        offset += prev_off - new_col.tile_offset(0);
        new_col.tiles[0].animate_move_from(offset);
    }

    pub fn swap_window_in_direction(&mut self, direction: ScrollDirection) {
        if GRID_LAYOUT {
            match direction {
                ScrollDirection::Left => self.move_left(),
                ScrollDirection::Right => self.move_right(),
            };
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        // if this is the first (resp. last column), then this operation is equivalent
        // to an `consume_or_expel_window_left` (resp. `consume_or_expel_window_right`)
        match direction {
            ScrollDirection::Left => {
                if self.active_column_idx == 0 {
                    return;
                }
            }
            ScrollDirection::Right => {
                if self.active_column_idx == self.columns.len() - 1 {
                    return;
                }
            }
        }

        let source_column_idx = self.active_column_idx;
        let target_column_idx = self.active_column_idx.wrapping_add_signed(match direction {
            ScrollDirection::Left => -1,
            ScrollDirection::Right => 1,
        });

        // if both source and target columns contain a single tile, then the operation is equivalent
        // to a simple column move
        if self.columns[source_column_idx].tiles.len() == 1
            && self.columns[target_column_idx].tiles.len() == 1
        {
            return self.move_column_to(target_column_idx);
        }

        let source_tile_idx = self.columns[source_column_idx].active_tile_idx;
        let target_tile_idx = self.columns[target_column_idx].active_tile_idx;
        let source_column_drained = self.columns[source_column_idx].tiles.len() == 1;

        // capture the original positions of the tiles
        let (mut source_pt, mut target_pt) = (
            self.columns[source_column_idx].render_offset()
                + self.columns[source_column_idx].tile_offset(source_tile_idx),
            self.columns[target_column_idx].render_offset()
                + self.columns[target_column_idx].tile_offset(target_tile_idx),
        );
        source_pt.x += self.column_x(source_column_idx);
        target_pt.x += self.column_x(target_column_idx);

        let transaction = Transaction::new();

        // If the source column contains a single tile, this will also remove the column.
        // When this happens `source_column_drained` will be set and the column will need to be
        // recreated with `add_tile`
        let source_removed = self.remove_tile_by_idx(
            source_column_idx,
            source_tile_idx,
            transaction.clone(),
            None,
        );

        {
            // special case when the source column disappears after removing its last tile
            let adjusted_target_column_idx =
                if direction == ScrollDirection::Right && source_column_drained {
                    target_column_idx - 1
                } else {
                    target_column_idx
                };

            self.add_tile_to_column(
                adjusted_target_column_idx,
                Some(target_tile_idx),
                source_removed.tile,
                false,
            );

            let RemovedTile {
                tile: target_tile, ..
            } = self.remove_tile_by_idx(
                adjusted_target_column_idx,
                target_tile_idx + 1,
                transaction.clone(),
                None,
            );

            if source_column_drained {
                // recreate the drained column with only the target tile
                self.add_tile(
                    Some(source_column_idx),
                    target_tile,
                    true,
                    source_removed.width,
                    source_removed.is_full_width,
                    None,
                )
            } else {
                // simply add the removed target tile to the source column
                self.add_tile_to_column(
                    source_column_idx,
                    Some(source_tile_idx),
                    target_tile,
                    false,
                );
            }
        }

        // update the active tile in the modified columns
        self.columns[source_column_idx].active_tile_idx = source_tile_idx;
        self.columns[target_column_idx].active_tile_idx = target_tile_idx;

        // Animations
        self.columns[target_column_idx].tiles[target_tile_idx]
            .animate_move_from(source_pt - target_pt);
        self.columns[target_column_idx].tiles[target_tile_idx].ensure_alpha_animates_to_1();

        // FIXME: this stop_move_animations() causes the target tile animation to "reset" when
        // swapping. It's here as a workaround to stop the unwanted animation of moving the source
        // tile down when adding the target tile above it. This code needs to be written in some
        // other way not to trigger that animation, or to cancel it properly, so that swap doesn't
        // cancel all ongoing target tile animations.
        self.columns[source_column_idx].tiles[source_tile_idx].stop_move_animations();
        self.columns[source_column_idx].tiles[source_tile_idx]
            .animate_move_from(target_pt - source_pt);
        self.columns[source_column_idx].tiles[source_tile_idx].ensure_alpha_animates_to_1();

        self.activate_column(target_column_idx);
    }

    pub fn toggle_column_tabbed_display(&mut self) {
        if GRID_LAYOUT {
            let Some(col) = self.columns.get(self.active_column_idx) else {
                return;
            };
            let display = match col.display_mode {
                ColumnDisplay::Normal => ColumnDisplay::Tabbed,
                ColumnDisplay::Tabbed => ColumnDisplay::Normal,
            };
            self.set_grid_display(display);
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let col = &mut self.columns[self.active_column_idx];
        let display = match col.display_mode {
            ColumnDisplay::Normal => ColumnDisplay::Tabbed,
            ColumnDisplay::Tabbed => ColumnDisplay::Normal,
        };

        self.set_column_display(display);
    }

    pub fn set_column_display(&mut self, display: ColumnDisplay) {
        if GRID_LAYOUT {
            self.set_grid_display(display);
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let col = &mut self.columns[self.active_column_idx];
        if col.display_mode == display {
            return;
        }

        cancel_resize_for_column(&mut self.interactive_resize, col);
        col.set_column_display(display);

        // With place_within_column, the tab indicator changes the column size immediately.
        self.data[self.active_column_idx].update(col);
        col.update_tile_sizes(true);

        // Disable fullscreen if needed.
        if col.display_mode != ColumnDisplay::Tabbed && col.tiles.len() > 1 {
            let window = col.tiles[col.active_tile_idx].window().id().clone();
            self.set_fullscreen(&window, false);
            self.set_maximized(&window, false);
        }
    }

    pub fn center_column(&mut self) {
        if self.columns.is_empty() {
            return;
        }

        self.animate_view_offset_to_column_centered(
            None,
            self.active_column_idx,
            self.options.animations.horizontal_view_movement.0,
        );

        let col = &mut self.columns[self.active_column_idx];
        cancel_resize_for_column(&mut self.interactive_resize, col);
    }

    pub fn center_window(&mut self, window: Option<&W::Id>) {
        if self.columns.is_empty() {
            return;
        }

        let col_idx = if let Some(window) = window {
            self.columns
                .iter()
                .position(|col| col.contains(window))
                .unwrap()
        } else {
            self.active_column_idx
        };

        // We can reasonably center only the active column.
        if col_idx != self.active_column_idx {
            return;
        }

        self.center_column();
    }

    pub fn center_visible_columns(&mut self) {
        if self.columns.is_empty() {
            return;
        }

        if self.is_centering_focused_column() {
            return;
        }

        // Consider the end of an ongoing animation because that's what compute to fit does too.
        let view_x = self.target_view_pos();
        let working_x = self.working_area.loc.x;
        let working_w = self.working_area.size.w;

        // Find the span of all windows that are fully visible inside the working area. Windows
        // can be stacked, so this is the span on the grid rather than their widths added up.
        let gap = self.options.layout.gaps;
        let mut span: Option<(f64, f64)> = None;
        let mut active_visible = false;
        let col_xs = self.column_xs(self.data.iter().copied());
        for (idx, (col_x, data)) in zip(col_xs, &self.data).enumerate() {
            let right = col_x + data.width;
            if col_x < view_x + working_x + gap || view_x + working_x + working_w < right + gap {
                // Column goes off-screen.
                continue;
            }

            active_visible |= idx == self.active_column_idx;
            span = Some(span.map_or((col_x, right), |(left, r)| {
                (f64::min(left, col_x), f64::max(r, right))
            }));
        }

        if !active_visible {
            // The active column wasn't fully on screen, so we can't meaningfully do anything.
            return;
        }
        let (left, right) = span.unwrap();

        let col = &mut self.columns[self.active_column_idx];
        cancel_resize_for_column(&mut self.interactive_resize, col);

        let free_space = working_w - (right - left);
        let new_view_x = left - free_space / 2. - working_x;

        let active_col_x = self.column_x(self.active_column_idx);
        self.animate_view_offset(self.active_column_idx, new_view_x - active_col_x);
        // Just in case.
        self.animate_view_offset_to_column(None, self.active_column_idx, None);
    }

    pub fn view_pos(&self) -> f64 {
        self.view_offset.current()
    }

    pub fn target_view_pos(&self) -> f64 {
        self.view_offset.target()
    }

    // HACK: pass a self.data iterator in manually as a workaround for the lack of method partial
    // borrowing. Note that this method's return value does not borrow the entire &Self!
    fn column_xs(&self, data: impl Iterator<Item = ColumnData>) -> impl Iterator<Item = f64> {
        if GRID_LAYOUT {
            // Columns sit on the grid, so their X comes from their grid area. Chain one past the
            // end of the grid.
            let xs: Vec<f64> = self
                .columns
                .iter()
                .map(|col| {
                    self.grid
                        .area(&col.id)
                        .map_or(0., |area| self.grid_x(area.col))
                })
                .chain(iter::once(self.grid_x(self.grid.width())))
                .collect();
            return xs.into_iter();
        }

        let gaps = self.options.layout.gaps;
        let mut x = 0.;

        // Chain with a dummy value to be able to get one past all columns' X.
        let dummy = ColumnData { width: 0. };
        let data = data.chain(iter::once(dummy));

        let xs: Vec<f64> = data
            .map(move |data| {
                let rv = x;
                x += data.width + gaps;
                rv
            })
            .collect();
        xs.into_iter()
    }

    fn column_x(&self, column_idx: usize) -> f64 {
        self.column_xs(self.data.iter().copied())
            .nth(column_idx)
            .unwrap()
    }

    fn column_xs_in_render_order(
        &self,
        data: impl Iterator<Item = ColumnData>,
    ) -> impl Iterator<Item = f64> {
        let active_idx = self.active_column_idx;
        let active_pos = self.column_x(active_idx);
        let offsets = self
            .column_xs(data)
            .enumerate()
            .filter_map(move |(idx, pos)| (idx != active_idx).then_some(pos));
        iter::once(active_pos).chain(offsets)
    }

    pub fn columns(&self) -> impl Iterator<Item = &Column<W>> {
        self.columns.iter()
    }

    fn columns_mut(&mut self) -> impl Iterator<Item = (&mut Column<W>, f64)> + '_ {
        let offsets = self.column_xs(self.data.iter().copied());
        zip(&mut self.columns, offsets)
    }

    fn columns_in_render_order(&self) -> impl Iterator<Item = (&Column<W>, f64)> + '_ {
        let offsets = self.column_xs_in_render_order(self.data.iter().copied());

        let (first, active, rest) = if self.columns.is_empty() {
            (&[][..], &[][..], &[][..])
        } else {
            let (first, rest) = self.columns.split_at(self.active_column_idx);
            let (active, rest) = rest.split_at(1);
            (first, active, rest)
        };

        let columns = active.iter().chain(first).chain(rest);
        zip(columns, offsets)
    }

    fn columns_in_render_order_mut(&mut self) -> impl Iterator<Item = (&mut Column<W>, f64)> + '_ {
        let offsets = self.column_xs_in_render_order(self.data.iter().copied());

        let (first, active, rest) = if self.columns.is_empty() {
            (&mut [][..], &mut [][..], &mut [][..])
        } else {
            let (first, rest) = self.columns.split_at_mut(self.active_column_idx);
            let (active, rest) = rest.split_at_mut(1);
            (first, active, rest)
        };

        let columns = active.iter_mut().chain(first).chain(rest);
        zip(columns, offsets)
    }

    pub fn columns_with_render_positions(
        &self,
    ) -> impl Iterator<Item = (&Column<W>, Point<f64, Logical>)> {
        let view_off = Point::from((-self.view_pos(), 0.));
        self.columns_in_render_order().map(move |(col, col_x)| {
            let col_off = Point::from((col_x, 0.));
            let col_render_off = col.render_offset();
            let pos = view_off + col_off + col_render_off;
            (col, pos)
        })
    }

    pub fn columns_with_render_positions_mut(
        &mut self,
    ) -> impl Iterator<Item = (&mut Column<W>, Point<f64, Logical>)> {
        let view_off = Point::from((-self.view_pos(), 0.));
        self.columns_in_render_order_mut().map(move |(col, col_x)| {
            let col_off = Point::from((col_x, 0.));
            let col_render_off = col.render_offset();
            let pos = view_off + col_off + col_render_off;
            (col, pos)
        })
    }

    pub fn tiles_with_render_positions(
        &self,
    ) -> impl Iterator<Item = (&Tile<W>, Point<f64, Logical>, bool)> {
        let scale = self.scale;
        self.columns_with_render_positions()
            .flat_map(move |(col, col_pos)| {
                col.tiles_in_render_order()
                    .map(move |(tile, tile_off, visible)| {
                        let pos = col_pos + tile_off + tile.render_offset();
                        // Round to physical pixels.
                        let pos = pos.to_physical_precise_round(scale).to_logical(scale);
                        (tile, pos, visible)
                    })
            })
    }

    pub fn tiles_with_render_positions_mut(
        &mut self,
        round: bool,
    ) -> impl Iterator<Item = (&mut Tile<W>, Point<f64, Logical>)> {
        let scale = self.scale;
        self.columns_with_render_positions_mut()
            .flat_map(move |(col, col_pos)| {
                col.tiles_in_render_order_mut()
                    .map(move |(tile, tile_off)| {
                        let mut pos = col_pos + tile_off + tile.render_offset();
                        // Round to physical pixels.
                        if round {
                            pos = pos.to_physical_precise_round(scale).to_logical(scale);
                        }
                        (tile, pos)
                    })
            })
    }

    pub fn tiles_with_ipc_layouts(&self) -> impl Iterator<Item = (&Tile<W>, WindowLayout)> {
        // On the grid, a window's column is where it starts among the grid's columns, and its
        // place in it is from the top among the windows and tabs covering that column of cells,
        // matching the column and in-column actions.
        let starts = self.grid.column_starts();
        let grid_pos = |col: &Column<W>, tile_idx: usize| {
            let area = self.grid.area(&col.id)?;
            let col_idx = starts.iter().position(|start| *start == area.col)?;
            let stack = self.grid_stack_tiles(area.col);
            let tile_idx = stack.iter().position(|pos| *pos == (col.id, tile_idx))?;
            Some((col_idx, tile_idx))
        };

        let mut layouts = Vec::new();
        for (col_idx, col) in self.columns.iter().enumerate() {
            for (tile_idx, (tile, _)) in col.tiles().enumerate() {
                let grid_pos = if GRID_LAYOUT {
                    grid_pos(col, tile_idx)
                } else {
                    None
                };
                let (col_idx, tile_idx) = grid_pos.unwrap_or((col_idx, tile_idx));
                let layout = WindowLayout {
                    // Our indices are 1-based, consistent with the actions.
                    pos_in_scrolling_layout: Some((col_idx + 1, tile_idx + 1)),
                    ..tile.ipc_layout_template()
                };
                layouts.push((tile, layout));
            }
        }
        layouts.into_iter()
    }

    pub(super) fn insert_hint_area(
        &self,
        position: InsertPosition,
    ) -> Option<Rectangle<f64, Logical>> {
        let mut hint_area = match position {
            InsertPosition::NewColumn(column_index) => {
                if column_index == 0 || column_index == self.columns.len() {
                    let size = Size::from((
                        300.,
                        self.working_area.size.h - self.options.layout.gaps * 2.,
                    ));
                    let mut loc = Point::from((
                        self.column_x(column_index),
                        self.working_area.loc.y + self.options.layout.gaps,
                    ));
                    if column_index == 0 && !self.columns.is_empty() {
                        loc.x -= size.w + self.options.layout.gaps;
                    }
                    Rectangle::new(loc, size)
                } else if column_index > self.columns.len() {
                    error!("insert hint column index is out of range");
                    return None;
                } else {
                    let size = Size::from((
                        300.,
                        self.working_area.size.h - self.options.layout.gaps * 2.,
                    ));
                    let loc = Point::from((
                        self.column_x(column_index) - size.w / 2. - self.options.layout.gaps / 2.,
                        self.working_area.loc.y + self.options.layout.gaps,
                    ));
                    Rectangle::new(loc, size)
                }
            }
            InsertPosition::InColumn(column_index, tile_index) => {
                if column_index > self.columns.len() {
                    error!("insert hint column index is out of range");
                    return None;
                }

                let col = &self.columns[column_index];
                if tile_index > col.tiles.len() {
                    error!("insert hint tile index is out of range");
                    return None;
                }

                let is_tabbed = col.display_mode == ColumnDisplay::Tabbed;

                let (height, y) = if is_tabbed {
                    // In tabbed mode, there's only one tile visible, and we want to draw the hint
                    // at its top or bottom.
                    let top = col.tile_offset(col.active_tile_idx).y;
                    let bottom = top + col.data[col.active_tile_idx].size.h;

                    if tile_index <= col.active_tile_idx {
                        (150., top)
                    } else {
                        (150., bottom - 150.)
                    }
                } else {
                    let top = col.tile_offset(tile_index).y;

                    if tile_index == 0 {
                        (150., top)
                    } else if tile_index == col.tiles.len() {
                        (150., top - self.options.layout.gaps - 150.)
                    } else {
                        (300., top - self.options.layout.gaps / 2. - 150.)
                    }
                };

                // Adjust for place-within-column tab indicator.
                let origin_x = col.tiles_origin().x;
                let extra_w = if is_tabbed && col.sizing_mode().is_normal() {
                    col.tab_indicator.extra_size(col.tiles.len(), col.scale).w
                } else {
                    0.
                };

                let size = Size::from((self.data[column_index].width - extra_w, height));
                let loc = Point::from((self.column_x(column_index) + origin_x, y));
                Rectangle::new(loc, size)
            }
            InsertPosition::Cell(area) => {
                let geometry = self.grid_geometry(area);
                let loc =
                    Point::from((self.grid_x(area.col), self.working_area.loc.y + geometry.y));
                Rectangle::new(loc, geometry.size)
            }
            InsertPosition::Floating => return None,
        };

        // First window on an empty workspace will cancel out any view offset. Replicate this
        // effect here.
        if self.columns.is_empty() {
            let view_offset = if self.is_centering_focused_column() {
                self.compute_new_view_offset_centered(
                    Some(0.),
                    0.,
                    hint_area.size.w,
                    SizingMode::Normal,
                )
            } else {
                self.compute_new_view_offset_fit(Some(0.), 0., hint_area.size.w, SizingMode::Normal)
            };
            hint_area.loc.x -= view_offset;
        } else {
            hint_area.loc.x -= self.view_pos();
        }

        Some(hint_area)
    }

    /// Returns the geometry of the active window relative to and clamped to the view.
    ///
    /// During animations, assumes the final view position.
    pub fn active_window_visual_rectangle(&self) -> Option<Rectangle<f64, Logical>> {
        let col = self.columns.get(self.active_column_idx)?;

        let final_view_offset = self.view_offset.target() - self.column_x(self.active_column_idx);
        let view_off = Point::from((-final_view_offset, 0.));

        let (tile, tile_off) = col.tiles().nth(col.active_tile_idx).unwrap();

        let window_pos = view_off + tile_off + tile.window_loc();
        let window_size = tile.window_size();
        let window_rect = Rectangle::new(window_pos, window_size);

        let view = Rectangle::from_size(self.view_size);
        view.intersection(window_rect)
    }

    pub fn popup_target_rect(&self, id: &W::Id, overflow: bool) -> Option<Rectangle<f64, Logical>> {
        let view_pos = self.target_view_pos();
        let col_xs = self.column_xs(self.data.iter().copied());
        for (col, col_x) in zip(&self.columns, col_xs) {
            for (tile, pos) in col.tiles() {
                if tile.window().id() == id {
                    // In the scrolling layout, we try to position popups horizontally within the
                    // window geometry (so they remain visible even if the window scrolls flush with
                    // the left/right edge of the screen), and vertically within the whole parent
                    // working area.
                    let target = if overflow {
                        // For IME popups, this is a bit of an unexpected behavior though; people
                        // are used to them extending to the right. Also, IME popups disappear when
                        // keyboard focus changes, so changing scroll position is not a problem. So,
                        // constrain them to the parent area instead.
                        let mut target = self.parent_area;
                        target.loc.x -= col_x - view_pos;
                        target.loc -= pos;
                        target.loc -= tile.window_loc();
                        target
                    } else {
                        let width = tile.window_size().w;
                        let height = self.parent_area.size.h;

                        let mut target = Rectangle::from_size(Size::from((width, height)));
                        target.loc.y += self.parent_area.loc.y;
                        target.loc.y -= pos.y;
                        target.loc.y -= tile.window_loc().y;
                        target
                    };

                    return Some(target);
                }
            }
        }
        None
    }

    pub fn toggle_width(&mut self, forwards: bool) {
        if GRID_LAYOUT {
            self.toggle_grid_width(None, forwards);
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let col = &mut self.columns[self.active_column_idx];
        col.toggle_width(None, forwards);

        cancel_resize_for_column(&mut self.interactive_resize, col);
    }

    pub fn toggle_full_width(&mut self) {
        if GRID_LAYOUT {
            self.toggle_grid_full_width();
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let col = &mut self.columns[self.active_column_idx];
        col.toggle_full_width();

        cancel_resize_for_column(&mut self.interactive_resize, col);
    }

    pub fn set_window_width(&mut self, window: Option<&W::Id>, change: SizeChange) {
        if GRID_LAYOUT {
            self.set_grid_width(window, change);
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let (col, tile_idx) = if let Some(window) = window {
            self.columns
                .iter_mut()
                .find_map(|col| {
                    col.tiles
                        .iter()
                        .position(|tile| tile.window().id() == window)
                        .map(|tile_idx| (col, Some(tile_idx)))
                })
                .unwrap()
        } else {
            (&mut self.columns[self.active_column_idx], None)
        };

        col.set_column_width(change, tile_idx, true);

        cancel_resize_for_column(&mut self.interactive_resize, col);
    }

    pub fn set_window_height(&mut self, window: Option<&W::Id>, change: SizeChange) {
        if GRID_LAYOUT {
            self.set_grid_height(window, change);
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let (col, tile_idx) = if let Some(window) = window {
            self.columns
                .iter_mut()
                .find_map(|col| {
                    col.tiles
                        .iter()
                        .position(|tile| tile.window().id() == window)
                        .map(|tile_idx| (col, Some(tile_idx)))
                })
                .unwrap()
        } else {
            (&mut self.columns[self.active_column_idx], None)
        };

        col.set_window_height(change, tile_idx, true);

        cancel_resize_for_column(&mut self.interactive_resize, col);
    }

    pub fn reset_window_height(&mut self, window: Option<&W::Id>) {
        if GRID_LAYOUT {
            self.resize_grid(window, None, Some(self.cell_metrics.rows));
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let (col, tile_idx) = if let Some(window) = window {
            self.columns
                .iter_mut()
                .find_map(|col| {
                    col.tiles
                        .iter()
                        .position(|tile| tile.window().id() == window)
                        .map(|tile_idx| (col, Some(tile_idx)))
                })
                .unwrap()
        } else {
            (&mut self.columns[self.active_column_idx], None)
        };

        col.reset_window_height(tile_idx);

        cancel_resize_for_column(&mut self.interactive_resize, col);
    }

    pub fn toggle_window_width(&mut self, window: Option<&W::Id>, forwards: bool) {
        if GRID_LAYOUT {
            self.toggle_grid_width(window, forwards);
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let (col, tile_idx) = if let Some(window) = window {
            self.columns
                .iter_mut()
                .find_map(|col| {
                    col.tiles
                        .iter()
                        .position(|tile| tile.window().id() == window)
                        .map(|tile_idx| (col, Some(tile_idx)))
                })
                .unwrap()
        } else {
            (&mut self.columns[self.active_column_idx], None)
        };

        col.toggle_width(tile_idx, forwards);

        cancel_resize_for_column(&mut self.interactive_resize, col);
    }

    pub fn toggle_window_height(&mut self, window: Option<&W::Id>, forwards: bool) {
        if GRID_LAYOUT {
            self.toggle_grid_height(window, forwards);
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let (col, tile_idx) = if let Some(window) = window {
            self.columns
                .iter_mut()
                .find_map(|col| {
                    col.tiles
                        .iter()
                        .position(|tile| tile.window().id() == window)
                        .map(|tile_idx| (col, Some(tile_idx)))
                })
                .unwrap()
        } else {
            (&mut self.columns[self.active_column_idx], None)
        };

        col.toggle_window_height(tile_idx, forwards);

        cancel_resize_for_column(&mut self.interactive_resize, col);
    }

    pub fn expand_column_to_available_width(&mut self) {
        if GRID_LAYOUT {
            self.expand_grid_to_available_width();
            return;
        }

        if self.columns.is_empty() {
            return;
        }

        let col = &mut self.columns[self.active_column_idx];
        if !col.pending_sizing_mode().is_normal() || col.is_full_width {
            return;
        }

        if self.is_centering_focused_column() {
            // Always-centered mode is different since the active window position cannot be
            // controlled (it's always at the center). I guess you could come up with different
            // logic here that computes the width in such a way so as to leave nearby columns fully
            // on screen while taking into account that the active column will remain centered
            // after resizing. But I'm not sure it's that useful? So let's do the simple thing.
            let col = &mut self.columns[self.active_column_idx];
            col.toggle_full_width();
            cancel_resize_for_column(&mut self.interactive_resize, col);
            return;
        }

        // NOTE: This logic won't work entirely correctly with small fixed-size maximized windows
        // (they have a different area and padding).

        // Consider the end of an ongoing animation because that's what compute to fit does too.
        let view_x = self.target_view_pos();
        let working_x = self.working_area.loc.x;
        let working_w = self.working_area.size.w;

        // Count all columns that are fully visible inside the working area.
        let mut width_taken = 0.;
        let mut leftmost_col_x = None;
        let mut active_col_x = None;
        let mut counted_non_active_column = false;

        let gap = self.options.layout.gaps;
        let col_xs = self.column_xs(self.data.iter().copied());
        for (idx, col_x) in col_xs.take(self.columns.len()).enumerate() {
            if col_x < view_x + working_x + gap {
                // Column goes off-screen to the left.
                continue;
            }

            leftmost_col_x.get_or_insert(col_x);

            let width = self.data[idx].width;
            if view_x + working_x + working_w < col_x + width + gap {
                // Column goes off-screen to the right. We can stop here.
                break;
            }

            if idx == self.active_column_idx {
                active_col_x = Some(col_x);
            } else {
                counted_non_active_column = true;
            }

            width_taken += width + gap;
        }

        if active_col_x.is_none() {
            // The active column wasn't fully on screen, so we can't meaningfully do anything.
            return;
        }

        let col = &mut self.columns[self.active_column_idx];

        let available_width = working_w - gap - width_taken - col.extra_size().w;
        if available_width <= 0. {
            // Nowhere to expand.
            return;
        }

        cancel_resize_for_column(&mut self.interactive_resize, col);

        if !counted_non_active_column {
            // Only the active column was fully on-screen (maybe it's the only column), so we're
            // about to set its width to 100% of the working area. Let's do it via
            // toggle_full_width() as it lets you back out of it more intuitively.
            col.toggle_full_width();
            return;
        }

        let active_width = self.data[self.active_column_idx].width;
        col.width = ColumnWidth::Fixed(active_width + available_width);
        col.preset_width_idx = None;
        col.is_full_width = false;
        col.update_tile_sizes(true);

        // Put the leftmost window into the view.
        let new_view_x = leftmost_col_x.unwrap() - gap - working_x;
        self.animate_view_offset(self.active_column_idx, new_view_x - active_col_x.unwrap());
        // Just in case.
        self.animate_view_offset_to_column(None, self.active_column_idx, None);
    }

    pub fn set_fullscreen(&mut self, window: &W::Id, is_fullscreen: bool) -> bool {
        let mut col_idx = self
            .columns
            .iter()
            .position(|col| col.contains(window))
            .unwrap();

        if is_fullscreen == self.columns[col_idx].is_pending_fullscreen {
            return false;
        }

        let mut col = &mut self.columns[col_idx];
        let is_tabbed = col.display_mode == ColumnDisplay::Tabbed;

        cancel_resize_for_column(&mut self.interactive_resize, col);

        if is_fullscreen && (col.tiles.len() > 1 && !is_tabbed) {
            // This wasn't the only window in its column; extract it into a separate column.
            self.consume_or_expel_window_right(Some(window));
            col_idx += 1;
            col = &mut self.columns[col_idx];
        }

        col.set_fullscreen(is_fullscreen);

        // With place_within_column, the tab indicator changes the column size immediately.
        self.data[col_idx].update(col);

        // Fullscreen windows push the windows beside them aside on the grid.
        if GRID_LAYOUT {
            self.fit_grid_to_sizing_mode(col_idx);
        }

        true
    }

    pub fn set_maximized(&mut self, window: &W::Id, maximize: bool) -> bool {
        let mut col_idx = self
            .columns
            .iter()
            .position(|col| col.contains(window))
            .unwrap();

        if maximize == self.columns[col_idx].is_pending_maximized {
            return false;
        }

        let mut col = &mut self.columns[col_idx];
        let is_tabbed = col.display_mode == ColumnDisplay::Tabbed;

        cancel_resize_for_column(&mut self.interactive_resize, col);

        if maximize && (col.tiles.len() > 1 && !is_tabbed) {
            // This wasn't the only window in its column; extract it into a separate column.
            self.consume_or_expel_window_right(Some(window));
            col_idx += 1;
            col = &mut self.columns[col_idx];
        }

        col.set_maximized(maximize);

        // With place_within_column, the tab indicator changes the column size immediately.
        self.data[col_idx].update(col);

        // Maximized windows push the windows beside them aside on the grid.
        if GRID_LAYOUT {
            self.fit_grid_to_sizing_mode(col_idx);
        }

        true
    }

    pub fn render_above_top_layer(&self) -> bool {
        // Render above the top layer if we're on a fullscreen window and the view is stationary.
        if self.columns.is_empty() {
            return false;
        }

        if !self.view_offset.is_static() {
            return false;
        }

        self.columns[self.active_column_idx]
            .sizing_mode()
            .is_fullscreen()
    }

    pub fn render<R: NiriRenderer>(
        &self,
        mut ctx: RenderCtx<R>,
        xray_pos: XrayPos,
        focus_ring: bool,
        layer: RenderLayer,
        push: &mut dyn FnMut(ScrollingSpaceRenderElement<R>),
    ) {
        let scale = Scale::from(self.scale);

        // Draw the closing windows on top of the other windows.
        if layer.is_normal() {
            let view_rect = Rectangle::new(Point::from((self.view_pos(), 0.)), self.view_size);
            for closing in self.closing_windows.iter().rev() {
                let elem = closing.render(ctx.as_gles(), view_rect, scale);
                push(elem.into());
            }
        }

        if self.columns.is_empty() {
            return;
        }

        let mut first = true;

        // This matches self.tiles_with_render_positions().
        for (col, col_pos) in self.columns_with_render_positions() {
            // Skip columns belonging to a different render layer.
            if layer.is_normal() == col.is_moving_between_workspaces() {
                first = false;
                continue;
            }

            // Draw the tab indicator on top.
            {
                let pos = col_pos.to_physical_precise_round(scale).to_logical(scale);
                col.tab_indicator
                    .render(ctx.renderer, pos, &mut |elem| push(elem.into()));
            }

            for (tile, tile_off, visible) in col.tiles_in_render_order() {
                let tile_pos = col_pos + tile_off + tile.render_offset();
                // Round to physical pixels.
                let tile_pos = tile_pos.to_physical_precise_round(scale).to_logical(scale);

                // And now the drawing logic.

                // For the active tile (which comes first), draw the focus ring.
                let focus_ring = focus_ring && first;
                first = false;

                // In the scrolling layout, we currently use visible only for hidden tabs in the
                // tabbed mode. We want to animate their opacity when going in and out of tabbed
                // mode, so we don't want to apply "visible" immediately. However, "visible" is
                // also used for input handling, and there we *do* want to apply it immediately.
                // So, let's just selectively ignore "visible" here when animating alpha.
                let visible = visible || tile.alpha_animation.is_some();
                if !visible {
                    continue;
                }

                let xray_pos = xray_pos.offset(tile_pos);
                tile.render(ctx.r(), tile_pos, xray_pos, focus_ring, &mut |elem| {
                    push(elem.into())
                });
            }
        }
    }

    pub fn window_under(&self, pos: Point<f64, Logical>) -> Option<(&W, HitType)> {
        // This matches self.tiles_with_render_positions().
        let scale = self.scale;
        for (col, col_pos) in self.columns_with_render_positions() {
            // Hit the tab indicator.
            if col.display_mode == ColumnDisplay::Tabbed && col.sizing_mode().is_normal() {
                let col_pos = col_pos.to_physical_precise_round(scale).to_logical(scale);

                if let Some(idx) = col.tab_indicator.hit(
                    col.tab_indicator_area(),
                    col.tiles.len(),
                    scale,
                    pos - col_pos,
                ) {
                    let hit = HitType::Activate {
                        is_tab_indicator: true,
                    };
                    return Some((col.tiles[idx].window(), hit));
                }
            }

            for (tile, tile_off, visible) in col.tiles_in_render_order() {
                if !visible {
                    continue;
                }

                let tile_pos = col_pos + tile_off + tile.render_offset();
                // Round to physical pixels.
                let tile_pos = tile_pos.to_physical_precise_round(scale).to_logical(scale);

                if let Some(rv) = HitType::hit_tile(tile, tile_pos, pos) {
                    return Some(rv);
                }
            }
        }

        None
    }

    pub fn view_offset_gesture_begin(&mut self, is_touchpad: bool) {
        if self.columns.is_empty() {
            return;
        }

        if self.interactive_resize.is_some() {
            return;
        }

        let gesture = ViewGesture {
            current_view_offset: self.view_offset.current(),
            animation: None,
            tracker: SwipeTracker::new(),
            delta_from_tracker: self.view_offset.current(),
            stationary_view_offset: self.view_offset.stationary(),
            is_touchpad,
            dnd_last_event_time: None,
            dnd_nonzero_start_time: None,
        };
        self.view_offset = ViewOffset::Gesture(gesture);
    }

    pub fn dnd_scroll_gesture_begin(&mut self) {
        if let ViewOffset::Gesture(ViewGesture {
            dnd_last_event_time: Some(_),
            ..
        }) = &self.view_offset
        {
            // Already active.
            return;
        }

        let gesture = ViewGesture {
            current_view_offset: self.view_offset.current(),
            animation: None,
            tracker: SwipeTracker::new(),
            delta_from_tracker: self.view_offset.current(),
            stationary_view_offset: self.view_offset.stationary(),
            is_touchpad: false,
            dnd_last_event_time: Some(self.clock.now_unadjusted()),
            dnd_nonzero_start_time: None,
        };
        self.view_offset = ViewOffset::Gesture(gesture);

        self.interactive_resize = None;
    }

    pub fn view_offset_gesture_update(
        &mut self,
        delta_x: f64,
        timestamp: Duration,
        is_touchpad: bool,
    ) -> Option<bool> {
        let ViewOffset::Gesture(gesture) = &mut self.view_offset else {
            return None;
        };

        if gesture.is_touchpad != is_touchpad || gesture.dnd_last_event_time.is_some() {
            return None;
        }

        gesture.tracker.push(delta_x, timestamp);

        let norm_factor = if gesture.is_touchpad {
            self.working_area.size.w / VIEW_GESTURE_WORKING_AREA_MOVEMENT
        } else {
            1.
        };
        let pos = gesture.tracker.pos() * norm_factor;
        let view_offset = pos + gesture.delta_from_tracker;
        gesture.current_view_offset = view_offset;

        Some(true)
    }

    pub fn dnd_scroll_gesture_scroll(&mut self, delta: f64) -> bool {
        let bounds = self.dnd_scroll_bounds();

        let ViewOffset::Gesture(gesture) = &mut self.view_offset else {
            return false;
        };

        let Some(last_time) = gesture.dnd_last_event_time else {
            // Not a DnD scroll.
            return false;
        };

        let config = &self.options.gestures.dnd_edge_view_scroll;

        let now = self.clock.now_unadjusted();
        gesture.dnd_last_event_time = Some(now);

        if delta == 0. {
            // We're outside the scrolling zone.
            gesture.dnd_nonzero_start_time = None;
            return false;
        }

        let nonzero_start = *gesture.dnd_nonzero_start_time.get_or_insert(now);

        // Delay starting the gesture a bit to avoid unwanted movement when dragging across
        // monitors.
        let delay = Duration::from_millis(u64::from(config.delay_ms));
        if now.saturating_sub(nonzero_start) < delay {
            return true;
        }

        let time_delta = now.saturating_sub(last_time).as_secs_f64();

        let delta = delta * time_delta * config.max_speed;

        gesture.tracker.push(delta, now);

        let view_offset = gesture.tracker.pos() + gesture.delta_from_tracker;

        // Clamp it so that it doesn't go too much out of bounds.
        let (leftmost, rightmost) = bounds;
        let min_offset = f64::min(leftmost, rightmost);
        let max_offset = f64::max(leftmost, rightmost);
        let clamped_offset = view_offset.clamp(min_offset, max_offset);

        gesture.delta_from_tracker += clamped_offset - view_offset;
        gesture.current_view_offset = clamped_offset;
        true
    }

    /// How far DnD scrolling can move the view: from a working area width left of the leftmost
    /// window, to the left edge of the working area reaching the right edge of the rightmost
    /// window.
    fn dnd_scroll_bounds(&self) -> (f64, f64) {
        if self.columns.is_empty() {
            return (0., 0.);
        }

        let col_xs = || zip(self.column_xs(self.data.iter().copied()), &self.data);
        let left_edge = col_xs()
            .map(|(col_x, _)| col_x)
            .fold(f64::INFINITY, f64::min);
        let leftmost = left_edge - self.working_area.size.w;
        let right_edge = col_xs()
            .map(|(col_x, data)| col_x + data.width)
            .fold(f64::NEG_INFINITY, f64::max);
        let rightmost = right_edge - self.working_area.loc.x;
        (leftmost, rightmost)
    }

    pub fn view_offset_gesture_end(&mut self, is_touchpad: Option<bool>) -> bool {
        let ViewOffset::Gesture(gesture) = &mut self.view_offset else {
            return false;
        };

        if is_touchpad.is_some_and(|x| gesture.is_touchpad != x) {
            return false;
        }

        // We do not handle cancelling, just like GNOME Shell doesn't. For this gesture, proper
        // cancelling would require keeping track of the original active column, and then updating
        // it in all the right places (adding columns, removing columns, etc.) -- quite a bit of
        // effort and bug potential.

        // Take into account any idle time between the last event and now.
        let now = self.clock.now_unadjusted();
        gesture.tracker.push(0., now);

        let norm_factor = if gesture.is_touchpad {
            self.working_area.size.w / VIEW_GESTURE_WORKING_AREA_MOVEMENT
        } else {
            1.
        };
        let velocity = gesture.tracker.velocity() * norm_factor;
        let pos = gesture.tracker.pos() * norm_factor;
        let current_view_offset = pos + gesture.delta_from_tracker;

        if self.columns.is_empty() {
            self.view_offset = ViewOffset::Static(current_view_offset);
            return true;
        }

        // Figure out where the gesture would stop after deceleration.
        let end_pos = gesture.tracker.projected_end_pos() * norm_factor;
        let target_view_offset = end_pos + gesture.delta_from_tracker;

        // Compute the snapping points. These are where the view aligns with window edges on
        // either side, with windows at their X on the grid.
        struct Snap {
            // View position relative to x = 0 (the left edge of the grid).
            view_pos: f64,
            // Column to activate for this snapping point.
            col_idx: usize,
        }

        let col_xs: Vec<f64> = self
            .column_xs(self.data.iter().copied())
            .take(self.columns.len())
            .collect();
        let mut snapping_points = Vec::new();

        if self.is_centering_focused_column() {
            for (col_idx, col) in self.columns.iter().enumerate() {
                let col_x = col_xs[col_idx];
                let col_w = col.width();
                let mode = col.sizing_mode();

                let area = if mode.is_maximized() {
                    self.parent_area
                } else {
                    self.working_area
                };

                let left_strut = area.loc.x;

                let view_pos = if mode.is_fullscreen() {
                    col_x
                } else if area.size.w <= col_w {
                    col_x - left_strut
                } else {
                    col_x - (area.size.w - col_w) / 2. - left_strut
                };
                snapping_points.push(Snap { view_pos, col_idx });
            }
        } else {
            let center_on_overflow = matches!(
                self.options.layout.center_focused_column,
                CenterFocusedColumn::OnOverflow
            );

            let view_width = self.view_size.w;
            let gaps = self.options.layout.gaps;

            // Width of the window next to the column in `dir` on the grid, if any.
            let adjacent_w = |col: &Column<W>, dir| {
                let id = self.grid.adjacent(&col.id, dir, None)?;
                self.columns
                    .iter()
                    .find(|col| col.id == *id)
                    .map(|col| col.width())
            };

            let snap_points = |col_x, col: &Column<W>| {
                let col_w = col.width();
                let mode = col.sizing_mode();

                let area = if mode.is_maximized() {
                    self.parent_area
                } else {
                    self.working_area
                };

                let left_strut = area.loc.x;
                let right_strut = self.view_size.w - area.size.w - area.loc.x;

                // Normal columns align with the working area, but fullscreen columns align with
                // the view size.
                if mode.is_fullscreen() {
                    let left = col_x;
                    let right = left + col_w;
                    (left, right)
                } else {
                    // Logic from compute_new_view_offset.
                    let padding = if mode.is_maximized() {
                        0.
                    } else {
                        ((area.size.w - col_w) / 2.).clamp(0., gaps)
                    };

                    let center = if area.size.w <= col_w {
                        col_x - left_strut
                    } else {
                        col_x - (area.size.w - col_w) / 2. - left_strut
                    };
                    let is_overflowing = |adj_col_w: Option<f64>| {
                        // NOTE: This logic won't work entirely correctly with small fixed-size
                        // maximized windows (they have a different area and padding).
                        center_on_overflow
                            && adj_col_w.is_some_and(|adj_col_w| {
                                adj_col_w + 3.0 * gaps + col_w > area.size.w
                            })
                    };

                    let left = if is_overflowing(adjacent_w(col, grid::Direction::Right)) {
                        center
                    } else {
                        col_x - padding - left_strut
                    };
                    let right = if is_overflowing(adjacent_w(col, grid::Direction::Left)) {
                        center + view_width
                    } else {
                        col_x + col_w + padding + right_strut
                    };
                    (left, right - view_width)
                }
            };

            let snaps: Vec<(f64, f64)> = self
                .columns
                .iter()
                .enumerate()
                .map(|(col_idx, col)| snap_points(col_xs[col_idx], col))
                .collect();

            // Prevent the gesture from snapping further than the leftmost or rightmost window, as
            // this is generally undesired.
            //
            // It's ok if leftmost_snap is > rightmost_snap (this happens if the windows on a
            // workspace span less than the workspace width).
            let (leftmost_idx, leftmost_snap) = snaps
                .iter()
                .map(|(left, _)| *left)
                .enumerate()
                .min_by_key(|(_, left)| NotNan::new(*left).unwrap())
                .unwrap();
            let (rightmost_idx, rightmost_snap) = snaps
                .iter()
                .map(|(_, right)| *right)
                .enumerate()
                .max_by_key(|(_, right)| NotNan::new(*right).unwrap())
                .unwrap();

            snapping_points.push(Snap {
                view_pos: leftmost_snap,
                col_idx: leftmost_idx,
            });
            snapping_points.push(Snap {
                view_pos: rightmost_snap,
                col_idx: rightmost_idx,
            });

            for (col_idx, (left, right)) in snaps.into_iter().enumerate() {
                for view_pos in [left, right] {
                    if leftmost_snap < view_pos && view_pos < rightmost_snap {
                        snapping_points.push(Snap { view_pos, col_idx });
                    }
                }
            }
        }

        // Find the closest snapping point.
        let target_snap = snapping_points
            .iter()
            .min_by_key(|snap| NotNan::new((snap.view_pos - target_view_offset).abs()).unwrap())
            .unwrap();

        let mut new_col_idx = target_snap.col_idx;

        if !self.is_centering_focused_column() {
            // Focus the furthest window towards the direction of the gesture that is fully visible
            // after snapping, preferring the row the user was in.
            let view_pos = target_snap.view_pos;
            let is_visible = |col_idx: usize| {
                let col = &self.columns[col_idx];
                let col_x = col_xs[col_idx];
                let col_w = col.width();
                let mode = col.sizing_mode();

                if mode.is_fullscreen() {
                    return view_pos <= col_x && col_x + col_w <= view_pos + self.view_size.w;
                }

                let area = if mode.is_maximized() {
                    self.parent_area
                } else {
                    self.working_area
                };
                let padding = if mode.is_maximized() {
                    0.
                } else {
                    ((area.size.w - col_w) / 2.).clamp(0., self.options.layout.gaps)
                };

                view_pos + area.loc.x <= col_x - padding
                    && col_x + col_w + padding <= view_pos + area.loc.x + area.size.w
            };

            let row = self.columns.get(self.active_column_idx).and_then(|col| {
                let anchor = self.focus_anchor.filter(|anchor| anchor.column == col.id);
                let area = self.grid.area(&col.id)?;
                Some(anchor.and_then(|anchor| anchor.row).unwrap_or(area.row))
            });
            let in_row = |col_idx: usize| {
                let area = self.grid.area(&self.columns[col_idx].id);
                row.zip(area)
                    .is_some_and(|(row, area)| area.row <= row && row < area.bottom())
            };

            let forward = target_view_offset >= current_view_offset;
            let furthest = |in_row_only: bool| {
                (0..self.columns.len())
                    .filter(|&idx| is_visible(idx) && (!in_row_only || in_row(idx)))
                    .max_by_key(|&idx| {
                        let left = NotNan::new(col_xs[idx]).unwrap();
                        let right = NotNan::new(col_xs[idx] + self.columns[idx].width()).unwrap();
                        if forward {
                            (right, left)
                        } else {
                            (-left, -right)
                        }
                    })
            };
            if let Some(idx) = furthest(true).or_else(|| furthest(false)) {
                new_col_idx = idx;
            }
        }

        if self.active_column_idx != new_col_idx {
            self.view_offset_to_restore = None;
            self.activate_prev_column_on_removal = None;
        }

        self.active_column_idx = new_col_idx;

        self.view_offset = ViewOffset::Animation(Animation::new(
            self.clock.clone(),
            current_view_offset,
            target_snap.view_pos,
            velocity,
            self.options.animations.horizontal_view_movement.0,
        ));

        // HACK: deal with things like snapping to the right edge of a larger-than-view window.
        self.animate_view_offset_to_column(None, new_col_idx, None);

        true
    }

    pub fn dnd_scroll_gesture_end(&mut self) {
        let ViewOffset::Gesture(gesture) = &mut self.view_offset else {
            return;
        };

        if gesture.dnd_last_event_time.is_some() && gesture.tracker.pos() == 0. {
            // DnD didn't scroll anything, so preserve the current view position (rather than
            // snapping the window).

            // If there's an ongoing animation within the gesture (e.g. from a window being removed
            // during DnD), preserve it.
            if let Some(mut anim) = gesture.animation.take() {
                anim.offset(gesture.current_view_offset);
                self.view_offset = ViewOffset::Animation(anim);
            } else {
                self.view_offset = ViewOffset::Static(gesture.delta_from_tracker);
            }

            if !self.columns.is_empty() {
                // Just in case, make sure the active window remains on screen.
                self.animate_view_offset_to_column(None, self.active_column_idx, None);
            }
            return;
        }

        self.view_offset_gesture_end(None);
    }

    pub fn interactive_resize_begin(&mut self, window: W::Id, edges: ResizeEdge) -> bool {
        if self.interactive_resize.is_some() {
            return false;
        }

        let col = self
            .columns
            .iter_mut()
            .find(|col| col.contains(&window))
            .unwrap();

        if !col.pending_sizing_mode().is_normal() {
            return false;
        }
        let col_id = col.id;

        let tile = col
            .tiles
            .iter_mut()
            .find(|tile| tile.window().id() == &window)
            .unwrap();

        let original_window_size = tile.window_size();

        let resize = InteractiveResize {
            window,
            original_window_size,
            data: InteractiveResizeData { edges },
        };
        self.interactive_resize = Some(resize);

        // The window being resized is held by the pointer, so gravity leaves it alone until the
        // resize ends.
        self.grid.set_pinned(Some(col_id));
        self.grid_resize_original = Some(self.grid.clone());

        self.view_offset.stop_anim_and_gesture();

        true
    }

    pub fn interactive_resize_update(
        &mut self,
        window: &W::Id,
        delta: Point<f64, Logical>,
    ) -> bool {
        let Some(resize) = &self.interactive_resize else {
            return false;
        };

        if window != &resize.window {
            return false;
        }

        if GRID_LAYOUT {
            return self.interactive_resize_grid(window, delta);
        }

        let is_centering = self.is_centering_focused_column();

        let col = self
            .columns
            .iter_mut()
            .find(|col| col.contains(window))
            .unwrap();

        let tile_idx = col
            .tiles
            .iter()
            .position(|tile| tile.window().id() == window)
            .unwrap();

        if resize.data.edges.intersects(ResizeEdge::LEFT_RIGHT) {
            let mut dx = delta.x;
            if resize.data.edges.contains(ResizeEdge::LEFT) {
                dx = -dx;
            };

            if is_centering {
                dx *= 2.;
            }

            let window_width = (resize.original_window_size.w + dx).round() as i32;
            col.set_column_width(SizeChange::SetFixed(window_width), Some(tile_idx), false);
        }

        if resize.data.edges.intersects(ResizeEdge::TOP_BOTTOM) {
            // Prevent the simplest case of weird resizing (top edge when this is the topmost
            // window).
            if !(resize.data.edges.contains(ResizeEdge::TOP) && tile_idx == 0) {
                let mut dy = delta.y;
                if resize.data.edges.contains(ResizeEdge::TOP) {
                    dy = -dy;
                };

                // FIXME: some smarter height distribution would be nice here so that vertical
                // resizes work as expected in more cases.

                let window_height = (resize.original_window_size.h + dy).round() as i32;
                col.set_window_height(SizeChange::SetFixed(window_height), Some(tile_idx), false);
            }
        }

        true
    }

    pub fn interactive_resize_end(&mut self, window: Option<&W::Id>) {
        let Some(resize) = &self.interactive_resize else {
            return;
        };

        if let Some(window) = window {
            if window != &resize.window {
                return;
            }

            // Animate the active window into view right away.
            if self.columns[self.active_column_idx].contains(window) {
                self.animate_view_offset_to_column(None, self.active_column_idx, None);
            }
        }

        self.interactive_resize = None;
        self.grid_resize_original = None;
        self.release_grid_pin();
    }

    pub fn refresh(&mut self, is_active: bool, is_focused: bool) {
        // A resize can end without interactive_resize_end(), for example when focus moves away.
        if self.interactive_resize.is_none() {
            self.release_grid_pin();
        }

        for (col_idx, col) in self.columns.iter_mut().enumerate() {
            let mut col_resize_data = None;
            if let Some(resize) = &self.interactive_resize {
                if col.contains(&resize.window) {
                    col_resize_data = Some(resize.data);
                }
            }

            let is_tabbed = col.display_mode == ColumnDisplay::Tabbed;
            let extra_size = col.extra_size();

            // If transactions are disabled, also disable combined throttling, for more intuitive
            // behavior. In tabbed display mode, only one window is visible, so individual
            // throttling makes more sense.
            let individual_throttling = self.options.disable_transactions || is_tabbed;

            let intent = if self.options.disable_resize_throttling {
                ConfigureIntent::CanSend
            } else if individual_throttling {
                // In this case, we don't use combined throttling, but rather compute throttling
                // individually below.
                ConfigureIntent::CanSend
            } else {
                col.tiles
                    .iter()
                    .fold(ConfigureIntent::NotNeeded, |intent, tile| {
                        match (intent, tile.window().configure_intent()) {
                            (_, ConfigureIntent::ShouldSend) => ConfigureIntent::ShouldSend,
                            (ConfigureIntent::NotNeeded, tile_intent) => tile_intent,
                            (ConfigureIntent::CanSend, ConfigureIntent::Throttled) => {
                                ConfigureIntent::Throttled
                            }
                            (intent, _) => intent,
                        }
                    })
            };

            for (tile_idx, tile) in col.tiles.iter_mut().enumerate() {
                let win = tile.window_mut();

                let active_in_column = col.active_tile_idx == tile_idx;
                win.set_active_in_column(active_in_column);
                win.set_floating(false);

                let mut active = is_active && self.active_column_idx == col_idx;
                if self.options.deactivate_unfocused_windows {
                    active &= active_in_column && is_focused;
                } else {
                    // In tabbed mode, all tabs have activated state to reduce unnecessary
                    // animations when switching tabs.
                    active &= active_in_column || is_tabbed;
                }
                win.set_activated(active);

                win.set_interactive_resize(col_resize_data);

                let border_config = self.options.layout.border.merged_with(&win.rules().border);
                let bounds = compute_toplevel_bounds(
                    border_config,
                    self.working_area.size,
                    extra_size,
                    self.options.layout.gaps,
                );
                win.set_bounds(bounds);

                let intent = if individual_throttling {
                    win.configure_intent()
                } else {
                    intent
                };

                if matches!(
                    intent,
                    ConfigureIntent::CanSend | ConfigureIntent::ShouldSend
                ) {
                    win.send_pending_configure();
                }

                win.refresh();
            }
        }
    }

    #[cfg(test)]
    pub fn view_size(&self) -> Size<f64, Logical> {
        self.view_size
    }

    #[cfg(test)]
    pub fn parent_area(&self) -> Rectangle<f64, Logical> {
        self.parent_area
    }

    #[cfg(test)]
    pub fn clock(&self) -> &Clock {
        &self.clock
    }

    #[cfg(test)]
    pub fn options(&self) -> &Rc<Options> {
        &self.options
    }

    #[cfg(test)]
    pub fn active_column_idx(&self) -> usize {
        self.active_column_idx
    }

    #[cfg(test)]
    pub(super) fn view_offset(&self) -> &ViewOffset {
        &self.view_offset
    }

    #[cfg(test)]
    pub fn verify_invariants(&self) {
        assert!(self.view_size.w > 0.);
        assert!(self.view_size.h > 0.);
        assert!(self.scale > 0.);
        assert!(self.scale.is_finite());
        assert_eq!(self.columns.len(), self.data.len());
        assert_eq!(
            self.working_area,
            compute_working_area(self.parent_area, self.scale, self.options.layout.struts)
        );

        if !self.columns.is_empty() {
            assert!(self.active_column_idx < self.columns.len());

            for (column, data) in zip(&self.columns, &self.data) {
                assert!(Rc::ptr_eq(&self.options, &column.options));
                assert_eq!(self.clock, column.clock);
                assert_eq!(self.scale, column.scale);
                column.verify_invariants();

                let mut data2 = *data;
                data2.update(column);
                assert_eq!(data, &data2, "column data must be up to date");
            }

            let col = &self.columns[self.active_column_idx];

            if let Some((id, _)) = self.view_offset_to_restore {
                assert_eq!(
                    id, col.id,
                    "view_offset_to_restore must be for the active column"
                );
                assert!(
                    !col.sizing_mode().is_normal(),
                    "when view_offset_to_restore is set, \
                     the active column must be fullscreen or maximized"
                );
            }

            if let Some(prev) = &self.activate_prev_column_on_removal {
                assert_eq!(
                    prev.opened, col.id,
                    "activate_prev_column_on_removal must be for the active column"
                );
                assert!(
                    self.columns.iter().any(|col| col.id == prev.prev),
                    "activate_prev_column_on_removal must point to an existing column"
                );
            }
        }

        if let Some(resize) = &self.interactive_resize {
            assert!(
                self.columns
                    .iter()
                    .flat_map(|col| &col.tiles)
                    .any(|tile| tile.window().id() == &resize.window),
                "interactive resize window must be present in the layout"
            );
        }

        if GRID_LAYOUT {
            self.verify_grid_invariants();
        }
    }
}

/// bento's cell grid.
impl<W: LayoutElement> ScrollingSpace<W> {
    /// X of a grid column; the first column starts at 0, like niri's columns.
    fn grid_x(&self, col: i32) -> f64 {
        let m = &self.cell_metrics;
        round_logical_in_physical(self.scale, f64::from(col) * (m.cell_w + m.gap))
    }

    /// Tile geometry for a grid area.
    fn grid_geometry(&self, area: Area) -> GridGeometry {
        let rect = self.cell_metrics.rect(area);

        // Round the edges rather than the sizes so that neighbors line up exactly.
        let left = self.grid_x(area.col);
        let right = self.grid_x(area.right()) - self.cell_metrics.gap;
        let top = round_logical_in_physical(self.scale, rect.loc.y);
        let bottom = round_logical_in_physical(self.scale, rect.loc.y + rect.size.h);

        // With gaps too large to fit the rows, cells bottom out at 1 px and the rows overflow.
        // Keep tiles within the working area like niri does.
        let max_h = (self.working_area.size.h - self.cell_metrics.gap * 2.).max(1.);

        GridGeometry {
            y: top,
            size: Size::from((right - left, (bottom - top).min(max_h))),
        }
    }

    /// Number of grid columns for a column width.
    fn grid_cols_for_width(&self, width: ColumnWidth) -> i32 {
        let m = &self.cell_metrics;
        let cols = match width {
            ColumnWidth::Proportion(proportion) => m.cols_for_proportion(proportion),
            ColumnWidth::Fixed(width) => ((width + m.gap) / (m.cell_w + m.gap)).round() as i32,
        };
        cols.clamp(1, grid::MAX_COLS)
    }

    /// Number of grid columns for a preset width.
    ///
    /// Like in niri, a fixed width is for the window, so `borders` is added to it: the width of
    /// the borders on both sides.
    fn grid_cols_for_preset(&self, preset: PresetSize, borders: f64) -> i32 {
        match preset {
            PresetSize::Fixed(width) => {
                self.grid_cols_for_width(ColumnWidth::Fixed(f64::from(width) + borders))
            }
            PresetSize::Proportion(_) => self.grid_cols_for_width(ColumnWidth::from(preset)),
        }
    }

    /// Number of grid rows for a preset height.
    ///
    /// Like in niri, a fixed height is for the window, so `borders` is added to it: the height of
    /// the borders on both sides.
    fn grid_rows_for_preset(&self, preset: PresetSize, borders: f64) -> i32 {
        let m = &self.cell_metrics;
        match preset {
            PresetSize::Proportion(proportion) => m.rows_for_proportion(proportion),
            PresetSize::Fixed(height) => {
                let rows = (f64::from(height) + borders + m.gap) / (m.cell_h + m.gap);
                (rows.round() as i32).clamp(1, m.rows)
            }
        }
    }

    /// Number of grid columns for a span of a window with `borders`.
    fn grid_cols_for_span(&self, span: GridSpan, borders: f64) -> i32 {
        match span {
            GridSpan::Cells { n, of } => {
                scale_span(n, of, self.cell_metrics.cols_per_view).clamp(1, grid::MAX_COLS)
            }
            GridSpan::Preset(preset) => self.grid_cols_for_preset(preset, borders),
        }
    }

    /// Number of grid rows for a span of a window with `borders`.
    fn grid_rows_for_span(&self, span: GridSpan, borders: f64) -> i32 {
        let rows = self.cell_metrics.rows;
        match span {
            GridSpan::Cells { n, of } => scale_span(n, of, rows).clamp(1, rows),
            GridSpan::Preset(preset) => self.grid_rows_for_preset(preset, borders),
        }
    }

    /// Grid size in (columns, rows) that a column should have, from the size of its window.
    fn grid_span_for(&self, column: &Column<W>) -> (i32, i32) {
        self.grid_span_for_size(&column.tiles[0], column.width, column.is_full_width)
    }

    /// Grid size in (columns, rows) that a column takes in its requested sizing mode: its own
    /// size, or when fullscreen or maximized, enough columns to cover its width and all rows.
    fn grid_span_in_mode(&self, column: &Column<W>) -> (i32, i32) {
        let m = &self.cell_metrics;
        let width = match column.pending_sizing_mode() {
            SizingMode::Normal => return self.grid_span_for(column),
            SizingMode::Fullscreen => self.view_size.w,
            SizingMode::Maximized => self.parent_area.size.w,
        };

        // Cover the width and the gap after it, so that the next window starts past it.
        let cols = ((width + m.gap) / (m.cell_w + m.gap)).ceil() as i32;
        (cols.clamp(1, grid::MAX_COLS), m.rows)
    }

    /// Grid size in (columns, rows) for the window of `tile` in a column of `width`.
    fn grid_span_for_size(
        &self,
        tile: &Tile<W>,
        width: ColumnWidth,
        is_full_width: bool,
    ) -> (i32, i32) {
        let size = tile.grid_size;
        let cols = if is_full_width {
            self.cell_metrics.cols_per_view
        } else {
            let borders = tile.tile_width_for_window_width(0.);
            size.cols.map_or_else(
                || self.grid_cols_for_width(width),
                |span| self.grid_cols_for_span(span, borders),
            )
        };
        let borders = tile.tile_height_for_window_height(0.);
        let rows = size.rows.map_or(self.cell_metrics.rows, |span| {
            self.grid_rows_for_span(span, borders)
        });
        (cols, rows)
    }

    /// Area of `cols` × `rows` cells nearest to a window with its top-left corner at `pos`,
    /// relative to the view.
    fn grid_area_at(&self, pos: Point<f64, Logical>, cols: i32, rows: i32) -> Area {
        let m = &self.cell_metrics;

        // Top-left corner of the window in strip coordinates.
        let x = pos.x + self.view_pos();
        let y = pos.y - self.working_area.loc.y - m.gap;

        let col = (x / (m.cell_w + m.gap)).round() as i32;
        let row = ((y / (m.cell_h + m.gap)).round() as i32).clamp(0, m.rows - rows);
        Area::new(col, row, cols, rows)
    }

    /// Current position of every column, for animating grid changes.
    fn grid_positions(&self) -> Vec<(ColumnId, Point<f64, Logical>)> {
        zip(&self.columns, self.column_xs(self.data.iter().copied()))
            .map(|(col, x)| (col.id, Point::from((x, col.tiles_origin().y))))
            .collect()
    }

    /// Brings the columns in line with the grid after it changed.
    ///
    /// Sorts the columns by grid position and updates their geometry. If `prev_positions` is set,
    /// animates tile sizes and columns that moved from there.
    fn sync_grid(
        &mut self,
        prev_positions: Option<Vec<(ColumnId, Point<f64, Logical>)>>,
        anim_config: Option<niri_config::Animation>,
    ) {
        let active_id = self.columns.get(self.active_column_idx).map(|col| col.id);

        // Sort top-left first, so that the order is stable. Nothing should rely on it meaning
        // anything spatial beyond that: neighbors in the order can be anywhere on the grid.
        let grid = &self.grid;
        let key = |col: &Column<W>| grid.area(&col.id).map(|a| (a.col, a.row));
        if !self.columns.is_sorted_by_key(key) {
            let mut order: Vec<_> = (0..self.columns.len()).collect();
            order.sort_by_key(|&i| key(&self.columns[i]));

            let mut columns: Vec<_> = self.columns.drain(..).map(Some).collect();
            self.columns = order.iter().map(|&i| columns[i].take().unwrap()).collect();
            self.data = order.iter().map(|&i| self.data[i]).collect();
        }

        if let Some(id) = active_id {
            self.active_column_idx = self.columns.iter().position(|col| col.id == id).unwrap();
        }

        let animate = prev_positions.is_some();
        for idx in 0..self.columns.len() {
            let Some(area) = self.grid.area(&self.columns[idx].id) else {
                continue;
            };
            let geometry = self.grid_geometry(area);

            // The grid has the final say on sizes: if the window's size resolves to a different
            // span, the user resized it, so remember the new size. Fullscreen and maximized
            // windows take more space for the time being.
            let col = &self.columns[idx];
            let (cols, rows) =
                if col.sizing_mode().is_normal() && col.pending_sizing_mode().is_normal() {
                    self.grid_span_for(col)
                } else {
                    (area.cols, area.rows)
                };
            let m = &self.cell_metrics;
            let cols_span = (area.cols != cols).then_some(GridSpan::Cells {
                n: area.cols,
                of: m.cols_per_view,
            });
            let rows_span = (area.rows != rows).then_some(GridSpan::Cells {
                n: area.rows,
                of: m.rows,
            });

            let col = &mut self.columns[idx];
            if cols_span.is_some() {
                col.is_full_width = false;
                col.tiles[0].grid_size.cols = cols_span;
            }
            if rows_span.is_some() {
                col.tiles[0].grid_size.rows = rows_span;
            }

            // Tabs share the area, so they all have its size, and keep it if they leave.
            let size = col.tiles[0].grid_size;
            for tile in &mut col.tiles[1..] {
                tile.grid_size = size;
            }

            col.set_grid_geometry(geometry, (area.cols, area.rows), animate);
            self.data[idx].update(col);
        }

        let Some(prev_positions) = prev_positions else {
            return;
        };

        let config = anim_config.unwrap_or(self.options.animations.window_movement.0);
        for (idx, (id, pos)) in self.grid_positions().into_iter().enumerate() {
            let Some((_, prev)) = prev_positions.iter().find(|(prev_id, _)| *prev_id == id) else {
                continue;
            };

            let col = &mut self.columns[idx];
            if prev.x != pos.x {
                col.animate_move_x_from_with_config(prev.x - pos.x, config);
            }
            if prev.y != pos.y {
                col.animate_move_y_from_with_config(prev.y - pos.y, config);
            }
        }
    }

    /// Focuses the nearest window in `dir`.
    fn focus_grid(&mut self, dir: grid::Direction) -> bool {
        self.focus_grid_in(dir, false)
    }

    /// Focuses the nearest window in `dir`, or with `adjacent`, only one sharing rows or columns
    /// of cells with the active window.
    fn focus_grid_in(&mut self, dir: grid::Direction, adjacent: bool) -> bool {
        let Some(id) = self.columns.get(self.active_column_idx).map(|col| col.id) else {
            return false;
        };
        let Some(area) = self.grid.area(&id) else {
            return false;
        };

        let anchor = self.focus_anchor.filter(|anchor| anchor.column == id);
        let row = anchor.and_then(|anchor| anchor.row);
        let col = anchor.and_then(|anchor| anchor.col);

        let along = if dir.is_horizontal() { row } else { col };
        let target = if adjacent {
            self.grid.adjacent(&id, dir, along)
        } else {
            self.grid.neighbor(&id, dir, along)
        };
        let Some(&target) = target else {
            return false;
        };
        let target_area = self.grid.area(&target).unwrap();

        // Keep travelling along the same row or column, and remember which side of the new
        // window we entered from.
        let (row, col) = match dir {
            grid::Direction::Right => (row.unwrap_or(area.row), target_area.col),
            grid::Direction::Left => (row.unwrap_or(area.row), target_area.right() - 1),
            grid::Direction::Down => (target_area.row, col.unwrap_or(area.col)),
            grid::Direction::Up => (target_area.bottom() - 1, col.unwrap_or(area.col)),
        };

        let idx = self
            .columns
            .iter()
            .position(|col| col.id == target)
            .unwrap();
        self.activate_column(idx);
        self.focus_anchor = Some(FocusAnchor {
            column: target,
            row: Some(row),
            col: Some(col),
        });
        true
    }

    /// Moves the active window by one cell in `dir`.
    fn move_grid(&mut self, dir: grid::Direction) -> bool {
        let Some(id) = self.columns.get(self.active_column_idx).map(|col| col.id) else {
            return false;
        };

        let prev_positions = self.grid_positions();
        if !self.grid.move_window(&id, dir) {
            return false;
        }

        self.finish_grid_move(prev_positions);
        true
    }

    /// Gives a column that is going fullscreen or maximized enough cells to cover its width,
    /// across all rows, or gives it back its own size when it's going back to normal.
    ///
    /// This happens as the change is requested, so that the window is configured right away
    /// with the size it ends up with.
    fn fit_grid_to_sizing_mode(&mut self, idx: usize) {
        let id = self.columns[idx].id;
        let area = self.grid.area(&id).unwrap();

        let (cols, rows) = self.grid_span_in_mode(&self.columns[idx]);
        let new_area = if self.columns[idx].pending_sizing_mode().is_normal() {
            let row = self.columns[idx].grid_row_to_restore.take();
            Area::new(area.col, row.unwrap_or(area.row), cols, rows)
        } else {
            self.columns[idx]
                .grid_row_to_restore
                .get_or_insert(area.row);
            Area::new(area.col, 0, cols, rows)
        };
        if new_area == area {
            return;
        }

        let prev_positions = self.grid_positions();
        self.grid.set_area(&id, new_area, grid::Direction::Right);
        self.sync_grid(Some(prev_positions), None);
    }

    /// Grows the active window into the empty cells beside it in its rows, right and left, up to
    /// the next window or the edge of the view, without pushing anything.
    fn expand_grid_to_available_width(&mut self) {
        let Some(col) = self.columns.get(self.active_column_idx) else {
            return;
        };
        if !col.pending_sizing_mode().is_normal() || col.is_full_width {
            return;
        }
        let id = col.id;
        let area = self.grid.area(&id).unwrap();

        // Columns of cells fully inside the working area, once the view gets where it's going.
        let m = &self.cell_metrics;
        let step = m.cell_w + m.gap;
        let view_left = self.target_view_pos() + self.working_area.loc.x;
        let view_right = view_left + self.working_area.size.w;
        let first_visible = (view_left / step).ceil() as i32;
        let end_visible = ((view_right - m.cell_w) / step).floor() as i32 + 1;

        // Edges of the nearest windows on either side in the same rows.
        let in_rows = |b: &Area| b.row < area.bottom() && area.row < b.bottom();
        let others = || {
            self.grid
                .iter()
                .filter(|(other, b)| **other != id && in_rows(b))
        };
        let right_limit = others()
            .map(|(_, b)| b.col)
            .filter(|col| *col >= area.right())
            .min()
            .unwrap_or(i32::MAX);
        // With gravity, the grid starts at column 0. Without, it goes on to the left too.
        let left_limit = others()
            .map(|(_, b)| b.right())
            .filter(|right| *right <= area.col)
            .max()
            .unwrap_or(if self.grid.left_gravity() {
                0
            } else {
                i32::MIN
            });

        let left = area.col.min(left_limit.max(first_visible));
        let right = area.right().max(right_limit.min(end_visible));
        let new_area = Area {
            col: left,
            cols: right - left,
            ..area
        };
        if new_area == area {
            return;
        }

        let col = &mut self.columns[self.active_column_idx];
        cancel_resize_for_column(&mut self.interactive_resize, col);

        let prev_positions = self.grid_positions();
        self.grid.set_area(&id, new_area, grid::Direction::Right);
        self.sync_grid(Some(prev_positions), None);
        if !self.view_offset.is_gesture() {
            self.animate_view_offset_to_column(None, self.active_column_idx, None);
        }
    }

    /// Expels `window` (or the active one) from its stack towards `dir`, or if it's not stacked
    /// with other windows, consumes it into the stack of the window beside it towards `dir`.
    fn consume_or_expel_grid(&mut self, window: Option<&W::Id>, dir: grid::Direction) {
        let Some(idx) = self.grid_column_idx(window) else {
            return;
        };
        let id = self.columns[idx].id;
        let area = self.grid.area(&id).unwrap();

        // A tab leaves its tabbed area.
        let col = &self.columns[idx];
        if col.tiles.len() > 1 {
            let tile_idx =
                window.map_or(col.active_tile_idx, |window| col.position(window).unwrap());
            self.change_grid(|this| this.expel_grid_tab(id, tile_idx, dir));
            return;
        }

        let stacked = [grid::Direction::Up, grid::Direction::Down]
            .into_iter()
            .any(|dir| self.grid.adjacent(&id, dir, None).is_some());
        if stacked {
            self.change_grid(|this| this.expel_grid(id, dir));
            return;
        }

        let anchor = self
            .focus_anchor
            .filter(|anchor| anchor.column == id)
            .and_then(|anchor| anchor.row)
            .unwrap_or(area.row);
        if let Some(&into) = self.grid.adjacent(&id, dir, Some(anchor)) {
            self.change_grid(|this| this.consume_grid(id, into));
        }
    }

    /// Puts `window` in the column of cells of `into`, which keeps the top half of its rows,
    /// giving `window` the bottom half, like consuming a window into a column in niri.
    ///
    /// Returns whether anything changed.
    fn consume_grid(&mut self, window: ColumnId, into: ColumnId) -> bool {
        if window == into {
            return false;
        }

        // A tabbed area takes the window as another tab.
        let into_col = self.columns.iter().find(|col| col.id == into).unwrap();
        if into_col.display_mode == ColumnDisplay::Tabbed {
            return self.consume_grid_as_tab(window, into);
        }

        if self.grid.area(&into).unwrap().rows < 2 {
            return false;
        }

        // Take the window out first so that its cells don't get in the way. Gravity can move
        // the target then, so look it up again after every change.
        self.grid.remove(&window);
        let target = self.grid.area(&into).unwrap();
        let top = (target.rows + 1) / 2;
        let bottom = target.rows - top;
        self.grid.set_area(
            &into,
            Area {
                rows: top,
                ..target
            },
            grid::Direction::Up,
        );

        let target = self.grid.area(&into).unwrap();
        let area = Area::new(target.col, target.bottom(), target.cols, bottom);
        self.grid.insert(window, area);
        true
    }

    /// Takes `window` out of its stack, full height beside it towards `dir`, and gives its rows
    /// to the window it was split from, like expelling a window from a column in niri.
    ///
    /// Returns whether anything changed.
    fn expel_grid(&mut self, window: ColumnId, dir: grid::Direction) -> bool {
        let area = self.grid.area(&window).unwrap();

        // The window right above or below in exactly the same columns, if any, gets the rows
        // back.
        let same_columns = |id: &ColumnId| {
            let other = self.grid.area(id).unwrap();
            other.col == area.col && other.cols == area.cols
        };
        let partner = [grid::Direction::Up, grid::Direction::Down]
            .into_iter()
            .filter_map(|dir| self.grid.adjacent(&window, dir, None).copied())
            .find(same_columns);
        if partner.is_none()
            && [grid::Direction::Up, grid::Direction::Down]
                .into_iter()
                .all(|dir| self.grid.adjacent(&window, dir, None).is_none())
        {
            // Not stacked with anything.
            return false;
        }

        self.grid.remove(&window);
        if let Some(partner) = partner {
            let other = self.grid.area(&partner).unwrap();
            let row = other.row.min(area.row);
            let bottom = other.bottom().max(area.bottom());
            let grown = Area {
                row,
                rows: bottom - row,
                ..other
            };
            self.grid.set_area(&partner, grown, grid::Direction::Right);
        }

        // Go past everything sharing columns with where the window was, or before all of it.
        let x_overlap = |b: &Area| b.col < area.right() && area.col < b.right();
        let stack = self.grid.iter().filter(|(_, b)| x_overlap(b));
        let col = match dir {
            grid::Direction::Left => stack.map(|(_, b)| b.col).min().unwrap_or(area.col),
            _ => stack.map(|(_, b)| b.right()).max().unwrap_or(area.right()),
        };
        let rows = self.grid.rows();
        self.grid.insert(window, Area::new(col, 0, area.cols, rows));
        true
    }

    /// Moves the windows of `window` into the tabbed `into` as tabs after its own, focusing the
    /// first of them if `window` was active.
    fn consume_grid_as_tab(&mut self, window: ColumnId, into: ColumnId) -> bool {
        let was_active = self.columns[self.active_column_idx].id == window;
        let column = self.take_grid_column(window);

        let idx = self.columns.iter().position(|col| col.id == into).unwrap();
        let col = &mut self.columns[idx];
        let first = col.tiles.len();
        for tile in column.tiles {
            col.add_tile_at(col.tiles.len(), tile);
        }
        self.data[idx].update(col);

        if was_active {
            col.activate_idx(first);
            self.active_column_idx = idx;
        }
        true
    }

    /// Takes the tab at `tile_idx` out of the tabbed `window`, putting it beside the area
    /// towards `dir` at the same size, and focusing it if it was focused.
    fn expel_grid_tab(&mut self, window: ColumnId, tile_idx: usize, dir: grid::Direction) -> bool {
        let area = self.grid.area(&window).unwrap();
        let idx = self
            .columns
            .iter()
            .position(|col| col.id == window)
            .unwrap();
        let col = &mut self.columns[idx];
        let was_active = idx == self.active_column_idx && tile_idx == col.active_tile_idx;
        let (width, is_full_width) = (col.width, col.is_full_width);
        let tile = col.take_tile(tile_idx);
        self.data[idx].update(col);

        let col = match dir {
            grid::Direction::Left => area.col,
            _ => area.right(),
        };
        let area = Area { col, ..area };
        let new = self.put_grid_column(tile, width, is_full_width, area);
        if was_active {
            self.active_column_idx = self.columns.iter().position(|col| col.id == new).unwrap();
        }
        true
    }

    /// Switches the active column between tabbed and normal display.
    ///
    /// Going tabbed gathers the windows stacked right above and below it in exactly the same
    /// columns of cells into its area as tabs. Going normal splits the tabs back into a stack,
    /// sharing the area's rows.
    fn set_grid_display(&mut self, display: ColumnDisplay) {
        let Some(col) = self.columns.get(self.active_column_idx) else {
            return;
        };
        if col.display_mode == display || !col.pending_sizing_mode().is_normal() {
            return;
        }
        let id = col.id;

        // Where every window is on the strip, to animate the ones that change columns.
        let tile_positions = |this: &Self| {
            let mut positions = Vec::new();
            for (col, col_x) in zip(&this.columns, this.column_xs(this.data.iter().copied())) {
                for (tile, offset) in col.tiles() {
                    let pos = Point::from((col_x, 0.)) + offset;
                    positions.push((tile.window().id().clone(), col.id, pos));
                }
            }
            positions
        };
        let before = tile_positions(self);

        self.change_grid(|this| match display {
            ColumnDisplay::Tabbed => this.tab_grid_stack(id),
            ColumnDisplay::Normal => this.untab_grid(id),
        });

        for (window, col_id, pos) in tile_positions(self) {
            let Some((_, prev_col_id, prev_pos)) = before.iter().find(|(w, _, _)| *w == window)
            else {
                continue;
            };
            if *prev_col_id == col_id {
                continue;
            }
            let col = self
                .columns
                .iter_mut()
                .find(|col| col.id == col_id)
                .unwrap();
            let tile = col
                .tiles
                .iter_mut()
                .find(|tile| *tile.window().id() == window);
            tile.unwrap().animate_move_from(*prev_pos - pos);
        }
    }

    /// Gathers the windows stacked right above and below `id` in exactly the same columns of
    /// cells into its column as tabs.
    fn tab_grid_stack(&mut self, id: ColumnId) -> bool {
        let area = self.grid.area(&id).unwrap();
        let in_same_columns = |other: &Area| other.col == area.col && other.cols == area.cols;

        // Nearest first.
        let mut above = Vec::new();
        let mut top = area;
        while let Some((&other, a)) = self
            .grid
            .iter()
            .find(|(_, a)| in_same_columns(a) && a.bottom() == top.row)
        {
            above.push(other);
            top = a;
        }
        let mut below = Vec::new();
        let mut bottom = area;
        while let Some((&other, a)) = self
            .grid
            .iter()
            .find(|(_, a)| in_same_columns(a) && a.row == bottom.bottom())
        {
            below.push(other);
            bottom = a;
        }

        let idx = self.columns.iter().position(|col| col.id == id).unwrap();
        self.columns[idx].set_column_display(ColumnDisplay::Tabbed);

        for other in above {
            let column = self.take_grid_column(other);
            let col = self.columns.iter_mut().find(|col| col.id == id).unwrap();
            for tile in column.tiles.into_iter().rev() {
                col.add_tile_at(0, tile);
                col.active_tile_idx += 1;
            }
        }
        for other in below {
            let column = self.take_grid_column(other);
            let col = self.columns.iter_mut().find(|col| col.id == id).unwrap();
            for tile in column.tiles {
                col.add_tile_at(col.tiles.len(), tile);
            }
        }

        let merged = Area {
            row: top.row,
            rows: bottom.bottom() - top.row,
            ..area
        };
        self.grid.set_area(&id, merged, grid::Direction::Down);
        self.active_column_idx = self.columns.iter().position(|col| col.id == id).unwrap();
        true
    }

    /// Splits the tabs of `id` back into a stack sharing its area's rows. Tabs that don't get a
    /// row go after the area.
    fn untab_grid(&mut self, id: ColumnId) -> bool {
        let area = self.grid.area(&id).unwrap();
        let idx = self.columns.iter().position(|col| col.id == id).unwrap();
        let col = &mut self.columns[idx];
        let count = col.tiles.len() as i32;
        let active_tile_idx = col.active_tile_idx;
        let (width, is_full_width) = (col.width, col.is_full_width);

        let mut tiles = Vec::new();
        while col.tiles.len() > 1 {
            tiles.push(col.take_tile(col.tiles.len() - 1));
        }
        tiles.reverse();
        col.set_column_display(ColumnDisplay::Normal);
        self.data[idx].update(col);

        // Split the rows as evenly as possible, giving the remainder to the top ones.
        let stacked = count.min(area.rows);
        let base = area.rows / stacked;
        let extra = area.rows % stacked;
        let rows_of = |i: i32| base + i32::from(i < extra);

        let mut row = area.row;
        let mut slices = Vec::new();
        for i in 0..count {
            let slice = if i < stacked {
                let slice = Area {
                    row,
                    rows: rows_of(i),
                    ..area
                };
                row += rows_of(i);
                slice
            } else {
                Area {
                    col: area.right(),
                    ..area
                }
            };
            slices.push(slice);
        }

        self.grid.set_area(&id, slices[0], grid::Direction::Down);
        let mut active = id;
        for (i, (tile, slice)) in zip(tiles, &slices[1..]).enumerate() {
            let new = self.put_grid_column(tile, width, is_full_width, *slice);
            if i + 1 == active_tile_idx {
                active = new;
            }
        }
        self.active_column_idx = self
            .columns
            .iter()
            .position(|col| col.id == active)
            .unwrap();
        true
    }

    /// Takes a column off the grid without the focus and view handling of removing a window,
    /// for its windows to go to another column. Keeps the active column the same if it can.
    fn take_grid_column(&mut self, id: ColumnId) -> Column<W> {
        let active = self.columns[self.active_column_idx].id;
        let idx = self.columns.iter().position(|col| col.id == id).unwrap();
        let mut column = self.columns.remove(idx);
        self.data.remove(idx);
        self.grid.remove(&id);
        cancel_resize_for_column(&mut self.interactive_resize, &mut column);

        self.active_column_idx = self
            .columns
            .iter()
            .position(|col| col.id == active)
            .unwrap_or(0);
        self.activate_prev_column_on_removal = None;
        column
    }

    /// Adds a new column with `tile` at `area` on the grid, pushing others aside.
    fn put_grid_column(
        &mut self,
        tile: Tile<W>,
        width: ColumnWidth,
        is_full_width: bool,
        area: Area,
    ) -> ColumnId {
        let mut column = Column::new_with_tile(
            tile,
            self.view_size,
            self.working_area,
            self.parent_area,
            self.scale,
            width,
            is_full_width,
        );
        // It was a tab, and is a window of its own now.
        column.display_mode = ColumnDisplay::Normal;

        let id = column.id;
        self.data.push(ColumnData::new(&column));
        self.columns.push(column);
        self.grid.insert(id, area);
        id
    }

    /// Applies a grid change from `change`, if it returns true, scrolling to the active window if
    /// it went out of view.
    fn change_grid(&mut self, change: impl FnOnce(&mut Self) -> bool) {
        let prev_positions = self.grid_positions();
        if !change(self) {
            return;
        }

        let col = &mut self.columns[self.active_column_idx];
        cancel_resize_for_column(&mut self.interactive_resize, col);

        self.sync_grid(Some(prev_positions), None);
        if !self.view_offset.is_gesture() {
            self.animate_view_offset_to_column(None, self.active_column_idx, None);
        }
    }

    /// Moves the active window to where `target` is on the grid.
    fn move_grid_to_window(&mut self, target: ColumnId) {
        let target = self.grid.area(&target).unwrap();
        self.move_grid_to_col(|area| {
            if target.col <= area.col {
                // Take the target's place and push it right.
                (target.col, grid::Direction::Right)
            } else {
                // Line up with the target's right edge and push it left.
                (target.right() - area.cols, grid::Direction::Left)
            }
        });
    }

    /// Moves the active window to the column of cells from `place`, which also gives the
    /// direction to push others in.
    fn move_grid_to_col(&mut self, place: impl FnOnce(Area) -> (i32, grid::Direction)) {
        let Some(id) = self.columns.get(self.active_column_idx).map(|col| col.id) else {
            return;
        };
        let area = self.grid.area(&id).unwrap();
        let (col, dir) = place(area);
        if col == area.col {
            return;
        }

        let prev_positions = self.grid_positions();
        let moved = self.grid.set_area(&id, Area { col, ..area }, dir);

        // Pushing left can run into the left edge of the grid. Push right instead, from the same
        // place.
        if !moved && dir == grid::Direction::Left {
            let col = col.max(0);
            self.grid
                .set_area(&id, Area { col, ..area }, grid::Direction::Right);
        }
        self.finish_grid_move(prev_positions);
    }

    /// The active window with the row and column of cells that focus is travelling along.
    fn grid_focus_lines(&self) -> Option<(ColumnId, i32, i32)> {
        let id = self.columns.get(self.active_column_idx)?.id;
        let area = self.grid.area(&id)?;
        let anchor = self.focus_anchor.filter(|anchor| anchor.column == id);
        let row = anchor.and_then(|anchor| anchor.row).unwrap_or(area.row);
        let col = anchor.and_then(|anchor| anchor.col).unwrap_or(area.col);
        Some((id, row, col))
    }

    /// Focuses `target`, travelling along `row` and `col` where it covers them.
    fn focus_grid_window(&mut self, target: ColumnId, row: i32, col: i32) {
        let area = self.grid.area(&target).unwrap();
        let idx = self
            .columns
            .iter()
            .position(|col| col.id == target)
            .unwrap();
        self.activate_column(idx);
        self.focus_anchor = Some(FocusAnchor {
            column: target,
            row: Some(row.clamp(area.row, area.bottom() - 1)),
            col: Some(col.clamp(area.col, area.right() - 1)),
        });
    }

    /// Focuses the leftmost or rightmost window in the row that focus is travelling along.
    fn focus_grid_end(&mut self, last: bool) {
        let Some((_, row, _)) = self.grid_focus_lines() else {
            return;
        };
        let in_row = self
            .grid
            .iter()
            .filter(|(_, area)| area.row <= row && row < area.bottom());
        let target = if last {
            in_row.max_by_key(|(_, area)| (area.right(), -area.row))
        } else {
            in_row.min_by_key(|(_, area)| (area.col, area.row))
        };
        let Some((&target, area)) = target else {
            return;
        };

        let col = if last { area.right() - 1 } else { area.col };
        self.focus_grid_window(target, row, col);
    }

    /// Focuses the next tab down, or up, in the active column if it's tabbed.
    fn focus_grid_tab(&mut self, down: bool) -> bool {
        let Some(col) = self.columns.get_mut(self.active_column_idx) else {
            return false;
        };
        if col.display_mode != ColumnDisplay::Tabbed {
            return false;
        }
        if down {
            col.focus_down()
        } else {
            col.focus_up()
        }
    }

    /// Windows in the stack along column of cells `col`, top to bottom, with tabs in order, as
    /// (column, tile index).
    fn grid_stack_tiles(&self, col: i32) -> Vec<(ColumnId, usize)> {
        let mut tiles = Vec::new();
        for id in self.grid.stack(col) {
            let column = self.columns.iter().find(|column| column.id == *id).unwrap();
            tiles.extend((0..column.tiles.len()).map(|tile_idx| (*id, tile_idx)));
        }
        tiles
    }

    /// Focuses a window in the stack along the column of cells that focus is travelling along,
    /// counting tabs, at the index that `pick` returns for the stack's length.
    fn focus_grid_in_stack(&mut self, pick: impl FnOnce(usize) -> usize) {
        let Some((_, row, col)) = self.grid_focus_lines() else {
            return;
        };
        let stack = self.grid_stack_tiles(col);
        if stack.is_empty() {
            return;
        }

        let (target, tile_idx) = stack[pick(stack.len())];
        let area = self.grid.area(&target).unwrap();
        self.focus_grid_window(target, row.clamp(area.row, area.bottom() - 1), col);
        self.columns[self.active_column_idx].activate_idx(tile_idx);
    }

    /// Window in grid column `index`, counting from 1 like the actions do: the one in the row
    /// that focus is travelling along, or the top one. Indices past the end give the last
    /// column.
    fn grid_column_window(&self, index: usize) -> Option<ColumnId> {
        let (_, row, _) = self.grid_focus_lines()?;
        let starts = self.grid.column_starts();
        let start = *starts.get(index.saturating_sub(1).min(starts.len() - 1))?;

        let stack = self.grid.stack(start);
        let in_row = stack.iter().find(|id| {
            let area = self.grid.area(id).unwrap();
            area.row <= row && row < area.bottom()
        });
        in_row.or(stack.first()).map(|id| **id)
    }

    /// Syncs columns after moving the active window, then scrolls to it if it went out of view.
    fn finish_grid_move(&mut self, prev_positions: Vec<(ColumnId, Point<f64, Logical>)>) {
        self.sync_grid(Some(prev_positions), None);
        self.animate_view_offset_to_column_with_config(
            None,
            self.active_column_idx,
            None,
            self.options.animations.window_movement.0,
        );
    }

    /// Lets the window held by the pointer fall into place.
    fn release_grid_pin(&mut self) {
        if self.grid.pinned().is_none() {
            return;
        }

        let before = self.grid.clone();
        let prev_positions = self.grid_positions();
        self.grid.set_pinned(None);

        let moved = self
            .grid
            .iter()
            .any(|(other, area)| before.area(other) != Some(area));
        if !moved {
            return;
        }

        self.sync_grid(Some(prev_positions), None);
    }

    /// Resizes a window by dragging its edges, snapping to cells and pushing others aside.
    ///
    /// `delta` is the pointer movement since the start of the resize.
    fn interactive_resize_grid(&mut self, window: &W::Id, delta: Point<f64, Logical>) -> bool {
        let Some(resize) = &self.interactive_resize else {
            return false;
        };
        let edges = resize.data.edges;

        let idx = self
            .columns
            .iter()
            .position(|col| col.contains(window))
            .unwrap();
        let id = self.columns[idx].id;

        // Windows opened or closed during the resize make the snapshot stale, so stop there.
        let Some(original) = self
            .grid_resize_original
            .as_ref()
            .filter(|original| {
                original.len() == self.columns.len()
                    && self
                        .columns
                        .iter()
                        .all(|col| original.area(&col.id).is_some())
            })
            .cloned()
        else {
            self.interactive_resize = None;
            self.grid_resize_original = None;
            return false;
        };
        let orig = original.area(&id).unwrap();

        let m = self.cell_metrics;
        let dx = (delta.x / (m.cell_w + m.gap)).round() as i32;
        let dy = (delta.y / (m.cell_h + m.gap)).round() as i32;

        let mut grid = original;
        if edges.intersects(ResizeEdge::TOP_BOTTOM) {
            let (area, dir) = if edges.contains(ResizeEdge::TOP) {
                let row = (orig.row + dy).clamp(0, orig.bottom() - 1);
                let area = Area {
                    row,
                    rows: orig.bottom() - row,
                    ..orig
                };
                (area, grid::Direction::Up)
            } else {
                let rows = (orig.rows + dy).clamp(1, m.rows - orig.row);
                (Area { rows, ..orig }, grid::Direction::Down)
            };
            grid.set_area(&id, area, dir);
        }
        if edges.intersects(ResizeEdge::LEFT_RIGHT) {
            let cur = grid.area(&id).unwrap();
            if edges.contains(ResizeEdge::LEFT) {
                // Grow left only as far as the windows to the left can go before the left edge
                // of the grid.
                let mut dx = dx.min(orig.cols - 1);
                loop {
                    let area = Area {
                        col: cur.col + dx,
                        cols: orig.cols - dx,
                        ..cur
                    };
                    let mut attempt = grid.clone();
                    if attempt.set_area(&id, area, grid::Direction::Left) || dx >= 0 {
                        grid = attempt;
                        break;
                    }
                    dx += 1;
                }
            } else {
                let cols = (orig.cols + dx).max(1);
                grid.set_area(&id, Area { cols, ..cur }, grid::Direction::Right);
            }
        }

        if grid == self.grid {
            return true;
        }

        let prev_positions = self.grid_positions();
        self.grid = grid;
        self.sync_grid(Some(prev_positions), None);
        true
    }

    /// Where a window being dragged would drop on the grid.
    ///
    /// `pos` is the pointer position in the workspace view, and `pointer_ratio` is where the
    /// pointer is within the window, so the window lands where it's being held.
    fn grid_insert_position(
        &self,
        pos: Point<f64, Logical>,
        tile_size: Size<f64, Logical>,
        pointer_ratio: (f64, f64),
    ) -> InsertPosition {
        let m = &self.cell_metrics;

        let cols =
            (((tile_size.w + m.gap) / (m.cell_w + m.gap)).round() as i32).clamp(1, grid::MAX_COLS);
        let rows = (((tile_size.h + m.gap) / (m.cell_h + m.gap)).round() as i32).clamp(1, m.rows);

        let top_left = Point::from((
            pos.x - pointer_ratio.0 * tile_size.w,
            pos.y - pointer_ratio.1 * tile_size.h,
        ));
        let area = self.grid_area_at(top_left, cols, rows);

        // Show where the window will actually land, after pushing and gravity.
        let mut grid = self.grid.clone();
        let id = ColumnId::specific(u64::MAX);
        grid.insert(id, area);
        InsertPosition::Cell(grid.area(&id).unwrap())
    }

    /// Makes `tile`, when it's added next, go to the cells nearest to `pos`, its top-left corner
    /// relative to the view, at the size it had on the grid.
    pub fn set_grid_drop_pos(
        &mut self,
        tile: &Tile<W>,
        pos: Point<f64, Logical>,
        width: ColumnWidth,
        is_full_width: bool,
    ) {
        let (cols, rows) = self.grid_span_for_size(tile, width, is_full_width);
        let area = self.grid_area_at(pos, cols, rows);
        self.set_grid_drop_area(tile.window().id().clone(), area);
    }

    /// Makes `window` go to `area` on the grid when it's added next.
    pub fn set_grid_drop_area(&mut self, window: W::Id, area: Area) {
        self.grid_drop_area = Some((window, area));
    }

    /// Index of the column with `window`, or of the active column.
    fn grid_column_idx(&self, window: Option<&W::Id>) -> Option<usize> {
        match window {
            Some(window) => self.columns.iter().position(|col| col.contains(window)),
            None => (!self.columns.is_empty()).then_some(self.active_column_idx),
        }
    }

    /// Resizes a window to a new grid span, pushing others aside.
    fn resize_grid(&mut self, window: Option<&W::Id>, cols: Option<i32>, rows: Option<i32>) {
        let Some(idx) = self.grid_column_idx(window) else {
            return;
        };
        let id = self.columns[idx].id;
        let area = self.grid.area(&id).unwrap();

        let prev_positions = self.grid_positions();
        let cols = cols.unwrap_or(area.cols);
        let rows = rows.unwrap_or(area.rows);
        if !self.grid.resize(&id, cols, rows) {
            return;
        }

        self.sync_grid(Some(prev_positions), None);

        // Make sure the resized window stays in view.
        if self.columns[self.active_column_idx].id == id && !self.view_offset.is_gesture() {
            self.animate_view_offset_to_column(None, self.active_column_idx, None);
        }
    }

    fn set_grid_width(&mut self, window: Option<&W::Id>, change: SizeChange) {
        let Some(idx) = self.grid_column_idx(window) else {
            return;
        };
        let cols = self.grid.area(&self.columns[idx].id).unwrap().cols;
        let m = &self.cell_metrics;

        let cols = match change {
            SizeChange::SetFixed(width) => {
                // Like in niri, a fixed width is for the window, not counting its border.
                let tile = &self.columns[idx].tiles[0];
                let width = tile.tile_width_for_window_width(f64::from(width));
                self.grid_cols_for_width(ColumnWidth::Fixed(width))
            }
            SizeChange::SetProportion(percent) => m.cols_for_proportion(percent / 100.),
            SizeChange::AdjustFixed(delta) => {
                cols + cells_for_delta(f64::from(delta), m.cell_w + m.gap)
            }
            SizeChange::AdjustProportion(delta) => {
                cols + cells_for_delta(delta / 100. * f64::from(m.cols_per_view), 1.)
            }
        };
        self.resize_grid(window, Some(cols.max(1)), None);
    }

    fn set_grid_height(&mut self, window: Option<&W::Id>, change: SizeChange) {
        let Some(idx) = self.grid_column_idx(window) else {
            return;
        };
        let rows = self.grid.area(&self.columns[idx].id).unwrap().rows;
        let m = &self.cell_metrics;

        let rows = match change {
            SizeChange::SetFixed(height) => {
                // Like in niri, a fixed height is for the window, not counting its border.
                let tile = &self.columns[idx].tiles[0];
                let height = tile.tile_height_for_window_height(f64::from(height));
                let rows = (height + m.gap) / (m.cell_h + m.gap);
                (rows.round() as i32).clamp(1, m.rows)
            }
            SizeChange::SetProportion(percent) => m.rows_for_proportion(percent / 100.),
            SizeChange::AdjustFixed(delta) => {
                rows + cells_for_delta(f64::from(delta), m.cell_h + m.gap)
            }
            SizeChange::AdjustProportion(delta) => {
                rows + cells_for_delta(delta / 100. * f64::from(m.rows), 1.)
            }
        };
        self.resize_grid(window, None, Some(rows.clamp(1, m.rows)));
    }

    fn toggle_grid_width(&mut self, window: Option<&W::Id>, forwards: bool) {
        let Some(idx) = self.grid_column_idx(window) else {
            return;
        };
        let cols = self.grid.area(&self.columns[idx].id).unwrap().cols;
        let borders = self.columns[idx].tiles[0].tile_width_for_window_width(0.);
        let presets = self.options.layout.preset_column_widths.iter();
        let spans = presets.map(|preset| self.grid_cols_for_preset(*preset, borders));
        let cols = next_preset_span(cols, spans, forwards);
        self.resize_grid(window, Some(cols), None);
    }

    fn toggle_grid_height(&mut self, window: Option<&W::Id>, forwards: bool) {
        let Some(idx) = self.grid_column_idx(window) else {
            return;
        };
        let rows = self.grid.area(&self.columns[idx].id).unwrap().rows;
        let borders = self.columns[idx].tiles[0].tile_height_for_window_height(0.);
        let presets = self.options.layout.preset_window_heights.iter();
        let spans = presets.map(|preset| self.grid_rows_for_preset(*preset, borders));
        let rows = next_preset_span(rows, spans, forwards);
        self.resize_grid(window, None, Some(rows));
    }

    /// Toggles a window between the full view width and the width it had before.
    fn toggle_grid_full_width(&mut self) {
        let Some(idx) = self.grid_column_idx(None) else {
            return;
        };

        // The window keeps its own width while full-width, so it comes back to it.
        let col = &mut self.columns[idx];
        col.is_full_width = !col.is_full_width;
        let (cols, _) = self.grid_span_for(&self.columns[idx]);
        self.resize_grid(None, Some(cols), None);
    }

    #[cfg(test)]
    fn verify_grid_invariants(&self) {
        assert_eq!(self.grid.len(), self.columns.len());
        self.grid.verify_invariants();

        assert_eq!(
            self.cell_metrics,
            compute_cell_metrics(self.working_area, &self.options),
            "cell metrics must match the working area"
        );
        assert_eq!(self.grid.rows(), self.cell_metrics.rows);
        assert_eq!(self.grid.left_gravity(), self.options.layout.left_gravity);

        let mut prev_key = None;
        for col in &self.columns {
            assert!(
                col.tiles.len() == 1 || col.display_mode == ColumnDisplay::Tabbed,
                "every window must be its own column on the grid, except for tabs"
            );
            assert!(
                col.tiles
                    .iter()
                    .all(|tile| tile.grid_size == col.tiles[0].grid_size),
                "tabs must share their size on the grid"
            );

            let area = self
                .grid
                .area(&col.id)
                .expect("every column must be on the grid");
            let key = (area.col, area.row);
            assert!(
                prev_key.is_none_or(|prev| prev < key),
                "columns must be sorted by grid position"
            );
            prev_key = Some(key);

            assert_eq!(col.grid_span, Some((area.cols, area.rows)));
            assert_eq!(col.grid_geometry, Some(self.grid_geometry(area)));
            assert_eq!(
                self.grid_span_in_mode(col),
                (area.cols, area.rows),
                "the window's size must match its area on the grid"
            );
        }
    }
}

/// Number of cells to grow or shrink by for `amount`, where a cell takes up `cell`.
///
/// Rounds to the nearest cell, but a nonzero amount changes the size by at least one cell.
fn cells_for_delta(amount: f64, cell: f64) -> i32 {
    let cells = (amount / cell).round() as i32;
    if cells == 0 && amount != 0. {
        amount.signum() as i32
    } else {
        cells
    }
}

/// Scales a span of `n` cells out of `of` to a view `total` cells across.
fn scale_span(n: i32, of: i32, total: i32) -> i32 {
    if of == total || of <= 0 {
        return n;
    }
    (f64::from(n) * f64::from(total) / f64::from(of)).round() as i32
}

/// Next span when cycling through presets: the smallest one larger than `current` going
/// forwards, or the largest one smaller going backwards, wrapping around.
fn next_preset_span(current: i32, spans: impl Iterator<Item = i32>, forwards: bool) -> i32 {
    let mut spans: Vec<i32> = spans.collect();
    spans.sort_unstable();
    spans.dedup();

    let next = if forwards {
        spans
            .iter()
            .copied()
            .find(|&span| span > current)
            .or(spans.first().copied())
    } else {
        spans
            .iter()
            .rev()
            .copied()
            .find(|&span| span < current)
            .or(spans.last().copied())
    };
    next.unwrap_or(current)
}

impl<W: LayoutElement> Column<W> {
    fn set_grid_geometry(&mut self, geometry: GridGeometry, span: (i32, i32), animate: bool) {
        self.grid_span = Some(span);
        if self.grid_geometry != Some(geometry) {
            self.grid_geometry = Some(geometry);
            self.update_tile_sizes(animate);
        }
    }
}

impl ViewOffset {
    /// Returns the current view offset.
    pub fn current(&self) -> f64 {
        match self {
            ViewOffset::Static(offset) => *offset,
            ViewOffset::Animation(anim) => anim.value(),
            ViewOffset::Gesture(gesture) => {
                gesture.current_view_offset
                    + gesture.animation.as_ref().map_or(0., |anim| anim.value())
            }
        }
    }

    /// Returns the target view offset suitable for computing the new view offset.
    pub fn target(&self) -> f64 {
        match self {
            ViewOffset::Static(offset) => *offset,
            ViewOffset::Animation(anim) => anim.to(),
            // This can be used for example if a gesture is interrupted.
            ViewOffset::Gesture(gesture) => gesture.current_view_offset,
        }
    }

    /// Returns a view offset value suitable for saving and later restoration.
    ///
    /// This means that it shouldn't return an in-progress animation or gesture value.
    fn stationary(&self) -> f64 {
        match self {
            ViewOffset::Static(offset) => *offset,
            // For animations we can return the final value.
            ViewOffset::Animation(anim) => anim.to(),
            ViewOffset::Gesture(gesture) => gesture.stationary_view_offset,
        }
    }

    pub fn is_static(&self) -> bool {
        matches!(self, Self::Static(_))
    }

    pub fn is_gesture(&self) -> bool {
        matches!(self, Self::Gesture(_))
    }

    pub fn is_dnd_scroll(&self) -> bool {
        matches!(&self, ViewOffset::Gesture(gesture) if gesture.dnd_last_event_time.is_some())
    }

    pub fn is_animation_ongoing(&self) -> bool {
        match self {
            ViewOffset::Static(_) => false,
            ViewOffset::Animation(_) => true,
            ViewOffset::Gesture(gesture) => gesture.animation.is_some(),
        }
    }

    pub fn offset(&mut self, delta: f64) {
        match self {
            ViewOffset::Static(offset) => *offset += delta,
            ViewOffset::Animation(anim) => anim.offset(delta),
            ViewOffset::Gesture(gesture) => {
                gesture.stationary_view_offset += delta;
                gesture.delta_from_tracker += delta;
                gesture.current_view_offset += delta;
            }
        }
    }

    pub fn cancel_gesture(&mut self) {
        if let ViewOffset::Gesture(gesture) = self {
            *self = ViewOffset::Static(gesture.current_view_offset);
        }
    }

    pub fn stop_anim_and_gesture(&mut self) {
        *self = ViewOffset::Static(self.current());
    }
}

impl ViewGesture {
    fn animate_from(&mut self, from: f64, clock: Clock, config: niri_config::Animation) {
        let current = self.animation.as_ref().map_or(0., Animation::value);
        self.animation = Some(Animation::new(clock, from + current, 0., 0., config));
    }
}

impl ColumnData {
    pub fn new<W: LayoutElement>(column: &Column<W>) -> Self {
        let mut rv = Self { width: 0. };
        rv.update(column);
        rv
    }

    pub fn update<W: LayoutElement>(&mut self, column: &Column<W>) {
        self.width = column.width();
    }
}

impl TileData {
    pub fn new<W: LayoutElement>(tile: &Tile<W>, height: WindowHeight) -> Self {
        let mut rv = Self {
            height,
            size: Size::default(),
            interactively_resizing_by_left_edge: false,
        };
        rv.update(tile);
        rv
    }

    pub fn update<W: LayoutElement>(&mut self, tile: &Tile<W>) {
        self.size = tile.tile_size();
        self.interactively_resizing_by_left_edge = tile
            .window()
            .interactive_resize_data()
            .is_some_and(|data| data.edges.contains(ResizeEdge::LEFT));
    }
}

impl From<PresetSize> for ColumnWidth {
    fn from(value: PresetSize) -> Self {
        match value {
            PresetSize::Proportion(p) => Self::Proportion(p.clamp(0., 10000.)),
            PresetSize::Fixed(f) => Self::Fixed(f64::from(f.clamp(1, 100000))),
        }
    }
}

impl WindowHeight {
    const fn auto_1() -> Self {
        Self::Auto { weight: 1. }
    }
}

impl<W: LayoutElement> Column<W> {
    #[allow(clippy::too_many_arguments)]
    fn new_with_tile(
        tile: Tile<W>,
        view_size: Size<f64, Logical>,
        working_area: Rectangle<f64, Logical>,
        parent_area: Rectangle<f64, Logical>,
        scale: f64,
        width: ColumnWidth,
        is_full_width: bool,
    ) -> Self {
        let options = tile.options.clone();

        let display_mode = tile
            .window()
            .rules()
            .default_column_display
            .unwrap_or(options.layout.default_column_display);

        // Try to match width to a preset width. Consider the following case: a terminal (foot)
        // sizes itself to the terminal grid. We open it with default-column-width 0.5. It shrinks
        // by a few pixels to evenly match the terminal grid. Then we press
        // switch-preset-column-width intending to go to proportion 0.667, but the preset width
        // matching code picks the proportion 0.5 preset because it's the next smallest width after
        // the current foot's window width. Effectively, this makes the first
        // switch-preset-column-width press ignored.
        //
        // However, here, we do know that width = proportion 0.5 (regardless of what the window
        // opened with), and we can match it to a preset right away, if one exists.
        let preset_width_idx = options
            .layout
            .preset_column_widths
            .iter()
            .position(|preset| width == ColumnWidth::from(*preset));

        let mut rv = Self {
            tiles: vec![],
            data: vec![],
            active_tile_idx: 0,
            width,
            preset_width_idx,
            is_full_width,
            is_pending_maximized: false,
            is_pending_fullscreen: false,
            display_mode,
            tab_indicator: TabIndicator::new(options.layout.tab_indicator),
            move_x_animation: None,
            move_y_animation: None,
            view_size,
            working_area,
            parent_area,
            scale,
            clock: tile.clock.clone(),
            options,
            id: ColumnId::next(),
            grid_geometry: None,
            grid_span: None,
            grid_row_to_restore: None,
        };

        let pending_sizing_mode = tile.window().pending_sizing_mode();

        rv.add_tile_at(0, tile);

        match pending_sizing_mode {
            SizingMode::Normal => (),
            SizingMode::Maximized => rv.set_maximized(true),
            SizingMode::Fullscreen => rv.set_fullscreen(true),
        }

        // Animate the tab indicator for new columns.
        if display_mode == ColumnDisplay::Tabbed
            && !rv.options.layout.tab_indicator.hide_when_single_tab
            && rv.sizing_mode().is_normal()
        {
            // Usually new columns are created together with window movement actions. For new
            // windows, we handle that in start_open_animation().
            rv.tab_indicator
                .start_open_animation(rv.clock.clone(), rv.options.animations.window_movement.0);
        }

        rv
    }

    pub fn id(&self) -> ColumnId {
        self.id
    }

    fn update_config(
        &mut self,
        view_size: Size<f64, Logical>,
        working_area: Rectangle<f64, Logical>,
        parent_area: Rectangle<f64, Logical>,
        scale: f64,
        options: Rc<Options>,
    ) {
        let mut update_sizes = false;

        if self.view_size != view_size
            || self.working_area != working_area
            || self.parent_area != parent_area
        {
            update_sizes = true;
        }

        // If preset widths changed, clear our stored preset index.
        if self.options.layout.preset_column_widths != options.layout.preset_column_widths {
            self.preset_width_idx = None;
        }

        // If preset heights changed, make our heights non-preset.
        if self.options.layout.preset_window_heights != options.layout.preset_window_heights {
            self.convert_heights_to_auto();
            update_sizes = true;
        }

        if self.options.layout.gaps != options.layout.gaps {
            update_sizes = true;
        }

        if self.options.layout.border.off != options.layout.border.off
            || self.options.layout.border.width != options.layout.border.width
        {
            update_sizes = true;
        }

        if self.options.layout.tab_indicator != options.layout.tab_indicator {
            update_sizes = true;
        }

        for (tile, data) in zip(&mut self.tiles, &mut self.data) {
            tile.update_config(view_size, scale, options.clone());
            data.update(tile);
        }

        self.tab_indicator
            .update_config(options.layout.tab_indicator);
        self.view_size = view_size;
        self.working_area = working_area;
        self.parent_area = parent_area;
        self.scale = scale;
        self.options = options;

        if update_sizes {
            self.update_tile_sizes(false);
        }
    }

    pub fn update_shaders(&mut self) {
        for tile in &mut self.tiles {
            tile.update_shaders();
        }

        self.tab_indicator.update_shaders();
    }

    pub fn advance_animations(&mut self) {
        if let Some(move_) = &mut self.move_x_animation {
            if move_.anim.is_done() {
                self.move_x_animation = None;
            }
        }
        if let Some(move_) = &mut self.move_y_animation {
            if move_.anim.is_done() {
                self.move_y_animation = None;
            }
        }

        for tile in &mut self.tiles {
            tile.advance_animations();
        }

        self.tab_indicator.advance_animations();
    }

    pub fn are_animations_ongoing(&self) -> bool {
        self.move_x_animation.is_some()
            || self.move_y_animation.is_some()
            || self.tab_indicator.are_animations_ongoing()
            || self.tiles.iter().any(Tile::are_animations_ongoing)
    }

    pub fn are_transitions_ongoing(&self) -> bool {
        self.move_x_animation.is_some()
            || self.move_y_animation.is_some()
            || self.tab_indicator.are_animations_ongoing()
            || self.tiles.iter().any(Tile::are_transitions_ongoing)
    }

    pub fn is_moving_between_workspaces(&self) -> bool {
        self.move_y_animation
            .as_ref()
            .is_some_and(|anim| anim.is_between_workspaces)
            || self
                .move_x_animation
                .as_ref()
                .is_some_and(|anim| anim.is_between_workspaces)
            // When moving a window between workspaces into the scrolling layout, it will be
            // immediately put into a new column, however the Y move animation and the
            // moving-between-workspaces flag will be set on the tile rather than on that new
            // column. Since the scrolling layout rendering checks moving between workspaces
            // per-column rather than per-tile, we need to include the tiles here.
            //
            // This is not always visually correct: for example, interactive-moving a tile into an
            // otherwise stationary column will now cause that entire column to draw unculled. I'm
            // not sure there's a good solution overall for all edge cases here. Consider that the
            // column tab indicator needs to draw on top of the tiles, but in this counterexample
            // above, the column is stationary, so the single moving tile would instead need to draw
            // on top of the tab indicator.
            //
            // I'm going with this simple check for now that should look ok in most cases.
            || self
                .tiles
                .iter()
                .any(|tile| tile.is_moving_between_workspaces())
    }

    pub fn update_render_elements(&mut self, is_active: bool, view_rect: Rectangle<f64, Logical>) {
        let active_idx = self.active_tile_idx;
        for (tile_idx, (tile, tile_off)) in self.tiles_mut().enumerate() {
            let is_active = is_active && tile_idx == active_idx;

            let mut tile_view_rect = view_rect;
            tile_view_rect.loc -= tile_off + tile.render_offset();
            tile.update_render_elements(is_active, tile_view_rect);
        }

        let config = self.tab_indicator.config();
        let offsets = self.tile_offsets_iter(self.data.iter().copied());
        let tabs = zip(&self.tiles, offsets)
            .enumerate()
            .map(|(tile_idx, (tile, tile_off))| {
                let is_active = tile_idx == active_idx;
                let is_urgent = tile.window().is_urgent();
                let tile_pos = tile_off + tile.render_offset();
                TabInfo::from_tile(tile, tile_pos, is_active, is_urgent, &config)
            });

        // Hide the tab indicator in fullscreen. If you have it configured to overlap the window,
        // you don't want that to happen in fullscreen. Also, laying things out correctly when the
        // tab indicator is within the column and the column goes fullscreen, would require too
        // many changes to the code for too little benefit (it's mostly invisible anyway).
        let enabled = self.display_mode == ColumnDisplay::Tabbed && self.sizing_mode().is_normal();

        self.tab_indicator.update_render_elements(
            enabled,
            self.tab_indicator_area(),
            view_rect,
            self.tiles.len(),
            tabs,
            is_active,
            self.scale,
        );
    }

    pub fn is_pending_fullscreen(&self) -> bool {
        self.is_pending_fullscreen
    }

    pub fn is_pending_maximized(&self) -> bool {
        self.is_pending_maximized
    }

    pub fn pending_sizing_mode(&self) -> SizingMode {
        if self.is_pending_fullscreen {
            SizingMode::Fullscreen
        } else if self.is_pending_maximized {
            SizingMode::Maximized
        } else {
            SizingMode::Normal
        }
    }

    pub fn render_offset(&self) -> Point<f64, Logical> {
        let mut offset = Point::from((0., 0.));

        if let Some(move_) = &self.move_x_animation {
            offset.x += move_.from * move_.anim.value();
        }
        if let Some(move_) = &self.move_y_animation {
            offset.y += move_.from * move_.anim.value();
        }

        offset
    }

    pub fn animate_move_from(&mut self, from: Point<f64, Logical>) {
        self.animate_move_from_with_config(from, self.options.animations.window_movement.0);
    }

    pub fn animate_move_from_with_config(
        &mut self,
        from: Point<f64, Logical>,
        config: niri_config::Animation,
    ) {
        self.animate_move_x_from_with_config(from.x, config);
        self.animate_move_y_from_with_config(from.y, config);
    }

    pub fn animate_move_x_from(&mut self, from_x_offset: f64) {
        self.animate_move_x_from_with_config(
            from_x_offset,
            self.options.animations.window_movement.0,
        );
    }

    pub fn animate_move_x_from_with_config(
        &mut self,
        from_x_offset: f64,
        config: niri_config::Animation,
    ) {
        let (current_offset, current_between) =
            self.move_x_animation.as_ref().map_or((0., false), |move_| {
                (move_.from * move_.anim.value(), move_.is_between_workspaces)
            });

        let anim = Animation::new(self.clock.clone(), 1., 0., 0., config);
        self.move_x_animation = Some(MoveAnimation {
            anim,
            from: from_x_offset + current_offset,
            is_between_workspaces: current_between,
        });
    }

    pub fn animate_move_y_from(&mut self, from_y_offset: f64) {
        self.animate_move_y_from_with_config(
            from_y_offset,
            self.options.animations.window_movement.0,
        );
    }

    pub fn animate_move_y_from_with_config(
        &mut self,
        from_y_offset: f64,
        config: niri_config::Animation,
    ) {
        let (current_offset, current_between) =
            self.move_y_animation.as_ref().map_or((0., false), |move_| {
                (move_.from * move_.anim.value(), move_.is_between_workspaces)
            });

        let anim = Animation::new(self.clock.clone(), 1., 0., 0., config);
        self.move_y_animation = Some(MoveAnimation {
            anim,
            from: from_y_offset + current_offset,
            is_between_workspaces: current_between,
        });
    }

    pub fn offset_move_anim_current(&mut self, offset: f64) {
        if let Some(move_) = self.move_x_animation.as_mut() {
            // If the anim is almost done, there's little point trying to offset it; we can let
            // things jump. If it turns out like a bad idea, we could restart the anim instead.
            let value = move_.anim.value();
            if value > 0.001 {
                move_.from += offset / value;
            }
        }
    }

    pub fn set_anim_y_between_workspaces(&mut self) {
        if let Some(anim) = &mut self.move_y_animation {
            anim.is_between_workspaces = true;
        }
    }

    /// Returns whether this column is currently fullscreen.
    ///
    /// As in, if it contains one currently-fullscreen tile, or in tabbed mode, if it contains at
    /// least one currently-fullscreen tile.
    ///
    /// This will lag behind is_pending_fullscreen, depending on when the tiles actually respond to
    /// the un/fullscreen request. But, it's possible for is_fullscreen() to flip instantly, for
    /// example when consuming a fullscreen tile into a non-pending-fullscreen column.
    ///
    /// This controls things like:
    ///
    /// - whether the column draws at the top of the screen or at the start of the working area
    /// - whether the column draws above the top layer-shell layer
    /// - whether the tab indicator is shown
    /// - restoring view_offset_before_fullscreen
    ///
    /// Edge cases to watch out for:
    ///
    /// - Consuming a fullscreen tile into a non-tabbed column will keep that tile fullscreen until
    ///   it responds to the unfullscreen request. This tile may be anywhere in the column,
    ///   including at the active position.
    ///
    /// - Changing a fullscreen tabbed column into normal mode is an easy way to get randomly
    ///   delayed unfullscreening tiles in a normal column.
    ///
    /// - is_fullscreen() can suddenly change when consuming/expelling a fullscreen tile into/from a
    ///   non-fullscreen column. This can influence the code that saves/restores the unfullscreen
    ///   view offset.
    fn sizing_mode(&self) -> SizingMode {
        // Behaviors that we want:
        //
        // 1. The common case: single tile in a column. Assume no animations. Fullscreening the tile
        //    should make it jump to the top-left of the screen only when the tile finishes
        //    fullscreening. Similarly, unfullscreening should keep it at the top-left until the
        //    tile had unfullscreened.
        //
        // 2. Unfullscreening a tabbed column with multiple tiles should restore the view offset
        //    correctly. This means waiting for *all* tiles to unfullscreen, because otherwise the
        //    restored view offset will immediately get overwritten by the still screen-wide column
        //    (it uses the largest tile's width).
        //
        // 3. Changing a fullscreen tabbed column to normal should probably also restore the view
        //    offset correctly. Same problem as above, but now for normal columns (since display
        //    mode change applies instantly).
        //
        // The logic that satisfies these behaviors is to check if *any* tile is fullscreen.
        let mut any_fullscreen = false;
        let mut any_maximized = false;
        for tile in &self.tiles {
            match tile.sizing_mode() {
                SizingMode::Normal => (),
                SizingMode::Maximized => any_maximized = true,
                SizingMode::Fullscreen => any_fullscreen = true,
            }
        }

        if any_fullscreen {
            SizingMode::Fullscreen
        } else if any_maximized {
            SizingMode::Maximized
        } else {
            SizingMode::Normal
        }
    }

    pub fn contains(&self, window: &W::Id) -> bool {
        self.tiles
            .iter()
            .map(Tile::window)
            .any(|win| win.id() == window)
    }

    pub fn position(&self, window: &W::Id) -> Option<usize> {
        self.tiles
            .iter()
            .map(Tile::window)
            .position(|win| win.id() == window)
    }

    fn activate_idx(&mut self, idx: usize) -> bool {
        if self.active_tile_idx == idx {
            return false;
        }

        self.active_tile_idx = idx;

        self.tiles[idx].ensure_alpha_animates_to_1();

        true
    }

    fn activate_window(&mut self, window: &W::Id) {
        let idx = self.position(window).unwrap();
        self.activate_idx(idx);
    }

    fn add_tile_at(&mut self, idx: usize, mut tile: Tile<W>) {
        tile.update_config(self.view_size, self.scale, self.options.clone());

        // Inserting a tile pushes down all tiles below it, but also in always-centering mode it
        // will affect the X position of all tiles in the column.
        let mut prev_offsets = Vec::with_capacity(self.tiles.len() + 1);
        prev_offsets.extend(self.tile_offsets().take(self.tiles.len()));

        if self.display_mode != ColumnDisplay::Tabbed {
            self.is_pending_fullscreen = false;
            self.is_pending_maximized = false;
        }

        self.data
            .insert(idx, TileData::new(&tile, WindowHeight::auto_1()));
        self.tiles.insert(idx, tile);
        self.update_tile_sizes(true);

        // Animate tiles according to the offset changes.
        prev_offsets.insert(idx, Point::default());
        for (i, ((tile, offset), prev)) in zip(self.tiles_mut(), prev_offsets).enumerate() {
            if i == idx {
                continue;
            }

            tile.animate_move_from(prev - offset);
        }
    }

    /// Takes the tile at `idx` out of a column with more than one, for it to go to another
    /// column.
    fn take_tile(&mut self, idx: usize) -> Tile<W> {
        let tile = self.tiles.remove(idx);
        self.data.remove(idx);
        if idx < self.active_tile_idx || self.active_tile_idx == self.tiles.len() {
            self.active_tile_idx -= 1;
        }
        self.update_tile_sizes(true);
        tile
    }

    fn update_window(&mut self, window: &W::Id) {
        let (tile_idx, tile) = self
            .tiles
            .iter_mut()
            .enumerate()
            .find(|(_, tile)| tile.window().id() == window)
            .unwrap();

        let prev_height = self.data[tile_idx].size.h;

        tile.update_window();
        self.data[tile_idx].update(tile);

        let offset = prev_height - self.data[tile_idx].size.h;

        let is_tabbed = self.display_mode == ColumnDisplay::Tabbed;

        // Move windows below in tandem with resizing.
        //
        // FIXME: in always-centering mode, window resizing will affect the offsets of all other
        // windows in the column, so they should all be animated. How should this interact with
        // animated vs. non-animated resizes? For example, an animated +20 resize followed by two
        // non-animated -10 resizes.
        if !is_tabbed && offset != 0. {
            if tile.resize_animation().is_some() {
                // If there's a resize animation (that may have just started in
                // tile.update_window()), then the apparent size change is smooth with no sudden
                // jumps. This corresponds to adding an Y animation to tiles below.
                for tile in &mut self.tiles[tile_idx + 1..] {
                    tile.animate_move_y_from_with_config(
                        offset,
                        self.options.animations.window_resize.anim,
                    );
                }
            } else {
                // There's no resize animation, but the offset is nonzero. This could happen for
                // example:
                // - if the window resized on its own, which we don't animate
                // - if the window resized by less than 10 px (the resize threshold)
                //
                // The latter case could also cancel an ongoing resize animation.
                //
                // Now, stationary tiles below shouldn't react to this offset change in any way,
                // i.e. their apparent Y position should jump together with the resize. However,
                // tiles below that are already animating an Y movement should offset their
                // animations to avoid the jump.
                //
                // Notably, this is necessary to fix the animation jump when resizing height back
                // and forth in quick succession (in a way that cancels the resize animation).
                for tile in &mut self.tiles[tile_idx + 1..] {
                    tile.offset_move_y_anim_current(offset);
                }
            }
        }
    }

    /// Extra size taken up by elements in the column such as the tab indicator.
    fn extra_size(&self) -> Size<f64, Logical> {
        if self.display_mode == ColumnDisplay::Tabbed {
            self.tab_indicator.extra_size(self.tiles.len(), self.scale)
        } else {
            Size::from((0., 0.))
        }
    }

    fn resolve_preset_width(&self, preset: PresetSize) -> ResolvedSize {
        let extra = self.extra_size();
        resolve_preset_size(preset, &self.options, self.working_area.size.w, extra.w)
    }

    fn resolve_preset_height(&self, preset: PresetSize) -> ResolvedSize {
        let extra = self.extra_size();
        resolve_preset_size(preset, &self.options, self.working_area.size.h, extra.h)
    }

    fn resolve_column_width(&self, width: ColumnWidth) -> f64 {
        let working_size = self.working_area.size;
        let gaps = self.options.layout.gaps;
        let extra = self.extra_size();

        match width {
            ColumnWidth::Proportion(proportion) => {
                (working_size.w - gaps) * proportion - gaps - extra.w
            }
            ColumnWidth::Fixed(width) => width,
        }
    }

    fn update_tile_sizes(&mut self, animate: bool) {
        self.update_tile_sizes_with_transaction(animate, Transaction::new());
    }

    fn update_tile_sizes_with_transaction(&mut self, animate: bool, transaction: Transaction) {
        let sizing_mode = self.pending_sizing_mode();
        if matches!(sizing_mode, SizingMode::Fullscreen | SizingMode::Maximized) {
            for (tile_idx, tile) in self.tiles.iter_mut().enumerate() {
                // In tabbed mode, only the visible window participates in the transaction.
                let is_active = tile_idx == self.active_tile_idx;
                let transaction = if self.display_mode == ColumnDisplay::Tabbed && !is_active {
                    None
                } else {
                    Some(transaction.clone())
                };

                if matches!(sizing_mode, SizingMode::Fullscreen) {
                    tile.request_fullscreen(animate, transaction);
                } else {
                    tile.request_maximized(self.parent_area.size, animate, transaction);
                }
            }
            return;
        }

        if let Some(geometry) = self.grid_geometry {
            // Tabs share the area, minus the space for the tab indicator.
            let extra = self.extra_size();
            let size = Size::from((
                f64::max(geometry.size.w - extra.w, 1.),
                f64::max(geometry.size.h - extra.h, 1.),
            ));
            let is_tabbed = self.display_mode == ColumnDisplay::Tabbed;
            for (idx, tile) in self.tiles.iter_mut().enumerate() {
                // In tabbed mode, only the visible window participates in the transaction.
                let transaction =
                    (!is_tabbed || idx == self.active_tile_idx).then(|| transaction.clone());
                tile.request_tile_size(size, animate, transaction);
            }
            return;
        }

        let is_tabbed = self.display_mode == ColumnDisplay::Tabbed;

        let min_size: Vec<_> = self
            .tiles
            .iter()
            .map(Tile::min_size_nonfullscreen)
            .map(|mut size| {
                size.w = size.w.max(1.);
                size.h = size.h.max(1.);
                size
            })
            .collect();
        let max_size: Vec<_> = self
            .tiles
            .iter()
            .map(Tile::max_size_nonfullscreen)
            .collect();

        // Compute the column width.
        let min_width = min_size
            .iter()
            .map(|size| NotNan::new(size.w).unwrap())
            .max()
            .map(NotNan::into_inner)
            .unwrap();
        let max_width = max_size
            .iter()
            .filter_map(|size| {
                let w = size.w;
                if w == 0. {
                    None
                } else {
                    Some(NotNan::new(w).unwrap())
                }
            })
            .min()
            .map(NotNan::into_inner)
            .unwrap_or(f64::from(i32::MAX));
        let max_width = f64::max(max_width, min_width);

        let width = if self.is_full_width {
            ColumnWidth::Proportion(1.)
        } else {
            self.width
        };

        let working_size = self.working_area.size;
        let extra_size = self.extra_size();

        let width = self.resolve_column_width(width);
        let width = f64::max(f64::min(width, max_width), min_width);
        let max_tile_height = working_size.h - self.options.layout.gaps * 2. - extra_size.h;

        // If there are multiple windows in a column, clamp the non-auto window's height according
        // to other windows' min sizes.
        let mut max_non_auto_window_height = None;
        if self.tiles.len() > 1 && !is_tabbed {
            if let Some(non_auto_idx) = self
                .data
                .iter()
                .position(|data| !matches!(data.height, WindowHeight::Auto { .. }))
            {
                let min_height_taken = min_size
                    .iter()
                    .enumerate()
                    .filter(|(idx, _)| *idx != non_auto_idx)
                    .map(|(_, min_size)| min_size.h + self.options.layout.gaps)
                    .sum::<f64>();

                let tile = &self.tiles[non_auto_idx];
                let height_left = max_tile_height - min_height_taken;
                max_non_auto_window_height = Some(f64::max(
                    1.,
                    tile.window_height_for_tile_height(height_left).round(),
                ));
            }
        }

        // Compute the tile heights. Start by converting window heights to tile heights.
        let mut heights = zip(&self.tiles, &self.data)
            .map(|(tile, data)| match data.height {
                auto @ WindowHeight::Auto { .. } => auto,
                WindowHeight::Fixed(height) => {
                    let mut window_height = height.round().max(1.);
                    if let Some(max) = max_non_auto_window_height {
                        window_height = f64::min(window_height, max);
                    } else {
                        // In any case, clamp to the working area height.
                        let max = tile.window_height_for_tile_height(max_tile_height).round();
                        window_height = f64::min(window_height, max);
                    }

                    WindowHeight::Fixed(tile.tile_height_for_window_height(window_height))
                }
                WindowHeight::Preset(idx) => {
                    let preset = self.options.layout.preset_window_heights[idx];
                    let window_height = match self.resolve_preset_height(preset) {
                        ResolvedSize::Tile(h) => tile.window_height_for_tile_height(h),
                        ResolvedSize::Window(h) => h,
                    };

                    let mut window_height = window_height.round().clamp(1., 100000.);
                    if let Some(max) = max_non_auto_window_height {
                        window_height = f64::min(window_height, max);
                    }

                    let tile_height = tile.tile_height_for_window_height(window_height);
                    WindowHeight::Fixed(tile_height)
                }
            })
            .collect::<Vec<_>>();

        // In tabbed display mode, fill fixed heights right away.
        if is_tabbed {
            // All tiles have the same height, equal to the height of the only fixed tile (if any).
            let tabbed_height = heights
                .iter()
                .find_map(|h| {
                    if let WindowHeight::Fixed(h) = h {
                        Some(*h)
                    } else {
                        None
                    }
                })
                .unwrap_or(max_tile_height);

            // We also take min height of all tabs into account.
            let min_height = min_size
                .iter()
                .map(|size| NotNan::new(size.h).unwrap())
                .max()
                .map(NotNan::into_inner)
                .unwrap();
            // But, if there's a larger-than-workspace tab, we don't want to force all tabs to that
            // size.
            let min_height = f64::min(max_tile_height, min_height);
            let tabbed_height = f64::max(tabbed_height, min_height);

            heights.fill(WindowHeight::Fixed(tabbed_height));

            // The following logic will apply individual min/max height, etc.
        }

        let gaps_left = self.options.layout.gaps * (self.tiles.len() + 1) as f64;
        let mut height_left = working_size.h - gaps_left;
        let mut auto_tiles_left = self.tiles.len();

        // Subtract all fixed-height tiles.
        for (h, (min_size, max_size)) in zip(&mut heights, zip(&min_size, &max_size)) {
            // Check if the tile has an exact height constraint.
            if min_size.h == max_size.h {
                *h = WindowHeight::Fixed(min_size.h);
            }

            if let WindowHeight::Fixed(h) = h {
                if max_size.h > 0. {
                    *h = f64::min(*h, max_size.h);
                }
                *h = f64::max(*h, min_size.h);

                height_left -= *h;
                auto_tiles_left -= 1;
            }
        }

        let mut total_weight: f64 = heights
            .iter()
            .filter_map(|h| {
                if let WindowHeight::Auto { weight } = *h {
                    Some(weight)
                } else {
                    None
                }
            })
            .sum();

        // Iteratively try to distribute the remaining height, checking against tile min heights.
        // Pick an auto height according to the current sizes, then check if it satisfies all
        // remaining min heights. If not, allocate fixed height to those tiles and repeat the
        // loop. On each iteration the auto height will get smaller.
        //
        // NOTE: we do not respect max height here. Doing so would complicate things: if the current
        // auto height is above some tile's max height, then the auto height can become larger.
        // Combining this with the min height loop is where the complexity appears.
        //
        // However, most max height uses are for fixed-size dialogs, where min height == max_height.
        // This case is separately handled above.
        'outer: while auto_tiles_left > 0 {
            // Wayland requires us to round the requested size for a window to integer logical
            // pixels, therefore we compute the remaining auto height dynamically.
            let mut height_left_2 = height_left;
            let mut total_weight_2 = total_weight;
            for ((h, tile), min_size) in zip(zip(&mut heights, &self.tiles), &min_size) {
                let weight = match *h {
                    WindowHeight::Auto { weight } => weight,
                    WindowHeight::Fixed(_) => continue,
                    WindowHeight::Preset(_) => unreachable!(),
                };
                let factor = weight / total_weight_2;

                // Compute the current auto height.
                let mut auto = height_left_2 * factor;

                // Check if the auto height satisfies the min height.
                if min_size.h > auto {
                    auto = min_size.h;
                    *h = WindowHeight::Fixed(auto);
                    height_left -= auto;
                    total_weight -= weight;
                    auto_tiles_left -= 1;

                    // If a min height was unsatisfied, then we allocate the tile more than the
                    // auto height, which means that the remaining auto tiles now have less height
                    // to work with, and the loop must run again.
                    //
                    // If we keep going in this loop and break out later, we may allocate less
                    // height to the subsequent tiles than would be available next iteration and
                    // potentially trip their min height check earlier than necessary, leading to
                    // visible snapping.
                    continue 'outer;
                }

                auto = tile.tile_height_for_window_height(
                    tile.window_height_for_tile_height(auto).round().max(1.),
                );

                height_left_2 -= auto;
                total_weight_2 -= weight;
            }

            // All min heights were satisfied, fill them in.
            for (h, tile) in zip(&mut heights, &self.tiles) {
                let weight = match *h {
                    WindowHeight::Auto { weight } => weight,
                    WindowHeight::Fixed(_) => continue,
                    WindowHeight::Preset(_) => unreachable!(),
                };
                let factor = weight / total_weight;

                // Compute the current auto height.
                let auto = height_left * factor;
                let auto = tile.tile_height_for_window_height(
                    tile.window_height_for_tile_height(auto).round().max(1.),
                );

                *h = WindowHeight::Fixed(auto);
                height_left -= auto;
                total_weight -= weight;
                auto_tiles_left -= 1;
            }

            assert_eq!(auto_tiles_left, 0);
        }

        for (tile_idx, (tile, h)) in zip(&mut self.tiles, heights).enumerate() {
            let WindowHeight::Fixed(height) = h else {
                unreachable!()
            };

            let size = Size::from((width, height));

            // In tabbed mode, only the visible window participates in the transaction.
            let is_active = tile_idx == self.active_tile_idx;
            let transaction = if self.display_mode == ColumnDisplay::Tabbed && !is_active {
                None
            } else {
                Some(transaction.clone())
            };

            tile.request_tile_size(size, animate, transaction);
        }
    }

    fn width(&self) -> f64 {
        let mut tiles_width = self
            .data
            .iter()
            .map(|data| NotNan::new(data.size.w).unwrap())
            .max()
            .map(NotNan::into_inner)
            .unwrap();

        if self.display_mode == ColumnDisplay::Tabbed && self.sizing_mode().is_normal() {
            let extra_size = self.tab_indicator.extra_size(self.tiles.len(), self.scale);
            tiles_width += extra_size.w;
        }

        tiles_width
    }

    fn focus_index(&mut self, index: u8) {
        let idx = min(usize::from(index.saturating_sub(1)), self.tiles.len() - 1);
        self.activate_idx(idx);
    }

    fn focus_up(&mut self) -> bool {
        self.activate_idx(self.active_tile_idx.saturating_sub(1))
    }

    fn focus_down(&mut self) -> bool {
        self.activate_idx(min(self.active_tile_idx + 1, self.tiles.len() - 1))
    }

    fn focus_top(&mut self) {
        self.activate_idx(0);
    }

    fn focus_bottom(&mut self) {
        self.activate_idx(self.tiles.len() - 1);
    }

    fn move_up(&mut self) -> bool {
        let new_idx = self.active_tile_idx.saturating_sub(1);
        if self.active_tile_idx == new_idx {
            return false;
        }

        let mut ys = self.tile_offsets().skip(self.active_tile_idx);
        let active_y = ys.next().unwrap().y;
        let next_y = ys.next().unwrap().y;
        drop(ys);

        self.tiles.swap(self.active_tile_idx, new_idx);
        self.data.swap(self.active_tile_idx, new_idx);
        self.active_tile_idx = new_idx;

        // Animate the movement.
        let new_active_y = self.tile_offset(new_idx).y;
        self.tiles[new_idx].animate_move_y_from(active_y - new_active_y);
        self.tiles[new_idx + 1].animate_move_y_from(active_y - next_y);

        true
    }

    fn move_down(&mut self) -> bool {
        let new_idx = min(self.active_tile_idx + 1, self.tiles.len() - 1);
        if self.active_tile_idx == new_idx {
            return false;
        }

        let mut ys = self.tile_offsets().skip(self.active_tile_idx);
        let active_y = ys.next().unwrap().y;
        let next_y = ys.next().unwrap().y;
        drop(ys);

        self.tiles.swap(self.active_tile_idx, new_idx);
        self.data.swap(self.active_tile_idx, new_idx);
        self.active_tile_idx = new_idx;

        // Animate the movement.
        let new_active_y = self.tile_offset(new_idx).y;
        self.tiles[new_idx].animate_move_y_from(active_y - new_active_y);
        self.tiles[new_idx - 1].animate_move_y_from(next_y - active_y);

        true
    }

    fn toggle_width(&mut self, tile_idx: Option<usize>, forwards: bool) {
        let tile_idx = tile_idx.unwrap_or(self.active_tile_idx);

        let preset_idx = if self.is_full_width || self.is_pending_maximized {
            None
        } else {
            self.preset_width_idx
        };

        let len = self.options.layout.preset_column_widths.len();
        let preset_idx = if let Some(idx) = preset_idx {
            (idx + if forwards { 1 } else { len - 1 }) % len
        } else {
            let tile = &self.tiles[tile_idx];
            let current_window = tile.window_expected_or_current_size().w;
            let current_tile = tile.tile_expected_or_current_size().w;

            let mut it = self
                .options
                .layout
                .preset_column_widths
                .iter()
                .map(|preset| self.resolve_preset_width(*preset));

            if forwards {
                it.position(|resolved| {
                    match resolved {
                        // Some allowance for fractional scaling purposes.
                        ResolvedSize::Tile(resolved) => current_tile + 1. < resolved,
                        ResolvedSize::Window(resolved) => current_window + 1. < resolved,
                    }
                })
                .unwrap_or(0)
            } else {
                it.rposition(|resolved| {
                    match resolved {
                        // Some allowance for fractional scaling purposes.
                        ResolvedSize::Tile(resolved) => resolved + 1. < current_tile,
                        ResolvedSize::Window(resolved) => resolved + 1. < current_window,
                    }
                })
                .unwrap_or(len - 1)
            }
        };

        let preset = self.options.layout.preset_column_widths[preset_idx];
        self.set_column_width(SizeChange::from(preset), Some(tile_idx), true);

        self.preset_width_idx = Some(preset_idx);
    }

    fn toggle_full_width(&mut self) {
        if self.is_pending_maximized {
            // Treat it as unmaximize.
            self.is_pending_maximized = false;
            self.is_full_width = false;
        } else {
            self.is_full_width = !self.is_full_width;
        }

        self.update_tile_sizes(true);
    }

    fn set_column_width(&mut self, change: SizeChange, tile_idx: Option<usize>, animate: bool) {
        let current = if self.is_full_width || self.is_pending_maximized {
            ColumnWidth::Proportion(1.)
        } else {
            self.width
        };

        let current_px = self.resolve_column_width(current);

        // FIXME: fix overflows then remove limits.
        const MAX_PX: f64 = 100000.;
        const MAX_F: f64 = 10000.;

        let width = match (current, change) {
            (_, SizeChange::SetFixed(fixed)) => {
                // As a special case, setting a fixed column width will compute it in such a way
                // that the specified (usually active) window gets that width. This is the
                // intention behind the ability to set a fixed size.
                let tile_idx = tile_idx.unwrap_or(self.active_tile_idx);
                let tile = &self.tiles[tile_idx];
                ColumnWidth::Fixed(
                    tile.tile_width_for_window_width(f64::from(fixed))
                        .clamp(1., MAX_PX),
                )
            }
            (_, SizeChange::SetProportion(proportion)) => {
                ColumnWidth::Proportion((proportion / 100.).clamp(0., MAX_F))
            }
            (_, SizeChange::AdjustFixed(delta)) => {
                let width = (current_px + f64::from(delta)).clamp(1., MAX_PX);
                ColumnWidth::Fixed(width)
            }
            (ColumnWidth::Proportion(current), SizeChange::AdjustProportion(delta)) => {
                let proportion = (current + delta / 100.).clamp(0., MAX_F);
                ColumnWidth::Proportion(proportion)
            }
            (ColumnWidth::Fixed(_), SizeChange::AdjustProportion(delta)) => {
                let full = self.working_area.size.w - self.options.layout.gaps;
                let current = if full == 0. {
                    1.
                } else {
                    (current_px + self.options.layout.gaps + self.extra_size().w) / full
                };
                let proportion = (current + delta / 100.).clamp(0., MAX_F);
                ColumnWidth::Proportion(proportion)
            }
        };

        self.width = width;
        self.preset_width_idx = None;
        self.is_full_width = false;
        self.is_pending_maximized = false;
        self.update_tile_sizes(animate);
    }

    fn set_window_height(&mut self, change: SizeChange, tile_idx: Option<usize>, animate: bool) {
        let tile_idx = tile_idx.unwrap_or(self.active_tile_idx);

        // Start by converting all heights to automatic, since only one window in the column can be
        // non-auto-height. If the current tile is already non-auto, however, we can skip that
        // step. Which is not only for optimization, but also preserves automatic weights in case
        // one window is resized in such a way that other windows hit their min size, and then
        // back.
        if matches!(self.data[tile_idx].height, WindowHeight::Auto { .. }) {
            self.convert_heights_to_auto();
        }

        let current = self.data[tile_idx].height;
        let tile = &self.tiles[tile_idx];
        let current_window_px = match current {
            WindowHeight::Auto { .. } | WindowHeight::Preset(_) => tile.window_size().h,
            WindowHeight::Fixed(height) => height,
        };
        let current_tile_px = tile.tile_height_for_window_height(current_window_px);

        let working_size = self.working_area.size.h;
        let gaps = self.options.layout.gaps;
        let extra_size = self.extra_size().h;
        let full = working_size - gaps;
        let current_prop = if full == 0. {
            1.
        } else {
            (current_tile_px + gaps) / full
        };

        // FIXME: fix overflows then remove limits.
        const MAX_PX: f64 = 100000.;

        let mut window_height = match change {
            SizeChange::SetFixed(fixed) => f64::from(fixed),
            SizeChange::SetProportion(proportion) => {
                let tile_height = (working_size - gaps) * (proportion / 100.) - gaps - extra_size;
                tile.window_height_for_tile_height(tile_height)
            }
            SizeChange::AdjustFixed(delta) => current_window_px + f64::from(delta),
            SizeChange::AdjustProportion(delta) => {
                let proportion = current_prop + delta / 100.;
                let tile_height = (working_size - gaps) * proportion - gaps - extra_size;
                tile.window_height_for_tile_height(tile_height)
            }
        };

        // Clamp the height according to other windows' min sizes, or simply to working area height.
        let min_height_taken = if self.display_mode == ColumnDisplay::Tabbed {
            0.
        } else {
            self.tiles
                .iter()
                .enumerate()
                .filter(|(idx, _)| *idx != tile_idx)
                .map(|(_, tile)| f64::max(1., tile.min_size_nonfullscreen().h) + gaps)
                .sum::<f64>()
        };
        let height_left = working_size - extra_size - gaps - min_height_taken - gaps;
        let height_left = f64::max(1., tile.window_height_for_tile_height(height_left));
        window_height = f64::min(height_left, window_height);

        // Clamp it against the window height constraints.
        let win = &self.tiles[tile_idx].window();
        let min_h = win.min_size().h;
        let max_h = win.max_size().h;

        if max_h > 0 {
            window_height = f64::min(window_height, f64::from(max_h));
        }
        if min_h > 0 {
            window_height = f64::max(window_height, f64::from(min_h));
        }

        self.data[tile_idx].height = WindowHeight::Fixed(window_height.clamp(1., MAX_PX));
        self.is_pending_maximized = false;
        self.update_tile_sizes(animate);
    }

    fn reset_window_height(&mut self, tile_idx: Option<usize>) {
        if self.display_mode == ColumnDisplay::Tabbed {
            // When tabbed, reset window height should work on any window, not just the fixed-size
            // one.
            for data in &mut self.data {
                data.height = WindowHeight::auto_1();
            }
        } else {
            let tile_idx = tile_idx.unwrap_or(self.active_tile_idx);
            self.data[tile_idx].height = WindowHeight::auto_1();
        }

        self.update_tile_sizes(true);
    }

    fn toggle_window_height(&mut self, tile_idx: Option<usize>, forwards: bool) {
        let tile_idx = tile_idx.unwrap_or(self.active_tile_idx);

        // Start by converting all heights to automatic, since only one window in the column can be
        // non-auto-height. If the current tile is already non-auto, however, we can skip that
        // step. Which is not only for optimization, but also preserves automatic weights in case
        // one window is resized in such a way that other windows hit their min size, and then
        // back.
        if matches!(self.data[tile_idx].height, WindowHeight::Auto { .. }) {
            self.convert_heights_to_auto();
        }

        let len = self.options.layout.preset_window_heights.len();
        let preset_idx = match self.data[tile_idx].height {
            WindowHeight::Preset(idx) if !self.is_pending_maximized => {
                (idx + if forwards { 1 } else { len - 1 }) % len
            }
            _ => {
                let current = self.data[tile_idx].size.h;
                let tile = &self.tiles[tile_idx];

                let mut it = self
                    .options
                    .layout
                    .preset_window_heights
                    .iter()
                    .copied()
                    .map(|preset| {
                        let window_height = match self.resolve_preset_height(preset) {
                            ResolvedSize::Tile(h) => tile.window_height_for_tile_height(h),
                            ResolvedSize::Window(h) => h,
                        };
                        tile.tile_height_for_window_height(window_height.round().clamp(1., 100000.))
                    });

                if forwards {
                    it.position(|resolved| {
                        // Some allowance for fractional scaling purposes.
                        current + 1. < resolved
                    })
                    .unwrap_or(0)
                } else {
                    it.rposition(|resolved| {
                        // Some allowance for fractional scaling purposes.
                        resolved + 1. < current
                    })
                    .unwrap_or(len - 1)
                }
            }
        };
        self.data[tile_idx].height = WindowHeight::Preset(preset_idx);
        self.is_pending_maximized = false;
        self.update_tile_sizes(true);
    }

    /// Converts all heights in the column to automatic, preserving the apparent heights.
    ///
    /// All weights are recomputed to preserve the current tile heights while "centering" the
    /// weights at the median window height (it gets weight = 1).
    ///
    /// One case where apparent heights will not be preserved is when the column is taller than the
    /// working area.
    fn convert_heights_to_auto(&mut self) {
        let heights: Vec<_> = self.tiles.iter().map(|tile| tile.tile_size().h).collect();

        // Weights are invariant to multiplication: a column with weights 2, 2, 1 is equivalent to
        // a column with weights 4, 4, 2. So we find the median window height and use that as 1.
        let mut sorted = heights.clone();
        sorted.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        let median = sorted[sorted.len() / 2];

        for (data, height) in zip(&mut self.data, heights) {
            let weight = height / median;
            data.height = WindowHeight::Auto { weight };
        }
    }

    fn set_fullscreen(&mut self, is_fullscreen: bool) {
        if self.is_pending_fullscreen == is_fullscreen {
            return;
        }

        if is_fullscreen {
            assert!(self.tiles.len() == 1 || self.display_mode == ColumnDisplay::Tabbed);
        }

        self.is_pending_fullscreen = is_fullscreen;
        self.update_tile_sizes(true);
    }

    fn set_maximized(&mut self, maximize: bool) {
        if self.is_pending_maximized == maximize {
            return;
        }

        if maximize {
            assert!(self.tiles.len() == 1 || self.display_mode == ColumnDisplay::Tabbed);
        }

        self.is_pending_maximized = maximize;
        self.update_tile_sizes(true);
    }

    fn set_column_display(&mut self, display: ColumnDisplay) {
        if self.display_mode == display {
            return;
        }

        // Animate the movement.
        //
        // We're doing some shortcuts here because we know that currently normal vs. tabbed can
        // only cause a vertical shift + a shift to the origin.
        //
        // Doing it this way to avoid storing all tile positions in a vector. If more display modes
        // are added it might be simpler to just collect everything into a smallvec.
        let prev_origin = self.tiles_origin();
        self.display_mode = display;
        let new_origin = self.tiles_origin();
        let origin_delta = prev_origin - new_origin;

        // When need to walk the tiles in the normal display mode to get the right offsets.
        self.display_mode = ColumnDisplay::Normal;
        for (tile, pos) in self.tiles_mut() {
            let mut y_delta = pos.y - prev_origin.y;

            // Invert the Y motion when transitioning *to* normal display mode.
            if display == ColumnDisplay::Normal {
                y_delta *= -1.;
            }

            let mut delta = origin_delta;
            delta.y += y_delta;
            tile.animate_move_from(delta);
        }

        // Animate the opacity.
        for (idx, tile) in self.tiles.iter_mut().enumerate() {
            let is_active = idx == self.active_tile_idx;
            if !is_active {
                let (from, to) = if display == ColumnDisplay::Tabbed {
                    (1., 0.)
                } else {
                    (0., 1.)
                };
                tile.animate_alpha(from, to, self.options.animations.window_movement.0);
            }
        }

        // Animate the appearance of the tab indicator.
        if display == ColumnDisplay::Tabbed {
            self.tab_indicator.start_open_animation(
                self.clock.clone(),
                self.options.animations.window_movement.0,
            );
        }

        // Now switch the display mode for real.
        self.display_mode = display;
        self.update_tile_sizes(true);
    }

    fn tiles_origin(&self) -> Point<f64, Logical> {
        let mut origin = Point::from((0., 0.));

        match self.sizing_mode() {
            SizingMode::Normal => (),
            SizingMode::Maximized => {
                origin.y += self.parent_area.loc.y;
                return origin;
            }
            SizingMode::Fullscreen => return origin,
        }

        if let Some(geometry) = self.grid_geometry {
            origin.y += self.working_area.loc.y + geometry.y;
            if self.display_mode == ColumnDisplay::Tabbed {
                origin += self
                    .tab_indicator
                    .content_offset(self.tiles.len(), self.scale);
            }
            return origin;
        }

        origin.y += self.working_area.loc.y + self.options.layout.gaps;

        if self.display_mode == ColumnDisplay::Tabbed {
            origin += self
                .tab_indicator
                .content_offset(self.tiles.len(), self.scale);
        }

        origin
    }

    // HACK: pass a self.data iterator in manually as a workaround for the lack of method partial
    // borrowing. Note that this method's return value does not borrow the entire &Self!
    fn tile_offsets_iter(
        &self,
        data: impl Iterator<Item = TileData>,
    ) -> impl Iterator<Item = Point<f64, Logical>> {
        // FIXME: this should take into account always-center-single-column, which means that
        // Column should somehow know when it is being centered due to being the single column on
        // the workspace or some other reason.
        let center = self.options.layout.center_focused_column == CenterFocusedColumn::Always;
        let gaps = self.options.layout.gaps;
        let tabbed = self.display_mode == ColumnDisplay::Tabbed;

        // Does not include extra size from the tab indicator.
        let tiles_width = self
            .data
            .iter()
            .map(|data| NotNan::new(data.size.w).unwrap())
            .max()
            .map(NotNan::into_inner)
            .unwrap_or(0.);

        let mut origin = self.tiles_origin();

        // Chain with a dummy value to be able to get one past all tiles' Y.
        let dummy = TileData {
            height: WindowHeight::auto_1(),
            size: Size::default(),
            interactively_resizing_by_left_edge: false,
        };
        let data = data.chain(iter::once(dummy));

        data.map(move |data| {
            let mut pos = origin;

            if center {
                pos.x += (tiles_width - data.size.w) / 2.;
            } else if data.interactively_resizing_by_left_edge {
                pos.x += tiles_width - data.size.w;
            }

            if !tabbed {
                origin.y += data.size.h + gaps;
            }

            pos
        })
    }

    fn tile_offsets(&self) -> impl Iterator<Item = Point<f64, Logical>> + '_ {
        self.tile_offsets_iter(self.data.iter().copied())
    }

    fn tile_offset(&self, tile_idx: usize) -> Point<f64, Logical> {
        self.tile_offsets().nth(tile_idx).unwrap()
    }

    fn tile_offsets_in_render_order(
        &self,
        data: impl Iterator<Item = TileData>,
    ) -> impl Iterator<Item = Point<f64, Logical>> {
        let active_idx = self.active_tile_idx;
        let active_pos = self.tile_offset(active_idx);
        let offsets = self
            .tile_offsets_iter(data)
            .enumerate()
            .filter_map(move |(idx, pos)| (idx != active_idx).then_some(pos));
        iter::once(active_pos).chain(offsets)
    }

    pub fn tiles(&self) -> impl Iterator<Item = (&Tile<W>, Point<f64, Logical>)> + '_ {
        let offsets = self.tile_offsets_iter(self.data.iter().copied());
        zip(&self.tiles, offsets)
    }

    fn tiles_mut(&mut self) -> impl Iterator<Item = (&mut Tile<W>, Point<f64, Logical>)> + '_ {
        let offsets = self.tile_offsets_iter(self.data.iter().copied());
        zip(&mut self.tiles, offsets)
    }

    fn tiles_in_render_order(
        &self,
    ) -> impl Iterator<Item = (&Tile<W>, Point<f64, Logical>, bool)> + '_ {
        let offsets = self.tile_offsets_in_render_order(self.data.iter().copied());

        let (first, rest) = self.tiles.split_at(self.active_tile_idx);
        let (active, rest) = rest.split_at(1);

        let active = active.iter().map(|tile| (tile, true));

        let rest_visible = self.display_mode != ColumnDisplay::Tabbed;
        let rest = first.iter().chain(rest);
        let rest = rest.map(move |tile| (tile, rest_visible));

        let tiles = active.chain(rest);
        zip(tiles, offsets).map(|((tile, visible), pos)| (tile, pos, visible))
    }

    fn tiles_in_render_order_mut(
        &mut self,
    ) -> impl Iterator<Item = (&mut Tile<W>, Point<f64, Logical>)> + '_ {
        let offsets = self.tile_offsets_in_render_order(self.data.iter().copied());

        let (first, rest) = self.tiles.split_at_mut(self.active_tile_idx);
        let (active, rest) = rest.split_at_mut(1);

        let tiles = active.iter_mut().chain(first).chain(rest);
        zip(tiles, offsets)
    }

    fn tab_indicator_area(&self) -> Rectangle<f64, Logical> {
        // We'd like to use the active tile's animated size for the tab indicator, however we need
        // to be mindful of the case where the active tile is smaller than some other tile in the
        // column. The column assumes the size of the largest tile.
        //
        // We expect users to mainly resize tabbed columns by width, so matching the animated size
        // is more important here. Besides, we always try to resize all windows in a column to the
        // same width when possible, and also the animation for going into tabbed mode doesn't move
        // tiles horizontally as much.
        //
        // For height though, it's a different story. First, users probably aren't resizing a
        // tabbed column by height. Second, we don't match windows by height, so it's easy to have
        // a smaller active tile than the rest of the column, e.g. by adding a fixed-size dialog.
        // Then, switching to that dialog and back should ideally keep the tab indicator position
        // fixed. Third, the animation for making a column tabbed moves tiles vertically, and using
        // the active tile's animated size in this case only works for the topmost tile, and looks
        // broken otherwise.
        let mut max_height = 0.;
        for tile in &self.tiles {
            max_height = f64::max(max_height, tile.tile_size().h);
        }

        let tile = &self.tiles[self.active_tile_idx];
        let area_size = Size::from((tile.animated_tile_size().w, max_height));

        Rectangle::new(self.tiles_origin(), area_size)
    }

    pub fn start_open_animation(&mut self, id: &W::Id) -> bool {
        for tile in &mut self.tiles {
            if tile.window().id() == id {
                tile.start_open_animation();

                // Animate the appearance of the tab indicator.
                if self.display_mode == ColumnDisplay::Tabbed
                    && self.sizing_mode().is_normal()
                    && self.tiles.len() == 1
                    && !self.tab_indicator.config().hide_when_single_tab
                {
                    self.tab_indicator.start_open_animation(
                        self.clock.clone(),
                        self.options.animations.window_open.anim,
                    );
                }

                return true;
            }
        }

        false
    }

    #[cfg(test)]
    fn verify_invariants(&self) {
        assert!(!self.tiles.is_empty(), "columns can't be empty");
        assert!(self.active_tile_idx < self.tiles.len());
        assert_eq!(self.tiles.len(), self.data.len());

        if !self.pending_sizing_mode().is_normal() {
            assert!(self.tiles.len() == 1 || self.display_mode == ColumnDisplay::Tabbed);
        }

        if let Some(idx) = self.preset_width_idx {
            assert!(idx < self.options.layout.preset_column_widths.len());
        }

        let is_tabbed = self.display_mode == ColumnDisplay::Tabbed;

        let tile_count = self.tiles.len();
        if tile_count == 1 {
            if let WindowHeight::Auto { weight } = self.data[0].height {
                assert_eq!(
                    weight, 1.,
                    "auto height weight must reset to 1 for a single window"
                );
            }
        }

        let working_size = self.working_area.size;
        let extra_size = self.extra_size();
        let gaps = self.options.layout.gaps;

        let mut found_fixed = false;
        let mut total_height = 0.;
        let mut total_min_height = 0.;
        for (tile, data) in zip(&self.tiles, &self.data) {
            assert!(Rc::ptr_eq(&self.options, &tile.options));
            assert_eq!(self.clock, tile.clock);
            assert_eq!(self.scale, tile.scale());
            assert_eq!(
                self.pending_sizing_mode(),
                tile.window().pending_sizing_mode()
            );
            assert_eq!(self.view_size, tile.view_size());
            tile.verify_invariants();

            let mut data2 = *data;
            data2.update(tile);
            assert_eq!(data, &data2, "tile data must be up to date");

            if matches!(data.height, WindowHeight::Fixed(_)) {
                assert!(
                    !found_fixed,
                    "there can only be one fixed-height window in a column"
                );
                found_fixed = true;
            }

            if let WindowHeight::Preset(idx) = data.height {
                assert!(self.options.layout.preset_window_heights.len() > idx);
            }

            let requested_size = tile.window().requested_size().unwrap();
            let requested_tile_height =
                tile.tile_height_for_window_height(f64::from(requested_size.h));
            let min_tile_height = f64::max(1., tile.min_size_nonfullscreen().h);

            if !is_tabbed
                && self.pending_sizing_mode().is_normal()
                && self.scale.round() == self.scale
                && working_size.h.round() == working_size.h
                && gaps.round() == gaps
            {
                let total_height = requested_tile_height + gaps * 2. + extra_size.h;
                let total_min_height = min_tile_height + gaps * 2. + extra_size.h;
                let max_height = f64::max(total_min_height, working_size.h);
                assert!(
                    total_height <= max_height,
                    "each tile in a column mustn't go beyond working area height \
                     (tile height {total_height} > max height {max_height})"
                );
            }

            total_height += requested_tile_height;
            total_min_height += min_tile_height;
        }

        if !is_tabbed
            && tile_count > 1
            && self.scale.round() == self.scale
            && working_size.h.round() == working_size.h
            && gaps.round() == gaps
        {
            total_height += gaps * (tile_count + 1) as f64 + extra_size.h;
            total_min_height += gaps * (tile_count + 1) as f64 + extra_size.h;
            let max_height = f64::max(total_min_height, working_size.h);
            assert!(
                total_height <= max_height,
                "multiple tiles in a column mustn't go beyond working area height \
                 (total height {total_height} > max height {max_height})"
            );
        }
    }
}

fn compute_new_view_offset(
    cur_x: f64,
    view_width: f64,
    new_col_x: f64,
    new_col_width: f64,
    gaps: f64,
) -> f64 {
    // If the column is wider than the view, always left-align it.
    if view_width <= new_col_width {
        return 0.;
    }

    // Compute the padding in case it needs to be smaller due to large tile width.
    let padding = ((view_width - new_col_width) / 2.).clamp(0., gaps);

    // Compute the desired new X with padding.
    let new_x = new_col_x - padding;
    let new_right_x = new_col_x + new_col_width + padding;

    // If the column is already fully visible, leave the view as is.
    if cur_x <= new_x && new_right_x <= cur_x + view_width {
        return -(new_col_x - cur_x);
    }

    // Otherwise, prefer the alignment that results in less motion from the current position.
    let dist_to_left = (cur_x - new_x).abs();
    let dist_to_right = ((cur_x + view_width) - new_right_x).abs();
    if dist_to_left <= dist_to_right {
        -padding
    } else {
        -(view_width - padding - new_col_width)
    }
}

fn compute_cell_metrics(working_area: Rectangle<f64, Logical>, options: &Options) -> CellMetrics {
    CellMetrics::new(working_area.size, grid::DEFAULT_ROWS, options.layout.gaps)
}

fn compute_working_area(
    parent_area: Rectangle<f64, Logical>,
    scale: f64,
    struts: Struts,
) -> Rectangle<f64, Logical> {
    let mut working_area = parent_area;

    // Add struts.
    working_area.size.w = f64::max(0., working_area.size.w - struts.left.0 - struts.right.0);
    working_area.loc.x += struts.left.0;

    working_area.size.h = f64::max(0., working_area.size.h - struts.top.0 - struts.bottom.0);
    working_area.loc.y += struts.top.0;

    // Round location to start at a physical pixel.
    let loc = working_area
        .loc
        .to_physical_precise_ceil(scale)
        .to_logical(scale);

    let mut size_diff = (loc - working_area.loc).to_size();
    size_diff.w = f64::min(working_area.size.w, size_diff.w);
    size_diff.h = f64::min(working_area.size.h, size_diff.h);

    working_area.size -= size_diff;
    working_area.loc = loc;

    working_area
}

fn compute_toplevel_bounds(
    border_config: niri_config::Border,
    working_area_size: Size<f64, Logical>,
    extra_size: Size<f64, Logical>,
    gaps: f64,
) -> Size<i32, Logical> {
    let mut border = 0.;
    if !border_config.off {
        border = border_config.width * 2.;
    }

    Size::from((
        f64::max(working_area_size.w - gaps * 2. - extra_size.w - border, 1.),
        f64::max(working_area_size.h - gaps * 2. - extra_size.h - border, 1.),
    ))
    .to_i32_floor()
}

fn cancel_resize_for_column<W: LayoutElement>(
    interactive_resize: &mut Option<InteractiveResize<W>>,
    column: &mut Column<W>,
) {
    if let Some(resize) = interactive_resize {
        if column.contains(&resize.window) {
            *interactive_resize = None;
        }
    }

    for tile in &mut column.tiles {
        tile.window_mut().cancel_interactive_resize();
    }
}

fn resolve_preset_size(
    preset: PresetSize,
    options: &Options,
    view_size: f64,
    extra_size: f64,
) -> ResolvedSize {
    match preset {
        PresetSize::Proportion(proportion) => ResolvedSize::Tile(
            (view_size - options.layout.gaps) * proportion - options.layout.gaps - extra_size,
        ),
        PresetSize::Fixed(width) => ResolvedSize::Window(f64::from(width)),
    }
}

#[cfg(test)]
mod tests {
    use niri_config::FloatOrInt;

    use super::*;
    use crate::utils::round_logical_in_physical;

    #[test]
    fn working_area_starts_at_physical_pixel() {
        let struts = Struts {
            left: FloatOrInt(0.5),
            right: FloatOrInt(1.),
            top: FloatOrInt(0.75),
            bottom: FloatOrInt(1.),
        };

        let parent_area = Rectangle::from_size(Size::from((1280., 720.)));
        let area = compute_working_area(parent_area, 1., struts);

        assert_eq!(round_logical_in_physical(1., area.loc.x), area.loc.x);
        assert_eq!(round_logical_in_physical(1., area.loc.y), area.loc.y);
    }

    #[test]
    fn large_fractional_strut() {
        let struts = Struts {
            left: FloatOrInt(0.),
            right: FloatOrInt(0.),
            top: FloatOrInt(50000.5),
            bottom: FloatOrInt(0.),
        };

        let parent_area = Rectangle::from_size(Size::from((1280., 720.)));
        compute_working_area(parent_area, 1., struts);
    }
}
