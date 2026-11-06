//! The config behind the app.
//!
//! The app edits bento's `config.kdl` and the running engine's own file in place, changing only
//! the values it sets (see `kdl.rs`). Values shown come from the whole config as bento parses it,
//! includes, engine file and all. bento watches the files and applies changes itself.
//!
//! Parsing the config is slow next to drawing a frame, so the window never does it. Edits queue up
//! and are written together on a background thread a moment after the last one.

use std::cell::{Cell, Ref, RefCell};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};
use niri_config::{engine_config_path, Config};

use crate::kdl::{Document, Value};

/// How long after the last change to save, so dragging a slider writes once.
const SAVE_DELAY: Duration = Duration::from_millis(150);

/// A change to the config, keyed by what it changes so a newer one replaces an older one.
struct Edit {
    key: String,
    file: File,
    f: Box<dyn FnOnce(&mut Document) + Send>,
}

/// Which file an edit goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum File {
    /// bento's `config.kdl`.
    Main,
    /// The running engine's own file, with its options and binds.
    Engine,
}

#[derive(Clone)]
pub struct Store(Rc<Inner>);

struct Inner {
    config_path: PathBuf,
    config: RefCell<Config>,
    /// Edits waiting for the next save.
    queued: RefCell<Vec<Edit>>,
    /// Keys of the edits being saved right now.
    saving: RefCell<Option<Vec<String>>>,
    status: RefCell<Status>,
    /// What the app last wrote to each file, to tell its own writes from someone else's.
    last_written: RefCell<HashMap<PathBuf, String>>,
    pending_save: RefCell<Option<glib::SourceId>>,
    pending_reload: RefCell<Option<glib::SourceId>>,
    listeners: RefCell<Vec<Rc<dyn Fn()>>>,
    /// The config and the running engine's file, with their monitors.
    monitors: RefCell<Vec<(PathBuf, gio::FileMonitor)>>,
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

/// What a save on the background thread comes back with.
struct Saved {
    /// Each file written, with its new text.
    written: Vec<(PathBuf, String)>,
    config: Config,
}

impl Store {
    pub fn load() -> Self {
        Self::load_from(config_path())
    }

    pub fn load_from(config_path: PathBuf) -> Self {
        let store = Self(Rc::new(Inner {
            config_path,
            config: RefCell::new(Config::default()),
            queued: RefCell::new(Vec::new()),
            saving: RefCell::new(None),
            status: RefCell::new(Status::default()),
            last_written: RefCell::new(HashMap::new()),
            pending_save: RefCell::new(None),
            pending_reload: RefCell::new(None),
            listeners: RefCell::new(Vec::new()),
            monitors: RefCell::new(Vec::new()),
            notifying: Cell::new(false),
        }));
        let result = load(&store.0.config_path);
        store.apply_load(result);
        store.watch();
        store
    }

    pub fn config_path(&self) -> &Path {
        &self.0.config_path
    }

    /// The running engine's own file, which may not exist yet.
    pub fn engine_path(&self) -> PathBuf {
        engine_config_path(&self.0.config_path, &self.0.config.borrow().tiling.layout)
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

    /// Whether a change to `key` hasn't reached the config yet. Rows showing it keep what the
    /// person set instead of the old value.
    pub fn is_pending(&self, key: &str) -> bool {
        self.0.queued.borrow().iter().any(|edit| edit.key == key)
            || self
                .0
                .saving
                .borrow()
                .as_ref()
                .is_some_and(|keys| keys.iter().any(|k| k == key))
    }

    /// Queues a change to the config, saved shortly after the last one. `key` names what it
    /// changes, so a newer change to the same thing replaces the older one.
    pub fn edit(&self, key: &str, f: impl FnOnce(&mut Document) + Send + 'static) {
        self.edit_in(File::Main, key, f);
    }

    /// Queues a change like [`Store::edit`], to `file`.
    pub fn edit_in(&self, file: File, key: &str, f: impl FnOnce(&mut Document) + Send + 'static) {
        let mut queued = self.0.queued.borrow_mut();
        queued.retain(|edit| edit.key != key);
        queued.push(Edit {
            key: key.to_owned(),
            file,
            f: Box::new(f),
        });
        drop(queued);
        self.schedule_save();
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

    fn save(&self) {
        // One save at a time; edits made meanwhile go in the next one.
        if self.0.saving.borrow().is_some() || self.0.queued.borrow().is_empty() {
            return;
        }
        let edits = self.0.queued.take();
        self.0
            .saving
            .replace(Some(edits.iter().map(|edit| edit.key.clone()).collect()));

        let path = self.0.config_path.clone();
        let store = self.clone();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || save(&path, edits))
                .await
                .unwrap_or_else(|_| Err(String::from("Saving stopped unexpectedly")));
            store.0.saving.replace(None);
            store.apply_save(result);
            if !store.0.queued.borrow().is_empty() {
                store.schedule_save();
            }
        });
    }

