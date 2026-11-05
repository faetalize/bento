//! The tiling layout, where a layout engine decides where windows go.
//!
//! This is the compositor half of bento's scriptable layouts. The space holds the tiled windows
//! of a workspace, a camera over an unbounded canvas and the tree of nodes that the layout draws.
//! The layout engine (see [`engine`]) runs once for the whole session, and [`host`] talks to it:
//! the space queues what happens to it as events, and carries out what the layout answers when the
//! host passes it on: it moves and resizes windows with animations, moves the camera, renders,
//! and handles input.

use std::iter::zip;
use std::rc::Rc;
use std::time::Duration;

use niri_config::utils::MergeWith as _;
use niri_config::Struts;
use niri_ipc::WindowLayout;
use serde_json::{json, Value};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Point, Rectangle, Scale, Serial, Size};

use self::protocol::{
    CameraChange, ConfigInfo, Mode, Rect, Struts as StrutsInfo, ViewInfo, Wh, WindowChange,
    WindowInfo, WorkspaceInfo, Xy,
};
use self::tree::Tree;
use super::closing_window::{ClosingWindow, ClosingWindowRenderElement};
use super::monitor::{InsertPosition, TiledDrop};
use super::tile::{PendingMove, Tile, TileRenderElement, TileRenderSnapshot};
use super::workspace::InteractiveResize;
use super::{ConfigureIntent, HitType, InteractiveResizeData, LayoutElement, Options, RemovedTile};
use crate::animation::{Animation, Clock};
use crate::input::swipe_tracker::SwipeTracker;
use crate::layout::{RenderLayer, SizingMode};
use crate::niri_render_elements;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::xray::XrayPos;
use crate::render_helpers::RenderCtx;
use crate::utils::transaction::{Transaction, TransactionBlocker};
use crate::utils::ResizeEdge;
use crate::window::ResolvedWindowRules;

pub mod engine;
pub mod host;
mod protocol;
pub mod tree;

/// Amount of touchpad movement to scroll the view for the width of one working area.
const VIEW_GESTURE_WORKING_AREA_MOVEMENT: f64 = 1200.;

/// Space where the tiling layout places windows.
#[derive(Debug)]
pub struct TilingSpace<W: LayoutElement> {
    /// Tiles in top-to-bottom order.
    tiles: Vec<Tile<W>>,

    /// Extra per-tile data.
    data: Vec<TileData>,

    /// Id of the focused tile.
    active: Option<u64>,

    /// Where the view is on the canvas.
    camera: Camera,

    /// What the layout draws: the windows and everything else.
    tree: Tree,

    /// Horizontal swipe that the layout follows.
    swipe: Option<Swipe>,

    /// Where a window that's being dropped goes, for when it's added.
    drop: Option<(W::Id, TiledDrop)>,

    /// Why a window that's about to be removed is leaving, if it isn't closing.
    remove_reason: Option<(W::Id, &'static str)>,

    /// Ongoing interactive resize.
    interactive_resize: Option<InteractiveResize<W>>,

    /// Windows in the closing animation.
    closing_windows: Vec<ClosingWindow>,

    /// Id of the workspace this space is for, which the layout engine knows it by.
    id: u64,

    /// View size for this space.
    view_size: Size<f64, Logical>,

    /// Working area for this space, including struts.
    working_area: Rectangle<f64, Logical>,

    /// Working area for this space excluding struts.
    parent_area: Rectangle<f64, Logical>,

    /// Scale of the output the space is on (and rounds its sizes to).
    scale: f64,

    /// Clock for driving animations.
    clock: Clock,

    /// Configurable properties of the layout.
    options: Rc<Options>,
}

niri_render_elements! {
    TilingSpaceRenderElement<R> => {
        Tile = TileRenderElement<R>,
        ClosingWindow = ClosingWindowRenderElement,
    }
}

/// Extra per-tile data.
#[derive(Debug, Clone, PartialEq)]
struct TileData {
    /// Id of the window for the layout.
    id: u64,
    /// Where the layout put the tile on the canvas.
    rect: Option<Rectangle<f64, Logical>>,
    /// Whether the layout shows the tile.
    visible: bool,
    /// Sizing mode the layout gave the tile.
    mode: Mode,
    /// Whether the tile was maximized when it went fullscreen, so that leaving fullscreen goes
    /// back to maximized.
    maximized_under_fullscreen: bool,
}

/// Position of the view on the canvas.
#[derive(Debug)]
struct Camera {
    x: Axis,
    y: Axis,
    /// Drag-and-drop scrolling moving the camera horizontally.
    gesture: Option<CameraGesture>,
}

#[derive(Debug)]
enum Axis {
    Static(f64),
    Animation(Animation),
}

#[derive(Debug)]
struct CameraGesture {
    tracker: SwipeTracker,
    /// Camera x that the tracker position is relative to.
    delta_from_tracker: f64,
    /// Current camera x.
    current: f64,
    /// Time of the last event.
    dnd_last_event_time: Option<Duration>,
    /// Time when the scroll started being non-zero.
    dnd_nonzero_start_time: Option<Duration>,
}

/// A swipe along the workspace that the layout follows, which moves nothing by itself.
#[derive(Debug)]
struct Swipe {
    tracker: SwipeTracker,
    is_touchpad: bool,
}

/// How to carry out a layout's changes.
#[derive(Debug, Default, Clone)]
struct Apply {
    /// Transaction for resizes, so that windows resize together.
    transaction: Option<Transaction>,
    /// Animation for window movement, instead of the configured one.
    anim: Option<niri_config::Animation>,
    /// Initial velocity for a camera animation, after a swipe.
    camera_velocity: f64,
}

impl Axis {
    fn current(&self) -> f64 {
        match self {
            Axis::Static(value) => *value,
            Axis::Animation(anim) => anim.value(),
        }
    }

    fn target(&self) -> f64 {
        match self {
            Axis::Static(value) => *value,
            Axis::Animation(anim) => anim.to(),
        }
    }

    fn advance(&mut self) {
        if let Axis::Animation(anim) = self {
            if anim.is_done() {
                *self = Axis::Static(anim.to());
            }
        }
    }

