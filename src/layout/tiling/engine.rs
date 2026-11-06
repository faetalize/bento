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
use niri_config::{
    resolve_gradient, Color, EngineOptions, OutputName, Tiling, TilingLayout, TilingOption,
};
use niri_ipc::{
    LayoutEngineAction, LayoutEngineActionArg, LayoutEngineActionArgType, LayoutEngineOption,
    LayoutEngineOptionType, LayoutEngineOptionValue, LayoutEngineSource, LayoutEngineWindowRule,
    SizeChange,
};
use rquickjs::loader::{BuiltinLoader, BuiltinResolver, ImportAttributes, Resolver};
use rquickjs::{CatchResultExt as _, Context, Ctx, Function, Module, Object, Persistent, Runtime};
use serde::{Deserialize, Deserializer};
use serde_json::json;

use crate::layout::groups;

/// The prelude's entry point, which imports the rest.
const PRELUDE: &str = include_str!("../../../resources/layouts/prelude/index.js");

/// The prelude's other modules, by the name they import each other with.
const PRELUDE_MODULES: &[(&str, &str)] = &[
    (
        "bento:prelude/events.js",
        include_str!("../../../resources/layouts/prelude/events.js"),
    ),
    (
        "bento:prelude/groups.js",
        include_str!("../../../resources/layouts/prelude/groups.js"),
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
            // bento may have taken the focus off the window already.
            if ((ws.focused === win || ws.focused === null) && rest.length > 0) {
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
    #[serde(rename = "windowRules")]
    pub window_rules: Vec<OptionMeta>,
    /// Whether bento's floating layer exists under this layout.
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
        TilingOption::Bool(_) | TilingOption::Gradient(_) => None,
    }
}

/// An option the layout declares, or a property its window rules can set.
#[derive(Debug, Clone, Deserialize)]
pub struct OptionMeta {
    pub name: String,
    pub r#type: OptionType,
    /// Whether the value is a list, written as several arguments.
    #[serde(default)]
    pub list: bool,
    /// As written in the layout's file, like `"50%"`, or a list of them. `None` for no value,
    /// which a size takes to leave it to the window.
    pub default: Option<LayoutEngineOptionValue>,
    #[serde(default)]
    pub choices: Vec<String>,
    pub description: Option<String>,
}

/// The type of an option's values, spelled like in the layout's `meta`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OptionType {
    Boolean,
    Number,
    String,
    Choice,
    Size,
    Color,
    Gradient,
}

impl OptionType {
    fn ipc(self) -> LayoutEngineOptionType {
        match self {
            Self::Boolean => LayoutEngineOptionType::Boolean,
            Self::Number => LayoutEngineOptionType::Number,
            Self::String => LayoutEngineOptionType::String,
            Self::Choice => LayoutEngineOptionType::Choice,
            Self::Size => LayoutEngineOptionType::Size,
            Self::Color => LayoutEngineOptionType::Color,
            Self::Gradient => LayoutEngineOptionType::Gradient,
        }
    }
}

impl OptionMeta {
    /// The default, as the values it's written with in the layout's file.
    fn default_values(&self) -> Vec<TilingOption> {
        match &self.default {
            None => Vec::new(),
            Some(LayoutEngineOptionValue::List(values)) => {
                values.iter().filter_map(scalar_option).collect()
            }
            Some(value) => scalar_option(value).into_iter().collect(),
        }
    }

    /// Converts values written in the layout's file to what the layout receives: booleans,
    /// numbers, strings, choices and colours as they are, sizes as `{ proportion }` or
    /// `{ fixed }`, and gradients as `{ from, to, angle, relativeTo, in }` with what was written.
    /// A list option gets a list, and a size or a gradient with no value is `null`: the window
    /// decides, or there is none.
    pub fn convert(&self, values: &[TilingOption]) -> Result<serde_json::Value, String> {
        if self.list {
            let values: Result<Vec<_>, _> = values.iter().map(|v| self.convert_one(v)).collect();
            return values.map(serde_json::Value::Array);
        }
        match values {
            [] if matches!(self.r#type, OptionType::Size | OptionType::Gradient) => {
                Ok(serde_json::Value::Null)
            }
            [] => Err(String::from("needs a value")),
            [value] => self.convert_one(value),
            values => Err(format!("takes one value, not {}", values.len())),
        }
    }

