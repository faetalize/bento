//! Reads bento's objects out of the compositor.
//!
//! This is the part of the socket that looks inside the layout, so it's the only one that has to
//! follow when the layout changes.

use std::collections::BTreeMap;

use bento_ipc::{
    Cast, CastKind, CastTarget, Group, KeyboardInteractivity, KeyboardLayouts, Layer, LayerSurface,
    Mode, Monitor, Point, Rect, Size, SizingMode, Window, Workspace,
};
use smithay::desktop::layer_map_for_output;
use smithay::wayland::shell::wlr_layer::{
    KeyboardInteractivity as SmithayInteractivity, Layer as SmithayLayer,
};

use crate::handlers::image_copy_capture;
use crate::layout::{LayoutElement as _, SizingMode as LayoutSizingMode};
use crate::niri::State;
use crate::utils::with_toplevel_role;

/// The workspaces, on every monitor.
pub fn workspaces(state: &State) -> BTreeMap<u64, Workspace> {
    let layout = &state.niri.layout;

    layout
        .workspaces()
        .map(|(mon, index, ws)| {
            let id = ws.id().get();
            let workspace = Workspace {
                id,
                name: ws.name().cloned(),
                monitor: mon.map(|mon| mon.output_name().clone()),
                index,
                windows: ws.windows().map(|win| win.id().get()).collect(),
                focused: ws.active_window().map(|win| win.id().get()),
                active: mon.is_some_and(|mon| mon.active_workspace_idx() == index),
                urgent: ws.is_urgent(),
            };
            (id, workspace)
        })
        .collect()
}

/// The connected monitors. Their workspaces come from `workspaces`.
pub fn monitors(state: &State, workspaces: &BTreeMap<u64, Workspace>) -> BTreeMap<String, Monitor> {
    let layout = &state.niri.layout;
    let active_output = layout.active_output().map(|output| output.name());
    let outputs = state.backend.ipc_outputs().lock().unwrap().clone();

    outputs
        .into_values()
        .map(|output| {
            let layout_mon = layout
                .monitors()
                .find(|mon| *mon.output_name() == output.name);

            let mut stack: Vec<_> = workspaces
                .values()
                .filter(|ws| ws.monitor.as_ref() == Some(&output.name))
                .collect();
            stack.sort_by_key(|ws| ws.index);

            let working_area = layout_mon.map(|mon| {
                let area = mon.working_area();
                Rect {
                    x: area.loc.x,
                    y: area.loc.y,
                    w: area.size.w,
                    h: area.size.h,
                }
            });

            let logical = output.logical;
            let monitor = Monitor {
                focused: active_output.as_ref() == Some(&output.name),
                make: output.make,
                model: output.model,
                serial: output.serial,
                physical_size: output.physical_size.map(|(w, h)| Size {
                    w: f64::from(w),
                    h: f64::from(h),
                }),
                modes: output
                    .modes
                    .into_iter()
                    .map(|mode| Mode {
                        width: mode.width,
                        height: mode.height,
                        refresh_rate: mode.refresh_rate,
                        preferred: mode.is_preferred,
                    })
                    .collect(),
                current_mode: output.current_mode,
                custom_mode: output.is_custom_mode,
                vrr_supported: output.vrr_supported,
                vrr_enabled: output.vrr_enabled,
                max_bpc: output.max_bpc,
                enabled: logical.is_some(),
                position: logical.map(|l| Point {
                    x: f64::from(l.x),
                    y: f64::from(l.y),
                }),
                size: logical.map(|l| Size {
                    w: f64::from(l.width),
                    h: f64::from(l.height),
                }),
                scale: logical.map(|l| l.scale),
                transform: logical.map(|l| l.transform),
                working_area,
                workspaces: stack.iter().map(|ws| ws.id).collect(),
                active_workspace: stack.iter().find(|ws| ws.active).map(|ws| ws.id),
                name: output.name,
            };
            (monitor.name.clone(), monitor)
        })
        .collect()
}

/// The open windows, with the groups they're in from `groups`.
pub fn windows(state: &State, groups: &BTreeMap<u64, Group>) -> BTreeMap<u64, Window> {
    let mut windows = BTreeMap::new();

    state
        .niri
        .layout
        .with_windows(|mapped, _, workspace, layout| {
            let id = mapped.id().get();
            let (title, app_id) = with_toplevel_role(mapped.toplevel(), |role| {
                (role.title.clone(), role.app_id.clone())
            });

            let rect = state
                .niri
                .layout
                .window_canvas_rect(&mapped.window)
                .map(|rect| Rect {
                    x: rect.loc.x,
                    y: rect.loc.y,
                    w: rect.size.w,
                    h: rect.size.h,
                });
            let mode = match mapped.sizing_mode() {
                LayoutSizingMode::Normal => SizingMode::Normal,
                LayoutSizingMode::Fullscreen => SizingMode::Fullscreen,
                LayoutSizingMode::Maximized => SizingMode::Maximized,
            };

            let window = Window {
                id,
                app_id,
                title,
                pid: mapped.credentials().map(|c| c.pid),
                workspace: workspace.map(|id| id.get()),
                rect,
                size: Size {
                    w: f64::from(layout.window_size.0),
                    h: f64::from(layout.window_size.1),
                },
                mode,
                floating: mapped.is_floating(),
                focused: mapped.is_focused(),
                urgent: mapped.is_urgent(),
                group: groups
                    .values()
                    .find(|group| group.windows.contains(&id))
                    .map(|group| group.id),
                focus_timestamp: mapped.get_focus_timestamp().map(Into::into),
            };
            windows.insert(id, window);
        });

    windows
}

