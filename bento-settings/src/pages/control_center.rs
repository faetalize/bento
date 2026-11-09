//! What the buttons in the shell's control center run.

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
    Category::settings("control-center", "Control Center")
        .icon("ph-sliders-horizontal-symbolic")
        .keywords(&[
            "quick settings",
            "shell",
            "buttons",
            "screenshot",
            "screen record",
            "recording",
            "command",
        ])
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
