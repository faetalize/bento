//! niri config parsing.
//!
//! The config can be constructed from multiple files (includes). To support this, many types are
//! split into two. For example, `Layout` and `LayoutPart` where `Layout` is the final config and
//! `LayoutPart` is one part parsed from one config file.
//!
//! The convention for `Default` impls is to set the initial values before the parsing occurs.
//! Then, parsing will update the values with those parsed from the config.
//!
//! The `Default` values match those from `default-config.kdl` in almost all cases, with a notable
//! exception of `binds {}` and some window rules.

#[macro_use]
extern crate tracing;

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use knuffel::errors::DecodeError;
use knuffel::Decode as _;
use miette::{miette, Context as _, IntoDiagnostic as _};

#[macro_use]
pub mod macros;

pub mod actions;
pub mod animations;
pub mod appearance;
pub mod binds;
pub mod convert;
pub mod debug;
pub mod error;
pub mod gestures;
pub mod input;
pub mod layer_rule;
pub mod layout;
pub mod misc;
pub mod output;
pub mod recent_windows;
pub mod tiling;
pub mod utils;
pub mod window_rule;
pub mod workspace;

pub use crate::animations::{Animation, Animations};
pub use crate::appearance::*;
pub use crate::binds::*;
pub use crate::debug::Debug;
pub use crate::error::{ConfigIncludeError, ConfigParseResult};
pub use crate::gestures::Gestures;
pub use crate::input::{Input, ModKey, ScrollMethod, TrackLayout, WarpMouseToFocusMode, Xkb};
pub use crate::layer_rule::LayerRule;
pub use crate::layout::*;
pub use crate::misc::*;
pub use crate::output::{Output, OutputName, Outputs, Position, Vrr};
use crate::recent_windows::RecentWindowsPart;
pub use crate::recent_windows::{MruDirection, MruFilter, MruPreviews, MruScope, RecentWindows};
pub use crate::tiling::{
    default_engine_config, engine_config_path, engine_config_path_in, resolve_gradient,
    EngineOptions, EngineRules, EngineWindowRule, ScopedOptions, Tiling, TilingLayout,
    TilingOption,
};
use crate::tiling::{EngineFile, EnginePart};
pub use crate::utils::FloatOrInt;
use crate::utils::{expand_home_path, Flag, MergeWith as _};
pub use crate::window_rule::{
    FloatingPosition, OnXdgActivate, PopupsRule, RelativeTo, ResolvedPopupsRules, WindowRule,
};
pub use crate::workspace::{Workspace, WorkspaceLayoutPart};

const RECURSION_LIMIT: u8 = 10;

/// The config bento writes on first start.
pub const DEFAULT_CONFIG: &str = include_str!("../../resources/default-config.kdl");

/// Reads the engine's file at a path, like `fs::read_to_string`.
type ReadEngine<'a> = dyn FnOnce(&Path) -> std::io::Result<String> + 'a;

#[derive(Debug, Default, PartialEq)]
pub struct Config {
    pub input: Input,
    pub outputs: Outputs,
    pub spawn_at_startup: Vec<SpawnAtStartup>,
    pub spawn_sh_at_startup: Vec<SpawnShAtStartup>,
    pub shell: Option<Shell>,
    pub layout: Layout,
    pub tiling: Tiling,
    pub prefer_no_csd: bool,
    pub cursor: Cursor,
    pub screenshot_path: ScreenshotPath,
    pub clipboard: Clipboard,
    pub hotkey_overlay: HotkeyOverlay,
    pub config_notification: ConfigNotification,
    pub animations: Animations,
    pub blur: Blur,
    pub gestures: Gestures,
    pub overview: Overview,
    pub environment: Environment,
    pub xwayland_satellite: XwaylandSatellite,
    pub window_rules: Vec<WindowRule>,
    pub layer_rules: Vec<LayerRule>,
    pub binds: Binds,
    pub bind_files: BindFiles,
    pub switch_events: SwitchBinds,
    pub debug: Debug,
    pub workspaces: Vec<Workspace>,
    pub recent_windows: RecentWindows,
}

/// The file each bind comes from, by its key: the main config, an included file or the engine's
/// file.
#[derive(Default, Clone, PartialEq)]
pub struct BindFiles(pub Vec<(Key, PathBuf)>);

impl BindFiles {
    pub fn get(&self, key: &Key) -> Option<&Path> {
        let file = self.0.iter().find(|(k, _)| k == key);
        file.map(|(_, path)| path.as_path())
    }
}

// Only the count, so config snapshots don't list every bind twice.
impl std::fmt::Debug for BindFiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BindFiles({})", self.0.len())
    }
}

#[derive(Debug, Clone)]
pub enum ConfigPath {
    /// Explicitly set config path.
    ///
    /// Load the config only from this path, never create it.
    Explicit(PathBuf),

    /// Default config path.
    ///
    /// Prioritize the user path, fallback to the system path, fallback to creating the user path
    /// at compositor startup.
    Regular {
        /// User config path, usually `$XDG_CONFIG_HOME/bento/config.kdl`.
        user_path: PathBuf,
        /// System config path, usually `/etc/bento/config.kdl`.
        system_path: PathBuf,
    },
}

// Newtypes for putting information into the knuffel context.
struct BasePath(PathBuf);
/// The file being parsed, to remember where binds come from.
pub(crate) struct CurrentFile(pub(crate) PathBuf);
struct RootBase(PathBuf);
struct Recursion(u8);

// FIXME: This is currently used to track all the files that needed to be watched. The name
// `Includes` is not really correct and should be renamed in the future.
#[derive(Default)]
struct Includes(Vec<PathBuf>);

#[derive(Default)]
struct IncludeErrors(Vec<knuffel::Error>);
// Used for recursive include detection.
//
// We don't *need* it because we have a recursion limit, but it makes for nicer error messages.
struct IncludeStack(HashSet<PathBuf>);
struct SawMruBinds(Rc<Cell<bool>>);

// Rather than listing all fields and deriving knuffel::Decode, we implement
// knuffel::DecodeChildren by hand, since we need custom logic for every field anyway: we want to
// merge the values into the config from the context as we go to support the positionality of
// includes. The reason we need this type at all is because knuffel's only entry point that allows
// setting default values on a context is `parse_with_context()` that needs a type to parse.
pub struct ConfigPart;

