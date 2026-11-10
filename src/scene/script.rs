//! The shell's scripts: JavaScript that builds and changes the scene, and reacts to its events.
//!
//! The shell is `~/.config/bento/shell/main.js` if it's there, or bento's own otherwise. Both
//! import bento's widgets from `bento:ui`. bento's own shell and widgets are built in, or read from
//! a folder that `debug { shell-scripts }` names, to work on them. A changed file is loaded again; a shell that fails to
//! load or throws leaves the last working one on screen, and the error is logged.
//!
//! Scripts never run per frame: only when something happens, like a click, a timer or bento's
//! state changing. A call that runs too long is stopped.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, Context as _};
use rquickjs::loader::{BuiltinLoader, BuiltinResolver};
use rquickjs::prelude::Opt;
use rquickjs::{CatchResultExt as _, Context, Function, Module, Persistent, Runtime};

use super::Scene;

/// bento's own shell.
const MAIN: &str = include_str!("../../resources/shell/main.js");
/// Its modules, each imported as `bento:` and its name: the widgets, and the shell's parts.
const MODULES: &[(&str, &str)] = &[
    ("ui", include_str!("../../resources/shell/ui.js")),
    (
        "calendar",
        include_str!("../../resources/shell/calendar.js"),
    ),
    ("control", include_str!("../../resources/shell/control.js")),
    (
        "launcher",
        include_str!("../../resources/shell/launcher.js"),
    ),
    ("toasts", include_str!("../../resources/shell/toasts.js")),
    (
        "workspace",
        include_str!("../../resources/shell/workspace.js"),
    ),
];

const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
const CALL_TIME_LIMIT: Duration = Duration::from_millis(200);

/// A loaded shell: its scene, and the script that drives it.
pub struct Shell {
    pub scene: Rc<RefCell<Scene>>,
    /// bento actions the shell asked to run, as `{ name, args, properties, window, workspace }`.
    actions: Rc<RefCell<Vec<serde_json::Value>>>,
    /// Commands for the data program, like `{ "cmd": "volume", "value": 0.5 }`.
    commands: Rc<RefCell<Vec<serde_json::Value>>>,
    timers: Rc<RefCell<Vec<(Instant, u32)>>>,
    /// The time the host last moved the shell on to, which timers count from when it's ahead of
    /// the clock on the wall, as in tests.
    clock: Rc<Cell<Instant>>,
    deadline: Rc<Cell<Option<Instant>>>,
    dispatch: Persistent<Function<'static>>,
    context: Context,
    _runtime: Runtime,
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell").field("scene", &self.scene).finish()
    }
}

/// The user's shell, if they have one.
pub fn user_path() -> Option<PathBuf> {
    let path = directories::BaseDirs::new()?
        .config_dir()
        .join("bento/shell/main.js");
    path.is_file().then_some(path)
}

/// The scripts in `dir`, sorted by name.
fn scripts_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "js"))
        .collect();
    files.sort();
    files
}

/// When the shell's files last changed, with bento's own read from `dir`, to load them again.
pub fn mtimes(dir: Option<&Path>) -> Vec<Option<SystemTime>> {
    let mtime = |path: PathBuf| path.metadata().ok()?.modified().ok();
    let mut files: Vec<PathBuf> = user_path().into_iter().collect();
    if let Some(dir) = dir {
        files.extend(scripts_in(dir));
    }
    files.into_iter().map(mtime).collect()
}

/// The shell's source and its modules by name, with bento's own read from `dir` if it's given:
/// `main.js` there, and every other script as the module of its name.
fn sources(dir: Option<&Path>) -> anyhow::Result<(String, Vec<(String, String)>)> {
    let read = |path: &Path| {
        std::fs::read_to_string(path).with_context(|| format!("error reading {}", path.display()))
    };
    let main = match (user_path(), dir) {
        (Some(path), _) => read(&path)?,
        (None, Some(dir)) => read(&dir.join("main.js"))?,
        (None, None) => String::from(MAIN),
    };
    let modules = match dir {
        Some(dir) => scripts_in(dir)
            .into_iter()
            .filter(|path| path.file_name().is_some_and(|name| name != "main.js"))
            .map(|path| {
                let name = path.file_stem().unwrap_or_default().to_string_lossy();
                Ok((name.into_owned(), read(&path)?))
            })
            .collect::<anyhow::Result<_>>()?,
        None => MODULES
            .iter()
            .map(|(name, source)| (String::from(*name), String::from(*source)))
            .collect(),
    };
    Ok((main, modules))
}