    fn apply_save(&self, result: Result<Saved, String>) {
        match result {
            Ok(saved) => {
                self.0.last_written.borrow_mut().extend(saved.written);
                self.0.config.replace(saved.config);
                let mut status = self.0.status.borrow_mut();
                status.write_error = None;
                status.config_error = None;
            }
            Err(err) => self.0.status.borrow_mut().write_error = Some(err),
        }
        self.watch();
        self.notify();
    }

    /// Saves queued changes right away, on this thread. For closing the window and for tests.
    pub fn flush(&self) {
        if let Some(source) = self.0.pending_save.take() {
            source.remove();
        }
        let edits = self.0.queued.take();
        if !edits.is_empty() {
            let result = save(&self.0.config_path, edits);
            self.apply_save(result);
        }
    }

    fn apply_load(&self, result: Result<Config, String>) {
        let mut status = self.0.status.borrow_mut();
        match result {
            Ok(config) => {
                self.0.config.replace(config);
                status.config_error = None;
            }
            Err(err) => status.config_error = Some(err),
        }
        drop(status);
        self.watch();
    }

    /// Re-reads the config, on a background thread.
    pub fn reload(&self) {
        let path = self.0.config_path.clone();
        let store = self.clone();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || load(&path))
                .await
                .unwrap_or_else(|_| Err(String::from("Loading stopped unexpectedly")));
            store.apply_load(result);
            store.notify();
        });
    }

    /// Watches the config and the running engine's file, which changes with the engine.
    fn watch(&self) {
        let paths = [self.0.config_path.clone(), self.engine_path()];
        let mut monitors = self.0.monitors.borrow_mut();
        monitors.retain(|(path, _)| paths.contains(path));
        for path in paths {
            if monitors.iter().any(|(watched, _)| *watched == path) {
                continue;
            }
            if let Some(monitor) = self.monitor(&path) {
                monitors.push((path, monitor));
            }
        }
    }

    fn monitor(&self, path: &Path) -> Option<gio::FileMonitor> {
        let file = gio::File::for_path(path);
        let monitor = file
            .monitor_file(
                gio::FileMonitorFlags::WATCH_MOVES,
                None::<&gio::Cancellable>,
            )
            .ok()?;
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
        Some(monitor)
    }

    fn schedule_reload(&self) {
        if let Some(source) = self.0.pending_reload.take() {
            source.remove();
        }
        let store = self.clone();
        let source = glib::timeout_add_local_once(Duration::from_millis(150), move || {
            store.0.pending_reload.take();
            let last_written = store.0.last_written.borrow();
            let changed = store.0.monitors.borrow().iter().any(|(path, _)| {
                let on_disk = fs::read_to_string(path).ok();
                on_disk.as_ref() != last_written.get(path)
            });
            drop(last_written);
            if changed {
                store.reload();
            }
        });
        self.0.pending_reload.replace(Some(source));
    }
}

/// Applies edits to the config and the engine's file and writes them, if bento accepts the result.
/// Runs off the main thread.
fn save(path: &Path, edits: Vec<Edit>) -> Result<Saved, String> {
    let (engine_edits, main_edits): (Vec<_>, Vec<_>) = edits
        .into_iter()
        .partition(|edit| edit.file == File::Engine);
    let engine_changes = !engine_edits.is_empty();
    let main = apply(path, main_edits)?;

    // The engine's file is known once the config says which engine runs, which an edit may have
    // just changed.
    let mut engine = None;
    let parsed = Config::parse_with_engine(path, &main, |engine_path| {
        let text = apply(engine_path, engine_edits);
        let result = text.clone().map_err(std::io::Error::other);
        engine = Some((engine_path.to_owned(), text));
        result
    });
    let engine = match engine {
        Some((path, text)) => Some((path, text?)),
        None => None,
    };
    // Check the whole config first, so a bad value never reaches bento.
    let config = parsed.config.map_err(|err| describe(&err))?;

    let mut written = vec![(path.to_owned(), main)];
    // Only an edit creates the engine's file.
    written.extend(engine.filter(|_| engine_changes));
    for (path, text) in &written {
        write(path, text)?;
    }
    Ok(Saved { written, config })
}

