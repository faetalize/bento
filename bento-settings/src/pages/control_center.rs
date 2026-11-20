//! bento's own shell: whether bento draws it, which shell it is, and the settings of the shell
//! and its modules that name this page.

use crate::model::{Category, Group, Setting};

pub fn category() -> Category {
    Category::settings("shell", "Shell")
        .icon("ph-sliders-horizontal-symbolic")
        .keywords(&[
            "quick settings",
            "control center",
            "shell",
            "bar",
            "panel",
            "buttons",
            "screenshot",
            "screen record",
            "recording",
            "command",
        ])
        .group(
            Group::untitled().setting(
                Setting::switch("Built-in Shell", "shell.built-in", |c| {
                    c.shell.as_ref().is_some_and(|shell| shell.built_in())
                })
                .keywords(&["bar", "panel", "experimental", "desktop shell"]),
            ),
        )
        .group(
            Group::custom(super::shell::build)
                .search_item(
                    "shell.choice",
                    "Shell",
                    &[
                        "unix", "windows", "taskbar", "custom", "file", "copy", "tsx",
                    ],
                )
                .search_item(
                    "shell.choice",
                    "Shell Settings",
                    &["pinned", "modules", "settings.kdl", "height"],
                )
                .search_item(
                    "shell.actions",
                    "Shell Shortcuts",
                    &["shell-action", "actions", "binds", "keybinds"],
                ),
        )
}
