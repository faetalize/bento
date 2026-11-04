use crate::model::{Category, Group, Setting};

pub fn category() -> Category {
    Category::settings("system", "System")
        .icon("ph-gear-six-symbolic")
        .keywords(&["general", "misc"])
        .group(
            Group::untitled()
                .setting(
                    Setting::switch(
                        "Show Shortcuts at Login",
                        "hotkey-overlay.skip-at-startup",
                        |c| c.hotkey_overlay.skip_at_startup,
                    )
                    .inverted()
                    .keywords(&["hotkey overlay", "keybinds", "cheat sheet"]),
                )
                .setting(
                    Setting::switch(
                        "Config Error Notifications",
                        "config-notification.disable-failed",
                        |c| c.config_notification.disable_failed,
                    )
                    .inverted()
                    .keywords(&["failed", "warning"]),
                )
                .setting(
                    Setting::switch("Middle-Click Paste", "clipboard.disable-primary", |c| {
                        c.clipboard.disable_primary
                    })
                    .inverted()
                    .keywords(&["primary selection", "clipboard"]),
                )
                .setting(
                    Setting::switch("Power Button", "input.disable-power-key-handling", |c| {
                        c.input.disable_power_key_handling
                    })
                    .inverted()
                    .keywords(&["power key", "suspend", "logind"]),
                ),
        )
        .group(
            Group::untitled().setting(
                Setting::text("Screenshot Path", "screenshot-path", |c| {
                    c.screenshot_path.0.clone().unwrap_or_default()
                })
                .keywords(&["screenshot", "folder", "pictures"]),
            ),
        )
}
