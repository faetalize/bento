use std::path::PathBuf;

use knuffel::errors::DecodeError;

use crate::utils::{expand_home_path, MergeWith};

/// Which tiling layout bento runs, and its options.
///
/// ```kdl
/// tiling {
///     layout "bento" // or "niri", or a path to a .js file
///     options {
///         gravity true
///     }
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
    pub fn from_arg(arg: String) -> Self {
        match arg.as_str() {
            "true" => Self::Bool(true),
            "false" => Self::Bool(false),
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
                            Some(path) => TilingLayout::File(path),
                            None => continue,
                        }
                    } else {
                        TilingLayout::BuiltIn(name)
                    };
                }
                "options" => {
                    for option in child.children() {
                        let name = String::from(&**option.node_name);
                        let Some(value) = option.arguments.first() else {
                            ctx.emit_error(DecodeError::missing(option, "the option value"));
                            continue;
                        };
                        let value = knuffel::traits::DecodeScalar::decode(value, ctx)?;
                        tiling.options.push((name, value));
                    }
                }
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
    use crate::Config;

    #[test]
    fn parse_tiling() {
        let config = Config::parse_mem(
            r#"
            tiling {
                layout "niri"
                options {
                    gravity false
                    rows 12
                    ratio 0.5
                    name "x"
                }
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
}
