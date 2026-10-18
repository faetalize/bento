//! Tests for bento's cell grid layout.
//!
//! The test output is 1280 × 720 with 16 px gaps, which makes the grid 22 columns by 12 rows of
//! roughly 41 × 43 px cells. Test windows are 100 px wide, so they open 2 cells wide.

use std::fmt::Write as _;

use insta::assert_snapshot;

use super::*;

/// Tiles in the order they were opened, with their positions in the workspace view.
fn format_tiles(layout: &Layout<TestWindow>) -> String {
    let mut buf = String::new();
    let ws = layout.active_workspace().unwrap();
    let mut tiles: Vec<_> = ws.tiles_with_render_positions().collect();

    // Ids are a global counter, so sort by them but don't print them.
    tiles.sort_by_key(|(tile, _, _)| tile.window().id());
    for (tile, pos, _visible) in tiles {
        let Size { w, h, .. } = tile.tile_size();
        let Point { x, y, .. } = pos;
        writeln!(&mut buf, "{w:>4.0} × {h:>3.0} at x:{x:>4.0} y:{y:>3.0}").unwrap();
    }
    buf
}

fn focused_id(layout: &Layout<TestWindow>) -> usize {
    layout.focus().unwrap().0.id
}

fn settle() -> [Op; 4] {
    [
        Op::Communicate(1),
        Op::Communicate(2),
        Op::Communicate(3),
        Op::CompleteAnimations,
    ]
}

fn add_window(id: usize) -> Op {
    Op::AddWindow {
        params: TestWindowParams::new(id),
    }
}

/// Window 1 full height, window 2 half height on top to its right, with a hole below 2.
fn set_up_hole() -> Layout<TestWindow> {
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        add_window(2),
        Op::SetWindowHeight {
            id: Some(2),
            change: SizeChange::SetProportion(50.),
        },
    ];
    ops.extend(settle());
    check_ops(ops)
}

#[test]
fn new_windows_open_to_the_right() {
    let mut ops = vec![Op::AddOutput(1), add_window(1), add_window(2)];
    ops.extend(settle());
    let layout = check_ops(ops);

    assert_snapshot!(format_tiles(&layout), @"
    99 × 688 at x:  16 y: 16
    99 × 688 at x: 131 y: 16
    ");
}

#[test]
fn shrinking_height_leaves_a_hole() {
    let layout = set_up_hole();
    assert_snapshot!(format_tiles(&layout), @"
    99 × 688 at x:  16 y: 16
    99 × 336 at x: 131 y: 16
    ");
}

#[test]
fn moving_down_into_a_hole_goes_one_cell_at_a_time() {
    let mut layout = set_up_hole();

    let mut ops = vec![Op::MoveWindowDown];
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);
    assert_snapshot!(format_tiles(&layout), @"
    99 × 688 at x:  16 y: 16
    99 × 336 at x: 131 y: 75
    ");

    // Moving past the bottom does nothing.
    let mut ops = vec![Op::MoveWindowDown; 10];
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);
    assert_snapshot!(format_tiles(&layout), @"
    99 × 688 at x:  16 y: 16
    99 × 336 at x: 131 y:368
    ");
}

#[test]
fn moving_into_a_window_swaps() {
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        add_window(2),
        Op::MoveColumnLeft,
    ];
    ops.extend(settle());
    let layout = check_ops(ops);

    // Window 2 is now on the left.
    assert_snapshot!(format_tiles(&layout), @"
    99 × 688 at x: 131 y: 16
    99 × 688 at x:  16 y: 16
    ");
    assert_eq!(focused_id(&layout), 2);
}

#[test]
fn growing_down_pushes_the_window_below() {
    let mut layout = set_up_hole();

    // Put window 3 below window 2 by opening it and moving it down. Left gravity slides it under
    // window 2.
    let mut ops = vec![
        add_window(3),
        Op::SetWindowHeight {
            id: Some(3),
            change: SizeChange::SetProportion(50.),
        },
    ];
    ops.extend(vec![Op::MoveWindowDown; 6]);
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);
    assert_snapshot!(format_tiles(&layout), @"
    99 × 688 at x:  16 y: 16
    99 × 336 at x: 131 y: 16
    99 × 336 at x: 131 y:368
    ");

    // Focus moves geometrically.
    check_ops_on_layout(&mut layout, [Op::FocusWindowUp]);
    assert_eq!(focused_id(&layout), 2);
    check_ops_on_layout(&mut layout, [Op::FocusWindowDown]);
    assert_eq!(focused_id(&layout), 3);
    check_ops_on_layout(&mut layout, [Op::FocusColumnLeft]);
    assert_eq!(focused_id(&layout), 1);

    // Growing 2 down pushes 3, which doesn't fit below anymore and goes to the right.
    let mut ops = vec![Op::SetWindowHeight {
        id: Some(2),
        change: SizeChange::SetProportion(75.),
    }];
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);
    assert_snapshot!(format_tiles(&layout), @"
    99 × 688 at x:  16 y: 16
    99 × 512 at x: 131 y: 16
    99 × 336 at x: 246 y:368
    ");
}

#[test]
fn closing_a_window_collapses_empty_columns() {
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        add_window(2),
        add_window(3),
        Op::CloseWindow(2),
    ];
    ops.extend(settle());
    let layout = check_ops(ops);

    // Window 3 slides into the place of window 2. The view stays on the active window 3, like in
    // niri.
    assert_snapshot!(format_tiles(&layout), @"
    99 × 688 at x: 131 y: 16
    99 × 688 at x: 246 y: 16
    ");
}

#[test]
fn switching_preset_width_snaps_to_cells() {
    let mut ops = vec![Op::AddOutput(1), add_window(1), Op::SwitchPresetColumnWidth];
    ops.extend(settle());
    let mut layout = check_ops(ops);

    // From 2 cells to the next larger preset, 1/3 (7 cells).
    assert_snapshot!(format_tiles(&layout), @" 386 × 688 at x:  16 y: 16");

    let mut ops = vec![Op::SwitchPresetColumnWidth];
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);

    // Then 1/2 (11 cells).
    assert_snapshot!(format_tiles(&layout), @" 616 × 688 at x:  16 y: 16");
}

/// Distance between the left edges of neighboring grid columns in the test output.
const COL_STEP: f64 = (1280. - 16. * 23.) / 22. + 16.;
/// Distance between the top edges of neighboring grid rows in the test output.
const ROW_STEP: f64 = (720. - 16. * 13.) / 12. + 16.;

#[test]
fn dragging_the_right_edge_snaps_and_pushes() {
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        add_window(2),
        Op::InteractiveResizeBegin {
            window: 1,
            edges: ResizeEdge::RIGHT,
        },
        // A bit over 2.5 cells snaps to 3 more cells.
        Op::InteractiveResizeUpdate {
            window: 1,
            dx: COL_STEP * 2.6,
            dy: 0.,
        },
    ];
    ops.extend(settle());
    let mut layout = check_ops(ops);

    // Window 1 is 5 cells wide and pushed window 2 along.
    assert_snapshot!(format_tiles(&layout), @"
    271 × 688 at x:  16 y: 16
     99 × 688 at x: 303 y: 16
    ");

    // Dragging back puts window 2 back where it was.
    let mut ops = vec![
        Op::InteractiveResizeUpdate {
            window: 1,
            dx: COL_STEP * 0.4,
            dy: 0.,
        },
        Op::InteractiveResizeEnd { window: 1 },
    ];
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);
    assert_snapshot!(format_tiles(&layout), @"
    99 × 688 at x:  16 y: 16
    99 × 688 at x: 131 y: 16
    ");
}

#[test]
fn dragging_the_bottom_edge_leaves_a_hole() {
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        Op::InteractiveResizeBegin {
            window: 1,
            edges: ResizeEdge::BOTTOM,
        },
        Op::InteractiveResizeUpdate {
            window: 1,
            dx: 0.,
            dy: -ROW_STEP * 4.,
        },
        Op::InteractiveResizeEnd { window: 1 },
    ];
    ops.extend(settle());
    let layout = check_ops(ops);

    // 8 rows tall.
    assert_snapshot!(format_tiles(&layout), @"  99 × 453 at x:  16 y: 16");
}

