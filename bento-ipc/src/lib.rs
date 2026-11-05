//! Messages for bento's private socket, and a blocking client for it.
//!
//! bento's own programs (`bento msg`, bento Settings, the shell and scripts) talk to the
//! compositor through one Unix socket, in JSON. The format belongs to bento and its programs and
//! can change at any time; it is not niri's IPC.
//!
//! The socket is at the path in the `BENTO_SOCKET` environment variable, which bento sets for the
//! programs it starts. Each message is one line of JSON, with its kind in `"type"` and camelCase
//! keys. bento answers each [`Request`] line with one [`Response`] line, in order.
//!
//! ```text
//! → {"type":"get","what":"windows"}
//! ← {"type":"windows","data":[{"id":3,"appId":"foot", ...}]}
//! → {"type":"action","name":"focus-workspace","args":[3],"properties":{}}
//! ← {"type":"handled"}
//! → {"type":"get","what":"nothing"}
//! ← {"type":"error","data":"error parsing request: unknown variant ..."}
//! ```
//!
//! A [`Request::Subscribe`] turns the connection into a stream: bento answers it, then writes
//! [`Event`] lines until the connection closes, and reads no more requests. A client that wants
//! requests and events at once opens two connections. A new subscriber first gets the current
//! state as events (`monitorAdded`, `windowAdded`, ...), so it doesn't need to `get` anything.
//!
//! Requests are processed one at a time with time passing between them: a window can open between
//! a `get windows` and a `get workspaces`. Events are sent in the order things changed, and aren't
//! always consistent with each other: a `windowAdded` can name a workspace whose `workspaceAdded`
//! comes later in the same burst.
//!
//! The objects mirror what layout engines see (`resources/layouts/bento-layout.d.ts`), so code for
//! the shell and code for engines share one vocabulary.
#![warn(missing_docs)]

pub use niri_ipc::{
    LayoutEngine, LayoutEngineAction, OutputAction, PickedColor, Timestamp, Transform,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod socket;

/// A point in logical pixels.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
pub struct Point {
    /// Horizontal position.
    pub x: f64,
    /// Vertical position.
    pub y: f64,
}

/// A size in logical pixels.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
pub struct Size {
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
}

/// A rectangle in logical pixels.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
}

/// A message from a client to bento.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Request {
    /// Reads something.
    Get {
        /// What to read.
        what: Get,
    },
    /// Runs an action, native or one of the running engine's, written like in a bind:
    /// `{"type":"action","name":"move-window-to-workspace","args":[3],"properties":{"focus":false}}`.
    /// It's parsed the way a bind in the config is, so the same things are invalid.
    ///
    /// Screenshot actions also take a `path` property, which must be absolute.
    Action {
        /// The action's name.
        name: String,
        /// Its arguments, in order. Strings, numbers and booleans.
        #[serde(default)]
        args: Vec<Value>,
        /// Its named properties, like `focus=false` in a bind.
        #[serde(default)]
        properties: serde_json::Map<String, Value>,
        /// Runs it on this window instead of the focused one. Only actions that act on a window
        /// take it.
        #[serde(default)]
        window: Option<u64>,
        /// Runs it on this workspace instead of the focused one. Only actions that act on a
        /// workspace take it.
        #[serde(default)]
        workspace: Option<u64>,
    },
    /// Changes a monitor's configuration temporarily.
    ///
    /// The change isn't saved into the config file. If the monitor's configuration changes in the
    /// config file afterwards, the temporary changes are forgotten.
    Output {
        /// The monitor's connector name.
        output: String,
        /// The change.
        action: OutputAction,
    },
    /// Lets the user pick a window with the pointer, and answers with it.
    PickWindow,
    /// Lets the user pick a colour from the screen with the pointer, and answers with it.
    PickColor,
    /// Reloads the config file.
    LoadConfigFile {
        /// A different config file to load, by absolute path. The current one if unset.
        #[serde(default)]
        path: Option<String>,
    },
    /// Focuses a window.
    FocusWindow {
        /// The window's id.
        id: u64,
    },
    /// Sets whether a window asks for attention.
    SetWindowUrgent {
        /// The window's id.
        id: u64,
        /// The new state, or `null` to toggle it.
        #[serde(default)]
        urgent: Option<bool>,
    },
    /// Stops a PipeWire screencast. wlr-screencopy screencasts can't be stopped.
    StopCast {
        /// The session to stop, with all its streams.
        session_id: u64,
    },
    /// Turns this connection into a stream of events.
    Subscribe {
        /// The kinds of events to get, or all of them if unset.
        #[serde(default)]
        events: Option<Vec<EventKind>>,
    },
}

