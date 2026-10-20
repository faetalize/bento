//! The JS engine that runs tiling layouts.
//!
//! There is one engine per thread, holding the current layout module. Each workspace has its own
//! instance of the layout in it. bento and the layout talk in JSON through `prelude.js`, see
//! `resources/layouts/bento-layout.d.ts` for the API.
//!
//! The engine has no access to files, the network or other modules, and every call has a time
//! limit, so a broken layout can't hang the compositor.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, Context as _};
use niri_config::{Tiling, TilingLayout, TilingOption};
use rquickjs::{CatchResultExt as _, Context, Function, Module, Object, Persistent, Runtime};
use serde::Deserialize;

const PRELUDE: &str = include_str!("../../../resources/layouts/prelude.js");

/// Layouts that ship with bento.
const BUILT_IN: &[(&str, &str)] = &[("fallback", FALLBACK)];

/// The layout used when the configured one fails to load: windows side by side.
const FALLBACK: &str = r#"
export const meta = { api: 1, name: "fallback" };
export default function create(ws) {
    const order = [];
    const layout = () => {
        const gaps = ws.config.gaps;
        const area = ws.view.workingArea;
        const w = Math.max(1, (area.w - gaps) / 2 - gaps);
        order.forEach((win, i) => {
            const x = area.x + gaps + i * (w + gaps);
            ws.place(win, { x, y: area.y + gaps, w, h: area.h - gaps * 2 });
        });
    };
    const show = (win) => {
        const area = ws.view.workingArea;
        const rect = win.rect;
        if (rect.x < ws.camera.x + area.x || ws.camera.x + area.x + area.w < rect.x + rect.w) {
            ws.moveCamera({ x: rect.x - area.x - ws.config.gaps });
        }
    };
    return {
        windowAdded(win, how) {
            order.push(win);
            layout();
            if (how.activate || ws.focused === null) {
                ws.focus(win);
                show(win);
            }
        },
        windowRemoved(win) {
            const i = order.indexOf(win);
            order.splice(i, 1);
            layout();
            if (ws.focused === null && order.length > 0) {
                ws.focus(order[Math.min(i, order.length - 1)]);
            }
        },
        activate(win) {
            ws.focus(win);
            show(win);
        },
        action(name) {
            const i = order.indexOf(ws.focused);
            const next = { "focus-column-left": i - 1, "focus-column-right": i + 1 }[name];
            if (next === undefined || next < 0 || next >= order.length) {
                return false;
            }
            ws.focus(order[next]);
            show(order[next]);
            return true;
        },
    };
}
"#;

/// Time a layout call may take before it's stopped.
const CALL_TIME_LIMIT: Duration = Duration::from_millis(250);

/// Memory the layout may use.
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    static NEXT_INSTANCE: Cell<u64> = const { Cell::new(1) };
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

/// What the layout says about itself when it loads.
#[derive(Debug, Clone, Deserialize)]
pub struct LayoutMeta {
    pub name: String,
    pub actions: Vec<String>,
    pub options: HashMap<String, String>,
}

struct Engine {
    /// Config the engine was loaded for.
    config: Tiling,
    /// Modification time of the layout file, to reload it when it changes.
    mtime: Option<SystemTime>,
    /// Source of the layout, to tell whether a reload changed anything.
    source: String,
    meta: LayoutMeta,
    /// Increases with every load; instances from an older generation are gone.
    generation: u64,
    deadline: Rc<Cell<Option<Instant>>>,
    // Field order matters: the functions have to go before the context and runtime.
    handle: Persistent<Function<'static>>,
    save_all: Persistent<Function<'static>>,
    context: Context,
    _runtime: Runtime,
    /// State that instances saved before the previous layout was unloaded, by instance id, with
    /// the name of the layout that saved it.
    saved: HashMap<u64, (String, serde_json::Value)>,
}

