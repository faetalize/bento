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

    // Window 3 slides into the place of window 2, and the view stays still.
    assert_snapshot!(format_tiles(&layout), @"
    99 × 688 at x:  16 y: 16
    99 × 688 at x: 131 y: 16
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

#[test]
fn dragging_the_bottom_window_of_a_stack_keeps_the_view_still() {
    // Window 2 under window 1, focused.
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        Op::SetWindowHeight {
            id: Some(1),
            change: SizeChange::SetProportion(50.),
        },
        add_window(2),
        Op::SetWindowHeight {
            id: Some(2),
            change: SizeChange::SetProportion(50.),
        },
    ];
    ops.extend(std::iter::repeat_n(Op::MoveWindowDown, 6));
    ops.push(Op::MoveColumnLeft);
    ops.extend(settle());
    let mut layout = check_ops(ops);
    assert_eq!(focused_id(&layout), 2);
    assert_snapshot!(format_tiles(&layout), @"
    99 × 336 at x:  16 y: 16
    99 × 336 at x:  16 y:368
    ");

    // Drag window 2 far enough to lift it out of the layout.
    let start: Point<f64, Logical> = Point::from((0., 0.));
    let ops = [
        Op::InteractiveMoveBegin {
            window: 2,
            output_idx: 1,
            px: start.x,
            py: start.y,
        },
        Op::InteractiveMoveUpdate {
            window: 2,
            dx: 300.,
            dy: 0.,
            output_idx: 1,
            px: start.x + 300.,
            py: start.y,
        },
    ];
    check_ops_on_layout(&mut layout, ops);

    // Window 1 hasn't jumped.
    assert_snapshot!(format_tiles(&layout), @"
    99 × 336 at x:  16 y: 16
    ");
}

#[test]
fn dragging_a_window_that_fell_left_keeps_the_view_still() {
    // Windows 1 and 2 on the top half, then window 3 opens right of window 2, and falls under
    // window 1 when moved to the bottom half.
    let half = || SizeChange::SetProportion(50.);
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        Op::SetWindowHeight {
            id: Some(1),
            change: half(),
        },
        add_window(2),
        Op::SetWindowHeight {
            id: Some(2),
            change: half(),
        },
    ];
    ops.extend(settle());
    ops.extend([
        add_window(3),
        Op::SetWindowHeight {
            id: Some(3),
            change: half(),
        },
    ]);
    ops.extend(std::iter::repeat_n(Op::MoveWindowDown, 6));
    ops.extend(settle());
    let mut layout = check_ops(ops);
    assert_eq!(focused_id(&layout), 3);
    assert_snapshot!(format_tiles(&layout), @"
    99 × 336 at x:  16 y: 16
    99 × 336 at x: 131 y: 16
    99 × 336 at x:  16 y:368
    ");

    // Drag window 3 far enough to lift it out of the layout.
    let mut ops = vec![
        Op::InteractiveMoveBegin {
            window: 3,
            output_idx: 1,
            px: 0.,
            py: 0.,
        },
        Op::InteractiveMoveUpdate {
            window: 3,
            dx: 300.,
            dy: 0.,
            output_idx: 1,
            px: 300.,
            py: 0.,
        },
    ];
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);

    // Windows 1 and 2 stay where they were.
    assert_snapshot!(format_tiles(&layout), @"
    99 × 336 at x:  16 y: 16
    99 × 336 at x: 131 y: 16
    ");
}

/// Windows 1 and 2 on the top half, window 3 in the bottom half under window 1.
fn set_up_two_over_one() -> Vec<Op> {
    let half = || SizeChange::SetProportion(50.);
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        Op::SetWindowHeight {
            id: Some(1),
            change: half(),
        },
        add_window(2),
        Op::SetWindowHeight {
            id: Some(2),
            change: half(),
        },
        add_window(3),
        Op::SetWindowHeight {
            id: Some(3),
            change: half(),
        },
    ];
    ops.extend(std::iter::repeat_n(Op::MoveWindowDown, 6));
    ops.extend(settle());
    ops
}

#[test]
fn closing_a_window_keeps_focus_on_another() {
    let mut ops = set_up_two_over_one();
    ops.push(Op::FocusWindow(2));
    ops.extend(settle());
    let mut layout = check_ops(ops);
    assert_snapshot!(format_tiles(&layout), @"
    99 × 336 at x:  16 y: 16
    99 × 336 at x: 131 y: 16
    99 × 336 at x:  16 y:368
    ");

    // Closing window 1 lets window 2 slide left, which reorders the windows.
    let mut ops = vec![Op::CloseWindow(1)];
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);
    assert_eq!(focused_id(&layout), 2);
    assert_snapshot!(format_tiles(&layout), @"
    99 × 336 at x:  16 y: 16
    99 × 336 at x:  16 y:368
    ");
}

#[test]
fn closing_the_focused_window_focuses_the_one_above() {
    let mut ops = set_up_two_over_one();
    // Focus away and back so that closing window 3 doesn't go back to window 2 it was opened from.
    ops.extend([Op::FocusWindow(1), Op::FocusWindow(3), Op::CloseWindow(3)]);
    ops.extend(settle());
    let layout = check_ops(ops);
    assert_eq!(focused_id(&layout), 1);
}

#[test]
fn closing_a_new_window_goes_back_to_the_previous_one() {
    // Window 1 on top of window 2, both on the left.
    let half = || SizeChange::SetProportion(50.);
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        Op::SetWindowHeight {
            id: Some(1),
            change: half(),
        },
        add_window(2),
        Op::SetWindowHeight {
            id: Some(2),
            change: half(),
        },
    ];
    ops.extend(std::iter::repeat_n(Op::MoveWindowDown, 6));
    ops.push(Op::MoveColumnLeft);
    ops.push(Op::FocusWindow(1));
    ops.extend(settle());
    let mut layout = check_ops(ops);
    assert_snapshot!(format_tiles(&layout), @"
    99 × 336 at x:  16 y: 16
    99 × 336 at x:  16 y:368
    ");

    // Window 3 opens right of window 1, after window 2 in the order.
    let mut ops = vec![add_window(3)];
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);
    assert_eq!(focused_id(&layout), 3);

    let mut ops = vec![Op::CloseWindow(3)];
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);
    assert_eq!(focused_id(&layout), 1);
}