/// Something that [`Request::Get`] reads.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Get {
    /// bento's version.
    Version,
    /// The connected monitors.
    Monitors,
    /// The workspaces, on every monitor.
    Workspaces,
    /// The open windows.
    Windows,
    /// The groups of windows.
    Groups,
    /// The layer-shell surfaces.
    Layers,
    /// The focused monitor and the focused window.
    Focused,
    /// The configured keyboard layouts.
    KeyboardLayouts,
    /// Whether the overview is open.
    Overview,
    /// The screencasts.
    Casts,
    /// The running engine: its name, actions, options and window rules.
    Engine,
    /// The state the running engine exposes. Its shape is the engine's own.
    EngineState,
    /// bento's own actions, with their arguments.
    Actions,
}

/// bento's answer to a [`Request`].
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum Response {
    /// The request was done and has nothing to answer with.
    Handled,
    /// The request failed.
    Error(String),
    /// bento's version.
    Version(String),
    /// The connected monitors.
    Monitors(Vec<Monitor>),
    /// The workspaces.
    Workspaces(Vec<Workspace>),
    /// The open windows.
    Windows(Vec<Window>),
    /// The groups of windows.
    Groups(Vec<Group>),
    /// The layer-shell surfaces.
    Layers(Vec<LayerSurface>),
    /// The focused monitor and window.
    Focused(Focused),
    /// The keyboard layouts.
    KeyboardLayouts(KeyboardLayouts),
    /// The overview.
    Overview(Overview),
    /// The screencasts.
    Casts(Vec<Cast>),
    /// The running engine.
    Engine(LayoutEngine),
    /// The running engine's state.
    EngineState(Value),
    /// bento's own actions.
    Actions(Vec<LayoutEngineAction>),
    /// The window that was picked, if the user picked one.
    PickedWindow(Option<Window>),
    /// The colour that was picked, if the user picked one.
    PickedColor(Option<PickedColor>),
    /// What became of an [`Request::Output`].
    OutputConfigChanged(OutputConfigChanged),
}

impl Response {
    /// Splits an [`Response::Error`] from the answers that worked.
    pub fn into_result(self) -> Result<Self, String> {
        match self {
            Self::Error(message) => Err(message),
            response => Ok(response),
        }
    }
}

/// What became of an [`Request::Output`].
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OutputConfigChanged {
    /// The monitor was connected and the change was applied.
    Applied,
    /// The monitor isn't connected, so the change applies when it is.
    OutputWasMissing,
}

/// A connected monitor, enabled or not.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Monitor {
    /// Connector name, like `"DP-2"`.
    pub name: String,
    /// The manufacturer.
    pub make: String,
    /// The model.
    pub model: String,
    /// The serial number, if known.
    pub serial: Option<String>,
    /// Physical size in millimetres, if known.
    pub physical_size: Option<Size>,
    /// The modes the monitor offers.
    pub modes: Vec<Mode>,
    /// The index in `modes` of the current one, or `null` if the monitor is off.
    pub current_mode: Option<usize>,
    /// Whether the current mode is a custom one.
    pub custom_mode: bool,
    /// Whether the monitor supports variable refresh rate.
    pub vrr_supported: bool,
    /// Whether variable refresh rate is on.
    pub vrr_enabled: bool,
    /// Maximum bits per channel, if known.
    pub max_bpc: Option<u8>,
    /// Whether it shows anything. The properties below are `null` when it doesn't.
    pub enabled: bool,
    /// Where it is among the other monitors.
    pub position: Option<Point>,
    /// Its size in logical pixels.
    pub size: Option<Size>,
    /// Its scale.
    pub scale: Option<f64>,
    /// How it's rotated, counter-clockwise.
    pub transform: Option<Transform>,
    /// Its size minus panels and other exclusive layer surfaces, in view coordinates.
    pub working_area: Option<Rect>,
    /// The ids of its vertical stack of workspaces, top to bottom.
    pub workspaces: Vec<u64>,
    /// The id of the workspace it shows.
    pub active_workspace: Option<u64>,
    /// Whether it has keyboard focus.
    pub focused: bool,
}

/// A monitor's mode.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Mode {
    /// Width in physical pixels.
    pub width: u16,
    /// Height in physical pixels.
    pub height: u16,
    /// Refresh rate in millihertz.
    pub refresh_rate: u32,
    /// Whether the monitor prefers this mode.
    pub preferred: bool,
}

