//! bento's own actions, declared like a layout engine's `meta.actions`: a name, a description and
//! typed arguments, so settings screens can list them next to the engine's and offer the right
//! field for each argument.
//!
//! `layout-action` isn't here: configs don't bind it, it's for the command line.

use niri_ipc::{
    LayoutEngineAction as Declaration, LayoutEngineActionArg as Arg,
    LayoutEngineActionArgType as Type, LayoutEngineOptionValue as Value,
};

/// Every action bento has, as configs bind it.
pub fn native_actions() -> Vec<Declaration> {
    let focus = || property("focus", true, "Whether to follow the window there.");
    let workspace = || required("workspace", Type::Workspace, "Its index or its name.");
    let monitor = || required("monitor", Type::Monitor, "Its name, like \"DP-2\".");
    let size = |what| required("change", Type::Size, what);

    vec![
        action(
            "quit",
            "Exit bento.",
            vec![property(
                "skip-confirmation",
                false,
                "Exit without asking first.",
            )],
        ),
        action("suspend", "Suspend the computer.", vec![]),
        action(
            "power-off-monitors",
            "Turn all monitors off until there's input.",
            vec![],
        ),
        action("power-on-monitors", "Turn all monitors back on.", vec![]),
        action(
            "change-vt",
            "Switch to another virtual terminal.",
            vec![required("vt", Type::Number, "Its number.")],
        ),
        action("toggle-touchpad", "Turn the touchpad off or on.", vec![]),
        action(
            "toggle-keyboard-shortcuts-inhibit",
            "Let the focused window take bento's shortcuts, or stop letting it.",
            vec![],
        ),
        action(
            "show-hotkey-overlay",
            "Show the important shortcuts.",
            vec![],
        ),
        action(
            "toggle-launcher",
            "Show or hide the shell's launcher.",
            vec![],
        ),
        action(
            "switch-layout",
            "Switch the keyboard layout.",
            vec![required(
                "layout",
                Type::String,
                "\"next\", \"prev\", or the layout's index.",
            )],
        ),
        // Spawning.
        action(
            "spawn",
            "Run a program.",
            vec![required(
                "command",
                Type::Command,
                "The program and its arguments.",
            )],
        ),
        action(
            "spawn-sh",
            "Run a shell command.",
            vec![required(
                "command",
                Type::String,
                "The command, run with `sh -c`.",
            )],
        ),
        // Screenshots and casting.
        action(
            "screenshot",
            "Open the screenshot screen to pick an area.",
            vec![property(
                "show-pointer",
                true,
                "Whether the pointer is in the screenshot.",
            )],
        ),
        action(
            "screenshot-screen",
            "Take a screenshot of the focused monitor.",
            vec![
                property("write-to-disk", true, "Whether to save it as a file too."),
                property(
                    "show-pointer",
                    true,
                    "Whether the pointer is in the screenshot.",
                ),
            ],
        ),
        action(
            "screenshot-window",
            "Take a screenshot of the focused window.",
            vec![
                property("write-to-disk", true, "Whether to save it as a file too."),
                property(
                    "show-pointer",
                    false,
                    "Whether the pointer is in the screenshot.",
                ),
            ],
        ),
        action(
            "set-dynamic-cast-window",
            "Show the focused window in the dynamic screencast.",
            vec![],
        ),
        action(
            "set-dynamic-cast-monitor",
            "Show a monitor in the dynamic screencast.",
            vec![Arg {
                optional: true,
                ..required(
                    "monitor",
                    Type::Monitor,
                    "Its name; the focused one if left out.",
                )
            }],
        ),
        action(
            "clear-dynamic-cast-target",
            "Show nothing in the dynamic screencast.",
            vec![],
        ),
        action(
            "do-screen-transition",
            "Crossfade into whatever comes next on screen.",
            vec![Arg {
                property: true,
                default: Some(Value::Number(250.)),
                ..required("delay-ms", Type::Number, "How long to wait before fading.")
            }],
        ),
        // Windows.
        action("close-window", "Close the focused window.", vec![]),
        action(
            "fullscreen-window",
            "Make the focused window fullscreen, or not.",
            vec![],
        ),
        action(
            "toggle-windowed-fullscreen",
            "Tell the focused window it's fullscreen while it keeps its place, or stop.",
            vec![],
        ),
        action(
            "maximize-window-to-edges",
            "Maximize the focused window to the edges of the screen, or not.",
            vec![],
        ),
        action(
            "focus-window-previous",
            "Focus the window that had focus before.",
            vec![],
        ),
        action(
            "toggle-window-rule-opacity",
            "Turn the opacity from window rules off or on for the focused window.",
            vec![],
        ),
        // Floating.
        action(
            "toggle-window-floating",
            "Move the focused window between floating and tiling.",
            vec![],
        ),
        action(
            "move-window-to-floating",
            "Make the focused window float.",
            vec![],
        ),
        action("move-window-to-tiling", "Tile the focused window.", vec![]),
        action("focus-floating", "Focus the floating windows.", vec![]),
        action("focus-tiling", "Focus the tiled windows.", vec![]),
        action(
            "switch-focus-between-floating-and-tiling",
            "Focus the floating windows, or the tiled ones if they have focus.",
            vec![],
        ),
        action(
            "focus-floating-left",
            "Focus the floating window to the left.",
            vec![],
        ),
        action(
            "focus-floating-right",
            "Focus the floating window to the right.",
            vec![],
        ),
        action(
            "focus-floating-up",
            "Focus the floating window above.",
            vec![],
        ),
        action(
            "focus-floating-down",
            "Focus the floating window below.",
            vec![],
        ),
        action(
            "focus-floating-leftmost",
            "Focus the leftmost floating window.",
            vec![],
        ),
        action(
            "focus-floating-rightmost",
            "Focus the rightmost floating window.",
            vec![],
        ),
        action(
            "focus-floating-topmost",
            "Focus the topmost floating window.",
            vec![],
        ),
        action(
            "focus-floating-bottommost",
            "Focus the bottommost floating window.",
            vec![],
        ),
        action(
            "move-floating-left",
            "Move the floating window to the left.",
            vec![],
        ),
        action(
            "move-floating-right",
            "Move the floating window to the right.",
            vec![],
        ),
        action("move-floating-up", "Move the floating window up.", vec![]),
        action(
            "move-floating-down",
            "Move the floating window down.",
            vec![],
        ),
        action(
            "center-floating-window",
            "Center the floating window on screen.",
            vec![],
        ),
        action(
            "set-floating-window-width",
            "Set or change the floating window's width.",
            vec![size(
                "A size like \"50%\" or \"800\", or a change like \"+10%\".",
            )],
        ),
        action(
            "set-floating-window-height",
            "Set or change the floating window's height.",
            vec![size(
                "A size like \"50%\" or \"800\", or a change like \"+10%\".",
            )],
        ),
        // Groups.
        action(
            "group-window-left",
            "Group the focused window with the window to the left.",
            vec![],
        ),
        action(
            "group-window-right",
            "Group the focused window with the window to the right.",
            vec![],
        ),
        action(
            "group-window-up",
            "Group the focused window with the window above.",
            vec![],
        ),
        action(
            "group-window-down",
            "Group the focused window with the window below.",
            vec![],
        ),
        action(
            "ungroup-window",
            "Take the focused window out of its group.",
            vec![],
        ),
        // Workspaces.
        action(
            "focus-workspace-down",
            "Switch to the workspace below.",
            vec![],
        ),
        action(
            "focus-workspace-up",
            "Switch to the workspace above.",
            vec![],
        ),
        action(
            "focus-workspace",
            "Switch to a workspace.",
            vec![workspace()],
        ),
        action(
            "focus-workspace-previous",
            "Switch back to the workspace you were on.",
            vec![],
        ),
        action(
            "move-window-to-workspace-down",
            "Move the focused window to the workspace below.",
            vec![focus()],
        ),
        action(
            "move-window-to-workspace-up",
            "Move the focused window to the workspace above.",
            vec![focus()],
        ),
        action(
            "move-window-to-workspace",
            "Move the focused window to a workspace.",
            vec![workspace(), focus()],
        ),
        action(
            "move-workspace-down",
            "Move the focused workspace down.",
            vec![],
        ),
        action(
            "move-workspace-up",
            "Move the focused workspace up.",
            vec![],
        ),
        action(
            "move-workspace-to-index",
            "Move the focused workspace to a place on its monitor.",
            vec![required(
                "index",
                Type::Number,
                "Counting from 1 at the top.",
            )],
        ),
        action(
            "set-workspace-name",
            "Name the focused workspace.",
            vec![required("name", Type::String, "The name.")],
        ),
        action(
            "unset-workspace-name",
            "Remove the focused workspace's name.",
            vec![],
        ),
        // Monitors.
        action(
            "focus-monitor-left",
            "Focus the monitor to the left.",
            vec![],
        ),
        action(
            "focus-monitor-right",
            "Focus the monitor to the right.",
            vec![],
        ),
        action("focus-monitor-up", "Focus the monitor above.", vec![]),
        action("focus-monitor-down", "Focus the monitor below.", vec![]),
        action(
            "focus-monitor-previous",
            "Focus the previous monitor.",
            vec![],
        ),
        action("focus-monitor-next", "Focus the next monitor.", vec![]),
        action("focus-monitor", "Focus a monitor.", vec![monitor()]),
        action(
            "move-window-to-monitor-left",
            "Move the focused window to the monitor to the left.",
            vec![],
        ),
        action(
            "move-window-to-monitor-right",
            "Move the focused window to the monitor to the right.",
            vec![],
        ),
        action(
            "move-window-to-monitor-up",
            "Move the focused window to the monitor above.",
            vec![],
        ),
        action(
            "move-window-to-monitor-down",
            "Move the focused window to the monitor below.",
            vec![],
        ),
        action(
            "move-window-to-monitor-previous",
            "Move the focused window to the previous monitor.",
            vec![],
        ),
        action(
            "move-window-to-monitor-next",
            "Move the focused window to the next monitor.",
            vec![],
        ),
        action(
            "move-window-to-monitor",
            "Move the focused window to a monitor.",
            vec![monitor()],
        ),
        action(
            "move-workspace-to-monitor-left",
            "Move the focused workspace to the monitor to the left.",
            vec![],
        ),
        action(
            "move-workspace-to-monitor-right",
            "Move the focused workspace to the monitor to the right.",
            vec![],
        ),
        action(
            "move-workspace-to-monitor-up",
            "Move the focused workspace to the monitor above.",
            vec![],
        ),
        action(
            "move-workspace-to-monitor-down",
            "Move the focused workspace to the monitor below.",
            vec![],
        ),
        action(
            "move-workspace-to-monitor-previous",
            "Move the focused workspace to the previous monitor.",
            vec![],
        ),
        action(
            "move-workspace-to-monitor-next",
            "Move the focused workspace to the next monitor.",
            vec![],
        ),
        action(
            "move-workspace-to-monitor",
            "Move the focused workspace to a monitor.",
            vec![monitor()],
        ),
        // The overview.
        action("toggle-overview", "Open or close the overview.", vec![]),
        action("open-overview", "Open the overview.", vec![]),
        action("close-overview", "Close the overview.", vec![]),
        // Debugging bento itself.
        action(
            "toggle-debug-tint",
            "Tint windows, for debugging bento.",
            vec![],
        ),
        action(
            "debug-toggle-opaque-regions",
            "Show the areas bento treats as opaque, for debugging.",
            vec![],
        ),
        action(
            "debug-toggle-damage",
            "Show what bento redraws, for debugging.",
            vec![],
        ),
    ]
}

