use std::cmp::max;
use std::rc::Rc;
use std::time::Duration;

use niri_config::{CornerRadius, OutputName, Workspace as WorkspaceConfig};
use niri_ipc::{PositionChange, WindowLayout};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::{layer_map_for_output, Window};
use smithay::output::Output;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Serial, Size, Transform};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::SurfaceCachedState;

use super::floating::{FloatingSpace, FloatingSpaceRenderElement};
use super::monitor::TiledDrop;
use super::shadow::Shadow;
use super::tile::{PendingMove, Tile, TileRenderSnapshot};
use super::tiling::{TilingSpace, TilingSpaceRenderElement};
use super::{
    ActivateWindow, HitType, InsertPosition, InteractiveResizeData, LayoutElement, Options,
    RemovedTile, SizeFrac,
};
use crate::animation::Clock;
use crate::layout::RenderLayer;
use crate::niri_render_elements;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::shadow::ShadowRenderElement;
use crate::render_helpers::solid_color::{SolidColorBuffer, SolidColorRenderElement};
use crate::render_helpers::xray::{Xray, XrayPos};
use crate::render_helpers::RenderCtx;
use crate::utils::id::IdCounter;
use crate::utils::transaction::{Transaction, TransactionBlocker};
use crate::utils::{
    ensure_min_max_size, ensure_min_max_size_maybe_zero, output_size, send_scale_transform,
    ResizeEdge,
};
use crate::window::ResolvedWindowRules;

#[derive(Debug)]
pub struct Workspace<W: LayoutElement> {
    /// The tiling layout, which a layout script controls.
    scrolling: TilingSpace<W>,

    /// The floating layout.
    floating: FloatingSpace<W>,

    /// Whether the floating layout is active instead of the scrolling layout.
    floating_is_active: FloatingActive,

    /// The original outputs of this workspace, in preference order.
    ///
    /// Most of the time this will be the workspace's current output, however, after an output
    /// disconnection, they may remain pointing to disconnected outputs.
    pub(super) original_outputs: Vec<OutputId>,

    /// Current output of this workspace.
    output: Option<Output>,

    /// Latest known output scale for this workspace.
    ///
    /// This should be set from the current workspace output, or, if all outputs have been
    /// disconnected, preserved until a new output is connected.
    scale: smithay::output::Scale,

    /// Latest known output transform for this workspace.
    ///
    /// This should be set from the current workspace output, or, if all outputs have been
    /// disconnected, preserved until a new output is connected.
    transform: Transform,

    /// Latest known view size for this workspace.
    ///
    /// This should be computed from the current workspace output size, or, if all outputs have
    /// been disconnected, preserved until a new output is connected.
    view_size: Size<f64, Logical>,

    /// Latest known working area for this workspace.
    ///
    /// Not rounded to physical pixels.
    ///
    /// This is similar to view size, but takes into account things like layer shell exclusive
    /// zones.
    working_area: Rectangle<f64, Logical>,

    /// This workspace's shadow in the overview.
    shadow: Shadow,

    /// This workspace's background.
    background_buffer: SolidColorBuffer,

    /// Clock for driving animations.
    pub(super) clock: Clock,

    /// Configurable properties of the layout as received from the parent monitor.
    pub(super) base_options: Rc<Options>,

    /// Configurable properties of the layout with logical sizes adjusted for the current `scale`.
    pub(super) options: Rc<Options>,

    /// Optional name of this workspace.
    pub(super) name: Option<String>,

    /// Layout config overrides for this workspace.
    layout_config: Option<niri_config::LayoutPart>,

    /// Unique ID of this workspace.
    id: WorkspaceId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputId(String);

impl OutputId {
    pub fn matches(&self, output: &Output) -> bool {
        let output_name = output.user_data().get::<OutputName>().unwrap();
        output_name.matches(&self.0)
    }
}

static WORKSPACE_ID_COUNTER: IdCounter = IdCounter::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkspaceId(u64);

impl WorkspaceId {
    fn next() -> WorkspaceId {
        WorkspaceId(WORKSPACE_ID_COUNTER.next())
    }

    pub fn get(self) -> u64 {
        self.0
    }

    pub fn specific(id: u64) -> Self {
        Self(id)
    }
}

niri_render_elements! {
    WorkspaceRenderElement<R> => {
        Scrolling = TilingSpaceRenderElement<R>,
        Floating = FloatingSpaceRenderElement<R>,
    }
}

#[derive(Debug)]
pub(super) struct InteractiveResize<W: LayoutElement> {
    pub window: W::Id,
    pub original_window_size: Size<f64, Logical>,
    pub data: InteractiveResizeData,
}

/// Whether the floating space is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FloatingActive {
    /// The scrolling space is active.
    No,
    /// The scrolling space is active, but the floating space should render on top, even if the
    /// active scrolling window is fullscreen.
    ///
    /// This is necessary for focus-follows-mouse that activates but doesn't raise the window to
    /// avoid being annoying.
    NoButRaised,
    /// The floating space is active.
    Yes,
}

/// Where to put a newly added window.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceAddWindowTarget<'a, W: LayoutElement> {
    /// No particular preference.
    #[default]
    Auto,
    /// Next to this existing window.
    NextTo(&'a W::Id),
}

impl OutputId {
    pub fn new(output: &Output) -> Self {
        let output_name = output.user_data().get::<OutputName>().unwrap();
        Self(output_name.format_make_model_serial_or_connector())
    }
}

impl FloatingActive {
    fn get(self) -> bool {
        self == Self::Yes
    }
}

impl<W: LayoutElement> Workspace<W> {
    pub fn new(output: Output, clock: Clock, options: Rc<Options>) -> Self {
        Self::new_with_config(output, None, clock, options)
    }