    fn move_to(&mut self, to: f64, animate: Option<(Clock, niri_config::Animation, f64)>) {
        let from = self.current();
        match animate {
            Some((clock, config, velocity)) if from != to => {
                *self = Axis::Animation(Animation::new(clock, from, to, velocity, config));
            }
            _ => *self = Axis::Static(to),
        }
    }
}

impl Camera {
    fn current(&self) -> Point<f64, Logical> {
        let x = match &self.gesture {
            Some(gesture) => gesture.current,
            None => self.x.current(),
        };
        Point::from((x, self.y.current()))
    }

    fn target(&self) -> Point<f64, Logical> {
        let x = match &self.gesture {
            Some(gesture) => gesture.current,
            None => self.x.target(),
        };
        Point::from((x, self.y.target()))
    }

    fn is_static(&self) -> bool {
        self.gesture.is_none()
            && matches!(self.x, Axis::Static(_))
            && matches!(self.y, Axis::Static(_))
    }
}

impl<W: LayoutElement> TilingSpace<W> {
    pub fn new(
        id: u64,
        view_size: Size<f64, Logical>,
        parent_area: Rectangle<f64, Logical>,
        scale: f64,
        clock: Clock,
        options: Rc<Options>,
    ) -> Self {
        let working_area = compute_working_area(parent_area, scale, options.layout.struts);

        Self {
            tiles: Vec::new(),
            data: Vec::new(),
            active: None,
            camera: Camera {
                x: Axis::Static(0.),
                y: Axis::Static(0.),
                gesture: None,
            },
            tree: Tree::new(id),
            swipe: None,
            drop: None,
            remove_reason: None,
            interactive_resize: None,
            closing_windows: Vec::new(),
            id,
            view_size,
            working_area,
            parent_area,
            scale,
            clock,
            options,
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

        for tile in &mut self.tiles {
            tile.update_config(view_size, scale, options.clone());
        }

        self.view_size = view_size;
        self.working_area = working_area;
        self.parent_area = parent_area;
        self.scale = scale;
        self.options = options;
    }

    pub fn update_shaders(&mut self) {
        for tile in &mut self.tiles {
            tile.update_shaders();
        }
    }

    pub fn advance_animations(&mut self) {
        self.camera.x.advance();
        self.camera.y.advance();

        for tile in &mut self.tiles {
            tile.advance_animations();
        }

        self.closing_windows.retain_mut(|closing| {
            closing.advance_animations();
            closing.are_animations_ongoing()
        });
    }

    pub fn are_animations_ongoing(&self) -> bool {
        !self.camera.is_static()
            || self.tiles.iter().any(Tile::are_animations_ongoing)
            || !self.closing_windows.is_empty()
    }

    pub fn are_transitions_ongoing(&self) -> bool {
        !self.camera.is_static()
            || self.tiles.iter().any(Tile::are_transitions_ongoing)
            || !self.closing_windows.is_empty()
    }

    pub fn update_render_elements(&mut self, is_active: bool, layer: RenderLayer) {
        let view_rect = Rectangle::from_size(self.view_size);
        let active = self.active;
        let camera = self.camera.current();
        for (tile, data) in zip(&mut self.tiles, &self.data) {
            // Skip tiles belonging to a different render layer.
            if layer.is_normal() == tile.is_moving_between_workspaces() {
                continue;
            }

            let is_active = is_active && Some(data.id) == active;
            let pos = data.rect.unwrap_or_default().loc - camera;

            let mut tile_view_rect = view_rect;
            tile_view_rect.loc -= pos + tile.render_offset();
            tile.update_render_elements(is_active, tile_view_rect);
        }
    }

    pub fn tiles(&self) -> impl Iterator<Item = &Tile<W>> + '_ {
        self.tiles.iter()
    }

    pub fn tiles_mut(&mut self) -> impl Iterator<Item = &mut Tile<W>> + '_ {
        self.tiles.iter_mut()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    fn idx_of(&self, id: &W::Id) -> Option<usize> {
        self.tiles.iter().position(|tile| tile.window().id() == id)
    }

    fn idx_of_layout_id(&self, id: u64) -> Option<usize> {
        self.data.iter().position(|data| data.id == id)
    }

    fn active_idx(&self) -> Option<usize> {
        self.idx_of_layout_id(self.active?)
    }

    fn layout_id(&self, id: &W::Id) -> Option<u64> {
        Some(self.data[self.idx_of(id)?].id)
    }

    pub fn has_window(&self, id: &W::Id) -> bool {
        self.idx_of(id).is_some()
    }

    pub fn active_window(&self) -> Option<&W> {
        self.active_idx().map(|idx| self.tiles[idx].window())
    }

    pub fn active_window_mut(&mut self) -> Option<&mut W> {
        let idx = self.active_idx()?;
        Some(self.tiles[idx].window_mut())
    }

    pub fn active_tile_mut(&mut self) -> Option<&mut Tile<W>> {
        let idx = self.active_idx()?;
        Some(&mut self.tiles[idx])
    }

    pub fn is_active_pending_fullscreen(&self) -> bool {
        self.active_window()
            .is_some_and(|win| win.pending_sizing_mode().is_fullscreen())
    }

    /// Position of a tile relative to the view, not counting its own animations.
    fn tile_view_pos(&self, idx: usize) -> Point<f64, Logical> {
        self.data[idx].rect.unwrap_or_default().loc - self.camera.current()
    }

    pub fn tiles_with_render_positions(
        &self,
    ) -> impl Iterator<Item = (&Tile<W>, Point<f64, Logical>, bool)> {
        let scale = self.scale;
        let camera = self.camera.current();
        zip(&self.tiles, &self.data).map(move |(tile, data)| {
            let pos = data.rect.unwrap_or_default().loc - camera + tile.render_offset();
            let pos = pos.to_physical_precise_round(scale).to_logical(scale);
            (tile, pos, data.visible)
        })
    }

