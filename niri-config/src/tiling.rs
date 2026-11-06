use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use knuffel::errors::DecodeError;
use knuffel::Decode as _;
use niri_ipc::LayoutEngineGradient;

use crate::appearance::{Color, Gradient, GradientInterpolation, GradientRelativeTo};
use crate::binds::{Binds, Key};
use crate::utils::{expand_home_path, MergeWith};
use crate::window_rule::Match;
use crate::{Config, CurrentFile, Includes, OutputName};

/// Which tiling layout bento runs, and its options.
///
/// ```kdl
/// tiling {
///     layout "bento" // or "niri", or a path to a .js file
/// }
/// ```
///
/// The options and window rules come from the engine's own file, next to its binds (see
/// [`engine_config_path`]):
///
/// ```kdl
/// options {
///     gravity true
///     preset-column-widths "33%" "50%" "67%"
///     active-gradient from="#80c8ff" to="#c7ff7f" angle=45
/// }
///
/// // Options for the workspaces of an output, and for a named workspace.
/// output "DP-2" {
///     options {
///         gravity false
///     }
/// }
///
/// workspace "web" {
///     options {
///         preset-column-widths "50%"
///     }
/// }
///
/// window-rule {
///     match app-id="firefox"
///     default-column-width "67%"
/// }
///
/// binds {
///     Mod+S { split-h; }
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Tiling {
    pub layout: TilingLayout,
    /// Options for the layout, by name, with the values as written: an option is `name value...`,
    /// and several values make a list. The layout declares their types, so they're checked and
    /// converted when it loads.
    pub options: EngineOptions,
    /// Options that differ on the workspaces of an output, in the order they're written.
    pub output_options: Vec<ScopedOptions>,
    /// Options that differ on a named workspace, in the order they're written.
    pub workspace_options: Vec<ScopedOptions>,
    /// The layout's own window rules, in the order they're written.
    pub window_rules: Vec<EngineWindowRule>,
}

/// Options by name, with the values as written.
pub type EngineOptions = Vec<(String, Vec<TilingOption>)>;

/// The options that an `output` or `workspace` block of the engine's file sets for the output or
/// workspace it names.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopedOptions {
    pub name: String,
    pub options: EngineOptions,
}

impl Tiling {
    /// The lists of options that apply to a workspace, in the order they take effect: the
    /// top-level options, then those of the outputs `output` matches, then those of the
    /// workspaces called `workspace`. An option that a later list sets replaces the earlier ones.
    pub fn option_layers(
        &self,
        output: Option<&OutputName>,
        workspace: Option<&str>,
    ) -> Vec<&EngineOptions> {
        let outputs = self
            .output_options
            .iter()
            .filter(|scoped| output.is_some_and(|output| output.matches(&scoped.name)));
        let workspaces = self.workspace_options.iter().filter(|scoped| {
            workspace.is_some_and(|workspace| workspace.eq_ignore_ascii_case(&scoped.name))
        });
        let scoped = outputs.chain(workspaces).map(|scoped| &scoped.options);
        std::iter::once(&self.options).chain(scoped).collect()
    }
}

/// A `window-rule` of the engine's file: bento's matchers, with properties that the engine
/// declares.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineWindowRule {
    pub matches: Vec<Match>,
    pub excludes: Vec<Match>,
    pub properties: EngineRules,
}

/// Properties set by the layout's window rules for one window, by name and with the values as
/// written: `default-column-width "50%"` is `("default-column-width", ["50%"])`. A property
/// written with no value is how a size is left to the window. The layout declares the types, so
/// the values are converted once it's loaded.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct EngineRules(pub Vec<(String, Vec<TilingOption>)>);

impl EngineRules {
    pub fn get(&self, name: &str) -> Option<&[TilingOption]> {
        let property = self.0.iter().find(|(n, _)| n == name);
        property.map(|(_, values)| values.as_slice())
    }
}

