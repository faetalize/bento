//! What subscribers know about bento's objects, and the events that keep them up to date.

use std::collections::BTreeMap;

use bento_ipc::{Cast, Event, Group, KeyboardLayouts, Monitor, Window, Workspace};

/// Who has keyboard focus.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Focus {
    /// The name of the focused monitor.
    pub monitor: Option<String>,
    /// The id of the focused window.
    pub window: Option<u64>,
}

/// The state that the events describe, at one moment.
#[derive(Debug, Default, Clone)]
pub struct Snapshot {
    pub monitors: BTreeMap<String, Monitor>,
    pub workspaces: BTreeMap<u64, Workspace>,
    pub windows: BTreeMap<u64, Window>,
    pub groups: BTreeMap<u64, Group>,
    pub focus: Focus,
    pub keyboard_layouts: Option<KeyboardLayouts>,
    pub overview_open: bool,
    pub casts: BTreeMap<u64, Cast>,
    pub touchpad_on: bool,
}

/// What happened to the objects of one kind between two snapshots.
struct Diff<'a, K, V> {
    added: Vec<&'a V>,
    changed: Vec<&'a V>,
    removed: Vec<&'a K>,
}

/// Compares the objects of `old` and `new`. Without `old`, everything in `new` is added.
fn diff<'a, K: Ord, V: PartialEq>(
    old: Option<&'a BTreeMap<K, V>>,
    new: &'a BTreeMap<K, V>,
) -> Diff<'a, K, V> {
    let mut diff = Diff {
        added: Vec::new(),
        changed: Vec::new(),
        removed: Vec::new(),
    };

    for (key, value) in new {
        match old.and_then(|old| old.get(key)) {
            None => diff.added.push(value),
            Some(old) if old != value => diff.changed.push(value),
            Some(_) => (),
        }
    }
    if let Some(old) = old {
        diff.removed = old.keys().filter(|key| !new.contains_key(*key)).collect();
    }

    diff
}

impl Snapshot {
    /// The events that take a subscriber who knows `old` to this state, or tell a new subscriber
    /// everything if there is no `old`.
    ///
    /// Objects come in the order they refer to each other, so that what an event names has
    /// mostly been announced already: monitors, workspaces, windows and groups when they appear
    /// and change, and the other way round when they go away.
    pub fn events_since(&self, old: Option<&Snapshot>) -> Vec<Event> {
        let mut events = Vec::new();

        let monitors = diff(old.map(|old| &old.monitors), &self.monitors);
        let workspaces = diff(old.map(|old| &old.workspaces), &self.workspaces);
        let windows = diff(old.map(|old| &old.windows), &self.windows);
        let groups = diff(old.map(|old| &old.groups), &self.groups);

        events.extend(
            monitors
                .added
                .into_iter()
                .cloned()
                .map(|monitor| Event::MonitorAdded { monitor }),
        );
        events.extend(
            workspaces
                .added
                .into_iter()
                .cloned()
                .map(|workspace| Event::WorkspaceAdded { workspace }),
        );
        events.extend(
            windows
                .added
                .into_iter()
                .cloned()
                .map(|window| Event::WindowAdded { window }),
        );
        events.extend(
            groups
                .added
                .into_iter()
                .cloned()
                .map(|group| Event::GroupAdded { group }),
        );

        events.extend(
            monitors
                .changed
                .into_iter()
                .cloned()
                .map(|monitor| Event::MonitorChanged { monitor }),
        );
        events.extend(
            workspaces
                .changed
                .into_iter()
                .cloned()
                .map(|workspace| Event::WorkspaceChanged { workspace }),
        );
        events.extend(
            windows
                .changed
                .into_iter()
                .cloned()
                .map(|window| Event::WindowChanged { window }),
        );
        events.extend(
            groups
                .changed
                .into_iter()
                .cloned()
                .map(|group| Event::GroupChanged { group }),
        );

        if old.is_none_or(|old| old.focus != self.focus) {
            events.push(Event::FocusChanged {
                monitor: self.focus.monitor.clone(),
                window: self.focus.window,
            });
        }

        if let Some(layouts) = &self.keyboard_layouts {
            match old.and_then(|old| old.keyboard_layouts.as_ref()) {
                Some(old) if old == layouts => (),
                Some(old) if old.names == layouts.names => {
                    events.push(Event::KeyboardLayoutSwitched {
                        index: layouts.current_idx,
                    });
                }
                _ => events.push(Event::KeyboardLayoutsChanged {
                    keyboard_layouts: layouts.clone(),
                }),
            }
        }

        if old.is_none_or(|old| old.overview_open != self.overview_open) {
            events.push(Event::OverviewChanged {
                is_open: self.overview_open,
            });
        }

        if old.is_none_or(|old| old.casts != self.casts) {
            events.push(Event::CastsChanged {
                casts: self.casts.values().cloned().collect(),
            });
        }

        if old.is_none_or(|old| old.touchpad_on != self.touchpad_on) {
            events.push(Event::TouchpadToggled {
                on: self.touchpad_on,
            });
        }

        events.extend(
            groups
                .removed
                .into_iter()
                .map(|&id| Event::GroupRemoved { id }),
        );
        events.extend(
            windows
                .removed
                .into_iter()
                .map(|&id| Event::WindowRemoved { id }),
        );
        events.extend(
            workspaces
                .removed
                .into_iter()
                .map(|&id| Event::WorkspaceRemoved { id }),
        );
        events.extend(
            monitors
                .removed
                .into_iter()
                .map(|name| Event::MonitorRemoved { name: name.clone() }),
        );

        events
    }
}

