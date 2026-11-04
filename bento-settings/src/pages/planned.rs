//! Pages that aren't in the app yet. They keep their place in the sidebar and in search, and point
//! at config.kdl meanwhile.

use crate::model::Category;

pub fn workspaces() -> Category {
    Category::planned("workspaces", "Workspaces")
        .icon("view-paged-symbolic")
        .description("Named workspaces, and which monitor they open on.")
        .keywords(&["virtual desktops", "spaces"])
}

pub fn window_rules() -> Category {
    Category::planned("window-rules", "Window Rules")
        .icon("window-new-symbolic")
        .description("Per-app rules: where windows open, their size, opacity and corners.")
        .keywords(&["apps", "floating", "opacity", "rounded corners", "match"])
}

pub fn shortcuts() -> Category {
    Category::planned("shortcuts", "Shortcuts")
        .icon("preferences-desktop-keyboard-shortcuts-symbolic")
        .description("Key bindings for windows, workspaces, the layout engine and apps.")
        .keywords(&["binds", "keybinds", "hotkeys", "keyboard"])
}

pub fn gestures() -> Category {
    Category::planned("gestures", "Gestures")
        .icon("view-fullscreen-symbolic")
        .description("Touchpad swipes, hot corners and dragging windows to the screen edge.")
        .keywords(&["swipe", "hot corners", "dnd", "edge"])
}

pub fn displays() -> Category {
    Category::planned("displays", "Displays")
        .icon("video-display-symbolic")
        .description("Resolution, scale, refresh rate and where each monitor sits.")
        .keywords(&[
            "monitors",
            "outputs",
            "resolution",
            "scale",
            "refresh rate",
            "vrr",
        ])
}

pub fn startup() -> Category {
    Category::planned("startup", "Startup Apps")
        .icon("system-run-symbolic")
        .description("Programs bento starts with the session, and their environment.")
        .keywords(&["autostart", "spawn at startup", "environment", "login"])
}