    pub fn new_with_config(
        output: Output,
        mut config: Option<WorkspaceConfig>,
        clock: Clock,
        base_options: Rc<Options>,
    ) -> Self {
        let mut original_outputs: Vec<OutputId> = config
            .as_ref()
            .map(|c| c.open_on_output.iter().cloned().map(OutputId).collect())
            .unwrap_or_default();
        if original_outputs.is_empty() {
            original_outputs.push(OutputId::new(&output));
        } else {
            Self::normalize_original_outputs(&mut original_outputs, &output);
        }

        let layout_config = config.as_mut().and_then(|c| c.layout.take().map(|x| x.0));

        let scale = output.current_scale();
        let options = Rc::new(
            Options::clone(&base_options)
                .with_merged_layout(layout_config.as_ref())
                .adjusted_for_scale(scale.fractional_scale()),
        );

        let view_size = output_size(&output);
        let working_area = compute_working_area(&output);

        let id = WorkspaceId::next();
        let scrolling = TilingSpace::new(
            id.get(),
            view_size,
            working_area,
            scale.fractional_scale(),
            clock.clone(),
            options.clone(),
        );

        let floating = FloatingSpace::new(
            view_size,
            working_area,
            scale.fractional_scale(),
            clock.clone(),
            options.clone(),
        );

        let shadow_config =
            compute_workspace_shadow_config(options.overview.workspace_shadow, view_size);

        Self {
            scrolling,
            floating,
            floating_is_active: FloatingActive::No,
            original_outputs,
            scale,
            transform: output.current_transform(),
            view_size,
            working_area,
            shadow: Shadow::new(shadow_config),
            background_buffer: SolidColorBuffer::new(view_size, options.layout.background_color),
            output: Some(output),
            clock,
            base_options,
            options,
            name: config.map(|c| c.name.0),
            layout_config,
            id,
        }
    }

    pub fn new_with_config_no_outputs(
        mut config: Option<WorkspaceConfig>,
        clock: Clock,
        base_options: Rc<Options>,
    ) -> Self {
        let original_outputs = config
            .as_ref()
            .map(|c| c.open_on_output.iter().cloned().map(OutputId).collect())
            .unwrap_or_default();

        let layout_config = config.as_mut().and_then(|c| c.layout.take().map(|x| x.0));

        let scale = smithay::output::Scale::Integer(1);
        let options = Rc::new(
            Options::clone(&base_options)
                .with_merged_layout(layout_config.as_ref())
                .adjusted_for_scale(scale.fractional_scale()),
        );

        let view_size = Size::from((1280., 720.));
        let working_area = Rectangle::from_size(Size::from((1280., 720.)));

        let id = WorkspaceId::next();
        let scrolling = TilingSpace::new(
            id.get(),
            view_size,
            working_area,
            scale.fractional_scale(),
            clock.clone(),
            options.clone(),
        );

        let floating = FloatingSpace::new(
            view_size,
            working_area,
            scale.fractional_scale(),
            clock.clone(),
            options.clone(),
        );

        let shadow_config =
            compute_workspace_shadow_config(options.overview.workspace_shadow, view_size);

        Self {
            scrolling,
            floating,
            floating_is_active: FloatingActive::No,
            output: None,
            scale,
            transform: Transform::Normal,
            original_outputs,
            view_size,
            working_area,
            shadow: Shadow::new(shadow_config),
            background_buffer: SolidColorBuffer::new(view_size, options.layout.background_color),
            clock,
            base_options,
            options,
            name: config.map(|c| c.name.0),
            layout_config,
            id,
        }
    }

    pub fn new_no_outputs(clock: Clock, options: Rc<Options>) -> Self {
        Self::new_with_config_no_outputs(None, clock, options)
    }

    pub fn id(&self) -> WorkspaceId {
        self.id
    }

    pub fn name(&self) -> Option<&String> {
        self.name.as_ref()
    }

    pub fn unname(&mut self) {
        self.name = None;
    }

    pub fn has_windows_or_name(&self) -> bool {
        self.has_windows() || self.name.is_some()
    }

    pub fn scale(&self) -> smithay::output::Scale {
        self.scale
    }

    pub fn advance_animations(&mut self) {
        self.scrolling.advance_animations();
        self.floating.advance_animations();
    }

    pub fn are_animations_ongoing(&self) -> bool {
        self.scrolling.are_animations_ongoing() || self.floating.are_animations_ongoing()
    }

    pub fn are_transitions_ongoing(&self) -> bool {
        self.scrolling.are_transitions_ongoing() || self.floating.are_transitions_ongoing()
    }

    pub fn update_render_elements(&mut self, is_active: bool, layer: RenderLayer) {
        self.scrolling
            .update_render_elements(is_active && !self.floating_is_active.get(), layer);

        let view_rect = Rectangle::from_size(self.view_size);
        self.floating.update_render_elements(
            is_active && self.floating_is_active.get(),
            view_rect,
            layer,
        );

        if layer.is_normal() {
            self.shadow.update_render_elements(
                self.view_size,
                true,
                CornerRadius::default(),
                self.scale.fractional_scale(),
                1.,
            );
        }
    }

    pub fn update_config(&mut self, base_options: Rc<Options>) {
        let scale = self.scale.fractional_scale();
        let options = Rc::new(
            Options::clone(&base_options)
                .with_merged_layout(self.layout_config.as_ref())
                .adjusted_for_scale(scale),
        );

        self.scrolling.update_config(
            self.view_size,
            self.working_area,
            self.scale.fractional_scale(),
            options.clone(),
        );

        self.floating.update_config(
            self.view_size,
            self.working_area,
            self.scale.fractional_scale(),
            options.clone(),
        );

        let shadow_config =
            compute_workspace_shadow_config(options.overview.workspace_shadow, self.view_size);
        self.shadow.update_config(shadow_config);

        self.background_buffer
            .set_color(options.layout.background_color);

        self.base_options = base_options;
        self.options = options;
    }

    pub fn update_layout_config(&mut self, layout_config: Option<niri_config::LayoutPart>) {
        if self.layout_config == layout_config {
            return;
        }

        self.layout_config = layout_config;
        self.update_config(self.base_options.clone());
    }

    pub fn update_shaders(&mut self) {
        self.scrolling.update_shaders();
        self.floating.update_shaders();
        self.shadow.update_shaders();
    }