    pub fn tiles_with_render_positions_mut(
        &mut self,
        round: bool,
    ) -> impl Iterator<Item = (&mut Tile<W>, Point<f64, Logical>)> {
        let scale = self.scale;
        let camera = self.camera.current();
        zip(&mut self.tiles, &self.data).map(move |(tile, data)| {
            let mut pos = data.rect.unwrap_or_default().loc - camera + tile.render_offset();
            if round {
                pos = pos.to_physical_precise_round(scale).to_logical(scale);
            }
            (tile, pos)
        })
    }

    pub fn tiles_with_ipc_layouts(&self) -> impl Iterator<Item = (&Tile<W>, WindowLayout)> {
        let scale = self.scale;
        // Use the target camera position, to not spam IPC clients during animations.
        let camera = self.camera.target();
        zip(&self.tiles, &self.data).map(move |(tile, data)| {
            let pos = data.rect.unwrap_or_default().loc - camera;
            let pos = pos.to_physical_precise_round(scale).to_logical(scale);
            let layout = WindowLayout {
                tile_pos_in_workspace_view: Some(pos.into()),
                ..tile.ipc_layout_template()
            };
            (tile, layout)
        })
    }

    pub fn new_window_toplevel_bounds(&self, rules: &ResolvedWindowRules) -> Size<i32, Logical> {
        let border = self.options.layout.border.merged_with(&rules.border);
        toplevel_bounds(border, self.working_area.size, self.options.layout.gaps)
    }

    /// Size to suggest to a new window before it opens, from the layout.
    ///
    /// This asks the layout as it is: what was queued since hasn't reached it.
    pub fn new_window_size(&self, rules: &ResolvedWindowRules) -> Size<i32, Logical> {
        let border = self.options.layout.border.merged_with(&rules.border);
        let border = if border.off { 0. } else { border.width * 2. };

        let event = json!({
            "type": "newwindowsize",
            "workspace": self.id,
            "rules": engine::rules_json(&rules.engine),
            "minSize": Wh { w: 0., h: 0. },
            "maxSize": Wh { w: 0., h: 0. },
            "borders": Wh { w: border, h: border },
        });
        let size = host::query(event)
            .and_then(|size| serde_json::from_value::<Wh>(size).ok())
            .unwrap_or(Wh { w: 0., h: 0. });

        // The layout gives a tile size; the window doesn't include the borders.
        let window = |tile: f64| {
            if tile <= 0. {
                0
            } else {
                f64::max(1., tile - border).floor().min(100000.) as i32
            }
        };
        Size::from((window(size.w), window(size.h)))
    }

    /// Returns the geometry of the active window relative to and clamped to the working area.
    pub fn active_window_visual_rectangle(&self) -> Option<Rectangle<f64, Logical>> {
        let idx = self.active_idx()?;
        let tile = &self.tiles[idx];
        let window_pos = self.tile_view_pos(idx) + tile.window_loc();
        let window_rect = Rectangle::new(window_pos, tile.window_size());
        self.working_area.intersection(window_rect)
    }

    pub fn popup_target_rect(
        &self,
        id: &W::Id,
        _overflow: bool,
    ) -> Option<Rectangle<f64, Logical>> {
        let idx = self.idx_of(id)?;
        let mut target = self.parent_area;
        target.loc -= self.tile_view_pos(idx);
        target.loc -= self.tiles[idx].window_loc();
        Some(target)
    }

    /// Id of the workspace, which the layout engine knows it by.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// What the layout draws on the workspace.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    pub(super) fn tree_mut(&mut self) -> &mut Tree {
        &mut self.tree
    }

    /// Id of a window for the layout engine.
    pub fn engine_id_of(&self, id: &W::Id) -> Option<u64> {
        self.layout_id(id)
    }

    /// Id of the window that the layout engine knows by `engine_id`.
    pub fn window_id_of(&self, engine_id: u64) -> Option<&W::Id> {
        let idx = self.idx_of_layout_id(engine_id)?;
        Some(self.tiles[idx].window().id())
    }

    /// Engine id of the focused window.
    pub fn active_engine_id(&self) -> Option<u64> {
        self.active
    }

    /// Where the camera is going, if it's moving, or where it is.
    pub fn camera_target(&self) -> Point<f64, Logical> {
        self.camera.target()
    }

    /// What bento tells the layout about the workspace.
    pub(super) fn workspace_info(
        &self,
        name: Option<&String>,
        monitor: Option<String>,
        index: usize,
    ) -> WorkspaceInfo {
        let Struts {
            left,
            right,
            top,
            bottom,
        } = self.options.layout.struts;
        WorkspaceInfo {
            id: self.id,
            name: name.cloned(),
            monitor,
            index,
            view: ViewInfo {
                size: Wh::from_size(self.view_size),
                working_area: Rect::from_rect(self.parent_area),
                scale: self.scale,
            },
            camera: Xy::from_point(self.camera.target()),
            config: ConfigInfo {
                gaps: self.options.layout.gaps,
                struts: StrutsInfo {
                    left: left.0,
                    right: right.0,
                    top: top.0,
                    bottom: bottom.0,
                },
            },
            focused: self.active,
        }
    }

    /// Info about a window for the layout, on workspace `workspace`.
    pub(super) fn window_info_of(tile: &Tile<W>, workspace: u64, layout_name: &str) -> WindowInfo {
        let win = tile.window();
        let borders = tile.tile_width_for_window_width(0.);
        let data = match &tile.layout_data {
            Some((name, data)) if name == layout_name => data.clone(),
            _ => Value::Null,
        };

        WindowInfo {
            id: tile.engine_id,
            app_id: win.app_id(),
            title: win.title(),
            workspace,
            size: Wh::from_size(tile.tile_size()),
            min_size: Wh::from_size(tile.min_size_nonfullscreen()),
            max_size: Wh::from_size(tile.max_size_nonfullscreen()),
            borders: Wh {
                w: borders,
                h: tile.tile_height_for_window_height(0.),
            },
            mode: mode_of(win.pending_sizing_mode()),
            urgent: win.is_urgent(),
            rules: engine::rules_json(&win.rules().engine),
            data,
            rect: None,
        }
    }

