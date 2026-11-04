//! Pages that aren't in the app yet. They keep their place in the sidebar and in search, and point
//! at config.kdl meanwhile.

use crate::model::Category;

pub fn workspaces() -> Category {
    Category::planned("workspaces", "Workspaces")
        .icon("ph-browsers-symbolic")
        .keywords(&["virtual desktops", "spaces"])
}

pub fn window_rules() -> Category {
    Category::planned("window-rules", "Window Rules")
        .icon("ph-app-window-symbolic")
        .keywords(&["apps", "floating", "opacity", "rounded corners", "match"])
}

pub fn shortcuts() -> Category {
    Category::planned("shortcuts", "Shortcuts")
        .icon("ph-command-symbolic")
        .keywords(&["binds", "keybinds", "hotkeys", "keyboard"])
}

pub fn gestures() -> Category {
    Category::planned("gestures", "Gestures")
        .icon("ph-hand-swipe-right-symbolic")
        .keywords(&["swipe", "hot corners", "dnd", "edge"])
}

pub fn displays() -> Category {
    Category::planned("displays", "Displays")
        .icon("ph-monitor-symbolic")
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
        .icon("ph-rocket-launch-symbolic")
        .keywords(&["autostart", "spawn at startup", "environment", "login"])
}
