//! Tests for the tiling host: mostly with the built-in fallback layout, which puts windows side by
//! side, each half the working area wide, and a few with the shipped layouts, to check that they
//! run in the engine.
//!
//! The test output is 1280 × 720 with 16 px gaps.

use std::fmt::Write as _;

use insta::assert_snapshot;

use super::*;

fn options_for(layout: &str) -> Options {
    Options {
        tiling: niri_config::Tiling {
            layout: niri_config::TilingLayout::BuiltIn(String::from(layout)),
            options: Vec::new(),
        },
        ..Options::default()
    }
}

/// Tiles in the order they were opened, with their positions on screen and which one has focus.
fn format_tiles(layout: &Layout<TestWindow>) -> String {
    let mut buf = String::new();
    let ws = layout.active_workspace().unwrap();
    let focused = layout.focus().map(|win| win.0.id);
    let mut tiles: Vec<_> = ws.tiles_with_render_positions().collect();
    tiles.sort_by_key(|(tile, _, _)| tile.window().id());
    for (tile, pos, _visible) in tiles {
        let Size { w, h, .. } = tile.tile_size();
        let Point { x, y, .. } = pos;
        let mark = if Some(tile.window().0.id) == focused {
            " focused"
        } else {
            ""
        };
        writeln!(
            &mut buf,
            "{w:>4.0} × {h:>3.0} at x:{x:>5.0} y:{y:>3.0}{mark}"
        )
        .unwrap();
    }
    buf
}

fn settle() -> [Op; 5] {
    [
        Op::Communicate(1),
        Op::Communicate(2),
        Op::Communicate(3),
        Op::Communicate(4),
        Op::CompleteAnimations,
    ]
}

fn add_window(id: usize) -> Op {
    Op::AddWindow {
        params: TestWindowParams::new(id),
    }
}

#[test]
fn layout_places_windows_and_moves_the_camera() {
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        add_window(2),
        add_window(3),
    ];
    ops.extend(settle());
    let mut layout = check_ops_with_options(options_for("fallback"), ops);

    // The third window is off to the right, so the camera followed it.
    assert_snapshot!(format_tiles(&layout), @"
    616 × 688 at x:-1248 y: 16
    616 × 688 at x: -616 y: 16
    616 × 688 at x:   16 y: 16 focused
    ");

    // Closing the focused window focuses the one before it, and the camera follows it.
    let mut ops = vec![Op::CloseWindow(3)];
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);
    assert_snapshot!(format_tiles(&layout), @"
    616 × 688 at x: -616 y: 16
    616 × 688 at x:   16 y: 16 focused
    ");
}

#[test]
fn niri_layout_runs_in_the_engine() {
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        add_window(2),
        add_window(3),
    ];
    ops.extend([
        Op::LayoutAction("focus-column-left", vec![]),
        Op::LayoutAction("consume-or-expel-window-left", vec![]),
    ]);
    ops.extend(settle());
    let layout = check_ops_with_options(options_for("niri"), ops);
    assert_snapshot!(format_tiles(&layout), @"
    100 × 336 at x:   16 y: 16
    100 × 336 at x:   16 y:368 focused
    100 × 688 at x:  132 y: 16
    ");
}

#[test]
fn bento_layout_runs_in_the_engine() {
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        add_window(2),
        add_window(3),
    ];
    let half = || vec![niri_config::TilingOption::String(String::from("50%"))];
    ops.extend([
        Op::FocusWindow(2),
        Op::LayoutAction("set-height", half()),
        Op::FocusWindow(3),
        Op::LayoutAction("set-height", half()),
    ]);
    ops.extend(settle());
    let layout = check_ops_with_options(options_for("bento"), ops);
    assert_snapshot!(format_tiles(&layout), @"
      89 × 688 at x:   16 y: 16
      90 × 336 at x:  121 y: 16
      89 × 336 at x:  227 y: 16 focused
    ");
}