/// Where what scripts keep between runs is written, like which sound outputs are pinned. Tests
/// keep it in memory alone.
fn state_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    Some(
        directories::BaseDirs::new()?
            .state_dir()?
            .join("bento/shell.json"),
    )
}

impl Shell {
    /// Loads the user's shell, or bento's own, onto `outputs`, each its logical size, with
    /// bento's own shell and widgets read from `dir` if it's given.
    pub fn load(
        outputs: &[(String, (f32, f32))],
        dir: Option<&Path>,
        style: serde_json::Value,
    ) -> anyhow::Result<Self> {
        let (source, modules) = sources(dir)?;

        let mut scene = Scene::new();
        for (name, size) in outputs {
            scene.set_output(name, *size);
        }
        scene.style = style;
        let scene = Rc::new(RefCell::new(scene));

        let runtime = Runtime::new().context("error creating the JS runtime")?;
        runtime.set_memory_limit(MEMORY_LIMIT);
        runtime.set_max_stack_size(1024 * 1024);
        let mut resolver = BuiltinResolver::default();
        let mut loader = BuiltinLoader::default();
        for (name, module) in modules {
            let name = format!("bento:{name}");
            resolver.add_module(&name);
            loader.add_module(&name, module);
        }
        runtime.set_loader(resolver, loader);

        let deadline: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
        let deadline_ = deadline.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            deadline_.get().is_some_and(|d| d < Instant::now())
        })));

        let context = Context::full(&runtime).context("error creating the JS context")?;
        let timers: Rc<RefCell<Vec<(Instant, u32)>>> = Rc::default();
        let clock = Rc::new(Cell::new(Instant::now()));
        let actions: Rc<RefCell<Vec<serde_json::Value>>> = Rc::default();
        let commands: Rc<RefCell<Vec<serde_json::Value>>> = Rc::default();
        let apps: Rc<RefCell<super::apps::Apps>> = Rc::default();
        let next_timer = Rc::new(Cell::new(0u32));

        deadline.set(Some(Instant::now() + CALL_TIME_LIMIT * 5));
        let dispatch = context.with(|ctx| -> anyhow::Result<_> {
            let native = rquickjs::Object::new(ctx.clone())?;
            let js = |err: rquickjs::Error| anyhow!("{err}");

            let s = scene.clone();
            native.set(
                "create",
                Function::new(ctx.clone(), move |kind: String, props: String| {
                    let props = serde_json::from_str(&props).unwrap_or_default();
                    s.borrow_mut().create(&kind, &props)
                })
                .map_err(js)?,
            )?;
            let s = scene.clone();
            native.set(
                "set",
                Function::new(ctx.clone(), move |id: u32, props: String| {
                    let props = serde_json::from_str(&props).unwrap_or_default();
                    s.borrow_mut().set(id, &props);
                })
                .map_err(js)?,
            )?;
            let s = scene.clone();
            native.set(
                "append",
                Function::new(
                    ctx.clone(),
                    move |parent: u32, child: u32, index: Opt<i32>| {
                        let index = index.0.and_then(|i| usize::try_from(i).ok());
                        s.borrow_mut().append(parent, child, index);
                    },
                )
                .map_err(js)?,
            )?;
            let s = scene.clone();
            native.set(
                "reserve",
                Function::new(
                    ctx.clone(),
                    move |output: String, top: f64, right: f64, bottom: f64, left: f64| {
                        let edges = [top, right, bottom, left].map(|e| e.max(0.) as f32);
                        s.borrow_mut().reserve(&output, edges);
                    },
                )
                .map_err(js)?,
            )?;
            let s = scene.clone();
            native.set(
                "edit",
                Function::new(ctx.clone(), move |id: u32, key: String| {
                    let key = serde_json::from_str(&key).unwrap_or_default();
                    let state = s.borrow_mut().edit(id, &key);
                    state.map(|state| state.to_string())
                })
                .map_err(js)?,
            )?;
            let s = scene.clone();
            native.set(
                "frame",
                Function::new(ctx.clone(), move |id: u32| {
                    s.borrow()
                        .frame(id)
                        .map(|f| serde_json::json!(f).to_string())
                })
                .map_err(js)?,
            )?;
            let c = commands.clone();
            native.set(
                "data",
                Function::new(ctx.clone(), move |command: String| {
                    if let Ok(command) = serde_json::from_str(&command) {
                        c.borrow_mut().push(command);
                    }
                })
                .map_err(js)?,
            )?;
            let a = actions.clone();
            native.set(
                "action",
                Function::new(ctx.clone(), move |request: String| {
                    if let Ok(request) = serde_json::from_str(&request) {
                        a.borrow_mut().push(request);
                    }
                })
                .map_err(js)?,
            )?;
            let a = apps.clone();
            native.set(
                "apps",
                Function::new(ctx.clone(), move || {
                    serde_json::to_string(a.borrow_mut().list()).unwrap_or_default()
                })
                .map_err(js)?,
            )?;
            let a = apps.clone();
            native.set(
                "launch",
                Function::new(ctx.clone(), move |id: String| a.borrow_mut().launch(&id))
                    .map_err(js)?,
            )?;
            let s = scene.clone();
            native.set(
                "keyboard",
                Function::new(ctx.clone(), move |wants: bool| {
                    s.borrow_mut().wants_keyboard = wants;
                })
                .map_err(js)?,
            )?;
            let s = scene.clone();
            native.set(
                "style",
                Function::new(ctx.clone(), move || s.borrow().style.to_string()).map_err(js)?,
            )?;
            let s = scene.clone();
            native.set(
                "layout",
                Function::new(ctx.clone(), move |id: u32| {
                    s.borrow_mut()
                        .layout(id)
                        .map(|f| serde_json::json!(f).to_string())
                })
                .map_err(js)?,
            )?;
            let s = scene.clone();
            native.set(
                "remove",
                Function::new(ctx.clone(), move |id: u32| s.borrow_mut().remove(id)).map_err(js)?,
            )?;
            let s = scene.clone();
            native.set(
                "root",
                Function::new(ctx.clone(), move |output: String| {
                    s.borrow().root(&output).map_or(-1, i64::from)
                })
                .map_err(js)?,
            )?;
            let s = scene.clone();
            native.set(
                "outputs",
                Function::new(ctx.clone(), move || {
                    let scene = s.borrow();
                    serde_json::to_string(&scene.outputs().collect::<Vec<_>>()).unwrap_or_default()
                })
                .map_err(js)?,
            )?;
            let t = timers.clone();
            let c = clock.clone();
            native.set(
                "after",
                Function::new(ctx.clone(), move |ms: f64| {
                    let id = next_timer.get();
                    next_timer.set(id.wrapping_add(1));
                    let wait = Duration::from_secs_f64(ms.max(0.) / 1000.);
                    let now = c.get().max(Instant::now());
                    t.borrow_mut().push((now + wait, id));
                    id
                })
                .map_err(js)?,
            )?;
            let t = timers.clone();
            native.set(
                "cancel",
                Function::new(ctx.clone(), move |id: u32| {
                    t.borrow_mut().retain(|(_, timer)| *timer != id);
                })
                .map_err(js)?,
            )?;
            native.set(
                "log",
                Function::new(ctx.clone(), |message: String| {
                    info!("shell: {message}");
                })
                .map_err(js)?,
            )?;
            // What scripts keep between runs, as one JSON object.
            let state = state_path()
                .and_then(|path| std::fs::read_to_string(path).ok())
                .filter(|text| serde_json::from_str::<serde_json::Value>(text).is_ok())
                .unwrap_or_else(|| String::from("{}"));
            let state = Rc::new(RefCell::new(state));
            let kept = state.clone();
            native.set(
                "stored",
                Function::new(ctx.clone(), move || kept.borrow().clone()).map_err(js)?,
            )?;
            native.set(
                "store",
                Function::new(ctx.clone(), move |json: String| {
                    if serde_json::from_str::<serde_json::Value>(&json).is_err() {
                        return;
                    }
                    if let Some(path) = state_path() {
                        if let Some(parent) = path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if let Err(err) = std::fs::write(&path, &json) {
                            warn!("error keeping the shell's state: {err:?}");
                        }
                    }
                    *state.borrow_mut() = json;
                })
                .map_err(js)?,
            )?;
            ctx.globals().set("__bento", native)?;

            let module = Module::declare(ctx.clone(), "main.js", source)
                .catch(&ctx)
                .map_err(|err| anyhow!("{err}"))?;
            let (_, promise) = module.eval().catch(&ctx).map_err(|err| anyhow!("{err}"))?;
            promise
                .finish::<()>()
                .catch(&ctx)
                .map_err(|err| anyhow!("{err}"))?;

            let ui = Module::import(&ctx, "bento:ui")
                .catch(&ctx)
                .map_err(|err| anyhow!("{err}"))?
                .finish::<rquickjs::Object>()
                .catch(&ctx)
                .map_err(|err| anyhow!("{err}"))?;
            let dispatch: Function = ui.get("__dispatch")?;
            Ok(Persistent::save(&ctx, dispatch))
        });
        deadline.set(None);

        Ok(Self {
            scene,
            actions,
            commands,
            timers,
            clock,
            deadline,
            dispatch: dispatch?,
            context,
            _runtime: runtime,
        })
    }

    /// Hands the script an event, as JSON: `{ "type": ..., ... }`.
    pub fn send(&self, event: &serde_json::Value) {
        self.deadline.set(Some(Instant::now() + CALL_TIME_LIMIT));
        let res = self.context.with(|ctx| -> anyhow::Result<()> {
            let dispatch = self.dispatch.clone().restore(&ctx)?;
            dispatch
                .call::<_, ()>((event.to_string(),))
                .catch(&ctx)
                .map_err(|err| anyhow!("{err}"))?;
            // Promises the event started.
            while ctx.execute_pending_job() {}
            Ok(())
        });
        self.deadline.set(None);
        if let Err(err) = res {
            // A script that throws is a bug in bento's own shell, which tests catch.
            if cfg!(test) {
                panic!("the shell failed handling {event}: {err:?}");
            }
            warn!("the shell failed handling {event}: {err:?}");
        }
    }

    /// The bento actions the shell asked to run since this was last asked.
    pub fn take_actions(&self) -> Vec<serde_json::Value> {
        std::mem::take(&mut self.actions.borrow_mut())
    }

    /// The commands for the data program since this was last asked.
    pub fn take_commands(&self) -> Vec<serde_json::Value> {
        std::mem::take(&mut self.commands.borrow_mut())
    }

    /// When the next timer is due.
    pub fn next_timer(&self) -> Option<Instant> {
        self.timers.borrow().iter().map(|(at, _)| *at).min()
    }

    /// Runs the timers that are due.
    pub fn run_timers(&self, now: Instant) {
        self.clock.set(self.clock.get().max(now));
        let due: Vec<u32> = {
            let mut timers = self.timers.borrow_mut();
            let due = timers
                .iter()
                .filter(|(at, _)| *at <= now)
                .map(|(_, id)| *id)
                .collect();
            timers.retain(|(at, _)| *at > now);
            due
        };
        for id in due {
            self.send(&serde_json::json!({ "type": "timer", "id": id }));
        }
    }
}