/// The groups of windows.
pub fn groups(state: &State) -> BTreeMap<u64, Group> {
    state
        .niri
        .layout
        .window_groups()
        .into_iter()
        .map(|group| {
            let windows = group.windows.iter().map(|win| win.id().get()).collect();
            let active = group.active.id().get();
            let group = Group {
                id: group.id,
                windows,
                active,
            };
            (group.id, group)
        })
        .collect()
}

/// The layer-shell surfaces, on every monitor.
pub fn layers(state: &State) -> Vec<LayerSurface> {
    let mut layers = Vec::new();

    for output in state.niri.global_space.outputs() {
        let monitor = output.name();
        for surface in layer_map_for_output(output).layers() {
            let layer = match surface.layer() {
                SmithayLayer::Background => Layer::Background,
                SmithayLayer::Bottom => Layer::Bottom,
                SmithayLayer::Top => Layer::Top,
                SmithayLayer::Overlay => Layer::Overlay,
            };
            let keyboard_interactivity = match surface.cached_state().keyboard_interactivity {
                SmithayInteractivity::None => KeyboardInteractivity::None,
                SmithayInteractivity::Exclusive => KeyboardInteractivity::Exclusive,
                SmithayInteractivity::OnDemand => KeyboardInteractivity::OnDemand,
            };

            layers.push(LayerSurface {
                namespace: surface.namespace().to_owned(),
                monitor: monitor.clone(),
                layer,
                keyboard_interactivity,
            });
        }
    }

    layers
}

/// The configured keyboard layouts, with the active one.
pub fn keyboard_layouts(state: &mut State) -> KeyboardLayouts {
    let keyboard = state.niri.seat.get_keyboard().unwrap();
    keyboard.with_xkb_state(state, |context| {
        let xkb = context.xkb().lock().unwrap();
        KeyboardLayouts {
            names: xkb
                .layouts()
                .map(|layout| xkb.layout_name(layout).to_owned())
                .collect(),
            current_idx: xkb.active_layout().0 as u8,
        }
    })
}

/// The screencasts, by stream id.
pub fn casts(state: &mut State) -> BTreeMap<u64, Cast> {
    let mut casts = BTreeMap::new();

    // PipeWire screencasts.
    #[cfg(feature = "xdp-gnome-screencast")]
    {
        // Dynamic casts that are waiting for their first target.
        for pending in &state.niri.casting.pending_dynamic_casts {
            let stream_id = pending.stream_id.get();
            casts.insert(
                stream_id,
                Cast {
                    stream_id,
                    session_id: pending.session_id.get(),
                    kind: CastKind::PipeWire,
                    target: CastTarget::Nothing,
                    dynamic_target: true,
                    active: false,
                    pid: None,
                    pw_node_id: None,
                },
            );
        }

        for cast in &state.niri.casting.casts {
            let stream_id = cast.stream_id.get();
            casts.insert(
                stream_id,
                Cast {
                    stream_id,
                    session_id: cast.session_id.get(),
                    kind: CastKind::PipeWire,
                    target: cast.target.make_ipc(),
                    dynamic_target: cast.dynamic_target,
                    active: cast.is_active(),
                    pid: None,
                    pw_node_id: cast.node_id(),
                },
            );
        }
    }

    // wlr-screencopy casts. A deadline timer would clear the expired ones, but the one second
    // frame callback timer refreshes regularly, which is enough.
    state.niri.screencopy_state.clear_expired_casts();
    for queue in state.niri.screencopy_state.queues() {
        if let Some(cast_info) = queue.cast() {
            let stream_id = cast_info.stream_id.get();
            casts.insert(
                stream_id,
                Cast {
                    stream_id,
                    session_id: cast_info.session_id.get(),
                    kind: CastKind::WlrScreencopy,
                    target: CastTarget::Output {
                        name: cast_info.output_name.clone(),
                    },
                    dynamic_target: false,
                    active: true,
                    pid: queue.credentials().map(|creds| creds.pid),
                    pw_node_id: None,
                },
            );
        }
    }

    // ext-image-copy-capture casts. A session is an explicit protocol object, so the cast lasts
    // as long as it does. Dead sessions are dropped by `refresh_image_copy_capture`, which runs
    // before this.
    let output_sessions = state
        .niri
        .image_copy_sessions
        .iter()
        .map(|s| (s.session_id, s.stream_id, s.session.source(), s.credentials));
    let cursor_sessions = state
        .niri
        .image_copy_cursor_sessions
        .iter()
        .map(|s| (s.session_id, s.stream_id, s.session.source(), s.credentials));
    for (session_id, stream_id, source, credentials) in output_sessions.chain(cursor_sessions) {
        let Some(output) = image_copy_capture::source_output(&source) else {
            continue;
        };

        let stream_id = stream_id.get();
        casts.insert(
            stream_id,
            Cast {
                stream_id,
                session_id: session_id.get(),
                kind: CastKind::ExtImageCopyCapture,
                target: CastTarget::Output {
                    name: output.name(),
                },
                dynamic_target: false,
                active: true,
                pid: credentials.map(|creds| creds.pid),
                pw_node_id: None,
            },
        );
    }

    casts
}