fn action(name: &str, description: &str, args: Vec<Arg>) -> Declaration {
    Declaration {
        name: String::from(name),
        description: Some(String::from(description)),
        args,
    }
}

/// A positional argument that must be given.
fn required(name: &str, r#type: Type, description: &str) -> Arg {
    Arg {
        name: String::from(name),
        r#type: Some(r#type),
        default: None,
        optional: false,
        property: false,
        choices: Vec::new(),
        description: Some(String::from(description)),
    }
}

/// A `true` or `false` property, like `focus=false`.
fn property(name: &str, default: bool, description: &str) -> Arg {
    Arg {
        property: true,
        default: Some(Value::Boolean(default)),
        ..required(name, Type::Boolean, description)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    /// A bind of `action` with an example for each argument it needs.
    fn example(action: &Declaration) -> String {
        let mut node = action.name.clone();
        for arg in &action.args {
            if arg.default.is_some() || arg.optional {
                continue;
            }
            let value = match arg.r#type {
                Some(Type::Number) => "1",
                Some(Type::Size) => "\"50%\"",
                Some(Type::Workspace) => "1",
                Some(Type::Command) => "\"true\"",
                Some(Type::String) if action.name == "switch-layout" => "\"next\"",
                _ => "\"x\"",
            };
            node = format!("{node} {value}");
        }
        node
    }

    #[test]
    fn every_native_action_parses_in_a_config() {
        for action in native_actions() {
            let text = format!("binds {{\n    Mod+A {{ {}; }}\n}}", example(&action));
            if let Err(err) = Config::parse_mem(&text) {
                panic!("{text}: {:?}", miette::Report::new(err));
            }
        }
    }

    #[test]
    fn native_actions_have_unique_names() {
        let actions = native_actions();
        let mut names: Vec<_> = actions.iter().map(|a| a.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), actions.len());
    }
}