impl Engine {
    fn load(config: &Tiling, source: String, name: &str) -> anyhow::Result<Self> {
        let runtime = Runtime::new().context("error creating the JS runtime")?;
        runtime.set_memory_limit(MEMORY_LIMIT);
        runtime.set_max_stack_size(1024 * 1024);

        let deadline: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
        let deadline_ = deadline.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            deadline_
                .get()
                .is_some_and(|deadline| deadline < Instant::now())
        })));

        let context = Context::full(&runtime).context("error creating the JS context")?;

        deadline.set(Some(Instant::now() + CALL_TIME_LIMIT));
        let res = context.with(|ctx| -> anyhow::Result<_> {
            let eval = |name: &str, source: &str| -> anyhow::Result<Object> {
                let module = Module::declare(ctx.clone(), name, source)
                    .catch(&ctx)
                    .map_err(|err| anyhow!("{err}"))?;
                let (module, promise) =
                    module.eval().catch(&ctx).map_err(|err| anyhow!("{err}"))?;
                promise
                    .finish::<()>()
                    .catch(&ctx)
                    .map_err(|err| anyhow!("{err}"))?;
                module.namespace().map_err(|err| anyhow!("{err}"))
            };

            let prelude = eval("bento:prelude", PRELUDE).context("error loading the prelude")?;
            let layout = eval(name, &source)?;

            let load: Function = prelude.get("load")?;
            let meta: String = load
                .call((layout,))
                .catch(&ctx)
                .map_err(|err| anyhow!("{err}"))?;
            let meta: LayoutMeta =
                serde_json::from_str(&meta).context("error reading the layout metadata")?;

            let handle: Function = prelude.get("handle")?;
            let save_all: Function = prelude.get("saveAll")?;
            Ok((
                meta,
                Persistent::save(&ctx, handle),
                Persistent::save(&ctx, save_all),
            ))
        });
        deadline.set(None);
        let (meta, handle, save_all) = res?;

        let generation = GENERATION.get() + 1;
        GENERATION.set(generation);

        Ok(Self {
            config: config.clone(),
            mtime: file_mtime(config),
            source,
            meta,
            generation,
            deadline,
            handle,
            save_all,
            context,
            _runtime: runtime,
            saved: HashMap::new(),
        })
    }

    /// Saves the state of every instance, by instance id.
    fn save_all(&self) -> HashMap<u64, (String, serde_json::Value)> {
        self.deadline.set(Some(Instant::now() + CALL_TIME_LIMIT));
        let res = self.context.with(|ctx| {
            let save_all = self.save_all.clone().restore(&ctx)?;
            save_all
                .call::<_, String>(())
                .catch(&ctx)
                .map_err(|err| anyhow!("{err}"))
        });
        self.deadline.set(None);

        let states: HashMap<u64, serde_json::Value> = match res
            .and_then(|json| serde_json::from_str(&json).context("error reading saved states"))
        {
            Ok(states) => states,
            Err(err) => {
                warn!("error saving tiling layout state: {err:?}");
                HashMap::new()
            }
        };
        let name = &self.meta.name;
        states
            .into_iter()
            .map(|(id, state)| (id, (name.clone(), state)))
            .collect()
    }

    fn call(&self, event: &str) -> anyhow::Result<String> {
        self.deadline.set(Some(Instant::now() + CALL_TIME_LIMIT));
        let res = self.context.with(|ctx| {
            let handle = self.handle.clone().restore(&ctx)?;
            handle
                .call::<_, String>((event,))
                .catch(&ctx)
                .map_err(|err| anyhow!("{err}"))
        });
        self.deadline.set(None);
        res
    }
}

/// Modification time of the layout file in `config`, if it's a file.
fn file_mtime(config: &Tiling) -> Option<SystemTime> {
    match &config.layout {
        TilingLayout::BuiltIn(_) => None,
        TilingLayout::File(path) => std::fs::metadata(path).and_then(|m| m.modified()).ok(),
    }
}

/// Reads the source of the layout in `config`.
fn read_source(config: &Tiling) -> anyhow::Result<(String, String)> {
    match &config.layout {
        TilingLayout::BuiltIn(name) => {
            let (_, source) = BUILT_IN
                .iter()
                .find(|(built_in, _)| built_in == name)
                .ok_or_else(|| anyhow!("there's no built-in layout called {name:?}"))?;
            Ok((name.clone(), String::from(*source)))
        }
        TilingLayout::File(path) => {
            let source = std::fs::read_to_string(path)
                .with_context(|| format!("error reading {}", path.display()))?;
            Ok((path.display().to_string(), source))
        }
    }
}

