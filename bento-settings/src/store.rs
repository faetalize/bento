//! The files behind the app.
//!
//! The app writes only `settings.kdl`, next to `config.kdl`, which includes it. Values shown come
//! from the whole config as bento sees it, so they're right whichever file sets them. bento
//! watches both files and applies changes itself.

use std::cell::{Cell, Ref, RefCell};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};
use niri_config::Config;

use crate::kdl::{self, Node, Value};

const HEADER: &str = "\
// Written by Bento Settings. Edits here are kept, but comments and formatting aren't.
// config.kdl includes this file, so settings here override the ones before the include.

";

/// Sections that bento replaces as a whole when a later file sets them, instead of merging them
/// key by key. Before changing anything inside one, the app copies the whole section from the
/// config into settings.kdl, so the rest of it keeps working.
const WHOLE_SECTIONS: &[&str] = &[
    "tiling",
    "input.keyboard.xkb",
    "input.touchpad",
    "input.mouse",
    "input.trackpoint",
    "input.trackball",
    "input.tablet",
    "input.touch",
];

const SAVE_DELAY: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub struct Store(Rc<Inner>);

struct Inner {
    config_path: PathBuf,
    settings_path: PathBuf,
    doc: RefCell<Vec<Node>>,
    config: RefCell<Config>,
    status: RefCell<Status>,
    /// What the app last wrote, to tell its own writes from someone else's.
    last_written: RefCell<Option<String>>,
    pending_save: RefCell<Option<glib::SourceId>>,
    pending_reload: RefCell<Option<glib::SourceId>>,
    listeners: RefCell<Vec<Rc<dyn Fn()>>>,
    monitors: RefCell<Vec<gio::FileMonitor>>,
    notifying: Cell<bool>,
}

/// Problems the app shows in a banner.
#[derive(Debug, Clone, Default)]
pub struct Status {
    /// config.kdl doesn't include settings.kdl, so nothing the app writes applies.
    pub not_included: bool,
    /// settings.kdl doesn't parse; the app won't overwrite it until it does.
    pub settings_error: Option<String>,
    /// The whole config doesn't load, so bento keeps running its last good config.
    pub config_error: Option<String>,
    /// The last write failed.
    pub write_error: Option<String>,
}

impl Store {
    pub fn load() -> Self {
        Self::load_from(config_path())
    }

    pub fn load_from(config_path: PathBuf) -> Self {
        let settings_path = config_path.with_file_name("settings.kdl");
        let store = Self(Rc::new(Inner {
            config_path,
            settings_path,
            doc: RefCell::new(Vec::new()),
            config: RefCell::new(Config::default()),
            status: RefCell::new(Status::default()),
            last_written: RefCell::new(None),
            pending_save: RefCell::new(None),
            pending_reload: RefCell::new(None),
            listeners: RefCell::new(Vec::new()),
            monitors: RefCell::new(Vec::new()),
            notifying: Cell::new(false),
        }));
        store.read_settings();
        store.read_config();
        store.watch();
        store
    }

    pub fn config_path(&self) -> &Path {
        &self.0.config_path
    }

    pub fn settings_path(&self) -> &Path {
        &self.0.settings_path
    }

