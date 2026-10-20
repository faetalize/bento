//! niri's built-in layout actions, which go to the layout under their config names.
//!
//! Keeping niri's method names lets the rest of the compositor call the tiling space like it
//! called niri's scrolling space.

use niri_config::TilingOption;
use niri_ipc::{ColumnDisplay, SizeChange};
use serde_json::{json, Value};

use super::{size_change_json, ScrollDirection, TilingSpace};
use crate::layout::LayoutElement;

impl<W: LayoutElement> TilingSpace<W> {
    /// Runs an action that the layout provides, from `layout-action`.
    pub fn layout_action(&mut self, name: &str, args: &[TilingOption]) -> bool {
        let args = args
            .iter()
            .map(|arg| match arg {
                TilingOption::Bool(value) => json!(value),
                TilingOption::Number(value) => json!(value),
                TilingOption::String(value) => json!(value),
            })
            .collect();
        self.action(name, args, None)
    }

    fn simple(&mut self, name: &str) -> bool {
        self.action(name, Vec::new(), None)
    }

    fn on(&mut self, name: &str, window: Option<&W::Id>) -> bool {
        self.action(name, Vec::new(), window)
    }

    fn sized(&mut self, name: &str, window: Option<&W::Id>, change: SizeChange) -> bool {
        self.action(name, vec![size_change_json(change)], window)
    }

    pub fn focus_left(&mut self) -> bool {
        self.simple("focus-column-left")
    }

    pub fn focus_right(&mut self) -> bool {
        self.simple("focus-column-right")
    }

    pub fn focus_column_first(&mut self) {
        self.simple("focus-column-first");
    }

    pub fn focus_column_last(&mut self) {
        self.simple("focus-column-last");
    }

    pub fn focus_column(&mut self, index: usize) {
        self.action("focus-column", vec![json!(index)], None);
    }

    pub fn focus_window_in_column(&mut self, index: u8) {
        self.action("focus-window-in-column", vec![json!(index)], None);
    }

    pub fn focus_down(&mut self) -> bool {
        self.simple("focus-window-down")
    }

    pub fn focus_up(&mut self) -> bool {
        self.simple("focus-window-up")
    }

    pub fn focus_down_or_left(&mut self) {
        self.simple("focus-window-down-or-column-left");
    }

    pub fn focus_down_or_right(&mut self) {
        self.simple("focus-window-down-or-column-right");
    }

    pub fn focus_up_or_left(&mut self) {
        self.simple("focus-window-up-or-column-left");
    }

    pub fn focus_up_or_right(&mut self) {
        self.simple("focus-window-up-or-column-right");
    }

    pub fn focus_top(&mut self) {
        self.simple("focus-window-top");
    }

    pub fn focus_bottom(&mut self) {
        self.simple("focus-window-bottom");
    }

    pub fn move_left(&mut self) -> bool {
        self.simple("move-column-left")
    }

    pub fn move_right(&mut self) -> bool {
        self.simple("move-column-right")
    }

    pub fn move_column_to_first(&mut self) {
        self.simple("move-column-to-first");
    }

    pub fn move_column_to_last(&mut self) {
        self.simple("move-column-to-last");
    }

    pub fn move_column_to_index(&mut self, index: usize) {
        self.action("move-column-to-index", vec![json!(index)], None);
    }

    pub fn move_down(&mut self) -> bool {
        self.simple("move-window-down")
    }

    pub fn move_up(&mut self) -> bool {
        self.simple("move-window-up")
    }

    pub fn consume_or_expel_window_left(&mut self, window: Option<&W::Id>) {
        self.on("consume-or-expel-window-left", window);
    }

    pub fn consume_or_expel_window_right(&mut self, window: Option<&W::Id>) {
        self.on("consume-or-expel-window-right", window);
    }

    pub fn consume_into_column(&mut self) {
        self.simple("consume-window-into-column");
    }

    pub fn expel_from_column(&mut self) {
        self.simple("expel-window-from-column");
    }

    pub fn swap_window_in_direction(&mut self, direction: ScrollDirection) {
        match direction {
            ScrollDirection::Left => self.simple("swap-window-left"),
            ScrollDirection::Right => self.simple("swap-window-right"),
        };
    }

    pub fn toggle_column_tabbed_display(&mut self) {
        self.simple("toggle-column-tabbed-display");
    }

    pub fn set_column_display(&mut self, display: ColumnDisplay) {
        let display = match display {
            ColumnDisplay::Normal => "normal",
            ColumnDisplay::Tabbed => "tabbed",
        };
        self.action("set-column-display", vec![Value::from(display)], None);
    }

    pub fn center_column(&mut self) {
        self.simple("center-column");
    }

    pub fn center_window(&mut self, window: Option<&W::Id>) {
        self.on("center-window", window);
    }

    pub fn center_visible_columns(&mut self) {
        self.simple("center-visible-columns");
    }

    pub fn toggle_width(&mut self, forwards: bool) {
        if forwards {
            self.simple("switch-preset-column-width");
        } else {
            self.simple("switch-preset-column-width-back");
        }
    }

    pub fn toggle_full_width(&mut self) {
        self.simple("maximize-column");
    }

    pub fn set_window_width(&mut self, window: Option<&W::Id>, change: SizeChange) {
        if window.is_some() {
            self.sized("set-window-width", window, change);
        } else {
            self.sized("set-column-width", None, change);
        }
    }

    pub fn set_window_height(&mut self, window: Option<&W::Id>, change: SizeChange) {
        self.sized("set-window-height", window, change);
    }

    pub fn reset_window_height(&mut self, window: Option<&W::Id>) {
        self.on("reset-window-height", window);
    }

    pub fn toggle_window_width(&mut self, window: Option<&W::Id>, forwards: bool) {
        if forwards {
            self.on("switch-preset-window-width", window);
        } else {
            self.on("switch-preset-window-width-back", window);
        }
    }

    pub fn toggle_window_height(&mut self, window: Option<&W::Id>, forwards: bool) {
        if forwards {
            self.on("switch-preset-window-height", window);
        } else {
            self.on("switch-preset-window-height-back", window);
        }
    }

    pub fn expand_column_to_available_width(&mut self) {
        self.simple("expand-column-to-available-width");
    }
}
