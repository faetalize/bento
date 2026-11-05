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
            },
            windowRules: {
                "width": { type: "size" },
                "open-maximized": { type: "boolean", default: false },
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
        })
    );

    let rules = EngineRules(vec![
        (String::from("width"), strings(&["67%"])),
        (
            String::from("open-maximized"),
            vec![TilingOption::Bool(true)],
        ),
        (String::from("nope"), vec![TilingOption::Bool(true)]),
    ]);
    // Only what the rules set, and what the engine declares.
    assert_eq!(
        engine::rules_json(&rules),
        json!({ "width": { "proportion": 0.67 }, "open-maximized": true })
    );
    assert_eq!(engine::rules_json(&EngineRules::default()), json!({}));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn engine_options_and_rules_are_checked_against_the_running_engine() {
    use niri_config::{EngineRules, EngineWindowRule, TilingOption};

    use crate::layout::tiling::engine;

    let (dir, layout) = typed_engine("check");
    let mut config = niri_config::Config::default();
    config.tiling = niri_config::Tiling {
        layout,
        options: vec![
            (String::from("nope"), vec![TilingOption::Bool(true)]),
            (String::from("center"), strings(&["sideways"])),
            (String::from("width"), strings(&["+10%"])),
            (String::from("presets"), vec![TilingOption::Bool(true)]),
            (String::from("gap"), vec![]),
            (String::from("flag"), vec![TilingOption::Bool(true)]),
        ],
        window_rules: vec![EngineWindowRule {
            matches: Vec::new(),
            excludes: Vec::new(),
            properties: EngineRules(vec![
                (String::from("width"), strings(&["50%"])),
                (String::from("open-maximized"), strings(&["yes"])),
                (String::from("zzz"), vec![TilingOption::Bool(true)]),
            ]),
        }],
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
            r#"layout engine window rule "open-maximized" must be true or false"#,
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