/// A workspace.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    /// Its id, which stays the same while it moves between monitors and places.
    pub id: u64,
    /// Its name, if it has one.
    pub name: Option<String>,
    /// The name of the monitor it's on, if there is one.
    pub monitor: Option<String>,
    /// Its place in its monitor's stack, from 0 at the top.
    pub index: usize,
    /// The ids of the windows on it.
    pub windows: Vec<u64>,
    /// The id of the window that has focus when the workspace does.
    pub focused: Option<u64>,
    /// Whether its monitor shows it.
    pub active: bool,
    /// Whether a window on it asks for attention.
    pub urgent: bool,
}

/// How a window is sized.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SizingMode {
    /// Neither fullscreen nor maximized.
    Normal,
    /// Fullscreen.
    Fullscreen,
    /// Maximized.
    Maximized,
}

/// A toplevel window.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    /// Its id, which stays the same while it's open. Don't assume that ids only increase.
    pub id: u64,
    /// Its application id, if set.
    pub app_id: Option<String>,
    /// Its title, if set.
    pub title: Option<String>,
    /// The process that created the Wayland connection it's on, if known.
    pub pid: Option<i32>,
    /// The id of the workspace it's on, if any.
    pub workspace: Option<u64>,
    /// Where the layout engine placed it, with its borders, in logical pixels on its workspace's
    /// canvas, the same coordinates the engine uses. `null` when the engine doesn't place it, like
    /// a floating window.
    pub rect: Option<Rect>,
    /// The size its surface took, without borders.
    pub size: Size,
    /// Whether it's fullscreen or maximized.
    pub mode: SizingMode,
    /// Whether it's in the floating layer.
    pub floating: bool,
    /// Whether it has keyboard focus.
    pub focused: bool,
    /// Whether it asks for attention.
    pub urgent: bool,
    /// The id of the group it's in, if any.
    pub group: Option<u64>,
    /// When it was last focused, on the monotonic clock. It updates after a delay, so that quickly
    /// passing over windows doesn't count, which is what a most-recently-used window switcher
    /// wants.
    pub focus_timestamp: Option<Timestamp>,
}

/// Windows grouped together.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    /// Its id.
    pub id: u64,
    /// The ids of its windows.
    pub windows: Vec<u64>,
    /// The id of its active window.
    pub active: u64,
}

/// A layer-shell layer, from the back to the front.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum Layer {
    /// The background layer.
    Background,
    /// The bottom layer.
    Bottom,
    /// The top layer.
    Top,
    /// The overlay layer.
    Overlay,
}

/// Whether a layer-shell surface takes keyboard focus.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum KeyboardInteractivity {
    /// It can't.
    None,
    /// It does whenever possible.
    Exclusive,
    /// It does when asked to, for example when clicked.
    OnDemand,
}

/// A layer-shell surface.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LayerSurface {
    /// The namespace its client gave it.
    pub namespace: String,
    /// The name of the monitor it's on.
    pub monitor: String,
    /// Its layer.
    pub layer: Layer,
    /// Whether it takes keyboard focus.
    pub keyboard_interactivity: KeyboardInteractivity,
}

/// The monitor and the window that have focus.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Focused {
    /// The focused monitor, if there is one.
    pub monitor: Option<Monitor>,
    /// The focused window. There is none when a layer-shell surface has focus, or the workspace
    /// is empty.
    pub window: Option<Window>,
}

/// The configured keyboard layouts.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KeyboardLayouts {
    /// The XKB names of the layouts.
    pub names: Vec<String>,
    /// The index in `names` of the active layout.
    pub current_idx: u8,
}

/// The overview.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    /// Whether it's open.
    pub is_open: bool,
}

/// A screencast.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Cast {
    /// Identifies the cast.
    pub stream_id: u64,
    /// A session can have several streams, which are casts with the same session id.
    pub session_id: u64,
    /// How the cast is made.
    pub kind: CastKind,
    /// What it shows.
    pub target: CastTarget,
    /// Whether actions like `set-dynamic-cast-window` change its target.
    pub dynamic_target: bool,
    /// Whether it's streaming frames. A paused stream, like OBS on another scene, isn't.
    pub active: bool,
    /// The process that consumes it, if known. Only wlr-screencopy and ext-image-copy-capture
    /// casts have one.
    pub pid: Option<i32>,
    /// Its PipeWire node, once it has one.
    pub pw_node_id: Option<u32>,
}

