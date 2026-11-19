//! bento's own shell: whether bento draws it, which shell it is, its settings, and what the
//! buttons in its control center run.

use niri_config::{Config, ShellCommand};

use crate::model::{Category, Group, Setting};

fn screenshot(config: &Config) -> ShellCommand {
    let shell = config.shell.clone().unwrap_or_default();
    shell.screenshot.unwrap_or_default()
}

fn screen_record(config: &Config) -> ShellCommand {
    let shell = config.shell.clone().unwrap_or_default();
    shell.screen_record.unwrap_or_default()
}

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
            Group::untitled()
                .setting(
                    Setting::switch("Built-in Shell", "shell.built-in", |c| {
                        c.shell.as_ref().is_some_and(|shell| shell.built_in())
                    })
                    .keywords(&[
                        "bar",
                        "panel",
                        "experimental",
                        "desktop shell",
                    ]),
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
                ),
        )
        .group(
            Group::new("Screenshot Button")
                .description(
                    "The command runs with sh. Without one, the button takes bento's own \
                     screenshot. A notification with the title shows once the command is done; \
                     without a title, none does.",
                )
                .setting(
                    Setting::text("Command", "shell.screenshot.command", |c| {
                        screenshot(c).command.unwrap_or_default()
                    })
                    .keywords(&["screenshot", "grim", "slurp", "capture"]),
                )
                .setting(
                    Setting::text("Notification Title", "shell.screenshot.notification", |c| {
                        screenshot(c).notification.unwrap_or_default()
                    })
                    .keywords(&["screenshot", "notify", "toast"]),
                )
                .setting(
                    Setting::text(
                        "Notification Text",
                        "shell.screenshot.notification-body",
                        |c| screenshot(c).notification_body.unwrap_or_default(),
                    )
                    .keywords(&["screenshot", "notify", "body"]),
                ),
        )
        .group(
            Group::new("Screen Record Button")
                .description(
                    "The command runs with sh, and again to stop a recording if that's how \
                     the program works. A notification with the title shows once the command \
                     is done; without a title, none does.",
                )
                .setting(
                    Setting::text("Command", "shell.screen-record.command", |c| {
                        screen_record(c).command.unwrap_or_default()
                    })
                    .keywords(&["record", "recording", "video", "capture"]),
                )
                .setting(
                    Setting::text(
                        "Notification Title",
                        "shell.screen-record.notification",
                        |c| screen_record(c).notification.unwrap_or_default(),
                    )
                    .keywords(&["record", "notify", "toast"]),
                )
                .setting(
                    Setting::text(
                        "Notification Text",
                        "shell.screen-record.notification-body",
                        |c| screen_record(c).notification_body.unwrap_or_default(),
                    )
                    .keywords(&["record", "notify", "body"]),
                ),
        )
}
