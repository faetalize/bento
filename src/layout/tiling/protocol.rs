//! JSON that bento and layouts exchange, see `resources/layouts/prelude/index.js`.

use serde::{Deserialize, Serialize};
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::layout::search::Scope;
use crate::layout::tile::Decorations;
use crate::utils::ResizeEdge;

/// Largest coordinate or size bento accepts from a layout, to keep arithmetic sane.
const MAX_COORD: f64 = 1e7;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn from_rect(rect: Rectangle<f64, Logical>) -> Self {
        Self {
            x: rect.loc.x,
            y: rect.loc.y,
            w: rect.size.w,
            h: rect.size.h,
        }
    }

    /// The rect as a rectangle, with coordinates kept in a sane range and a size of at least 1.
    pub fn to_rect(self) -> Rectangle<f64, Logical> {
        let coord = |v: f64| v.clamp(-MAX_COORD, MAX_COORD);
        let size = |v: f64| v.clamp(1., MAX_COORD);
        Rectangle::new(
            Point::from((coord(self.x), coord(self.y))),
            Size::from((size(self.w), size(self.h))),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Xy {
    pub x: f64,
    pub y: f64,
}

impl Xy {
    pub fn from_point(point: Point<f64, Logical>) -> Self {
        Self {
            x: point.x,
            y: point.y,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Wh {
    pub w: f64,
    pub h: f64,
}

impl Wh {
    pub fn from_size(size: Size<f64, Logical>) -> Self {
        Self {
            w: size.w,
            h: size.h,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Normal,
    Fullscreen,
    Maximized,
}

/// What bento tells the layout about a window.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub id: u64,
    pub app_id: Option<String>,
    pub title: Option<String>,
    /// Id of the workspace the window is on.
    pub workspace: u64,
    pub size: Wh,
    /// Whether the window has answered the size it was asked to take, see
    /// `LayoutElement::settled`.
    pub settled: bool,
    pub min_size: Wh,
    pub max_size: Wh,
    pub borders: Wh,
    /// Which of bento's border, focus ring and shadow the layout left on.
    pub decorations: Decorations,
    pub mode: Mode,
    pub urgent: bool,
    /// The layout's window rules that match the window, resolved.
    pub rules: serde_json::Value,
    /// What the layout stored on the window.
    pub data: serde_json::Value,
    /// Where the window is, as a hint when a layout takes over.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
}

/// What bento tells the layout about a monitor.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    pub name: String,
    pub size: Wh,
    pub scale: f64,
    pub working_area: Rect,
    /// Id of the workspace the monitor shows.
    pub active: u64,
    pub left: Option<String>,
    pub right: Option<String>,
    pub up: Option<String>,
    pub down: Option<String>,
}

/// The view of a workspace: its monitor's, or the last one it had.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewInfo {
    pub size: Wh,
    pub working_area: Rect,
    pub scale: f64,
}

/// bento's settings for a workspace, that engines may follow.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfigInfo {
    pub gaps: f64,
    pub struts: Struts,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Struts {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

/// Where a workspace's camera is going, if it's moving, or where it is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CameraInfo {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

/// What bento tells the layout about a workspace.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub id: u64,
    pub name: Option<String>,
    /// Name of the monitor the workspace is on, if there is one.
    pub monitor: Option<String>,
    /// Place in the monitor's stack, from the top.
    pub index: usize,
    pub view: ViewInfo,
    pub camera: CameraInfo,
    pub config: ConfigInfo,
    /// The engine's options for the workspace: the top-level ones with its output's and its own on
    /// top.
    pub options: serde_json::Value,
    /// Id of the window that has the workspace's focus.
    pub focused: Option<u64>,
}

/// What bento tells the layout about a group.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GroupInfo {
    pub id: u64,
    pub windows: Vec<u64>,
    pub active: u64,
}

/// What bento tells the layout about the running search.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchInfo {
    pub query: String,
    /// The windows that match the query, with how well: `(window, rank)`, the greater the better.
    pub hits: Vec<(u64, u8)>,
}

/// What changed about the session since the layout was last told, which the prelude applies
/// before it handles an event.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncInfo {
    pub monitors: Vec<serde_json::Value>,
    pub removed_monitors: Vec<String>,
    pub workspaces: Vec<serde_json::Value>,
    pub removed_workspaces: Vec<u64>,
    pub windows: Vec<serde_json::Value>,
    /// Every group, when they changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<GroupInfo>>,
    /// The running search, when its query or matches changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<SearchInfo>,
    /// Name of the monitor with keyboard focus.
    pub focused_monitor: Option<String>,
    /// The layout's options, when they changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,
    /// Whether the focused monitor changed since the layout was last told.
    #[serde(skip)]
    pub focus_changed: bool,
    /// Whether the layout can take the keyboard, when that changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyboard_available: Option<bool>,
}

impl SyncInfo {
    /// Whether there's nothing to tell.
    pub fn is_empty(&self) -> bool {
        self.monitors.is_empty()
            && self.removed_monitors.is_empty()
            && self.workspaces.is_empty()
            && self.removed_workspaces.is_empty()
            && self.windows.is_empty()
            && self.groups.is_none()
            && self.search.is_none()
            && self.options.is_none()
            && self.keyboard_available.is_none()
            && !self.focus_changed
    }

    /// Takes out the monitors and workspaces that went away, as a sync of their own.
    pub fn take_removals(&mut self) -> SyncInfo {
        SyncInfo {
            removed_monitors: std::mem::take(&mut self.removed_monitors),
            removed_workspaces: std::mem::take(&mut self.removed_workspaces),
            focused_monitor: self.focused_monitor.clone(),
            ..SyncInfo::default()
        }
    }
}

/// What a layout call changed.
#[derive(Debug, Default, Deserialize)]
pub struct Output {
    #[serde(default)]
    pub result: serde_json::Value,
    pub error: Option<String>,
    #[serde(default)]
    pub logs: Vec<String>,
    /// Window and workspace changes, in the order the layout made them.
    #[serde(default)]
    pub ops: Vec<Op>,
    #[serde(default)]
    pub windows: Vec<WindowChange>,
    /// Data the layout stored on windows.
    #[serde(default)]
    pub data: Vec<(u64, serde_json::Value)>,
    /// The window that has the focus of each workspace.
    #[serde(default)]
    pub focus: Vec<(u64, Option<u64>)>,
    #[serde(default)]
    pub cameras: Vec<CameraChange>,
    /// What the layout turned on or off around windows.
    #[serde(default)]
    pub decorations: Vec<(u64, Decorations)>,
    /// The pointer move or resize that the layout began, while handling a press.
    #[serde(default)]
    pub grab: Option<Grab>,
    /// The monitor that has keyboard focus.
    #[serde(default, rename = "monitorFocus")]
    pub monitor_focus: Option<String>,
    #[serde(default)]
    pub tree: Vec<TreeOp>,
    /// Whether the layout says that its exposed state changed.
    #[serde(default, rename = "stateChanged")]
    pub state_changed: bool,
    /// Whether the layout threw while it started.
    #[serde(default, rename = "startFailed")]
    pub start_failed: bool,
    /// Timers that the layout started.
    #[serde(default)]
    pub timers: Vec<TimerStart>,
    /// Timers that the layout cleared, which it had started in an earlier call.
    #[serde(default, rename = "clearedTimers")]
    pub cleared_timers: Vec<u64>,
    /// Whether the layout holds the keyboard, if it took it or gave it back.
    #[serde(default)]
    pub keyboard: Option<bool>,
}

/// A timer that the layout asked for: the event `timer` with its `id` comes after `ms`.
#[derive(Debug, Deserialize)]
pub struct TimerStart {
    pub id: u64,
    pub ms: u64,
}

/// A change to the workspace structure or the groups that the layout asked for.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Op {
    Show {
        workspace: u64,
        animate: bool,
    },
    MoveWorkspace {
        workspace: u64,
        monitor: String,
        index: Option<usize>,
    },
    Rename {
        workspace: u64,
        name: Option<String>,
    },
    MoveWindow {
        window: u64,
        workspace: u64,
    },
    CreateGroup {
        id: u64,
        windows: Vec<u64>,
        active: u64,
    },
    GroupAdd {
        group: u64,
        window: u64,
    },
    GroupRemove {
        group: u64,
        window: u64,
    },
    GroupActivate {
        group: u64,
        window: u64,
    },
    DissolveGroup {
        group: u64,
    },
    Search {
        scope: Scope,
    },
    /// The layout ended its search, which it was told about already.
    EndSearch,
}