impl<S> knuffel::DecodeChildren<S> for ConfigPart
where
    S: knuffel::traits::ErrorSpan,
{
    fn decode_children(
        nodes: &[knuffel::ast::SpannedNode<S>],
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, DecodeError<S>> {
        let _span = tracy_client::span!("decode config file");

        let config = ctx.get::<Rc<RefCell<Config>>>().unwrap().clone();
        let includes = ctx.get::<Rc<RefCell<Includes>>>().unwrap().clone();
        let include_errors = ctx.get::<Rc<RefCell<IncludeErrors>>>().unwrap().clone();
        let recursion = ctx.get::<Recursion>().unwrap().0;
        let saw_mru_binds = ctx.get::<SawMruBinds>().unwrap().0.clone();

        let mut seen = HashSet::new();

        for node in nodes {
            let name = &**node.node_name;

            // Within one config file, splitting sections into multiple parts is not allowed to
            // reduce confusion. The exceptions here aren't multipart; they all add new values.
            if !matches!(
                name,
                "output"
                    | "spawn-at-startup"
                    | "spawn-sh-at-startup"
                    | "window-rule"
                    | "layer-rule"
                    | "workspace"
                    | "include"
            ) && !seen.insert(name)
            {
                ctx.emit_error(DecodeError::unexpected(
                    &node.node_name,
                    "node",
                    format!("duplicate node `{name}`, single node expected"),
                ));
                continue;
            }

            macro_rules! m_merge {
                ($field:ident) => {{
                    let part = knuffel::Decode::decode_node(node, ctx)?;
                    config.borrow_mut().$field.merge_with(&part);
                }};
            }

            macro_rules! m_push {
                ($field:ident) => {{
                    let part = knuffel::Decode::decode_node(node, ctx)?;
                    config.borrow_mut().$field.push(part);
                }};
            }

            match name {
                "input" => m_merge!(input),
                "cursor" => m_merge!(cursor),
                "clipboard" => m_merge!(clipboard),
                "hotkey-overlay" => m_merge!(hotkey_overlay),
                "config-notification" => m_merge!(config_notification),
                "animations" => m_merge!(animations),
                "blur" => {
                    let part = BlurPart::decode_node(node, ctx)?;
                    let lights = part.glass.as_ref().map_or(0, |glass| glass.lights.len());
                    if lights > MAX_GLASS_LIGHTS {
                        ctx.emit_error(DecodeError::unexpected(
                            &node.node_name,
                            "node",
                            format!("glass takes at most {MAX_GLASS_LIGHTS} lights, not {lights}"),
                        ));
                    }
                    config.borrow_mut().blur.merge_with(&part);
                }
                "gestures" => m_merge!(gestures),
                "overview" => m_merge!(overview),
                "xwayland-satellite" => m_merge!(xwayland_satellite),
                "switch-events" => m_merge!(switch_events),
                "debug" => m_merge!(debug),

                // Multipart sections.
                "output" => {
                    let part = Output::decode_node(node, ctx)?;
                    config.borrow_mut().outputs.0.push(part);
                }
                "spawn-at-startup" => m_push!(spawn_at_startup),
                "spawn-sh-at-startup" => m_push!(spawn_sh_at_startup),
                "window-rule" => m_push!(window_rules),
                "layer-rule" => m_push!(layer_rules),
                "workspace" => m_push!(workspaces),

                // Single-part sections.
                "binds" => {
                    let part = Binds::decode_node(node, ctx)?;

                    // We replace conflicting binds, rather than error, to support the use-case
                    // where you import some preconfigured-dots.kdl, then override some binds with
                    // your own.
                    let mut config = config.borrow_mut();
                    let binds = &mut config.binds.0;
                    // Remove existing binds matching any new bind.
                    binds
                        .retain(|bind| !part.0.iter().any(|new| new.key.conflicts_with(&bind.key)));
                    let file = &ctx.get::<CurrentFile>().unwrap().0;
                    let files = &mut config.bind_files.0;
                    files.retain(|(key, _)| !part.0.iter().any(|new| new.key.conflicts_with(key)));
                    files.extend(part.0.iter().map(|bind| (bind.key, file.clone())));
                    // Add all new binds.
                    config.binds.0.extend(part.0);
                }
                "environment" => {
                    let part = Environment::decode_node(node, ctx)?;
                    config.borrow_mut().environment.0.extend(part.0);
                }

                "tiling" => {
                    let part = Tiling::decode_node(node, ctx)?;
                    config.borrow_mut().tiling.merge_with(&part);
                }

                "prefer-no-csd" => {
                    config.borrow_mut().prefer_no_csd = Flag::decode_node(node, ctx)?.0
                }

                "shell" => {
                    let part = knuffel::Decode::decode_node(node, ctx)?;
                    config.borrow_mut().shell = Some(part);
                }

                "screenshot-path" => {
                    let part = knuffel::Decode::decode_node(node, ctx)?;
                    config.borrow_mut().screenshot_path = part;
                }

                "layout" => {
                    let mut part = LayoutPart::decode_node(node, ctx)?;

                    // Preserve the behavior we'd always had for the border section:
                    // - `layout {}` gives border = off
                    // - `layout { border {} }` gives border = on
                    // - `layout { border { off } }` gives border = off
                    //
                    // This behavior is inconsistent with the rest of the config where adding an
                    // empty section generally doesn't change the outcome. Particularly, shadows
                    // are also disabled by default (like borders), and they always had an `on`
                    // instead of an `off` for this reason, so that writing `layout { shadow {} }`
                    // still results in shadow = off, as it should.
                    //
                    // Unfortunately, the default config has always had wording that heavily
                    // implies that `layout { border {} }` enables the borders. This wording is
                    // sure to be present in a lot of users' configs by now, which we can't change.
                    //
                    // Another way to make things consistent would be to default borders to on.
                    // However, that is annoying because it would mean changing many tests that
                    // rely on borders being off by default. This would also contradict the
                    // intended default borders value (off).
                    //
                    // So, let's just work around the problem here, preserving the original
                    // behavior.
                    if recursion == 0 {
                        if let Some(border) = part.border.as_mut() {
                            if !border.on && !border.off {
                                border.on = true;
                            }
                        }
                    }

                    config.borrow_mut().layout.merge_with(&part);
                }

                "recent-windows" => {
                    let part = RecentWindowsPart::decode_node(node, ctx)?;

                    let mut config = config.borrow_mut();

                    // When an MRU binds section is encountered for the first time, clear out the
                    // default MRU binds.
                    if !saw_mru_binds.get() && part.binds.is_some() {
                        saw_mru_binds.set(true);
                        config.recent_windows.binds.clear();
                    }

                    config.recent_windows.merge_with(&part);
                }

                "include" => {
                    // Parse the path argument
                    let mut iter_args = node.arguments.iter();
                    let path_val = iter_args.next().ok_or_else(|| {
                        DecodeError::missing(
                            node,
                            "additional argument for include path is required",
                        )
                    })?;
                    // Check for extra arguments
                    if let Some(val) = iter_args.next() {
                        ctx.emit_error(DecodeError::unexpected(
                            &val.literal,
                            "argument",
                            "unexpected argument",
                        ));
                    }

                    // Parse the optional property
                    let mut optional = false;
                    for (name, val) in &node.properties {
                        match &***name {
                            "optional" => {
                                optional = knuffel::traits::DecodeScalar::decode(val, ctx)?;
                            }
                            name_str => {
                                ctx.emit_error(DecodeError::unexpected(
                                    name,
                                    "property",
                                    format!("unexpected property `{}`", name_str.escape_default()),
                                ));
                            }
                        }
                    }

                    // Check for unexpected children
                    for child in node.children() {
                        ctx.emit_error(DecodeError::unexpected(
                            child,
                            "node",
                            format!("unexpected node `{}`", child.node_name.escape_default()),
                        ));
                    }

                    // We use DecodeError::Missing throughout this block because it results in the
                    // least confusing error messages while still allowing to provide a span.

                    let decoded_path = knuffel::traits::DecodeScalar::decode(path_val, ctx)?;

                    // Expand ~ into the home dir
                    let path = match expand_home_path(decoded_path, node, ctx) {
                        Ok(p) => p,
                        Err(e) => {
                            ctx.emit_error(e);
                            continue;
                        }
                    };

                    let recursion = ctx.get::<Recursion>().unwrap().0 + 1;
                    if recursion == RECURSION_LIMIT {
                        ctx.emit_error(DecodeError::missing(
                            node,
                            format!(
                                "reached the recursion limit; \
                                 includes cannot be {RECURSION_LIMIT} levels deep"
                            ),
                        ));
                        continue;
                    }

                    let Some(filename) = path.file_name().and_then(OsStr::to_str) else {
                        ctx.emit_error(DecodeError::missing(
                            node,
                            "include path doesn't have a valid file name",
                        ));
                        continue;
                    };
                    let base = path.parent().map(Path::to_path_buf).unwrap_or_default();

                    // Check for recursive include for a nicer error message.
                    let mut include_stack = ctx.get::<IncludeStack>().unwrap().0.clone();
                    if !include_stack.insert(path.to_path_buf()) {
                        ctx.emit_error(DecodeError::missing(
                            node,
                            "recursive include (file includes itself)",
                        ));
                        continue;
                    }

                    // Store even if the include fails to read or parse, so it gets watched.
                    includes.borrow_mut().0.push(path.to_path_buf());

                    match fs::read_to_string(&path) {
                        Ok(text) => {
                            // Try to get filename relative to the root base config folder for
                            // clearer error messages.
                            let root_base = &ctx.get::<RootBase>().unwrap().0;
                            // Failing to strip prefix usually means absolute path; show it in full.
                            let relative_path = path.strip_prefix(root_base).ok().unwrap_or(&path);
                            let filename = relative_path.to_str().unwrap_or(filename);

                            let part = knuffel::parse_with_context::<
                                ConfigPart,
                                knuffel::span::Span,
                                _,
                            >(filename, &text, |ctx| {
                                ctx.set(CurrentFile(path.to_path_buf()));
                                ctx.set(BasePath(base));
                                ctx.set(RootBase(root_base.clone()));
                                ctx.set(Recursion(recursion));
                                ctx.set(includes.clone());
                                ctx.set(include_errors.clone());
                                ctx.set(IncludeStack(include_stack));
                                ctx.set(SawMruBinds(saw_mru_binds.clone()));
                                ctx.set(config.clone());
                            });

                            match part {
                                Ok(_) => {}
                                Err(err) => {
                                    include_errors.borrow_mut().0.push(err);

                                    ctx.emit_error(DecodeError::missing(
                                        node,
                                        "failed to parse included config",
                                    ));
                                }
                            }
                        }
                        Err(err) => {
                            if optional && err.kind() == std::io::ErrorKind::NotFound {
                                // Warn about missing optional includes
                                warn!("optional include not found: {path:?}");
                            } else {
                                // Report all other errors normally
                                ctx.emit_error(DecodeError::missing(
                                    node,
                                    format!("failed to read included config from {path:?}: {err}"),
                                ));
                            }
                        }
                    }
                }

                name => {
                    ctx.emit_error(DecodeError::unexpected(
                        node,
                        "node",
                        format!("unexpected node `{}`", name.escape_default()),
                    ));
                }
            }
        }

        Ok(Self)
    }
}

impl Config {
    pub fn load_default() -> Self {
        let res = Config::parse_inner(
            Path::new("default-config.kdl"),
            Path::new(""),
            DEFAULT_CONFIG,
            None,
        );

        // Includes in the default config can break its parsing at runtime.
        assert!(
            res.includes.is_empty(),
            "default config must not have includes",
        );

        res.config.unwrap()
    }

    pub fn load(path: &Path) -> ConfigParseResult<Self, miette::Report> {
        Self::load_in(path, path.parent().unwrap_or(Path::new("")))
    }

    /// Loads the config at `path`, with built-in engines' files in `engines/` under `dir`.
    pub fn load_in(path: &Path, dir: &Path) -> ConfigParseResult<Self, miette::Report> {
        let contents = match fs::read_to_string(path) {
            Ok(x) => x,
            Err(err) => {
                return ConfigParseResult::from_err(
                    miette!(err).context(format!("error reading {path:?}")),
                );
            }
        };

        Self::parse_inner(
            path,
            dir,
            &contents,
            Some(Box::new(|path: &Path| fs::read_to_string(path))),
        )
        .map_config_res(|res| {
            let config = res.context("error parsing")?;
            debug!("loaded config from {path:?}");
            Ok(config)
        })
    }

    /// Parses the config at `path`, with the running engine's file read from disk.
    pub fn parse(path: &Path, text: &str) -> ConfigParseResult<Self, ConfigIncludeError> {
        Self::parse_with_engine(path, text, |path| fs::read_to_string(path))
    }

    /// Parses the config at `path`, with `read_engine` reading the running engine's file from the
    /// path it gets, like `fs::read_to_string`. A file that isn't found is an empty one.
    pub fn parse_with_engine(
        path: &Path,
        text: &str,
        read_engine: impl FnOnce(&Path) -> std::io::Result<String>,
    ) -> ConfigParseResult<Self, ConfigIncludeError> {
        let dir = path.parent().unwrap_or(Path::new(""));
        Self::parse_inner(path, dir, text, Some(Box::new(read_engine)))
    }

    /// Parses the config at `path`, with built-in engines' files in `engines/` under `dir`.
    fn parse_inner(
        path: &Path,
        dir: &Path,
        text: &str,
        read_engine: Option<Box<ReadEngine<'_>>>,
    ) -> ConfigParseResult<Self, ConfigIncludeError> {
        let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let filename = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("config.kdl");

        let config = Rc::new(RefCell::new(Config::default()));
        let includes = Rc::new(RefCell::new(Includes(Vec::new())));
        let include_errors = Rc::new(RefCell::new(IncludeErrors(Vec::new())));
        let include_stack = HashSet::from([path.to_path_buf()]);

        let part = knuffel::parse_with_context::<ConfigPart, knuffel::span::Span, _>(
            filename,
            text,
            |ctx| {
                ctx.set(CurrentFile(path.to_path_buf()));
                ctx.set(BasePath(base.clone()));
                ctx.set(RootBase(base));
                ctx.set(Recursion(0));
                ctx.set(includes.clone());
                ctx.set(include_errors.clone());
                ctx.set(IncludeStack(include_stack));
                ctx.set(SawMruBinds(Rc::new(Cell::new(false))));
                ctx.set(config.clone());
            },
        );

        let mut includes = includes.take().0;
        let include_errors = include_errors.take().0;
        let mut part = part.map_err(move |err| ConfigIncludeError {
            main: err,
            includes: include_errors,
        });

        // The engine's file goes last, once the main config has said which engine runs.
        if let (Ok(_), Some(read_engine)) = (&part, read_engine) {
            let engine_path = engine_config_path_in(dir, &config.borrow().tiling.layout);
            // Watched even while it doesn't exist, so creating it applies it.
            includes.push(engine_path.clone());
            match read_engine(&engine_path) {
                Ok(text) => {
                    let parent = path.parent().unwrap_or(Path::new(""));
                    let name = engine_path.strip_prefix(parent).unwrap_or(&engine_path);
                    let name = name.to_string_lossy();
                    let engine = knuffel::parse_with_context::<EnginePart, knuffel::span::Span, _>(
                        &name,
                        &text,
                        |ctx| {
                            let base = engine_path.parent().map(Path::to_path_buf);
                            ctx.set(BasePath(base.unwrap_or_default()));
                            ctx.set(CurrentFile(engine_path.clone()));
                            ctx.set(EngineFile);
                            ctx.set(config.clone());
                        },
                    );
                    if let Err(err) = engine {
                        part = Err(ConfigIncludeError {
                            main: err,
                            includes: Vec::new(),
                        });
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => (),
                Err(err) => warn!("error reading {engine_path:?}: {err}"),
            }
        }

        let config = part.map(|_| config.take());
        ConfigParseResult { config, includes }
    }

    /// Parses a config that isn't on disk, without an engine file.
    pub fn parse_mem(text: &str) -> Result<Self, ConfigIncludeError> {
        Self::parse_inner(Path::new("config.kdl"), Path::new(""), text, None).config
    }

    /// Parses a config that isn't on disk, with `engine` as the running engine's file.
    pub fn parse_mem_with_engine(text: &str, engine: &str) -> Result<Self, ConfigIncludeError> {
        Self::parse_with_engine(Path::new("config.kdl"), text, |_| Ok(engine.to_owned())).config
    }
}

impl ConfigPath {
    /// Loads the config, returns an error if it doesn't exist.
    pub fn load(&self) -> ConfigParseResult<Config, miette::Report> {
        let _span = tracy_client::span!("ConfigPath::load");

        self.load_inner(|user_path, system_path| {
            Err(miette!(
                "no config file found; create one at {user_path:?} or {system_path:?}",
            ))
        })
        .map_config_res(|res| res.context("error loading config"))
    }

    /// Loads the config, or creates it if it doesn't exist.
    ///
    /// Returns a tuple containing the path that was created, if any, and the loaded config.
    ///
    /// If the config was created, but for some reason could not be read afterwards,
    /// this may return `(Some(_), Err(_))`.
    pub fn load_or_create(&self) -> (Option<&Path>, ConfigParseResult<Config, miette::Report>) {
        let _span = tracy_client::span!("ConfigPath::load_or_create");

        let mut created_at = None;

        let result = self
            .load_inner(|user_path, _| {
                Self::create(user_path, &mut created_at)
                    .map(|()| user_path)
                    .with_context(|| format!("error creating config at {user_path:?}"))
            })
            .map_config_res(|res| res.context("error loading config"));

        (created_at, self.create_engine_config(result))
    }

    /// Loads the config like [`ConfigPath::load`], first writing the running built-in engine's
    /// default file if it has none. For the compositor, which runs the engine.
    pub fn load_and_create_engine_config(&self) -> ConfigParseResult<Config, miette::Report> {
        let result = self.load();
        self.create_engine_config(result)
    }

    /// Writes the default file of the built-in engine that a loaded config runs, if it has none
    /// yet, and loads the config again with it.
    fn create_engine_config(
        &self,
        result: ConfigParseResult<Config, miette::Report>,
    ) -> ConfigParseResult<Config, miette::Report> {
        match &result.config {
            Ok(config) if tiling::create_engine_config(self.dir(), &config.tiling.layout) => {
                self.load()
            }
            _ => result,
        }
    }

    /// The directory of the user's config, where built-in engines' files are in `engines/`, even
    /// when bento runs the system config.
    fn dir(&self) -> &Path {
        let path = match self {
            ConfigPath::Explicit(path) => path,
            ConfigPath::Regular { user_path, .. } => user_path,
        };
        path.parent().unwrap_or(Path::new(""))
    }

    fn load_inner<'a>(
        &'a self,
        maybe_create: impl FnOnce(&'a Path, &'a Path) -> miette::Result<&'a Path>,
    ) -> ConfigParseResult<Config, miette::Report> {
        let path = match self {
            ConfigPath::Explicit(path) => path.as_path(),
            ConfigPath::Regular {
                user_path,
                system_path,
            } => {
                if user_path.exists() {
                    user_path.as_path()
                } else if system_path.exists() {
                    system_path.as_path()
                } else {
                    match maybe_create(user_path.as_path(), system_path.as_path()) {
                        Ok(x) => x,
                        Err(err) => return ConfigParseResult::from_err(miette!(err)),
                    }
                }
            }
        };
        Config::load_in(path, self.dir())
    }

    fn create<'a>(path: &'a Path, created_at: &mut Option<&'a Path>) -> miette::Result<()> {
        if let Some(default_parent) = path.parent() {
            fs::create_dir_all(default_parent)
                .into_diagnostic()
                .with_context(|| format!("error creating config directory {default_parent:?}"))?;
        }

        // Create the config and fill it with the default config if it doesn't exist.
        let mut new_file = match File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
        {
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
            res => res,
        }
        .into_diagnostic()
        .with_context(|| format!("error opening config file at {path:?}"))?;

        *created_at = Some(path);

        let default = include_bytes!("../../resources/default-config.kdl");

        new_file
            .write_all(default)
            .into_diagnostic()
            .with_context(|| format!("error writing default config to {path:?}"))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use insta::{assert_debug_snapshot, assert_snapshot};
    use pretty_assertions::assert_eq;
    use smithay::input::keyboard::Keysym;

    use super::*;

    #[test]
    fn can_create_default_config() {
        let _ = Config::load_default();
    }

    #[test]
    fn default_repeat_params() {
        let config = Config::parse_mem("").unwrap();
        assert_eq!(config.input.keyboard.repeat_delay, 600);
        assert_eq!(config.input.keyboard.repeat_rate, 25);
    }

    #[track_caller]
    fn do_parse(text: &str) -> Config {
        Config::parse_mem(text)
            .map_err(miette::Report::new)
            .unwrap()
    }

    #[test]
    fn parse_release_binds_for_supported_triggers() {
        // Keys and mouse buttons have release events, so release sections are supported.
        do_parse(
            r#"
            binds {
                Mod+MouseLeft {
                    release { close-window; }
                }
                Mod+Q {
                    press { close-window; }
                    release { close-window; }
                }
            }
            "#,
        );
    }

    #[test]
    fn parse_release_binds_for_unsupported_triggers() {
        // Scroll ticks are dispatched as press events only, so scroll binds can't have a release
        // action.
        assert_parse_error(
            r#"
            binds {
                Mod+WheelScrollDown {
                    release { toggle-overview; }
                }
            }
            "#,
            "release sections are not supported for scroll binds",
        );
        assert_parse_error(
            r#"
            binds {
                Mod+TouchpadScrollDown {
                    release { toggle-overview; }
                }
            }
            "#,
            "release sections are not supported for scroll binds",
        );
    }

    #[test]
    fn parse_bind_repeat_on_release_only() {
        // A release-only bind has no press action to repeat, so repeat=true is rejected.
        assert_parse_error(
            r#"
            binds {
                Mod+Q repeat=true {
                    release { close-window; }
                }
            }
            "#,
            "release-only binds cannot repeat",
        );

        do_parse(
            r#"
            binds {
                Mod+T repeat=true { spawn "alacritty"; }
                Mod+Q repeat=true {
                    press { close-window; }
                    release { close-window; }
                }
                Mod+W {
                    release { close-window; }
                }
                Mod+E repeat=false {
                    release { close-window; }
                }
            }
            "#,
        );
    }

    #[test]
    fn parse_bind_pass_through() {
        let config = do_parse(
            r#"
            binds {
                Mod+A pass-through=true { close-window; }
                Mod+B pass-through=false { close-window; }
                Mod+C { close-window; } // Should be off by default
            }
            "#,
        );

        assert_eq!(config.binds.0[0].pass_through, Some(true));
        assert_eq!(config.binds.0[1].pass_through, Some(false));
        assert_eq!(config.binds.0[2].pass_through, None);
    }

    #[test]
    fn parse_bind_pass_through_on_pointer_binds() {
        assert_parse_error(
            r#"
            binds {
                Mod+MouseLeft pass-through=true { close-window; }
            }
            "#,
            "pass-through is only supported for keyboard binds",
        );
    }

    /// Asserts that parsing fails with a diagnostic containing `message`.
    #[track_caller]
    fn assert_parse_error(text: &str, message: &str) {
        let err = match Config::parse_mem(text) {
            Ok(_) => panic!("expected a parse error for:\n{text}"),
            Err(err) => err,
        };

        // knuffel keeps the individual diagnostics in a private field, so check the Debug
        // representation, which includes their messages.
        let err = format!("{err:?}");
        assert!(
            err.contains(message),
            "expected {message:?} in parse error:\n{err}"
        );
    }

    #[test]
    fn parse_bind_section_errors() {
        // A direct action node and a `press` section both specify the press action, so mixing them
        // is reported as mixing the two syntaxes rather than as a duplicate `press` section.
        assert_parse_error(
            r#"
            binds {
                Mod+Q {
                    close-window;
                    press { center-floating-window; }
                }
            }
            "#,
            "cannot mix direct actions with press/release sections",
        );

        // A second direct action is a duplicate action, not a mix of the two syntaxes.
        assert_parse_error(
            r#"
            binds {
                Mod+Q {
                    close-window;
                    center-floating-window;
                }
            }
            "#,
            "only one action is allowed per keybind",
        );

        // A direct action following a `press` section is mixing the two syntaxes.
        assert_parse_error(
            r#"
            binds {
                Mod+Q {
                    press { close-window; }
                    center-floating-window;
                }
            }
            "#,
            "cannot mix direct actions with press/release sections",
        );

        // Two `press` sections are still duplicates.
        assert_parse_error(
            r#"
            binds {
                Mod+Q {
                    press { close-window; }
                    press { center-floating-window; }
                }
            }
            "#,
            "duplicate `press` section",
        );

        assert_parse_error(
            r#"
            binds {
                Mod+Q {
                    center-floating-window;
                    release { close-window; }
                }
            }
            "#,
            "cannot mix direct actions with press/release sections",
        );

        assert_parse_error(
            r#"
            binds {
                Mod+Q {
                    release { close-window; }
                    center-floating-window;
                }
            }
            "#,
            "cannot mix direct actions with press/release sections",
        );
    }

    #[test]
    fn parse_bind_bound_action() {
        let config = do_parse(
            r#"
            binds {
                Mod+A { close-window; }
                Mod+B {
                    press { close-window; }
                }
                Mod+C {
                    release { toggle-overview; }
                }
                Mod+D {
                    press { close-window; }
                    release { toggle-overview; }
                }
            }
            "#,
        );

        let action = |index: usize| &config.binds.0[index].action;

        // A direct action and a press section both bind only the press phase.
        assert!(matches!(action(0), BoundAction::Press(Action::CloseWindow)));
        assert!(matches!(action(1), BoundAction::Press(Action::CloseWindow)));

        // A release section binds only the release phase.
        assert!(matches!(
            action(2),
            BoundAction::Release(Action::ToggleOverview)
        ));

        // Both sections bind both phases.
        assert!(matches!(
            action(3),
            BoundAction::Both {
                press: Action::CloseWindow,
                release: Action::ToggleOverview,
            }
        ));
    }

    #[test]
    fn parse_bare_modifier_keys() {
        let config = do_parse(
            r#"
            binds {
                Mod {
                    release { toggle-overview; }
                }
                Ctrl {
                    release { close-window; }
                }
                Shift {
                    release { close-window; }
                }
                Alt {
                    release { close-window; }
                }
                Super {
                    release { close-window; }
                }
                Mod5 {
                    release { close-window; }
                }
                Mod3 {
                    release { close-window; }
                }

                Ctrl+Alt_L {
                    release { close-window; }
                }
            }
            "#,
        );

        let triggers = config
            .binds
            .0
            .iter()
            .map(|bind| bind.key.trigger)
            .collect::<Vec<_>>();

        assert_eq!(
            triggers,
            vec![
                Trigger::CompositorMod,
                Trigger::Modifier(ModKey::Ctrl),
                Trigger::Modifier(ModKey::Shift),
                Trigger::Modifier(ModKey::Alt),
                Trigger::Modifier(ModKey::Super),
                Trigger::Modifier(ModKey::IsoLevel3Shift),
                Trigger::Modifier(ModKey::IsoLevel5Shift),
                Trigger::Keysym(Keysym::Alt_L),
            ]
        );

        // Ctrl+Alt_L has modifiers of Ctrl (manually specified) and Alt (implied by the keysym).
        assert_eq!(
            config.binds.0[7].key.modifiers,
            Modifiers::CTRL | Modifiers::ALT
        );
    }

    #[test]
    fn parse_bind_conflicting_modifier_keys() {
        // A modifier key can be spelled as the modifier itself (`Alt`) or as its keysym (`Alt_L`),
        // and a modifier key is triggered by either of its keysyms, so one of these two binds
        // would always shadow the other.
        assert_parse_error(
            r#"
            binds {
                Alt {
                    release { toggle-overview; }
                }
                Alt_L {
                    release { close-window; }
                }
            }
            "#,
            "conflicting keybind later defined here",
        );
        assert_parse_error(
            r#"
            recent-windows {
                binds {
                    Alt+Control_L { next-window; }
                    Alt+Ctrl { next-window; }
                }
            }
            "#,
            "conflicting keybind later defined here",
        );

        // The left and right keysyms are different keys, so those can be bound separately.
        do_parse(
            r#"
            binds {
                Alt_L {
                    release { close-window; }
                }
                Alt_R {
                    release { center-floating-window; }
                }
            }
            "#,
        );

        // A modifier keysym on its own is an ordinary modifier-only bind.
        let config = do_parse(
            r#"
            binds {
                Alt_L {
                    release { close-window; }
                }
            }
            "#,
        );
        assert!(config.binds.0[0].key.trigger.is_modifier());
    }

    #[test]
    fn parse_recent_windows_bind_modifier() {
        // Modifier-only binds are allowed.
        do_parse(
            r#"
            recent-windows {
                binds {
                    Ctrl+Alt_L { next-window; }
                    Ctrl+Shift { next-window; }
                }
            }
            "#,
        );

        // Binds must have an extra modifier.
        assert_parse_error(
            r#"
            recent-windows {
                binds {
                    Alt_L { next-window; }
                }
            }
            "#,
            "keybind must have a modifier key",
        );
        assert_parse_error(
            r#"
            recent-windows {
                binds {
                    Alt+Alt_L { next-window; }
                }
            }
            "#,
            "keybind must have a modifier key",
        );

        // A trigger with no modifier at all is rejected.
        assert_parse_error(
            r#"
            recent-windows {
                binds {
                    Tab { next-window; }
                }
            }
            "#,
            "keybind must have a modifier key",
        );

        // Only keyboard keys are supported here for now.
        assert_parse_error(
            r#"
            recent-windows {
                binds {
                    Alt+MouseLeft { next-window; }
                }
            }
            "#,
            "key must be a keyboard key",
        );
    }

    #[test]
    fn parse_bind_redundant_modifiers_are_the_same_bind() {
        // Redundant modifiers are normalized, so these should error as duplicates.
        assert_parse_error(
            r#"
            binds {
                Alt_L { close-window; }
                Alt+Alt_L { center-floating-window; }
            }
            "#,
            "duplicate keybind later defined here",
        );
        assert_parse_error(
            r#"
            binds {
                Alt { close-window; }
                Alt+Alt { center-floating-window; }
            }
            "#,
            "duplicate keybind later defined here",
        );
        assert_parse_error(
            r#"
            binds {
                Mod { close-window; }
                Mod+Mod { center-floating-window; }
            }
            "#,
            "duplicate keybind later defined here",
        );
        assert_parse_error(
            r#"
            recent-windows {
                binds {
                    Alt+Control_L { next-window; }
                    Alt+Ctrl+Control_L { next-window; }
                }
            }
            "#,
            "duplicate keybind later defined here",
        );
    }

    #[test]
    fn parse_on_xdg_activate() {
        let parsed = do_parse(
            r#"
            window-rule { on-xdg-activate "ignore"; }
            window-rule { on-xdg-activate "set-urgent"; }
            window-rule { on-xdg-activate "focus"; }
            "#,
        );

        assert_eq!(
            parsed
                .window_rules
                .iter()
                .map(|rule| rule.on_xdg_activate)
                .collect::<Vec<_>>(),
            vec![
                Some(OnXdgActivate::Ignore),
                Some(OnXdgActivate::SetUrgent),
                Some(OnXdgActivate::Focus),
            ]
        );
    }

    #[test]
    fn parse() {
        let parsed = do_parse(
            r##"
            input {
                keyboard {
                    repeat-delay 600
                    repeat-rate 25
                    track-layout "window"
                    xkb {
                        layout "us,ru"
                        options "grp:win_space_toggle"
                    }
                }

                touchpad {
                    tap
                    dwt
                    dwtp
                    drag true
                    click-method "clickfinger"
                    accel-speed 0.2
                    accel-profile "flat"
                    scroll-method "two-finger"
                    scroll-button 272
                    scroll-button-lock
                    tap-button-map "left-middle-right"
                    disabled-on-external-mouse
                    scroll-factor 0.9
                    pinch-sensitivity 1.8
                }

                mouse {
                    natural-scroll
                    accel-speed 0.4
                    accel-profile "flat"
                    scroll-method "no-scroll"
                    scroll-button 273
                    middle-emulation
                    scroll-factor 0.2
                }

                trackpoint {
                    off
                    natural-scroll
                    accel-speed 0.0
                    accel-profile "flat"
                    scroll-method "on-button-down"
                    scroll-button 274
                }

                trackball {
                    off
                    natural-scroll
                    accel-speed 0.0
                    accel-profile "flat"
                    scroll-method "edge"
                    scroll-button 275
                    scroll-button-lock
                    left-handed
                    middle-emulation
                }

                tablet {
                    map-to-output "eDP-1"
                    map-to-focused-output
                    map-to-focused-window
                    calibration-matrix 1.0 2.0 3.0 \
                                       4.0 5.0 6.0
                }

                touch {
                    map-to-output "eDP-1"
                }

                disable-power-key-handling

                warp-mouse-to-focus
                focus-follows-mouse
                workspace-auto-back-and-forth

                mod-key "Mod5"
                mod-key-nested "Super"
            }

            output "eDP-1" {
                focus-at-startup
                scale 2
                transform "flipped-90"
                position x=10 y=20
                mode "1920x1080@144"
                max-bpc 10
                variable-refresh-rate on-demand=true
                background-color "rgba(25, 25, 102, 1.0)"
                hot-corners {
                    off
                    top-left
                    top-right
                    bottom-left
                    bottom-right
                }
            }

            output "eDP-2" {
                mode custom=true "1920x1080@144"
            }

            output "eDP-3" {
                modeline 173.00  1920 2048 2248 2576  1080 1083 1088 1120 "-hsync" "+vsync"
            }

            layout {
                focus-ring {
                    width 5
                    active-color 0 100 200 255
                    inactive-color 255 200 100 0
                    active-gradient from="rgba(10, 20, 30, 1.0)" to="#0080ffff" relative-to="workspace-view"
                }

                border {
                    width 3
                    inactive-color "rgba(255, 200, 100, 0.0)"
                }

                shadow {
                    offset x=10 y=-20
                }

                gaps 8

                struts {
                    left 1
                    right 2
                    top 3
                }

            }

            spawn-at-startup "alacritty" "-e" "fish"
            spawn-sh-at-startup "qs -c ~/source/qs/MyAwesomeShell"

            prefer-no-csd

            cursor {
                xcursor-theme "breeze_cursors"
                xcursor-size 16
                hide-when-typing
                hide-after-inactive-ms 3000
            }

            screenshot-path "~/Screenshots/screenshot.png"

            clipboard {
                disable-primary
            }

            hotkey-overlay {
                skip-at-startup
            }

            animations {
                slowdown 2.0

                workspace-switch {
                    spring damping-ratio=1.0 stiffness=1000 epsilon=0.0001
                }

                horizontal-view-movement {
                    duration-ms 100
                    curve "ease-out-expo"
                }

                window-open { off; }

                window-close {
                    curve "cubic-bezier" 0.05 0.7 0.1 1
                }

                recent-windows-close {
                    off
                }
            }

            gestures {
                dnd-edge-view-scroll {
                    trigger-width 10
                    max-speed 50
                }
            }

            environment {
                QT_QPA_PLATFORM "wayland"
                DISPLAY null
            }

            window-rule {
                match app-id=".*alacritty"
                exclude title="~"
                exclude is-active=true is-focused=false

                open-on-output "eDP-1"
                open-fullscreen false
                open-floating false
                open-focused true
                default-floating-position x=100 y=-200 relative-to="bottom-left"
                on-xdg-activate "ignore"
                variable-refresh-rate "fullscreen-only"

                focus-ring {
                    off
                    width 3
                }

                border {
                    on
                    width 8.5
                }

                pinch-sensitivity 1.2
            }

            layer-rule {
                match namespace="^notifications$"
                block-out-from "screencast"
            }

            binds {
                Mod+Escape hotkey-overlay-title="Inhibit" { toggle-keyboard-shortcuts-inhibit; }
                Mod+Shift+Escape allow-inhibiting=true { toggle-keyboard-shortcuts-inhibit; }
                Mod+T allow-when-locked=true { spawn "alacritty"; }
                Mod+Q hotkey-overlay-title=null { close-window; }
                Mod+Shift+H { focus-monitor-left; }
                Mod+Shift+O { focus-monitor "eDP-1"; }
                Mod+Ctrl+Shift+L { move-window-to-monitor-right; }
                Mod+Ctrl+Alt+O { move-window-to-monitor "eDP-1"; }
                Mod+Comma { center-floating-window; }
                Mod+1 { focus-workspace 1; }
                Mod+Shift+1 { focus-workspace "workspace-1"; }
                Mod+Shift+E allow-inhibiting=false { quit skip-confirmation=true; }
                Mod+WheelScrollDown cooldown-ms=150 { focus-workspace-down; }
                Super+Alt+S allow-when-locked=true { spawn-sh "pkill orca || exec orca"; }
                Mod {
                    release { toggle-overview; }
                }
                Shift+Mod {
                    release { toggle-window-floating; }
                }
            }

            switch-events {
                tablet-mode-on { spawn "bash" "-c" "gsettings set org.gnome.desktop.a11y.applications screen-keyboard-enabled true"; }
                tablet-mode-off { spawn "bash" "-c" "gsettings set org.gnome.desktop.a11y.applications screen-keyboard-enabled false"; }
            }

            debug {
                render-drm-device "/dev/dri/renderD129"
                ignore-drm-device "/dev/dri/renderD128"
                ignore-drm-device "/dev/dri/renderD130"
            }

            workspace "workspace-1" {
                open-on-output "eDP-1"
                open-on-output "DP-1"
            }
            workspace "workspace-2"
            workspace "workspace-3"

            recent-windows {
                off

                highlight {
                    padding 15
                    active-color "#00ff00"
                }

                previews {
                    max-height 960
                }

                binds {
                    Alt+Tab { next-window; }
                    Alt+grave { next-window filter="app-id"; }
                    Super+Tab { next-window scope="output"; }
                }
            }
            "##,
        );

        assert_debug_snapshot!(parsed, @r#"
        Config {
            input: Input {
                keyboard: Keyboard {
                    xkb: Xkb {
                        rules: "",
                        model: "",
                        layout: "us,ru",
                        variant: "",
                        options: Some(
                            "grp:win_space_toggle",
                        ),
                        file: None,
                    },
                    repeat_delay: 600,
                    repeat_rate: 25,
                    track_layout: Window,
                    numlock: false,
                },
                touchpad: Touchpad {
                    off: false,
                    tap: true,
                    dwt: true,
                    dwtp: true,
                    drag: Some(
                        true,
                    ),
                    drag_lock: false,
                    natural_scroll: false,
                    click_method: Some(
                        Clickfinger,
                    ),
                    accel_speed: FloatOrInt(
                        0.2,
                    ),
                    accel_profile: Some(
                        Flat,
                    ),
                    scroll_method: Some(
                        TwoFinger,
                    ),
                    scroll_button: Some(
                        272,
                    ),
                    scroll_button_lock: true,
                    tap_button_map: Some(
                        LeftMiddleRight,
                    ),
                    left_handed: false,
                    disabled_on_external_mouse: true,
                    middle_emulation: false,
                    scroll_factor: Some(
                        ScrollFactor {
                            base: Some(
                                FloatOrInt(
                                    0.9,
                                ),
                            ),
                            horizontal: None,
                            vertical: None,
                        },
                    ),
                    pinch_sensitivity: Some(
                        FloatOrInt(
                            1.8,
                        ),
                    ),
                },
                mouse: Mouse {
                    off: false,
                    natural_scroll: true,
                    accel_speed: FloatOrInt(
                        0.4,
                    ),
                    accel_profile: Some(
                        Flat,
                    ),
                    scroll_method: Some(
                        NoScroll,
                    ),
                    scroll_button: Some(
                        273,
                    ),
                    scroll_button_lock: false,
                    left_handed: false,
                    middle_emulation: true,
                    scroll_factor: Some(
                        ScrollFactor {
                            base: Some(
                                FloatOrInt(
                                    0.2,
                                ),
                            ),
                            horizontal: None,
                            vertical: None,
                        },
                    ),
                },
                trackpoint: Trackpoint {
                    off: true,
                    natural_scroll: true,
                    accel_speed: FloatOrInt(
                        0.0,
                    ),
                    accel_profile: Some(
                        Flat,
                    ),
                    scroll_method: Some(
                        OnButtonDown,
                    ),
                    scroll_button: Some(
                        274,
                    ),
                    scroll_button_lock: false,
                    left_handed: false,
                    middle_emulation: false,
                },
                trackball: Trackball {
                    off: true,
                    natural_scroll: true,
                    accel_speed: FloatOrInt(
                        0.0,
                    ),
                    accel_profile: Some(
                        Flat,
                    ),
                    scroll_method: Some(
                        Edge,
                    ),
                    scroll_button: Some(
                        275,
                    ),
                    scroll_button_lock: true,
                    left_handed: true,
                    middle_emulation: true,
                },
                tablet: Tablet {
                    off: false,
                    calibration_matrix: Some(
                        [
                            1.0,
                            2.0,
                            3.0,
                            4.0,
                            5.0,
                            6.0,
                        ],
                    ),
                    map_to_output: Some(
                        "eDP-1",
                    ),
                    map_to_focused_output: true,
                    map_to_focused_window: true,
                    left_handed: false,
                },
                touch: Touch {
                    off: false,
                    calibration_matrix: None,
                    map_to_output: Some(
                        "eDP-1",
                    ),
                },
                disable_power_key_handling: true,
                warp_mouse_to_focus: Some(
                    WarpMouseToFocus {
                        mode: None,
                    },
                ),
                focus_follows_mouse: Some(
                    FocusFollowsMouse {
                        max_scroll_amount: None,
                    },
                ),
                workspace_auto_back_and_forth: true,
                mod_key: Some(
                    IsoLevel3Shift,
                ),
                mod_key_nested: Some(
                    Super,
                ),
            },
            outputs: Outputs(
                [
                    Output {
                        off: false,
                        name: "eDP-1",
                        scale: Some(
                            FloatOrInt(
                                2.0,
                            ),
                        ),
                        transform: Flipped90,
                        position: Some(
                            Position {
                                x: 10,
                                y: 20,
                            },
                        ),
                        max_bpc: Some(
                            MaxBpc(
                                _10,
                            ),
                        ),
                        mode: Some(
                            Mode {
                                custom: false,
                                mode: ConfiguredMode {
                                    width: 1920,
                                    height: 1080,
                                    refresh: Some(
                                        144.0,
                                    ),
                                },
                            },
                        ),
                        modeline: None,
                        variable_refresh_rate: Some(
                            Vrr {
                                on_demand: true,
                            },
                        ),
                        focus_at_startup: true,
                        background_color: Some(
                            Color {
                                r: 0.09803922,
                                g: 0.09803922,
                                b: 0.4,
                                a: 1.0,
                            },
                        ),
                        backdrop_color: None,
                        hot_corners: Some(
                            HotCorners {
                                off: true,
                                top_left: true,
                                top_right: true,
                                bottom_left: true,
                                bottom_right: true,
                            },
                        ),
                        layout: None,
                    },
                    Output {
                        off: false,
                        name: "eDP-2",
                        scale: None,
                        transform: Normal,
                        position: None,
                        max_bpc: None,
                        mode: Some(
                            Mode {
                                custom: true,
                                mode: ConfiguredMode {
                                    width: 1920,
                                    height: 1080,
                                    refresh: Some(
                                        144.0,
                                    ),
                                },
                            },
                        ),
                        modeline: None,
                        variable_refresh_rate: None,
                        focus_at_startup: false,
                        background_color: None,
                        backdrop_color: None,
                        hot_corners: None,
                        layout: None,
                    },
                    Output {
                        off: false,
                        name: "eDP-3",
                        scale: None,
                        transform: Normal,
                        position: None,
                        max_bpc: None,
                        mode: None,
                        modeline: Some(
                            Modeline {
                                clock: 173.0,
                                hdisplay: 1920,
                                hsync_start: 2048,
                                hsync_end: 2248,
                                htotal: 2576,
                                vdisplay: 1080,
                                vsync_start: 1083,
                                vsync_end: 1088,
                                vtotal: 1120,
                                hsync_polarity: NHSync,
                                vsync_polarity: PVSync,
                            },
                        ),
                        variable_refresh_rate: None,
                        focus_at_startup: false,
                        background_color: None,
                        backdrop_color: None,
                        hot_corners: None,
                        layout: None,
                    },
                ],
            ),
            spawn_at_startup: [
                SpawnAtStartup {
                    command: [
                        "alacritty",
                        "-e",
                        "fish",
                    ],
                },
            ],
            spawn_sh_at_startup: [
                SpawnShAtStartup {
                    command: "qs -c ~/source/qs/MyAwesomeShell",
                },
            ],
            shell: None,
            layout: Layout {
                focus_ring: FocusRing {
                    off: false,
                    width: 5.0,
                    active_color: Color {
                        r: 0.0,
                        g: 0.39215687,
                        b: 0.78431374,
                        a: 1.0,
                    },
                    inactive_color: Color {
                        r: 1.0,
                        g: 0.78431374,
                        b: 0.39215687,
                        a: 0.0,
                    },
                    urgent_color: Color {
                        r: 0.60784316,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    },
                    active_gradient: Some(
                        Gradient {
                            from: Color {
                                r: 0.039215688,
                                g: 0.078431375,
                                b: 0.11764706,
                                a: 1.0,
                            },
                            to: Color {
                                r: 0.0,
                                g: 0.5019608,
                                b: 1.0,
                                a: 1.0,
                            },
                            angle: 180,
                            relative_to: WorkspaceView,
                            in_: GradientInterpolation {
                                color_space: Srgb,
                                hue_interpolation: Shorter,
                            },
                        },
                    ),
                    inactive_gradient: None,
                    urgent_gradient: None,
                },
                border: Border {
                    off: false,
                    width: 3.0,
                    active_color: Color {
                        r: 1.0,
                        g: 0.78431374,
                        b: 0.49803922,
                        a: 1.0,
                    },
                    inactive_color: Color {
                        r: 1.0,
                        g: 0.78431374,
                        b: 0.39215687,
                        a: 0.0,
                    },
                    urgent_color: Color {
                        r: 0.60784316,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    },
                    active_gradient: None,
                    inactive_gradient: None,
                    urgent_gradient: None,
                },
                shadow: Shadow {
                    on: false,
                    offset: ShadowOffset {
                        x: FloatOrInt(
                            10.0,
                        ),
                        y: FloatOrInt(
                            -20.0,
                        ),
                    },
                    softness: 30.0,
                    spread: 5.0,
                    draw_behind_window: false,
                    color: Color {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.46666667,
                    },
                    inactive_color: None,
                },
                empty_workspace_above_first: false,
                gaps: 8.0,
                struts: Struts {
                    left: FloatOrInt(
                        1.0,
                    ),
                    right: FloatOrInt(
                        2.0,
                    ),
                    top: FloatOrInt(
                        3.0,
                    ),
                    bottom: FloatOrInt(
                        0.0,
                    ),
                },
                background_color: Color {
                    r: 0.25,
                    g: 0.25,
                    b: 0.25,
                    a: 1.0,
                },
                fullscreen_backdrop_color: Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                },
                background_image: None,
            },
            tiling: Tiling {
                layout: BuiltIn(
                    "bento",
                ),
                options: [],
                output_options: [],
                workspace_options: [],
                window_rules: [],
            },
            prefer_no_csd: true,
            cursor: Cursor {
                xcursor_theme: "breeze_cursors",
                xcursor_size: 16,
                hide_when_typing: true,
                hide_after_inactive_ms: Some(
                    3000,
                ),
            },
            screenshot_path: ScreenshotPath(
                Some(
                    "~/Screenshots/screenshot.png",
                ),
            ),
            clipboard: Clipboard {
                disable_primary: true,
            },
            hotkey_overlay: HotkeyOverlay {
                skip_at_startup: true,
                hide_not_bound: false,
            },
            config_notification: ConfigNotification {
                disable_failed: false,
            },
            animations: Animations {
                off: false,
                slowdown: 2.0,
                workspace_switch: WorkspaceSwitchAnim(
                    Animation {
                        off: false,
                        kind: Spring(
                            SpringParams {
                                damping_ratio: 1.0,
                                stiffness: 1000,
                                epsilon: 0.0001,
                            },
                        ),
                    },
                ),
                window_open: WindowOpenAnim {
                    anim: Animation {
                        off: true,
                        kind: Easing(
                            EasingParams {
                                duration_ms: 150,
                                curve: EaseOutExpo,
                            },
                        ),
                    },
                    custom_shader: None,
                },
                window_close: WindowCloseAnim {
                    anim: Animation {
                        off: false,
                        kind: Easing(
                            EasingParams {
                                duration_ms: 150,
                                curve: CubicBezier(
                                    0.05,
                                    0.7,
                                    0.1,
                                    1.0,
                                ),
                            },
                        ),
                    },
                    custom_shader: None,
                },
                horizontal_view_movement: HorizontalViewMovementAnim(
                    Animation {
                        off: false,
                        kind: Easing(
                            EasingParams {
                                duration_ms: 100,
                                curve: EaseOutExpo,
                            },
                        ),
                    },
                ),
                window_movement: WindowMovementAnim(
                    Animation {
                        off: false,
                        kind: Spring(
                            SpringParams {
                                damping_ratio: 1.0,
                                stiffness: 800,
                                epsilon: 0.0001,
                            },
                        ),
                    },
                ),
                window_resize: WindowResizeAnim {
                    anim: Animation {
                        off: false,
                        kind: Spring(
                            SpringParams {
                                damping_ratio: 1.0,
                                stiffness: 800,
                                epsilon: 0.0001,
                            },
                        ),
                    },
                    custom_shader: None,
                },
                config_notification_open_close: ConfigNotificationOpenCloseAnim(
                    Animation {
                        off: false,
                        kind: Spring(
                            SpringParams {
                                damping_ratio: 0.6,
                                stiffness: 1000,
                                epsilon: 0.001,
                            },
                        ),
                    },
                ),
                exit_confirmation_open_close: ExitConfirmationOpenCloseAnim(
                    Animation {
                        off: false,
                        kind: Spring(
                            SpringParams {
                                damping_ratio: 0.6,
                                stiffness: 500,
                                epsilon: 0.01,
                            },
                        ),
                    },
                ),
                screenshot_ui_open: ScreenshotUiOpenAnim(
                    Animation {
                        off: false,
                        kind: Easing(
                            EasingParams {
                                duration_ms: 200,
                                curve: EaseOutQuad,
                            },
                        ),
                    },
                ),
                overview_open_close: OverviewOpenCloseAnim(
                    Animation {
                        off: false,
                        kind: Spring(
                            SpringParams {
                                damping_ratio: 1.0,
                                stiffness: 800,
                                epsilon: 0.0001,
                            },
                        ),
                    },
                ),
                recent_windows_close: RecentWindowsCloseAnim(
                    Animation {
                        off: true,
                        kind: Spring(
                            SpringParams {
                                damping_ratio: 1.0,
                                stiffness: 800,
                                epsilon: 0.001,
                            },
                        ),
                    },
                ),
            },
            blur: Blur {
                off: false,
                passes: 2,
                offset: 2.05,
                noise: 0.01,
                saturation: 1.15,
                shade: 0.7,
                shade_dark: Color {
                    r: 0.13333334,
                    g: 0.13333334,
                    b: 0.14901961,
                    a: 1.0,
                },
                shade_light: Color {
                    r: 0.98039216,
                    g: 0.98039216,
                    b: 0.9843137,
                    a: 1.0,
                },
                material: Glass,
                glass: Glass {
                    bezel: 20.0,
                    thickness: 40.0,
                    refractive_index: 1.5,
                    dispersion: 0.14,
                    reflection: 0.5,
                    sheen: 0.6,
                    color: Color {
                        r: 0.9411765,
                        g: 0.96862745,
                        b: 0.95686275,
                        a: 1.0,
                    },
                    follow_accent: false,
                    pattern: Smooth,
                    pattern_depth: 0.5,
                    pointer_light: 0.5,
                    lights: [
                        Some(
                            GlassLight {
                                on: true,
                                intensity: 0.6,
                                angle: 315.0,
                                height: 40.0,
                                size: 0.35,
                            },
                        ),
                        None,
                        None,
                        None,
                    ],
                },
            },
            gestures: Gestures {
                dnd_edge_view_scroll: DndEdgeViewScroll {
                    trigger_width: 10.0,
                    delay_ms: 100,
                    max_speed: 50.0,
                },
                dnd_edge_workspace_switch: DndEdgeWorkspaceSwitch {
                    trigger_height: 50.0,
                    delay_ms: 100,
                    max_speed: 1500.0,
                },
                hot_corners: HotCorners {
                    off: false,
                    top_left: false,
                    top_right: false,
                    bottom_left: false,
                    bottom_right: false,
                },
            },
            overview: Overview {
                zoom: 0.5,
                backdrop_color: Color {
                    r: 0.15,
                    g: 0.15,
                    b: 0.15,
                    a: 1.0,
                },
                workspace_shadow: WorkspaceShadow {
                    off: false,
                    offset: ShadowOffset {
                        x: FloatOrInt(
                            0.0,
                        ),
                        y: FloatOrInt(
                            10.0,
                        ),
                    },
                    softness: 40.0,
                    spread: 10.0,
                    color: Color {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.3137255,
                    },
                },
            },
            environment: Environment(
                [
                    EnvironmentVariable {
                        name: "QT_QPA_PLATFORM",
                        value: Some(
                            "wayland",
                        ),
                    },
                    EnvironmentVariable {
                        name: "DISPLAY",
                        value: None,
                    },
                ],
            ),
            xwayland_satellite: XwaylandSatellite {
                off: false,
                path: "xwayland-satellite",
            },
            window_rules: [
                WindowRule {
                    matches: [
                        Match {
                            app_id: Some(
                                RegexEq(
                                    Regex(
                                        ".*alacritty",
                                    ),
                                ),
                            ),
                            title: None,
                            is_active: None,
                            is_focused: None,
                            is_floating: None,
                            is_window_cast_target: None,
                            is_urgent: None,
                            at_startup: None,
                        },
                    ],
                    excludes: [
                        Match {
                            app_id: None,
                            title: Some(
                                RegexEq(
                                    Regex(
                                        "~",
                                    ),
                                ),
                            ),
                            is_active: None,
                            is_focused: None,
                            is_floating: None,
                            is_window_cast_target: None,
                            is_urgent: None,
                            at_startup: None,
                        },
                        Match {
                            app_id: None,
                            title: None,
                            is_active: Some(
                                true,
                            ),
                            is_focused: Some(
                                false,
                            ),
                            is_floating: None,
                            is_window_cast_target: None,
                            is_urgent: None,
                            at_startup: None,
                        },
                    ],
                    open_on_output: Some(
                        "eDP-1",
                    ),
                    open_on_workspace: None,
                    open_maximized_to_edges: None,
                    open_fullscreen: Some(
                        false,
                    ),
                    open_floating: Some(
                        false,
                    ),
                    open_focused: Some(
                        true,
                    ),
                    on_xdg_activate: Some(
                        Ignore,
                    ),
                    min_width: None,
                    min_height: None,
                    max_width: None,
                    max_height: None,
                    focus_ring: BorderRule {
                        off: true,
                        on: false,
                        width: Some(
                            FloatOrInt(
                                3.0,
                            ),
                        ),
                        active_color: None,
                        inactive_color: None,
                        urgent_color: None,
                        active_gradient: None,
                        inactive_gradient: None,
                        urgent_gradient: None,
                    },
                    border: BorderRule {
                        off: false,
                        on: true,
                        width: Some(
                            FloatOrInt(
                                8.5,
                            ),
                        ),
                        active_color: None,
                        inactive_color: None,
                        urgent_color: None,
                        active_gradient: None,
                        inactive_gradient: None,
                        urgent_gradient: None,
                    },
                    shadow: ShadowRule {
                        off: false,
                        on: false,
                        offset: None,
                        softness: None,
                        spread: None,
                        draw_behind_window: None,
                        color: None,
                        inactive_color: None,
                    },
                    draw_border_with_background: None,
                    opacity: None,
                    geometry_corner_radius: None,
                    clip_to_geometry: None,
                    baba_is_float: None,
                    block_out_from: None,
                    variable_refresh_rate: Some(
                        FullscreenOnly,
                    ),
                    default_floating_position: Some(
                        FloatingPosition {
                            x: FloatOrInt(
                                100.0,
                            ),
                            y: FloatOrInt(
                                -200.0,
                            ),
                            relative_to: BottomLeft,
                        },
                    ),
                    scroll_factor: None,
                    pinch_sensitivity: Some(
                        FloatOrInt(
                            1.2,
                        ),
                    ),
                    tiled_state: None,
                    background_effect: BackgroundEffectRule {
                        xray: None,
                        blur: None,
                        noise: None,
                        saturation: None,
                        material: None,
                    },
                    popups: PopupsRule {
                        opacity: None,
                        geometry_corner_radius: None,
                        background_effect: BackgroundEffectRule {
                            xray: None,
                            blur: None,
                            noise: None,
                            saturation: None,
                            material: None,
                        },
                    },
                },
            ],
            layer_rules: [
                LayerRule {
                    matches: [
                        Match {
                            namespace: Some(
                                RegexEq(
                                    Regex(
                                        "^notifications$",
                                    ),
                                ),
                            ),
                            at_startup: None,
                            layer: None,
                        },
                    ],
                    excludes: [],
                    opacity: None,
                    block_out_from: Some(
                        Screencast,
                    ),
                    shadow: ShadowRule {
                        off: false,
                        on: false,
                        offset: None,
                        softness: None,
                        spread: None,
                        draw_behind_window: None,
                        color: None,
                        inactive_color: None,
                    },
                    geometry_corner_radius: None,
                    place_within_backdrop: None,
                    baba_is_float: None,
                    background_effect: BackgroundEffectRule {
                        xray: None,
                        blur: None,
                        noise: None,
                        saturation: None,
                        material: None,
                    },
                    popups: PopupsRule {
                        opacity: None,
                        geometry_corner_radius: None,
                        background_effect: BackgroundEffectRule {
                            xray: None,
                            blur: None,
                            noise: None,
                            saturation: None,
                            material: None,
                        },
                    },
                },
            ],
            binds: Binds(
                [
                    Bind {
                        key: Key {
                            trigger: Keysym(
                                XK_Escape,
                            ),
                            modifiers: Modifiers(
                                COMPOSITOR,
                            ),
                        },
                        action: Press(
                            ToggleKeyboardShortcutsInhibit,
                        ),
                        repeat: true,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: false,
                        allow_inhibiting: false,
                        hotkey_overlay_title: Some(
                            Some(
                                "Inhibit",
                            ),
                        ),
                    },
                    Bind {
                        key: Key {
                            trigger: Keysym(
                                XK_Escape,
                            ),
                            modifiers: Modifiers(
                                SHIFT | COMPOSITOR,
                            ),
                        },
                        action: Press(
                            ToggleKeyboardShortcutsInhibit,
                        ),
                        repeat: true,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: false,
                        allow_inhibiting: false,
                        hotkey_overlay_title: None,
                    },
                    Bind {
                        key: Key {
                            trigger: Keysym(
                                XK_t,
                            ),
                            modifiers: Modifiers(
                                COMPOSITOR,
                            ),
                        },
                        action: Press(
                            Spawn(
                                [
                                    "alacritty",
                                ],
                            ),
                        ),
                        repeat: true,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: true,
                        allow_inhibiting: true,
                        hotkey_overlay_title: None,
                    },
                    Bind {
                        key: Key {
                            trigger: Keysym(
                                XK_q,
                            ),
                            modifiers: Modifiers(
                                COMPOSITOR,
                            ),
                        },
                        action: Press(
                            CloseWindow,
                        ),
                        repeat: true,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: false,
                        allow_inhibiting: true,
                        hotkey_overlay_title: Some(
                            None,
                        ),
                    },
                    Bind {
                        key: Key {
                            trigger: Keysym(
                                XK_h,
                            ),
                            modifiers: Modifiers(
                                SHIFT | COMPOSITOR,
                            ),
                        },
                        action: Press(
                            FocusMonitorLeft,
                        ),
                        repeat: true,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: false,
                        allow_inhibiting: true,
                        hotkey_overlay_title: None,
                    },
                    Bind {
                        key: Key {
                            trigger: Keysym(
                                XK_o,
                            ),
                            modifiers: Modifiers(
                                SHIFT | COMPOSITOR,
                            ),
                        },
                        action: Press(
                            FocusMonitor(
                                "eDP-1",
                            ),
                        ),
                        repeat: true,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: false,
                        allow_inhibiting: true,
                        hotkey_overlay_title: None,
                    },
                    Bind {
                        key: Key {
                            trigger: Keysym(
                                XK_l,
                            ),
                            modifiers: Modifiers(
                                CTRL | SHIFT | COMPOSITOR,
                            ),
                        },
                        action: Press(
                            MoveWindowToMonitorRight,
                        ),
                        repeat: true,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: false,
                        allow_inhibiting: true,
                        hotkey_overlay_title: None,
                    },
                    Bind {
                        key: Key {
                            trigger: Keysym(
                                XK_o,
                            ),
                            modifiers: Modifiers(
                                CTRL | ALT | COMPOSITOR,
                            ),
                        },
                        action: Press(
                            MoveWindowToMonitor(
                                "eDP-1",
                            ),
                        ),
                        repeat: true,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: false,
                        allow_inhibiting: true,
                        hotkey_overlay_title: None,
                    },
                    Bind {
                        key: Key {
                            trigger: Keysym(
                                XK_comma,
                            ),
                            modifiers: Modifiers(
                                COMPOSITOR,
                            ),
                        },
                        action: Press(
                            CenterFloatingWindow,
                        ),
                        repeat: true,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: false,
                        allow_inhibiting: true,
                        hotkey_overlay_title: None,
                    },
                    Bind {
                        key: Key {
                            trigger: Keysym(
                                XK_1,
                            ),
                            modifiers: Modifiers(
                                COMPOSITOR,
                            ),
                        },
                        action: Press(
                            FocusWorkspace(
                                Index(
                                    1,
                                ),
                            ),
                        ),
                        repeat: true,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: false,
                        allow_inhibiting: true,
                        hotkey_overlay_title: None,
                    },
                    Bind {
                        key: Key {
                            trigger: Keysym(
                                XK_1,
                            ),
                            modifiers: Modifiers(
                                SHIFT | COMPOSITOR,
                            ),
                        },
                        action: Press(
                            FocusWorkspace(
                                Name(
                                    "workspace-1",
                                ),
                            ),
                        ),
                        repeat: true,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: false,
                        allow_inhibiting: true,
                        hotkey_overlay_title: None,
                    },
                    Bind {
                        key: Key {
                            trigger: Keysym(
                                XK_e,
                            ),
                            modifiers: Modifiers(
                                SHIFT | COMPOSITOR,
                            ),
                        },
                        action: Press(
                            Quit(
                                true,
                            ),
                        ),
                        repeat: true,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: false,
                        allow_inhibiting: false,
                        hotkey_overlay_title: None,
                    },
                    Bind {
                        key: Key {
                            trigger: WheelScrollDown,
                            modifiers: Modifiers(
                                COMPOSITOR,
                            ),
                        },
                        action: Press(
                            FocusWorkspaceDown,
                        ),
                        repeat: true,
                        pass_through: None,
                        cooldown: Some(
                            150ms,
                        ),
                        allow_when_locked: false,
                        allow_inhibiting: true,
                        hotkey_overlay_title: None,
                    },
                    Bind {
                        key: Key {
                            trigger: Keysym(
                                XK_s,
                            ),
                            modifiers: Modifiers(
                                ALT | SUPER,
                            ),
                        },
                        action: Press(
                            SpawnSh(
                                "pkill orca || exec orca",
                            ),
                        ),
                        repeat: true,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: true,
                        allow_inhibiting: true,
                        hotkey_overlay_title: None,
                    },
                    Bind {
                        key: Key {
                            trigger: CompositorMod,
                            modifiers: Modifiers(
                                COMPOSITOR,
                            ),
                        },
                        action: Release(
                            ToggleOverview,
                        ),
                        repeat: false,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: false,
                        allow_inhibiting: true,
                        hotkey_overlay_title: None,
                    },
                    Bind {
                        key: Key {
                            trigger: CompositorMod,
                            modifiers: Modifiers(
                                SHIFT | COMPOSITOR,
                            ),
                        },
                        action: Release(
                            ToggleWindowFloating,
                        ),
                        repeat: false,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: false,
                        allow_inhibiting: true,
                        hotkey_overlay_title: None,
                    },
                ],
            ),
            bind_files: BindFiles(16),
            switch_events: SwitchBinds {
                lid_open: None,
                lid_close: None,
                tablet_mode_on: Some(
                    SwitchAction {
                        spawn: [
                            "bash",
                            "-c",
                            "gsettings set org.gnome.desktop.a11y.applications screen-keyboard-enabled true",
                        ],
                    },
                ),
                tablet_mode_off: Some(
                    SwitchAction {
                        spawn: [
                            "bash",
                            "-c",
                            "gsettings set org.gnome.desktop.a11y.applications screen-keyboard-enabled false",
                        ],
                    },
                ),
            },
            debug: Debug {
                preview_render: None,
                dbus_interfaces_in_non_session_instances: false,
                wait_for_frame_completion_before_queueing: false,
                enable_overlay_planes: false,
                disable_cursor_plane: false,
                disable_direct_scanout: false,
                restrict_primary_scanout_to_matching_format: false,
                force_disable_connectors_on_resume: false,
                render_drm_device: Some(
                    "/dev/dri/renderD129",
                ),
                ignored_drm_devices: [
                    "/dev/dri/renderD128",
                    "/dev/dri/renderD130",
                ],
                force_pipewire_invalid_modifier: false,
                disable_pipewire_dmabuf: false,
                emulate_zero_presentation_time: false,
                disable_resize_throttling: false,
                disable_transactions: false,
                keep_laptop_panel_on_when_lid_is_closed: false,
                disable_monitor_names: false,
                strict_new_window_focus_policy: false,
                honor_xdg_activation_with_invalid_serial: false,
                deactivate_unfocused_windows: false,
                skip_cursor_only_updates_during_vrr: false,
                disable_10bit_output: false,
                nested_show_host_cursor: false,
                glass_shader: None,
                scene_shell: false,
            },
            workspaces: [
                Workspace {
                    name: WorkspaceName(
                        "workspace-1",
                    ),
                    open_on_output: [
                        "eDP-1",
                        "DP-1",
                    ],
                    layout: None,
                },
                Workspace {
                    name: WorkspaceName(
                        "workspace-2",
                    ),
                    open_on_output: [],
                    layout: None,
                },
                Workspace {
                    name: WorkspaceName(
                        "workspace-3",
                    ),
                    open_on_output: [],
                    layout: None,
                },
            ],
            recent_windows: RecentWindows {
                on: false,
                debounce_ms: 750,
                open_delay_ms: 150,
                highlight: MruHighlight {
                    active_color: Color {
                        r: 0.0,
                        g: 1.0,
                        b: 0.0,
                        a: 1.0,
                    },
                    urgent_color: Color {
                        r: 1.0,
                        g: 0.6,
                        b: 0.6,
                        a: 1.0,
                    },
                    padding: 15.0,
                    corner_radius: 0.0,
                },
                previews: MruPreviews {
                    max_height: 960.0,
                    max_scale: 0.5,
                },
                binds: [
                    Bind {
                        key: Key {
                            trigger: Keysym(
                                XK_Tab,
                            ),
                            modifiers: Modifiers(
                                ALT,
                            ),
                        },
                        action: Press(
                            MruAdvance {
                                direction: Forward,
                                scope: None,
                                filter: Some(
                                    All,
                                ),
                            },
                        ),
                        repeat: true,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: false,
                        allow_inhibiting: true,
                        hotkey_overlay_title: None,
                    },
                    Bind {
                        key: Key {
                            trigger: Keysym(
                                XK_grave,
                            ),
                            modifiers: Modifiers(
                                ALT,
                            ),
                        },
                        action: Press(
                            MruAdvance {
                                direction: Forward,
                                scope: None,
                                filter: Some(
                                    AppId,
                                ),
                            },
                        ),
                        repeat: true,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: false,
                        allow_inhibiting: true,
                        hotkey_overlay_title: None,
                    },
                    Bind {
                        key: Key {
                            trigger: Keysym(
                                XK_Tab,
                            ),
                            modifiers: Modifiers(
                                SUPER,
                            ),
                        },
                        action: Press(
                            MruAdvance {
                                direction: Forward,
                                scope: Some(
                                    Output,
                                ),
                                filter: Some(
                                    All,
                                ),
                            },
                        ),
                        repeat: true,
                        pass_through: None,
                        cooldown: None,
                        allow_when_locked: false,
                        allow_inhibiting: true,
                        hotkey_overlay_title: None,
                    },
                ],
            },
        }
        "#);
    }

    fn diff_lines(expected: &str, actual: &str) -> String {
        let mut output = String::new();
        let mut in_change = false;

        for change in diff::lines(expected, actual) {
            match change {
                diff::Result::Both(_, _) => {
                    in_change = false;
                }
                diff::Result::Left(line) => {
                    if !output.is_empty() && !in_change {
                        output.push('\n');
                    }
                    output.push('-');
                    output.push_str(line);
                    output.push('\n');
                    in_change = true;
                }
                diff::Result::Right(line) => {
                    if !output.is_empty() && !in_change {
                        output.push('\n');
                    }
                    output.push('+');
                    output.push_str(line);
                    output.push('\n');
                    in_change = true;
                }
            }
        }

        output
    }

    #[test]
    fn diff_empty_to_default() {
        // We try to write the config defaults in such a way that empty sections (and an empty
        // config) give the same outcome as the default config bundled with niri. This test
        // verifies the actual differences between the two.
        let mut default_config = Config::load_default();
        let empty_config = Config::parse_mem("").unwrap();

        // Some notable omissions: the default config has some window rules, and an empty config
        // will not have any binds. Clear them out so they don't spam the diff.
        default_config.window_rules.clear();
        default_config.binds.0.clear();

        assert_snapshot!(
            diff_lines(
                &format!("{empty_config:#?}"),
                &format!("{default_config:#?}")
            ),
            @r#"
        -            numlock: false,
        +            numlock: true,

        -            tap: false,
        +            tap: true,

        -            natural_scroll: false,
        +            natural_scroll: true,

        -    shell: None,
        +    shell: Some(
        +        Shell {
        +            command: [
        +                "bento-shell",
        +            ],
        +        },
        +    ),

        -    bind_files: BindFiles(0),
        +    bind_files: BindFiles(107),
        "#,
        );
    }
}
