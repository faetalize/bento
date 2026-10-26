use std::fmt::Write as _;

use insta::assert_snapshot;
use niri_config::animations::{Curve, EasingParams, Kind};

use super::*;

fn format_tiles(layout: &Layout<TestWindow>) -> String {
    let mut buf = String::new();
    let ws = layout.active_workspace().unwrap();
    let mut tiles: Vec<_> = ws.tiles_with_render_positions().collect();

    // We sort by id since that gives us a consistent order (from first opened to last), but we
    // don't print the id since it's nondeterministic (the id is a global counter across all
    // running tests in the same binary).
    tiles.sort_by_key(|(tile, _, _)| tile.window().id());
    for (tile, pos, _visible) in tiles {
        let Size { w, h, .. } = tile.animated_tile_size();
        let Point { x, y, .. } = pos;
        writeln!(&mut buf, "{w:>3.0} × {h:>3.0} at x:{x:>3.0} y:{y:>3.0}").unwrap();
    }
    buf
}

fn make_options() -> Options {
    const LINEAR: Kind = Kind::Easing(EasingParams {
        duration_ms: 1000,
        curve: Curve::Linear,
    });

    let mut options = Options {
        layout: niri_config::Layout {
            gaps: 0.0,
            ..Default::default()
        },
        ..Options::default()
    };
    options.animations.window_resize.anim.kind = LINEAR;
    options.animations.window_movement.0.kind = LINEAR;

    options
}

fn set_up_two_in_column() -> Layout<TestWindow> {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::ConsumeWindowIntoColumn,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];

    check_ops_with_options(make_options(), ops)
}

#[test]
fn height_resize_animates_next_y() {
    let mut layout = set_up_two_in_column();

    let ops = [
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::AdjustFixed(-50),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 50)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // No time had passed yet, so we're at the initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");

    // Advance the time halfway.
    Op::AdvanceAnimations { msec_delta: 500 }.apply(&mut layout);

    // Top window is half-resized at 75 px tall, bottom window is at y=75 matching it.
    assert_snapshot!(format_tiles(&layout), @r"
    100 ×  75 at x:  0 y:  0
    200 × 200 at x:  0 y: 75
    ");

    // Advance the time to completion.
    Op::AdvanceAnimations { msec_delta: 500 }.apply(&mut layout);

    // Final state at 50 px.
    assert_snapshot!(format_tiles(&layout), @r"
    100 ×  50 at x:  0 y:  0
    200 × 200 at x:  0 y: 50
    ");
}

#[test]
fn height_resize_and_back() {
    let mut layout = set_up_two_in_column();

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");

    let ops = [
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The top window grows in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time halfway.
        Op::AdvanceAnimations { msec_delta: 500 },
    ];
    check_ops_on_layout(&mut layout, ops);

    // Top window is half-resized at 150 px tall, bottom window is at y=150 matching it.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 150 at x:  0 y:  0
    200 × 200 at x:  0 y:150
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This starts a new resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // No time had passed yet, and we expect no animation jumps, so this state matches the last.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 150 at x:  0 y:  0
    200 × 200 at x:  0 y:150
    ");

    // Advance the time halfway.
    Op::AdvanceAnimations { msec_delta: 500 }.apply(&mut layout);

    // Halfway through at 125px.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 125 at x:  0 y:  0
    200 × 200 at x:  0 y:125
    ");

    // Advance the time to completion.
    Op::AdvanceAnimations { msec_delta: 500 }.apply(&mut layout);

    // Final state back at 100px.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");
}

#[test]
fn height_resize_and_cancel() {
    let mut layout = set_up_two_in_column();

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");

    let ops = [
        // Issue a resize.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The top window grows in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 200)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time slightly.
        Op::AdvanceAnimations { msec_delta: 50 },
    ];
    check_ops_on_layout(&mut layout, ops);

    // Top window is half-resized at 105 px tall, bottom window is at y=105 matching it.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 105 at x:  0 y:  0
    200 × 200 at x:  0 y:105
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowHeight {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response, the bottom remains as is.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This cancels the resize animation since the change of 5 px is less than the resize
        // animation threshold.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // Since the resize animation is cancelled, the height goes to the new value immediately. The Y
    // position doesn't jump, instead the animation is offset to preserve the current position.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:105
    ");

    // Advance to the end of the move animation.
    Op::AdvanceAnimations { msec_delta: 950 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:  0 y:100
    ");
}

#[test]
fn width_resize_and_cancel() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::FocusColumnLeft,
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];
    let mut layout = check_ops_with_options(make_options(), ops);

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    let ops = [
        // Issue a resize.
        Op::SetWindowWidth {
            id: None,
            change: SizeChange::SetFixed(200),
        },
        // The left window grows in response.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(200, 100)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time slightly.
        Op::AdvanceAnimations { msec_delta: 50 },
    ];
    check_ops_on_layout(&mut layout, ops);

    // Left window is half-resized at 105 px wide, right window is at x=105 matching it.
    assert_snapshot!(format_tiles(&layout), @r"
    105 × 100 at x:  0 y:  0
    200 × 200 at x:105 y:  0
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowWidth {
            id: None,
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This cancels the resize animation since the change of 5 px is less than the resize
        // animation threshold.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // Since the resize animation is cancelled, the width goes to the new value immediately. The X
    // position doesn't jump, instead the animation is restarted to preserve the current position.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:105 y:  0
    ");

    // Advance to the end of the move animation.
    Op::AdvanceAnimations { msec_delta: 1000 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");
}

#[test]
fn width_resize_and_cancel_of_column_to_the_left() {
    let ops = [
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::AddWindow {
            params: TestWindowParams::new(2),
        },
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        Op::SetForcedSize {
            id: 2,
            size: Some(Size::new(200, 200)),
        },
        Op::Communicate(1),
        Op::Communicate(2),
        Op::CompleteAnimations,
    ];
    let mut layout = check_ops_with_options(make_options(), ops);

    // The initial state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");

    let ops = [
        // Issue a resize.
        Op::SetWindowWidth {
            id: Some(1),
            change: SizeChange::SetFixed(200),
        },
        // The left window grows in response.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(200, 100)),
        },
        // This starts the resize animation.
        Op::Communicate(1),
        Op::Communicate(2),
        // Advance the time slightly.
        Op::AdvanceAnimations { msec_delta: 50 },
    ];
    check_ops_on_layout(&mut layout, ops);

    // Left window is half-resized at 105 px wide, it's at x=-5 matching the right edge position.
    assert_snapshot!(format_tiles(&layout), @r"
    105 × 100 at x: -5 y:  0
    200 × 200 at x:100 y:  0
    ");

    let ops = [
        // Issue a resize back.
        Op::SetWindowWidth {
            id: Some(1),
            change: SizeChange::SetFixed(100),
        },
        // The top window shrinks in response.
        Op::SetForcedSize {
            id: 1,
            size: Some(Size::new(100, 100)),
        },
        // This cancels the resize animation since the change of 5 px is less than the resize
        // animation threshold.
        Op::Communicate(1),
        Op::Communicate(2),
    ];
    check_ops_on_layout(&mut layout, ops);

    // Since the resize animation is cancelled, the width goes to the new value immediately. The X
    // position doesn't jump, instead the animation is restarted to preserve the current position.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x: -5 y:  0
    200 × 200 at x:100 y:  0
    ");

    // Advance to the end of the move animation.
    Op::AdvanceAnimations { msec_delta: 1000 }.apply(&mut layout);

    // Final state.
    assert_snapshot!(format_tiles(&layout), @r"
    100 × 100 at x:  0 y:  0
    200 × 200 at x:100 y:  0
    ");
}