/// How a screencast is made.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CastKind {
    /// PipeWire, typically through xdg-desktop-portal-gnome.
    PipeWire,
    /// The wlr-screencopy protocol, for example from wf-recorder or xdg-desktop-portal-wlr. Only
    /// casts that use damage tracking are reported, the others count as screenshots.
    WlrScreencopy,
    /// The ext-image-copy-capture protocol, for example from wayvnc or wl-mirror. A client that
    /// captures a monitor and the cursor makes two casts with one session id.
    ExtImageCopyCapture,
}

/// What a screencast shows.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CastTarget {
    /// Nothing yet, or it was cleared.
    Nothing,
    /// A monitor.
    Output {
        /// Its name.
        name: String,
    },
    /// A window.
    Window {
        /// Its id.
        id: u64,
    },
}

/// A message from bento to a subscribed client.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Event {
    /// A monitor was connected, or the subscription started and it was there.
    MonitorAdded {
        /// The monitor.
        monitor: Monitor,
    },
    /// A monitor changed.
    MonitorChanged {
        /// The monitor as it is now.
        monitor: Monitor,
    },
    /// A monitor was disconnected.
    MonitorRemoved {
        /// Its name.
        name: String,
    },
    /// A workspace appeared, or the subscription started and it was there.
    WorkspaceAdded {
        /// The workspace.
        workspace: Workspace,
    },
    /// A workspace changed: it moved, was named, got or lost windows, was shown or hidden.
    WorkspaceChanged {
        /// The workspace as it is now.
        workspace: Workspace,
    },
    /// A workspace went away.
    WorkspaceRemoved {
        /// Its id.
        id: u64,
    },
    /// A window opened, or the subscription started and it was there.
    WindowAdded {
        /// The window.
        window: Window,
    },
    /// A window changed, which includes moving or resizing.
    WindowChanged {
        /// The window as it is now.
        window: Window,
    },
    /// A window closed.
    WindowRemoved {
        /// Its id.
        id: u64,
    },
    /// Keyboard focus moved.
    FocusChanged {
        /// The name of the focused monitor, if any.
        monitor: Option<String>,
        /// The id of the focused window, if any.
        window: Option<u64>,
    },
    /// A group appeared, or the subscription started and it was there.
    GroupAdded {
        /// The group.
        group: Group,
    },
    /// Windows joined or left a group, or its active window changed.
    GroupChanged {
        /// The group as it is now.
        group: Group,
    },
    /// A group went away.
    GroupRemoved {
        /// Its id.
        id: u64,
    },
    /// The running engine's state changed.
    EngineStateChanged {
        /// The state, as `get engineState` would answer.
        state: Value,
    },
    /// The running engine changed, was reloaded, or got other options.
    EngineChanged {
        /// The engine, as `get engine` would answer.
        engine: LayoutEngine,
    },
    /// The configured keyboard layouts changed.
    KeyboardLayoutsChanged {
        /// The layouts, with the active one.
        keyboard_layouts: KeyboardLayouts,
    },
    /// The keyboard layout switched.
    KeyboardLayoutSwitched {
        /// The index of the layout that's active now.
        index: u8,
    },
    /// The overview opened or closed.
    OverviewChanged {
        /// Whether it's open now.
        is_open: bool,
    },
    /// The screencasts changed.
    CastsChanged {
        /// All the casts, which replace the ones from before.
        casts: Vec<Cast>,
    },
    /// The config was loaded again. A new subscriber gets the last attempt.
    ConfigLoaded {
        /// Whether it failed, for example because the file didn't parse.
        failed: bool,
    },
    /// A screenshot was taken.
    ScreenshotCaptured {
        /// Where it was saved, if it was written to disk.
        path: Option<String>,
    },
    /// The touchpad was turned off or on with `toggle-touchpad`.
    TouchpadToggled {
        /// Whether it's on now. A touchpad can also be off in the config, which isn't reflected
        /// here.
        on: bool,
    },
}

/// The kind of an [`Event`], for choosing which ones to subscribe to.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum EventKind {
    MonitorAdded,
    MonitorChanged,
    MonitorRemoved,
    WorkspaceAdded,
    WorkspaceChanged,
    WorkspaceRemoved,
    WindowAdded,
    WindowChanged,
    WindowRemoved,
    FocusChanged,
    GroupAdded,
    GroupChanged,
    GroupRemoved,
    EngineStateChanged,
    EngineChanged,
    KeyboardLayoutsChanged,
    KeyboardLayoutSwitched,
    OverviewChanged,
    CastsChanged,
    ConfigLoaded,
    ScreenshotCaptured,
    TouchpadToggled,
}