    pub fn config(&self) -> Ref<'_, Config> {
        self.0.config.borrow()
    }

    pub fn doc(&self) -> Ref<'_, Vec<Node>> {
        self.0.doc.borrow()
    }

    pub fn status(&self) -> Status {
        self.0.status.borrow().clone()
    }

    /// Calls `f` whenever the config or settings change, from the app or from outside.
    pub fn subscribe(&self, f: impl Fn() + 'static) {
        self.0.listeners.borrow_mut().push(Rc::new(f));
    }

    fn notify(&self) {
        // A listener that triggers another notification would otherwise loop.
        if self.0.notifying.replace(true) {
            return;
        }
        let listeners = self.0.listeners.borrow().clone();
        for listener in listeners {
            listener();
        }
        self.0.notifying.set(false);
    }

    /// Changes settings.kdl and saves it shortly after, so dragging a slider writes once.
    ///
    /// `path` is where the change lands. If that's inside a section bento replaces as a whole,
    /// the section is first copied over from the config.
    pub fn edit(&self, path: &str, f: impl FnOnce(&mut Vec<Node>)) {
        if self.0.status.borrow().settings_error.is_some() {
            return;
        }
        {
            let mut doc = self.0.doc.borrow_mut();
            if let Some(section) = whole_section(path) {
                self.copy_section(&mut doc, section);
            }
            f(&mut doc);
        }
        self.schedule_save();
    }

    fn copy_section(&self, doc: &mut Vec<Node>, section: &str) {
        let parts = kdl::split(section);
        if kdl::get(doc, &parts).is_some() {
            return;
        }
        let nodes = self.config_nodes();
        if let Some(found) = find_last(&nodes, &parts) {
            let (_, parents) = parts.split_last().unwrap();
            let siblings = if parents.is_empty() {
                &mut *doc
            } else {
                &mut kdl::ensure(doc, parents).children
            };
            siblings.push(found.clone());
        }
    }

    /// The nodes of config.kdl with its includes spliced in, leaving out settings.kdl.
    fn config_nodes(&self) -> Vec<Node> {
        let mut out = Vec::new();
        self.collect_nodes(&self.0.config_path, &mut out, 0);
        out
    }

    fn collect_nodes(&self, path: &Path, out: &mut Vec<Node>, depth: u8) {
        if depth > 10 {
            return;
        }
        let Ok(text) = fs::read_to_string(path) else {
            return;
        };
        let Ok(nodes) = kdl::parse(&path.to_string_lossy(), &text) else {
            return;
        };
        for node in nodes {
            if node.name == "include" {
                if let Some(target) = include_target(path, &node) {
                    if !same_file(&target, &self.0.settings_path) {
                        self.collect_nodes(&target, out, depth + 1);
                    }
                }
            } else {
                out.push(node);
            }
        }
    }

    fn schedule_save(&self) {
        if let Some(source) = self.0.pending_save.take() {
            source.remove();
        }
        let store = self.clone();
        let source = glib::timeout_add_local_once(SAVE_DELAY, move || {
            store.0.pending_save.take();
            store.save();
        });
        self.0.pending_save.replace(Some(source));
    }

    /// Saves pending changes right away.
    pub fn flush(&self) {
        if let Some(source) = self.0.pending_save.take() {
            source.remove();
            self.save();
        }
    }

    fn save(&self) {
        let text = format!("{HEADER}{}", kdl::to_string(&self.0.doc.borrow()));
        let result = self.write(&text);
        self.0.status.borrow_mut().write_error = result.err();
        self.read_config();
        self.notify();
    }

    fn write(&self, text: &str) -> Result<(), String> {
        // Check the file on its own first, so a bad value never reaches bento.
        if let Err(err) = Config::parse(&self.0.settings_path, text).config {
            return Err(describe(&err));
        }

        // Write through a symlink rather than replacing it, for settings.kdl kept in a dotfiles
        // repo. The temporary file sits next to the real one so the rename stays on one disk.
        let target = fs::canonicalize(&self.0.settings_path)
            .unwrap_or_else(|_| self.0.settings_path.clone());
        let tmp = target.with_extension("kdl.tmp");
        fs::write(&tmp, text)
            .and_then(|()| fs::rename(&tmp, &target))
            .map_err(|err| format!("Couldn't write {}: {err}", target.display()))?;
        self.0.last_written.replace(Some(text.to_owned()));
        Ok(())
    }

    fn read_settings(&self) {
        let mut status = self.0.status.borrow_mut();
        status.settings_error = None;
        let text = match fs::read_to_string(&self.0.settings_path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => {
                status.settings_error = Some(err.to_string());
                return;
            }
        };
        match kdl::parse(&self.0.settings_path.to_string_lossy(), &text) {
            Ok(nodes) => {
                self.0.doc.replace(nodes);
            }
            Err(err) => status.settings_error = Some(describe(&err)),
        }
    }

    fn read_config(&self) {
        let result = Config::load(&self.0.config_path);
        let mut status = self.0.status.borrow_mut();
        match result.config {
            Ok(config) => {
                self.0.config.replace(config);
                status.config_error = None;
            }
            Err(err) => status.config_error = Some(describe(err.as_ref())),
        }
        status.not_included = !self.includes_settings();
    }

    fn includes_settings(&self) -> bool {
        let Ok(text) = fs::read_to_string(&self.0.config_path) else {
            return false;
        };
        let Ok(nodes) = kdl::parse("config.kdl", &text) else {
            // Can't tell; the config error banner covers this case.
            return true;
        };
        nodes.iter().any(|node| {
            node.name == "include"
                && include_target(&self.0.config_path, node)
                    .is_some_and(|target| same_file(&target, &self.0.settings_path))
        })
    }

    /// Appends `include "settings.kdl"` to config.kdl, so the app's settings apply.
    pub fn add_include(&self) -> Result<(), String> {
        let mut text = fs::read_to_string(&self.0.config_path)
            .map_err(|err| format!("Couldn't read {}: {err}", self.0.config_path.display()))?;
        if !text.ends_with('\n') && !text.is_empty() {
            text.push('\n');
        }
        text.push_str(
            "\n// Settings from Bento Settings. Keep this last, so they override the ones above.\n\
             include optional=true \"settings.kdl\"\n",
        );
        fs::write(&self.0.config_path, text)
            .map_err(|err| format!("Couldn't write {}: {err}", self.0.config_path.display()))?;
        self.reload();
        Ok(())
    }

    /// Re-reads both files.
    pub fn reload(&self) {
        self.read_settings();
        self.read_config();
        self.notify();
    }

    fn watch(&self) {
        for path in [&self.0.config_path, &self.0.settings_path] {
            let file = gio::File::for_path(path);
            let Ok(monitor) = file.monitor_file(
                gio::FileMonitorFlags::WATCH_MOVES,
                None::<&gio::Cancellable>,
            ) else {
                continue;
            };
            let store = Rc::downgrade(&self.0);
            monitor.connect_changed(move |_, _, _, event| {
                if matches!(
                    event,
                    gio::FileMonitorEvent::ChangesDoneHint
                        | gio::FileMonitorEvent::Created
                        | gio::FileMonitorEvent::Deleted
                        | gio::FileMonitorEvent::Renamed
                        | gio::FileMonitorEvent::MovedIn
                ) {
                    if let Some(inner) = store.upgrade() {
                        Store(inner).schedule_reload();
                    }
                }
            });
            self.0.monitors.borrow_mut().push(monitor);
        }
    }

    fn schedule_reload(&self) {
        if let Some(source) = self.0.pending_reload.take() {
            source.remove();
        }
        let store = self.clone();
        let source = glib::timeout_add_local_once(Duration::from_millis(150), move || {
            store.0.pending_reload.take();
            // Our own writes are already in memory; re-reading them would only drop edits made
            // since.
            let on_disk = fs::read_to_string(&store.0.settings_path).ok();
            let ours = on_disk.is_some() && *store.0.last_written.borrow() == on_disk;
            if !ours && store.0.pending_save.borrow().is_none() {
                store.read_settings();
            }
            store.read_config();
            store.notify();
        });
        self.0.pending_reload.replace(Some(source));
    }
}