#[test]
fn layout_action_binds_are_checked_against_declared_args() {
    use crate::layout::tiling::engine;

    let config = niri_config::Config::parse_mem_with_engine(
        r#"
        tiling {
            layout "bento"
        }
        "#,
        r#"
        binds {
            Mod+A { set-width "+10%"; }
            Mod+B { set-tabs true; }
            Mod+C { focus-index 2; }
            Mod+D { set-width true; }
            Mod+E { set-tabs "sideways"; }
            Mod+F { focus-index; }
            Mod+G { swap 1; }
            Mod+H { nope; }
            Mod+I { split-h; }
            Mod+J { focus-column-left; }
        }
        "#,
    )
    .unwrap();

    let problems = engine::check(&config).unwrap_err();
    assert_eq!(
        problems,
        [
            r#"the "change" argument of action "set-width" must be a size, like "50%" or "+10%""#,
            r#"the "tabbed" argument of action "set-tabs" must be true or false"#,
            r#"action "focus-index" needs its "index" argument"#,
            r#"action "swap" takes no arguments, not 1"#,
            r#"the bento layout engine has no action called "nope""#,
            // niri's names belong to niri.js.
            r#"the bento layout engine has no action called "focus-column-left""#,
        ]
    );
}

#[test]
fn engine_actions_cannot_take_native_names() {
    use crate::layout::tiling::engine;

    let dir = std::env::temp_dir().join(format!("bento-engine-names-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("clash.js");
    std::fs::write(
        &path,
        r#"
        export const meta = { api: 1, name: "clash", actions: { spawn: { description: "x" } } };
        export default function create(ws) { return {}; }
        "#,
    )
    .unwrap();

    let mut config = niri_config::Config::default();
    config.tiling.layout = niri_config::TilingLayout::File(path);
    let problems = engine::check(&config).unwrap_err();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(problems.len(), 1);
    assert!(
        problems[0].contains(
            r#"the layout engine's action "spawn" has the name of one of bento's actions"#
        ),
        "{problems:?}"
    );
}

#[test]
fn layout_engine_ipc_info_has_options_and_actions() {
    use niri_ipc::{
        LayoutEngineActionArgType, LayoutEngineOptionType, LayoutEngineOptionValue,
        LayoutEngineSource,
    };

    use crate::layout::tiling::engine;

    let tiling = niri_config::Tiling {
        layout: niri_config::TilingLayout::BuiltIn(String::from("bento")),
        options: vec![(
            String::from("gravity"),
            niri_config::TilingOption::Bool(false),
        )],
    };
    engine::ensure_loaded(&tiling);
    let info = engine::ipc_info(&tiling).unwrap();

    assert_eq!(info.name, "bento");
    assert_eq!(info.source, LayoutEngineSource::BuiltIn {});

    let gravity = &info.options[0];
    assert_eq!(gravity.name, "gravity");
    assert_eq!(gravity.r#type, LayoutEngineOptionType::Boolean);
    assert_eq!(gravity.default, LayoutEngineOptionValue::Boolean(true));
    assert_eq!(gravity.value, LayoutEngineOptionValue::Boolean(false));
    assert!(gravity.description.is_some());

    let actions: Vec<_> = info.actions.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(actions[0], "focus-left");
    assert!(actions.ends_with(&["split-h", "split-v", "swap", "stuff"]));

    let width = info.actions.iter().find(|a| a.name == "set-width").unwrap();
    assert_eq!(width.args.len(), 1);
    assert_eq!(width.args[0].r#type, Some(LayoutEngineActionArgType::Size));
}

/// Checks the native action table against the actions `bento msg action` knows. Actions only
/// configs have, like `suspend`, aren't in that list, so this can't catch those.
#[test]
fn native_action_table_lists_every_bindable_action() {
    use clap::Subcommand as _;

    let declared: Vec<_> = niri_config::actions::native_actions()
        .into_iter()
        .map(|action| action.name)
        .collect();
    let actions = niri_ipc::Action::augment_subcommands(clap::Command::new("action"));
    for action in actions.get_subcommands() {
        let name = action.get_name();
        if declared.iter().any(|d| d == name) || name == "layout-action" {
            continue;
        }
        // Actions only for the command line, like `focus-window --id`, don't parse in a config.
        for args in ["", " 1", " \"x\""] {
            let text = format!("binds {{\n    Mod+A {{ {name}{args}; }}\n}}");
            assert!(
                niri_config::Config::parse_mem(&text).is_err(),
                "{name} can be bound but isn't in the native action table"
            );
        }
    }
}

/// Options for an API 2 layout engine from `source`, which runs from a file.
fn options_for_source(name: &str, source: &str) -> Options {
    let dir = std::env::temp_dir().join(format!("bento-engine-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.js"));
    std::fs::write(&path, source).unwrap();
    Options {
        tiling: niri_config::Tiling {
            layout: niri_config::TilingLayout::File(path),
            options: Vec::new(),
        },
        ..Options::default()
    }
}

/// An engine that places windows in the left half of their workspace, and tells what it saw
/// through `bento.exposeState`.
const RECORDER: &str = r#"
export const meta = { api: 2, name: "recorder", floating: false };
export default function create(bento) {
    const log = [];
    bento.exposeState(() => ({
        monitors: bento.monitors.map((m) => [m.name, m.workspaces.length, m.activeWorkspace.index]),
        windows: bento.windows.map((w) => [w.id, w.workspace.monitor.name, w.workspace.index]),
        log,
    }));
    bento.addEventListener("windowadd", (e) => {
        const win = e.window;
        log.push(["windowadd", win.id, e.reason]);
        const area = win.workspace.tilingArea;
        win.place({ x: area.x, y: area.y, w: area.w / 2, h: area.h });
        win.focus();
        win.addEventListener("move", (m) => log.push(["move", win.id, m.from.index, m.to.index]));
        win.addEventListener("floatrequest", (f) => log.push(["floatrequest", win.id, f.reason]));
    });
}
"#;

#[test]
fn one_engine_sees_every_monitor_and_workspace() {
    let ops = [
        Op::AddOutput(1),
        Op::AddOutput(2),
        add_window(1),
        Op::FocusOutput(2),
        add_window(2),
    ];
    let layout = check_ops_with_options(options_for_source("sees-all", RECORDER), ops);
    drop(layout);

    let state = crate::layout::tiling::engine::exposed_state().unwrap();
    // Each monitor has its window's workspace and an empty one below.
    assert_eq!(
        state["monitors"],
        serde_json::json!([["output1", 2, 0], ["output2", 2, 0]])
    );
    assert_eq!(
        state["windows"],
        serde_json::json!([[1, "output1", 0], [2, "output2", 0]])
    );
}

#[test]
fn windows_moved_between_workspaces_are_told_to_the_engine() {
    let ops = [
        Op::AddOutput(1),
        add_window(1),
        Op::MoveWindowToWorkspace {
            window_id: Some(1),
            workspace_idx: 1,
        },
    ];
    let layout = check_ops_with_options(options_for_source("moves", RECORDER), ops);
    drop(layout);

    let state = crate::layout::tiling::engine::exposed_state().unwrap();
    assert_eq!(
        state["log"],
        serde_json::json!([["windowadd", 1, "open"], ["move", 1, 0, 1]])
    );
}

#[test]
fn engines_without_floating_get_float_requests() {
    let ops = [
        Op::AddOutput(1),
        add_window(1),
        Op::ToggleWindowFloating { id: Some(1) },
    ];
    let layout = check_ops_with_options(options_for_source("float", RECORDER), ops);

    assert!(!layout.active_workspace().unwrap().is_floating(&1));
    let state = crate::layout::tiling::engine::exposed_state().unwrap();
    assert_eq!(
        state["log"],
        serde_json::json!([["windowadd", 1, "open"], ["floatrequest", 1, "action"]])
    );
}

#[test]
fn nodes_that_the_engine_draws_are_in_the_workspace_tree() {
    use crate::layout::tiling::tree::NodeKind;

    let source = r##"
    export const meta = { api: 2, name: "boxes" };
    export default function create(bento) {
        bento.addEventListener("windowadd", (e) => {
            const win = e.window;
            win.place({ x: 0, y: 0, w: 100, h: 100 });
            win.focus();
            const box = bento.createNode("box", { rect: { x: 0, y: 0, w: 10, h: 10 }, background: "#fff" });
            win.workspace.tree.append(box);
        });
    }
    "##;
    let ops = [Op::AddOutput(1), add_window(1)];
    let layout = check_ops_with_options(options_for_source("boxes", source), ops);

    let ws = layout.active_workspace().unwrap();
    let tree = ws.scrolling().tree();
    let order = tree.draw_order();
    // The window is at the bottom, and what the engine drew goes above it.
    assert_eq!(order[1].1.kind, NodeKind::Window);
    assert_eq!(order[2].1.kind, NodeKind::Box);
    assert_eq!(order[2].1.props["background"], "#fff");
}
