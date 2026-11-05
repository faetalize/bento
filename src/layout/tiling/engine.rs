//! The JS engine that runs the tiling layout.
//!
//! There is one engine per thread, holding the current layout module and its one instance, which
//! sees the whole session. bento and the layout talk in JSON through the prelude, see
//! `resources/layouts/prelude/` for it and `resources/layouts/bento-layout.d.ts` for the API.
//!
//! The engine has no access to files, the network or other modules, and every call has a time
//! limit, so a broken layout can't hang the compositor.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, Context as _};
use niri_config::{Tiling, TilingLayout, TilingOption};
use niri_ipc::{
    LayoutEngineAction, LayoutEngineActionArg, LayoutEngineActionArgType, LayoutEngineOption,
    LayoutEngineOptionType, LayoutEngineOptionValue, LayoutEngineSource, SizeChange,
};
use rquickjs::loader::{BuiltinLoader, BuiltinResolver, ImportAttributes, Resolver};
use rquickjs::{CatchResultExt as _, Context, Ctx, Function, Module, Object, Persistent, Runtime};
use serde::{Deserialize, Deserializer};

/// The prelude's entry point, which imports the rest.
const PRELUDE: &str = include_str!("../../../resources/layouts/prelude/index.js");

/// The prelude's other modules, by the name they import each other with.
const PRELUDE_MODULES: &[(&str, &str)] = &[
    (
        "bento:prelude/events.js",
        include_str!("../../../resources/layouts/prelude/events.js"),
    ),
    (
        "bento:prelude/nodes.js",
        include_str!("../../../resources/layouts/prelude/nodes.js"),
    ),
    (
        "bento:prelude/objects.js",
        include_str!("../../../resources/layouts/prelude/objects.js"),
    ),
    (
        "bento:prelude/v1.js",
        include_str!("../../../resources/layouts/prelude/v1.js"),
    ),
];

/// Layouts that ship with bento.
const BUILT_IN: &[(&str, &str)] = &[
    ("bento", include_str!("../../../resources/layouts/bento.js")),
    ("niri", include_str!("../../../resources/layouts/niri.js")),
    ("fallback", FALLBACK),
];

/// The layout used when the configured one fails to load: windows side by side. It has no actions:
/// the binds in the engine's file are for the engine that failed.
const FALLBACK: &str = r#"
export const meta = { api: 2, name: "fallback" };
export default function create(bento) {
    const place = (windows) => {
        windows.forEach((win, i) => {
            const ws = win.workspace;
            const gaps = ws.config.gaps;
            const area = ws.tilingArea;
            const w = Math.max(1, (area.w - gaps) / 2 - gaps);
            const x = area.x + gaps + i * (w + gaps);
            win.place({ x, y: area.y + gaps, w, h: area.h - gaps * 2 });
        });
    };
    const show = (win) => {
        const ws = win.workspace;
        const area = ws.tilingArea;
        const rect = win.rect;
        if (rect.x < ws.camera.x + area.x || ws.camera.x + area.x + area.w < rect.x + rect.w) {
            ws.camera.move({ x: rect.x - area.x - ws.config.gaps });
        }
    };
    bento.addEventListener("windowadd", (e) => {
        const win = e.window;
        const ws = win.workspace;
        place(ws.windows);
        if (e.activate || ws.focused === null) {
            win.focus();
            show(win);
        }
        win.addEventListener("remove", () => {
            const rest = ws.windows.filter((other) => other !== win);
            const i = ws.windows.indexOf(win);
            place(rest);
            if (ws.focused === win && rest.length > 0) {
                const next = rest[Math.min(i, rest.length - 1)];
                next.focus();
                show(next);
            }
        });
    });
}
"#;

/// Time a layout call may take before it's stopped.
const CALL_TIME_LIMIT: Duration = Duration::from_millis(250);

/// Memory the layout may use.
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    /// Why the configured layout last failed to load.
    static LOAD_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