impl Event {
    /// The event's kind.
    pub fn kind(&self) -> EventKind {
        match self {
            Self::MonitorAdded { .. } => EventKind::MonitorAdded,
            Self::MonitorChanged { .. } => EventKind::MonitorChanged,
            Self::MonitorRemoved { .. } => EventKind::MonitorRemoved,
            Self::WorkspaceAdded { .. } => EventKind::WorkspaceAdded,
            Self::WorkspaceChanged { .. } => EventKind::WorkspaceChanged,
            Self::WorkspaceRemoved { .. } => EventKind::WorkspaceRemoved,
            Self::WindowAdded { .. } => EventKind::WindowAdded,
            Self::WindowChanged { .. } => EventKind::WindowChanged,
            Self::WindowRemoved { .. } => EventKind::WindowRemoved,
            Self::FocusChanged { .. } => EventKind::FocusChanged,
            Self::GroupAdded { .. } => EventKind::GroupAdded,
            Self::GroupChanged { .. } => EventKind::GroupChanged,
            Self::GroupRemoved { .. } => EventKind::GroupRemoved,
            Self::EngineStateChanged { .. } => EventKind::EngineStateChanged,
            Self::EngineChanged { .. } => EventKind::EngineChanged,
            Self::KeyboardLayoutsChanged { .. } => EventKind::KeyboardLayoutsChanged,
            Self::KeyboardLayoutSwitched { .. } => EventKind::KeyboardLayoutSwitched,
            Self::OverviewChanged { .. } => EventKind::OverviewChanged,
            Self::CastsChanged { .. } => EventKind::CastsChanged,
            Self::ConfigLoaded { .. } => EventKind::ConfigLoaded,
            Self::ScreenshotCaptured { .. } => EventKind::ScreenshotCaptured,
            Self::TouchpadToggled { .. } => EventKind::TouchpadToggled,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn requests_are_tagged_and_camel_case() {
        let request: Request = serde_json::from_value(json!({
            "type": "action",
            "name": "focus-workspace",
            "args": [3],
            "properties": {},
        }))
        .unwrap();
        let Request::Action {
            name,
            args,
            window,
            workspace,
            ..
        } = request
        else {
            panic!("not an action");
        };
        assert_eq!(name, "focus-workspace");
        assert_eq!(args, [json!(3)]);
        assert_eq!((window, workspace), (None, None));

        let request: Request =
            serde_json::from_value(json!({"type": "get", "what": "engineState"})).unwrap();
        assert!(matches!(
            request,
            Request::Get {
                what: Get::EngineState
            }
        ));

        let request: Request = serde_json::from_value(json!({
            "type": "subscribe",
            "events": ["windowAdded", "keyboardLayoutSwitched"],
        }))
        .unwrap();
        let Request::Subscribe { events } = request else {
            panic!("not a subscription");
        };
        assert_eq!(
            events,
            Some(vec![
                EventKind::WindowAdded,
                EventKind::KeyboardLayoutSwitched
            ])
        );

        let request: Request =
            serde_json::from_value(json!({"type": "stopCast", "sessionId": 4})).unwrap();
        assert!(matches!(request, Request::StopCast { session_id: 4 }));
    }

    #[test]
    fn responses_carry_their_data_next_to_the_type() {
        let reply = serde_json::to_value(Response::Handled).unwrap();
        assert_eq!(reply, json!({"type": "handled"}));

        let reply = serde_json::to_value(Response::Version(String::from("1"))).unwrap();
        assert_eq!(reply, json!({"type": "version", "data": "1"}));

        let reply = serde_json::to_value(Response::Error(String::from("no"))).unwrap();
        assert_eq!(reply, json!({"type": "error", "data": "no"}));
    }

    #[test]
    fn events_are_tagged_and_camel_case() {
        let event = Event::KeyboardLayoutsChanged {
            keyboard_layouts: KeyboardLayouts {
                names: vec![String::from("us")],
                current_idx: 0,
            },
        };
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            json!({
                "type": "keyboardLayoutsChanged",
                "keyboardLayouts": {"names": ["us"], "currentIdx": 0},
            })
        );
        assert_eq!(event.kind(), EventKind::KeyboardLayoutsChanged);
    }
}
