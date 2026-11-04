//! The config behind the app.
//!
//! The app edits bento's `config.kdl` in place, changing only the values it sets (see `kdl.rs`).
//! Values shown come from the whole config as bento parses it, includes and all. bento watches the
//! file and applies changes itself.

use std::cell::{Cell, Ref, RefCell};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};
use niri_config::Config;

use crate::kdl::{Document, Value};

const SAVE_DELAY: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub struct Store(Rc<Inner>);

struct Inner {
    config_path: PathBuf,
    config: RefCell<Config>,
    /// Edits not written yet, so dragging a slider writes once.
    pending: RefCell<Option<Document>>,
    status: RefCell<Status>,
    /// What the app last wrote, to tell its own writes from someone else's.
    last_written: RefCell<Option<String>>,
    pending_save: RefCell<Option<glib::SourceId>>,
    pending_reload: RefCell<Option<glib::SourceId>>,
    listeners: RefCell<Vec<Rc<dyn Fn()>>>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    notifying: Cell<bool>,
}

/// Problems the app shows in a banner.
#[derive(Debug, Clone, Default)]
pub struct Status {
    /// The config doesn't load, so bento keeps running its last good one, and the app can't edit
    /// a file it can't parse.
    pub config_error: Option<String>,
    /// The last change would have broken the config, so it wasn't saved.
    pub write_error: Option<String>,
}

impl Store {
    pub fn load() -> Self {
        Self::load_from(config_path())
    }

    pub fn load_from(config_path: PathBuf) -> Self {
        let store = Self(Rc::new(Inner {
            config_path,
            config: RefCell::new(Config::default()),
            pending: RefCell::new(None),
            status: RefCell::new(Status::default()),
            last_written: RefCell::new(None),
            pending_save: RefCell::new(None),
            pending_reload: RefCell::new(None),
            listeners: RefCell::new(Vec::new()),
            monitor: RefCell::new(None),
            notifying: Cell::new(false),
        }));
        store.read_config();
        store.watch();
        store
    }

    pub fn config_path(&self) -> &Path {
        &self.0.config_path
    }

    pub fn config(&self) -> Ref<'_, Config> {
        self.0.config.borrow()
    }

    pub fn status(&self) -> Status {
        self.0.status.borrow().clone()
    }

    /// Calls `f` whenever the config changes, from the app or from outside.
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

    /// Changes the config, and saves it shortly after.
    pub fn edit(&self, f: impl FnOnce(&mut Document)) {
        let mut pending = self.0.pending.borrow_mut();
        if pending.is_none() {
            // Start from the file as it is now, so edits made elsewhere since are kept.
            let text = match fs::read_to_string(&self.0.config_path) {
                Ok(text) => text,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(err) => {
                    drop(pending);
                    self.fail(format!(
                        "Couldn't read {}: {err}",
                        self.0.config_path.display()
                    ));
                    return;
                }
            };
            match Document::parse(text) {
                Ok(doc) => *pending = Some(doc),
                Err(err) => {
                    drop(pending);
                    self.fail(describe(&err));
                    return;
                }
            }
        }
        f(pending.as_mut().unwrap());
        drop(pending);
        self.schedule_save();
    }

    fn fail(&self, err: String) {
        self.0.status.borrow_mut().write_error = Some(err);
        self.notify();
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
        let Some(doc) = self.0.pending.take() else {
            return;
        };
        let result = self.write(doc.text());
        self.0.status.borrow_mut().write_error = result.err();
        self.read_config();
        self.notify();
    }

    fn write(&self, text: &str) -> Result<(), String> {
        // Check the whole config first, so a bad value never reaches bento.
        if let Err(err) = Config::parse(&self.0.config_path, text).config {
            return Err(describe(&err));
        }

        // Write through a symlink rather than replacing it, for a config kept in a dotfiles
        // repo. The temporary file sits next to the real one so the rename stays on one disk.
        let target =
            fs::canonicalize(&self.0.config_path).unwrap_or_else(|_| self.0.config_path.clone());
        let tmp = target.with_extension("kdl.tmp");
        fs::write(&tmp, text)
            .and_then(|()| fs::rename(&tmp, &target))
            .map_err(|err| format!("Couldn't write {}: {err}", target.display()))?;
        self.0.last_written.replace(Some(text.to_owned()));
        Ok(())
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
    }

    /// Re-reads the config.
    pub fn reload(&self) {
        self.read_config();
        self.notify();
    }

    fn watch(&self) {
        let file = gio::File::for_path(&self.0.config_path);
        let Ok(monitor) = file.monitor_file(
            gio::FileMonitorFlags::WATCH_MOVES,
            None::<&gio::Cancellable>,
        ) else {
            return;
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
        self.0.monitor.replace(Some(monitor));
    }

    fn schedule_reload(&self) {
        if let Some(source) = self.0.pending_reload.take() {
            source.remove();
        }
        let store = self.clone();
        let source = glib::timeout_add_local_once(Duration::from_millis(150), move || {
            store.0.pending_reload.take();
            let on_disk = fs::read_to_string(&store.0.config_path).ok();
            if on_disk.is_none() || *store.0.last_written.borrow() != on_disk {
                store.reload();
            }
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
    fn edits_land_in_the_config() {
        let dir = std::env::temp_dir().join(format!("bento-settings-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("config.kdl");
        let text = "// Touchpad.\ninput {\n    touchpad {\n        tap\n        natural-scroll // yes\n    }\n}\n\nlayout {\n    gaps 16\n}\n";
        fs::write(&config_path, text).unwrap();

        let store = Store::load_from(config_path.clone());
        assert!(store.config().input.touchpad.natural_scroll);

        let natural_scroll = Setting::switch("", "input.touchpad.natural-scroll", |c| {
            c.input.touchpad.natural_scroll
        })
        .style(BoolStyle::Presence);
        let focus_ring = Setting::switch("", "layout.focus-ring", |c| !c.layout.focus_ring.off)
            .style(BoolStyle::OnOff);
        let numlock = Setting::switch("", "input.keyboard.numlock", |c| c.input.keyboard.numlock);
        store.edit(|doc| natural_scroll.write(doc, Input::Bool(false)));
        store.edit(|doc| focus_ring.write(doc, Input::Bool(false)));
        store.edit(|doc| numlock.write(doc, Input::Bool(false)));
        store.flush();

        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            "// Touchpad.\ninput {\n    touchpad {\n        tap\n    }\n    keyboard {\n        numlock false\n    }\n}\n\nlayout {\n    gaps 16\n    focus-ring {\n        off\n    }\n}\n"
        );
        let config = store.config();
        assert!(config.input.touchpad.tap);
        assert!(!config.input.touchpad.natural_scroll);
        assert!(config.layout.focus_ring.off);
        drop(config);

        // Turning it back on swaps `off` for `on`.
        store.edit(|doc| focus_ring.write(doc, Input::Bool(true)));
        store.flush();
        assert!(!store.config().layout.focus_ring.off);
        assert!(fs::read_to_string(&config_path)
            .unwrap()
            .contains("    focus-ring {\n        on\n    }\n"));

        // A value bento rejects isn't written.
        let gaps = Setting::number("", "layout.gaps", |c| c.layout.gaps);
        let before = fs::read_to_string(&config_path).unwrap();
        store.edit(|doc| gaps.write(doc, Input::Number(-5.)));
        store.flush();
        assert_eq!(fs::read_to_string(&config_path).unwrap(), before);
        assert!(store.status().write_error.is_some());

        fs::remove_dir_all(&dir).unwrap();
    }
}