    fn convert_one(&self, value: &TilingOption) -> Result<serde_json::Value, String> {
        match (self.r#type, value) {
            (OptionType::Boolean, TilingOption::Bool(value)) => Ok(json!(value)),
            (OptionType::Number, TilingOption::Number(value)) => Ok(json!(value)),
            (OptionType::String, TilingOption::String(value)) => Ok(json!(value)),
            (OptionType::Choice, TilingOption::String(value)) if self.choices.contains(value) => {
                Ok(json!(value))
            }
            (OptionType::Size, value) => match size_arg(value) {
                Some(SizeChange::SetProportion(percent)) => {
                    Ok(json!({ "proportion": percent / 100. }))
                }
                Some(SizeChange::SetFixed(pixels)) => Ok(json!({ "fixed": pixels })),
                _ => Err(String::from(r#"must be a size, like "50%" or "800""#)),
            },
            (OptionType::Color, TilingOption::String(value)) => Color::from_hex(value)
                .map(|_| json!(value))
                .map_err(|_| String::from(r##"must be a colour, like "#rrggbb" or "#rrggbbaa""##)),
            (OptionType::Gradient, TilingOption::Gradient(gradient)) => resolve_gradient(gradient)
                .map(|_| json!(gradient))
                .map_err(|err| format!("has an invalid gradient, {err}")),
            (OptionType::Boolean, _) => Err(String::from("must be true or false")),
            (OptionType::Number, _) => Err(String::from("must be a number")),
            (OptionType::String, _) => Err(String::from("must be a string")),
            (OptionType::Choice, _) => Err(format!("must be one of {:?}", self.choices)),
            (OptionType::Color, _) => Err(String::from(
                r##"must be a colour, like "#rrggbb" or "#rrggbbaa""##,
            )),
            (OptionType::Gradient, _) => Err(String::from(
                r##"must be a gradient, like `from="#000" to="#fff"`"##,
            )),
        }
    }

    /// The values written for it in `values`, as bento shows them to clients.
    fn ipc_value(&self, values: &[TilingOption]) -> Option<LayoutEngineOptionValue> {
        let mut values = values.iter().filter_map(ipc_scalar);
        if self.list {
            Some(LayoutEngineOptionValue::List(values.collect()))
        } else {
            values.next_back()
        }
    }
}

fn scalar_option(value: &LayoutEngineOptionValue) -> Option<TilingOption> {
    match value {
        LayoutEngineOptionValue::Boolean(value) => Some(TilingOption::Bool(*value)),
        LayoutEngineOptionValue::Number(value) => Some(TilingOption::Number(*value)),
        LayoutEngineOptionValue::String(value) => Some(TilingOption::String(value.clone())),
        LayoutEngineOptionValue::Gradient(value) => Some(TilingOption::Gradient(value.clone())),
        LayoutEngineOptionValue::List(_) => None,
    }
}

fn ipc_scalar(value: &TilingOption) -> Option<LayoutEngineOptionValue> {
    Some(match value {
        TilingOption::Bool(value) => LayoutEngineOptionValue::Boolean(*value),
        TilingOption::Number(value) => LayoutEngineOptionValue::Number(*value),
        TilingOption::String(value) => LayoutEngineOptionValue::String(value.clone()),
        TilingOption::Gradient(value) => LayoutEngineOptionValue::Gradient(value.clone()),
    })
}

/// Checks that the defaults a layout declares are values of the declared types, so that a layout
/// that loads can always be given them.
fn check_defaults(meta: &LayoutMeta) -> anyhow::Result<()> {
    let options = meta.options.iter().map(|option| (option, "option", true));
    let rules = meta
        .window_rules
        .iter()
        .map(|rule| (rule, "window rule", false));
    for (option, kind, required) in options.chain(rules) {
        // A window rule property that has no default is unset for windows no rule sets it for.
        if option.default.is_none() && !required {
            continue;
        }
        if let Err(problem) = option.convert(&option.default_values()) {
            return Err(anyhow!(
                "the layout engine's default for its {kind} {:?} is wrong: it {problem}",
                option.name
            ));
        }
    }
    Ok(())
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

            // Groups made by the layout get their ids from bento, since bento makes groups too.
            let set_group_ids: Function = prelude.get("setGroupIdSource")?;
            set_group_ids
                .call::<_, ()>((Function::new(ctx.clone(), groups::next_id)?,))
                .catch(&ctx)
                .map_err(|err| anyhow!("{err}"))?;

            let load: Function = prelude.get("load")?;
            let meta: String = load
                .call((layout,))
                .catch(&ctx)
                .map_err(|err| anyhow!("{err}"))?;
            let meta: LayoutMeta = serde_json::from_str(&meta)
                .context("error reading the layout engine's metadata")?;
            check_defaults(&meta)?;
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
                        ..Tiling::default()
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

/// Runs the fallback layout instead of the current one, which failed while running. The failed
/// layout loads again once its file or the config changes.
pub fn fail(error: String) {
    warn!("{error}, using the fallback layout engine");
    LOAD_ERROR.set(Some(error));
    ENGINE.with_borrow_mut(|engine| {
        let Some(old) = engine.take() else {
            return;
        };
        let fallback = Tiling {
            layout: TilingLayout::BuiltIn(String::from("fallback")),
            ..Tiling::default()
        };
        let new = Engine::load(&fallback, String::from(FALLBACK), "fallback")
            .expect("the fallback layout engine must load");
        *engine = Some(Engine {
            config: old.config.clone(),
            mtime: old.mtime,
            ..new
        });
    });
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
            // Like the engine sees it: the last value wins, and one that doesn't fit the type
            // gives the default.
            let default = option.ipc_value(&option.default_values());
            let value = configured(&[&config.options], option)
                .map(|values| option.ipc_value(values))
                .unwrap_or_else(|| default.clone());
            LayoutEngineOption {
                name: option.name.clone(),
                r#type: option.r#type.ipc(),
                list: option.list,
                choices: option.choices.clone(),
                default,
                description: option.description.clone(),
                value,
            }
        });
        let window_rules = meta.window_rules.iter().map(|rule| LayoutEngineWindowRule {
            name: rule.name.clone(),
            r#type: rule.r#type.ipc(),
            list: rule.list,
            choices: rule.choices.clone(),
            default: rule.ipc_value(&rule.default_values()),
            description: rule.description.clone(),
        });

        Some(niri_ipc::LayoutEngine {
            name: meta.name.clone(),
            description: meta.description.clone(),
            source,
            floating: meta.floating,
            options: options.collect(),
            window_rules: window_rules.collect(),
            actions: meta.actions.clone(),
            error: LOAD_ERROR.with_borrow(Clone::clone),
        })
    })
}

/// The values the config writes for `option` in the last of `layers` that sets it, if they fit its
/// type. In a list, the last one written wins. A layer that sets the option to something that
/// doesn't fit leaves it to the layers before.
fn configured<'a>(layers: &[&'a EngineOptions], option: &OptionMeta) -> Option<&'a [TilingOption]> {
    layers.iter().rev().find_map(|options| {
        let (_, values) = options
            .iter()
            .rev()
            .find(|(name, _)| *name == option.name)?;
        option.convert(values).ok().map(|_| values.as_slice())
    })
}

