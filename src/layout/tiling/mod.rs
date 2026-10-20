//! The tiling layout, where a layout script decides where windows go.
//!
//! This is the compositor half of bento's scriptable layouts. The space holds the tiled windows
//! of a workspace and a camera over an unbounded canvas. A layout instance in the JS engine (see
//! [`engine`]) decides everything else: where each window goes, which one has focus, where the
//! camera points, and what actions do. The space tells the layout about everything that happens
//! as events, and carries out what the layout answers: it moves and resizes windows with
//! animations, moves the camera, renders, and handles input.

use std::iter::zip;
use std::rc::Rc;
use std::time::Duration;

use anyhow::Context as _;
use niri_config::utils::MergeWith as _;
use niri_config::{CenterFocusedColumn, PresetSize, Struts};
use niri_ipc::{ColumnDisplay, SizeChange, WindowLayout};
use serde_json::{json, Value};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Point, Rectangle, Scale, Serial, Size};

use self::protocol::{Mode, Output, Preset, Rect, Rules, TransferOut, Wh, WindowInfo, Xy};
use super::closing_window::{ClosingWindow, ClosingWindowRenderElement};
use super::monitor::{InsertPosition, TiledDrop};
use super::tile::{Tile, TileRenderElement, TileRenderSnapshot};
use super::workspace::InteractiveResize;
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
use crate::utils::ResizeEdge;
use crate::window::ResolvedWindowRules;

mod actions;
pub mod engine;
mod protocol;

/// Amount of touchpad movement to scroll the view for the width of one working area.
const VIEW_GESTURE_WORKING_AREA_MOVEMENT: f64 = 1200.;

static TILE_ID_COUNTER: IdCounter = IdCounter::new();

/// Width of a column, as window rules and moves between workspaces carry it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColumnWidth {
    /// Proportion of the working area width.
    Proportion(f64),
    /// Fixed width in logical pixels, including borders.
    Fixed(f64),
}