#[test]
fn dragging_the_left_edge_keeps_the_right_edge_still() {
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        add_window(2),
        Op::InteractiveResizeBegin {
            window: 2,
            edges: ResizeEdge::LEFT,
        },
        Op::InteractiveResizeUpdate {
            window: 2,
            dx: -COL_STEP * 3.,
            dy: 0.,
        },
    ];
    ops.extend(settle());
    let mut layout = check_ops(ops);

    // While dragging, window 2 grows 3 cells to the left, pushing window 1 left, and its right
    // edge stays at x = 230.
    assert_snapshot!(format_tiles(&layout), @"
     99 × 688 at x:-156 y: 16
    271 × 688 at x: -41 y: 16
    ");

    // Ending the resize scrolls the active window into view, like in niri.
    let mut ops = vec![Op::InteractiveResizeEnd { window: 2 }];
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);
    assert_snapshot!(format_tiles(&layout), @"
     99 × 688 at x: -99 y: 16
    271 × 688 at x:  16 y: 16
    ");
}

#[test]
fn dropping_a_window_into_a_cell() {
    // Window 1 full height, window 2 on top to its right, window 3 half height further right.
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        add_window(2),
        Op::SetWindowHeight {
            id: Some(2),
            change: SizeChange::SetProportion(50.),
        },
        add_window(3),
        Op::SetWindowHeight {
            id: Some(3),
            change: SizeChange::SetProportion(50.),
        },
    ];
    ops.extend(settle());
    let mut layout = check_ops(ops);
    assert_snapshot!(format_tiles(&layout), @"
    99 × 688 at x:  16 y: 16
    99 × 336 at x: 131 y: 16
    99 × 336 at x: 246 y: 16
    ");

    // Grab window 3 near its top-left corner and drop it into the hole under window 2.
    let start: Point<f64, Logical> = Point::from((16. + 4. * COL_STEP + 4., 16. + 4.));
    let end: Point<f64, Logical> =
        Point::from((16. + 2. * COL_STEP + 4., 16. + 6. * ROW_STEP + 4.));
    let mut ops = vec![
        Op::InteractiveMoveBegin {
            window: 3,
            output_idx: 1,
            px: start.x,
            py: start.y,
        },
        Op::InteractiveMoveUpdate {
            window: 3,
            dx: end.x - start.x,
            dy: end.y - start.y,
            output_idx: 1,
            px: end.x,
            py: end.y,
        },
        Op::InteractiveMoveEnd { window: 3 },
    ];
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);
    assert_snapshot!(format_tiles(&layout), @"
    99 × 688 at x:  16 y: 16
    99 × 336 at x: 131 y: 16
    99 × 336 at x: 131 y:368
    ");
}