/// Options for the layout as JSON: every option it declares, converted by its type, with the
/// default for the ones the config doesn't set or sets to something that doesn't fit.
pub fn options_json(config: &Tiling) -> serde_json::Value {
    resolved_options_json(&[&config.options])
}

/// The options of one workspace as JSON: the top-level options with those that the engine's file
/// sets for the workspace's output and for the workspace by name on top, see
/// [`Tiling::option_layers`].
pub fn workspace_options_json(
    config: &Tiling,
    output: Option<&OutputName>,
    workspace: Option<&str>,
) -> serde_json::Value {
    resolved_options_json(&config.option_layers(output, workspace))
}

fn resolved_options_json(layers: &[&EngineOptions]) -> serde_json::Value {
    ENGINE.with_borrow(|engine| {
        let Some(engine) = engine else {
            return json!({});
        };
        let options = engine.meta.options.iter().map(|option| {
            let values = configured(layers, option)
                .map(<[TilingOption]>::to_vec)
                .unwrap_or_else(|| option.default_values());
            let value = option.convert(&values).unwrap_or(serde_json::Value::Null);
            (option.name.clone(), value)
        });
        serde_json::Value::Object(options.collect())
    })
}

/// The properties that the layout's window rules set for one window, as JSON, converted by the
/// types the layout declares for them. Properties that no rule sets, and ones the layout doesn't
/// declare, are left out.
pub fn rules_json(rules: &niri_config::EngineRules) -> serde_json::Value {
    ENGINE.with_borrow(|engine| {
        let Some(engine) = engine else {
            return json!({});
        };
        let properties = engine.meta.window_rules.iter().filter_map(|rule| {
            let values = rules.get(&rule.name)?;
            let value = rule.convert(values).ok()?;
            Some((rule.name.clone(), value))
        });
        serde_json::Value::Object(properties.collect())
    })
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
/// that the options and window rule properties exist and have values of their types, and that
/// binds to the engine's actions name actions that the engine has, with arguments that fit.
pub fn check(config: &niri_config::Config) -> Result<(), Vec<String>> {
    ensure_loaded(&config.tiling);
    if let Some(err) = LOAD_ERROR.with_borrow(Clone::clone) {
        return Err(vec![format!("error loading the layout engine: {err}")]);
    }
    let Some(meta) = meta() else {
        return Err(vec![String::from("no layout engine is loaded")]);
    };

    let mut problems = Vec::new();
    let tiling = &config.tiling;
    check_options(&meta, &tiling.options, "", &mut problems);
    for scoped in &tiling.output_options {
        let scope = format!(" of output {:?}", scoped.name);
        check_options(&meta, &scoped.options, &scope, &mut problems);
    }
    for scoped in &tiling.workspace_options {
        let scope = format!(" of workspace {:?}", scoped.name);
        check_options(&meta, &scoped.options, &scope, &mut problems);
    }

    for rule in &config.tiling.window_rules {
        for (name, values) in &rule.properties.0 {
            match meta.window_rules.iter().find(|rule| rule.name == *name) {
                None => problems.push(format!(
                    "the {} layout engine has no window rule called {name:?}",
                    meta.name
                )),
                Some(property) => {
                    if let Err(problem) = property.convert(values) {
                        problems.push(format!("layout engine window rule {name:?} {problem}"));
                    }
                }
            }
        }
    }

    for bind in &config.binds.0 {
        for action in [bind.press_action(), bind.release_action()] {
            if let Some(niri_config::Action::LayoutAction(name, args, _)) = action {
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

/// Checks that `options` are options of the layout, with values of their types. `scope` says
/// whose they are, for the messages.
fn check_options(
    meta: &LayoutMeta,
    options: &EngineOptions,
    scope: &str,
    problems: &mut Vec<String>,
) {
    for (name, values) in options {
        match meta.options.iter().find(|option| option.name == *name) {
            None => problems.push(format!(
                "the {} layout engine has no option called {name:?}{scope}",
                meta.name
            )),
            Some(option) => {
                if let Err(problem) = option.convert(values) {
                    problems.push(format!("layout engine option {name:?}{scope} {problem}"));
                }
            }
        }
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
            (
                LayoutEngineActionArgType::Workspace,
                TilingOption::Number(_) | TilingOption::String(_),
            )
            | (LayoutEngineActionArgType::Monitor, TilingOption::String(_)) => continue,
            (LayoutEngineActionArgType::Boolean, _) => String::from("true or false"),
            (LayoutEngineActionArgType::Number, _) => String::from("a number"),
            (LayoutEngineActionArgType::String, _) => String::from("a string"),
            (LayoutEngineActionArgType::Choice, _) => format!("one of {:?}", arg.choices),
            (LayoutEngineActionArgType::Size, _) => String::from(r#"a size, like "50%" or "+10%""#),
            (LayoutEngineActionArgType::Workspace, _) => {
                String::from("a workspace, by its index or its name")
            }
            (LayoutEngineActionArgType::Monitor, _) => {
                String::from(r#"a monitor's name, like "DP-2""#)
            }
            // Only bento's own actions have these, and the config parser checks them.
            (LayoutEngineActionArgType::Command, _) => continue,
        };
        problems.push(format!(
            "the {:?} argument of action {name:?} must be {expected}",
            arg.name
        ));
    }
}

#[cfg(test)]
mod tests {
    use niri_ipc::LayoutEngineGradient;
    use serde_json::json;

    use super::*;

    fn option(r#type: &str, list: bool) -> OptionMeta {
        serde_json::from_value(json!({
            "name": "x",
            "type": r#type,
            "list": list,
            "default": null,
            "description": null,
        }))
        .unwrap()
    }

    fn string(value: &str) -> TilingOption {
        TilingOption::String(value.to_owned())
    }

    fn gradient(from: &str, to: &str) -> LayoutEngineGradient {
        LayoutEngineGradient {
            from: from.to_owned(),
            to: to.to_owned(),
            angle: None,
            relative_to: None,
            color_space: None,
        }
    }

    #[test]
    fn colors_are_hex_strings() {
        let color = option("color", false);
        assert_eq!(color.convert(&[string("#80c8ff")]), Ok(json!("#80c8ff")));
        assert_eq!(color.convert(&[string("#fff")]), Ok(json!("#fff")));
        assert_eq!(
            color.convert(&[string("#00000080")]),
            Ok(json!("#00000080"))
        );

        for bad in [string("red"), string("#ff"), TilingOption::Number(1.)] {
            assert!(
                color.convert(std::slice::from_ref(&bad)).is_err(),
                "{bad:?}"
            );
        }
        // A colour is always set.
        assert!(color.convert(&[]).is_err());
    }

    #[test]
    fn gradients_reach_the_engine_as_written() {
        let option = option("gradient", false);
        let written = LayoutEngineGradient {
            angle: Some(45.),
            relative_to: Some(String::from("view")),
            color_space: Some(String::from("oklch shorter hue")),
            ..gradient("#80c8ff", "#c7ff7f")
        };
        assert_eq!(
            option.convert(&[TilingOption::Gradient(written)]),
            Ok(json!({
                "from": "#80c8ff",
                "to": "#c7ff7f",
                "angle": 45.0,
                "relativeTo": "view",
                "in": "oklch shorter hue",
            }))
        );

        // What isn't written stays out, for the engine to default.
        let plain = TilingOption::Gradient(gradient("#000", "#fff"));
        assert_eq!(
            option.convert(&[plain]),
            Ok(json!({ "from": "#000", "to": "#fff" }))
        );

        // Left out, there is none.
        assert_eq!(option.convert(&[]), Ok(serde_json::Value::Null));
    }

    #[test]
    fn bad_gradients_are_refused() {
        let option = option("gradient", false);
        let bad = LayoutEngineGradient {
            relative_to: Some(String::from("window")),
            ..gradient("#000", "#fff")
        };
        for value in [
            TilingOption::Gradient(gradient("red", "#fff")),
            TilingOption::Gradient(bad),
            string("#000"),
            TilingOption::Bool(true),
        ] {
            assert!(
                option.convert(std::slice::from_ref(&value)).is_err(),
                "{value:?}"
            );
        }
    }

    #[test]
    fn sizes_and_gradients_without_a_value_are_null_and_others_need_one() {
        for r#type in ["size", "gradient"] {
            assert_eq!(
                option(r#type, false).convert(&[]),
                Ok(serde_json::Value::Null)
            );
        }
        for r#type in ["boolean", "number", "string", "color"] {
            assert!(option(r#type, false).convert(&[]).is_err(), "{type}");
        }
    }

    #[test]
    fn lists_of_colours_convert_each_one() {
        let colors = option("color", true);
        assert_eq!(
            colors.convert(&[string("#000"), string("#ffffff")]),
            Ok(json!(["#000", "#ffffff"]))
        );
        assert!(colors.convert(&[string("#000"), string("white")]).is_err());
    }

    #[test]
    fn later_layers_win_per_option() {
        let option = option("number", false);
        let number = |value: f64| vec![TilingOption::Number(value)];
        let top: EngineOptions = vec![(String::from("x"), number(1.))];
        let output: EngineOptions = vec![(String::from("x"), number(2.))];
        let unrelated: EngineOptions = vec![(String::from("y"), number(3.))];
        let wrong: EngineOptions = vec![(String::from("x"), vec![string("two")])];

        let layers = [&top, &output, &unrelated];
        assert_eq!(configured(&layers, &option), Some(&number(2.)[..]));
        assert_eq!(configured(&layers[..1], &option), Some(&number(1.)[..]));
        // A layer with a value of the wrong type leaves the option to the ones before it.
        let layers = [&top, &output, &wrong];
        assert_eq!(configured(&layers, &option), Some(&number(2.)[..]));
        assert_eq!(configured(&[&unrelated], &option), None);
        // In a layer, the last one written wins.
        let twice: EngineOptions = vec![
            (String::from("x"), number(4.)),
            (String::from("x"), number(5.)),
        ];
        assert_eq!(configured(&[&twice], &option), Some(&number(5.)[..]));
    }
}