/// What the layout says about itself when it loads.
#[derive(Debug, Clone, Deserialize)]
pub struct LayoutMeta {
    pub name: String,
    pub description: Option<String>,
    #[serde(deserialize_with = "deserialize_actions")]
    pub actions: Vec<LayoutEngineAction>,
    pub options: Vec<OptionMeta>,
    /// Whether bento's floating layer exists under the layout.
    pub floating: bool,
}

impl LayoutMeta {
    pub fn action(&self, name: &str) -> Option<&LayoutEngineAction> {
        self.actions.iter().find(|action| action.name == name)
    }
}

/// An action as the prelude reports it, with argument types spelled like in the engine's `meta`.
#[derive(Deserialize)]
struct ActionMeta {
    name: String,
    description: Option<String>,
    args: Vec<ArgMeta>,
}

#[derive(Deserialize)]
struct ArgMeta {
    name: String,
    r#type: Option<ArgType>,
    default: Option<LayoutEngineOptionValue>,
    choices: Vec<String>,
    description: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum ArgType {
    Boolean,
    Number,
    String,
    Choice,
    Size,
    Workspace,
    Monitor,
}

fn deserialize_actions<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<LayoutEngineAction>, D::Error> {
    let actions = Vec::<ActionMeta>::deserialize(deserializer)?;
    let arg = |arg: ArgMeta| LayoutEngineActionArg {
        name: arg.name,
        r#type: arg.r#type.map(|r#type| match r#type {
            ArgType::Boolean => LayoutEngineActionArgType::Boolean,
            ArgType::Number => LayoutEngineActionArgType::Number,
            ArgType::String => LayoutEngineActionArgType::String,
            ArgType::Choice => LayoutEngineActionArgType::Choice,
            ArgType::Size => LayoutEngineActionArgType::Size,
            ArgType::Workspace => LayoutEngineActionArgType::Workspace,
            ArgType::Monitor => LayoutEngineActionArgType::Monitor,
        }),
        default: arg.default,
        optional: false,
        property: false,
        choices: arg.choices,
        description: arg.description,
    };
    let actions = actions.into_iter().map(|action| LayoutEngineAction {
        name: action.name,
        description: action.description,
        args: action.args.into_iter().map(arg).collect(),
    });
    Ok(actions.collect())
}

/// Whether `name` is one of bento's own actions, as binds and `bento msg action` write it.
fn is_native_action(name: &str) -> bool {
    use clap::Subcommand as _;

    let actions = niri_ipc::Action::augment_subcommands(clap::Command::new("action"));
    actions.find_subcommand(name).is_some()
}

/// Reads a `Size` argument, written like `"50%"`, `"+10%"` or `"800"`.
pub fn size_arg(value: &TilingOption) -> Option<SizeChange> {
    match value {
        TilingOption::String(value) => value.parse().ok(),
        // Numbers come from configs and the command line, like `800` or `-100`.
        TilingOption::Number(value) => value.to_string().parse().ok(),
        TilingOption::Bool(_) => None,
    }
}

/// An option the layout declares.
#[derive(Debug, Clone, Deserialize)]
pub struct OptionMeta {
    pub name: String,
    /// Also gives the option's type: the prelude checks that it matches.
    pub default: LayoutEngineOptionValue,
    pub description: Option<String>,
}

struct Engine {
    /// Config the engine was loaded for.
    config: Tiling,
    /// Layout that is running, which isn't the one in `config` when that one failed to load.
    layout: TilingLayout,
    /// Modification time of the layout file, to reload it when it changes.
    mtime: Option<SystemTime>,
    /// Source of the layout, to tell whether a reload changed anything.
    source: String,
    meta: LayoutMeta,
    /// Increases with every load; the instance of an older generation is gone.
    generation: u64,
    deadline: Rc<Cell<Option<Instant>>>,
    // Field order matters: the functions have to go before the context and runtime.
    handle: Persistent<Function<'static>>,
    save_all: Persistent<Function<'static>>,
    exposed_state: Persistent<Function<'static>>,
    context: Context,
    _runtime: Runtime,
    /// State that the previous layout saved before it was unloaded, with its name.
    saved: Option<(String, serde_json::Value)>,
}