/// A move or resize with the pointer that the layout asks for, from a press on one of its nodes.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Grab {
    Move { window: u64 },
    Resize { window: u64, edges: Edges },
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Edges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl Edges {
    pub fn to_resize_edge(self) -> ResizeEdge {
        let mut edges = ResizeEdge::empty();
        edges.set(ResizeEdge::LEFT, self.left);
        edges.set(ResizeEdge::RIGHT, self.right);
        edges.set(ResizeEdge::TOP, self.top);
        edges.set(ResizeEdge::BOTTOM, self.bottom);
        edges
    }
}

/// A change to a workspace's tree of drawn nodes, see `prelude/nodes.js`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum TreeOp {
    Create {
        id: String,
        kind: String,
        props: serde_json::Value,
    },
    Set {
        id: String,
        props: serde_json::Value,
    },
    Append {
        parent: String,
        child: String,
        before: Option<String>,
    },
    Remove {
        id: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct WindowChange {
    pub id: u64,
    /// Where the layout put the window, or `None` if it didn't yet.
    pub rect: Option<Rect>,
    pub visible: bool,
    pub mode: Mode,
    pub animate: bool,
}

#[derive(Debug, Deserialize)]
pub struct CameraChange {
    pub workspace: u64,
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
    pub animate: bool,
    /// Initial velocity of the animation, after a swipe.
    #[serde(default)]
    pub velocity: f64,
}

/// What bento needs to know about how a cancelable event went.
#[derive(Debug, Default, Deserialize)]
pub struct Verdict {
    /// Whether a listener prevented the event's default.
    #[serde(default)]
    pub prevented: bool,
    /// Whether any listener took the event on, for events that are refused otherwise.
    #[serde(default)]
    pub handled: bool,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_layout_starts_and_ends_searches() {
        let op = |value| serde_json::from_value::<Op>(value).unwrap();

        assert!(matches!(
            op(json!({ "op": "search", "scope": { "kind": "all" } })),
            Op::Search { scope: Scope::All }
        ));
        assert!(matches!(
            op(json!({ "op": "search", "scope": { "kind": "workspace", "id": 4 } })),
            Op::Search {
                scope: Scope::Workspace { id: 4 }
            }
        ));
        assert!(matches!(
            op(json!({ "op": "search", "scope": { "kind": "group", "id": 3 } })),
            Op::Search {
                scope: Scope::Group { id: 3 }
            }
        ));
        assert!(matches!(op(json!({ "op": "endSearch" })), Op::EndSearch));
    }

    #[test]
    fn the_layout_is_told_the_query_and_how_well_windows_match() {
        let sync = SyncInfo {
            search: Some(SearchInfo {
                query: String::from("fire"),
                hits: vec![(3, 4), (1, 3)],
            }),
            ..SyncInfo::default()
        };
        assert!(!sync.is_empty());
        assert_eq!(
            serde_json::to_value(&sync).unwrap()["search"],
            json!({ "query": "fire", "hits": [[3, 4], [1, 3]] })
        );

        // A sync without one leaves it out.
        assert!(serde_json::to_value(SyncInfo::default()).unwrap()["search"].is_null());
    }
}