#[cfg(test)]
mod tests {
    use bento_ipc::{Mode, SizingMode};

    use super::*;

    fn window(id: u64, title: &str) -> Window {
        Window {
            id,
            app_id: None,
            title: Some(String::from(title)),
            pid: None,
            workspace: Some(1),
            rect: None,
            size: bento_ipc::Size { w: 10., h: 10. },
            mode: SizingMode::Normal,
            floating: false,
            focused: false,
            urgent: false,
            group: None,
            focus_timestamp: None,
        }
    }

    fn workspace(id: u64) -> Workspace {
        Workspace {
            id,
            name: None,
            monitor: Some(String::from("DP-1")),
            index: 0,
            windows: Vec::new(),
            focused: None,
            active: true,
            urgent: false,
        }
    }

    fn monitor(name: &str) -> Monitor {
        Monitor {
            name: String::from(name),
            make: String::from("Make"),
            model: String::from("Model"),
            serial: None,
            physical_size: None,
            modes: vec![Mode {
                width: 1920,
                height: 1080,
                refresh_rate: 60_000,
                preferred: true,
            }],
            current_mode: Some(0),
            custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            max_bpc: None,
            enabled: true,
            position: None,
            size: None,
            scale: Some(1.),
            transform: None,
            working_area: None,
            workspaces: vec![1],
            active_workspace: Some(1),
            focused: true,
        }
    }

    fn kinds(events: &[Event]) -> Vec<bento_ipc::EventKind> {
        events.iter().map(Event::kind).collect()
    }

    #[test]
    fn new_subscriber_gets_everything() {
        let mut snapshot = Snapshot::default();
        snapshot.monitors.insert("DP-1".into(), monitor("DP-1"));
        snapshot.workspaces.insert(1, workspace(1));
        snapshot.windows.insert(3, window(3, "a"));

        use bento_ipc::EventKind::*;
        assert_eq!(
            kinds(&snapshot.events_since(None)),
            [
                MonitorAdded,
                WorkspaceAdded,
                WindowAdded,
                FocusChanged,
                OverviewChanged,
                CastsChanged,
                TouchpadToggled,
            ]
        );
    }

    #[test]
    fn nothing_changed_nothing_sent() {
        let mut snapshot = Snapshot::default();
        snapshot.windows.insert(3, window(3, "a"));
        assert!(snapshot.events_since(Some(&snapshot.clone())).is_empty());
    }

    #[test]
    fn changes_are_added_changed_and_removed() {
        let mut old = Snapshot::default();
        old.windows.insert(1, window(1, "gone"));
        old.windows.insert(2, window(2, "before"));
        old.windows.insert(3, window(3, "same"));

        let mut new = Snapshot::default();
        new.windows.insert(2, window(2, "after"));
        new.windows.insert(3, window(3, "same"));
        new.windows.insert(4, window(4, "new"));

        let events = new.events_since(Some(&old));
        assert_eq!(
            events,
            [
                Event::WindowAdded {
                    window: window(4, "new")
                },
                Event::WindowChanged {
                    window: window(2, "after")
                },
                Event::WindowRemoved { id: 1 },
            ]
        );
    }

    #[test]
    fn removals_come_after_what_refers_to_them() {
        let mut old = Snapshot::default();
        old.monitors.insert("DP-1".into(), monitor("DP-1"));
        old.workspaces.insert(1, workspace(1));
        old.windows.insert(3, window(3, "a"));

        use bento_ipc::EventKind::*;
        assert_eq!(
            kinds(&Snapshot::default().events_since(Some(&old))),
            [WindowRemoved, WorkspaceRemoved, MonitorRemoved]
        );
    }

    #[test]
    fn keyboard_layout_switch_is_not_a_change_of_layouts() {
        let layouts = |current_idx| KeyboardLayouts {
            names: vec![String::from("us"), String::from("de")],
            current_idx,
        };
        let old = Snapshot {
            keyboard_layouts: Some(layouts(0)),
            ..Snapshot::default()
        };
        let mut new = old.clone();
        new.keyboard_layouts = Some(layouts(1));

        assert_eq!(
            new.events_since(Some(&old)),
            [Event::KeyboardLayoutSwitched { index: 1 }]
        );

        new.keyboard_layouts = Some(KeyboardLayouts {
            names: vec![String::from("us")],
            current_idx: 0,
        });
        assert_eq!(
            bento_ipc::Event::kind(&new.events_since(Some(&old))[0]),
            bento_ipc::EventKind::KeyboardLayoutsChanged
        );
    }
}