/// bento's config path: `$NIRI_CONFIG`, or `config.kdl` in bento's config directory.
fn config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("NIRI_CONFIG").filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    glib::user_config_dir().join("bento").join("config.kdl")
}

fn whole_section(path: &str) -> Option<&'static str> {
    WHOLE_SECTIONS
        .iter()
        .copied()
        .filter(|section| path == *section || path.starts_with(&format!("{section}.")))
        .max_by_key(|section| section.len())
}

/// Finds the last node at a path across every section of that name, which is the one bento uses.
fn find_last<'a>(nodes: &'a [Node], path: &[&str]) -> Option<&'a Node> {
    let (first, rest) = path.split_first()?;
    nodes
        .iter()
        .rev()
        .filter(|node| node.name == *first)
        .find_map(|node| {
            if rest.is_empty() {
                Some(node)
            } else {
                find_last(&node.children, rest)
            }
        })
}

fn include_target(from: &Path, node: &Node) -> Option<PathBuf> {
    let path = node.arg()?.as_str()?;
    let path = match path.strip_prefix("~/") {
        Some(rest) => glib::home_dir().join(rest),
        None => PathBuf::from(path),
    };
    Some(match from.parent() {
        Some(dir) => dir.join(path),
        None => path,
    })
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Renders a config error the way bento's own messages read, without colors.
pub fn describe(err: &dyn miette::Diagnostic) -> String {
    let handler =
        miette::GraphicalReportHandler::new_themed(miette::GraphicalTheme::unicode_nocolor());
    let mut out = String::new();
    if handler.render_report(&mut out, err).is_err() {
        out = err.to_string();
    }
    out.trim_end().to_owned()
}

/// Shortens a path under the home directory to `~/...`, the way a config would spell it.
pub fn tilde(path: &Path) -> String {
    match path.strip_prefix(glib::home_dir()) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// A plain value for a layout engine option.
pub fn option_value(value: &niri_ipc::LayoutEngineOptionValue) -> Value {
    match value {
        niri_ipc::LayoutEngineOptionValue::Boolean(x) => Value::Bool(*x),
        niri_ipc::LayoutEngineOptionValue::Number(x) if x.fract() == 0. => Value::Int(*x as i64),
        niri_ipc::LayoutEngineOptionValue::Number(x) => Value::Float(*x),
        niri_ipc::LayoutEngineOptionValue::String(x) => Value::Str(x.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BoolStyle, Input, Setting};

    #[test]
    fn edits_keep_the_rest_of_whole_sections() {
        let dir = std::env::temp_dir().join(format!("bento-settings-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("config.kdl");
        fs::write(
            &config_path,
            "input {\n    touchpad {\n        tap\n        natural-scroll\n    }\n}\ninclude \"settings.kdl\"\n",
        )
        .unwrap();
        fs::write(dir.join("settings.kdl"), "").unwrap();

        let store = Store::load_from(config_path);
        assert!(!store.status().not_included);
        assert!(store.config().input.touchpad.natural_scroll);

        let natural_scroll = Setting::switch("", "input.touchpad.natural-scroll", |c| {
            c.input.touchpad.natural_scroll
        })
        .style(BoolStyle::Presence);
        store.edit(natural_scroll.path, |doc| {
            natural_scroll.write(doc, Input::Bool(false))
        });
        let focus_ring = Setting::switch("", "layout.focus-ring", |c| !c.layout.focus_ring.off)
            .style(BoolStyle::OnOff);
        store.edit(focus_ring.path, |doc| {
            focus_ring.write(doc, Input::Bool(false))
        });
        let numlock = Setting::switch("", "input.keyboard.numlock", |c| c.input.keyboard.numlock);
        store.edit(numlock.path, |doc| numlock.write(doc, Input::Bool(false)));
        store.flush();

        let written = fs::read_to_string(dir.join("settings.kdl")).unwrap();
        let body = written.trim_start_matches(HEADER);
        assert_eq!(
            body,
            "input {\n    touchpad {\n        tap\n    }\n    keyboard {\n        numlock false\n    }\n}\nlayout {\n    focus-ring {\n        off\n    }\n}\n"
        );

        let config = store.config();
        assert!(config.input.touchpad.tap);
        assert!(!config.input.touchpad.natural_scroll);
        assert!(config.layout.focus_ring.off);
        assert!(natural_scroll.applied(&store.doc(), &config));
        drop(config);

        // Resetting the focus ring takes the whole layout section out again.
        store.edit(focus_ring.path, |doc| focus_ring.reset(doc));
        store.flush();
        assert!(!store.config().layout.focus_ring.off);
        assert!(kdl::get(&store.doc(), &["layout"]).is_none());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finds_whole_sections() {
        assert_eq!(whole_section("input.touchpad.tap"), Some("input.touchpad"));
        assert_eq!(
            whole_section("input.keyboard.xkb.layout"),
            Some("input.keyboard.xkb")
        );
        assert_eq!(whole_section("tiling.options.gravity"), Some("tiling"));
        assert_eq!(whole_section("input.keyboard.numlock"), None);
        assert_eq!(whole_section("input.touchpadx"), None);
    }

    #[test]
    fn finds_last_section_across_parents() {
        let nodes = kdl::parse(
            "config.kdl",
            "input { touchpad { tap; }; }\ninput { touchpad { natural-scroll; }; }\ninput { keyboard {}; }\n",
        )
        .unwrap();
        let found = find_last(&nodes, &["input", "touchpad"]).unwrap();
        assert_eq!(found.children[0].name, "natural-scroll");
    }
}
