//! The session host: how the layout engine, which runs once for the whole session, sees and
//! changes what bento owns.
//!
//! The engine sees every monitor, workspace and window as objects. [`Host`] remembers what the
//! engine was told about them, and before each event [`Layout::flush_engine`] sends what changed
//! since, as a `sync`, so that the engine's objects are current when it handles the event. Events
//! come from deep inside workspaces and their tiling spaces, which queue them with [`queue`];
//! the layout sends them at the end of its public methods, and in its refresh. What the engine
//! answers is carried out here: window placements, the order they're stacked in, the nodes it
//! draws, cameras, focus, and changes to the workspace structure, each routed to the workspace
//! or monitor it concerns.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use anyhow::Context as _;
use niri_config::{OutputName, TilingOption, WorkspaceReference};
use niri_ipc::{LayoutEngineActionArg, LayoutEngineActionArgType, LayoutEngineOptionValue};
use serde_json::{json, Value};
use smithay::utils::{Logical, Point, Rectangle};

use super::protocol::{
    CameraChange, GroupInfo, MonitorInfo, Op, Output as EngineOutput, Rect, SyncInfo, TreeOp,
    Verdict, Wh, WindowChange, Xy,
};
use super::{engine, TilingSpace};
use crate::layout::monitor::InsertWorkspace;
use crate::layout::workspace::WorkspaceId;
use crate::layout::{
    ActivateWindow, InteractiveMoveState, Layout, LayoutElement, MonitorSet, NodeHit,
};
use crate::utils::transaction::Transaction;
use crate::utils::{output_size, ResizeEdge};

/// Most rounds of sending events and what changed because of them in one flush, in case the layout
/// keeps answering what it's told.
const MAX_ROUNDS: usize = 8;

/// Calls in a row that may run out of time or fail before bento gives up on the engine and runs
/// the fallback one, so that an engine that keeps failing can't stall every frame.
const MAX_FAILURES: u32 = 3;

struct Queued {
    event: Value,
    anim: Option<niri_config::Animation>,
    /// Transaction that the windows that resize because of the event are in together with the
    /// window that caused it.
    transaction: Option<Transaction>,
}

thread_local! {
    static QUEUE: RefCell<Vec<Queued>> = const { RefCell::new(Vec::new()) };
    /// How many things that the layout did are being carried out, which it isn't told about.
    static SILENCED: Cell<u32> = const { Cell::new(0) };
}

/// Queues an event for the layout, which gets it when the layout is flushed.
pub fn queue(event: Value) {
    queue_with(event, None);
}

/// Queues an event, with the animation that windows move with because of it, instead of the
/// configured one.
pub fn queue_with(event: Value, anim: Option<niri_config::Animation>) {
    if SILENCED.get() == 0 {
        QUEUE.with_borrow_mut(|queue| {
            queue.push(Queued {
                event,
                anim,
                transaction: None,
            })
        });
    }
}

/// Queues an event, with the transaction that the windows resizing because of it share with the
/// window that caused it, like one that closes.
pub fn queue_with_transaction(event: Value, transaction: Option<Transaction>) {
    if SILENCED.get() == 0 {
        QUEUE.with_borrow_mut(|queue| {
            queue.push(Queued {
                event,
                anim: None,
                transaction,
            })
        });
    }
}

/// Runs something that carries out what the layout asked for, which the layout isn't told
/// about: events it causes aren't queued.
fn silenced<R>(f: impl FnOnce() -> R) -> R {
    SILENCED.set(SILENCED.get() + 1);
    let res = f();
    SILENCED.set(SILENCED.get() - 1);
    res
}

fn take_queue() -> Vec<Queued> {
    QUEUE.with_borrow_mut(std::mem::take)
}

/// Sends a call to the layout engine and reads its answer.
fn call(call: &Value) -> anyhow::Result<EngineOutput> {
    let out = engine::call(&call.to_string())?;
    serde_json::from_str(&out).context("error reading the layout's answer")
}