    /// Info about each window for the layout, with where it is, from the bottom to the top.
    pub(super) fn window_infos(&self, layout_name: &str) -> Vec<WindowInfo> {
        zip(&self.tiles, &self.data)
            .rev()
            .map(|(tile, data)| WindowInfo {
                rect: data.rect.map(Rect::from_rect),
                ..Self::window_info_of(tile, self.id, layout_name)
            })
            .collect()
    }

    /// Starts the tree over for a new layout: only the windows are in it.
    pub(super) fn reset_tree(&mut self) {
        let windows = self.data.iter().rev().map(|data| data.id);
        self.tree.reset(windows);
    }

    /// Carries out what a layout call changed here: where windows go and where the camera points.
    pub(super) fn apply(
        &mut self,
        changes: Vec<WindowChange>,
        camera: Option<&CameraChange>,
        anim: Option<niri_config::Animation>,
        transaction: Option<Transaction>,
    ) {
        // Like in niri, windows that change size together resize in one transaction, so that they
        // show their new sizes in the same frame, and what moves because of a resize, other
        // windows and the view, animates with `window-resize`, in sync with it.
        let resizing = changes.iter().any(|change| self.resizes(change));
        let apply = if resizing {
            Apply {
                transaction: Some(transaction.unwrap_or_else(Transaction::new)),
                anim: Some(anim.unwrap_or(self.options.animations.window_resize.anim)),
                camera_velocity: camera.map_or(0., |camera| camera.velocity),
            }
        } else {
            Apply {
                transaction,
                anim,
                camera_velocity: camera.map_or(0., |camera| camera.velocity),
            }
        };

        for change in changes {
            let Some(idx) = self.idx_of_layout_id(change.id) else {
                continue;
            };
            self.data[idx].visible = change.visible;
            let rect = change.rect.map(Rect::to_rect).or(self.data[idx].rect);
            if let Some(rect) = rect {
                self.place(idx, rect, change.mode, change.animate, &apply);
            }
        }

        if let Some(camera) = camera {
            self.move_camera(camera, resizing, &apply);
        }
    }

    /// Moves the camera as the layout says, unless drag-and-drop scrolling is moving it.
    fn move_camera(&mut self, camera: &CameraChange, resizing: bool, apply: &Apply) {
        if self.camera.gesture.is_some() {
            return;
        }

        let animate = |config: niri_config::Animation, velocity: f64| {
            camera
                .animate
                .then(|| (self.clock.clone(), config, velocity))
        };
        let config = if resizing {
            self.options.animations.window_resize.anim
        } else {
            self.options.animations.horizontal_view_movement.0
        };
        let to = |v: f64| v.clamp(-1e7, 1e7);
        let x = animate(config, apply.camera_velocity);
        let y = animate(config, 0.);
        self.camera.x.move_to(to(camera.x), x);
        self.camera.y.move_to(to(camera.y), y);
    }

    /// Focuses a window, as the layout says.
    pub(super) fn focus(&mut self, id: Option<u64>) {
        self.active = id.filter(|id| self.idx_of_layout_id(*id).is_some());
    }

    /// Keeps what the layout stored on a window.
    pub(super) fn set_window_data(&mut self, id: u64, value: Value, layout_name: &str) {
        if let Some(idx) = self.idx_of_layout_id(id) {
            self.tiles[idx].layout_data = Some((layout_name.to_owned(), value));
        }
    }

    /// Stacks the windows as the tree does: the first in the tree is at the bottom.
    pub(super) fn stack_by_tree(&mut self) {
        // Tiles are top to bottom.
        let mut order: Vec<usize> = self
            .tree
            .window_order()
            .iter()
            .rev()
            .filter_map(|id| self.idx_of_layout_id(*id))
            .collect();
        order.dedup();
        for idx in 0..self.tiles.len() {
            if !order.contains(&idx) {
                order.push(idx);
            }
        }

        let mut tiles: Vec<_> = self.tiles.drain(..).map(Some).collect();
        let mut data: Vec<_> = self.data.drain(..).map(Some).collect();
        for idx in order {
            self.tiles.push(tiles[idx].take().unwrap());
            self.data.push(data[idx].take().unwrap());
        }
    }

    /// Whether a change resizes a window that the layout already placed, or changes its mode.
    fn resizes(&self, change: &WindowChange) -> bool {
        self.idx_of_layout_id(change.id).is_some_and(|idx| {
            let data = &self.data[idx];
            data.rect.is_some_and(|prev| {
                change
                    .rect
                    .is_some_and(|rect| prev.size != rect.to_rect().size)
                    || data.mode != change.mode
            })
        })
    }

    /// Puts a tile at `rect` on the canvas and asks it to take that size.
    fn place(
        &mut self,
        idx: usize,
        rect: Rectangle<f64, Logical>,
        mode: Mode,
        animate: bool,
        apply: &Apply,
    ) {
        let camera = self.camera.current();
        let tile = &mut self.tiles[idx];
        let data = &mut self.data[idx];

        if let Some(prev) = data.rect {
            if animate && prev.loc != rect.loc {
                let config = apply
                    .anim
                    .unwrap_or(self.options.animations.window_movement.0);
                tile.animate_move_from_with_config(prev.loc - rect.loc, config);
            }
        }

        let resized = data.rect.is_none_or(|prev| prev.size != rect.size) || data.mode != mode;
        data.rect = Some(rect);
        if mode == Mode::Fullscreen {
            if data.mode != Mode::Fullscreen {
                data.maximized_under_fullscreen = data.mode == Mode::Maximized;
            }
        } else {
            data.maximized_under_fullscreen = false;
        }
        data.mode = mode;

        // A tile that was moved here animates from where it was now that it knows where it is.
        if let Some(moved) = tile.pending_move.take() {
            let pos = rect.loc - camera + tile.render_offset();
            let config = moved
                .config
                .unwrap_or(self.options.animations.window_movement.0);
            tile.animate_move_from_with_config(moved.from - pos, config);
            if moved.between_workspaces {
                tile.set_anim_y_between_workspaces();
            }
        }

        if resized {
            // Like inactive tabs in niri, hidden windows don't hold up the others.
            let transaction = apply.transaction.clone().filter(|_| data.visible);
            match mode {
                Mode::Normal => tile.request_tile_size(rect.size, animate, transaction),
                Mode::Fullscreen => tile.request_fullscreen(animate, transaction),
                Mode::Maximized => tile.request_maximized(rect.size, animate, transaction),
            }
        }
    }