impl From<PresetSize> for ColumnWidth {
    fn from(value: PresetSize) -> Self {
        match value {
            PresetSize::Proportion(prop) => Self::Proportion(prop),
            PresetSize::Fixed(width) => Self::Fixed(f64::from(width.clamp(1, 100000))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Left,
    Right,
}

/// Windows that move to another workspace together, as the layout decides.
#[derive(Debug)]
pub struct TileGroup<W: LayoutElement> {
    /// The tiles, with where they were on screen, relative to the workspace view.
    tiles: Vec<(Tile<W>, Point<f64, Logical>)>,
    /// What the layout wants to tell the layout on the other workspace about them.
    data: Value,
}

impl<W: LayoutElement> TileGroup<W> {
    pub fn tiles(&self) -> impl Iterator<Item = (&Tile<W>, Point<f64, Logical>)> {
        self.tiles.iter().map(|(tile, pos)| (tile, *pos))
    }
}

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

    /// Drop target highlight from the layout, on the canvas.
    hint: Option<Rectangle<f64, Logical>>,

    /// Where a window that's being dropped goes, for when it's added.
    drop: Option<(W::Id, TiledDrop)>,

    /// Why a window that's about to be removed is leaving, if it isn't closing.
    remove_reason: Option<(W::Id, &'static str)>,

    /// Ongoing interactive resize.
    interactive_resize: Option<InteractiveResize<W>>,

    /// Windows in the closing animation.
    closing_windows: Vec<ClosingWindow>,

    /// Id of the layout instance for this space in the engine.
    instance: u64,

    /// Engine generation the instance was created in, or 0 if it doesn't exist.
    generation: u64,

    /// Generation in which creating the instance failed, to not retry over and over.
    failed_generation: u64,

    /// View and config as last told to the layout.
    sent_view: Value,
    sent_config: Value,

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
    /// Column and row for IPC.
    ipc: Option<(usize, usize)>,
    /// Width the tile came with, as its default width for the layout.
    default_width: Option<Preset>,
    /// What the layout was last told about the window.
    reported: Option<WindowInfo>,
}

/// What tests can see of the camera.
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub(super) struct CameraState {
    is_animation_ongoing: bool,
}

#[cfg(test)]
impl CameraState {
    pub fn is_animation_ongoing(&self) -> bool {
        self.is_animation_ongoing
    }
}

/// Position of the view on the canvas.
#[derive(Debug)]
struct Camera {
    x: Axis,
    y: Axis,
    /// Swipe gesture moving the camera horizontally.
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
    is_touchpad: bool,
    /// For drag-and-drop scrolling, time of the last event.
    dnd_last_event_time: Option<Duration>,
    /// For drag-and-drop scrolling, time when the scroll started being non-zero.
    dnd_nonzero_start_time: Option<Duration>,
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
            hint: None,
            drop: None,
            remove_reason: None,
            interactive_resize: None,
            closing_windows: Vec::new(),
            instance: engine::next_instance(),
            generation: 0,
            failed_generation: 0,
            sent_view: Value::Null,
            sent_config: Value::Null,
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

        // A new layout or layout version gets created with the current view and config.
        if self.generation != engine::ensure_loaded(&self.options.tiling) {
            self.ensure_instance();
            return;
        }

        let view = self.view_json();
        if view != self.sent_view {
            self.sent_view = view.clone();
            self.dispatch(json!({ "type": "view", "view": view }), Apply::default());
        }

        let config = self.config_json();
        let options = engine::options_json(&self.options.tiling);
        let both = json!([config, options]);
        if both != self.sent_config {
            self.sent_config = both;
            self.dispatch(
                json!({ "type": "config", "config": config, "options": options }),
                Apply::default(),
            );
        }
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
                pos_in_scrolling_layout: data.ipc,
                tile_pos_in_workspace_view: Some(pos.into()),
                ..tile.ipc_layout_template()
            };
            (tile, layout)
        })
    }

    pub fn new_window_toplevel_bounds(&self, rules: &ResolvedWindowRules) -> Size<i32, Logical> {
        let border = self.options.layout.border.merged_with(&rules.border);
        let border = if border.off { 0. } else { border.width * 2. };
        let gaps = self.options.layout.gaps;
        Size::from((
            f64::max(self.working_area.size.w - gaps * 2. - border, 1.),
            f64::max(self.working_area.size.h - gaps * 2. - border, 1.),
        ))
        .to_i32_floor()
    }

    /// Size to suggest to a new window before it opens, from the layout.
    pub fn new_window_size(
        &self,
        width: Option<PresetSize>,
        height: Option<PresetSize>,
        rules: &ResolvedWindowRules,
    ) -> Size<i32, Logical> {
        let border = self.options.layout.border.merged_with(&rules.border);
        let border = if border.off { 0. } else { border.width * 2. };

        let info = json!({
            "rules": Rules {
                default_width: width.map(preset),
                default_height: height.map(preset),
            },
            "minSize": Wh { w: 0., h: 0. },
            "maxSize": Wh { w: 0., h: 0. },
            "borders": Wh { w: border, h: border },
        });
        let size = self
            .query(json!({ "type": "new-window-size", "info": info }))
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

    /// Info about a window for the layout.
    fn window_info(&self, idx: usize) -> WindowInfo {
        let tile = &self.tiles[idx];
        let data = &self.data[idx];
        let win = tile.window();

        let borders = tile.tile_width_for_window_width(0.);
        let default_height = win.rules().default_height.flatten().map(preset);

        WindowInfo {
            id: data.id,
            app_id: win.app_id(),
            title: win.title(),
            size: Wh::from_size(tile.tile_size()),
            min_size: Wh::from_size(tile.min_size_nonfullscreen()),
            max_size: Wh::from_size(tile.max_size_nonfullscreen()),
            borders: Wh {
                w: borders,
                h: tile.tile_height_for_window_height(0.),
            },
            mode: mode_of(win.pending_sizing_mode()),
            rules: Rules {
                default_width: data.default_width,
                default_height,
            },
            is_urgent: win.is_urgent(),
            rect: None,
        }
    }

    fn view_json(&self) -> Value {
        json!({
            "size": Wh::from_size(self.view_size),
            "workingArea": Rect::from_rect(self.parent_area),
            "scale": self.scale,
        })
    }

    fn config_json(&self) -> Value {
        let layout = &self.options.layout;
        let Struts {
            left,
            right,
            top,
            bottom,
        } = layout.struts;
        let center = match layout.center_focused_column {
            CenterFocusedColumn::Never => "never",
            CenterFocusedColumn::Always => "always",
            CenterFocusedColumn::OnOverflow => "on-overflow",
        };
        let display = match layout.default_column_display {
            ColumnDisplay::Normal => "normal",
            ColumnDisplay::Tabbed => "tabbed",
        };
        json!({
            "gaps": layout.gaps,
            "struts": { "left": left.0, "right": right.0, "top": top.0, "bottom": bottom.0 },
            "centerFocusedColumn": center,
            "alwaysCenterSingleColumn": layout.always_center_single_column,
            "defaultColumnWidth": layout.default_column_width.map(preset),
            "presetColumnWidths": layout.preset_column_widths.iter().copied().map(preset).collect::<Vec<_>>(),
            "presetWindowHeights": layout.preset_window_heights.iter().copied().map(preset).collect::<Vec<_>>(),
            "defaultColumnDisplay": display,
        })
    }

    /// Creates the layout instance if the engine doesn't have it, with the windows already here.
    fn ensure_instance(&mut self) {
        let generation = engine::ensure_loaded(&self.options.tiling);
        if self.generation == generation || self.failed_generation == generation {
            return;
        }
        self.generation = generation;

        let windows: Vec<WindowInfo> = (0..self.tiles.len())
            .rev()
            .map(|idx| WindowInfo {
                rect: self.data[idx].rect.map(Rect::from_rect),
                ..self.window_info(idx)
            })
            .collect();
        for (data, info) in zip(self.data.iter_mut().rev(), &windows) {
            data.reported = Some(WindowInfo {
                rect: None,
                ..info.clone()
            });
        }

        self.sent_view = self.view_json();
        let config = self.config_json();
        let options = engine::options_json(&self.options.tiling);
        self.sent_config = json!([config, options]);

        let camera = self.camera.target();
        let event = json!({
            "type": "create",
            "view": self.sent_view,
            "camera": Xy::from_point(camera),
            "config": config,
            "options": options,
            "windows": windows,
            "focused": self.active,
            "state": engine::take_state(self.instance),
        });
        if self.send(event, &Apply::default()).is_none() {
            self.failed_generation = generation;
        }
    }

    /// Sends an event to the layout and carries out what it changed. Returns the result.
    fn dispatch(&mut self, event: Value, apply: Apply) -> Value {
        self.ensure_instance();
        if self.generation != engine::generation() {
            self.place_unplaced();
            return Value::Null;
        }
        self.send(event, &apply).unwrap_or(Value::Null)
    }

    fn send(&mut self, mut event: Value, apply: &Apply) -> Option<Value> {
        event["instance"] = json!(self.instance);
        let res = engine::call(&event.to_string()).and_then(|out| {
            serde_json::from_str::<Output>(&out).context("error reading the layout's answer")
        });

        match res {
            Ok(out) => {
                for log in &out.logs {
                    info!("tiling layout: {log}");
                }
                if let Some(error) = &out.error {
                    warn!("error in the tiling layout: {error}");
                }
                let result = self.apply(out, apply);
                self.place_unplaced();
                Some(result)
            }
            Err(err) => {
                warn!("error in the tiling layout, starting it over: {err:?}");
                // Create the instance again next time, from where the windows are now.
                self.generation = 0;
                self.place_unplaced();
                None
            }
        }
    }

    /// Asks the layout something without changing anything.
    fn query(&self, event: Value) -> Option<Value> {
        if self.generation == 0 || self.generation != engine::generation() {
            return None;
        }

        let mut event = event;
        event["instance"] = json!(self.instance);
        let res = engine::call(&event.to_string()).and_then(|out| {
            serde_json::from_str::<Output>(&out).context("error reading the layout's answer")
        });
        match res {
            Ok(out) => {
                if let Some(error) = &out.error {
                    warn!("error in the tiling layout: {error}");
                    return None;
                }
                Some(out.result)
            }
            Err(err) => {
                warn!("error in the tiling layout: {err:?}");
                None
            }
        }
    }

    /// Carries out what a layout call changed.
    fn apply(&mut self, out: Output, apply: &Apply) -> Value {
        if let Some(order) = out.order {
            // The layout lists windows bottom to top; tiles are top to bottom.
            let mut order: Vec<usize> = order
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

        for change in out.windows {
            let Some(idx) = self.idx_of_layout_id(change.id) else {
                continue;
            };
            self.place(
                idx,
                change.rect.to_rect(),
                change.mode,
                change.animate,
                apply,
            );
            self.data[idx].visible = change.visible;
        }

        if let Some(focused) = out.focused {
            self.active = focused.filter(|id| self.idx_of_layout_id(*id).is_some());
        }

        if let Some(camera) = out.camera {
            let animate = |config: niri_config::Animation, velocity: f64| {
                camera
                    .animate
                    .then(|| (self.clock.clone(), config, velocity))
            };
            let config = self.options.animations.horizontal_view_movement.0;
            let velocity = apply.camera_velocity;
            self.camera.gesture = None;
            let to = |v: f64| v.clamp(-1e7, 1e7);
            self.camera
                .x
                .move_to(to(camera.x), animate(config, velocity));
            self.camera.y.move_to(to(camera.y), animate(config, 0.));
        }

        if let Some(hint) = out.hint {
            self.hint = hint.map(Rect::to_rect);
        }

        if let Some(ipc) = out.ipc {
            for (id, pos) in ipc {
                if let Some(idx) = self.idx_of_layout_id(id) {
                    self.data[idx].ipc = pos;
                }
            }
        }

        out.result
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
        data.mode = mode;

        if resized {
            let transaction = apply.transaction.clone();
            match mode {
                Mode::Normal => tile.request_tile_size(rect.size, animate, transaction),
                Mode::Fullscreen => tile.request_fullscreen(animate, transaction),
                Mode::Maximized => tile.request_maximized(rect.size, animate, transaction),
            }
        }
    }

    /// Puts tiles that the layout didn't place in the middle of the view.
    fn place_unplaced(&mut self) {
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

    /// Tells the layout about windows whose size, limits or other properties changed.
    fn report_changes(&mut self, apply: Apply) {
        let mut changed = Vec::new();
        for idx in 0..self.tiles.len() {
            let info = self.window_info(idx);
            if self.data[idx].reported.as_ref() != Some(&info) {
                self.data[idx].reported = Some(info.clone());
                changed.push(info);
            }
        }
        if !changed.is_empty() {
            self.dispatch(json!({ "type": "change", "windows": changed }), apply);
        }
    }

    /// Adds a tile, with the layout deciding where it goes.
    pub fn add_tile(
        &mut self,
        col_idx: Option<usize>,
        tile: Tile<W>,
        activate: bool,
        width: ColumnWidth,
        is_full_width: bool,
        anim: Option<niri_config::Animation>,
    ) {
        let _ = col_idx;
        self.add_tile_with(tile, activate, width, is_full_width, anim, None, None);
    }

    /// Adds a tile next to the tile of `next_to`, like a dialog next to its parent.
    pub fn add_tile_right_of(
        &mut self,
        next_to: &W::Id,
        tile: Tile<W>,
        activate: bool,
        width: ColumnWidth,
        is_full_width: bool,
        anim: Option<niri_config::Animation>,
    ) {
        let next_to = self.layout_id(next_to);
        self.add_tile_with(tile, activate, width, is_full_width, anim, next_to, None);
    }

    /// Adds a tile that stopped floating, last seen at `rect` relative to the view.
    pub fn add_tile_from_floating(
        &mut self,
        tile: Tile<W>,
        activate: bool,
        width: ColumnWidth,
        rect: Rectangle<f64, Logical>,
    ) {
        let rect = Rectangle::new(rect.loc + self.camera.target(), rect.size);
        self.add_tile_with(
            tile,
            activate,
            width,
            false,
            None,
            None,
            Some(("tile", rect)),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn add_tile_with(
        &mut self,
        mut tile: Tile<W>,
        activate: bool,
        width: ColumnWidth,
        is_full_width: bool,
        anim: Option<niri_config::Animation>,
        next_to: Option<u64>,
        from: Option<(&str, Rectangle<f64, Logical>)>,
    ) {
        // The layout has to exist before the window, or it would get it twice.
        self.ensure_instance();

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
        } else if tile.was_tiled {
            "move"
        } else {
            "open"
        };
        tile.was_tiled = true;

        let default_width = if is_full_width {
            Preset::Proportion(1.)
        } else {
            match width {
                ColumnWidth::Proportion(prop) => Preset::Proportion(prop),
                ColumnWidth::Fixed(width) => Preset::Fixed(width),
            }
        };

        let id = TILE_ID_COUNTER.next();
        self.tiles.insert(0, tile);
        self.data.insert(
            0,
            TileData {
                id,
                rect: None,
                visible: true,
                mode: Mode::Normal,
                ipc: None,
                default_width: Some(default_width),
                reported: None,
            },
        );

        let info = self.window_info(0);
        self.data[0].reported = Some(info.clone());

        let (rect, pointer) = match (&drop, from) {
            (Some(drop), _) => (Some(drop.rect), Some(drop.pointer)),
            (None, Some((_, rect))) => (Some(rect), None),
            (None, None) => (None, None),
        };
        let event = json!({
            "type": "add",
            "window": info,
            "how": {
                "reason": reason,
                "activate": activate || self.tiles.len() == 1,
                "rect": rect.map(Rect::from_rect),
                "pointer": pointer.map(Xy::from_point),
                "nextTo": next_to,
            },
        });
        self.dispatch(
            event,
            Apply {
                anim,
                ..Apply::default()
            },
        );

        // The layout decides focus, but a window added to be focused has to be.
        if activate {
            self.active = Some(id);
        }
    }

    /// Adds windows that moved here together from another workspace.
    pub fn add_group(
        &mut self,
        group: TileGroup<W>,
        activate: bool,
        anim: Option<niri_config::Animation>,
    ) {
        self.ensure_instance();

        let mut infos = Vec::new();
        for (mut tile, _) in group.tiles {
            tile.update_config(self.view_size, self.scale, self.options.clone());
            tile.was_tiled = true;

            let width = tile.tile_expected_or_current_size().w;
            let id = TILE_ID_COUNTER.next();
            self.tiles.insert(0, tile);
            self.data.insert(
                0,
                TileData {
                    id,
                    rect: None,
                    visible: true,
                    mode: Mode::Normal,
                    ipc: None,
                    default_width: Some(Preset::Fixed(width)),
                    reported: None,
                },
            );
            let info = self.window_info(0);
            self.data[0].reported = Some(info.clone());
            infos.push(info);
        }

        let first = infos.first().map(|info| info.id);
        let event = json!({
            "type": "transfer-in",
            "windows": infos,
            "data": group.data,
            "activate": activate,
        });
        self.dispatch(
            event,
            Apply {
                anim,
                ..Apply::default()
            },
        );

        if activate {
            if let Some(first) = first {
                if self
                    .active
                    .is_none_or(|id| !infos.iter().any(|info| info.id == id))
                {
                    self.active = Some(first);
                }
            }
        }
    }

    pub fn remove_tile(&mut self, id: &W::Id, transaction: Transaction) -> RemovedTile<W> {
        // The layout has to exist with the window in it to remove it.
        self.ensure_instance();
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
        self.ensure_instance();
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

        self.dispatch(
            json!({ "type": "remove", "id": data.id, "reason": reason }),
            Apply {
                transaction,
                ..Apply::default()
            },
        );

        let width = tile.tile_expected_or_current_size().w;
        RemovedTile {
            tile,
            width: ColumnWidth::Fixed(width),
            is_full_width: false,
            is_floating: false,
        }
    }

    /// Removes the focused window together with the windows the layout moves along with it.
    pub fn remove_active_group(&mut self) -> Option<TileGroup<W>> {
        self.ensure_instance();
        let active = self.active?;
        let transfer = self
            .query(json!({ "type": "transfer-out", "id": active }))
            .and_then(|out| serde_json::from_value::<TransferOut>(out).ok())
            .unwrap_or(TransferOut {
                windows: vec![active],
                data: Value::Null,
            });

        let mut tiles = Vec::new();
        for id in transfer.windows {
            let Some(idx) = self.idx_of_layout_id(id) else {
                continue;
            };
            let (_, pos, _) = self.tiles_with_render_positions().nth(idx).unwrap();
            let removed = self.remove_tile_by_idx(idx, "move", None);
            tiles.push((removed.tile, pos));
        }

        Some(TileGroup {
            tiles,
            data: transfer.data,
        })
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

        self.report_changes(Apply::default());
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

    pub fn activate_window(&mut self, id: &W::Id) -> bool {
        let Some(idx) = self.idx_of(id) else {
            return false;
        };
        let layout_id = self.data[idx].id;
        self.dispatch(
            json!({ "type": "activate", "id": layout_id }),
            Apply::default(),
        );

        // Whatever the layout does, the window asked to be focused.
        self.active = Some(layout_id);
        true
    }

    pub fn set_fullscreen(&mut self, id: &W::Id, is_fullscreen: bool) -> bool {
        let mode = if is_fullscreen {
            "fullscreen"
        } else {
            "normal"
        };
        self.request_mode(id, mode)
    }

    pub fn set_maximized(&mut self, id: &W::Id, maximize: bool) -> bool {
        let mode = if maximize { "maximized" } else { "normal" };
        self.request_mode(id, mode)
    }

    fn request_mode(&mut self, id: &W::Id, mode: &str) -> bool {
        let Some(layout_id) = self.layout_id(id) else {
            return false;
        };
        self.dispatch(
            json!({ "type": "mode", "id": layout_id, "mode": mode }),
            Apply::default(),
        );
        true
    }

    pub fn render_above_top_layer(&self) -> bool {
        // Render above the top layer if we're on a fullscreen window and the view is stationary.
        self.camera.is_static()
            && self
                .active_window()
                .is_some_and(|win| win.sizing_mode().is_fullscreen())
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

    /// Where a dragged tile would go if dropped with the pointer at `pos`, relative to the view.
    ///
    /// `tile_pos` is where the dragged tile is, relative to the view.
    pub(super) fn insert_position(
        &self,
        pos: Point<f64, Logical>,
        tile: &Tile<W>,
        tile_pos: Point<f64, Logical>,
    ) -> InsertPosition {
        let camera = self.camera.current();
        let pointer = pos + camera;
        let rect = Rectangle::new(tile_pos + camera, tile.tile_size());

        let borders = tile.tile_width_for_window_width(0.);
        let window = WindowInfo {
            id: 0,
            app_id: None,
            title: None,
            size: Wh::from_size(tile.tile_size()),
            min_size: Wh::from_size(tile.min_size_nonfullscreen()),
            max_size: Wh::from_size(tile.max_size_nonfullscreen()),
            borders: Wh {
                w: borders,
                h: tile.tile_height_for_window_height(0.),
            },
            mode: Mode::Normal,
            rules: Rules {
                default_width: Some(Preset::Fixed(tile.tile_size().w)),
                default_height: None,
            },
            is_urgent: false,
            rect: None,
        };
        let hint = self
            .query(json!({
                "type": "drop-target",
                "window": window,
                "pointer": Xy::from_point(pointer),
                "rect": Rect::from_rect(rect),
            }))
            .and_then(|hint| serde_json::from_value::<Option<Rect>>(hint).ok())
            .flatten()
            .map(Rect::to_rect);

        InsertPosition::Tiled(Some(TiledDrop {
            pointer,
            rect,
            hint,
        }))
    }

    /// Area of the drop target highlight, relative to the view.
    pub(super) fn insert_hint_area(
        &self,
        position: InsertPosition,
    ) -> Option<Rectangle<f64, Logical>> {
        let InsertPosition::Tiled(Some(drop)) = position else {
            return None;
        };
        let mut hint = drop.hint?;
        hint.loc -= self.camera.current();
        Some(hint)
    }

    /// Makes the window go where it was dropped when it's added next.
    pub(super) fn set_drop(&mut self, window: W::Id, drop: TiledDrop) {
        self.drop = Some((window, drop));
    }

    pub fn view_offset_gesture_begin(&mut self, is_touchpad: bool) {
        if self.tiles.is_empty() || self.interactive_resize.is_some() {
            return;
        }

        let current = self.camera.current().x;
        self.camera.gesture = Some(CameraGesture {
            tracker: SwipeTracker::new(),
            delta_from_tracker: current,
            current,
            is_touchpad,
            dnd_last_event_time: None,
            dnd_nonzero_start_time: None,
        });
    }

    pub fn view_offset_gesture_update(
        &mut self,
        delta_x: f64,
        timestamp: Duration,
        is_touchpad: bool,
    ) -> Option<bool> {
        let gesture = self.camera.gesture.as_mut()?;
        if gesture.is_touchpad != is_touchpad || gesture.dnd_last_event_time.is_some() {
            return None;
        }

        gesture.tracker.push(delta_x, timestamp);

        let norm_factor = if gesture.is_touchpad {
            self.working_area.size.w / VIEW_GESTURE_WORKING_AREA_MOVEMENT
        } else {
            1.
        };
        gesture.current = gesture.tracker.pos() * norm_factor + gesture.delta_from_tracker;

        Some(true)
    }

    pub fn view_offset_gesture_end(&mut self, is_touchpad: Option<bool>) -> bool {
        let Some(gesture) = &mut self.camera.gesture else {
            return false;
        };
        if is_touchpad.is_some_and(|x| gesture.is_touchpad != x) {
            return false;
        }

        // Take into account any idle time between the last event and now.
        let now = self.clock.now_unadjusted();
        gesture.tracker.push(0., now);

        let norm_factor = if gesture.is_touchpad {
            self.working_area.size.w / VIEW_GESTURE_WORKING_AREA_MOVEMENT
        } else {
            1.
        };
        let velocity = gesture.tracker.velocity() * norm_factor;
        let current = gesture.tracker.pos() * norm_factor + gesture.delta_from_tracker;
        let projected =
            gesture.tracker.projected_end_pos() * norm_factor + gesture.delta_from_tracker;

        let y = self.camera.y.target();
        self.camera.gesture = None;
        self.camera.x = Axis::Static(current);

        self.dispatch(
            json!({
                "type": "gesture-end",
                "camera": Xy { x: current, y },
                "velocity": Xy { x: velocity, y: 0. },
                "projected": Xy { x: projected, y },
            }),
            Apply {
                camera_velocity: velocity,
                ..Apply::default()
            },
        );
        true
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
            is_touchpad: false,
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

        let camera = Xy::from_point(self.camera.target());
        self.dispatch(
            json!({ "type": "camera", "camera": camera }),
            Apply::default(),
        );
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

    pub fn interactive_resize_begin(&mut self, window: W::Id, edges: ResizeEdge) -> bool {
        if self.interactive_resize.is_some() {
            return false;
        }
        let Some(idx) = self.idx_of(&window) else {
            return false;
        };

        let event = json!({
            "type": "resize-begin",
            "id": self.data[idx].id,
            "resize": resize_json(edges, Point::from((0., 0.))),
        });
        if self.dispatch(event, Apply::default()) != Value::Bool(true) {
            return false;
        }

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
        let edges = resize.data.edges;
        let Some(id) = self.layout_id(window) else {
            return false;
        };

        let event = json!({
            "type": "resize-update",
            "id": id,
            "resize": resize_json(edges, delta),
        });
        self.dispatch(event, Apply::default());
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
            self.dispatch(json!({ "type": "resize-end", "id": id }), Apply::default());
        }
    }

    pub fn refresh(&mut self, is_active: bool, is_focused: bool) {
        let active = self.active;
        for (tile, data) in zip(&mut self.tiles, &self.data) {
            let win = tile.window_mut();

            win.set_active_in_column(data.visible);
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
            let border = if border.off { 0. } else { border.width * 2. };
            let gaps = self.options.layout.gaps;
            let bounds = Size::from((
                f64::max(self.working_area.size.w - gaps * 2. - border, 1.),
                f64::max(self.working_area.size.h - gaps * 2. - border, 1.),
            ))
            .to_i32_floor();
            win.set_bounds(bounds);

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

    /// Runs an action on the layout. Returns whether it did anything.
    pub fn action(&mut self, name: &str, args: Vec<Value>, window: Option<&W::Id>) -> bool {
        let target = window.and_then(|id| self.layout_id(id));
        if window.is_some() && target.is_none() {
            return false;
        }

        let event = json!({ "type": "action", "name": name, "args": args, "target": target });
        let res = self.dispatch(event, Apply::default());

        // Actions on a window end resizing it with the mouse.
        if let Some(resize) = &self.interactive_resize {
            if window.is_none_or(|id| id == &resize.window) {
                let window = resize.window.clone();
                self.interactive_resize_end(Some(&window));
            }
        }

        res == Value::Bool(true)
    }

    /// Whether a swipe or drag-and-drop gesture is moving the camera.
    pub fn is_camera_gesture(&self) -> bool {
        self.camera.gesture.is_some()
    }

    /// Camera x on the canvas, like niri's view position.
    pub fn view_pos(&self) -> f64 {
        self.camera.current().x
    }

    /// The camera, for tests written against niri's view offset.
    #[cfg(test)]
    pub(super) fn view_offset(&self) -> CameraState {
        CameraState {
            is_animation_ongoing: !self.camera.is_static() && self.camera.gesture.is_none(),
        }
    }

    /// Index of the focused window's column, from the layout's IPC positions.
    #[cfg(test)]
    pub fn active_column_idx(&self) -> usize {
        self.active_idx()
            .and_then(|idx| self.data[idx].ipc)
            .map_or(0, |(col, _)| col.saturating_sub(1))
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
            assert!(
                tile.was_tiled,
                "tiles in the tiling layout must be marked as tiled"
            );
        }

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

fn preset(value: PresetSize) -> Preset {
    match value {
        PresetSize::Proportion(prop) => Preset::Proportion(prop),
        PresetSize::Fixed(width) => Preset::Fixed(f64::from(width)),
    }
}

fn mode_of(mode: SizingMode) -> Mode {
    match mode {
        SizingMode::Normal => Mode::Normal,
        SizingMode::Fullscreen => Mode::Fullscreen,
        SizingMode::Maximized => Mode::Maximized,
    }
}

fn resize_json(edges: ResizeEdge, delta: Point<f64, Logical>) -> Value {
    json!({
        "edges": {
            "left": edges.contains(ResizeEdge::LEFT),
            "right": edges.contains(ResizeEdge::RIGHT),
            "top": edges.contains(ResizeEdge::TOP),
            "bottom": edges.contains(ResizeEdge::BOTTOM),
        },
        "delta": Xy::from_point(delta),
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

/// A size change from an action, as the layout gets it.
fn size_change_json(change: SizeChange) -> Value {
    let (op, value, unit) = match change {
        SizeChange::SetFixed(value) => ("set", f64::from(value), "px"),
        SizeChange::SetProportion(value) => ("set", value, "%"),
        SizeChange::AdjustFixed(value) => ("adjust", f64::from(value), "px"),
        SizeChange::AdjustProportion(value) => ("adjust", value, "%"),
    };
    json!({ "op": op, "value": value, "unit": unit })
}