/// Applies edits to the file at `path` as it is now, so changes made elsewhere since are kept.
fn apply(path: &Path, edits: Vec<Edit>) -> Result<String, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(format!("Couldn't read {}: {err}", path.display())),
    };
    if edits.is_empty() {
        return Ok(text);
    }
    let mut doc = Document::parse(text).map_err(|err| describe(&err))?;
    for edit in edits {
        (edit.f)(&mut doc);
    }
    Ok(doc.text().to_owned())
}

/// Writes `text` to `path` if it changed, creating the engine's directory if needed.
fn write(path: &Path, text: &str) -> Result<(), String> {
    if fs::read_to_string(path).is_ok_and(|on_disk| on_disk == text) {
        return Ok(());
    }
    // Write through a symlink rather than replacing it, for a config kept in a dotfiles repo. The
    // temporary file sits next to the real one so the rename stays on one disk.
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    let tmp = target.with_extension("kdl.tmp");
    target
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(&tmp, text))
        .and_then(|()| fs::rename(&tmp, &target))
        .map_err(|err| format!("Couldn't write {}: {err}", target.display()))
}

fn load(path: &Path) -> Result<Config, String> {
    Config::load(path)
        .config
        .map_err(|err| describe(err.as_ref()))
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

/// A plain value for a layout engine option or action argument, or `None` for a list.
pub fn option_value(value: &niri_ipc::LayoutEngineOptionValue) -> Option<Value> {
    use niri_ipc::LayoutEngineOptionValue as OptionValue;

    Some(match value {
        OptionValue::Boolean(x) => Value::Bool(*x),
        OptionValue::Number(x) if x.fract() == 0. => Value::Int(*x as i64),
        OptionValue::Number(x) => Value::Float(*x),
        OptionValue::String(x) => Value::Str(x.clone()),
        OptionValue::List(_) => return None,
    })
}

/// The values an option is written with: one for a plain option, one each for a list, none for a
/// size left to the window.
pub fn option_values(value: &Option<niri_ipc::LayoutEngineOptionValue>) -> Vec<Value> {
    use niri_ipc::LayoutEngineOptionValue as OptionValue;

    match value {
        None => Vec::new(),
        Some(OptionValue::List(items)) => items.iter().filter_map(option_value).collect(),
        Some(value) => option_value(value).into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BoolStyle, Input, Setting};

    /// Edits run on another thread, so the settings they use live for the whole program.
    fn leak(setting: Setting) -> &'static Setting {
        Box::leak(Box::new(setting))
    }

    #[test]
    fn edits_land_in_the_config() {
        let dir = std::env::temp_dir().join(format!("bento-settings-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("config.kdl");
        let text = "// Touchpad.\ninput {\n    touchpad {\n        tap\n        natural-scroll // yes\n    }\n}\n\nlayout {\n    gaps 16\n}\n";
        fs::write(&config_path, text).unwrap();

        let store = Store::load_from(config_path.clone());
        assert!(store.config().input.touchpad.natural_scroll);

        let natural_scroll = leak(
            Setting::switch("", "input.touchpad.natural-scroll", |c| {
                c.input.touchpad.natural_scroll
            })
            .style(BoolStyle::Presence),
        );
        let focus_ring = leak(
            Setting::switch("", "layout.focus-ring", |c| !c.layout.focus_ring.off)
                .style(BoolStyle::OnOff),
        );
        let numlock = leak(Setting::switch("", "input.keyboard.numlock", |c| {
            c.input.keyboard.numlock
        }));
        store.edit(natural_scroll.path, move |doc| {
            natural_scroll.write(doc, Input::Bool(false))
        });
        store.edit(focus_ring.path, move |doc| {
            focus_ring.write(doc, Input::Bool(false))
        });
        store.edit(numlock.path, move |doc| {
            numlock.write(doc, Input::Bool(false))
        });
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
        store.edit(focus_ring.path, move |doc| {
            focus_ring.write(doc, Input::Bool(true))
        });
        store.flush();
        assert!(!store.config().layout.focus_ring.off);
        assert!(fs::read_to_string(&config_path)
            .unwrap()
            .contains("    focus-ring {\n        on\n    }\n"));

        // A value bento rejects isn't written.
        let gaps = leak(Setting::number("", "layout.gaps", |c| c.layout.gaps));
        let before = fs::read_to_string(&config_path).unwrap();
        store.edit(gaps.path, move |doc| gaps.write(doc, Input::Number(-5.)));
        store.flush();
        assert_eq!(fs::read_to_string(&config_path).unwrap(), before);
        assert!(store.status().write_error.is_some());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn engine_edits_land_in_the_engine_file() {
        let dir =
            std::env::temp_dir().join(format!("bento-settings-engine-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("config.kdl");
        let text = "tiling {\n    layout \"niri\"\n}\n";
        fs::write(&config_path, text).unwrap();
        let engine_path = dir.join("engines").join("niri.kdl");

        let store = Store::load_from(config_path.clone());
        assert_eq!(store.engine_path(), engine_path);

        // Editing the main config leaves the engine's file alone.
        store.edit("layout.gaps", |doc| {
            doc.set(&["layout", "gaps"], vec![Value::Int(8)])
        });
        store.flush();
        assert!(!engine_path.exists());

        store.edit_in(File::Engine, "options.gravity", |doc| {
            doc.set(&["options", "gravity"], vec![Value::Bool(true)])
        });
        store.flush();
        assert_eq!(
            fs::read_to_string(&engine_path).unwrap(),
            "options {\n    gravity true\n}\n"
        );
        assert!(!fs::read_to_string(&config_path)
            .unwrap()
            .contains("gravity"));
        assert_eq!(
            store.config().tiling.options,
            [(
                String::from("gravity"),
                vec![niri_config::TilingOption::Bool(true)]
            )]
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn engine_edits_keep_window_rules_and_typed_options() {
        use niri_ipc::LayoutEngineOptionValue as OptionValue;

        let dir = std::env::temp_dir().join(format!("bento-settings-rules-{}", std::process::id()));
        fs::create_dir_all(dir.join("engines")).unwrap();
        let config_path = dir.join("config.kdl");
        fs::write(&config_path, "tiling {\n    layout \"niri\"\n}\n").unwrap();
        let engine_path = dir.join("engines").join("niri.kdl");
        let engine = "\
// Mine.
options {
    default-column-width
    center-focused-column \"always\"
}

// Terminals open wide.
window-rule {
    match app-id=\"kitty\"
    default-column-width \"67%\"
}

binds {
    Mod+L { focus-column-right; }
}
";
        fs::write(&engine_path, engine).unwrap();

        let store = Store::load_from(config_path);
        assert_eq!(store.config().tiling.window_rules.len(), 1);
        assert!(store.config().tiling.options[0].1.is_empty());

        // A list is written as several values, and a size with no value as a bare name.
        let widths = Some(OptionValue::List(vec![
            OptionValue::String(String::from("33%")),
            OptionValue::String(String::from("800")),
        ]));
        store.edit_in(File::Engine, "options.preset-column-widths", move |doc| {
            doc.set(&["options", "preset-column-widths"], option_values(&widths))
        });
        store.edit_in(File::Engine, "options.default-column-width", |doc| {
            doc.set(&["options", "default-column-width"], option_values(&None))
        });
        store.flush();

        let text = fs::read_to_string(&engine_path).unwrap();
        assert_eq!(
            text,
            engine.replace(
                "    center-focused-column \"always\"\n",
                "    center-focused-column \"always\"\n    preset-column-widths \"33%\" \"800\"\n"
            )
        );
        let config = store.config();
        assert_eq!(config.tiling.window_rules.len(), 1);
        assert_eq!(config.tiling.options.len(), 3);
        assert_eq!(config.tiling.options[2].1.len(), 2);
        drop(config);

        fs::remove_dir_all(&dir).unwrap();
    }
}