    /// Puts tiles that the layout didn't place in the middle of the view, and focuses a window if
    /// none has focus.
    pub(super) fn place_unplaced(&mut self) {
        for idx in 0..self.tiles.len() {
            if self.data[idx].rect.is_some() {
                continue;
            }

            let area = self.working_area;
            let size = self.tiles[idx].tile_size();
            let w = if size.w > 0. {
                size.w
            } else {
                area.size.w / 2.
            }
            .min(area.size.w);
            let h = if size.h > 0. {
                size.h
            } else {
                area.size.h / 2.
            }
            .min(area.size.h);
            let loc = self.camera.target()
                + area.loc
                + Point::from(((area.size.w - w) / 2., (area.size.h - h) / 2.));
            let rect = Rectangle::new(loc, Size::from((w, h)));
            self.place(idx, rect, Mode::Normal, false, &Apply::default());
        }

        if self
            .active
            .is_none_or(|id| self.idx_of_layout_id(id).is_none())
        {
            self.active = self.data.first().map(|data| data.id);
        }
    }

    /// Adds a tile, with the layout deciding where it goes.
    pub fn add_tile(
        &mut self,
        tile: Tile<W>,
        activate: bool,
        anim: Option<niri_config::Animation>,
    ) {
        self.add_tile_with(tile, activate, anim, None, None);
    }

    /// Adds a tile next to the tile of `next_to`, like a dialog next to its parent.
    pub fn add_tile_right_of(
        &mut self,
        next_to: &W::Id,
        tile: Tile<W>,
        activate: bool,
        anim: Option<niri_config::Animation>,
    ) {
        let next_to = self.layout_id(next_to);
        self.add_tile_with(tile, activate, anim, next_to, None);
    }

    /// Adds a tile that stopped floating, last seen at `rect` relative to the view.
    pub fn add_tile_from_floating(
        &mut self,
        tile: Tile<W>,
        activate: bool,
        rect: Rectangle<f64, Logical>,
    ) {
        let rect = Rectangle::new(rect.loc + self.camera.target(), rect.size);
        self.add_tile_with(tile, activate, None, None, Some(("tile", rect)));
    }

    fn add_tile_with(
        &mut self,
        mut tile: Tile<W>,
        activate: bool,
        anim: Option<niri_config::Animation>,
        next_to: Option<u64>,
        from: Option<(&str, Rectangle<f64, Logical>)>,
    ) {
        tile.update_config(self.view_size, self.scale, self.options.clone());

        let drop = self
            .drop
            .take()
            .filter(|(id, _)| id == tile.window().id())
            .map(|(_, drop)| drop);

        let reason = if drop.is_some() {
            "drop"
        } else if let Some((reason, _)) = from {
            reason
        } else {
            "open"
        };

        let moved_from = tile.engine_workspace.filter(|from| *from != self.id);
        let known = tile.engine_workspace == Some(self.id);
        tile.engine_workspace = Some(self.id);

        let id = tile.engine_id;
        match tile.tree_node.take() {
            Some(node) => self.tree.put_window(node),
            None => self.tree.add_window(id),
        }
        self.tiles.insert(0, tile);
        self.data.insert(
            0,
            TileData {
                id,
                rect: None,
                visible: true,
                mode: Mode::Normal,
                maximized_under_fullscreen: false,
            },
        );

        let activate = activate || self.tiles.len() == 1;
        if let Some(from) = moved_from {
            host::queue_with(
                json!({
                    "type": "windowmove",
                    "id": id,
                    "from": from,
                    "to": self.id,
                    "activate": activate,
                }),
                anim,
            );
        } else if !known {
            let (rect, pointer) = match (&drop, from) {
                (Some(drop), _) => (Some(drop.rect), Some(drop.pointer)),
                (None, Some((_, rect))) => (Some(rect), None),
                (None, None) => (None, None),
            };
            let window = Self::window_info_of(&self.tiles[0], self.id, &engine::name());
            host::queue_with(
                json!({
                    "type": "windowadd",
                    "window": window,
                    "how": {
                        "reason": reason,
                        "activate": activate,
                        "rect": rect.map(Rect::from_rect),
                        "pointer": pointer.map(Xy::from_point),
                        "nextTo": next_to,
                    },
                }),
                anim,
            );
        }
    }

    pub fn remove_tile(&mut self, id: &W::Id, transaction: Transaction) -> RemovedTile<W> {
        let idx = self.idx_of(id).unwrap();
        let reason = self
            .remove_reason
            .take()
            .filter(|(window, _)| window == id)
            .map_or("close", |(_, reason)| reason);
        self.remove_tile_by_idx(idx, reason, Some(transaction))
    }

    /// Makes the next removal of `id` say it's for `reason` rather than closing: `"drag"` or
    /// `"move"`.
    pub fn set_remove_reason(&mut self, id: &W::Id, reason: &'static str) {
        self.remove_reason = Some((id.clone(), reason));
    }

    /// Removes a tile because it starts floating.
    pub fn remove_tile_for_floating(&mut self, id: &W::Id) -> RemovedTile<W> {
        let idx = self.idx_of(id).unwrap();
        self.remove_tile_by_idx(idx, "float", None)
    }

    fn remove_tile_by_idx(
        &mut self,
        idx: usize,
        reason: &str,
        transaction: Option<Transaction>,
    ) -> RemovedTile<W> {
        let mut tile = self.tiles.remove(idx);
        let data = self.data.remove(idx);

        if self
            .interactive_resize
            .as_ref()
            .is_some_and(|resize| tile.window().id() == &resize.window)
        {
            self.interactive_resize = None;
        }
        tile.window_mut().cancel_interactive_resize();

        if self.active == Some(data.id) {
            self.active = None;
        }

        // A window that moves to another workspace stays in the layout, and its node goes along.
        if reason == "move" {
            tile.tree_node = self.tree.take_window(data.id);
        } else {
            self.tree.remove_window(data.id);
            tile.engine_workspace = None;
            host::queue_with_transaction(
                json!({ "type": "windowremove", "id": data.id, "reason": reason }),
                transaction,
            );
        }

        RemovedTile {
            tile,
            is_floating: false,
        }
    }

