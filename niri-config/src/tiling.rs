use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use knuffel::errors::DecodeError;
use knuffel::Decode as _;

use crate::binds::{Binds, Key};
use crate::utils::{expand_home_path, MergeWith};
use crate::{Config, Includes};

/// Which tiling layout bento runs, and its options.
///
/// ```kdl
/// tiling {
///     layout "bento" // or "niri", or a path to a .js file
/// }
/// ```
///
/// The options come from the engine's own file, next to its binds (see [`engine_config_path`]):
///
/// ```kdl
/// options {
///     gravity true
/// }
///
/// binds {
///     Mod+S { split-h; }
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Tiling {
    pub layout: TilingLayout,
    /// Options for the layout, checked against what the layout declares when it loads.
    pub options: Vec<(String, TilingOption)>,
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
    match layout {
        TilingLayout::BuiltIn(name) => {
            let dir = config_path.parent().unwrap_or(Path::new(""));
            dir.join("engines").join(format!("{name}.kdl"))
        }
        TilingLayout::File(path) => path.with_extension("kdl"),
    }
}

/// Default files of the engines that ship with bento, with their binds and options.
const DEFAULT_ENGINE_CONFIGS: &[(&str, &str)] = &[
    ("bento", include_str!("../../resources/layouts/bento.kdl")),
    ("niri", include_str!("../../resources/layouts/niri.kdl")),
];

/// Writes the default file of the built-in engine in `layout` if it has none yet, the way bento
/// writes the default config on first start. From then on the file is the user's. Returns whether
/// it wrote one.
pub fn create_engine_config(config_path: &Path, layout: &TilingLayout) -> bool {
    let TilingLayout::BuiltIn(name) = layout else {
        return false;
    };
    let Some((_, text)) = DEFAULT_ENGINE_CONFIGS.iter().find(|(n, _)| n == name) else {
        return false;
    };
    let path = engine_config_path(config_path, layout);
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
            if seen.contains(&name) {
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
                    let mut options = Vec::new();
                    for option in node.children() {
                        let name = String::from(&**option.node_name);
                        let Some(value) = option.arguments.first() else {
                            ctx.emit_error(DecodeError::missing(option, "the option value"));
                            continue;
                        };
                        let value = knuffel::traits::DecodeScalar::decode(value, ctx)?;
                        options.push((name, value));
                    }
                    config.borrow_mut().tiling.options = options;
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
                    config.borrow_mut().binds.0.extend(part.0);
                }
                name => ctx.emit_error(DecodeError::unexpected(
                    node,
                    "node",
                    format!(
                        "unexpected node `{}`, an engine's file has `options` and `binds`",
                        name.escape_default()
                    ),
                )),
            }
        }

        Ok(Self)
    }
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
            }
            "#,
        )
        .unwrap();
        assert_eq!(
            config.tiling,
            Tiling {
                layout: TilingLayout::BuiltIn(String::from("niri")),
                options: vec![
                    (String::from("gravity"), TilingOption::Bool(false)),
                    (String::from("rows"), TilingOption::Number(12.)),
                    (String::from("ratio"), TilingOption::Number(0.5)),
                    (
                        String::from("name"),
                        TilingOption::String(String::from("x"))
                    ),
                ],
            }
        );

        let config = Config::parse_mem("").unwrap();
        assert_eq!(config.tiling, Tiling::default());
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
                    ]
                ),
                &Action::LayoutAction(String::from("focus-column-right"), vec![]),
            ]
        );

        // The main config only has bento's own actions, and niri's layout actions aren't bento's.
        let config = Config::parse_mem("binds { Mod+R { resize 2; } }");
        assert!(config.is_err());
        let config = Config::parse_mem("binds { Mod+L { focus-column-right; } }");
        assert!(config.is_err());
        let config = Config::parse_mem(r#"binds { Mod+R { layout-action "resize" 2; } }"#);
        assert!(config.is_err());
        // `layout-action` is for the command line; configs name engine actions directly.
        let config =
            Config::parse_mem_with_engine("", r#"binds { Mod+R { layout-action "resize"; } }"#);
        assert!(config.is_err());
    }

    #[test]
    fn engine_file_errors_keep_their_meaning() {
        // A typo in one of bento's actions' arguments is still an error, not an engine action.
        let config = Config::parse_mem_with_engine("", "binds { Mod+W { focus-workspace; } }");
        assert!(config.is_err());

        let config = Config::parse_mem_with_engine("", "layout { gaps 8; }");
        assert!(config.is_err());
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
    fn a_key_is_bound_in_one_file_only() {
        let config = Config::parse_mem_with_engine(
            "binds { Mod+R { spawn \"x\"; } }",
            "binds { Mod+R { resize; } }",
        );
        assert!(config.is_err());
    }
}
