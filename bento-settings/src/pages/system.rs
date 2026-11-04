use crate::model::{Category, Group, Setting};

pub fn category() -> Category {
    Category::settings("system", "System")
        .icon("preferences-system-symbolic")
        .description("Startup, notifications, the clipboard, screenshots and the power button.")
        .keywords(&["general", "misc"])
        .group(
            Group::new("Startup").setting(
                Setting::switch(
                    "Show important shortcuts at startup",
                    "hotkey-overlay.skip-at-startup",
                    |c| c.hotkey_overlay.skip_at_startup,
                )
                .inverted()
                .keywords(&["hotkey overlay", "keybinds", "help", "cheat sheet"]),
            ),
        )
        .group(
            Group::new("Notifications").setting(
                Setting::switch(
                    "Tell me when the config has an error",
                    "config-notification.disable-failed",
                    |c| c.config_notification.disable_failed,
                )
                .inverted()
                .keywords(&["config error", "failed", "warning"]),
            ),
        )
        .group(
            Group::new("Clipboard").setting(
                Setting::switch("Middle-click paste", "clipboard.disable-primary", |c| {
                    c.clipboard.disable_primary
                })
                .inverted()
                .subtitle("Paste the last selected text with the middle button")
                .keywords(&["primary selection", "middle click"]),
            ),
        )
        .group(
            Group::new("Screenshots").setting(
                Setting::text("Save screenshots to", "screenshot-path", |c| {
                    c.screenshot_path.0.clone().unwrap_or_default()
                })
                .keywords(&["screenshot", "folder", "path", "pictures"]),
            ),
        )
        .group(
            Group::new("Power").setting(
                Setting::switch(
                    "Handle the power button",
                    "input.disable-power-key-handling",
                    |c| c.input.disable_power_key_handling,
                )
                .inverted()
                .subtitle("Turn off to let another program decide what the power button does")
                .keywords(&["power key", "suspend", "logind"]),
            ),
        )
}
