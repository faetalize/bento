//! Tests for the tiling host: mostly with the built-in fallback layout, which puts windows side by
//! side, each half the working area wide, and a few with the shipped layouts, to check that they
//! run in the engine.
//!
//! The test output is 1280 × 720 with 16 px gaps.

use std::fmt::Write as _;

use insta::assert_snapshot;

use super::*;

fn options() -> Options {
    options_for("fallback")
}

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
    let mut layout = check_ops_with_options(options(), ops);

    // The third window is off to the right, so the camera followed it.
    assert_snapshot!(format_tiles(&layout), @"
    616 × 688 at x:-1248 y: 16
    616 × 688 at x: -616 y: 16
    616 × 688 at x:   16 y: 16 focused
    ");

    // Focusing left goes through the layout's action.
    let mut ops = vec![Op::FocusColumnLeft];
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);
    assert_snapshot!(format_tiles(&layout), @"
    616 × 688 at x: -616 y: 16
    616 × 688 at x:   16 y: 16 focused
    616 × 688 at x:  648 y: 16
    ");

    // Closing the focused window closes the gap and focuses the next one.
    let mut ops = vec![Op::CloseWindow(2)];
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
        Op::FocusColumnLeft,
        Op::ConsumeOrExpelWindowLeft { id: None },
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
    ops.extend([
        Op::SetWindowHeight {
            id: Some(2),
            change: SizeChange::SetProportion(50.),
        },
        Op::SetWindowHeight {
            id: Some(3),
            change: SizeChange::SetProportion(50.),
        },
    ]);
    ops.extend(settle());
    let layout = check_ops_with_options(options_for("bento"), ops);
    assert_snapshot!(format_tiles(&layout), @"
      99 × 688 at x:   16 y: 16
      99 × 336 at x:  131 y: 16
      99 × 336 at x:  246 y: 16 focused
    ");
}