/// Asks the layout something, as it is now. It's not told what changed since it was last told, and
/// it changes nothing.
pub fn query(event: Value) -> Option<Value> {
    match call(&json!({ "event": event })) {
        Ok(out) => {
            for log in &out.logs {
                info!("layout engine: {log}");
            }
            if let Some(error) = &out.error {
                warn!("error in the layout engine: {error}");
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

/// A drag over a workspace's canvas, as the layout last heard.
#[derive(Debug)]
struct Drag {
    workspace: u64,
    last: Value,
}

/// What the layout engine was told about the session, and what it asked bento to do that needs
/// remembering.
#[derive(Debug, Default)]
pub struct Host {
    /// Engine generation that the instance was created in, or 0 if it doesn't exist.
    generation: u64,
    /// Generation in which creating the instance failed, to not retry over and over.
    failed_generation: u64,
    /// Calls in a row that ran out of time or failed, to give up on an engine that keeps
    /// failing.
    failures: u32,
    /// What the layout was last told about each monitor, workspace and window.
    monitors: HashMap<String, Value>,
    workspaces: HashMap<u64, Value>,
    windows: HashMap<u64, Value>,
    /// Every group, as the layout was last told. It hears about all of them at once, since there
    /// are few.
    groups: Option<Vec<GroupInfo>>,
    focused_monitor: Option<String>,
    options: Option<Value>,
    /// What to tell the layout about again, because it changed it and bento may not agree.
    resend_all: bool,
    resend_workspaces: HashSet<u64>,
    resend_windows: HashSet<u64>,
    /// Whether the layout's exposed state changed since it was last asked for.
    state_changed: bool,
    drag: Option<Drag>,
    /// Whether events are being sent, which carrying out what the layout answers doesn't start
    /// again.
    flushing: bool,
}

impl Host {
    /// Forgets what the layout was told, for a new instance.
    fn reset(&mut self) {
        self.monitors.clear();
        self.workspaces.clear();
        self.windows.clear();
        self.groups = None;
        self.focused_monitor = None;
        self.options = None;
        self.resend_all = false;
        self.resend_workspaces.clear();
        self.resend_windows.clear();
        self.drag = None;
    }
}

/// What the session looks like to the layout right now.
struct Snapshot {
    monitors: Vec<Value>,
    workspaces: Vec<Value>,
    windows: Vec<Value>,
    focused_monitor: Option<String>,
}

fn to_value<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// The monitors next to the one at `idx`, as names: left, right, up and down.
fn neighbours(
    rects: &[Rectangle<f64, Logical>],
    names: &[String],
    idx: usize,
) -> [Option<String>; 4] {
    let me = rects[idx];
    let others = || rects.iter().enumerate().filter(|(other, _)| *other != idx);
    let overlap_y = |r: &Rectangle<f64, Logical>| {
        r.loc.y < me.loc.y + me.size.h && me.loc.y < r.loc.y + r.size.h
    };
    let overlap_x = |r: &Rectangle<f64, Logical>| {
        r.loc.x < me.loc.x + me.size.w && me.loc.x < r.loc.x + r.size.w
    };
    let nearest = |candidates: Vec<(usize, f64)>| {
        candidates
            .into_iter()
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(other, _)| names[other].clone())
    };

    let left = others()
        .filter(|(_, r)| overlap_y(r) && r.loc.x + r.size.w <= me.loc.x)
        .map(|(other, r)| (other, me.loc.x - (r.loc.x + r.size.w)))
        .collect();
    let right = others()
        .filter(|(_, r)| overlap_y(r) && me.loc.x + me.size.w <= r.loc.x)
        .map(|(other, r)| (other, r.loc.x - (me.loc.x + me.size.w)))
        .collect();
    let up = others()
        .filter(|(_, r)| overlap_x(r) && r.loc.y + r.size.h <= me.loc.y)
        .map(|(other, r)| (other, me.loc.y - (r.loc.y + r.size.h)))
        .collect();
    let down = others()
        .filter(|(_, r)| overlap_x(r) && me.loc.y + me.size.h <= r.loc.y)
        .map(|(other, r)| (other, r.loc.y - (me.loc.y + me.size.h)))
        .collect();
    [nearest(left), nearest(right), nearest(up), nearest(down)]
}

/// A size change from an action, as the layout gets it.
fn size_change_json(change: niri_ipc::SizeChange) -> Value {
    use niri_ipc::SizeChange;

    let (op, value, unit) = match change {
        SizeChange::SetFixed(value) => ("set", f64::from(value), "px"),
        SizeChange::SetProportion(value) => ("set", value, "%"),
        SizeChange::AdjustFixed(value) => ("adjust", f64::from(value), "px"),
        SizeChange::AdjustProportion(value) => ("adjust", value, "%"),
    };
    json!({ "op": op, "value": value, "unit": unit })
}

fn option_json(arg: &TilingOption) -> Value {
    match arg {
        TilingOption::Bool(value) => json!(value),
        TilingOption::Number(value) => json!(value),
        TilingOption::String(value) => json!(value),
        TilingOption::Gradient(value) => json!(value),
    }
}

impl<W: LayoutElement> Layout<W> {
    /// What the session looks like to the layout right now.
    fn snapshot(&self, with_rects: bool) -> Snapshot {
        let layout_name = engine::name();
        let mut snapshot = Snapshot {
            monitors: Vec::new(),
            workspaces: Vec::new(),
            windows: Vec::new(),
            focused_monitor: None,
        };

        let mut add_windows = |ws: &crate::layout::workspace::Workspace<W>| {
            for info in ws.scrolling().window_infos(&layout_name) {
                let info = if with_rects {
                    info
                } else {
                    super::protocol::WindowInfo { rect: None, ..info }
                };
                snapshot.windows.push(to_value(&info));
            }
        };

        match &self.monitor_set {
            MonitorSet::Normal {
                monitors,
                active_monitor_idx,
                ..
            } => {
                let names: Vec<String> = monitors
                    .iter()
                    .map(|mon| mon.output_name().clone())
                    .collect();
                let rects: Vec<Rectangle<f64, Logical>> = monitors
                    .iter()
                    .map(|mon| {
                        let loc = mon.output().current_location().to_f64();
                        Rectangle::new(loc, output_size(mon.output()))
                    })
                    .collect();

                let mut workspaces = Vec::new();
                let mut monitor_infos = Vec::new();
                for (idx, mon) in monitors.iter().enumerate() {
                    let [left, right, up, down] = neighbours(&rects, &names, idx);
                    monitor_infos.push(to_value(&MonitorInfo {
                        name: names[idx].clone(),
                        size: Wh::from_size(mon.view_size()),
                        scale: mon.scale().fractional_scale(),
                        working_area: Rect::from_rect(mon.working_area()),
                        active: mon.workspaces[mon.active_workspace_idx].id().get(),
                        left,
                        right,
                        up,
                        down,
                    }));
                    let output_name = mon.output().user_data().get::<OutputName>();
                    for (ws_idx, ws) in mon.workspaces.iter().enumerate() {
                        let options = engine::workspace_options_json(
                            &self.options.tiling,
                            output_name,
                            ws.name().map(String::as_str),
                        );
                        let info = ws.scrolling().workspace_info(
                            ws.name(),
                            Some(names[idx].clone()),
                            ws_idx,
                            options,
                        );
                        workspaces.push(to_value(&info));
                        add_windows(ws);
                    }
                }
                snapshot.monitors = monitor_infos;
                snapshot.workspaces = workspaces;
                snapshot.focused_monitor = Some(names[*active_monitor_idx].clone());
            }
            MonitorSet::NoOutputs { workspaces } => {
                for (idx, ws) in workspaces.iter().enumerate() {
                    let options = engine::workspace_options_json(
                        &self.options.tiling,
                        None,
                        ws.name().map(String::as_str),
                    );
                    let info = ws.scrolling().workspace_info(ws.name(), None, idx, options);
                    snapshot.workspaces.push(to_value(&info));
                    add_windows(ws);
                }
            }
        }
        snapshot
    }

    /// Every group, as the layout is told.
    fn group_infos(&self) -> Vec<GroupInfo> {
        self.groups
            .iter()
            .map(|group| GroupInfo {
                id: group.id,
                windows: group.windows.clone(),
                active: group.active,
            })
            .collect()
    }

    /// What changed since the layout was last told, for the events that are about to go to it.
    fn build_sync(&mut self, queued: &[Queued]) -> SyncInfo {
        let snapshot = self.snapshot(false);
        let groups = self.group_infos();
        let host = &mut self.host;
        let mut sync = SyncInfo::default();
        let resend_all = std::mem::take(&mut host.resend_all);

        let mut seen = HashSet::new();
        for info in snapshot.monitors {
            let name = info["name"].as_str().unwrap_or_default().to_owned();
            if resend_all || host.monitors.get(&name) != Some(&info) {
                sync.monitors.push(info.clone());
            }
            seen.insert(name.clone());
            host.monitors.insert(name, info);
        }
        host.monitors.retain(|name, _| {
            let keep = seen.contains(name);
            if !keep {
                sync.removed_monitors.push(name.clone());
            }
            keep
        });

        let mut seen = HashSet::new();
        for info in snapshot.workspaces {
            let id = info["id"].as_u64().unwrap_or_default();
            if resend_all
                || host.resend_workspaces.contains(&id)
                || host.workspaces.get(&id) != Some(&info)
            {
                sync.workspaces.push(info.clone());
            }
            seen.insert(id);
            host.workspaces.insert(id, info);
        }
        host.workspaces.retain(|id, _| {
            let keep = seen.contains(id);
            if !keep {
                sync.removed_workspaces.push(*id);
            }
            keep
        });
        host.resend_workspaces.clear();

        // Windows that the layout doesn't know are added by events, and windows that are gone
        // were removed by them. The layout knows a window added by a queued event once it
        // handles the event, and gets what changed about it after that.
        // A window that a queued `windowmove` moves keeps its workspace in the sync, so that the
        // event is what moves it; the layout takes a window moving in a sync as bento refusing a
        // move that the layout made.
        let moving: HashSet<u64> = queued
            .iter()
            .filter(|queued| queued.event["type"] == "windowmove")
            .filter_map(|queued| queued.event["id"].as_u64())
            .collect();
        let mut seen = HashSet::new();
        for info in snapshot.windows {
            let id = info["id"].as_u64().unwrap_or_default();
            seen.insert(id);
            let Some(told) = host.windows.get(&id) else {
                continue;
            };
            if host.resend_windows.contains(&id) || *told != info {
                let mut sent = info.clone();
                if moving.contains(&id) {
                    sent["workspace"] = told["workspace"].clone();
                }
                sync.windows.push(sent);
                host.windows.insert(id, info);
            }
        }
        let pending: HashSet<u64> = queued
            .iter()
            .filter(|queued| queued.event["type"] == "windowadd")
            .filter_map(|queued| queued.event["window"]["id"].as_u64())
            .collect();
        host.windows
            .retain(|id, _| seen.contains(id) || pending.contains(id));
        host.resend_windows.clear();

        // A window that an event adds is known to the layout from then on, as the event says.
        for queued in queued {
            if queued.event["type"] == "windowadd" {
                if let Some(id) = queued.event["window"]["id"].as_u64() {
                    host.windows.insert(id, queued.event["window"].clone());
                }
            }
        }

        // What the layout does to groups is carried out by bento, which may not agree, so after
        // that it hears about them again.
        if resend_all || host.groups.as_ref() != Some(&groups) {
            host.groups = Some(groups.clone());
            sync.groups = Some(groups);
        }

        sync.focus_changed = host.focused_monitor != snapshot.focused_monitor;
        host.focused_monitor = snapshot.focused_monitor.clone();
        sync.focused_monitor = snapshot.focused_monitor;

        let options = engine::options_json(&self.options.tiling);
        if host.options.as_ref() != Some(&options) {
            host.options = Some(options.clone());
            sync.options = Some(options);
        }

        sync
    }

    /// Makes sure that the engine has an instance for this session, creating it if the engine was
    /// loaded or reloaded. Returns whether there's one.
    fn ensure_engine(&mut self) -> bool {
        let mut generation = engine::generation();
        if generation == 0 {
            generation = engine::ensure_loaded(&self.options.tiling);
        }
        if self.host.generation == generation {
            return true;
        }
        if self.host.failed_generation == generation {
            return false;
        }

        // The new instance starts from the session as it is.
        take_queue();
        self.host.reset();
        self.host.generation = generation;

        let snapshot = self.snapshot(true);
        let windows = snapshot.windows.clone();
        let focused = self.focused_engine_window();

        let mut sync = SyncInfo::default();
        for info in &snapshot.monitors {
            self.host.monitors.insert(
                info["name"].as_str().unwrap_or_default().to_owned(),
                info.clone(),
            );
        }
        for info in &snapshot.workspaces {
            self.host
                .workspaces
                .insert(info["id"].as_u64().unwrap_or_default(), info.clone());
        }
        for info in &windows {
            let mut info = info.clone();
            if let Some(info) = info.as_object_mut() {
                info.remove("rect");
            }
            self.host
                .windows
                .insert(info["id"].as_u64().unwrap_or_default(), info);
        }
        self.host.focused_monitor = snapshot.focused_monitor.clone();
        let options = engine::options_json(&self.options.tiling);
        self.host.options = Some(options.clone());

        let groups = self.group_infos();
        self.host.groups = Some(groups.clone());

        sync.monitors = snapshot.monitors;
        sync.workspaces = snapshot.workspaces;
        sync.groups = Some(groups);
        sync.focused_monitor = snapshot.focused_monitor;

        for ws in self.workspaces_mut() {
            ws.scrolling_mut().reset_tree();
        }

        let event = json!({
            "type": "create",
            "options": options,
            "savedState": engine::take_state(),
            "windows": windows,
            "focused": focused,
        });
        if self.send_engine(event, Some(sync), None, None).is_none() {
            self.host.generation = 0;
            // An engine that can't start runs the fallback one instead, unless that's the one
            // that can't.
            if engine::generation() == generation && engine::name() != "fallback" {
                engine::fail(String::from("the layout engine failed to start"));
            } else {
                self.host.failed_generation = generation;
            }
            return false;
        }
        true
    }

    /// Sends what's queued for the layout, and carries out what it answers.
    pub(in crate::layout) fn flush_engine(&mut self) {
        if self.host.flushing {
            return;
        }
        self.host.flushing = true;
        self.flush_rounds();
        self.host.flushing = false;
    }

    fn flush_rounds(&mut self) {
        for _ in 0..MAX_ROUNDS {
            self.prune_groups();
            if !self.ensure_engine() {
                take_queue();
                break;
            }

            let queued = take_queue();
            let sync = self.build_sync(&queued);
            if queued.is_empty() {
                if sync.is_empty() {
                    break;
                }
                // Tell the layout about what changed, like workspaces that came or went.
                self.send_engine(json!({ "type": "sync" }), Some(sync), None, None);
                continue;
            }

            // Events can name monitors and workspaces that what they tell about removed, like a
            // window moved off a workspace that went away with it, so those go away after.
            let mut sync = sync;
            let removals = sync.take_removals();
            let mut sync = Some(sync);
            for queued in queued {
                self.send_engine(queued.event, sync.take(), queued.anim, queued.transaction);
            }
            if !removals.is_empty() {
                self.send_engine(json!({ "type": "sync" }), Some(removals), None, None);
            }
        }

        // Windows that the layout didn't place go in the middle of the view.
        for ws in self.workspaces_mut() {
            ws.scrolling_mut().place_unplaced();
            ws.scrolling_mut().stack_by_tree();
        }
    }

    /// Sends an event to the layout, with what changed before it, and carries out the answer.
    /// Returns the answer's result.
    fn send_engine(
        &mut self,
        event: Value,
        sync: Option<SyncInfo>,
        anim: Option<niri_config::Animation>,
        transaction: Option<Transaction>,
    ) -> Option<Value> {
        let call_json = match sync {
            Some(sync) => json!({ "sync": sync, "event": event }),
            None => json!({ "event": event }),
        };
        match call(&call_json) {
            Ok(out) => {
                self.host.failures = 0;
                let result = out.result.clone();
                for log in &out.logs {
                    info!("layout engine: {log}");
                }
                if let Some(error) = &out.error {
                    warn!("error in the layout engine: {error}");
                }
                if out.start_failed {
                    let error = out.error.unwrap_or_default();
                    engine::fail(format!("the layout engine failed to start: {error}"));
                    self.host.generation = 0;
                    return None;
                }
                if out.state_changed {
                    self.host.state_changed = true;
                }
                silenced(|| self.apply_output(out, anim, transaction));
                Some(result)
            }
            Err(err) => {
                self.host.failures += 1;
                if self.host.failures >= MAX_FAILURES {
                    self.host.failures = 0;
                    engine::fail(format!(
                        "the layout engine failed {MAX_FAILURES} calls in a row: {err:?}"
                    ));
                } else {
                    warn!("error in the layout engine, starting it over: {err:?}");
                }
                // Create the instance again next time, from where the windows are now.
                self.host.generation = 0;
                None
            }
        }
    }

    /// Sends an event to the layout right away, after what's queued, and returns the answer's
    /// result.
    fn engine_event(&mut self, event: Value) -> Option<Value> {
        self.flush_engine();
        if self.host.generation == 0 {
            return None;
        }
        let sync = self.build_sync(&[]);
        self.host.flushing = true;
        let result = self.send_engine(event, Some(sync), None, None);
        self.host.flushing = false;
        self.flush_engine();
        result
    }

    /// Sends a cancelable event to the layout, and returns how it went.
    fn engine_verdict(&mut self, event: Value) -> Option<Verdict> {
        let result = self.engine_event(event)?;
        serde_json::from_value(result).ok()
    }

    /// Carries out what a layout call changed.
    fn apply_output(
        &mut self,
        out: EngineOutput,
        anim: Option<niri_config::Animation>,
        transaction: Option<Transaction>,
    ) {
        let layout_name = engine::name();

        if !out.ops.is_empty() {
            self.host.resend_all = true;
        }
        for op in out.ops {
            self.apply_op(op);
        }

        // Group what concerns each workspace.
        let mut changes: HashMap<u64, Vec<WindowChange>> = HashMap::new();
        for change in out.windows {
            if let Some(ws) = self.workspace_of_window(change.id) {
                changes.entry(ws).or_default().push(change);
            }
        }
        let mut cameras: HashMap<u64, CameraChange> = HashMap::new();
        for camera in out.cameras {
            self.host.resend_workspaces.insert(camera.workspace);
            cameras.insert(camera.workspace, camera);
        }
        let mut workspaces: HashSet<u64> = changes.keys().chain(cameras.keys()).copied().collect();
        for ws_id in workspaces.drain() {
            let changes = changes.remove(&ws_id).unwrap_or_default();
            let camera = cameras.remove(&ws_id);
            if let Some(ws) = self.workspace_mut(ws_id) {
                ws.scrolling_mut()
                    .apply(changes, camera.as_ref(), anim, transaction.clone());
            }
        }

        for (id, value) in out.data {
            if let Some(ws) = self
                .workspace_of_window(id)
                .and_then(|ws| self.workspace_mut(ws))
            {
                ws.scrolling_mut().set_window_data(id, value, &layout_name);
            }
        }

        let mut stacked = self.apply_tree_ops(out.tree);
        for (ws_id, window) in out.focus {
            self.host.resend_workspaces.insert(ws_id);
            if let Some(ws) = self.workspace_mut(ws_id) {
                ws.scrolling_mut().focus(window);
            }
        }
        for ws_id in stacked.drain() {
            if let Some(ws) = self.workspace_mut(ws_id) {
                ws.scrolling_mut().stack_by_tree();
            }
        }

        if let Some(name) = out.monitor_focus {
            let output = self.outputs().find(|output| output.name() == name).cloned();
            if let Some(output) = output {
                self.focus_output(&output);
            }
        }
    }

    /// Carries out a change to the workspace structure that the layout asked for.
    fn apply_op(&mut self, op: Op) {
        match op {
            Op::Show { workspace, animate } => {
                let id = WorkspaceId::specific(workspace);
                for mon in self.monitors_mut() {
                    if let Some(idx) = mon.idx_of_ws(id) {
                        mon.activate_workspace(idx);
                        if !animate {
                            mon.workspace_switch = None;
                        }
                        break;
                    }
                }
            }
            Op::MoveWorkspace {
                workspace,
                monitor,
                index,
            } => self.engine_move_workspace(workspace, &monitor, index),
            Op::Rename { workspace, name } => {
                let reference = Some(WorkspaceReference::Id(workspace));
                match name {
                    Some(name) => self.set_workspace_name(name, reference),
                    None => self.unset_workspace_name(reference),
                }
            }
            Op::MoveWindow { window, workspace } => {
                self.host.resend_windows.insert(window);
                self.engine_move_window(window, workspace);
            }
            Op::CreateGroup {
                id,
                windows,
                active,
            } => {
                self.groups.create(id, &windows, Some(active));
            }
            Op::GroupAdd { group, window } => {
                self.groups.add(group, window);
            }
            Op::GroupRemove { group, window } => {
                self.groups.remove(group, window);
            }
            Op::GroupActivate { group, window } => {
                self.groups.activate(group, window);
            }
            Op::DissolveGroup { group } => self.groups.dissolve(group),
        }
    }

    fn engine_move_workspace(&mut self, workspace: u64, monitor: &str, index: Option<usize>) {
        let id = WorkspaceId::specific(workspace);
        let MonitorSet::Normal { monitors, .. } = &mut self.monitor_set else {
            return;
        };
        let Some((from_idx, old_idx)) = monitors
            .iter()
            .enumerate()
            .find_map(|(idx, mon)| Some((idx, mon.idx_of_ws(id)?)))
        else {
            return;
        };
        let Some(to_idx) = monitors.iter().position(|mon| mon.output_name() == monitor) else {
            return;
        };

        if from_idx == to_idx {
            if let Some(index) = index {
                monitors[to_idx].move_workspace_to_idx(old_idx, index);
            }
            return;
        }

        let old_output = monitors[from_idx].output().clone();
        let new_output = monitors[to_idx].output().clone();
        self.move_workspace_to_output_by_id(old_idx, Some(old_output), &new_output);

        if let (Some(index), MonitorSet::Normal { monitors, .. }) = (index, &mut self.monitor_set) {
            if let Some(new_idx) = monitors[to_idx].idx_of_ws(id) {
                monitors[to_idx].move_workspace_to_idx(new_idx, index);
            }
        }
    }

    fn engine_move_window(&mut self, window: u64, workspace: u64) {
        let Some(id) = self.window_by_engine_id(window) else {
            return;
        };
        let target = WorkspaceId::specific(workspace);
        let MonitorSet::Normal { monitors, .. } = &self.monitor_set else {
            return;
        };
        let Some((to_mon, to_idx)) = monitors
            .iter()
            .enumerate()
            .find_map(|(idx, mon)| Some((idx, mon.idx_of_ws(target)?)))
        else {
            return;
        };
        let Some(from_mon) = monitors.iter().position(|mon| mon.has_window(&id)) else {
            return;
        };

        if from_mon == to_mon {
            self.move_to_workspace(Some(&id), to_idx, ActivateWindow::No);
        } else {
            let output = monitors[to_mon].output().clone();
            self.move_to_output(Some(&id), &output, Some(to_idx), ActivateWindow::No);
        }
    }

    /// Mutable access to a workspace by id.
    fn workspace_mut(&mut self, id: u64) -> Option<&mut crate::layout::workspace::Workspace<W>> {
        self.workspaces_mut().find(|ws| ws.id().get() == id)
    }

    /// Id of the workspace that a window of the layout is on.
    fn workspace_of_window(&self, engine_id: u64) -> Option<u64> {
        self.workspaces()
            .find(|(_, _, ws)| ws.scrolling().window_id_of(engine_id).is_some())
            .map(|(_, _, ws)| ws.id().get())
    }

    /// The window that the layout knows by `engine_id`.
    fn window_by_engine_id(&self, engine_id: u64) -> Option<W::Id> {
        self.workspaces()
            .find_map(|(_, _, ws)| ws.scrolling().window_id_of(engine_id).cloned())
    }

    /// Id of a window for the layout, if the layout has it.
    fn engine_id_of(&self, window: &W::Id) -> Option<u64> {
        self.workspaces()
            .find_map(|(_, _, ws)| ws.scrolling().engine_id_of(window))
    }

    /// The workspace that a node is in the tree of. Windows take what's drawn on them along when
    /// they move, so this looks where the nodes are now.
    fn node_workspace(&self, node: &str) -> Option<u64> {
        let number = || node.get(1..)?.parse::<u64>().ok();
        match node.chars().next()? {
            'r' => number().filter(|id| self.host.workspaces.contains_key(id)),
            'w' => self.workspace_of_window(number()?),
            _ => self
                .workspaces()
                .find(|(_, _, ws)| ws.scrolling().tree().contains(node))
                .map(|(_, _, ws)| ws.id().get()),
        }
    }

    /// Carries out the changes to the nodes that the layout draws. Returns the workspaces whose
    /// trees changed.
    fn apply_tree_ops(&mut self, ops: Vec<TreeOp>) -> HashSet<u64> {
        let mut touched = HashSet::new();
        let mut created: HashMap<String, TreeOp> = HashMap::new();

        for op in ops {
            match op {
                TreeOp::Create { ref id, .. } => {
                    created.insert(id.clone(), op);
                }
                TreeOp::Append {
                    ref parent,
                    ref child,
                    ..
                } => {
                    let Some(to) = self.node_workspace(parent) else {
                        continue;
                    };
                    let from = self.node_workspace(child);
                    let create = created.remove(child);
                    let child = child.clone();

                    // A node that moves to another workspace's tree takes what's under it along.
                    // Windows aren't moved that way: they go with their workspace.
                    let moved = match from.filter(|from| *from != to) {
                        Some(from) if create.is_none() => {
                            let taken = self
                                .workspace_mut(from)
                                .and_then(|ws| ws.scrolling_mut().tree_mut().take(&child));
                            let Some(taken) = taken else {
                                warn!(
                                    "the layout moves {child} to another workspace's tree, \
                                     but there's a window in it"
                                );
                                continue;
                            };
                            touched.insert(from);
                            Some(taken)
                        }
                        _ => None,
                    };

                    let Some(ws) = self.workspace_mut(to) else {
                        continue;
                    };
                    let tree = ws.scrolling_mut().tree_mut();
                    match moved {
                        Some(moved) => {
                            let TreeOp::Append { parent, before, .. } = op else {
                                unreachable!()
                            };
                            tree.put(moved, &parent, before.as_deref());
                        }
                        None => {
                            let ops: Vec<Value> = create
                                .into_iter()
                                .chain([op])
                                .map(|op| to_value(&op))
                                .collect();
                            if let Err(err) = tree.apply(&ops) {
                                warn!("error in the nodes that the layout draws: {err}");
                            }
                        }
                    }
                    touched.insert(to);
                }
                TreeOp::Set { ref id, .. } => {
                    let Some(ws) = self
                        .node_workspace(id)
                        .and_then(|ws| self.workspace_mut(ws))
                    else {
                        continue;
                    };
                    if let Err(err) = ws.scrolling_mut().tree_mut().apply(&[to_value(&op)]) {
                        warn!("error in the nodes that the layout draws: {err}");
                    }
                }
                TreeOp::Remove { ref id } => {
                    let Some(workspace) = self.node_workspace(id) else {
                        continue;
                    };
                    let Some(ws) = self.workspace_mut(workspace) else {
                        continue;
                    };
                    if let Err(err) = ws.scrolling_mut().tree_mut().apply(&[to_value(&op)]) {
                        warn!("error in the nodes that the layout draws: {err}");
                    }
                    touched.insert(workspace);
                }
            }
        }
        touched
    }

    /// Whether the layout's exposed state changed since this was last called.
    pub fn take_engine_state_changed(&mut self) -> bool {
        std::mem::take(&mut self.host.state_changed)
    }

    /// Engine id of the focused window, if the layout has it: not a floating one.
    pub(in crate::layout) fn focused_engine_window(&self) -> Option<u64> {
        let ws = self.active_workspace()?;
        if ws.floating_is_active() {
            return None;
        }
        ws.scrolling().active_engine_id()
    }

    /// Runs one of the layout's actions, from a bind in its file or `layout-action`, for `window`
    /// or the focused window. Only actions the layout declares reach it, and only for a window it
    /// places. Arguments are converted to the types it declared, and arguments left out get their
    /// defaults.
    pub fn layout_action(&mut self, name: &str, args: &[TilingOption], window: Option<&W::Id>) {
        let Some(declared) = engine::action_args(name) else {
            return;
        };
        let values = self.action_args(&declared, args);
        let window = match window {
            Some(window) => match self.engine_id_of(window) {
                Some(id) => Some(id),
                None => return,
            },
            None => self.focused_engine_window(),
        };
        let event = json!({ "type": "action", "name": name, "args": values, "window": window });
        self.engine_event(event);
    }

    fn action_args(&self, declared: &[LayoutEngineActionArg], args: &[TilingOption]) -> Vec<Value> {
        let convert = |i: usize, arg: &TilingOption| match declared.get(i).and_then(|a| a.r#type) {
            Some(LayoutEngineActionArgType::Size) => {
                engine::size_arg(arg).map_or_else(|| option_json(arg), size_change_json)
            }
            Some(LayoutEngineActionArgType::Workspace) => {
                self.workspace_arg(arg).map_or(Value::Null, |id| json!(id))
            }
            Some(LayoutEngineActionArgType::Monitor) => match arg {
                TilingOption::String(name) if self.outputs().any(|o| o.name() == *name) => {
                    json!(name)
                }
                _ => Value::Null,
            },
            _ => option_json(arg),
        };

        let mut values: Vec<Value> = args
            .iter()
            .enumerate()
            .map(|(i, arg)| convert(i, arg))
            .collect();
        for (i, arg) in declared.iter().enumerate().skip(args.len()) {
            let Some(default) = &arg.default else {
                break;
            };
            let default = match default {
                LayoutEngineOptionValue::Boolean(value) => TilingOption::Bool(*value),
                LayoutEngineOptionValue::Number(value) => TilingOption::Number(*value),
                LayoutEngineOptionValue::String(value) => TilingOption::String(value.clone()),
                // Action arguments are single values; only options are lists or gradients.
                LayoutEngineOptionValue::Gradient(_) | LayoutEngineOptionValue::List(_) => break,
            };
            values.push(convert(i, &default));
        }
        values
    }

    /// Id of the workspace that an action argument names: an index on the focused monitor,
    /// counting from 1, where one past the end means the last workspace, or a name.
    fn workspace_arg(&self, arg: &TilingOption) -> Option<u64> {
        match arg {
            TilingOption::Number(index) => {
                let monitor = self.active_monitor_ref()?;
                let last = monitor.workspaces.len().checked_sub(1)?;
                let idx = ((*index as i64).max(1) as usize - 1).min(last);
                Some(monitor.workspaces[idx].id().get())
            }
            TilingOption::String(name) => self
                .find_workspace_by_name(name)
                .map(|(_, ws)| ws.id().get()),
            TilingOption::Bool(_) | TilingOption::Gradient(_) => None,
        }
    }

    /// Asks the layout whether something may focus a window, like a click or an app. Returns
    /// whether the layout prevented it: otherwise bento shows the window's workspace, focuses
    /// its monitor and focuses the window.
    pub(in crate::layout) fn activate_prevented(&mut self, window: &W::Id) -> bool {
        let Some(id) = self.engine_id_of(window) else {
            return false;
        };
        self.engine_verdict(json!({ "type": "activaterequest", "id": id }))
            .is_some_and(|verdict| verdict.prevented)
    }

    /// Starts an interactive resize of a tiled window, if the layout allows it.
    pub(in crate::layout) fn tiling_resize_begin(
        &mut self,
        window: W::Id,
        edges: ResizeEdge,
    ) -> bool {
        let Some(ws_id) = self
            .workspaces()
            .find(|(_, _, ws)| ws.scrolling().has_window(&window))
            .map(|(_, _, ws)| ws.id().get())
        else {
            return false;
        };
        let Some(id) = self
            .workspace_mut(ws_id)
            .and_then(|ws| ws.scrolling_mut().interactive_resize_begin(window, edges))
        else {
            return false;
        };

        let event = json!({
            "type": "resizebegin",
            "id": id,
            "edges": {
                "left": edges.contains(ResizeEdge::LEFT),
                "right": edges.contains(ResizeEdge::RIGHT),
                "top": edges.contains(ResizeEdge::TOP),
                "bottom": edges.contains(ResizeEdge::BOTTOM),
            },
            "delta": Xy { x: 0., y: 0. },
        });
        let verdict = self.engine_verdict(event).unwrap_or_default();
        if verdict.handled && !verdict.prevented {
            return true;
        }
        if let Some(ws) = self.workspace_mut(ws_id) {
            ws.scrolling_mut().interactive_resize_cancel();
        }
        false
    }

    /// Tells the layout about a double click on a window's resize edge.
    pub fn edge_double_click(&mut self, window: &W::Id, edges: ResizeEdge) {
        let Some(id) = self.engine_id_of(window) else {
            return;
        };
        let event = json!({
            "type": "edgedoubleclick",
            "id": id,
            "edges": {
                "left": edges.contains(ResizeEdge::LEFT),
                "right": edges.contains(ResizeEdge::RIGHT),
                "top": edges.contains(ResizeEdge::TOP),
                "bottom": edges.contains(ResizeEdge::BOTTOM),
            },
        });
        self.engine_event(event);
    }

    /// Tells the layout that a window asks to float, when floating is off.
    pub(in crate::layout) fn queue_float_request(
        &self,
        window: &W::Id,
        reason: &str,
        parent: Option<&W::Id>,
    ) {
        let Some(ws) = self
            .workspaces()
            .map(|(_, _, ws)| ws)
            .find(|ws| ws.scrolling().has_window(window))
        else {
            return;
        };
        let parent = parent.and_then(|parent| self.engine_id_of(parent));
        // The size the window has, which for one that just opened is the size it asked for.
        let size = ws
            .scrolling()
            .tiles()
            .find(|tile| tile.window().id() == window)
            .map(|tile| tile.window().size())
            .filter(|size| size.w > 0 && size.h > 0)
            .map(|size| (size.w, size.h));
        ws.scrolling().queue_window_event(
            window,
            "floatrequest",
            json!({
                "reason": reason,
                "parent": parent,
                "size": size.map(|(w, h)| Wh { w: f64::from(w), h: f64::from(h) }),
            }),
        );
    }

    /// Delivers a pointer event to a node that the layout draws, which bubbles to its parents.
    /// `pointer` is on the workspace's canvas, `button` is 1, 2 or 3 for the left, middle and right
    /// button, and 0 for events that aren't about one.
    #[allow(clippy::too_many_arguments)]
    pub fn node_pointer_event(
        &mut self,
        node: &str,
        event: &str,
        pointer: Point<f64, Logical>,
        button: u8,
        modifiers: [bool; 4],
        delta: Point<f64, Logical>,
    ) {
        let [shift, ctrl, alt, logo] = modifiers;
        self.engine_event(json!({
            "type": "pointer",
            "node": node,
            "event": event,
            "pointer": Xy::from_point(pointer),
            "button": button,
            "modifiers": { "shift": shift, "ctrl": ctrl, "alt": alt, "super": logo },
            "delta": Xy::from_point(delta),
        }));
    }

    /// Tells the layout that the pointer is on the node `hit`, or on none, so that the nodes it
    /// left get `pointerleave` and the ones it entered get `pointerenter`.
    pub fn node_hover(&mut self, hit: Option<&NodeHit>, modifiers: [bool; 4]) {
        let mut events = Vec::new();
        for ws in self.workspaces_mut() {
            let id = ws.id().get();
            let over = hit
                .filter(|hit| hit.workspace == id)
                .map(|hit| (hit.node.as_str(), hit.pos));
            events.extend(ws.scrolling_mut().pointer_over(over));
        }

        for (node, event, pointer) in events {
            self.node_pointer_event(&node, event, pointer, 0, modifiers, Point::from((0., 0.)));
        }
    }

    /// Tells the layout that the pointer moved, and is on the node `hit` or on none.
    pub fn node_pointer_moved(&mut self, hit: Option<&NodeHit>, modifiers: [bool; 4]) {
        self.node_hover(hit, modifiers);
        if let Some(hit) = hit {
            self.node_pointer_event(
                &hit.node,
                "pointermove",
                hit.pos,
                0,
                modifiers,
                Point::from((0., 0.)),
            );
        }
    }

    /// Tells the layout where an interactively moved window is over a workspace, or that it left.
    pub(in crate::layout) fn update_drag(&mut self) {
        let target = self.drag_target();
        let previous = self.host.drag.take();

        match (previous, target) {
            (Some(previous), Some((workspace, window, pointer_rect))) => {
                if previous.workspace == workspace {
                    if previous.last != pointer_rect {
                        host_queue_drag("dragmove", workspace, &window, &pointer_rect);
                    }
                } else {
                    queue(json!({ "type": "dragleave", "workspace": previous.workspace }));
                    host_queue_drag("dragenter", workspace, &window, &pointer_rect);
                }
                self.host.drag = Some(Drag {
                    workspace,
                    last: pointer_rect,
                });
            }
            (None, Some((workspace, window, pointer_rect))) => {
                host_queue_drag("dragenter", workspace, &window, &pointer_rect);
                self.host.drag = Some(Drag {
                    workspace,
                    last: pointer_rect,
                });
            }
            (Some(previous), None) => {
                queue(json!({ "type": "dragleave", "workspace": previous.workspace }));
            }
            (None, None) => (),
        }
    }

    /// The workspace that an interactively moved tiled window is over, with the window's info and
    /// the pointer and rect on the workspace's canvas.
    fn drag_target(&self) -> Option<(u64, Value, Value)> {
        let Some(InteractiveMoveState::Moving(move_)) = &self.interactive_move else {
            return None;
        };
        if move_.is_floating {
            return None;
        }
        let mon = self.monitor_for_output(&move_.output)?;
        let zoom = mon.overview_zoom();
        let (insert_ws, geo) = mon.insert_position(move_.pointer_pos_within_output);
        let InsertWorkspace::Existing(ws_id) = insert_ws else {
            return None;
        };
        let ws = mon.workspaces.iter().find(|ws| ws.id() == ws_id)?;

        let pos_within_workspace = (move_.pointer_pos_within_output - geo.loc).downscale(zoom);
        let tile_pos_within_workspace =
            (move_.tile_render_location(zoom) - geo.loc).downscale(zoom);
        let drop = ws.scrolling().drag_on_canvas(
            pos_within_workspace,
            &move_.tile,
            tile_pos_within_workspace,
        );
        let window = TilingSpace::<W>::window_info_of(&move_.tile, ws_id.get(), &engine::name());
        let pointer_rect = json!({
            "pointer": Xy::from_point(drop.pointer),
            "rect": Rect::from_rect(drop.rect),
        });
        Some((ws_id.get(), to_value(&window), pointer_rect))
    }
}

fn host_queue_drag(kind: &str, workspace: u64, window: &Value, pointer_rect: &Value) {
    queue(json!({
        "type": kind,
        "workspace": workspace,
        "window": window,
        "pointer": pointer_rect["pointer"],
        "rect": pointer_rect["rect"],
    }));
}