    pub fn start_close_animation_for_window(
        &mut self,
        renderer: &mut GlesRenderer,
        id: &W::Id,
        blocker: TransactionBlocker,
    ) {
        let Some(idx) = self.idx_of(id) else {
            return;
        };
        if !self.data[idx].visible {
            return;
        }

        let (tile, tile_pos) = self
            .tiles_with_render_positions_mut(false)
            .nth(idx)
            .unwrap();
        let Some(snapshot) = tile.take_unmap_snapshot() else {
            return;
        };
        let tile_size = tile.tile_size();

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

        // Closing windows live in view coordinates, so they stay put as the camera moves.
        let scale = Scale::from(self.scale);
        let res = ClosingWindow::new(
            renderer, snapshot, scale, tile_size, tile_pos, blocker, anim,
        );
        match res {
            Ok(closing) => self.closing_windows.push(closing),
            Err(err) => warn!("error creating a closing window animation: {err:?}"),
        }
    }

    pub fn start_open_animation(&mut self, id: &W::Id) -> bool {
        let Some(idx) = self.idx_of(id) else {
            return false;
        };
        self.tiles[idx].start_open_animation();
        true
    }

    pub fn update_window(&mut self, id: &W::Id, serial: Option<Serial>) {
        let Some(idx) = self.idx_of(id) else {
            return;
        };

        let tile = &mut self.tiles[idx];
        // Do this before calling update_window() so it can get up-to-date info.
        if let Some(serial) = serial {
            tile.window_mut().on_commit(serial);
        }
        tile.update_window();
    }

    /// How far the view would scroll to focus the window, in working area widths.
    pub fn scroll_amount_to_activate(&self, id: &W::Id) -> f64 {
        let Some(idx) = self.idx_of(id) else {
            return 0.;
        };
        if self.active == Some(self.data[idx].id) {
            return 0.;
        }

        let rect = self.data[idx].rect.unwrap_or_default();
        let left = rect.loc.x - self.camera.target().x;
        let right = left + rect.size.w;
        let area = self.working_area;
        let amount = if left < area.loc.x {
            area.loc.x - left
        } else if area.loc.x + area.size.w < right {
            right - (area.loc.x + area.size.w)
        } else {
            0.
        };
        amount / area.size.w.max(1.)
    }

    /// Focuses a window, which is what bento does when the layout lets it.
    pub fn activate_window(&mut self, id: &W::Id) -> bool {
        let Some(idx) = self.idx_of(id) else {
            return false;
        };
        self.active = Some(self.data[idx].id);
        true
    }

    /// Whether a window is maximized, or fullscreen with maximized to go back to.
    pub fn is_pending_maximized(&self, id: &W::Id) -> bool {
        self.layout_id(id)
            .and_then(|layout_id| self.idx_of_layout_id(layout_id))
            .is_some_and(|idx| {
                let data = &self.data[idx];
                data.mode == Mode::Maximized || data.maximized_under_fullscreen
            })
    }

    pub fn set_fullscreen(&mut self, id: &W::Id, is_fullscreen: bool) -> bool {
        let mode = if is_fullscreen {
            "fullscreen"
        } else {
            "normal"
        };
        self.request_mode(id, mode, json!({ "fullscreen": is_fullscreen }))
    }

    pub fn set_maximized(&mut self, id: &W::Id, maximize: bool) -> bool {
        let mode = if maximize { "maximized" } else { "normal" };
        let changed = self.request_mode(id, mode, json!({ "maximized": maximize }));

        // A fullscreen window stays fullscreen, but leaving it goes to what was asked for since.
        if let Some(idx) = self.layout_id(id).and_then(|id| self.idx_of_layout_id(id)) {
            let data = &mut self.data[idx];
            if data.mode == Mode::Fullscreen {
                data.maximized_under_fullscreen = maximize;
            }
        }
        changed
    }

    fn request_mode(&mut self, id: &W::Id, mode: &str, request: Value) -> bool {
        let Some(layout_id) = self.layout_id(id) else {
            return false;
        };
        host::queue(json!({
            "type": "moderequest",
            "id": layout_id,
            "mode": mode,
            "request": request,
        }));
        true
    }

    pub fn fullscreen_window_above_top_layer(&self) -> Option<&W> {
        // Render above the top layer if we're on a fullscreen window and the view is stationary.
        if !self.camera.is_static() {
            return None;
        }

        self.active_window()
            .filter(|win| win.sizing_mode().is_fullscreen())
    }

    pub fn render<R: NiriRenderer>(
        &self,
        mut ctx: RenderCtx<R>,
        xray_pos: XrayPos,
        focus_ring: bool,
        layer: RenderLayer,
        push: &mut dyn FnMut(TilingSpaceRenderElement<R>),
    ) {
        let scale = Scale::from(self.scale);

        // Draw the closing windows on top of the other windows.
        if layer.is_normal() {
            let view_rect = Rectangle::from_size(self.view_size);
            for closing in self.closing_windows.iter().rev() {
                let elem = closing.render(ctx.as_gles(), view_rect, scale);
                push(elem.into());
            }
        }

        let active = self.active;
        for ((tile, tile_pos, visible), data) in self.tiles_with_render_positions().zip(&self.data)
        {
            // Skip tiles belonging to a different render layer.
            if layer.is_normal() == tile.is_moving_between_workspaces() {
                continue;
            }

            // Keep hidden tiles that are fading out visible until they're done.
            if !visible && tile.alpha_animation.is_none() {
                continue;
            }

            let focus_ring = focus_ring && Some(data.id) == active;
            let xray_pos = xray_pos.offset(tile_pos);
            tile.render(ctx.r(), tile_pos, xray_pos, focus_ring, &mut |elem| {
                push(elem.into())
            });
        }
    }

