//! Actions that reach the layout engine.

use niri_config::TilingOption;
use niri_ipc::{LayoutEngineActionArgType, LayoutEngineOptionValue, SizeChange};
use serde_json::{json, Value};

use super::{engine, size_change_json, TilingSpace};
use crate::layout::LayoutElement;

impl<W: LayoutElement> TilingSpace<W> {
    /// Runs one of the engine's actions, from a bind in its file or `layout-action`. Only actions
    /// the engine declares reach it. `Size` arguments go to it as size changes, and arguments left
    /// out get their defaults.
    pub fn layout_action(&mut self, name: &str, args: &[TilingOption]) -> bool {
        let Some(declared) = engine::action_args(name) else {
            return false;
        };
        let is_size = |i: usize| {
            declared.get(i).and_then(|arg| arg.r#type) == Some(LayoutEngineActionArgType::Size)
        };

        let mut values: Vec<Value> = args
            .iter()
            .enumerate()
            .map(|(i, arg)| match engine::size_arg(arg) {
                Some(change) if is_size(i) => size_change_json(change),
                _ => match arg {
                    TilingOption::Bool(value) => json!(value),
                    TilingOption::Number(value) => json!(value),
                    TilingOption::String(value) => json!(value),
                },
            })
            .collect();
        for (i, arg) in declared.iter().enumerate().skip(args.len()) {
            let Some(default) = &arg.default else {
                break;
            };
            values.push(match default {
                LayoutEngineOptionValue::String(value) if is_size(i) => value
                    .parse::<SizeChange>()
                    .map_or_else(|_| json!(value), size_change_json),
                default => json!(default),
            });
        }
        self.action(name, values, None)
    }

    /// Applies the `default-window-height` window rule to a new window. The rule is niri's, so it
    /// reaches the engine as niri's `set-window-height`, which only `niri.js` has: rules that
    /// belong to a paradigm aren't declared by engines yet.
    pub fn apply_default_height(&mut self, window: &W::Id, change: SizeChange) {
        self.action(
            "set-window-height",
            vec![size_change_json(change)],
            Some(window),
        );
    }
}