/// Lets the prelude's modules import each other, and nothing else import them.
struct PreludeResolver(BuiltinResolver);

impl Resolver for PreludeResolver {
    fn resolve<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        if !base.starts_with("bento:prelude/") {
            return Err(rquickjs::Error::new_resolving(base, name));
        }
        self.0.resolve(ctx, base, name, attributes)
    }
}

impl Engine {
    fn load(config: &Tiling, source: String, name: &str) -> anyhow::Result<Self> {
        let runtime = Runtime::new().context("error creating the JS runtime")?;
        runtime.set_memory_limit(MEMORY_LIMIT);
        runtime.set_max_stack_size(1024 * 1024);

        let mut resolver = BuiltinResolver::default();
        let mut loader = BuiltinLoader::default();
        for (module, source) in PRELUDE_MODULES {
            resolver.add_module(*module);
            loader.add_module(*module, *source);
        }
        runtime.set_loader(PreludeResolver(resolver), loader);

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

            let prelude =
                eval("bento:prelude/index.js", PRELUDE).context("error loading the prelude")?;
            let layout = eval(name, &source)?;

            let load: Function = prelude.get("load")?;
            let meta: String = load
                .call((layout,))
                .catch(&ctx)
                .map_err(|err| anyhow!("{err}"))?;
            let meta: LayoutMeta = serde_json::from_str(&meta)
                .context("error reading the layout engine's metadata")?;
            // Binds name engine actions and bento's own the same way, so they can't share a name.
            if let Some(action) = meta.actions.iter().find(|a| is_native_action(&a.name)) {
                return Err(anyhow!(
                    "the layout engine's action {:?} has the name of one of bento's actions",
                    action.name
                ));
            }

            let handle: Function = prelude.get("handle")?;
            let save_all: Function = prelude.get("saveAll")?;
            let exposed_state: Function = prelude.get("exposedState")?;
            Ok((
                meta,
                Persistent::save(&ctx, handle),
                Persistent::save(&ctx, save_all),
                Persistent::save(&ctx, exposed_state),
            ))
        });
        deadline.set(None);
        let (meta, handle, save_all, exposed_state) = res?;

        let generation = GENERATION.get() + 1;
        GENERATION.set(generation);

        Ok(Self {
            config: config.clone(),
            layout: config.layout.clone(),
            mtime: file_mtime(config),
            source,
            meta,
            generation,
            deadline,
            handle,
            save_all,
            exposed_state,
            context,
            _runtime: runtime,
            saved: None,
        })
    }

    /// Calls one of the prelude's functions that take nothing and answer with JSON.
    fn call_json(
        &self,
        function: &Persistent<Function<'static>>,
    ) -> anyhow::Result<serde_json::Value> {
        self.deadline.set(Some(Instant::now() + CALL_TIME_LIMIT));
        let res = self.context.with(|ctx| {
            let function = function.clone().restore(&ctx)?;
            function
                .call::<_, String>(())
                .catch(&ctx)
                .map_err(|err| anyhow!("{err}"))
        });
        self.deadline.set(None);
        serde_json::from_str(&res?).context("error reading the layout's answer")
    }

    /// Saves the layout's state, for the next instance of the same layout.
    fn save_state(&self) -> Option<(String, serde_json::Value)> {
        match self.call_json(&self.save_all) {
            Ok(state) => Some((self.meta.name.clone(), state)),
            Err(err) => {
                warn!("error saving layout engine state: {err:?}");
                None
            }
        }
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
                .ok_or_else(|| anyhow!("there's no built-in layout engine called {name:?}"))?;
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
/// the engine generation. The instance of the layout it replaces is gone: whoever keeps the
/// session's host creates it again when the generation changes.
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
                LOAD_ERROR.set(None);
                if let Some(old) = engine.take() {
                    new.saved = old.save_state().or(old.saved);
                }
                info!("loaded layout engine {}", new.meta.name);
                *engine = Some(new);
            }
            Ok(None) => {
                LOAD_ERROR.set(None);
                // Same source: only the options changed, which the host passes on.
                let engine = engine.as_mut().unwrap();
                engine.config = config.clone();
                engine.layout = config.layout.clone();
                engine.mtime = file_mtime(config);
            }
            Err(err) => {
                warn!("error loading layout engine: {err:?}");
                LOAD_ERROR.set(Some(format!("{err:?}")));
                if engine.is_none() {
                    let fallback = Tiling {
                        layout: TilingLayout::BuiltIn(String::from("fallback")),
                        options: Vec::new(),
                    };
                    let new = Engine::load(&fallback, String::from(FALLBACK), "fallback")
                        .expect("the fallback layout engine must load");
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

/// Name of the current layout, or an empty string if none is loaded.
pub fn name() -> String {
    ENGINE.with_borrow(|engine| {
        engine
            .as_ref()
            .map_or_else(String::new, |engine| engine.meta.name.clone())
    })
}

/// Whether bento's floating layer exists under the current layout.
pub fn floating_enabled() -> bool {
    ENGINE.with_borrow(|engine| engine.as_ref().is_none_or(|engine| engine.meta.floating))
}

/// Metadata of the current layout.
pub fn meta() -> Option<LayoutMeta> {
    ENGINE.with_borrow(|engine| engine.as_ref().map(|engine| engine.meta.clone()))
}

/// Arguments that the running engine declares for action `name`.
pub fn action_args(name: &str) -> Option<Vec<LayoutEngineActionArg>> {
    ENGINE.with_borrow(|engine| Some(engine.as_ref()?.meta.action(name)?.args.clone()))
}

/// The current layout engine for IPC, with the option values from `config`.
pub fn ipc_info(config: &Tiling) -> Option<niri_ipc::LayoutEngine> {
    ENGINE.with_borrow(|engine| {
        let engine = engine.as_ref()?;
        let meta = &engine.meta;

        let source = match &engine.layout {
            TilingLayout::BuiltIn(_) => LayoutEngineSource::BuiltIn {},
            TilingLayout::File(path) => LayoutEngineSource::File {
                path: path.display().to_string(),
            },
        };

        let options = meta.options.iter().map(|option| {
            // Like the engine sees it: the last value wins, and one of the wrong type gives the
            // default.
            let value = config
                .options
                .iter()
                .rev()
                .find(|(name, _)| *name == option.name)
                .map(|(_, value)| option_value(value))
                .filter(|value| value.r#type() == option.default.r#type())
                .unwrap_or_else(|| option.default.clone());
            LayoutEngineOption {
                name: option.name.clone(),
                r#type: option.default.r#type(),
                default: option.default.clone(),
                description: option.description.clone(),
                value,
            }
        });

        Some(niri_ipc::LayoutEngine {
            name: meta.name.clone(),
            description: meta.description.clone(),
            source,
            options: options.collect(),
            actions: meta.actions.clone(),
        })
    })
}

fn option_value(value: &TilingOption) -> LayoutEngineOptionValue {
    match value {
        TilingOption::Bool(value) => LayoutEngineOptionValue::Boolean(*value),
        TilingOption::Number(value) => LayoutEngineOptionValue::Number(*value),
        TilingOption::String(value) => LayoutEngineOptionValue::String(value.clone()),
    }
}

fn type_name(r#type: LayoutEngineOptionType) -> &'static str {
    match r#type {
        LayoutEngineOptionType::Boolean => "boolean",
        LayoutEngineOptionType::Number => "number",
        LayoutEngineOptionType::String => "string",
    }
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

/// Sends an event to the layout and returns its answer.
pub fn call(event: &str) -> anyhow::Result<String> {
    ENGINE.with_borrow(|engine| {
        let engine = engine
            .as_ref()
            .ok_or_else(|| anyhow!("no layout engine is loaded"))?;
        engine.call(event)
    })
}

/// Takes the state that the previous layout saved, if the current layout is the one that saved it.
pub fn take_state() -> Option<serde_json::Value> {
    ENGINE.with_borrow_mut(|engine| {
        let engine = engine.as_mut()?;
        let (name, state) = engine.saved.take()?;
        (name == engine.meta.name).then_some(state)
    })
}

/// The state that the layout exposes with `bento.exposeState()`, which is `null` if it exposes
/// none.
pub fn exposed_state() -> anyhow::Result<serde_json::Value> {
    ENGINE.with_borrow(|engine| {
        let engine = engine
            .as_ref()
            .ok_or_else(|| anyhow!("no layout engine is loaded"))?;
        engine.call_json(&engine.exposed_state)
    })
}

/// Checks the layout engine in `config` and what the config asks of it: that the engine loads,
/// that the options exist and have the right types, and that binds to the engine's actions name
/// actions that the engine has, with arguments that fit.
pub fn check(config: &niri_config::Config) -> Result<(), Vec<String>> {
    ensure_loaded(&config.tiling);
    if let Some(err) = LOAD_ERROR.with_borrow(Clone::clone) {
        return Err(vec![format!("error loading the layout engine: {err}")]);
    }
    let Some(meta) = meta() else {
        return Err(vec![String::from("no layout engine is loaded")]);
    };

    let mut problems = Vec::new();
    for (name, value) in &config.tiling.options {
        let given = option_value(value).r#type();
        match meta.options.iter().find(|option| option.name == *name) {
            None => problems.push(format!(
                "the {} layout engine has no option called {name:?}",
                meta.name
            )),
            Some(option) if option.default.r#type() != given => {
                let expected = type_name(option.default.r#type());
                let given = type_name(given);
                problems.push(format!(
                    "layout engine option {name:?} must be a {expected}, not a {given}"
                ))
            }
            Some(_) => (),
        }
    }

    for bind in &config.binds.0 {
        for action in [bind.press_action(), bind.release_action()] {
            if let Some(niri_config::Action::LayoutAction(name, args)) = action {
                match meta.action(name) {
                    None => problems.push(format!(
                        "the {} layout engine has no action called {name:?}",
                        meta.name
                    )),
                    Some(action) => check_args(action, args, &mut problems),
                }
            }
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

/// Checks a bind's arguments for `action` against the ones it declares.
fn check_args(action: &LayoutEngineAction, args: &[TilingOption], problems: &mut Vec<String>) {
    let name = &action.name;
    let declared = &action.args;
    if args.len() > declared.len() {
        let takes = match declared.len() {
            0 => String::from("no arguments"),
            1 => String::from("one argument"),
            n => format!("up to {n} arguments"),
        };
        problems.push(format!("action {name:?} takes {takes}, not {}", args.len()));
    }
    for (i, arg) in declared.iter().enumerate() {
        let Some(r#type) = arg.r#type else {
            continue;
        };
        let Some(value) = args.get(i) else {
            if arg.default.is_none() && !arg.optional {
                problems.push(format!("action {name:?} needs its {:?} argument", arg.name));
            }
            continue;
        };
        let expected = match (r#type, value) {
            (LayoutEngineActionArgType::Boolean, TilingOption::Bool(_))
            | (LayoutEngineActionArgType::Number, TilingOption::Number(_))
            | (LayoutEngineActionArgType::String, TilingOption::String(_)) => continue,
            (LayoutEngineActionArgType::Choice, TilingOption::String(value))
                if arg.choices.contains(value) =>
            {
                continue
            }
            (LayoutEngineActionArgType::Size, value) if size_arg(value).is_some() => continue,
            (LayoutEngineActionArgType::Boolean, _) => String::from("true or false"),
            (LayoutEngineActionArgType::Number, _) => String::from("a number"),
            (LayoutEngineActionArgType::String, _) => String::from("a string"),
            (LayoutEngineActionArgType::Choice, _) => format!("one of {:?}", arg.choices),
            (LayoutEngineActionArgType::Size, _) => String::from(r#"a size, like "50%" or "+10%""#),
            // Only bento's own actions have these, and the config parser checks them.
            (
                LayoutEngineActionArgType::Workspace
                | LayoutEngineActionArgType::Monitor
                | LayoutEngineActionArgType::Command,
                _,
            ) => continue,
        };
        problems.push(format!(
            "the {:?} argument of action {name:?} must be {expected}",
            arg.name
        ));
    }
}