    pub fn window_under(&self, pos: Point<f64, Logical>) -> Option<(&W, HitType)> {
        self.tiles_with_render_positions()
            .filter(|(_, _, visible)| *visible)
            .find_map(|(tile, tile_pos, _)| HitType::hit_tile(tile, tile_pos, pos))
    }

    /// Where a dragged tile is on the canvas: the pointer at `pos` and the tile at `tile_pos`,
    /// both relative to the view.
    pub(super) fn drag_on_canvas(
        &self,
        pos: Point<f64, Logical>,
        tile: &Tile<W>,
        tile_pos: Point<f64, Logical>,
    ) -> TiledDrop {
        let camera = self.camera.current();
        TiledDrop {
            pointer: pos + camera,
            rect: Rectangle::new(tile_pos + camera, tile.tile_size()),
        }
    }

    /// Where a dragged tile goes if dropped with the pointer at `pos`, relative to the view.
    ///
    /// `tile_pos` is where the dragged tile is, relative to the view.
    pub(super) fn insert_position(
        &self,
        pos: Point<f64, Logical>,
        tile: &Tile<W>,
        tile_pos: Point<f64, Logical>,
    ) -> InsertPosition {
        InsertPosition::Tiled(Some(self.drag_on_canvas(pos, tile, tile_pos)))
    }

    /// Makes the window go where it was dropped when it's added next.
    pub(super) fn set_drop(&mut self, window: W::Id, drop: TiledDrop) {
        self.drop = Some((window, drop));
    }

    /// Sets how a tile that was just added animates to where the layout places it.
    pub(super) fn set_pending_move(&mut self, id: &W::Id, moved: PendingMove) {
        if let Some(idx) = self.idx_of(id) {
            self.tiles[idx].pending_move = Some(moved);
        }
    }

    /// Queues an event for the layout about a swipe.
    fn queue_swipe(&self, kind: &str, delta: f64, velocity: f64, touchpad: bool) {
        host::queue(json!({
            "type": kind,
            "workspace": self.id,
            "delta": delta,
            "velocity": velocity,
            "touchpad": touchpad,
        }));
    }

    pub fn view_offset_gesture_begin(&mut self, is_touchpad: bool) {
        if self.tiles.is_empty() || self.interactive_resize.is_some() {
            return;
        }

        self.swipe = Some(Swipe {
            tracker: SwipeTracker::new(),
            is_touchpad,
        });
        self.queue_swipe("swipebegin", 0., 0., is_touchpad);
    }

    pub fn view_offset_gesture_update(
        &mut self,
        delta_x: f64,
        timestamp: Duration,
        is_touchpad: bool,
    ) -> Option<bool> {
        let norm_factor = self.swipe_norm_factor(is_touchpad);
        let swipe = self.swipe.as_mut()?;
        if swipe.is_touchpad != is_touchpad {
            return None;
        }

        swipe.tracker.push(delta_x, timestamp);
        self.queue_swipe("swipeupdate", delta_x * norm_factor, 0., is_touchpad);

        Some(true)
    }

    pub fn view_offset_gesture_end(&mut self, is_touchpad: Option<bool>) -> bool {
        let Some(swipe) = &mut self.swipe else {
            return false;
        };
        if is_touchpad.is_some_and(|x| swipe.is_touchpad != x) {
            return false;
        }

        // Take into account any idle time between the last event and now.
        let now = self.clock.now_unadjusted();
        swipe.tracker.push(0., now);

        let is_touchpad = swipe.is_touchpad;
        let norm_factor = self.swipe_norm_factor(is_touchpad);
        let velocity = self.swipe.take().unwrap().tracker.velocity() * norm_factor;
        self.queue_swipe("swipeend", 0., velocity, is_touchpad);
        true
    }

    fn swipe_norm_factor(&self, is_touchpad: bool) -> f64 {
        if is_touchpad {
            self.working_area.size.w / VIEW_GESTURE_WORKING_AREA_MOVEMENT
        } else {
            1.
        }
    }

    pub fn dnd_scroll_gesture_begin(&mut self) {
        if self
            .camera
            .gesture
            .as_ref()
            .is_some_and(|gesture| gesture.dnd_last_event_time.is_some())
        {
            // Already active.
            return;
        }

        let current = self.camera.current().x;
        self.camera.gesture = Some(CameraGesture {
            tracker: SwipeTracker::new(),
            delta_from_tracker: current,
            current,
            dnd_last_event_time: Some(self.clock.now_unadjusted()),
            dnd_nonzero_start_time: None,
        });

        self.interactive_resize = None;
    }

    pub fn dnd_scroll_gesture_scroll(&mut self, delta: f64) -> bool {
        // Keep some of the windows in view.
        let (min_x, max_x) = self.camera_bounds();

        let Some(gesture) = &mut self.camera.gesture else {
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

        let x = gesture.tracker.pos() + gesture.delta_from_tracker;
        let clamped = x.clamp(min_x, max_x.max(min_x));
        gesture.delta_from_tracker += clamped - x;
        gesture.current = clamped;
        true
    }

    pub fn dnd_scroll_gesture_end(&mut self) {
        let Some(gesture) = &self.camera.gesture else {
            return;
        };
        if gesture.dnd_last_event_time.is_none() {
            return;
        }

        let current = gesture.current;
        self.camera.gesture = None;
        self.camera.x = Axis::Static(current);

        host::queue(json!({ "type": "camerachange", "workspace": self.id }));
    }

    /// Range of camera x that keeps some of the windows in view.
    fn camera_bounds(&self) -> (f64, f64) {
        let rects = || self.data.iter().filter_map(|data| data.rect);
        let left = rects().map(|r| r.loc.x).fold(f64::INFINITY, f64::min);
        let right = rects()
            .map(|r| r.loc.x + r.size.w)
            .fold(f64::NEG_INFINITY, f64::max);
        if !left.is_finite() || !right.is_finite() {
            let x = self.camera.target().x;
            return (x, x);
        }
        (
            left - self.working_area.size.w,
            right - self.working_area.loc.x,
        )
    }

    /// Starts an interactive resize of a window, before the layout is asked whether it's allowed.
    ///
    /// Returns `None` if the window can't be resized, or the engine id to ask the layout about.
    pub(super) fn interactive_resize_begin(
        &mut self,
        window: W::Id,
        edges: ResizeEdge,
    ) -> Option<u64> {
        if self.interactive_resize.is_some() {
            return None;
        }
        let idx = self.idx_of(&window)?;
        let id = self.data[idx].id;

        let original_window_size = self.tiles[idx].window_size();
        self.interactive_resize = Some(InteractiveResize {
            window,
            original_window_size,
            data: InteractiveResizeData { edges },
        });

        // Freeze the view while resizing.
        if let Some(gesture) = self.camera.gesture.take() {
            self.camera.x = Axis::Static(gesture.current);
        }
        self.swipe = None;

        Some(id)
    }

    /// Takes back an interactive resize that the layout refused.
    pub(super) fn interactive_resize_cancel(&mut self) {
        self.interactive_resize = None;
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
        let edges = resize.data.edges;
        let Some(id) = self.layout_id(window) else {
            return false;
        };

        host::queue(json!({
            "type": "resizeupdate",
            "id": id,
            "edges": edges_json(edges),
            "delta": Xy::from_point(delta),
        }));
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
        }

        let window = resize.window.clone();
        self.interactive_resize = None;
        if let Some(id) = self.layout_id(&window) {
            host::queue(json!({ "type": "resizeend", "id": id }));
        }
    }