/// Makes sure the engine runs the layout from `config`, loading it if it changed, and returns
/// the engine generation.
///
/// If the layout fails to load, keeps the previous one, or falls back to a simple built-in one.
pub fn ensure_loaded(config: &Tiling) -> u64 {
    ENGINE.with_borrow_mut(|engine| {
        if let Some(engine) = engine.as_ref() {
            if engine.config == *config && engine.mtime == file_mtime(config) {
                return engine.generation;
            }
        }

        let loaded = read_source(config).and_then(|(name, source)| {
            if let Some(engine) = engine.as_ref() {
                if engine.source == source {
                    return Ok(None);
                }
            }
            Engine::load(config, source, &name).map(Some)
        });

        match loaded {
            Ok(Some(mut new)) => {
                if let Some(old) = engine.take() {
                    let saved = old.save_all();
                    new.saved = old.saved;
                    new.saved.extend(saved);
                }
                info!("loaded tiling layout {}", new.meta.name);
                *engine = Some(new);
            }
            Ok(None) => {
                // Same source: only the options changed, which instances pick up on their own.
                let engine = engine.as_mut().unwrap();
                engine.config = config.clone();
                engine.mtime = file_mtime(config);
            }
            Err(err) => {
                warn!("error loading tiling layout: {err:?}");
                if engine.is_none() {
                    let fallback = Tiling {
                        layout: TilingLayout::BuiltIn(String::from("fallback")),
                        options: Vec::new(),
                    };
                    let new = Engine::load(&fallback, String::from(FALLBACK), "fallback")
                        .expect("the fallback layout must load");
                    *engine = Some(Engine {
                        config: config.clone(),
                        ..new
                    });
                } else {
                    // Keep running the previous layout, but don't retry until something changes.
                    let engine = engine.as_mut().unwrap();
                    engine.config = config.clone();
                    engine.mtime = file_mtime(config);
                }
            }
        }

        engine.as_ref().unwrap().generation
    })
}

/// Current engine generation, or 0 if no layout is loaded.
pub fn generation() -> u64 {
    ENGINE.with_borrow(|engine| engine.as_ref().map_or(0, |engine| engine.generation))
}

/// Metadata of the current layout.
pub fn meta() -> Option<LayoutMeta> {
    ENGINE.with_borrow(|engine| engine.as_ref().map(|engine| engine.meta.clone()))
}

/// Options for the layout from the config, as JSON.
pub fn options_json(config: &Tiling) -> serde_json::Value {
    let options = config.options.iter().map(|(name, value)| {
        let value = match value {
            TilingOption::Bool(value) => serde_json::Value::from(*value),
            TilingOption::Number(value) => serde_json::Value::from(*value),
            TilingOption::String(value) => serde_json::Value::from(value.clone()),
        };
        (name.clone(), value)
    });
    serde_json::Value::Object(options.collect())
}

/// A new unique instance id.
pub fn next_instance() -> u64 {
    let id = NEXT_INSTANCE.get();
    NEXT_INSTANCE.set(id + 1);
    id
}

/// Sends an event to the layout and returns its answer.
pub fn call(event: &str) -> anyhow::Result<String> {
    ENGINE.with_borrow(|engine| {
        let engine = engine
            .as_ref()
            .ok_or_else(|| anyhow!("no layout is loaded"))?;
        engine.call(event)
    })
}

/// Keeps state an instance saved, for when it's created again after the layout reloads.
pub fn save_state(instance: u64, state: serde_json::Value) {
    ENGINE.with_borrow_mut(|engine| {
        if let Some(engine) = engine {
            let name = engine.meta.name.clone();
            engine.saved.insert(instance, (name, state));
        }
    });
}

/// Takes the state an instance saved, if the current layout is the one that saved it.
pub fn take_state(instance: u64) -> Option<serde_json::Value> {
    ENGINE.with_borrow_mut(|engine| {
        let engine = engine.as_mut()?;
        let (name, state) = engine.saved.remove(&instance)?;
        (name == engine.meta.name).then_some(state)
    })
}