    pub fn windows(&self) -> impl Iterator<Item = &W> + '_ {
        self.tiles().map(Tile::window)
    }

    pub fn windows_mut(&mut self) -> impl Iterator<Item = &mut W> + '_ {
        self.tiles_mut().map(Tile::window_mut)
    }

    pub fn tiles(&self) -> impl Iterator<Item = &Tile<W>> + '_ {
        let scrolling = self.scrolling.tiles();
        let floating = self.floating.tiles();
        scrolling.chain(floating)
    }

    pub fn tiles_mut(&mut self) -> impl Iterator<Item = &mut Tile<W>> + '_ {
        let scrolling = self.scrolling.tiles_mut();
        let floating = self.floating.tiles_mut();
        scrolling.chain(floating)
    }

    pub fn is_floating(&self, id: &W::Id) -> bool {
        self.floating.has_window(id)
    }

    pub fn current_output(&self) -> Option<&Output> {
        self.output.as_ref()
    }

    pub(super) fn find_preferred_output<'a>(
        &self,
        outputs: impl Iterator<Item = &'a Output> + Clone,
    ) -> Option<usize> {
        self.original_outputs
            .iter()
            .find_map(|id| outputs.clone().position(|output| id.matches(output)))
    }

    pub fn active_window(&self) -> Option<&W> {
        if self.floating_is_active.get() {
            self.floating.active_window()
        } else {
            self.scrolling.active_window()
        }
    }

    pub fn active_window_mut(&mut self) -> Option<&mut W> {
        if self.floating_is_active.get() {
            self.floating.active_window_mut()
        } else {
            self.scrolling.active_window_mut()
        }
    }

    pub fn is_active_pending_fullscreen(&self) -> bool {
        self.scrolling.is_active_pending_fullscreen()
    }

    pub fn set_output(&mut self, output: Option<Output>) {
        if self.output == output {
            return;
        }

        if let Some(output) = self.output.take() {
            for win in self.windows() {
                win.output_leave(&output);
            }
        }

        self.output = output;

        if let Some(output) = &self.output {
            if self.original_outputs.is_empty() {
                self.original_outputs.push(OutputId::new(output));
            } else {
                // Normalize original outputs: possibly replace connectors with make/model/serial.
                Self::normalize_original_outputs(&mut self.original_outputs, output);
            }

            self.update_output_size();

            for win in self.windows() {
                self.enter_output_for_window(win);
            }
        }
    }

    fn normalize_original_outputs(outputs: &mut Vec<OutputId>, output: &Output) {
        for id in &mut *outputs {
            if id.matches(output) {
                *id = OutputId::new(output);
            }
        }

        // Connector and make/model/serial candidates may resolve to the same output.
        for idx in (0..outputs.len()).rev() {
            if outputs[..idx].contains(&outputs[idx]) {
                outputs.remove(idx);
            }
        }
    }

    fn enter_output_for_window(&self, window: &W) {
        if let Some(output) = &self.output {
            window.set_preferred_scale_transform(self.scale, self.transform);
            window.output_enter(output);
        }
    }

    pub fn update_output_size(&mut self) {
        let output = self.output.as_ref().unwrap();
        let scale = output.current_scale();
        let transform = output.current_transform();
        let view_size = output_size(output);
        let working_area = compute_working_area(output);
        self.set_view_size(scale, transform, view_size, working_area);
    }

    fn set_view_size(
        &mut self,
        scale: smithay::output::Scale,
        transform: Transform,
        size: Size<f64, Logical>,
        working_area: Rectangle<f64, Logical>,
    ) {
        let scale_transform_changed = self.transform != transform
            || self.scale.integer_scale() != scale.integer_scale()
            || self.scale.fractional_scale() != scale.fractional_scale();
        if !scale_transform_changed && self.view_size == size && self.working_area == working_area {
            return;
        }

        let fractional_scale_changed = self.scale.fractional_scale() != scale.fractional_scale();

        self.scale = scale;
        self.transform = transform;
        self.view_size = size;
        self.working_area = working_area;

        if fractional_scale_changed {
            // Options need to be recomputed for the new scale.
            self.update_config(self.base_options.clone());
        } else {
            // Pass our existing options as is.
            self.scrolling.update_config(
                size,
                working_area,
                scale.fractional_scale(),
                self.options.clone(),
            );
            self.floating.update_config(
                size,
                working_area,
                scale.fractional_scale(),
                self.options.clone(),
            );

            let shadow_config =
                compute_workspace_shadow_config(self.options.overview.workspace_shadow, size);
            self.shadow.update_config(shadow_config);
        }

        self.background_buffer.resize(size);

        if scale_transform_changed {
            for window in self.windows() {
                window.set_preferred_scale_transform(self.scale, self.transform);
            }
        }
    }

    pub fn view_size(&self) -> Size<f64, Logical> {
        self.view_size
    }

    pub fn make_tile(&self, window: W) -> Tile<W> {
        Tile::new(
            window,
            self.view_size,
            self.scale.fractional_scale(),
            self.clock.clone(),
            self.options.clone(),
        )
    }

    pub fn add_tile(
        &mut self,
        mut tile: Tile<W>,
        target: WorkspaceAddWindowTarget<W>,
        activate: ActivateWindow,
        is_floating: bool,
        anim: Option<niri_config::Animation>,
    ) {
        self.enter_output_for_window(tile.window());
        tile.restore_to_floating = is_floating;

        match target {
            WorkspaceAddWindowTarget::Auto => {
                // Don't steal focus from an active fullscreen window.
                let activate = activate.map_smart(|| !self.is_active_pending_fullscreen());

                // If the tile is pending maximized or fullscreen, open it in the scrolling layout
                // where it can do that.
                if is_floating && tile.window().pending_sizing_mode().is_normal() {
                    self.floating.add_tile(tile, activate);

                    if activate || self.scrolling.is_empty() {
                        self.floating_is_active = FloatingActive::Yes;
                    }
                } else {
                    self.scrolling.add_tile(tile, activate, anim);

                    if activate {
                        self.floating_is_active = FloatingActive::No;
                    }
                }
            }
            WorkspaceAddWindowTarget::NextTo(next_to) => {
                let activate = activate.map_smart(|| self.active_window().unwrap().id() == next_to);

                let floating_has_window = self.floating.has_window(next_to);

                if is_floating && tile.window().pending_sizing_mode().is_normal() {
                    if floating_has_window {
                        self.floating.add_tile_above(next_to, tile, activate);
                    } else {
                        // FIXME: use static pos
                        let (next_to_tile, render_pos, _visible) = self
                            .scrolling
                            .tiles_with_render_positions()
                            .find(|(tile, _, _)| tile.window().id() == next_to)
                            .unwrap();

                        // Position the new tile in the center above the next_to tile. Think a
                        // dialog opening on top of a window.
                        let tile_size = tile.tile_size();
                        let pos = render_pos
                            + (next_to_tile.tile_size().to_point() - tile_size.to_point())
                                .downscale(2.);
                        let pos = self.floating.clamp_within_working_area(pos, tile_size);
                        let pos = self.floating.logical_to_size_frac(pos);
                        tile.floating_pos = Some(pos);

                        self.floating.add_tile(tile, activate);
                    }

                    if activate || self.scrolling.is_empty() {
                        self.floating_is_active = FloatingActive::Yes;
                    }
                } else if floating_has_window {
                    self.scrolling.add_tile(tile, activate, anim);

                    if activate {
                        self.floating_is_active = FloatingActive::No;
                    }
                } else {
                    self.scrolling
                        .add_tile_right_of(next_to, tile, activate, anim);

                    if activate {
                        self.floating_is_active = FloatingActive::No;
                    }
                }
            }
        }
    }

    fn update_focus_floating_tiling_after_removing(&mut self, removed_from_floating: bool) {
        if removed_from_floating {
            if self.floating.is_empty() {
                self.floating_is_active = FloatingActive::No;
            }
        } else {
            // Scrolling should remain focused if both are empty.
            if self.scrolling.is_empty() && !self.floating.is_empty() {
                self.floating_is_active = FloatingActive::Yes;
            }
        }
    }

    pub fn remove_tile(&mut self, id: &W::Id, transaction: Transaction) -> RemovedTile<W> {
        let mut from_floating = false;
        let removed = if self.floating.has_window(id) {
            from_floating = true;
            self.floating.remove_tile(id)
        } else {
            self.scrolling.remove_tile(id, transaction)
        };

        if let Some(output) = &self.output {
            removed.tile.window().output_leave(output);
        }

        self.update_focus_floating_tiling_after_removing(from_floating);

        removed
    }

    /// Makes a tile that was just added to the tiling layout animate from where it was, once the
    /// layout places it.
    pub(super) fn set_pending_move(&mut self, id: &W::Id, moved: PendingMove) {
        self.scrolling.set_pending_move(id, moved);
    }

    /// Tells the tiling layout why `id` is about to be removed, when it isn't closing.
    pub(super) fn set_remove_reason(&mut self, id: &W::Id, reason: &'static str) {
        self.scrolling.set_remove_reason(id, reason);
    }

    pub fn new_window_size(
        &self,
        is_floating: bool,
        rules: &ResolvedWindowRules,
        (min_size, max_size): (Size<i32, Logical>, Size<i32, Logical>),
    ) -> Size<i32, Logical> {
        // Floating windows pick their own size.
        let mut size = if is_floating {
            Size::from((0, 0))
        } else {
            self.scrolling.new_window_size(rules)
        };

        // If the window has a fixed size, or we're picking some fixed size, apply min and max
        // size. This is to ensure that a fixed-size window rule works on open, while still
        // allowing the window freedom to pick its default size otherwise.
        let (min_size, max_size) = rules.apply_min_max_size(min_size, max_size);
        size.w = ensure_min_max_size_maybe_zero(size.w, min_size.w, max_size.w);
        // For scrolling (where height is > 0) only ensure fixed height, since at runtime scrolling
        // will only honor fixed height currently.
        if min_size.h == max_size.h {
            size.h = ensure_min_max_size(size.h, min_size.h, max_size.h);
        } else if size.h > 0 {
            // Also always honor min height, scrolling always does.
            size.h = max(size.h, min_size.h);
        }

        size
    }

    pub fn configure_new_window(
        &self,
        window: &Window,
        is_floating: bool,
        rules: &ResolvedWindowRules,
    ) {
        window.with_surfaces(|surface, data| {
            send_scale_transform(surface, data, self.scale, self.transform);
        });

        let toplevel = window.toplevel().expect("no x11 support");
        let (min_size, max_size) = with_states(toplevel.wl_surface(), |state| {
            let mut guard = state.cached_state.get::<SurfaceCachedState>();
            let current = guard.current();
            (current.min_size, current.max_size)
        });
        toplevel.with_pending_state(|state| {
            if state.states.contains(xdg_toplevel::State::Fullscreen) {
                state.size = Some(self.view_size.to_i32_round());
            } else if state.states.contains(xdg_toplevel::State::Maximized) {
                state.size = Some(self.working_area.size.to_i32_round());
            } else {
                let size = self.new_window_size(is_floating, rules, (min_size, max_size));
                state.size = Some(size);
            }

            if is_floating {
                state.bounds = Some(self.floating.new_window_toplevel_bounds(rules));
            } else {
                state.bounds = Some(self.scrolling.new_window_toplevel_bounds(rules));
            }
        });
    }

    pub fn set_fullscreen(&mut self, window: &W::Id, is_fullscreen: bool) {
        let mut restore_to_floating = false;
        if self.floating.has_window(window) {
            if is_fullscreen {
                restore_to_floating = true;
                self.toggle_window_floating(Some(window));
            } else {
                // Floating windows are never fullscreen, so this is an unfullscreen request for an
                // already unfullscreen window.
                return;
            }
        } else if !is_fullscreen {
            // The window is in the scrolling layout and we're requesting an unfullscreen. If it is
            // indeed fullscreen (i.e. this isn't a duplicate unfullscreen request), then we may
            // need to unfullscreen into floating.
            let tile = self
                .scrolling
                .tiles()
                .find(|tile| tile.window().id() == window)
                .unwrap();

            // When going from fullscreen to maximized, don't consider restore_to_floating yet.
            if tile.window().pending_sizing_mode().is_fullscreen()
                && !self.scrolling.is_pending_maximized(window)
            {
                if tile.restore_to_floating {
                    // Unfullscreen and float in one call so it has a chance to notice and request a
                    // (0, 0) size, rather than the scrolling column size.
                    self.toggle_window_floating(Some(window));
                    return;
                }
            }
        }

        let tile = self
            .scrolling
            .tiles()
            .find(|tile| tile.window().id() == window)
            .unwrap();
        let was_normal = tile.window().pending_sizing_mode().is_normal();

        self.scrolling.set_fullscreen(window, is_fullscreen);

        // When going from normal to fullscreen, remember if we should unfullscreen to floating.
        // The layout engine decides whether the window goes fullscreen, and does so after this.
        let tile = self
            .scrolling
            .tiles_mut()
            .find(|tile| tile.window().id() == window)
            .unwrap();
        if was_normal && is_fullscreen {
            tile.restore_to_floating = restore_to_floating;
        }
    }

    pub fn toggle_fullscreen(&mut self, window: &W::Id) {
        let tile = self
            .tiles()
            .find(|tile| tile.window().id() == window)
            .unwrap();
        let current = tile.window().pending_sizing_mode().is_fullscreen();
        self.set_fullscreen(window, !current);
    }

    pub fn set_maximized(&mut self, window: &W::Id, maximize: bool) {
        let mut restore_to_floating = false;
        if self.floating.has_window(window) {
            if maximize {
                restore_to_floating = true;
                self.toggle_window_floating(Some(window));
            } else {
                // Floating windows are never maximized, so this is an unmaximize request for an
                // already unmaximized window.
                return;
            }
        } else if !maximize {
            // The window is in the scrolling layout and we're requesting to unmaximize. If it is
            // indeed maximized (i.e. this isn't a duplicate unmaximize request), then we may
            // need to unmaximize into floating.
            let tile = self
                .scrolling
                .tiles()
                .find(|tile| tile.window().id() == window)
                .unwrap();
            // The tile cannot unmaximize into fullscreen (pending_sizing_mode() will be fullscreen
            // in that case and not maximized), so this check works.
            if tile.window().pending_sizing_mode().is_maximized() && tile.restore_to_floating {
                // Unmaximize and float in one call so it has a chance to notice and request a
                // (0, 0) size, rather than the scrolling column size.
                self.toggle_window_floating(Some(window));
                return;
            }
        }

        let tile = self
            .scrolling
            .tiles()
            .find(|tile| tile.window().id() == window)
            .unwrap();
        let was_normal = tile.window().pending_sizing_mode().is_normal();

        self.scrolling.set_maximized(window, maximize);

        // When going from normal to maximized, remember if we should unmaximize to floating. The
        // layout engine decides whether the window goes maximized, and does so after this.
        let tile = self
            .scrolling
            .tiles_mut()
            .find(|tile| tile.window().id() == window)
            .unwrap();
        if was_normal && maximize {
            tile.restore_to_floating = restore_to_floating;
        }
    }

    pub fn toggle_maximized(&mut self, window: &W::Id) {
        // A window can be both maximized and fullscreen, and then its sizing mode only says
        // fullscreen, so ask the tiling layout. In the floating layout, windows cannot be
        // maximized.
        let current = self.scrolling.is_pending_maximized(window);
        self.set_maximized(window, !current);
    }

    pub fn toggle_window_floating(&mut self, id: Option<&W::Id>) {
        let active_id = self.active_window().map(|win| win.id().clone());
        let target_is_active = id.is_none_or(|id| Some(id) == active_id.as_ref());
        let Some(id) = id.cloned().or(active_id) else {
            return;
        };

        let (tile, render_pos, _) = self
            .tiles_with_render_positions()
            .find(|(tile, _, _)| *tile.window().id() == id)
            .unwrap();
        let render_rect = Rectangle::new(render_pos, tile.tile_size());

        if self.floating.has_window(&id) {
            let removed = self.floating.remove_tile(&id);
            self.scrolling
                .add_tile_from_floating(removed.tile, target_is_active, render_rect);
            // The layout engine places the tile after this, so it animates from where it was then.
            self.scrolling.set_pending_move(
                &id,
                PendingMove {
                    from: render_pos,
                    config: None,
                    between_workspaces: false,
                },
            );
            if target_is_active {
                self.floating_is_active = FloatingActive::No;
            }
        } else {
            let mut removed = self.scrolling.remove_tile_for_floating(&id);
            removed.tile.stop_move_animations();

            // Come up with a default floating position close to the tile position.
            let stored_or_default = self.floating.stored_or_default_tile_pos(&removed.tile);
            if stored_or_default.is_none() {
                let pos = render_pos + Point::from((50., 50.));
                let size = removed.tile.tile_size();
                let pos = self.floating.clamp_within_working_area(pos, size);
                let pos = self.floating.logical_to_size_frac(pos);
                removed.tile.floating_pos = Some(pos);
            }

            self.floating.add_tile(removed.tile, target_is_active);
            if target_is_active {
                self.floating_is_active = FloatingActive::Yes;
            }

            let (tile, new_render_pos) = self
                .tiles_with_render_positions_mut(false)
                .find(|(tile, _)| *tile.window().id() == id)
                .unwrap();

            tile.animate_move_from(render_pos - new_render_pos);
        }
    }

    pub fn set_window_floating(&mut self, id: Option<&W::Id>, floating: bool) {
        if id.map_or(self.floating_is_active.get(), |id| {
            self.floating.has_window(id)
        }) == floating
        {
            return;
        }

        self.toggle_window_floating(id);
    }

    pub fn focus_floating(&mut self) {
        if !self.floating_is_active.get() {
            self.switch_focus_floating_tiling();
        }
    }

    /// Runs one of bento's floating actions on the floating layout. With `focus`, the floating
    /// layout gets focus first.
    pub fn floating_action(&mut self, focus: bool, f: impl FnOnce(&mut FloatingSpace<W>)) {
        if focus {
            self.focus_floating();
        }
        f(&mut self.floating);
    }

    pub fn focus_tiling(&mut self) {
        if self.floating_is_active.get() {
            self.switch_focus_floating_tiling();
        }
    }

    pub fn switch_focus_floating_tiling(&mut self) {
        if self.floating.is_empty() {
            // If floating is empty, keep focus on scrolling.
            return;
        } else if self.scrolling.is_empty() {
            // If floating isn't empty but scrolling is, keep focus on floating.
            return;
        }

        self.floating_is_active = if self.floating_is_active.get() {
            FloatingActive::No
        } else {
            FloatingActive::Yes
        };
    }

    pub fn move_floating_window(
        &mut self,
        id: Option<&W::Id>,
        x: PositionChange,
        y: PositionChange,
        animate: bool,
    ) {
        if id.map_or(self.floating_is_active.get(), |id| {
            self.floating.has_window(id)
        }) {
            self.floating.move_window(id, x, y, animate);
        } else {
            // If the target tile isn't floating, set its stored floating position.
            let tile = if let Some(id) = id {
                self.scrolling
                    .tiles_mut()
                    .find(|tile| tile.window().id() == id)
                    .unwrap()
            } else if let Some(tile) = self.scrolling.active_tile_mut() {
                tile
            } else {
                return;
            };

            let pos = self.floating.stored_or_default_tile_pos(tile);

            // If there's no stored floating position, we can only set both components at once, not
            // adjust.
            let pos = pos.or_else(|| {
                (matches!(
                    x,
                    PositionChange::SetFixed(_) | PositionChange::SetProportion(_)
                ) && matches!(
                    y,
                    PositionChange::SetFixed(_) | PositionChange::SetProportion(_)
                ))
                .then_some(Point::default())
            });

            let Some(mut pos) = pos else {
                return;
            };

            let working_area = self.floating.working_area();
            let available_width = working_area.size.w;
            let available_height = working_area.size.h;
            let working_area_loc = working_area.loc;

            const MAX_F: f64 = 10000.;

            match x {
                PositionChange::SetFixed(x) => pos.x = x + working_area_loc.x,
                PositionChange::SetProportion(prop) => {
                    let prop = (prop / 100.).clamp(0., MAX_F);
                    pos.x = available_width * prop + working_area_loc.x;
                }
                PositionChange::AdjustFixed(x) => pos.x += x,
                PositionChange::AdjustProportion(prop) => {
                    let current_prop = (pos.x - working_area_loc.x) / available_width.max(1.);
                    let prop = (current_prop + prop / 100.).clamp(0., MAX_F);
                    pos.x = available_width * prop + working_area_loc.x;
                }
            }
            match y {
                PositionChange::SetFixed(y) => pos.y = y + working_area_loc.y,
                PositionChange::SetProportion(prop) => {
                    let prop = (prop / 100.).clamp(0., MAX_F);
                    pos.y = available_height * prop + working_area_loc.y;
                }
                PositionChange::AdjustFixed(y) => pos.y += y,
                PositionChange::AdjustProportion(prop) => {
                    let current_prop = (pos.y - working_area_loc.y) / available_height.max(1.);
                    let prop = (current_prop + prop / 100.).clamp(0., MAX_F);
                    pos.y = available_height * prop + working_area_loc.y;
                }
            }

            let pos = self.floating.logical_to_size_frac(pos);
            tile.floating_pos = Some(pos);
        }
    }

    pub fn has_windows(&self) -> bool {
        self.windows().next().is_some()
    }

    pub fn has_window(&self, window: &W::Id) -> bool {
        self.windows().any(|win| win.id() == window)
    }

    pub fn find_wl_surface(&self, wl_surface: &WlSurface) -> Option<&W> {
        self.windows().find(|win| win.is_wl_surface(wl_surface))
    }

    pub fn find_wl_surface_mut(&mut self, wl_surface: &WlSurface) -> Option<&mut W> {
        self.windows_mut().find(|win| win.is_wl_surface(wl_surface))
    }

    pub fn tiles_with_render_positions(
        &self,
    ) -> impl Iterator<Item = (&Tile<W>, Point<f64, Logical>, bool)> {
        let scrolling = self.scrolling.tiles_with_render_positions();

        let floating = self.floating.tiles_with_render_positions();
        let visible = self.is_floating_visible();
        let floating = floating.map(move |(tile, pos)| (tile, pos, visible));

        floating.chain(scrolling)
    }

    pub fn tiles_with_render_positions_mut(
        &mut self,
        round: bool,
    ) -> impl Iterator<Item = (&mut Tile<W>, Point<f64, Logical>)> {
        let scrolling = self.scrolling.tiles_with_render_positions_mut(round);
        let floating = self.floating.tiles_with_render_positions_mut(round);
        floating.chain(scrolling)
    }

    pub fn tiles_with_ipc_layouts(&self) -> impl Iterator<Item = (&Tile<W>, WindowLayout)> {
        let scrolling = self.scrolling.tiles_with_ipc_layouts();
        let floating = self.floating.tiles_with_ipc_layouts();
        floating.chain(scrolling)
    }

    pub fn active_window_visual_rectangle(&self) -> Option<Rectangle<f64, Logical>> {
        if self.floating_is_active.get() {
            self.floating.active_window_visual_rectangle()
        } else {
            self.scrolling.active_window_visual_rectangle()
        }
    }

    pub fn popup_target_rect(
        &self,
        window: &W::Id,
        overflow: bool,
    ) -> Option<Rectangle<f64, Logical>> {
        if self.floating.has_window(window) {
            self.floating.popup_target_rect(window)
        } else {
            self.scrolling.popup_target_rect(window, overflow)
        }
    }

    pub fn render_scrolling<R: NiriRenderer>(
        &self,
        ctx: RenderCtx<R>,
        xray_pos: XrayPos,
        focus_ring: bool,
        layer: RenderLayer,
        push: &mut dyn FnMut(WorkspaceRenderElement<R>),
    ) {
        let scrolling_focus_ring = focus_ring && !self.floating_is_active();
        self.scrolling
            .render(ctx, xray_pos, scrolling_focus_ring, layer, &mut |elem| {
                push(elem.into())
            });
    }

    pub fn render_floating<R: NiriRenderer>(
        &self,
        ctx: RenderCtx<R>,
        xray_pos: XrayPos,
        focus_ring: bool,
        layer: RenderLayer,
        push: &mut dyn FnMut(WorkspaceRenderElement<R>),
    ) {
        if !self.is_floating_visible() && layer.is_normal() {
            return;
        }

        let view_rect = Rectangle::from_size(self.view_size);
        let floating_focus_ring = focus_ring && self.floating_is_active();
        self.floating.render(
            ctx,
            xray_pos,
            view_rect,
            floating_focus_ring,
            layer,
            &mut |elem| push(elem.into()),
        );
    }

    pub fn render_shadow<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        push: &mut dyn FnMut(ShadowRenderElement),
    ) {
        self.shadow.render(renderer, Point::from((0., 0.)), push);
    }

    /// The image to draw behind this workspace instead of its background color, as the config
    /// writes it.
    pub fn background_image(&self) -> Option<&str> {
        self.options.layout.background_image.as_deref()
    }

    pub fn render_background(&self) -> SolidColorRenderElement {
        SolidColorRenderElement::from_buffer(
            &self.background_buffer,
            Point::new(0., 0.),
            1.,
            Kind::Unspecified,
        )
    }

    pub fn fullscreen_window_above_top_layer(&self) -> Option<&W> {
        self.scrolling.fullscreen_window_above_top_layer()
    }

    pub fn is_floating_visible(&self) -> bool {
        // If the focus is on a fullscreen scrolling window, hide the floating windows.
        matches!(
            self.floating_is_active,
            FloatingActive::Yes | FloatingActive::NoButRaised
        ) || self.fullscreen_window_above_top_layer().is_none()
    }

    pub fn store_unmap_snapshot_if_empty(
        &mut self,
        renderer: &mut GlesRenderer,
        xray: Option<&mut Xray>,
        xray_has_blocked_out_layers: bool,
        xray_pos: XrayPos,
        window: &W::Id,
    ) {
        let view_size = self.view_size();
        for (tile, tile_pos) in self.tiles_with_render_positions_mut(false) {
            if tile.window().id() == window {
                let view_pos = Point::from((-tile_pos.x, -tile_pos.y));
                let view_rect = Rectangle::new(view_pos, view_size);
                tile.update_render_elements(false, view_rect);
                let xray_pos = xray_pos.offset(tile_pos);
                tile.store_unmap_snapshot_if_empty(
                    renderer,
                    xray,
                    xray_has_blocked_out_layers,
                    xray_pos,
                );
                return;
            }
        }
    }

    pub fn clear_unmap_snapshot(&mut self, window: &W::Id) {
        for tile in self.tiles_mut() {
            if tile.window().id() == window {
                let _ = tile.take_unmap_snapshot();
                return;
            }
        }
    }

    pub fn start_close_animation_for_window(
        &mut self,
        renderer: &mut GlesRenderer,
        window: &W::Id,
        blocker: TransactionBlocker,
    ) {
        if self.floating.has_window(window) {
            self.floating
                .start_close_animation_for_window(renderer, window, blocker);
        } else {
            self.scrolling
                .start_close_animation_for_window(renderer, window, blocker);
        }
    }

    pub fn start_close_animation_for_tile(
        &mut self,
        renderer: &mut GlesRenderer,
        snapshot: TileRenderSnapshot,
        tile_size: Size<f64, Logical>,
        tile_pos: Point<f64, Logical>,
        blocker: TransactionBlocker,
    ) {
        self.floating
            .start_close_animation_for_tile(renderer, snapshot, tile_size, tile_pos, blocker);
    }

    pub fn start_open_animation(&mut self, id: &W::Id) -> bool {
        self.scrolling.start_open_animation(id) || self.floating.start_open_animation(id)
    }

    pub fn window_under(&self, pos: Point<f64, Logical>) -> Option<(&W, HitType)> {
        // This logic is consistent with tiles_with_render_positions().
        if self.is_floating_visible() {
            if let Some(rv) = self
                .floating
                .tiles_with_render_positions()
                .find_map(|(tile, tile_pos)| HitType::hit_tile(tile, tile_pos, pos))
            {
                return Some(rv);
            }
        }

        self.scrolling.window_under(pos)
    }

    /// The interactive node that the tiling layout drew under `pos`, with the pointer's position
    /// on the canvas.
    pub fn node_under(&self, pos: Point<f64, Logical>) -> Option<(String, Point<f64, Logical>)> {
        // Floating windows are above what the layout draws.
        if self.is_floating_visible()
            && self
                .floating
                .tiles_with_render_positions()
                .any(|(tile, tile_pos)| HitType::hit_tile(tile, tile_pos, pos).is_some())
        {
            return None;
        }

        self.scrolling.node_under(pos)
    }

    pub fn resize_edges_under(&self, pos: Point<f64, Logical>) -> Option<ResizeEdge> {
        self.tiles_with_render_positions()
            .find_map(|(tile, tile_pos, visible)| {
                // This logic should be consistent with window_under() in when it returns Some vs.
                // None.
                if !visible {
                    return None;
                }

                let pos_within_tile = pos - tile_pos;

                if tile.hit(pos_within_tile).is_some() {
                    let size = tile.tile_size().to_f64();

                    let mut edges = ResizeEdge::empty();
                    if pos_within_tile.x < size.w / 3. {
                        edges |= ResizeEdge::LEFT;
                    } else if 2. * size.w / 3. < pos_within_tile.x {
                        edges |= ResizeEdge::RIGHT;
                    }
                    if pos_within_tile.y < size.h / 3. {
                        edges |= ResizeEdge::TOP;
                    } else if 2. * size.h / 3. < pos_within_tile.y {
                        edges |= ResizeEdge::BOTTOM;
                    }
                    return Some(edges);
                }

                None
            })
    }

    pub fn descendants_added(&mut self, id: &W::Id) -> bool {
        self.floating.descendants_added(id)
    }

    pub fn update_window(&mut self, window: &W::Id, serial: Option<Serial>) {
        if !self.floating.update_window(window, serial) {
            self.scrolling.update_window(window, serial);
        }
    }

    pub fn refresh(&mut self, is_active: bool, is_focused: bool) {
        self.scrolling
            .refresh(is_active && !self.floating_is_active.get(), is_focused);
        self.floating
            .refresh(is_active && self.floating_is_active.get(), is_focused);
    }

    pub fn scroll_amount_to_activate(&self, window: &W::Id) -> f64 {
        if self.floating.has_window(window) {
            return 0.;
        }

        self.scrolling.scroll_amount_to_activate(window)
    }

    pub fn is_urgent(&self) -> bool {
        self.windows().any(|win| win.is_urgent())
    }

    pub fn activate_window(&mut self, window: &W::Id) -> bool {
        if self.floating.activate_window(window) {
            self.floating_is_active = FloatingActive::Yes;
            true
        } else if self.scrolling.activate_window(window) {
            self.floating_is_active = FloatingActive::No;
            true
        } else {
            false
        }
    }

    pub fn activate_window_without_raising(&mut self, window: &W::Id) -> bool {
        if self.floating.activate_window_without_raising(window) {
            self.floating_is_active = FloatingActive::Yes;
            true
        } else if self.scrolling.activate_window(window) {
            self.floating_is_active = match self.floating_is_active {
                FloatingActive::No => FloatingActive::No,
                FloatingActive::NoButRaised => FloatingActive::NoButRaised,
                FloatingActive::Yes => FloatingActive::NoButRaised,
            };
            true
        } else {
            false
        }
    }

    /// Where a tile dragged with the pointer at `pos` would go, with the tile at `tile_pos`.
    pub(super) fn scrolling_insert_position(
        &self,
        pos: Point<f64, Logical>,
        tile: &Tile<W>,
        tile_pos: Point<f64, Logical>,
    ) -> InsertPosition {
        self.scrolling.insert_position(pos, tile, tile_pos)
    }

    /// Makes a dropped window go where it was dropped when it's added next.
    pub(super) fn set_tiling_drop(&mut self, window: W::Id, drop: TiledDrop) {
        self.scrolling.set_drop(window, drop);
    }

    pub fn view_offset_gesture_begin(&mut self, is_touchpad: bool) {
        self.scrolling.view_offset_gesture_begin(is_touchpad);
    }

    pub fn view_offset_gesture_update(
        &mut self,
        delta_x: f64,
        timestamp: Duration,
        is_touchpad: bool,
    ) -> Option<bool> {
        self.scrolling
            .view_offset_gesture_update(delta_x, timestamp, is_touchpad)
    }

    pub fn view_offset_gesture_end(&mut self, is_touchpad: Option<bool>) -> bool {
        self.scrolling.view_offset_gesture_end(is_touchpad)
    }

    pub fn dnd_scroll_gesture_begin(&mut self) {
        self.scrolling.dnd_scroll_gesture_begin();
    }

    pub fn dnd_scroll_gesture_scroll(&mut self, pos: Point<f64, Logical>, speed: f64) -> bool {
        let config = &self.options.gestures.dnd_edge_view_scroll;
        let trigger_width = config.trigger_width;

        // This working area intentionally does not include extra struts from Options.
        let x = pos.x - self.working_area.loc.x;
        let width = self.working_area.size.w;

        let x = x.clamp(0., width);
        let trigger_width = trigger_width.clamp(0., width / 2.);

        let delta = if x < trigger_width {
            -(trigger_width - x)
        } else if width - x < trigger_width {
            trigger_width - (width - x)
        } else {
            0.
        };

        let delta = if trigger_width < 0.01 {
            // Sanity check for trigger-width 0 or small window sizes.
            0.
        } else {
            // Normalize to [0, 1].
            delta / trigger_width
        };
        let delta = delta * speed;

        self.scrolling.dnd_scroll_gesture_scroll(delta)
    }

    pub fn dnd_scroll_gesture_end(&mut self) {
        self.scrolling.dnd_scroll_gesture_end();
    }

    /// Starts an interactive resize of a floating window. The tiling layout engine is asked about
    /// tiled windows, see `Layout::interactive_resize_begin`.
    pub fn floating_interactive_resize_begin(&mut self, window: W::Id, edges: ResizeEdge) -> bool {
        self.floating.interactive_resize_begin(window, edges)
    }

    pub fn interactive_resize_update(
        &mut self,
        window: &W::Id,
        delta: Point<f64, Logical>,
    ) -> bool {
        if self.floating.has_window(window) {
            self.floating.interactive_resize_update(window, delta)
        } else {
            self.scrolling.interactive_resize_update(window, delta)
        }
    }

    pub fn interactive_resize_end(&mut self, window: Option<&W::Id>) {
        if let Some(window) = window {
            if self.floating.has_window(window) {
                self.floating.interactive_resize_end(Some(window));
            } else {
                self.scrolling.interactive_resize_end(Some(window));
            }
        } else {
            self.floating.interactive_resize_end(None);
            self.scrolling.interactive_resize_end(None);
        }
    }

    pub fn floating_is_active(&self) -> bool {
        self.floating_is_active.get()
    }

    pub fn floating_logical_to_size_frac(
        &self,
        logical_pos: Point<f64, Logical>,
    ) -> Point<f64, SizeFrac> {
        self.floating.logical_to_size_frac(logical_pos)
    }

    pub fn working_area(&self) -> Rectangle<f64, Logical> {
        self.working_area
    }

    pub fn layout_config(&self) -> Option<&niri_config::LayoutPart> {
        self.layout_config.as_ref()
    }

    pub fn scrolling(&self) -> &TilingSpace<W> {
        &self.scrolling
    }

    pub fn scrolling_mut(&mut self) -> &mut TilingSpace<W> {
        &mut self.scrolling
    }

    pub fn floating(&self) -> &FloatingSpace<W> {
        &self.floating
    }

    #[cfg(test)]
    pub fn verify_invariants(&self, move_win_id: Option<&W::Id>) {
        use approx::assert_abs_diff_eq;

        let scale = self.scale.fractional_scale();
        assert!(scale > 0.);
        assert!(scale.is_finite());

        let options = Options::clone(&self.base_options)
            .with_merged_layout(self.layout_config.as_ref())
            .adjusted_for_scale(scale);
        assert_eq!(
            &*self.options, &options,
            "options must be base options adjusted for scale"
        );

        assert!(self.view_size.w > 0.);
        assert!(self.view_size.h > 0.);

        assert_eq!(self.background_buffer.size(), self.view_size);
        assert_eq!(
            self.background_buffer.color().components(),
            options.layout.background_color.to_array_unpremul(),
        );

        assert_eq!(self.view_size, self.scrolling.view_size());
        assert_eq!(self.working_area, self.scrolling.parent_area());
        assert_eq!(&self.clock, self.scrolling.clock());
        assert!(Rc::ptr_eq(&self.options, self.scrolling.options()));
        self.scrolling.verify_invariants();

        assert_eq!(self.view_size, self.floating.view_size());
        assert_eq!(self.working_area, self.floating.working_area());
        assert_eq!(&self.clock, self.floating.clock());
        assert!(Rc::ptr_eq(&self.options, self.floating.options()));
        self.floating.verify_invariants();

        if self.floating.is_empty() {
            assert!(
                !self.floating_is_active.get(),
                "when floating is empty it must never be active"
            );
        } else if self.scrolling.is_empty() {
            assert!(
                self.floating_is_active.get(),
                "when scrolling is empty but floating isn't, floating should be active"
            );
        }

        for (tile, tile_pos, visible) in self.tiles_with_render_positions() {
            if Some(tile.window().id()) != move_win_id {
                assert_eq!(tile.interactive_move_offset, Point::from((0., 0.)));
            }

            let rounded_pos = tile_pos.to_physical_precise_round(scale).to_logical(scale);

            // Tile positions must be rounded to physical pixels.
            assert_abs_diff_eq!(tile_pos.x, rounded_pos.x, epsilon = 1e-5);
            assert_abs_diff_eq!(tile_pos.y, rounded_pos.y, epsilon = 1e-5);

            if let Some(alpha) = &tile.alpha_animation {
                let anim = &alpha.anim;
                if visible {
                    assert_eq!(anim.to(), 1., "visible tiles can animate alpha only to 1");
                }

                assert!(
                    !alpha.hold_after_done,
                    "tiles in the layout cannot have held alpha animation"
                );
            }
        }
    }
}

pub(super) fn compute_working_area(output: &Output) -> Rectangle<f64, Logical> {
    layer_map_for_output(output).non_exclusive_zone().to_f64()
}

fn compute_workspace_shadow_config(
    config: niri_config::WorkspaceShadow,
    view_size: Size<f64, Logical>,
) -> niri_config::Shadow {
    // Gaps between workspaces are a multiple of the view height, so shadow settings should also be
    // normalized to the view height to prevent them from overlapping on lower resolutions.
    let norm = view_size.h / 1080.;

    let mut config = niri_config::Shadow::from(config);
    config.softness *= norm;
    config.spread *= norm;
    config.offset.x.0 *= norm;
    config.offset.y.0 *= norm;

    config
}
