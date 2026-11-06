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
            ..niri_config::Tiling::default()
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
        Op::LayoutAction("focus-column-left", vec![], None),
        Op::LayoutAction("consume-or-expel-window-left", vec![], None),
    ]);
    ops.extend(settle());
    let layout = check_ops_with_options(options_for("niri"), ops);
    assert_snapshot!(format_tiles(&layout), @"
    616 × 336 at x:   16 y: 16
    616 × 336 at x:   16 y:368 focused
    616 × 688 at x:  648 y: 16
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
        Op::LayoutAction("set-height", half(), None),
        Op::FocusWindow(3),
        Op::LayoutAction("set-height", half(), None),
    ]);
    ops.extend(settle());
    let layout = check_ops_with_options(options_for("bento"), ops);
    assert_snapshot!(format_tiles(&layout), @"
      616 × 688 at x: -616 y: 16
      616 × 336 at x:   16 y: 16
      616 × 336 at x:  648 y: 16 focused
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

/// An engine file that declares an option and a window rule property of every type, in a
/// directory of its own that the caller removes.
fn typed_engine(name: &str) -> (std::path::PathBuf, niri_config::TilingLayout) {
    let dir = std::env::temp_dir().join(format!("bento-engine-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("typed.js");
    std::fs::write(
        &path,
        r##"
        export const meta = {
            api: 2,
            name: "typed",
            options: {
                "center": { type: "choice", choices: ["never", "always"], default: "never" },
                "width": { type: "size", default: "50%" },
                "presets": { type: "size", list: true, default: ["33%", "50%"] },
                "flag": { type: "boolean", default: false },
                "gap": { type: "number", default: 5 },
                "color": { type: "string", default: "#fff" },
                "tint": { type: "color", default: "#102030" },
                "glow": { type: "gradient", default: null },
            },
            windowRules: {
                "width": { type: "size" },
                "open-maximized": { type: "boolean", default: false },
                "tab-tint": { type: "color" },
                "tab-glow": { type: "gradient" },
            },
        };
        export default function create(bento) {}
        "##,
    )
    .unwrap();
    (dir, niri_config::TilingLayout::File(path))
}

fn strings(values: &[&str]) -> Vec<niri_config::TilingOption> {
    let string = |v: &&str| niri_config::TilingOption::String(String::from(*v));
    values.iter().map(string).collect()
}

#[test]
fn engine_options_and_rules_are_converted_by_their_declared_types() {
    use niri_config::{EngineRules, TilingOption};
    use serde_json::json;

    use crate::layout::tiling::engine;

    let (dir, layout) = typed_engine("convert");
    let tiling = niri_config::Tiling {
        layout,
        options: vec![
            (String::from("center"), strings(&["always"])),
            // A size with no value is left to the window.
            (String::from("width"), vec![]),
            (String::from("presets"), strings(&["25%", "800"])),
            (String::from("nope"), vec![TilingOption::Bool(true)]),
        ],
        ..niri_config::Tiling::default()
    };
    engine::ensure_loaded(&tiling);

    // Options the config leaves out, or gets wrong, have their defaults.
    let options = engine::options_json(&tiling);
    assert_eq!(
        options,
        json!({
            "center": "always",
            "width": null,
            "presets": [{ "proportion": 0.25 }, { "fixed": 800 }],
            "flag": false,
            "gap": 5.0,
            "color": "#fff",
            "tint": "#102030",
            "glow": null,
        })
    );

    let glow = niri_ipc::LayoutEngineGradient {
        from: String::from("#f00"),
        to: String::from("#00f"),
        angle: Some(90.),
        relative_to: Some(String::from("view")),
        color_space: None,
    };
    let rules = EngineRules(vec![
        (String::from("width"), strings(&["67%"])),
        (
            String::from("open-maximized"),
            vec![TilingOption::Bool(true)],
        ),
        (String::from("tab-tint"), strings(&["#abc"])),
        (String::from("tab-glow"), vec![TilingOption::Gradient(glow)]),
        (String::from("nope"), vec![TilingOption::Bool(true)]),
    ]);
    // Only what the rules set, and what the engine declares.
    assert_eq!(
        engine::rules_json(&rules),
        json!({
            "width": { "proportion": 0.67 },
            "open-maximized": true,
            "tab-tint": "#abc",
            "tab-glow": { "from": "#f00", "to": "#00f", "angle": 90.0, "relativeTo": "view" },
        })
    );
    assert_eq!(engine::rules_json(&EngineRules::default()), json!({}));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn engine_options_and_rules_are_checked_against_the_running_engine() {
    use niri_config::{EngineRules, EngineWindowRule, TilingOption};

    use crate::layout::tiling::engine;

    let (dir, layout) = typed_engine("check");
    let tiling = niri_config::Tiling {
        layout,
        options: vec![
            (String::from("nope"), vec![TilingOption::Bool(true)]),
            (String::from("center"), strings(&["sideways"])),
            (String::from("width"), strings(&["+10%"])),
            (String::from("presets"), vec![TilingOption::Bool(true)]),
            (String::from("gap"), vec![]),
            (String::from("flag"), vec![TilingOption::Bool(true)]),
            (String::from("tint"), strings(&["blue"])),
            (String::from("glow"), strings(&["#fff"])),
        ],
        output_options: vec![niri_config::ScopedOptions {
            name: String::from("DP-2"),
            options: vec![(String::from("tint"), strings(&["#12"]))],
        }],
        workspace_options: vec![niri_config::ScopedOptions {
            name: String::from("web"),
            options: vec![(String::from("nope"), vec![TilingOption::Bool(true)])],
        }],
        window_rules: vec![EngineWindowRule {
            matches: Vec::new(),
            excludes: Vec::new(),
            properties: EngineRules(vec![
                (String::from("width"), strings(&["50%"])),
                (String::from("open-maximized"), strings(&["yes"])),
                (String::from("tab-tint"), strings(&["red"])),
                (String::from("zzz"), vec![TilingOption::Bool(true)]),
            ]),
        }],
    };
    let config = niri_config::Config {
        tiling,
        ..niri_config::Config::default()
    };

    let problems = engine::check(&config).unwrap_err();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(
        problems,
        [
            r#"the typed layout engine has no option called "nope""#,
            r#"layout engine option "center" must be one of ["never", "always"]"#,
            r#"layout engine option "width" must be a size, like "50%" or "800""#,
            r#"layout engine option "presets" must be a size, like "50%" or "800""#,
            r#"layout engine option "gap" needs a value"#,
            r##"layout engine option "tint" must be a colour, like "#rrggbb" or "#rrggbbaa""##,
            r##"layout engine option "glow" must be a gradient, like `from="#000" to="#fff"`"##,
            r##"layout engine option "tint" of output "DP-2" must be a colour, like "#rrggbb" or "#rrggbbaa""##,
            r#"the typed layout engine has no option called "nope" of workspace "web""#,
            r#"layout engine window rule "open-maximized" must be true or false"#,
            r##"layout engine window rule "tab-tint" must be a colour, like "#rrggbb" or "#rrggbbaa""##,
            r#"the typed layout engine has no window rule called "zzz""#,
        ]
    );
}

#[test]
fn engine_defaults_must_fit_their_types() {
    use crate::layout::tiling::engine;

    let dir = std::env::temp_dir().join(format!("bento-engine-defaults-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("wrong.js");
    std::fs::write(
        &path,
        r#"
        export const meta = {
            api: 2,
            name: "wrong",
            options: { "center": { type: "choice", choices: ["a"], default: "b" } },
        };
        export default function create(bento) {}
        "#,
    )
    .unwrap();

    let mut config = niri_config::Config::default();
    config.tiling.layout = niri_config::TilingLayout::File(path);
    let problems = engine::check(&config).unwrap_err();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(problems.len(), 1);
    assert!(
        problems[0].contains(r#"default for its option "center" is wrong"#),
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
            vec![niri_config::TilingOption::Bool(false)],
        )],
        ..niri_config::Tiling::default()
    };
    engine::ensure_loaded(&tiling);
    let info = engine::ipc_info(&tiling).unwrap();

    assert_eq!(info.name, "bento");
    assert_eq!(info.source, LayoutEngineSource::BuiltIn {});

    let gravity = &info.options[0];
    assert_eq!(gravity.name, "gravity");
    assert_eq!(gravity.r#type, LayoutEngineOptionType::Boolean);
    assert!(!gravity.list);
    assert_eq!(
        gravity.default,
        Some(LayoutEngineOptionValue::Boolean(true))
    );
    assert_eq!(gravity.value, Some(LayoutEngineOptionValue::Boolean(false)));
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
            ..niri_config::Tiling::default()
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
    // Window ids are unique across the process, so windows go by the order the engine saw them.
    const seen = new Map();
    const n = (win) => {
        if (!seen.has(win.id)) seen.set(win.id, seen.size + 1);
        return seen.get(win.id);
    };
    bento.exposeState(() => ({
        monitors: bento.monitors.map((m) => [m.name, m.workspaces.length, m.activeWorkspace.index]),
        windows: bento.windows.map((w) => [n(w), w.workspace.monitor.name, w.workspace.index]),
        log,
    }));
    bento.addEventListener("windowadd", (e) => {
        const win = e.window;
        log.push(["windowadd", n(win), e.reason]);
        const area = win.workspace.tilingArea;
        win.place({ x: area.x, y: area.y, w: area.w / 2, h: area.h });
        win.focus();
        win.addEventListener("move", (m) => log.push(["move", n(win), m.from.index, m.to.index]));
        win.addEventListener("floatrequest", (f) => log.push(["floatrequest", n(win), f.reason]));
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
fn workspaces_have_the_options_of_their_output_and_their_own() {
    use niri_config::{ScopedOptions, TilingOption};

    use crate::layout::tiling::engine;

    let source = r#"
        export const meta = {
            api: 2,
            name: "scoped",
            options: { rows: { type: "number", default: 1 } },
        };
        export default function create(bento) {
            bento.exposeState(() => ({
                top: bento.options.rows,
                workspaces: bento.monitors.flatMap((m) =>
                    m.workspaces.map((ws) => [m.name, ws.name, ws.options.rows]),
                ),
            }));
        }
    "#;
    let rows = |rows: f64| vec![(String::from("rows"), vec![TilingOption::Number(rows)])];
    let mut options = options_for_source("scoped-options", source);
    options.tiling.options = rows(4.);
    options.tiling.output_options = vec![ScopedOptions {
        name: String::from("output2"),
        options: rows(6.),
    }];
    options.tiling.workspace_options = vec![ScopedOptions {
        name: String::from("ws1"),
        options: rows(8.),
    }];

    let ops = [
        Op::AddOutput(1),
        Op::SetWorkspaceName {
            new_ws_name: 1,
            ws_name: None,
        },
        Op::AddOutput(2),
    ];
    let layout = check_ops_with_options(options, ops);
    drop(layout);

    let state = engine::exposed_state().unwrap();
    // `Bento.options` stays the top-level ones.
    assert_eq!(state["top"], serde_json::json!(4));
    let workspaces = state["workspaces"].as_array().unwrap();
    let rows_of = |monitor: &str, name: Option<&str>| {
        let found = workspaces
            .iter()
            .find(|ws| ws[0] == monitor && ws[1] == serde_json::json!(name));
        found.map(|ws| ws[2].clone())
    };
    assert_eq!(rows_of("output1", Some("ws1")), Some(serde_json::json!(8)));
    assert_eq!(rows_of("output2", None), Some(serde_json::json!(6)));
    // The empty workspace below it is no one's in particular.
    assert_eq!(rows_of("output1", None), Some(serde_json::json!(4)));
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
    let engine_id = ws.scrolling().engine_id_of(&1).unwrap();
    let tree = ws.scrolling().tree();
    let root = tree.root_id();
    // The window is at the bottom, and what the engine drew goes above it.
    assert_eq!(
        tree.children(root),
        [format!("w{engine_id}"), String::from("n1")]
    );
    assert_eq!(tree.window_order(), [engine_id]);
    assert_eq!(
        tree.drawn("n1").unwrap().rect,
        Rectangle::new(Point::from((0., 0.)), Size::from((10., 10.)))
    );
}

/// An engine that puts a window at the top left of its workspace and draws buttons around it: one
/// below it, one above it that overlaps it, one far from it, and a panel with a button in it. The
/// buttons tell what they got through `bento.exposeState`, as `[button, event, button number,
/// pointer x, pointer y, ctrl, wheel y]`.
const BUTTONS: &str = r#"
export const meta = { api: 2, name: "buttons" };
export default function create(bento) {
    const log = [];
    bento.exposeState(() => ({ log }));
    bento.addEventListener("windowadd", (e) => {
        const win = e.window;
        win.place({ x: 0, y: 0, w: 100, h: 100 });
        win.focus();
        const tree = win.workspace.tree;
        const button = (name, rect, parent, before) => {
            const node = bento.createNode("box", { rect, interactive: true });
            for (const type of ["pointerenter", "pointerleave", "pointermove", "pointerdown", "pointerup", "click", "wheel"]) {
                node.addEventListener(type, (p) => {
                    log.push([name, type, p.button, p.pointer.x, p.pointer.y, p.modifiers.ctrl, p.delta.y]);
                });
            }
            parent.append(node, before);
            return node;
        };
        button("below", { x: 0, y: 0, w: 30, h: 30 }, tree, win.node);
        button("above", { x: 50, y: 50, w: 100, h: 100 }, tree);
        button("far", { x: 500, y: 0, w: 50, h: 50 }, tree);
        const panel = bento.createNode("box", { rect: { x: 600, y: 0, w: 100, h: 100 } });
        tree.append(panel);
        const inner = button("inner", { x: 610, y: 10, w: 20, h: 20 }, panel);
        panel.addEventListener("click", (p) => log.push(["panel", "click", p.target === inner]));
    });
}
"#;

fn buttons_layout() -> Layout<TestWindow> {
    let mut ops = vec![Op::AddOutput(1), add_window(1)];
    ops.extend(settle());
    check_ops_with_options(options_for_source("buttons", BUTTONS), ops)
}

/// The id of the interactive node under a point of the view, if there's one.
fn node_under(layout: &Layout<TestWindow>, x: f64, y: f64) -> Option<String> {
    let output = layout.active_output().unwrap();
    layout
        .node_under(output, Point::from((x, y)))
        .map(|hit| hit.node)
}

fn node_log() -> serde_json::Value {
    crate::layout::tiling::engine::exposed_state().unwrap()["log"].clone()
}

#[test]
fn nodes_are_under_the_pointer_unless_a_window_is_above_them() {
    let layout = buttons_layout();

    // The nodes are created in this order, as n1 to n5.
    // Over the window, the node above it takes the pointer.
    assert_eq!(node_under(&layout, 75., 75.).as_deref(), Some("n2"));
    // Past the window it's still the node's.
    assert_eq!(node_under(&layout, 120., 120.).as_deref(), Some("n2"));
    // The node below the window is covered by it.
    assert_eq!(node_under(&layout, 10., 10.), None);
    assert_eq!(node_under(&layout, 520., 10.).as_deref(), Some("n3"));
    // The panel isn't interactive, and what's in it is.
    assert_eq!(node_under(&layout, 650., 80.), None);
    assert_eq!(node_under(&layout, 620., 20.).as_deref(), Some("n5"));
    assert_eq!(node_under(&layout, 300., 300.), None);
}

#[test]
fn pointer_events_reach_the_engine_and_bubble() {
    let mut layout = buttons_layout();
    let none = [false; 4];
    let origin = Point::from((0., 0.));

    let hit = |layout: &Layout<TestWindow>, x, y| {
        let output = layout.active_output().unwrap();
        layout.node_under(output, Point::from((x, y)))
    };

    // Entering a node, and moving on it.
    let far = hit(&layout, 520., 10.);
    layout.node_pointer_moved(far.as_ref(), none);
    assert_eq!(
        node_log(),
        serde_json::json!([
            ["far", "pointerenter", 0, 520, 10, false, 0],
            ["far", "pointermove", 0, 520, 10, false, 0],
        ])
    );

    // Moving to another one leaves the first.
    let inner = hit(&layout, 620., 20.);
    layout.node_pointer_moved(inner.as_ref(), none);
    assert_eq!(
        node_log().as_array().unwrap()[2..],
        [
            serde_json::json!(["far", "pointerleave", 0, 620, 20, false, 0]),
            serde_json::json!(["inner", "pointerenter", 0, 620, 20, false, 0]),
            serde_json::json!(["inner", "pointermove", 0, 620, 20, false, 0]),
        ]
    );

    // A click goes to the node, with its button and the modifiers, and bubbles to its parent.
    let before = node_log().as_array().unwrap().len();
    let pos = Point::from((620., 20.));
    layout.node_pointer_event("n5", "pointerdown", pos, 1, none, origin);
    layout.node_pointer_event("n5", "pointerup", pos, 1, none, origin);
    layout.node_pointer_event("n5", "click", pos, 1, [false, true, false, false], origin);
    layout.node_pointer_event("n5", "wheel", pos, 0, none, Point::from((0., 15.)));
    assert_eq!(
        node_log().as_array().unwrap()[before..],
        [
            serde_json::json!(["inner", "pointerdown", 1, 620, 20, false, 0]),
            serde_json::json!(["inner", "pointerup", 1, 620, 20, false, 0]),
            serde_json::json!(["inner", "click", 1, 620, 20, true, 0]),
            serde_json::json!(["panel", "click", true]),
            serde_json::json!(["inner", "wheel", 0, 620, 20, false, 15]),
        ]
    );

    // The pointer leaving all nodes leaves the one it was on, and nothing else.
    let before = node_log().as_array().unwrap().len();
    layout.node_pointer_moved(None, none);
    assert_eq!(
        node_log().as_array().unwrap()[before..],
        [serde_json::json!([
            "inner",
            "pointerleave",
            0,
            620,
            20,
            false,
            0
        ])]
    );
    layout.node_pointer_moved(None, none);
    assert_eq!(node_log().as_array().unwrap().len(), before + 1);
}

#[test]
fn a_window_covering_a_node_leaves_the_pointer_to_the_window() {
    let mut layout = buttons_layout();
    let none = [false; 4];

    // On the node above the window, then on the window where the node below it is.
    let output = layout.active_output().unwrap().clone();
    let hit = layout.node_under(&output, Point::from((75., 75.)));
    assert_eq!(hit.as_ref().map(|hit| hit.node.as_str()), Some("n2"));
    layout.node_pointer_moved(hit.as_ref(), none);

    let covered = layout.node_under(&output, Point::from((10., 10.)));
    assert!(covered.is_none());
    layout.node_pointer_moved(covered.as_ref(), none);

    let log = node_log();
    let events: Vec<_> = log
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| (entry[0].as_str().unwrap(), entry[1].as_str().unwrap()))
        .collect();
    assert_eq!(
        events,
        [
            ("above", "pointerenter"),
            ("above", "pointermove"),
            ("above", "pointerleave"),
        ]
    );
}

#[test]
fn what_is_drawn_on_a_window_moves_with_it_to_another_workspace() {
    let source = r##"
    export const meta = { api: 2, name: "badges" };
    export default function create(bento) {
        let badge;
        bento.addEventListener("windowadd", (e) => {
            const win = e.window;
            win.place({ x: 0, y: 0, w: 100, h: 100 });
            win.focus();
            badge = bento.createNode("box", { rect: { x: 0, y: 0, w: 10, h: 10 } });
            win.node.append(badge);
            win.addEventListener("move", () => badge.set({ opacity: 0.5 }));
        });
    }
    "##;
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        Op::MoveWindowToWorkspace {
            window_id: Some(1),
            workspace_idx: 1,
        },
    ];
    ops.extend(settle());
    let layout = check_ops_with_options(options_for_source("badges", source), ops);

    let (_, _, ws) = layout
        .workspaces()
        .find(|(_, _, ws)| ws.scrolling().has_window(&1))
        .unwrap();
    let engine_id = ws.scrolling().engine_id_of(&1).unwrap();
    let tree = ws.scrolling().tree();
    assert_eq!(
        tree.path("n1"),
        [
            String::from("n1"),
            format!("w{engine_id}"),
            tree.root_id().to_owned()
        ]
    );
    // The layout's change to the node went to the tree it's in now.
    assert_eq!(tree.drawn("n1").unwrap().opacity, 0.5);

    for (_, _, other) in layout
        .workspaces()
        .filter(|(_, _, other)| other.id() != ws.id())
    {
        assert!(other.scrolling().tree().window_order().is_empty());
        assert!(!other.scrolling().tree().contains("n1"));
    }
}

/// An engine that places windows side by side, 100 wide, groups on request, and tells what it
/// saw through `bento.exposeState`. Windows go by the order the engine saw them.
const GROUPER: &str = r#"
export const meta = {
    api: 2,
    name: "grouper",
    actions: {
        "group-all": { description: "Groups every window." },
        "group-first": { description: "Groups the first two windows, with the second one active." },
        "dissolve": { description: "Dissolves the first group." },
        "mark": { description: "Logs the window it's for." },
    },
};
export default function create(bento) {
    const log = [];
    const seen = new Map();
    const n = (win) => seen.get(win.id);
    const ids = (group) => [group.windows.map(n), n(group.active)];
    bento.exposeState(() => ({ groups: bento.groups.map(ids), log }));
    bento.addEventListener("windowadd", (e) => {
        const win = e.window;
        seen.set(win.id, seen.size + 1);
        const x = (win.workspace.windows.length - 1) * 110;
        win.place({ x, y: 0, w: 100, h: 100 });
        win.focus();
    });
    bento.addEventListener("groupadd", (e) => {
        log.push(["groupadd", ...ids(e.group)]);
        e.group.addEventListener("change", (c) => log.push(["change", ...ids(c.target)]));
        e.group.addEventListener("remove", () => log.push(["remove"]));
    });
    bento.addEventListener("action", (e) => {
        if (e.name === "group-all") {
            bento.createGroup(bento.windows);
        } else if (e.name === "group-first") {
            bento.createGroup(bento.windows.slice(0, 2), bento.windows[1]);
        } else if (e.name === "dissolve") {
            bento.groups[0].dissolve();
        } else if (e.name === "mark") {
            log.push(["mark", e.window ? n(e.window) : null]);
        }
    });
}
"#;

/// The groups of a layout as its windows' ids, in the order they joined, and the active window.
fn groups_of(layout: &Layout<TestWindow>) -> Vec<(Vec<usize>, usize)> {
    layout
        .window_groups()
        .iter()
        .map(|group| {
            let windows = group.windows.iter().map(|win| *win.id()).collect();
            (windows, *group.active.id())
        })
        .collect()
}

fn three_windows() -> [Op; 4] {
    [
        Op::AddOutput(1),
        add_window(1),
        add_window(2),
        add_window(3),
    ]
}

#[test]
fn groups_made_by_the_engine_are_bento_state() {
    let mut ops = three_windows().to_vec();
    ops.push(Op::LayoutAction("group-all", vec![], None));
    let layout = check_ops_with_options(options_for_source("groups-engine", GROUPER), ops);

    assert_eq!(groups_of(&layout), [(vec![1, 2, 3], 1)]);
    // The engine sees the group it made as bento's, announced once.
    let state = crate::layout::tiling::engine::exposed_state().unwrap();
    assert_eq!(state["groups"], serde_json::json!([[[1, 2, 3], 1]]));
    assert_eq!(
        state["log"],
        serde_json::json!([["groupadd", [1, 2, 3], 1]])
    );
}

#[test]
fn native_group_actions_work_under_any_engine() {
    use crate::layout::groups::Direction;

    let mut ops = three_windows().to_vec();
    // 3 has focus: it groups with the window to its left, and is active.
    ops.push(Op::GroupWindow(Direction::Left));
    let mut layout = check_ops_with_options(options_for_source("groups-native", GROUPER), ops);
    assert_eq!(groups_of(&layout), [(vec![2, 3], 3)]);

    // 1 groups with the nearest window to its right, and joins its group.
    check_ops_on_layout(
        &mut layout,
        [Op::FocusWindow(1), Op::GroupWindow(Direction::Right)],
    );
    assert_eq!(groups_of(&layout), [(vec![2, 3, 1], 1)]);

    // Nothing is above it.
    check_ops_on_layout(&mut layout, [Op::GroupWindow(Direction::Up)]);
    assert_eq!(groups_of(&layout), [(vec![2, 3, 1], 1)]);

    // Ungrouped, the next window in the order is active.
    check_ops_on_layout(&mut layout, [Op::UngroupWindow]);
    assert_eq!(groups_of(&layout), [(vec![2, 3], 3)]);

    check_ops_on_layout(&mut layout, [Op::UngroupWindow]);
    assert_eq!(groups_of(&layout), [(vec![2, 3], 3)]);
    check_ops_on_layout(&mut layout, [Op::FocusWindow(3), Op::UngroupWindow]);
    assert_eq!(groups_of(&layout), []);

    // The engine heard about all of it.
    let state = crate::layout::tiling::engine::exposed_state().unwrap();
    assert_eq!(state["groups"], serde_json::json!([]));
    assert_eq!(
        state["log"],
        serde_json::json!([
            ["groupadd", [2, 3], 3],
            ["change", [2, 3, 1], 1],
            ["change", [2, 3], 3],
            ["remove"],
        ])
    );
}

#[test]
fn a_closing_window_leaves_its_group() {
    let mut ops = three_windows().to_vec();
    ops.push(Op::LayoutAction("group-all", vec![], None));
    ops.push(Op::CloseWindow(2));
    ops.extend(settle());
    let mut layout = check_ops_with_options(options_for_source("groups-close", GROUPER), ops);
    assert_eq!(groups_of(&layout), [(vec![1, 3], 1)]);

    let mut ops = vec![Op::CloseWindow(1)];
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);
    // Two windows made a group, and one is left.
    assert_eq!(groups_of(&layout), []);

    let state = crate::layout::tiling::engine::exposed_state().unwrap();
    assert_eq!(state["groups"], serde_json::json!([]));
}

#[test]
fn a_window_that_floats_leaves_its_group() {
    let mut ops = three_windows().to_vec();
    ops.push(Op::LayoutAction("group-all", vec![], None));
    ops.push(Op::ToggleWindowFloating { id: Some(2) });
    ops.extend(settle());
    let layout = check_ops_with_options(options_for_source("groups-float", GROUPER), ops);

    assert!(layout.active_workspace().unwrap().is_floating(&2));
    assert_eq!(groups_of(&layout), [(vec![1, 3], 1)]);
}

#[test]
fn the_engine_can_dissolve_a_group() {
    let mut ops = three_windows().to_vec();
    ops.push(Op::LayoutAction("group-first", vec![], None));
    let mut layout = check_ops_with_options(options_for_source("groups-dissolve", GROUPER), ops);
    assert_eq!(groups_of(&layout), [(vec![1, 2], 2)]);

    check_ops_on_layout(&mut layout, [Op::LayoutAction("dissolve", vec![], None)]);
    assert_eq!(groups_of(&layout), []);

    let state = crate::layout::tiling::engine::exposed_state().unwrap();
    assert_eq!(
        state["log"],
        serde_json::json!([["groupadd", [1, 2], 2], ["remove"]])
    );
}

#[test]
fn an_engine_action_can_be_for_another_window_than_the_focused_one() {
    let mut ops = three_windows().to_vec();
    ops.push(Op::LayoutAction("mark", vec![], None));
    ops.push(Op::LayoutAction("mark", vec![], Some(1)));
    let layout = check_ops_with_options(options_for_source("groups-target", GROUPER), ops);
    drop(layout);

    let state = crate::layout::tiling::engine::exposed_state().unwrap();
    assert_eq!(state["log"], serde_json::json!([["mark", 3], ["mark", 1]]));
}

#[test]
fn a_move_off_a_workspace_that_goes_away_reaches_the_engine() {
    // Window 1 is alone on the first workspace, which isn't shown, so moving it away empties
    // the workspace, and bento removes it in the same call.
    let ops = [
        Op::AddOutput(1),
        add_window(1),
        Op::FocusWorkspaceDown,
        Op::CompleteAnimations,
        add_window(2),
        Op::MoveWindowToWorkspace {
            window_id: Some(1),
            workspace_idx: 2,
        },
    ];
    let layout = check_ops_with_options(options_for_source("gone", RECORDER), ops);
    drop(layout);

    let state = crate::layout::tiling::engine::exposed_state().unwrap();
    let log = state["log"].as_array().unwrap();
    assert_eq!(log[2][0], "move", "{log:?}");
    assert_eq!(log[2][1], 1, "{log:?}");
    // The engine knows where window 1 is.
    let windows = state["windows"].as_array().unwrap();
    assert!(windows.iter().any(|w| w[0] == 1), "{windows:?}");
}

#[test]
fn an_engine_that_throws_while_it_starts_falls_back() {
    let source = r#"
        export const meta = { api: 2, name: "broken" };
        export default function create(bento) {
            throw new Error("no");
        }
    "#;
    let ops = [Op::AddOutput(1), add_window(1)];
    let layout = check_ops_with_options(options_for_source("broken", source), ops);
    drop(layout);
    assert_eq!(crate::layout::tiling::engine::name(), "fallback");
}

#[test]
fn an_engine_that_keeps_running_out_of_time_falls_back() {
    let source = r#"
        export const meta = { api: 2, name: "stuck" };
        export default function create(bento) {
            bento.addEventListener("windowadd", () => {
                for (;;) {}
            });
        }
    "#;
    let ops = [
        Op::AddOutput(1),
        add_window(1),
        add_window(2),
        add_window(3),
    ];
    let layout = check_ops_with_options(options_for_source("stuck", source), ops);
    drop(layout);
    assert_eq!(crate::layout::tiling::engine::name(), "fallback");
}

/// An engine that logs swipes and resizes, and takes every resize.
const GESTURES: &str = r#"
export const meta = { api: 2, name: "gestures" };
export default function create(bento) {
    const log = [];
    bento.exposeState(() => log);
    const watch = (ws) => {
        for (const type of ["swipebegin", "swipeend"]) {
            ws.addEventListener(type, () => log.push(type));
        }
    };
    bento.monitors.forEach((m) => m.workspaces.forEach(watch));
    bento.addEventListener("workspaceadd", (e) => watch(e.workspace));
    bento.addEventListener("windowadd", (e) => {
        const win = e.window;
        win.place({ x: 0, y: 0, w: 100, h: 100 });
        win.focus();
        win.addEventListener("resizebegin", () => log.push("resizebegin"));
        win.addEventListener("resizeend", () => log.push("resizeend"));
    });
}
"#;

#[test]
fn a_swipe_that_a_resize_cuts_short_still_ends() {
    let ops = [
        Op::AddOutput(1),
        add_window(1),
        Op::ViewOffsetGestureBegin {
            output_idx: 1,
            workspace_idx: None,
            is_touchpad: true,
        },
        Op::InteractiveResizeBegin {
            window: 1,
            edges: ResizeEdge::RIGHT,
        },
    ];
    let layout = check_ops_with_options(options_for_source("swipes", GESTURES), ops);
    drop(layout);
    let log = crate::layout::tiling::engine::exposed_state().unwrap();
    assert_eq!(
        log,
        serde_json::json!(["swipebegin", "swipeend", "resizebegin"])
    );
}

#[test]
fn a_float_request_says_the_size_of_the_window() {
    let source = r#"
        export const meta = { api: 2, name: "sizes", floating: false };
        export default function create(bento) {
            const sizes = [];
            bento.exposeState(() => sizes);
            bento.addEventListener("windowadd", (e) => {
                e.window.place({ x: 0, y: 0, w: 300, h: 200 });
                e.window.addEventListener("floatrequest", (f) => sizes.push(f.size));
            });
        }
    "#;
    let ops = [
        Op::AddOutput(1),
        add_window(1),
        Op::Communicate(1),
        Op::ToggleWindowFloating { id: Some(1) },
    ];
    let layout = check_ops_with_options(options_for_source("sizes", source), ops);
    drop(layout);
    let sizes = crate::layout::tiling::engine::exposed_state().unwrap();
    assert_eq!(sizes, serde_json::json!([{ "w": 300, "h": 200 }]));
}

use crate::layout::search::Edit;

/// An engine that places windows side by side, searches on request, and tells what it saw through
/// `bento.exposeState`. Windows go by the order the engine saw them.
const SEARCHER: &str = r#"
export const meta = {
    api: 2,
    name: "searcher",
    actions: {
        "group-first-three": { description: "Groups the first three windows." },
        "dissolve": { description: "Dissolves the first group." },
        "search-group": { description: "Searches the first group." },
        "search-workspace": { description: "Searches the first monitor's active workspace." },
        "search-all": { description: "Searches every window." },
        "cancel-search": { description: "Cancels the search." },
        "hang": { description: "Never returns." },
    },
};
export default function create(bento) {
    const log = [];
    const seen = new Map();
    const n = (win) => seen.get(win.id);
    let field = null;
    bento.exposeState(() => {
        const search = bento.searching;
        return {
            log,
            searching: search !== null,
            query: search?.query ?? null,
            matches: search ? search.matches.map(n) : null,
            fieldAttached: field !== null && field.parent !== null,
        };
    });
    bento.addEventListener("windowadd", (e) => {
        const win = e.window;
        seen.set(win.id, seen.size + 1);
        const x = (win.workspace.windows.length - 1) * 110;
        win.place({ x, y: 0, w: 100, h: 100 });
        win.focus();
        win.addEventListener("activaterequest", () => log.push(["activate", n(win)]));
    });
    const start = (scope) => {
        const search = bento.search(scope);
        if (search === null) {
            log.push(["refused"]);
            return;
        }
        field = search.field;
        bento.monitors[0].activeWorkspace.tree.append(field);
        field.set({ rect: { x: 0, y: 200, w: 300, h: 30 }, placeholder: "Search" });
        search.addEventListener("change", () => log.push(["change", search.query, search.matches.map(n)]));
        search.addEventListener("end", (e) => log.push(["end", e.picked ? n(e.picked) : null]));
    };
    bento.addEventListener("action", (e) => {
        if (e.name === "group-first-three") {
            bento.createGroup(bento.windows.slice(0, 3));
        } else if (e.name === "dissolve") {
            bento.groups[0].dissolve();
        } else if (e.name === "search-group") {
            start(bento.groups[0]);
        } else if (e.name === "search-workspace") {
            start(bento.monitors[0].activeWorkspace);
        } else if (e.name === "search-all") {
            start(null);
        } else if (e.name === "cancel-search") {
            bento.searching.cancel();
        } else if (e.name === "hang") {
            for (;;) {}
        }
    });
}
"#;

fn titled(id: usize, title: &str) -> Op {
    Op::AddWindow {
        params: TestWindowParams {
            title: Some(title.to_owned()),
            ..TestWindowParams::new(id)
        },
    }
}

fn typing(text: &str) -> Op {
    Op::SearchEdit(Edit::Insert(text.to_owned()))
}

fn action(name: &'static str) -> Op {
    Op::LayoutAction(name, vec![], None)
}

/// A group of the first three windows, whose titles are "Firefox", "Terminal" and "Fire fox",
/// and a fourth window outside it, "Firewall", with a search over the group started.
fn group_search(name: &str) -> Layout<TestWindow> {
    let ops = [
        Op::AddOutput(1),
        titled(1, "Firefox"),
        titled(2, "Terminal"),
        titled(3, "Fire fox"),
        titled(4, "Firewall"),
        action("group-first-three"),
        action("search-group"),
    ];
    check_ops_with_options(options_for_source(name, SEARCHER), ops)
}

fn engine_state() -> serde_json::Value {
    crate::layout::tiling::engine::exposed_state().unwrap()
}

fn focused_window(layout: &Layout<TestWindow>) -> Option<usize> {
    layout.focus().map(|win| win.0.id)
}

#[test]
fn a_search_over_a_group_matches_its_windows_best_match_first() {
    let mut layout = group_search("search-group");
    assert!(layout.search_active());

    // Nothing is typed, so the group's windows match, in the engine's order: "Firewall" isn't in
    // the group. The engine isn't told of a change, since it knew that already.
    let state = engine_state();
    assert_eq!(state["searching"], true);
    assert_eq!(state["query"], "");
    assert_eq!(state["matches"], serde_json::json!([1, 2, 3]));
    assert_eq!(state["log"], serde_json::json!([]));
    assert_eq!(state["fieldAttached"], true);

    // "Fire fox" has the whole word, "Firefox" only starts with it, and "Terminal" drops out.
    check_ops_on_layout(&mut layout, [typing("fire")]);
    let state = engine_state();
    assert_eq!(state["query"], "fire");
    assert_eq!(state["matches"], serde_json::json!([3, 1]));

    // Matches that rank the same stay in the engine's order.
    check_ops_on_layout(&mut layout, [Op::SearchEdit(Edit::Backspace)]);
    assert_eq!(engine_state()["matches"], serde_json::json!([1, 3]));

    check_ops_on_layout(&mut layout, [typing("e")]);
    assert_eq!(engine_state()["matches"], serde_json::json!([3, 1]));
    assert_eq!(
        engine_state()["log"],
        serde_json::json!([
            ["change", "fire", [3, 1]],
            ["change", "fir", [1, 3]],
            ["change", "fire", [3, 1]],
        ])
    );

    check_ops_on_layout(&mut layout, [typing("z")]);
    assert_eq!(engine_state()["matches"], serde_json::json!([]));
}

#[test]
fn enter_picks_the_best_match_and_the_engine_is_asked_to_activate_it() {
    let mut layout = group_search("search-enter");
    // The last window opened has focus.
    assert_eq!(focused_window(&layout), Some(4));

    check_ops_on_layout(&mut layout, [typing("fire"), Op::SearchSubmit]);

    assert!(!layout.search_active());
    assert_eq!(focused_window(&layout), Some(3));
    let state = engine_state();
    assert_eq!(
        state["log"],
        serde_json::json!([["change", "fire", [3, 1]], ["end", 3], ["activate", 3]])
    );
    assert_eq!(state["searching"], false);
    // bento's field doesn't outlive the search.
    assert_eq!(state["fieldAttached"], false);
}

#[test]
fn enter_with_no_match_picks_nothing() {
    let mut layout = group_search("search-no-match");
    check_ops_on_layout(&mut layout, [typing("zzz"), Op::SearchSubmit]);

    assert!(!layout.search_active());
    assert_eq!(focused_window(&layout), Some(4));
    assert_eq!(
        engine_state()["log"],
        serde_json::json!([["change", "zzz", []], ["end", null]])
    );
}

#[test]
fn escape_cancels_the_search_without_picking() {
    let mut layout = group_search("search-escape");
    check_ops_on_layout(&mut layout, [typing("fire"), Op::SearchCancel]);

    assert!(!layout.search_active());
    assert_eq!(focused_window(&layout), Some(4));
    let state = engine_state();
    assert_eq!(
        state["log"],
        serde_json::json!([["change", "fire", [3, 1]], ["end", null]])
    );
    assert_eq!(state["searching"], false);

    // Typing goes nowhere once it's over.
    check_ops_on_layout(&mut layout, [typing("x")]);
    assert_eq!(engine_state()["log"].as_array().unwrap().len(), 2);
}

#[test]
fn the_engine_can_cancel_its_search_and_start_another() {
    let ops = [
        Op::AddOutput(1),
        titled(1, "Firefox"),
        titled(2, "Terminal"),
        action("search-all"),
        // Another search can't start while one runs.
        action("search-all"),
    ];
    let mut layout = check_ops_with_options(options_for_source("search-cancel", SEARCHER), ops);
    assert!(layout.search_active());
    assert_eq!(engine_state()["log"], serde_json::json!([["refused"]]));

    check_ops_on_layout(&mut layout, [typing("term"), action("cancel-search")]);
    assert!(!layout.search_active());
    let state = engine_state();
    assert_eq!(state["searching"], false);
    assert_eq!(state["fieldAttached"], false);
    assert_eq!(
        state["log"],
        serde_json::json!([["refused"], ["change", "term", [2]], ["end", null]])
    );

    // A new search starts over: nothing is typed.
    check_ops_on_layout(&mut layout, [action("search-all")]);
    assert!(layout.search_active());
    assert_eq!(engine_state()["query"], "");
    assert_eq!(engine_state()["matches"], serde_json::json!([1, 2]));
}

#[test]
fn matches_follow_windows_that_open_close_and_change_titles() {
    let ops = [
        Op::AddOutput(1),
        titled(1, "Firefox"),
        titled(2, "Terminal"),
        action("search-all"),
        typing("term"),
        titled(3, "Terminal Two"),
        // It stops matching.
        Op::SetWindowTitle(3, "Notes"),
        Op::Refresh { is_active: true },
        Op::CloseWindow(2),
    ];
    let mut ops = ops.to_vec();
    ops.extend(settle());
    let layout = check_ops_with_options(options_for_source("search-follow", SEARCHER), ops);

    assert!(layout.search_active());
    let state = engine_state();
    assert_eq!(
        state["log"],
        serde_json::json!([
            ["change", "term", [2]],
            ["change", "term", [2, 3]],
            ["change", "term", [2]],
            ["change", "term", []],
        ])
    );
}

#[test]
fn a_search_over_a_workspace_leaves_out_the_windows_on_others() {
    let ops = [
        Op::AddOutput(1),
        titled(1, "Firefox"),
        titled(2, "Firefox private"),
        Op::MoveWindowToWorkspace {
            window_id: Some(2),
            workspace_idx: 1,
        },
        action("search-workspace"),
        typing("fire"),
    ];
    let layout = check_ops_with_options(options_for_source("search-workspace", SEARCHER), ops);
    drop(layout);

    assert_eq!(engine_state()["matches"], serde_json::json!([1]));
}

#[test]
fn a_search_ends_when_its_group_is_gone() {
    let mut layout = group_search("search-group-gone");
    check_ops_on_layout(&mut layout, [action("dissolve")]);

    assert!(!layout.search_active());
    let state = engine_state();
    assert_eq!(state["searching"], false);
    assert_eq!(state["log"], serde_json::json!([["end", null]]));
}

#[test]
fn a_search_ends_when_its_workspace_is_gone() {
    // Window 1 is alone on the first workspace, which is searched. Moving it away empties the
    // workspace while it isn't shown, and bento removes it.
    let ops = [
        Op::AddOutput(1),
        titled(1, "Firefox"),
        action("search-workspace"),
        Op::FocusWorkspaceDown,
        Op::CompleteAnimations,
        titled(2, "Terminal"),
    ];
    let mut layout = check_ops_with_options(options_for_source("search-ws-gone", SEARCHER), ops);
    assert!(layout.search_active());

    check_ops_on_layout(
        &mut layout,
        [Op::MoveWindowToWorkspace {
            window_id: Some(1),
            workspace_idx: 2,
        }],
    );

    assert!(!layout.search_active());
    assert_eq!(engine_state()["log"], serde_json::json!([["end", null]]));
}

#[test]
fn a_search_ends_when_the_engine_starts_over() {
    let mut layout = group_search("search-restart");
    assert!(layout.search_active());

    // An engine that doesn't answer is created again, from where the windows are.
    check_ops_on_layout(&mut layout, [action("hang")]);

    assert!(!layout.search_active());
    assert_eq!(engine_state()["searching"], false);
}