impl MergeWith<EngineRules> for EngineRules {
    /// Properties the part sets replace the ones already set, the way bento's own rules resolve.
    fn merge_with(&mut self, part: &EngineRules) {
        for (name, values) in &part.0 {
            match self.0.iter_mut().find(|(n, _)| n == name) {
                Some((_, existing)) => existing.clone_from(values),
                None => self.0.push((name.clone(), values.clone())),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TilingLayout {
    /// A layout that ships with bento, by name.
    BuiltIn(String),
    /// A layout file.
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TilingOption {
    Bool(bool),
    Number(f64),
    String(String),
    /// A gradient, which is written with properties instead of arguments.
    Gradient(LayoutEngineGradient),
}

/// Checks a gradient that a layout engine writes, and turns it into the gradient that bento draws
/// for borders and focus rings.
pub fn resolve_gradient(gradient: &LayoutEngineGradient) -> Result<Gradient, String> {
    let color =
        |name: &str, text: &str| Color::from_hex(text).map_err(|err| format!("`{name}`: {err}"));
    let from = color("from", &gradient.from)?;
    let to = color("to", &gradient.to)?;

    let angle = match gradient.angle {
        // The same as niri's: from top to bottom.
        None => 180,
        Some(angle) if angle.is_finite() && angle.round().abs() <= f64::from(i16::MAX) => {
            angle.round() as i16
        }
        Some(_) => return Err(String::from("`angle` must be a number of degrees")),
    };

    let relative_to = match gradient.relative_to.as_deref() {
        None | Some("node") => GradientRelativeTo::Window,
        Some("view") => GradientRelativeTo::WorkspaceView,
        Some(other) => {
            return Err(format!(
                "`relative-to` must be \"node\" or \"view\", not {other:?}"
            ))
        }
    };

    let in_ = match &gradient.color_space {
        None => GradientInterpolation::default(),
        Some(space) => space.parse().map_err(|err| format!("`in`: {err}"))?,
    };

    Ok(Gradient {
        from,
        to,
        angle,
        relative_to,
        in_,
    })
}

impl TilingOption {
    /// Converts a command line argument, which is always a string, to a value.
    ///
    /// An argument starting with `+` stays a string: it's a change like `+10` for a `Size`
    /// argument, and turning it into the number 10 would make it a size to set instead.
    pub fn from_arg(arg: String) -> Self {
        match arg.as_str() {
            "true" => Self::Bool(true),
            "false" => Self::Bool(false),
            _ if arg.starts_with('+') => Self::String(arg),
            _ => match arg.parse() {
                Ok(number) => Self::Number(number),
                Err(_) => Self::String(arg),
            },
        }
    }
}

impl Default for Tiling {
    fn default() -> Self {
        Self {
            layout: TilingLayout::BuiltIn(String::from("bento")),
            options: Vec::new(),
            output_options: Vec::new(),
            workspace_options: Vec::new(),
            window_rules: Vec::new(),
        }
    }
}

impl MergeWith<Tiling> for Tiling {
    fn merge_with(&mut self, part: &Tiling) {
        *self = part.clone();
    }
}

impl<S: knuffel::traits::ErrorSpan> knuffel::Decode<S> for Tiling {
    fn decode_node(
        node: &knuffel::ast::SpannedNode<S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, DecodeError<S>> {
        let mut tiling = Tiling::default();

        for child in node.children() {
            match &**child.node_name {
                "layout" => {
                    let Some(value) = child.arguments.first() else {
                        ctx.emit_error(DecodeError::missing(child, "the layout name or path"));
                        continue;
                    };
                    let name: String = knuffel::traits::DecodeScalar::decode(value, ctx)?;
                    tiling.layout = if name.contains('/') || name.ends_with(".js") {
                        match expand_home_path(PathBuf::from(name), child, ctx) {
                            Ok(path) => {
                                // Watch the layout file like the config, to reload it on changes.
                                if let Some(includes) = ctx.get::<Rc<RefCell<Includes>>>() {
                                    includes.borrow_mut().0.push(path.clone());
                                }
                                TilingLayout::File(path)
                            }
                            Err(err) => {
                                ctx.emit_error(err);
                                continue;
                            }
                        }
                    } else {
                        TilingLayout::BuiltIn(name)
                    };
                }
                "options" => ctx.emit_error(DecodeError::unexpected(
                    child,
                    "node",
                    "engine options go in the engine's own file, like `engines/bento.kdl` next to \
                     this config",
                )),
                name => ctx.emit_error(DecodeError::unexpected(
                    child,
                    "node",
                    format!("unexpected node `{}`", name.escape_default()),
                )),
            }
        }

        Ok(tiling)
    }
}

/// The file with the engine's options and binds: `engines/<name>.kdl` next to the main config for
/// a built-in engine, or the engine file with a `.kdl` extension.
pub fn engine_config_path(config_path: &Path, layout: &TilingLayout) -> PathBuf {
    engine_config_path_in(config_path.parent().unwrap_or(Path::new("")), layout)
}

/// The engine's file, with built-in engines' files in `engines/` under `dir`.
pub fn engine_config_path_in(dir: &Path, layout: &TilingLayout) -> PathBuf {
    match layout {
        TilingLayout::BuiltIn(name) => dir.join("engines").join(format!("{name}.kdl")),
        TilingLayout::File(path) => path.with_extension("kdl"),
    }
}

/// The default file of a built-in engine, with its binds and options.
pub fn default_engine_config(name: &str) -> Option<&'static str> {
    let config = DEFAULT_ENGINE_CONFIGS.iter().find(|(n, _)| *n == name);
    config.map(|(_, text)| *text)
}

/// Default files of the engines that ship with bento, with their binds and options.
const DEFAULT_ENGINE_CONFIGS: &[(&str, &str)] = &[
    ("bento", include_str!("../../resources/layouts/bento.kdl")),
    ("niri", include_str!("../../resources/layouts/niri.kdl")),
];

/// Writes the default file of the built-in engine in `layout` if it has none yet, the way bento
/// writes the default config on first start. From then on the file is the user's. Returns whether
/// it wrote one.
pub fn create_engine_config(dir: &Path, layout: &TilingLayout) -> bool {
    let TilingLayout::BuiltIn(name) = layout else {
        return false;
    };
    let Some((_, text)) = DEFAULT_ENGINE_CONFIGS.iter().find(|(n, _)| n == name) else {
        return false;
    };
    let path = engine_config_path_in(dir, layout);
    if path.exists() {
        return false;
    }
    let result = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            // `create_new` so a file made meanwhile is never overwritten.
            let mut file = std::fs::File::options()
                .write(true)
                .create_new(true)
                .open(&path)?;
            std::io::Write::write_all(&mut file, text.as_bytes())
        });
    match result {
        Ok(()) => {
            info!("wrote the default {name} engine config to {path:?}");
            true
        }
        Err(err) => {
            warn!("error writing the default {name} engine config to {path:?}: {err}");
            false
        }
    }
}

/// Marks the engine's file in the knuffel context. Its binds can name the engine's actions by bare
/// name, so an action bento doesn't have becomes a `layout-action` there.
pub(crate) struct EngineFile;

/// The engine's file, parsed into the config from the context.
pub(crate) struct EnginePart;

impl<S> knuffel::DecodeChildren<S> for EnginePart
where
    S: knuffel::traits::ErrorSpan,
{
    fn decode_children(
        nodes: &[knuffel::ast::SpannedNode<S>],
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, DecodeError<S>> {
        let config = ctx.get::<Rc<RefCell<Config>>>().unwrap().clone();
        let mut seen = Vec::new();

        for node in nodes {
            let name = &**node.node_name;
            if !matches!(name, "window-rule" | "output" | "workspace") && seen.contains(&name) {
                ctx.emit_error(DecodeError::unexpected(
                    &node.node_name,
                    "node",
                    format!("duplicate node `{name}`, single node expected"),
                ));
                continue;
            }
            seen.push(name);

            match name {
                "options" => {
                    config.borrow_mut().tiling.options = decode_options(node, ctx)?;
                }
                "output" | "workspace" => {
                    let scoped = decode_scoped(node, ctx)?;
                    let mut config = config.borrow_mut();
                    let tiling = &mut config.tiling;
                    let list = if name == "output" {
                        &mut tiling.output_options
                    } else {
                        &mut tiling.workspace_options
                    };
                    if list
                        .iter()
                        .any(|o| o.name.eq_ignore_ascii_case(&scoped.name))
                    {
                        ctx.emit_error(DecodeError::unexpected(
                            &node.node_name,
                            "node",
                            format!("duplicate {name} {:?}, single node expected", scoped.name),
                        ));
                    } else {
                        list.push(scoped);
                    }
                }
                "window-rule" => {
                    let rule = decode_window_rule(node, ctx)?;
                    config.borrow_mut().tiling.window_rules.push(rule);
                }
                "binds" => {
                    // A key bound in both files would leave it unclear which one wins, so the
                    // files share one set of keys.
                    let main: Vec<Key> = config.borrow().binds.0.iter().map(|b| b.key).collect();
                    for child in node.children() {
                        let Ok(key) = child.node_name.parse::<Key>() else {
                            continue;
                        };
                        if main.iter().any(|main| main.conflicts_with(&key)) {
                            ctx.emit_error(DecodeError::unexpected(
                                &child.node_name,
                                "keybind",
                                "this key is already bound in the main config",
                            ));
                        }
                    }

                    let part = Binds::decode_node(node, ctx)?;
                    let file = ctx.get::<CurrentFile>().unwrap().0.clone();
                    let mut config = config.borrow_mut();
                    let files = part.0.iter().map(|bind| (bind.key, file.clone()));
                    config.bind_files.0.extend(files);
                    config.binds.0.extend(part.0);
                }
                name => ctx.emit_error(DecodeError::unexpected(
                    node,
                    "node",
                    format!(
                        "unexpected node `{}`, an engine's file has `options`, `output`, \
                         `workspace`, `window-rule` and `binds`",
                        name.escape_default()
                    ),
                )),
            }
        }

        Ok(Self)
    }
}

/// The options in an `options` node.
fn decode_options<S: knuffel::traits::ErrorSpan>(
    node: &knuffel::ast::SpannedNode<S>,
    ctx: &mut knuffel::decode::Context<S>,
) -> Result<EngineOptions, DecodeError<S>> {
    let mut options = Vec::new();
    for option in node.children() {
        let name = String::from(&**option.node_name);
        options.push((name, decode_values(option, ctx)?));
    }
    Ok(options)
}

/// An `output "DP-2" { options { … } }` or `workspace "web" { options { … } }` node.
fn decode_scoped<S: knuffel::traits::ErrorSpan>(
    node: &knuffel::ast::SpannedNode<S>,
    ctx: &mut knuffel::decode::Context<S>,
) -> Result<ScopedOptions, DecodeError<S>> {
    let kind = &**node.node_name;

    let mut arguments = node.arguments.iter();
    let name: String = match arguments.next() {
        Some(value) => knuffel::traits::DecodeScalar::decode(value, ctx)?,
        None => {
            ctx.emit_error(DecodeError::missing(node, format!("the {kind}'s name")));
            String::new()
        }
    };
    for value in arguments {
        ctx.emit_error(DecodeError::unexpected(
            &value.literal,
            "argument",
            "unexpected argument",
        ));
    }
    for name in node.properties.keys() {
        ctx.emit_error(DecodeError::unexpected(
            name,
            "property",
            format!("unexpected property `{}`", name.escape_default()),
        ));
    }

    let mut options = Vec::new();
    let mut seen = false;
    for child in node.children() {
        if &**child.node_name != "options" {
            ctx.emit_error(DecodeError::unexpected(
                child,
                "node",
                format!(
                    "unexpected node `{}`, an {kind} block has `options`",
                    child.node_name.escape_default()
                ),
            ));
        } else if seen {
            ctx.emit_error(DecodeError::unexpected(
                &child.node_name,
                "node",
                "duplicate node `options`, single node expected",
            ));
        } else {
            seen = true;
            options = decode_options(child, ctx)?;
        }
    }
    Ok(ScopedOptions { name, options })
}

/// A gradient option or window rule property, which is written with properties: `from` and `to`
/// colours, and optionally `angle`, `relative-to` and `in`.
fn decode_gradient<S: knuffel::traits::ErrorSpan>(
    node: &knuffel::ast::SpannedNode<S>,
    ctx: &mut knuffel::decode::Context<S>,
) -> Result<Vec<TilingOption>, DecodeError<S>> {
    for value in &node.arguments {
        ctx.emit_error(DecodeError::unexpected(
            &value.literal,
            "argument",
            "a gradient is written with properties, like `from=\"#000\" to=\"#fff\"`",
        ));
    }
    for child in node.children() {
        ctx.emit_error(DecodeError::unexpected(
            child,
            "node",
            format!(
                "`{}` takes its values as properties, not children",
                node.node_name.escape_default()
            ),
        ));
    }

    let mut gradient = LayoutEngineGradient {
        from: String::new(),
        to: String::new(),
        angle: None,
        relative_to: None,
        color_space: None,
    };
    let (mut has_from, mut has_to) = (false, false);
    for (name, value) in &node.properties {
        let option: TilingOption = knuffel::traits::DecodeScalar::decode(value, ctx)?;
        match (&***name, option) {
            ("from", TilingOption::String(text)) => (gradient.from, has_from) = (text, true),
            ("to", TilingOption::String(text)) => (gradient.to, has_to) = (text, true),
            ("angle", TilingOption::Number(degrees)) => gradient.angle = Some(degrees),
            ("relative-to", TilingOption::String(text)) => gradient.relative_to = Some(text),
            ("in", TilingOption::String(text)) => gradient.color_space = Some(text),
            ("from" | "to" | "relative-to" | "in", _) => {
                ctx.emit_error(DecodeError::conversion(&value.literal, "expected a string"));
            }
            ("angle", _) => {
                ctx.emit_error(DecodeError::conversion(&value.literal, "expected a number"));
            }
            _ => ctx.emit_error(DecodeError::unexpected(
                name,
                "property",
                format!(
                    "unexpected property `{}`, a gradient has `from`, `to`, `angle`, \
                     `relative-to` and `in`",
                    name.escape_default()
                ),
            )),
        }
    }

    if !(has_from && has_to) {
        ctx.emit_error(DecodeError::missing(
            node,
            "a gradient needs `from` and `to` colours",
        ));
        return Ok(Vec::new());
    }
    if let Err(err) = resolve_gradient(&gradient) {
        ctx.emit_error(DecodeError::conversion(&node.node_name, err));
        return Ok(Vec::new());
    }
    Ok(vec![TilingOption::Gradient(gradient)])
}

/// The values of an option or window rule property: the node's arguments, or for a gradient its
/// properties.
fn decode_values<S: knuffel::traits::ErrorSpan>(
    node: &knuffel::ast::SpannedNode<S>,
    ctx: &mut knuffel::decode::Context<S>,
) -> Result<Vec<TilingOption>, DecodeError<S>> {
    if !node.properties.is_empty() {
        return decode_gradient(node, ctx);
    }
    for child in node.children() {
        ctx.emit_error(DecodeError::unexpected(
            child,
            "node",
            format!(
                "`{}` takes its values as arguments, not children",
                node.node_name.escape_default()
            ),
        ));
    }

    node.arguments
        .iter()
        .map(|value| knuffel::traits::DecodeScalar::decode(value, ctx))
        .collect()
}

/// A `window-rule` of the engine's file: `match` and `exclude` like bento's own rules, and every
/// other child is a property that the engine declares.
fn decode_window_rule<S: knuffel::traits::ErrorSpan>(
    node: &knuffel::ast::SpannedNode<S>,
    ctx: &mut knuffel::decode::Context<S>,
) -> Result<EngineWindowRule, DecodeError<S>> {
    for value in &node.arguments {
        ctx.emit_error(DecodeError::unexpected(
            &value.literal,
            "argument",
            "unexpected argument",
        ));
    }
    for name in node.properties.keys() {
        ctx.emit_error(DecodeError::unexpected(
            name,
            "property",
            format!("unexpected property `{}`", name.escape_default()),
        ));
    }

    let mut rule = EngineWindowRule {
        matches: Vec::new(),
        excludes: Vec::new(),
        properties: EngineRules::default(),
    };
    for child in node.children() {
        match &**child.node_name {
            "match" => rule.matches.push(Match::decode_node(child, ctx)?),
            "exclude" => rule.excludes.push(Match::decode_node(child, ctx)?),
            name => {
                let values = decode_values(child, ctx)?;
                rule.properties.0.push((String::from(name), values));
            }
        }
    }
    Ok(rule)
}

impl<S: knuffel::traits::ErrorSpan> knuffel::DecodeScalar<S> for TilingOption {
    fn type_check(
        type_name: &Option<knuffel::span::Spanned<knuffel::ast::TypeName, S>>,
        ctx: &mut knuffel::decode::Context<S>,
    ) {
        if let Some(type_name) = &type_name {
            ctx.emit_error(DecodeError::unexpected(
                type_name,
                "type name",
                "no type name expected for this node",
            ));
        }
    }

    fn raw_decode(
        val: &knuffel::span::Spanned<knuffel::ast::Literal, S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<TilingOption, DecodeError<S>> {
        use knuffel::ast::Literal;

        Ok(match &**val {
            Literal::Bool(value) => TilingOption::Bool(*value),
            Literal::String(value) => TilingOption::String(String::from(&**value)),
            Literal::Int(value) => match i64::try_from(value) {
                Ok(value) => TilingOption::Number(value as f64),
                Err(e) => {
                    ctx.emit_error(DecodeError::conversion(val, e));
                    TilingOption::Number(0.)
                }
            },
            Literal::Decimal(value) => match f64::try_from(value) {
                Ok(value) => TilingOption::Number(value),
                Err(e) => {
                    ctx.emit_error(DecodeError::conversion(val, e));
                    TilingOption::Number(0.)
                }
            },
            Literal::Null => {
                ctx.emit_error(DecodeError::unexpected(
                    val,
                    "value",
                    "expected a boolean, number or string",
                ));
                TilingOption::Bool(false)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Config};

    #[test]
    fn arg_with_plus_stays_a_string() {
        assert_eq!(
            TilingOption::from_arg(String::from("+10")),
            TilingOption::String(String::from("+10"))
        );
        assert_eq!(
            TilingOption::from_arg(String::from("-10")),
            TilingOption::Number(-10.)
        );
    }

    #[test]
    fn parse_tiling() {
        let config = Config::parse_mem_with_engine(
            r#"
            tiling {
                layout "niri"
            }
            "#,
            r#"
            options {
                gravity false
                rows 12
                ratio 0.5
                name "x"
                preset-column-widths "33%" "50%" "800"
                default-column-width
            }
            "#,
        )
        .unwrap();
        let string = |value: &str| TilingOption::String(String::from(value));
        assert_eq!(
            config.tiling,
            Tiling {
                layout: TilingLayout::BuiltIn(String::from("niri")),
                options: vec![
                    (String::from("gravity"), vec![TilingOption::Bool(false)]),
                    (String::from("rows"), vec![TilingOption::Number(12.)]),
                    (String::from("ratio"), vec![TilingOption::Number(0.5)]),
                    (String::from("name"), vec![string("x")]),
                    (
                        String::from("preset-column-widths"),
                        vec![string("33%"), string("50%"), string("800")]
                    ),
                    (String::from("default-column-width"), vec![]),
                ],
                output_options: Vec::new(),
                workspace_options: Vec::new(),
                window_rules: Vec::new(),
            }
        );

        let config = Config::parse_mem("").unwrap();
        assert_eq!(config.tiling, Tiling::default());
    }

    #[test]
    fn parse_engine_window_rules() {
        let config = Config::parse_mem_with_engine(
            "",
            r#"
            window-rule {
                match app-id="^firefox$" is-floating=false
                exclude title="Picture"
                default-column-width "67%"
                default-column-display "tabbed"
            }

            window-rule {
                default-window-height
                open-maximized true
            }
            "#,
        )
        .unwrap();
        let rules = &config.tiling.window_rules;
        assert_eq!(rules.len(), 2);

        assert_eq!(rules[0].matches.len(), 1);
        assert_eq!(rules[0].matches[0].is_floating, Some(false));
        assert_eq!(rules[0].excludes.len(), 1);
        let width = [TilingOption::String(String::from("67%"))];
        assert_eq!(
            rules[0].properties.get("default-column-width"),
            Some(&width[..])
        );
        assert_eq!(rules[0].properties.0.len(), 2);

        assert!(rules[1].matches.is_empty());
        assert_eq!(
            rules[1].properties.get("default-window-height"),
            Some(&[][..])
        );
        let open = [TilingOption::Bool(true)];
        assert_eq!(rules[1].properties.get("open-maximized"), Some(&open[..]));
        assert_eq!(rules[1].properties.get("nope"), None);
    }

    fn gradient(from: &str, to: &str) -> LayoutEngineGradient {
        LayoutEngineGradient {
            from: String::from(from),
            to: String::from(to),
            angle: None,
            relative_to: None,
            color_space: None,
        }
    }

    #[test]
    fn gradients_are_written_with_properties() {
        let config = Config::parse_mem_with_engine(
            "",
            r##"
            options {
                active-gradient from="#80c8ff" to="#c7ff7f" angle=45 relative-to="view" in="oklch shorter hue"
                inactive-gradient from="#000" to="#ffffff80"
                urgent-gradient
            }

            window-rule {
                tab-indicator-active-gradient from="#f00" to="#00f"
            }
            "##,
        )
        .unwrap();

        let options = &config.tiling.options;
        assert_eq!(
            options[0].1,
            [TilingOption::Gradient(LayoutEngineGradient {
                angle: Some(45.),
                relative_to: Some(String::from("view")),
                color_space: Some(String::from("oklch shorter hue")),
                ..gradient("#80c8ff", "#c7ff7f")
            })]
        );
        assert_eq!(
            options[1].1,
            [TilingOption::Gradient(gradient("#000", "#ffffff80"))]
        );
        // A gradient with no value is left out.
        assert!(options[2].1.is_empty());

        let rule = &config.tiling.window_rules[0].properties;
        assert_eq!(
            rule.get("tab-indicator-active-gradient"),
            Some(&[TilingOption::Gradient(gradient("#f00", "#00f"))][..])
        );
    }

    #[test]
    fn gradients_with_bad_values_are_errors() {
        for (gradient, problem) in [
            (r##"from="#000""##, "needs `from` and `to`"),
            (r##"to="#000""##, "needs `from` and `to`"),
            (r##"from="red" to="#fff""##, "invalid colour"),
            (r##"from="#000" to="#fff" angle="up""##, "expected a number"),
            (
                r##"from="#000" to="#fff" angle=99999"##,
                "must be a number of degrees",
            ),
            (
                r##"from="#000" to="#fff" relative-to="window""##,
                "relative-to",
            ),
            (r##"from="#000" to="#fff" in="hsl""##, "invalid color space"),
            (
                r##"from="#000" to="#fff" in="srgb shorter hue""##,
                "hue interpolation",
            ),
            (r##"from="#000" to="#fff" width=2"##, "unexpected property"),
            (r##"from=1 to="#fff""##, "expected a string"),
            (r##""x" from="#000" to="#fff""##, "written with properties"),
        ] {
            let text = format!("options {{ active-gradient {gradient}; }}");
            let result = Config::parse_mem_with_engine("", &text);
            let text = format!("{:?}", miette::Report::new(result.unwrap_err()));
            assert!(text.contains(problem), "{gradient}: {text}");
        }
    }

    #[test]
    fn a_gradient_resolves_to_the_gradient_bento_draws() {
        let resolved = resolve_gradient(&gradient("#000", "#fff")).unwrap();
        // Like niri's: from top to bottom, over the node, in sRGB.
        assert_eq!(resolved.angle, 180);
        assert_eq!(resolved.relative_to, GradientRelativeTo::Window);
        assert_eq!(resolved.in_, GradientInterpolation::default());
        assert_eq!(resolved.from, Color::from_rgba8_unpremul(0, 0, 0, 255));

        let resolved = resolve_gradient(&LayoutEngineGradient {
            angle: Some(44.6),
            relative_to: Some(String::from("view")),
            color_space: Some(String::from("oklab")),
            ..gradient("#000", "#fff")
        })
        .unwrap();
        assert_eq!(resolved.angle, 45);
        assert_eq!(resolved.relative_to, GradientRelativeTo::WorkspaceView);
        assert_eq!(
            resolved.in_,
            "oklab".parse::<GradientInterpolation>().unwrap()
        );

        let infinite = LayoutEngineGradient {
            angle: Some(f64::INFINITY),
            ..gradient("#000", "#fff")
        };
        assert!(resolve_gradient(&infinite).is_err());
    }

    #[test]
    fn options_differ_per_output_and_workspace() {
        let config = Config::parse_mem_with_engine(
            "",
            r#"
            options {
                gravity true
                rows 4
            }

            output "DP-2" {
                options {
                    rows 6
                }
            }

            output "HDMI-A-1" {
                options {
                    gravity false
                }
            }

            workspace "web" {
                options {
                    rows 8
                }
            }
            "#,
        )
        .unwrap();
        let tiling = &config.tiling;
        assert_eq!(tiling.output_options.len(), 2);
        assert_eq!(tiling.workspace_options.len(), 1);

        let output = |connector: &str| OutputName {
            connector: String::from(connector),
            make: None,
            model: None,
            serial: None,
        };
        let rows = |layers: Vec<&EngineOptions>| -> Vec<f64> {
            let rows = layers
                .into_iter()
                .flatten()
                .filter(|(name, _)| name == "rows");
            rows.map(|(_, values)| match values[..] {
                [TilingOption::Number(rows)] => rows,
                _ => panic!("{values:?}"),
            })
            .collect()
        };

        // The top-level options come first, so the ones after them win.
        assert_eq!(rows(tiling.option_layers(None, None)), [4.]);
        let dp2 = output("dp-2");
        assert_eq!(rows(tiling.option_layers(Some(&dp2), None)), [4., 6.]);
        assert_eq!(
            rows(tiling.option_layers(Some(&dp2), Some("WEB"))),
            [4., 6., 8.]
        );
        assert_eq!(rows(tiling.option_layers(None, Some("web"))), [4., 8.]);
        let other = output("DP-3");
        assert_eq!(rows(tiling.option_layers(Some(&other), Some("mail"))), [4.]);
    }

    #[test]
    fn output_and_workspace_blocks_are_checked() {
        for (text, problem) in [
            ("output { options { rows 1; }; }", "the output's name"),
            (r#"output "a" "b" { }"#, "unexpected argument"),
            (r#"output "a" x=1 { }"#, "unexpected property"),
            (r#"workspace "a" { rows 1; }"#, "has `options`"),
            (
                r#"workspace "a" { options { }; options { }; }"#,
                "duplicate node `options`",
            ),
            (r#"output "a" { }; output "A" { }"#, "duplicate output"),
            (
                r#"workspace "a" { }; workspace "a" { }"#,
                "duplicate workspace",
            ),
        ] {
            let result = Config::parse_mem_with_engine("", text);
            let text = format!("{:?}", miette::Report::new(result.unwrap_err()));
            assert!(text.contains(problem), "{text}");
        }
    }

    #[test]
    fn engine_window_rules_resolve_later_rules_over_earlier() {
        let property = |name: &str, value: &str| {
            let value = TilingOption::String(String::from(value));
            EngineRules(vec![(String::from(name), vec![value])])
        };

        let mut resolved = EngineRules::default();
        resolved.merge_with(&property("default-column-width", "50%"));
        resolved.merge_with(&property("default-column-display", "tabbed"));
        resolved.merge_with(&property("default-column-width", "800"));

        let width = [TilingOption::String(String::from("800"))];
        assert_eq!(resolved.get("default-column-width"), Some(&width[..]));
        assert_eq!(resolved.0.len(), 2);
    }

    #[test]
    fn engine_window_rules_are_not_the_main_configs() {
        // The main config's rules only know bento's own properties and matchers.
        let rule = "window-rule { default-column-width; }";
        assert!(Config::parse_mem(rule).is_err());
        let rule = r#"window-rule { match is-active-in-column=true; }"#;
        assert!(Config::parse_mem(rule).is_err());
    }

    #[test]
    fn engine_options_stay_out_of_the_main_config() {
        let config = Config::parse_mem(
            r#"
            tiling {
                options {
                    gravity true
                }
            }
            "#,
        );
        assert!(config.is_err());
    }

    #[test]
    fn engine_file_binds_actions_by_bare_name() {
        let config = Config::parse_mem_with_engine(
            r#"
            binds {
                Mod+T { spawn "alacritty"; }
            }
            "#,
            r#"
            binds {
                Mod+R { resize 2 "right"; }
                Mod+L { focus-column-right; }
            }
            "#,
        )
        .unwrap();
        let actions: Vec<_> = config
            .binds
            .0
            .iter()
            .filter_map(|bind| bind.press_action())
            .collect();
        assert_eq!(
            actions,
            [
                &Action::Spawn(vec![String::from("alacritty")]),
                &Action::LayoutAction(
                    String::from("resize"),
                    vec![
                        TilingOption::Number(2.),
                        TilingOption::String(String::from("right"))
                    ],
                    None
                ),
                &Action::LayoutAction(String::from("focus-column-right"), vec![], None),
            ]
        );

        // The main config only has bento's own actions, and niri's layout actions aren't bento's.
        let unknown = "or one of";
        assert!(error(Config::parse_mem(&binds("Mod+R { resize 2; }"))).contains(unknown));
        let config = Config::parse_mem(&binds("Mod+L { focus-column-right; }"));
        assert!(error(config).contains(unknown));
        let config = Config::parse_mem(&binds(r#"Mod+R { layout-action "resize" 2; }"#));
        assert!(error(config).contains("engine actions are bound by name in the engine's own file"));
        // `layout-action` is for the command line; configs name engine actions directly.
        let engine = binds(r#"Mod+R { layout-action "resize"; }"#);
        let config = Config::parse_mem_with_engine("", &engine);
        assert!(error(config).contains("write the engine's action by its name"));
    }

    fn binds(body: &str) -> String {
        format!("binds {{\n    {body}\n}}\n")
    }

    /// The parse error as bento shows it, which must not be a syntax error.
    fn error(config: Result<Config, crate::ConfigIncludeError>) -> String {
        let text = format!("{:?}", miette::Report::new(config.unwrap_err()));
        assert!(!text.contains("found `"), "{text}");
        text
    }

    #[test]
    fn engine_file_errors_keep_their_meaning() {
        // A typo in one of bento's actions' arguments is still an error, not an engine action.
        let config = Config::parse_mem_with_engine("", &binds("Mod+W { focus-workspace; }"));
        assert!(error(config).contains("argument"));

        let config = Config::parse_mem_with_engine("", "layout { gaps 8; }");
        assert!(error(config).contains("unexpected node `layout`"));

        // An option's values are arguments (or the properties of a gradient), not children.
        let config = Config::parse_mem_with_engine("", "options { rows x=1; }");
        assert!(error(config).contains("unexpected property"));
        let config = Config::parse_mem_with_engine("", "options { rows { x; }; }");
        assert!(error(config).contains("takes its values as arguments"));
    }

    #[test]
    fn default_engine_configs_fit_the_default_config() {
        let main = include_str!("../../resources/default-config.kdl");
        for (name, engine) in DEFAULT_ENGINE_CONFIGS {
            let main = main.replace(r#"layout "bento""#, &format!("layout {name:?}"));
            let config = Config::parse_mem_with_engine(&main, engine)
                .unwrap_or_else(|err| panic!("{name}: {:?}", miette::Report::new(err)));
            let engine_binds = config
                .binds
                .0
                .iter()
                .filter(|bind| matches!(bind.press_action(), Some(Action::LayoutAction(..))));
            assert!(engine_binds.count() > 10, "{name}");
        }
    }

    #[test]
    fn binds_remember_their_file() {
        let main = binds(r#"Mod+T { spawn "x"; }"#);
        let config = Config::parse_mem_with_engine(&main, &binds("Mod+R { resize; }")).unwrap();
        let files: Vec<_> = config
            .bind_files
            .0
            .iter()
            .map(|(_, path)| path.to_str().unwrap())
            .collect();
        assert_eq!(files, ["config.kdl", "engines/bento.kdl"]);
    }

    #[test]
    fn a_key_is_bound_in_one_file_only() {
        let main = binds(r#"Mod+R { spawn "x"; }"#);
        let config = Config::parse_mem_with_engine(&main, &binds("Mod+R { resize; }"));
        assert!(error(config).contains("this key is already bound in the main config"));
    }

    #[test]
    fn the_engine_file_is_the_users_even_with_the_system_config() {
        let root = std::env::temp_dir().join(format!("bento-system-config-{}", std::process::id()));
        let user = root.join("user/bento");
        let system = root.join("etc/bento");
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(
            system.join("config.kdl"),
            "tiling {\n    layout \"niri\"\n}\n",
        )
        .unwrap();

        let path = crate::ConfigPath::Regular {
            user_path: user.join("config.kdl"),
            system_path: system.join("config.kdl"),
        };
        path.load_and_create_engine_config().config.unwrap();

        assert!(user.join("engines/niri.kdl").exists());
        assert!(!system.join("engines").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_relative_engine_path_is_relative_to_the_config() {
        let text = "tiling {\n    layout \"mine.js\"\n}\n";
        let config =
            Config::parse_with_engine(Path::new("/home/me/.config/bento/config.kdl"), text, |_| {
                Ok(String::new())
            })
            .config
            .unwrap();
        assert_eq!(
            config.tiling.layout,
            TilingLayout::File(PathBuf::from("/home/me/.config/bento/mine.js"))
        );
    }
}