#[test]
fn swiping_snaps_to_window_edges_with_stacked_windows() {
    // Windows 1 and 2 stacked on the left, then windows 3 and 4, all half the view wide.
    let half = || SizeChange::SetProportion(50.);
    let mut ops = vec![Op::AddOutput(1)];
    for id in 1..=4 {
        ops.extend([
            add_window(id),
            Op::SetWindowWidth {
                id: Some(id),
                change: half(),
            },
        ]);
    }
    ops.extend([
        Op::SetWindowHeight {
            id: Some(1),
            change: half(),
        },
        Op::SetWindowHeight {
            id: Some(2),
            change: half(),
        },
        Op::FocusWindow(2),
    ]);
    ops.extend(std::iter::repeat_n(Op::MoveWindowDown, 6));
    ops.extend([Op::MoveColumnLeft, Op::FocusWindow(1)]);
    ops.extend(settle());
    let mut layout = check_ops(ops);
    assert_snapshot!(format_tiles(&layout), @"
    616 × 336 at x:  16 y: 16
    616 × 336 at x:  16 y:368
    616 × 688 at x: 648 y: 16
    616 × 688 at x:1280 y: 16
    ");

    // Swipe right by a bit over one window.
    let mut ops = vec![
        Op::ViewOffsetGestureBegin {
            output_idx: 1,
            workspace_idx: None,
            is_touchpad: false,
        },
        Op::ViewOffsetGestureUpdate {
            delta: 700.,
            timestamp: Duration::from_millis(100),
            is_touchpad: false,
        },
        Op::ViewOffsetGestureEnd {
            is_touchpad: Some(false),
        },
    ];
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);

    // The view snaps to window 3 on the left and window 4 on the right, focusing window 4 as the
    // furthest in the direction of the swipe.
    assert_eq!(focused_id(&layout), 4);
    assert_snapshot!(format_tiles(&layout), @"
    616 × 336 at x:-616 y: 16
    616 × 336 at x:-616 y:368
    616 × 688 at x:  16 y: 16
    616 × 688 at x: 648 y: 16
    ");
}

/// Two windows of the given width stacked on the left, then a full-height window on the right,
/// focused.
fn set_up_stack_and_one(width: f64) -> Vec<Op> {
    let mut ops = vec![Op::AddOutput(1)];
    for id in 1..=3 {
        ops.extend([
            add_window(id),
            Op::SetWindowWidth {
                id: Some(id),
                change: SizeChange::SetProportion(width),
            },
        ]);
    }
    for id in 1..=2 {
        ops.push(Op::SetWindowHeight {
            id: Some(id),
            change: SizeChange::SetProportion(50.),
        });
    }
    ops.push(Op::FocusWindow(2));
    ops.extend(std::iter::repeat_n(Op::MoveWindowDown, 6));
    ops.extend([Op::MoveColumnLeft, Op::FocusWindow(3)]);
    ops.extend(settle());
    ops
}

#[test]
fn stacked_windows_count_as_a_single_column_for_centering() {
    let options = Options {
        layout: niri_config::Layout {
            always_center_single_column: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let half = || SizeChange::SetProportion(50.);
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        Op::SetWindowHeight {
            id: Some(1),
            change: half(),
        },
        add_window(2),
        Op::SetWindowHeight {
            id: Some(2),
            change: half(),
        },
    ];
    ops.extend(std::iter::repeat_n(Op::MoveWindowDown, 6));
    ops.push(Op::MoveColumnLeft);
    ops.extend(settle());
    let layout = check_ops_with_options(options, ops);
    assert_snapshot!(format_tiles(&layout), @"
    99 × 336 at x: 591 y: 16
    99 × 336 at x: 591 y:368
    ");
}

#[test]
fn centering_visible_columns_uses_their_span_on_the_grid() {
    let mut ops = set_up_stack_and_one(20.);
    ops.push(Op::CenterVisibleColumns);
    ops.extend(settle());
    let layout = check_ops(ops);
    assert_snapshot!(format_tiles(&layout), @"
    214 × 336 at x: 418 y: 16
    214 × 336 at x: 418 y:368
    214 × 688 at x: 648 y: 16
    ");
}

#[test]
fn center_on_overflow_compares_with_the_window_beside() {
    let options = Options {
        layout: niri_config::Layout {
            center_focused_column: CenterFocusedColumn::OnOverflow,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut ops = set_up_stack_and_one(60.);
    ops.push(Op::FocusColumnLeft);
    ops.extend(settle());
    let layout = check_ops_with_options(options, ops);
    assert_eq!(focused_id(&layout), 1);
    // Window 1 and window 3 beside it don't fit together, so window 1 is centered. Window 2 under
    // it is in the same column of cells and doesn't count.
    assert_snapshot!(format_tiles(&layout), @"
    731 × 336 at x: 275 y: 16
    731 × 336 at x: 275 y:368
    731 × 688 at x:1022 y: 16
    ");
}

#[test]
fn moving_a_resized_window_to_another_workspace_keeps_its_size() {
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        Op::SetWindowWidth {
            id: Some(1),
            change: SizeChange::SetProportion(33.),
        },
        Op::SetWindowHeight {
            id: Some(1),
            change: SizeChange::SetProportion(50.),
        },
        Op::MoveWindowToWorkspaceDown(true),
    ];
    ops.extend(settle());
    let layout = check_ops(ops);
    assert_snapshot!(format_tiles(&layout), @"
    386 × 336 at x:  16 y: 16
    ");
}

#[test]
fn toggling_full_width_restores_the_previous_width() {
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        Op::SetWindowWidth {
            id: Some(1),
            change: SizeChange::SetProportion(33.),
        },
        Op::MaximizeColumn,
    ];
    ops.extend(settle());
    let mut layout = check_ops(ops);
    assert_snapshot!(format_tiles(&layout), @"
    1248 × 688 at x:  16 y: 16
    ");

    let mut ops = vec![Op::MaximizeColumn];
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);
    assert_snapshot!(format_tiles(&layout), @"
    386 × 688 at x:  16 y: 16
    ");
}

#[test]
fn adjusting_width_goes_by_the_requested_amount() {
    // 10% of 22 columns is 2 columns.
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        Op::SetWindowWidth {
            id: Some(1),
            change: SizeChange::AdjustProportion(10.),
        },
    ];
    ops.extend(settle());
    let layout = check_ops(ops);
    assert_snapshot!(format_tiles(&layout), @"
    214 × 688 at x:  16 y: 16
    ");
}

#[test]
fn floating_and_tiling_again_keeps_the_size_and_place() {
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        add_window(2),
        Op::SetWindowWidth {
            id: Some(2),
            change: SizeChange::SetProportion(33.),
        },
        Op::SetWindowHeight {
            id: Some(2),
            change: SizeChange::SetProportion(50.),
        },
    ];
    ops.extend(settle());
    let mut layout = check_ops(ops);
    assert_snapshot!(format_tiles(&layout), @"
     99 × 688 at x:  16 y: 16
    386 × 336 at x: 131 y: 16
    ");

    let mut ops = vec![
        Op::ToggleWindowFloating { id: Some(2) },
        Op::ToggleWindowFloating { id: Some(2) },
    ];
    ops.extend(settle());
    check_ops_on_layout(&mut layout, ops);
    assert_snapshot!(format_tiles(&layout), @"
     99 × 688 at x:  16 y: 16
    386 × 336 at x: 131 y: 16
    ");
}

#[test]
fn default_window_height_opens_next_to_the_active_window() {
    // Window 1 in the bottom half.
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        Op::SetWindowHeight {
            id: Some(1),
            change: SizeChange::SetProportion(50.),
        },
    ];
    ops.extend(std::iter::repeat_n(Op::MoveWindowDown, 6));
    ops.extend(settle());
    let mut layout = check_ops(ops);

    // Window 2 opens half height, lined up with window 1.
    layout.add_window(
        TestWindow::new(TestWindowParams::new(2)),
        AddWindowTarget::Auto,
        None,
        Some(PresetSize::Proportion(0.5)),
        false,
        false,
        ActivateWindow::default(),
    );
    check_ops_on_layout(&mut layout, settle());
    assert_snapshot!(format_tiles(&layout), @"
    99 × 336 at x:  16 y:368
    99 × 336 at x: 131 y:368
    ");
}

/// Window 1 full height on the left, window 2 on the top half to its right, and window 3 under
/// window 2.
fn set_up_one_and_stack() -> Vec<Op> {
    let half = || SizeChange::SetProportion(50.);
    let mut ops = vec![
        Op::AddOutput(1),
        add_window(1),
        add_window(2),
        Op::SetWindowHeight {
            id: Some(2),
            change: half(),
        },
        add_window(3),
        Op::SetWindowHeight {
            id: Some(3),
            change: half(),
        },
    ];
    ops.extend(std::iter::repeat_n(Op::MoveWindowDown, 6));
    ops.extend(settle());
    ops
}

#[test]
fn focusing_the_last_column_stays_in_the_row() {
    let mut ops = set_up_one_and_stack();
    ops.extend([Op::FocusWindow(1), Op::FocusColumnLast]);
    let layout = check_ops(ops);
    assert_snapshot!(format_tiles(&layout), @"
    99 × 688 at x:  16 y: 16
    99 × 336 at x: 131 y: 16
    99 × 336 at x: 131 y:368
    ");

    // Window 3 comes last in the order, but window 2 is the last one in the top row.
    assert_eq!(focused_id(&layout), 2);
}

#[test]
fn focusing_the_top_window_goes_up_the_stack() {
    let mut ops = set_up_one_and_stack();
    ops.extend([Op::FocusWindow(3), Op::FocusWindowTop]);
    let layout = check_ops(ops);
    assert_eq!(focused_id(&layout), 2);
}

#[test]
fn focusing_down_or_to_the_workspace_below_skips_windows_not_below() {
    // Window 3 is under window 1, not under window 2.
    let mut ops = set_up_two_over_one();
    ops.extend([Op::FocusWindow(2), Op::FocusWindowOrWorkspaceDown]);
    let layout = check_ops(ops);
    assert!(
        layout.focus().is_none(),
        "should switch to the empty workspace below"
    );
}

#[test]
fn ipc_positions_follow_grid_columns() {
    let layout = check_ops(set_up_two_over_one());
    let ws = layout.active_workspace().unwrap();
    let mut positions: Vec<_> = ws
        .scrolling()
        .tiles_with_ipc_layouts()
        .map(|(tile, layout)| (tile.window().0.id, layout.pos_in_scrolling_layout.unwrap()))
        .collect();
    positions.sort();

    // Windows 1 and 3 are stacked in the first column, window 2 is in the second.
    assert_eq!(positions, [(1, (1, 1)), (2, (2, 1)), (3, (1, 2))]);
}