    pub fn refresh(&mut self, is_active: bool, is_focused: bool) {
        let active = self.active;
        for (tile, data) in zip(&mut self.tiles, &self.data) {
            let win = tile.window_mut();

            win.set_floating(false);

            let mut is_active = is_active && Some(data.id) == active;
            if self.options.deactivate_unfocused_windows {
                is_active &= is_focused;
            }
            win.set_activated(is_active);

            let resize_data = self
                .interactive_resize
                .as_ref()
                .filter(|resize| &resize.window == win.id())
                .map(|resize| resize.data);
            win.set_interactive_resize(resize_data);

            let border = self.options.layout.border.merged_with(&win.rules().border);
            let gaps = self.options.layout.gaps;
            win.set_bounds(toplevel_bounds(border, self.working_area.size, gaps));

            // If transactions are disabled, also disable combined throttling, for more
            // intuitive behavior.
            let intent = if self.options.disable_resize_throttling {
                ConfigureIntent::CanSend
            } else {
                win.configure_intent()
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

    /// Queues an event for the layout about a window of the space.
    pub(super) fn queue_window_event(&self, window: &W::Id, event: &str, fields: Value) {
        let Some(id) = self.layout_id(window) else {
            return;
        };
        let mut event = json!({ "type": event, "id": id });
        if let (Some(event), Value::Object(fields)) = (event.as_object_mut(), fields) {
            event.extend(fields);
        }
        host::queue(event);
    }

    /// Whether a swipe or drag-and-drop gesture is moving the camera.
    pub fn is_camera_gesture(&self) -> bool {
        self.swipe.is_some() || self.camera.gesture.is_some()
    }

    /// Camera x on the canvas, like niri's view position.
    pub fn view_pos(&self) -> f64 {
        self.camera.current().x
    }

    pub fn view_size(&self) -> Size<f64, Logical> {
        self.view_size
    }

    pub fn parent_area(&self) -> Rectangle<f64, Logical> {
        self.parent_area
    }

    pub fn clock(&self) -> &Clock {
        &self.clock
    }

    pub fn options(&self) -> &Rc<Options> {
        &self.options
    }

    #[cfg(test)]
    pub fn verify_invariants(&self) {
        assert!(self.view_size.w > 0.);
        assert!(self.view_size.h > 0.);
        assert!(self.scale > 0.);
        assert!(self.scale.is_finite());
        assert_eq!(self.tiles.len(), self.data.len());
        assert_eq!(
            self.working_area,
            compute_working_area(self.parent_area, self.scale, self.options.layout.struts)
        );

        for (tile, data) in zip(&self.tiles, &self.data) {
            assert!(Rc::ptr_eq(&self.options, &tile.options));
            assert_eq!(self.clock, tile.clock);
            assert_eq!(self.scale, tile.scale());
            tile.verify_invariants();

            assert!(data.rect.is_some(), "every tile must have a place");
            assert_eq!(
                tile.engine_workspace,
                Some(self.id),
                "tiles in the tiling layout must be known to the layout engine"
            );
            assert_eq!(data.id, tile.engine_id);
        }

        let mut windows = self.tree.window_order();
        windows.sort_unstable();
        let mut tiles: Vec<_> = self.data.iter().map(|data| data.id).collect();
        tiles.sort_unstable();
        assert_eq!(windows, tiles, "every window must have a node in the tree");

        if let Some(id) = self.active {
            assert!(
                self.idx_of_layout_id(id).is_some(),
                "the focused window must be in the layout"
            );
        } else {
            assert!(self.tiles.is_empty(), "a window must have focus");
        }

        if let Some(resize) = &self.interactive_resize {
            assert!(
                self.has_window(&resize.window),
                "interactive resize window must be present in the layout"
            );
        }
    }
}

fn mode_of(mode: SizingMode) -> Mode {
    match mode {
        SizingMode::Normal => Mode::Normal,
        SizingMode::Fullscreen => Mode::Fullscreen,
        SizingMode::Maximized => Mode::Maximized,
    }
}

fn edges_json(edges: ResizeEdge) -> Value {
    json!({
        "left": edges.contains(ResizeEdge::LEFT),
        "right": edges.contains(ResizeEdge::RIGHT),
        "top": edges.contains(ResizeEdge::TOP),
        "bottom": edges.contains(ResizeEdge::BOTTOM),
    })
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

/// Largest size a tiled window can take, which windows get as their configure bounds.
fn toplevel_bounds(
    border: niri_config::Border,
    working_area: Size<f64, Logical>,
    gaps: f64,
) -> Size<i32, Logical> {
    let border = if border.off { 0. } else { border.width * 2. };
    Size::from((
        f64::max(working_area.w - gaps * 2. - border, 1.),
        f64::max(working_area.h - gaps * 2. - border, 1.),
    ))
    .to_i32_floor()
}
