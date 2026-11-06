use std::time::Duration;

use niri_config::GradientRelativeTo;
use serde_json::json;

use super::*;

fn tree() -> (Tree, Clock) {
    let clock = Clock::with_time(Duration::ZERO);
    (Tree::new(1, clock.clone(), Animations::default()), clock)
}

fn apply(tree: &mut Tree, ops: Value) {
    tree.apply(ops.as_array().unwrap()).unwrap();
}

fn error(tree: &mut Tree, op: Value) -> String {
    tree.apply(&[op]).unwrap_err()
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rectangle<f64, Logical> {
    Rectangle::new(Point::from((x, y)), Size::from((w, h)))
}

fn point(x: f64, y: f64) -> Point<f64, Logical> {
    Point::from((x, y))
}

fn props_rect(x: f64, y: f64, w: f64, h: f64) -> Value {
    json!({ "x": x, "y": y, "w": w, "h": h })
}

/// Creates a box with `props` and appends it to `parent`.
fn add(tree: &mut Tree, id: &str, parent: &str, props: Value) {
    apply(
        tree,
        json!([
            { "op": "create", "id": id, "kind": "box", "props": props },
            { "op": "append", "parent": parent, "child": id },
        ]),
    );
}

fn set(tree: &mut Tree, id: &str, props: Value) {
    apply(tree, json!([{ "op": "set", "id": id, "props": props }]));
}

fn children(tree: &Tree, id: &str) -> Vec<String> {
    tree.nodes[id].children.clone()
}

fn linear(ms: u32) -> Value {
    json!({ "duration": ms, "curve": "linear" })
}

// Ops.

#[test]
fn created_nodes_are_in_the_tree_once_appended() {
    let (mut tree, _) = tree();
    apply(
        &mut tree,
        json!([{ "op": "create", "id": "n1", "kind": "box" }]),
    );
    assert!(tree.path("n1") == ["n1"]);
    assert!(children(&tree, "r1").is_empty());

    apply(
        &mut tree,
        json!([{ "op": "append", "parent": "r1", "child": "n1", "before": null }]),
    );
    add(&mut tree, "n2", "n1", json!({}));

    assert_eq!(tree.path("n2"), ["n2", "n1", "r1"]);
    assert_eq!(children(&tree, "r1"), ["n1"]);
    assert_eq!(tree.path("nope"), Vec::<String>::new());
}

#[test]
fn append_puts_a_node_before_a_sibling_or_last() {
    let (mut tree, _) = tree();
    add(&mut tree, "n1", "r1", json!({}));
    add(&mut tree, "n2", "r1", json!({}));
    add(&mut tree, "n3", "r1", json!({}));
    assert_eq!(children(&tree, "r1"), ["n1", "n2", "n3"]);

    // Moving within a parent.
    apply(
        &mut tree,
        json!([{ "op": "append", "parent": "r1", "child": "n3", "before": "n1" }]),
    );
    assert_eq!(children(&tree, "r1"), ["n3", "n1", "n2"]);

    apply(
        &mut tree,
        json!([{ "op": "append", "parent": "r1", "child": "n3" }]),
    );
    assert_eq!(children(&tree, "r1"), ["n1", "n2", "n3"]);

    // Moving to another parent.
    apply(
        &mut tree,
        json!([{ "op": "append", "parent": "n1", "child": "n2", "before": null }]),
    );
    assert_eq!(children(&tree, "r1"), ["n1", "n3"]);
    assert_eq!(children(&tree, "n1"), ["n2"]);
    assert_eq!(tree.path("n2"), ["n2", "n1", "r1"]);
}

#[test]
fn append_refuses_what_would_break_the_tree() {
    let (mut tree, _) = tree();
    add(&mut tree, "n1", "r1", json!({}));
    add(&mut tree, "n2", "n1", json!({}));
    add(&mut tree, "n3", "r1", json!({}));

    let cases = [
        (json!({ "parent": "n9", "child": "n1" }), "no node `n9`"),
        (json!({ "parent": "r1", "child": "n9" }), "no node `n9`"),
        (json!({ "parent": "n1", "child": "r1" }), "root"),
        (json!({ "parent": "n1", "child": "n1" }), "inside itself"),
        (json!({ "parent": "n2", "child": "n1" }), "inside itself"),
        (
            json!({ "parent": "n1", "child": "n2", "before": "n3" }),
            "isn't a child",
        ),
        (
            json!({ "parent": "r1", "child": "n3", "before": "n3" }),
            "isn't a child",
        ),
    ];
    for (mut op, expected) in cases {
        op["op"] = json!("append");
        let err = error(&mut tree, op.clone());
        assert!(err.contains(expected), "{op}: {err}");
    }

    // Nothing moved.
    assert_eq!(children(&tree, "r1"), ["n1", "n3"]);
    assert_eq!(children(&tree, "n1"), ["n2"]);
}

#[test]
fn remove_drops_a_node_and_its_subtree() {
    let (mut tree, _) = tree();
    add(&mut tree, "n1", "r1", json!({}));
    add(&mut tree, "n2", "n1", json!({}));
    add(&mut tree, "n3", "r1", json!({}));

    apply(&mut tree, json!([{ "op": "remove", "id": "n1" }]));

    assert_eq!(children(&tree, "r1"), ["n3"]);
    assert!(!tree.nodes.contains_key("n1"));
    assert!(!tree.nodes.contains_key("n2"));
    assert_eq!(tree.path("n2"), Vec::<String>::new());
    assert!(error(&mut tree, json!({ "op": "set", "id": "n2", "props": {} })).contains("no node"));
}

#[test]
fn a_node_that_was_never_appended_can_be_removed() {
    let (mut tree, _) = tree();
    apply(
        &mut tree,
        json!([
            { "op": "create", "id": "n1", "kind": "box" },
            { "op": "remove", "id": "n1" },
        ]),
    );
    assert!(!tree.nodes.contains_key("n1"));
}

#[test]
fn the_root_and_windows_can_be_moved_but_not_removed() {
    let (mut tree, _) = tree();
    tree.add_window(7);
    add(&mut tree, "n1", "r1", json!({}));

    assert!(error(&mut tree, json!({ "op": "remove", "id": "r1" })).contains("only moved"));
    assert!(error(&mut tree, json!({ "op": "remove", "id": "w7" })).contains("only moved"));

    apply(
        &mut tree,
        json!([{ "op": "append", "parent": "n1", "child": "w7" }]),
    );
    assert_eq!(tree.path("w7"), ["w7", "n1", "r1"]);

    // Its container can't be dropped with it inside.
    assert!(error(&mut tree, json!({ "op": "remove", "id": "n1" })).contains("w7"));
    assert!(tree.nodes.contains_key("n1"));

    apply(
        &mut tree,
        json!([
            { "op": "append", "parent": "r1", "child": "w7" },
            { "op": "remove", "id": "n1" },
        ]),
    );
    assert_eq!(children(&tree, "r1"), ["w7"]);
}

#[test]
fn windows_are_added_last_in_the_root_and_removed_with_what_is_in_them() {
    let (mut tree, _) = tree();
    add(&mut tree, "n1", "r1", json!({}));
    tree.add_window(7);
    tree.add_window(8);
    assert_eq!(children(&tree, "r1"), ["n1", "w7", "w8"]);

    add(&mut tree, "n2", "w7", json!({}));
    apply(
        &mut tree,
        json!([{ "op": "append", "parent": "w7", "child": "w8" }]),
    );
    assert_eq!(tree.path("n2"), ["n2", "w7", "r1"]);

    // Adding it twice changes nothing.
    tree.add_window(7);
    assert_eq!(children(&tree, "r1"), ["n1", "w7"]);

    // The window nested in it stays.
    tree.remove_window(7);
    assert!(!tree.nodes.contains_key("w7"));
    assert!(!tree.nodes.contains_key("n2"));
    assert_eq!(children(&tree, "r1"), ["n1", "w8"]);

    tree.remove_window(8);
    tree.remove_window(8);
    assert_eq!(children(&tree, "r1"), ["n1"]);
}

#[test]
fn created_nodes_need_a_fresh_numbered_id() {
    let (mut tree, _) = tree();
    add(&mut tree, "n1", "r1", json!({}));

    for id in ["n1", "n", "nx", "x1", "w1", "r2", ""] {
        let err = error(
            &mut tree,
            json!({ "op": "create", "id": id, "kind": "box" }),
        );
        assert!(err.contains("op 0"), "{id:?}: {err}");
        assert_eq!(tree.nodes.len(), 2, "{id:?}");
    }
}

#[test]
fn props_are_checked_against_the_kind_of_node() {
    let (mut tree, _) = tree();
    tree.add_window(7);
    add(&mut tree, "n1", "r1", json!({}));
    apply(
        &mut tree,
        json!([
            { "op": "create", "id": "n2", "kind": "text",
              "props": { "text": "hi", "font": "sans 12", "color": "#fff" } },
            { "op": "create", "id": "n3", "kind": "image", "props": { "image": "folder" } },
        ]),
    );

    for (id, props) in [
        ("n1", json!({ "text": "x" })),
        ("n1", json!({ "image": "x" })),
        ("n2", json!({ "image": "x" })),
        ("n3", json!({ "color": "#fff" })),
        ("w7", json!({ "background": "#fff" })),
        ("w7", json!({ "border": { "width": 1, "color": "#fff" } })),
        ("r1", json!({ "opacity": 0.5 })),
    ] {
        let err = error(&mut tree, json!({ "op": "set", "id": id, "props": props }));
        assert!(err.contains("op 0"), "{id} {props}: {err}");
    }

    // A window's node takes the props that aren't about drawing.
    set(
        &mut tree,
        "w7",
        json!({ "rect": props_rect(0., 0., 10., 10.), "opacity": 0.5, "interactive": true }),
    );
    assert_eq!(tree.drawn("w7").unwrap().rect, rect(0., 0., 10., 10.));
}

#[test]
fn invalid_ops_are_errors_and_earlier_ops_stay_applied() {
    let (mut tree, _) = tree();

    let err = tree
        .apply(&[
            json!({ "op": "create", "id": "n1", "kind": "box" }),
            json!({ "op": "set", "id": "n1", "props": { "opacity": "half" } }),
            json!({ "op": "create", "id": "n2", "kind": "box" }),
        ])
        .unwrap_err();

    assert!(err.starts_with("op 1: "), "{err}");
    assert!(tree.nodes.contains_key("n1"));
    assert!(!tree.nodes.contains_key("n2"));

    for op in [
        json!({ "op": "explode", "id": "n1" }),
        json!({ "id": "n1" }),
        json!({ "op": "create", "id": "n5", "kind": "circle" }),
        json!({ "op": "set", "id": "n1", "props": { "bogus": 1 } }),
        json!({ "op": "set", "id": "n1" }),
        json!({ "op": "append", "parent": "r1" }),
        json!("create"),
    ] {
        assert!(tree.apply(std::slice::from_ref(&op)).is_err(), "{op}");
    }
}

#[test]
fn set_changes_only_the_props_it_names() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(1., 2., 3., 4.), "opacity": 0.5, "background": "#000" }),
    );

    set(&mut tree, "n1", json!({ "opacity": 0.25 }));
    let drawn = tree.drawn("n1").unwrap();
    assert_eq!(drawn.rect, rect(1., 2., 3., 4.));
    assert_eq!(drawn.opacity, 0.25);
    assert_eq!(
        tree.nodes["n1"].background.current(),
        Some(Paint::Color(Color::from_rgba8_unpremul(0, 0, 0, 255)))
    );

    set(&mut tree, "n1", json!({ "background": null }));
    assert_eq!(tree.nodes["n1"].background.current(), None);
}

#[test]
fn a_fill_and_a_border_can_be_a_gradient() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({
            "background": { "from": "#000", "to": "#fff", "angle": 90, "relativeTo": "view" },
            "border": { "width": 2, "color": { "from": "#f00", "to": "#00f" } },
        }),
    );

    let node = &tree.nodes["n1"];
    let Some(Paint::Gradient(background)) = node.background.current() else {
        panic!("{:?}", node.background.current());
    };
    assert_eq!(background.angle, 90);
    assert_eq!(background.relative_to, GradientRelativeTo::WorkspaceView);
    assert!(matches!(node.border.unwrap().color, Paint::Gradient(_)));

    // A colour replaces it.
    set(&mut tree, "n1", json!({ "background": "#123" }));
    assert!(matches!(
        tree.nodes["n1"].background.current(),
        Some(Paint::Color(_))
    ));

    let err = error(
        &mut tree,
        json!({ "op": "set", "id": "n1", "props": { "background": { "from": "#000" } } }),
    );
    assert!(err.contains("op 0") && err.contains("background"), "{err}");
}

#[test]
fn a_background_changing_to_a_gradient_animates_its_colours() {
    let (mut tree, mut clock) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "background": "#000", "transitions": { "background": linear(100) } }),
    );

    set(
        &mut tree,
        "n1",
        json!({ "background": { "from": "#fff", "to": "#fff", "angle": 90 } }),
    );
    clock.set_unadjusted(Duration::from_millis(50));
    let Some(Paint::Gradient(mid)) = tree.nodes["n1"].background.current() else {
        panic!("{:?}", tree.nodes["n1"].background.current());
    };
    assert!((mid.from.r - 0.5).abs() < 1e-6 && (mid.to.r - 0.5).abs() < 1e-6);
    // It has the new gradient's direction from the start.
    assert_eq!(mid.angle, 90);
}

// Transitions.

#[test]
fn a_prop_with_a_transition_animates_to_its_new_value() {
    let (mut tree, mut clock) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(0., 0., 100., 100.), "opacity": 1 }),
    );
    assert!(!tree.are_animations_ongoing());

    set(
        &mut tree,
        "n1",
        json!({
            "transitions": { "rect": linear(100) },
            "rect": props_rect(100., 0., 200., 100.),
            // No transition: it changes at once.
            "opacity": 0.5,
        }),
    );
    assert!(tree.are_animations_ongoing());

    let drawn = tree.drawn("n1").unwrap();
    assert_eq!(drawn.rect, rect(0., 0., 100., 100.));
    assert_eq!(drawn.opacity, 0.5);

    clock.set_unadjusted(Duration::from_millis(50));
    assert_eq!(tree.drawn("n1").unwrap().rect, rect(50., 0., 150., 100.));
    tree.advance_animations();
    assert!(tree.are_animations_ongoing());

    clock.set_unadjusted(Duration::from_millis(100));
    assert_eq!(tree.drawn("n1").unwrap().rect, rect(100., 0., 200., 100.));
    tree.advance_animations();
    assert!(!tree.are_animations_ongoing());
}

#[test]
fn the_transitions_in_a_set_apply_to_that_set() {
    let (mut tree, mut clock) = tree();
    add(&mut tree, "n1", "r1", json!({ "transform": { "x": 0 } }));

    // Without a transition it's immediate, and setting one doesn't animate what was set before.
    set(&mut tree, "n1", json!({ "transform": { "x": 10 } }));
    set(
        &mut tree,
        "n1",
        json!({ "transitions": { "transform": linear(100) } }),
    );
    assert_eq!(tree.drawn("n1").unwrap().transform.x, 10.);
    assert!(!tree.are_animations_ongoing());

    set(&mut tree, "n1", json!({ "transform": { "x": 30 } }));
    clock.set_unadjusted(Duration::from_millis(50));
    assert_eq!(tree.drawn("n1").unwrap().transform.x, 20.);

    // Without a transition the next change is instant, and ends what's running.
    set(&mut tree, "n1", json!({ "transitions": {} }));
    set(&mut tree, "n1", json!({ "transform": { "x": 0 } }));
    assert_eq!(tree.drawn("n1").unwrap().transform.x, 0.);
}

#[test]
fn a_node_does_not_animate_into_existence() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({
            "rect": props_rect(5., 5., 10., 10.),
            "transitions": { "rect": linear(100), "opacity": linear(100) },
            "opacity": 0.5,
        }),
    );

    assert!(!tree.are_animations_ongoing());
    let drawn = tree.drawn("n1").unwrap();
    assert_eq!(drawn.rect, rect(5., 5., 10., 10.));
    assert_eq!(drawn.opacity, 0.5);
}

#[test]
fn a_transition_can_name_an_animation_of_the_config() {
    let (mut tree, mut clock) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(0., 0., 10., 10.), "transitions": { "rect": "window-movement" } }),
    );

    set(
        &mut tree,
        "n1",
        json!({ "rect": props_rect(100., 0., 10., 10.) }),
    );
    assert!(tree.are_animations_ongoing());
    assert_eq!(tree.drawn("n1").unwrap().rect.loc.x, 0.);

    // The spring has come to rest by then.
    clock.set_unadjusted(Duration::from_secs(10));
    assert_eq!(tree.drawn("n1").unwrap().rect.loc.x, 100.);
    tree.advance_animations();
    assert!(!tree.are_animations_ongoing());
}

#[test]
fn animations_that_are_off_finish_at_once() {
    let (mut tree, clock) = tree();
    clock.clone().set_complete_instantly(true);
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(0., 0., 10., 10.), "transitions": { "rect": linear(100) } }),
    );

    set(
        &mut tree,
        "n1",
        json!({ "rect": props_rect(100., 0., 10., 10.) }),
    );
    assert_eq!(tree.drawn("n1").unwrap().rect.loc.x, 100.);
    tree.advance_animations();
    assert!(!tree.are_animations_ongoing());
}

#[test]
fn a_node_can_fade_in_from_nothing() {
    let (mut tree, mut clock) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "opacity": 0, "transitions": { "opacity": linear(100) } }),
    );
    set(&mut tree, "n1", json!({ "opacity": 1 }));

    clock.set_unadjusted(Duration::from_millis(25));
    assert!((tree.drawn("n1").unwrap().opacity - 0.25).abs() < 1e-9);
}

// Drawn values.

#[test]
fn a_node_with_no_ancestors_but_the_root_is_drawn_as_it_is() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(10., 20., 30., 40.), "opacity": 0.5 }),
    );

    let drawn = tree.drawn("n1").unwrap();
    assert_eq!(drawn.rect, rect(10., 20., 30., 40.));
    assert_eq!(drawn.bounds(), rect(10., 20., 30., 40.));
    assert_eq!(drawn.opacity, 0.5);
    assert_eq!(drawn.transform, NodeTransform::IDENTITY);
    assert_eq!(drawn.clip, None);

    assert!(tree.drawn("r1").is_some());
    assert!(tree.drawn("n9").is_none());
}

#[test]
fn a_transform_scales_around_the_centre_of_the_rect_and_moves() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({
            "rect": props_rect(0., 0., 100., 100.),
            "transform": { "scale": 2, "x": 10, "y": -10 },
        }),
    );

    assert_eq!(
        tree.drawn("n1").unwrap().bounds(),
        rect(-40., -60., 200., 200.)
    );
}

#[test]
fn opacity_multiplies_down_the_tree() {
    let (mut tree, _) = tree();
    add(&mut tree, "n1", "r1", json!({ "opacity": 0.5 }));
    add(&mut tree, "n2", "n1", json!({ "opacity": 0.5 }));
    add(&mut tree, "n3", "n2", json!({}));

    assert_eq!(tree.drawn("n1").unwrap().opacity, 0.5);
    assert_eq!(tree.drawn("n2").unwrap().opacity, 0.25);
    assert_eq!(tree.drawn("n3").unwrap().opacity, 0.25);
}

#[test]
fn transforms_compose_around_each_nodes_own_centre() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({
            "rect": props_rect(0., 0., 100., 100.),
            "transform": { "scale": 2 },
            "opacity": 0.5,
            "clip": true,
            "cornerRadius": 10,
        }),
    );
    add(
        &mut tree,
        "n2",
        "n1",
        json!({
            "rect": props_rect(0., 0., 50., 50.),
            "transform": { "scale": 2 },
            "opacity": 0.5,
        }),
    );

    // Its own transform doesn't clip it, and it isn't affected by what's above it.
    let parent = tree.drawn("n1").unwrap();
    assert_eq!(parent.bounds(), rect(-50., -50., 200., 200.));
    assert_eq!(parent.opacity, 0.5);
    assert_eq!(parent.clip, None);

    // Doubled around its centre, then doubled around its parent's.
    let child = tree.drawn("n2").unwrap();
    assert_eq!(child.rect, rect(0., 0., 50., 50.));
    assert_eq!(child.bounds(), rect(-100., -100., 200., 200.));
    assert_eq!(
        child.transform,
        NodeTransform {
            scale: 4.,
            x: -25.,
            y: -25.
        }
    );
    assert_eq!(child.opacity, 0.25);
    assert_eq!(
        child.clip,
        Some(Clip {
            rect: rect(-50., -50., 200., 200.),
            radius: 20.,
        })
    );
}

#[test]
fn a_parents_move_scales_with_its_scale() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({
            "rect": props_rect(0., 0., 100., 100.),
            "transform": { "scale": 0.5, "x": 10, "y": 10 },
        }),
    );
    add(
        &mut tree,
        "n2",
        "n1",
        json!({
            "rect": props_rect(0., 0., 20., 20.),
            "transform": { "x": 4, "y": 4 },
        }),
    );

    // 0.5 * (p + 4) + (25 + 10).
    assert_eq!(tree.drawn("n2").unwrap().bounds(), rect(37., 37., 10., 10.));
}

#[test]
fn clips_intersect_and_keep_the_corners_of_the_smaller_one() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(0., 0., 100., 100.), "clip": true, "cornerRadius": 8 }),
    );
    add(
        &mut tree,
        "n2",
        "n1",
        json!({ "rect": props_rect(50., 50., 100., 100.), "clip": true }),
    );
    add(
        &mut tree,
        "n3",
        "n2",
        json!({ "rect": props_rect(0., 0., 200., 200.) }),
    );

    // The overlap isn't either one's rect, so it has square corners.
    assert_eq!(
        tree.drawn("n3").unwrap().clip,
        Some(Clip {
            rect: rect(50., 50., 50., 50.),
            radius: 0.
        })
    );

    add(
        &mut tree,
        "n4",
        "n1",
        json!({
            "rect": props_rect(10., 10., 20., 20.),
            "clip": true,
            "cornerRadius": 4,
        }),
    );
    add(&mut tree, "n5", "n4", json!({}));
    assert_eq!(
        tree.drawn("n5").unwrap().clip,
        Some(Clip {
            rect: rect(10., 10., 20., 20.),
            radius: 4.
        })
    );

    // Nothing is left of a clip that doesn't overlap.
    add(
        &mut tree,
        "n6",
        "n1",
        json!({ "rect": props_rect(500., 500., 10., 10.), "clip": true }),
    );
    add(&mut tree, "n7", "n6", json!({}));
    let clip = tree.drawn("n7").unwrap().clip.unwrap();
    assert!(clip.rect.size.w == 0. || clip.rect.size.h == 0.);
}

// Hit testing.

#[test]
fn the_topmost_interactive_node_is_hit() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(0., 0., 100., 100.), "interactive": true }),
    );
    add(
        &mut tree,
        "n2",
        "n1",
        json!({ "rect": props_rect(10., 10., 20., 20.), "interactive": true }),
    );
    add(
        &mut tree,
        "n3",
        "r1",
        json!({ "rect": props_rect(50., 50., 100., 100.), "interactive": true }),
    );
    // On top of everything, but not interactive.
    add(
        &mut tree,
        "n4",
        "r1",
        json!({ "rect": props_rect(0., 0., 300., 300.) }),
    );

    // Children are above their parent, later siblings above earlier ones.
    assert_eq!(tree.hit(point(15., 15.)).as_deref(), Some("n2"));
    assert_eq!(tree.hit(point(5., 5.)).as_deref(), Some("n1"));
    assert_eq!(tree.hit(point(60., 60.)).as_deref(), Some("n3"));
    assert_eq!(tree.hit(point(140., 140.)).as_deref(), Some("n3"));
    assert_eq!(tree.hit(point(200., 200.)), None);
    assert_eq!(tree.hit(point(-1., 5.)), None);

    // The far edges aren't in.
    assert_eq!(tree.hit(point(100., 5.)).as_deref(), None);
}

#[test]
fn windows_are_not_hit() {
    let (mut tree, _) = tree();
    tree.add_window(7);
    set(
        &mut tree,
        "w7",
        json!({ "rect": props_rect(0., 0., 300., 300.), "interactive": true }),
    );
    assert_eq!(tree.hit(point(10., 10.)), None);

    add(
        &mut tree,
        "n1",
        "w7",
        json!({ "rect": props_rect(0., 0., 50., 50.), "interactive": true }),
    );
    assert_eq!(tree.hit(point(10., 10.)).as_deref(), Some("n1"));
}

#[test]
fn what_a_clip_cuts_off_is_not_hit() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(0., 0., 50., 50.), "clip": true }),
    );
    add(
        &mut tree,
        "n2",
        "n1",
        json!({ "rect": props_rect(40., 40., 50., 50.), "interactive": true }),
    );

    assert_eq!(tree.hit(point(45., 45.)).as_deref(), Some("n2"));
    assert_eq!(tree.hit(point(60., 60.)), None);

    // Without the clip it's all of it.
    set(&mut tree, "n1", json!({ "clip": false }));
    assert_eq!(tree.hit(point(60., 60.)).as_deref(), Some("n2"));
}

#[test]
fn hit_testing_follows_transforms() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({
            "rect": props_rect(0., 0., 100., 100.),
            "interactive": true,
            "transform": { "scale": 2 },
        }),
    );
    add(
        &mut tree,
        "n2",
        "n1",
        json!({ "rect": props_rect(0., 0., 10., 10.), "interactive": true }),
    );

    // n1 covers -50 to 150, and n2 -50 to -30 as it's scaled along.
    assert_eq!(tree.hit(point(-40., -40.)).as_deref(), Some("n2"));
    assert_eq!(tree.hit(point(-20., -20.)).as_deref(), Some("n1"));
    assert_eq!(tree.hit(point(149., 149.)).as_deref(), Some("n1"));
    assert_eq!(tree.hit(point(150., 150.)), None);

    set(&mut tree, "n1", json!({ "transform": { "x": 1000 } }));
    assert_eq!(tree.hit(point(50., 50.)), None);
    assert_eq!(tree.hit(point(1050., 50.)).as_deref(), Some("n1"));
}

#[test]
fn hit_testing_follows_what_is_drawn_while_it_moves() {
    let (mut tree, mut clock) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({
            "rect": props_rect(0., 0., 10., 10.),
            "interactive": true,
            "transitions": { "rect": linear(100) },
        }),
    );
    set(
        &mut tree,
        "n1",
        json!({ "rect": props_rect(100., 0., 10., 10.) }),
    );

    assert_eq!(tree.hit(point(5., 5.)).as_deref(), Some("n1"));
    clock.set_unadjusted(Duration::from_millis(50));
    assert_eq!(tree.hit(point(5., 5.)), None);
    assert_eq!(tree.hit(point(55., 5.)).as_deref(), Some("n1"));
}

// Pointer enter and leave.

#[test]
fn the_pointer_enters_and_leaves_nodes() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(0., 0., 100., 100.), "interactive": true }),
    );
    // Not interactive, so it gets no events but doesn't stop its parent from getting them.
    add(
        &mut tree,
        "n2",
        "n1",
        json!({ "rect": props_rect(10., 10., 50., 50.) }),
    );
    add(
        &mut tree,
        "n3",
        "n2",
        json!({ "rect": props_rect(20., 20., 10., 10.), "interactive": true }),
    );

    assert!(tree.pointer_moved(point(500., 500.)).is_empty());

    // Outermost first on the way in.
    assert_eq!(
        tree.pointer_moved(point(25., 25.)),
        [
            ("n1".to_owned(), "pointerenter"),
            ("n3".to_owned(), "pointerenter")
        ]
    );
    assert!(tree.pointer_moved(point(26., 26.)).is_empty());

    assert_eq!(
        tree.pointer_moved(point(50., 50.)),
        [("n3".to_owned(), "pointerleave")]
    );

    // Innermost first on the way out.
    assert_eq!(
        tree.pointer_moved(point(25., 25.)),
        [("n3".to_owned(), "pointerenter")]
    );
    assert_eq!(
        tree.pointer_moved(point(500., 500.)),
        [
            ("n3".to_owned(), "pointerleave"),
            ("n1".to_owned(), "pointerleave")
        ]
    );
}

#[test]
fn moving_between_siblings_leaves_one_and_enters_the_other() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(0., 0., 10., 10.), "interactive": true }),
    );
    add(
        &mut tree,
        "n2",
        "r1",
        json!({ "rect": props_rect(20., 0., 10., 10.), "interactive": true }),
    );

    tree.pointer_moved(point(5., 5.));
    assert_eq!(
        tree.pointer_moved(point(25., 5.)),
        [
            ("n1".to_owned(), "pointerleave"),
            ("n2".to_owned(), "pointerenter")
        ]
    );
}

#[test]
fn the_pointer_leaving_the_workspace_leaves_everything() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(0., 0., 10., 10.), "interactive": true }),
    );

    assert!(tree.pointer_left().is_empty());
    tree.pointer_moved(point(5., 5.));
    assert_eq!(tree.pointer_left(), [("n1".to_owned(), "pointerleave")]);
    assert!(tree.pointer_left().is_empty());
}

#[test]
fn removed_nodes_are_not_left() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(0., 0., 10., 10.), "interactive": true }),
    );

    tree.pointer_moved(point(5., 5.));
    apply(&mut tree, json!([{ "op": "remove", "id": "n1" }]));
    assert!(tree.pointer_moved(point(5., 5.)).is_empty());
}

// Moving between trees.

#[test]
fn a_window_moves_to_another_tree_with_what_is_drawn_on_it() {
    let (mut from, _) = tree();
    let (mut to, _) = tree();
    from.add_window(7);
    from.add_window(8);
    add(&mut from, "n1", "w7", json!({ "opacity": 0.5 }));
    add(&mut from, "n2", "n1", json!({}));
    to.add_window(9);

    let subtree = from.take_window(7).unwrap();
    for id in ["w7", "n1", "n2"] {
        assert!(!from.nodes.contains_key(id));
    }
    assert_eq!(from.window_order(), [8]);

    to.put_window(subtree);
    assert_eq!(to.window_order(), [9, 7]);
    assert_eq!(children(&to, "w7"), ["n1"]);
    assert_eq!(to.path("n2"), ["n2", "n1", "w7", "r1"]);
    assert_eq!(to.drawn("n1").unwrap().opacity, 0.5);
}

#[test]
fn windows_nested_in_a_window_stay_when_it_moves() {
    let (mut from, _) = tree();
    from.add_window(7);
    from.add_window(8);
    apply(
        &mut from,
        json!([{ "op": "append", "parent": "w7", "child": "w8" }]),
    );

    let subtree = from.take_window(7).unwrap();
    assert_eq!(from.window_order(), [8]);
    assert_eq!(children(&from, "r1"), ["w8"]);

    let (mut to, _) = tree();
    to.put_window(subtree);
    assert_eq!(to.window_order(), [7]);
}

#[test]
fn taking_a_window_that_is_not_there_takes_nothing() {
    let (mut from, _) = tree();
    assert!(from.take_window(7).is_none());
}

#[test]
fn a_node_moves_to_another_tree_before_a_sibling() {
    let (mut from, _) = tree();
    let (mut to, _) = tree();
    add(&mut from, "n1", "r1", json!({}));
    add(&mut from, "n2", "n1", json!({}));
    add(&mut to, "n3", "r1", json!({}));
    add(&mut to, "n4", "r1", json!({}));

    let subtree = from.take("n1").unwrap();
    assert!(!from.nodes.contains_key("n1") && !from.nodes.contains_key("n2"));
    assert!(children(&from, "r1").is_empty());

    assert!(to.put(subtree, "r1", Some("n4")));
    assert_eq!(children(&to, "r1"), ["n3", "n1", "n4"]);
    assert_eq!(to.path("n2"), ["n2", "n1", "r1"]);
}

#[test]
fn a_node_is_put_last_when_the_sibling_it_goes_before_is_not_there() {
    let (mut from, _) = tree();
    let (mut to, _) = tree();
    add(&mut from, "n1", "r1", json!({}));
    add(&mut to, "n3", "r1", json!({}));

    let subtree = from.take("n1").unwrap();
    assert!(to.put(subtree, "r1", Some("n9")));
    assert_eq!(children(&to, "r1"), ["n3", "n1"]);
}

#[test]
fn the_root_and_nodes_with_windows_in_them_are_not_taken() {
    let (mut tree, _) = tree();
    add(&mut tree, "n1", "r1", json!({}));
    tree.add_window(7);
    apply(
        &mut tree,
        json!([{ "op": "append", "parent": "n1", "child": "w7" }]),
    );

    assert!(tree.take("r1").is_none());
    assert!(tree.take("n1").is_none());
    assert!(tree.take("n9").is_none());
    assert_eq!(tree.path("w7"), ["w7", "n1", "r1"]);
}

#[test]
fn nothing_is_put_in_a_parent_that_is_not_there() {
    let (mut from, _) = tree();
    let (mut to, _) = tree();
    add(&mut from, "n1", "r1", json!({}));
    let subtree = from.take("n1").unwrap();
    assert!(!to.put(subtree, "n9", None));
    assert!(!to.nodes.contains_key("n1"));
}

#[test]
fn a_node_taken_out_is_no_longer_hovered() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(0., 0., 100., 100.), "interactive": true }),
    );
    tree.pointer_moved(point(5., 5.));
    tree.take("n1").unwrap();
    assert!(tree.pointer_left().is_empty());
}

#[test]
fn windows_are_in_the_order_of_the_tree() {
    let (mut tree, _) = tree();
    tree.add_window(7);
    tree.add_window(8);
    add(&mut tree, "n1", "r1", json!({}));
    tree.add_window(9);
    apply(
        &mut tree,
        json!([{ "op": "append", "parent": "n1", "child": "w9" }]),
    );
    assert_eq!(tree.window_order(), [7, 8, 9]);

    apply(
        &mut tree,
        json!([{ "op": "append", "parent": "r1", "child": "w7" }]),
    );
    assert_eq!(tree.window_order(), [8, 9, 7]);
}

#[test]
fn starting_over_keeps_only_the_windows_given() {
    let (mut tree, _) = tree();
    tree.add_window(7);
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(0., 0., 100., 100.), "interactive": true }),
    );
    set(&mut tree, "w7", json!({ "opacity": 0.5 }));
    tree.pointer_moved(point(5., 5.));

    tree.reset([8, 7]);
    assert!(!tree.nodes.contains_key("n1"));
    assert_eq!(tree.window_order(), [8, 7]);
    assert_eq!(tree.drawn("w7").unwrap().opacity, 1.);
    assert!(tree.pointer_left().is_empty());
}

// Hit testing with windows.

#[test]
fn a_window_over_a_node_takes_the_pointer_from_it() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(0., 0., 100., 100.), "interactive": true }),
    );
    tree.add_window(7);
    add(
        &mut tree,
        "n2",
        "w7",
        json!({ "rect": props_rect(0., 0., 20., 20.), "interactive": true }),
    );

    // The window doesn't cover anything unless the caller says so.
    assert_eq!(tree.hit(point(50., 50.)).as_deref(), Some("n1"));
    assert_eq!(
        tree.hit_with(point(50., 50.), &|_| false).as_deref(),
        Some("n1")
    );

    let covers = |window: u64| window == 7;
    assert_eq!(tree.hit_with(point(50., 50.), &covers), None);
    // What's drawn on the window is above it.
    assert_eq!(
        tree.hit_with(point(10., 10.), &covers).as_deref(),
        Some("n2")
    );
}

#[test]
fn a_window_under_a_node_does_not_take_the_pointer_from_it() {
    let (mut tree, _) = tree();
    tree.add_window(7);
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(0., 0., 100., 100.), "interactive": true }),
    );
    assert_eq!(
        tree.hit_with(point(50., 50.), &|window| window == 7)
            .as_deref(),
        Some("n1")
    );
}

#[test]
fn the_pointer_can_be_moved_onto_a_node_the_caller_picked() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(0., 0., 100., 100.), "interactive": true }),
    );
    add(
        &mut tree,
        "n2",
        "n1",
        json!({ "rect": props_rect(0., 0., 10., 10.), "interactive": true }),
    );

    assert_eq!(
        tree.pointer_over(Some("n2")),
        [
            ("n1".to_owned(), "pointerenter"),
            ("n2".to_owned(), "pointerenter")
        ]
    );
    assert!(tree.pointer_over(Some("n2")).is_empty());
    assert_eq!(
        tree.pointer_over(None),
        [
            ("n2".to_owned(), "pointerleave"),
            ("n1".to_owned(), "pointerleave")
        ]
    );
}

#[test]
fn a_transform_is_the_same_move_around_another_rect() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({
            "rect": props_rect(0., 0., 100., 100.),
            "transform": { "scale": 2, "x": 10, "y": -10 },
        }),
    );
    let drawn = tree.drawn("n1").unwrap();

    // The same rect gives the same transform.
    assert_eq!(drawn.transform_around(drawn.rect), drawn.transform);

    // Another rect moves its own points where the node's points go.
    let other = rect(30., 0., 50., 50.);
    let around = drawn.transform_around(other);
    let moved = |rect: Rectangle<f64, Logical>, t: NodeTransform, p: Point<f64, Logical>| {
        let c = center_f64(rect);
        point(
            t.scale * (p.x - c.x) + c.x + t.x,
            t.scale * (p.y - c.y) + c.y + t.y,
        )
    };
    let p = point(12., 34.);
    assert_eq!(
        moved(drawn.rect, drawn.transform, p),
        moved(other, around, p)
    );
    assert_eq!(around.scale, 2.);
}

// Rounded clips.

#[test]
fn a_clip_with_a_radius_cuts_off_its_corners() {
    let clip = Clip {
        rect: rect(0., 0., 100., 50.),
        radius: 20.,
    };

    assert!(clip.contains(point(50., 25.)));
    // On the straight parts of the edge.
    assert!(clip.contains(point(0., 25.)));
    assert!(clip.contains(point(50., 0.)));
    // Inside the corner's circle, which has its centre at (20, 20).
    assert!(clip.contains(point(8., 8.)));
    assert!(clip.contains(point(99., 25.)));
    // In the square that the circle is cut from.
    assert!(!clip.contains(point(1., 1.)));
    assert!(!clip.contains(point(99., 1.)));
    assert!(!clip.contains(point(1., 49.)));
    assert!(!clip.contains(point(99., 49.)));
    // Outside the rect.
    assert!(!clip.contains(point(-1., 25.)));
    assert!(!clip.contains(point(100., 25.)));
}

#[test]
fn a_radius_larger_than_half_the_clip_makes_it_a_pill() {
    let clip = Clip {
        rect: rect(0., 0., 100., 20.),
        radius: 500.,
    };

    // The ends are half circles of radius 10.
    assert!(clip.contains(point(50., 10.)));
    assert!(clip.contains(point(10., 10.)));
    assert!(clip.contains(point(2., 10.)));
    assert!(!clip.contains(point(1., 1.)));
    assert!(!clip.contains(point(99., 19.)));
}

#[test]
fn a_clip_without_a_radius_is_its_rect() {
    let clip = Clip {
        rect: rect(10., 10., 20., 20.),
        radius: 0.,
    };

    assert!(clip.contains(point(10., 10.)));
    assert!(clip.contains(point(29.9, 29.9)));
    assert!(!clip.contains(point(30., 20.)));
    assert!(!clip.contains(point(9.9, 20.)));
}

#[test]
fn what_a_rounded_clip_cuts_off_is_not_hit() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(0., 0., 100., 100.), "clip": true, "cornerRadius": 20 }),
    );
    add(
        &mut tree,
        "n2",
        "n1",
        json!({ "rect": props_rect(0., 0., 100., 100.), "interactive": true }),
    );

    assert_eq!(tree.hit(point(50., 50.)).as_deref(), Some("n2"));
    assert_eq!(tree.hit(point(10., 10.)).as_deref(), Some("n2"));
    assert_eq!(tree.hit(point(2., 2.)), None);
    assert_eq!(tree.hit(point(98., 98.)), None);

    // Without the radius the corners are there.
    set(&mut tree, "n1", json!({ "cornerRadius": 0 }));
    assert_eq!(tree.hit(point(2., 2.)).as_deref(), Some("n2"));
}

#[test]
fn a_clips_radius_scales_with_its_ancestors() {
    let (mut tree, _) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "rect": props_rect(0., 0., 100., 100.), "transform": { "scale": 0.5 } }),
    );
    add(
        &mut tree,
        "n2",
        "n1",
        json!({ "rect": props_rect(0., 0., 100., 100.), "clip": true, "cornerRadius": 8 }),
    );
    add(&mut tree, "n3", "n2", json!({}));

    assert_eq!(
        tree.drawn("n3").unwrap().clip,
        Some(Clip {
            rect: rect(25., 25., 50., 50.),
            radius: 4.
        })
    );
}

// Opacity of a node with its children.

#[test]
fn a_node_with_children_and_less_opacity_is_drawn_as_a_group() {
    let (mut tree, _) = tree();
    add(&mut tree, "n1", "r1", json!({ "opacity": 0.5 }));
    add(&mut tree, "n2", "n1", json!({ "opacity": 0.5 }));
    add(&mut tree, "n3", "r1", json!({ "opacity": 0.5 }));

    // The group has the opacity, and what's in it is drawn at the full one.
    let group = tree.drawn("n1").unwrap();
    assert_eq!(group.opacity, 0.5);
    assert_eq!(group.local_opacity, 1.);
    assert!(tree.nodes["n1"].fades_as_group());

    // A child's own opacity is its own in the group, and the whole of it for the caller.
    let child = tree.drawn("n2").unwrap();
    assert_eq!(child.opacity, 0.25);
    assert_eq!(child.local_opacity, 0.5);
    assert!(!tree.nodes["n2"].fades_as_group());

    // Without children there's nothing to overlap, so it's drawn as it is.
    let single = tree.drawn("n3").unwrap();
    assert_eq!(single.opacity, 0.5);
    assert_eq!(single.local_opacity, 0.5);
    assert!(!tree.nodes["n3"].fades_as_group());
}

#[test]
fn a_node_at_full_opacity_is_not_a_group() {
    let (mut tree, _) = tree();
    add(&mut tree, "n1", "r1", json!({}));
    add(&mut tree, "n2", "n1", json!({ "opacity": 0.5 }));

    assert!(!tree.nodes["n1"].fades_as_group());
    assert_eq!(tree.drawn("n1").unwrap().local_opacity, 1.);
    // The child isn't in a group, so it's drawn with its opacity.
    assert_eq!(tree.drawn("n2").unwrap().local_opacity, 0.5);
}

#[test]
fn groups_nest_and_each_draws_what_is_in_it_at_full_opacity() {
    let (mut tree, _) = tree();
    add(&mut tree, "n1", "r1", json!({ "opacity": 0.5 }));
    add(&mut tree, "n2", "n1", json!({ "opacity": 0.5 }));
    add(&mut tree, "n3", "n2", json!({}));

    let inner = tree.drawn("n2").unwrap();
    assert_eq!(inner.opacity, 0.25);
    assert_eq!(inner.local_opacity, 1.);
    let leaf = tree.drawn("n3").unwrap();
    assert_eq!(leaf.opacity, 0.25);
    assert_eq!(leaf.local_opacity, 1.);
}

#[test]
fn a_window_in_a_group_is_drawn_with_the_opacity_of_all_its_ancestors() {
    let (mut tree, _) = tree();
    tree.add_window(7);
    add(&mut tree, "n1", "r1", json!({ "opacity": 0.5 }));
    apply(
        &mut tree,
        json!([{ "op": "append", "parent": "n1", "child": "w7" }]),
    );

    // The caller draws windows, so the opacity of the group is theirs.
    assert_eq!(tree.drawn("w7").unwrap().opacity, 0.5);
}

#[test]
fn an_opacity_that_changes_changes_whether_it_is_a_group() {
    let (mut tree, mut clock) = tree();
    add(
        &mut tree,
        "n1",
        "r1",
        json!({ "transitions": { "opacity": linear(100) } }),
    );
    add(&mut tree, "n2", "n1", json!({}));
    assert!(!tree.nodes["n1"].fades_as_group());

    set(&mut tree, "n1", json!({ "opacity": 0. }));
    clock.set_unadjusted(Duration::from_millis(50));
    assert!(tree.nodes["n1"].fades_as_group());
}

// Nodes entering.

fn enter_box(tree: &mut Tree, id: &str, props: Value) {
    apply(
        tree,
        json!([{ "op": "create", "id": id, "kind": "box", "props": props }]),
    );
}

fn append(tree: &mut Tree, parent: &str, child: &str) {
    apply(
        tree,
        json!([{ "op": "append", "parent": parent, "child": child }]),
    );
}

#[test]
fn a_node_enters_from_its_enter_props_with_its_transitions() {
    let (mut tree, mut clock) = tree();
    enter_box(
        &mut tree,
        "n1",
        json!({
            "rect": props_rect(0., 0., 100., 100.),
            "transitions": { "opacity": linear(100), "transform": linear(200) },
            "enter": { "opacity": 0, "transform": { "scale": 0.5, "y": 20 } },
        }),
    );

    // It's not drawn yet, so nothing runs.
    assert!(!tree.are_animations_ongoing());

    append(&mut tree, "r1", "n1");
    assert!(tree.are_animations_ongoing());
    let drawn = tree.drawn("n1").unwrap();
    assert_eq!(drawn.opacity, 0.);
    assert_eq!(
        drawn.transform,
        NodeTransform {
            scale: 0.5,
            x: 0.,
            y: 20.
        }
    );

    clock.set_unadjusted(Duration::from_millis(50));
    let drawn = tree.drawn("n1").unwrap();
    assert!((drawn.opacity - 0.5).abs() < 1e-9);
    assert!((drawn.transform.scale - 0.625).abs() < 1e-9);
    assert!((drawn.transform.y - 15.).abs() < 1e-9);

    clock.set_unadjusted(Duration::from_millis(100));
    assert_eq!(tree.drawn("n1").unwrap().opacity, 1.);
    tree.advance_animations();
    // The transform is slower.
    assert!(tree.are_animations_ongoing());

    clock.set_unadjusted(Duration::from_millis(200));
    assert_eq!(tree.drawn("n1").unwrap().transform, NodeTransform::IDENTITY);
    tree.advance_animations();
    assert!(!tree.are_animations_ongoing());
}

#[test]
fn a_node_enters_from_its_rect_radius_and_background() {
    let (mut tree, mut clock) = tree();
    enter_box(
        &mut tree,
        "n1",
        json!({
            "rect": props_rect(0., 0., 100., 20.),
            "cornerRadius": 10,
            "background": "#ffffff",
            "transitions": {
                "rect": linear(100),
                "cornerRadius": linear(100),
                "background": linear(100),
            },
            "enter": {
                "rect": props_rect(0., 0., 0., 20.),
                "cornerRadius": 0,
                "background": null,
            },
        }),
    );
    append(&mut tree, "r1", "n1");

    let drawn = tree.drawn("n1").unwrap();
    assert_eq!(drawn.rect, rect(0., 0., 0., 20.));
    assert_eq!(tree.nodes["n1"].corner_radius.current(), 0.);
    assert_eq!(tree.nodes["n1"].background.current().unwrap().solid().a, 0.);

    clock.set_unadjusted(Duration::from_millis(50));
    assert_eq!(tree.drawn("n1").unwrap().rect, rect(0., 0., 50., 20.));
    assert_eq!(tree.nodes["n1"].corner_radius.current(), 5.);
    assert!((tree.nodes["n1"].background.current().unwrap().solid().a - 0.5).abs() < 1e-6);
}

#[test]
fn a_prop_without_a_transition_does_not_start_from_its_enter_value() {
    let (mut tree, _) = tree();
    enter_box(
        &mut tree,
        "n1",
        json!({ "opacity": 1, "enter": { "opacity": 0 } }),
    );
    append(&mut tree, "r1", "n1");

    assert!(!tree.are_animations_ongoing());
    assert_eq!(tree.drawn("n1").unwrap().opacity, 1.);
}

#[test]
fn a_node_that_enters_where_it_already_is_does_not_animate() {
    let (mut tree, _) = tree();
    enter_box(
        &mut tree,
        "n1",
        json!({
            "opacity": 0.5,
            "transitions": { "opacity": linear(100) },
            "enter": { "opacity": 0.5 },
        }),
    );
    append(&mut tree, "r1", "n1");

    assert!(!tree.are_animations_ongoing());
}

#[test]
fn changes_before_a_node_is_in_a_tree_do_not_animate() {
    let (mut tree, _) = tree();
    enter_box(
        &mut tree,
        "n1",
        json!({ "opacity": 1, "transitions": { "opacity": linear(100) } }),
    );
    set(&mut tree, "n1", json!({ "opacity": 0.5 }));
    assert!(!tree.are_animations_ongoing());

    append(&mut tree, "r1", "n1");
    assert!(!tree.are_animations_ongoing());
    assert_eq!(tree.drawn("n1").unwrap().opacity, 0.5);
}

#[test]
fn enter_can_be_set_until_the_node_is_in_a_tree() {
    let (mut tree, _) = tree();
    enter_box(
        &mut tree,
        "n1",
        json!({ "transitions": { "opacity": linear(100) } }),
    );
    set(&mut tree, "n1", json!({ "enter": { "opacity": 0 } }));
    append(&mut tree, "r1", "n1");
    assert!(tree.are_animations_ongoing());
    assert_eq!(tree.drawn("n1").unwrap().opacity, 0.);

    let err = error(
        &mut tree,
        json!({ "op": "set", "id": "n1", "props": { "enter": { "opacity": 0.5 } } }),
    );
    assert!(err.contains("enter") && err.contains("already"), "{err}");

    tree.add_window(7);
    let err = error(
        &mut tree,
        json!({ "op": "set", "id": "w7", "props": { "enter": {} } }),
    );
    assert!(err.contains("enter"), "{err}");
}

#[test]
fn a_subtree_enters_when_its_top_is_added_to_the_tree() {
    let (mut tree, mut clock) = tree();
    let props = json!({
        "transitions": { "opacity": linear(100) },
        "enter": { "opacity": 0 },
    });
    enter_box(&mut tree, "n1", props.clone());
    enter_box(&mut tree, "n2", props);
    append(&mut tree, "n1", "n2");

    // Neither is in a tree yet.
    assert!(!tree.are_animations_ongoing());

    append(&mut tree, "r1", "n1");
    assert_eq!(tree.nodes["n1"].opacity.current(), 0.);
    assert_eq!(tree.nodes["n2"].opacity.current(), 0.);

    clock.set_unadjusted(Duration::from_millis(100));
    assert_eq!(tree.nodes["n1"].opacity.current(), 1.);
    assert_eq!(tree.nodes["n2"].opacity.current(), 1.);
}

#[test]
fn a_node_that_is_moved_in_the_tree_does_not_enter_again() {
    let (mut tree, mut clock) = tree();
    enter_box(
        &mut tree,
        "n1",
        json!({
            "transitions": { "opacity": linear(100) },
            "enter": { "opacity": 0 },
        }),
    );
    add(&mut tree, "n2", "r1", json!({}));
    append(&mut tree, "r1", "n1");

    clock.set_unadjusted(Duration::from_millis(100));
    tree.advance_animations();
    assert!(!tree.are_animations_ongoing());

    append(&mut tree, "n2", "n1");
    assert!(!tree.are_animations_ongoing());
    assert_eq!(tree.nodes["n1"].opacity.current(), 1.);
}

#[test]
fn a_node_taken_to_another_tree_does_not_enter_again() {
    let (mut tree, mut clock) = tree();
    enter_box(
        &mut tree,
        "n1",
        json!({
            "transitions": { "opacity": linear(100) },
            "enter": { "opacity": 0 },
        }),
    );
    append(&mut tree, "r1", "n1");
    clock.set_unadjusted(Duration::from_millis(100));
    tree.advance_animations();

    let taken = tree.take("n1").unwrap();
    let mut other = Tree::new(2, clock.clone(), Animations::default());
    assert!(other.put(taken, "r2", None));
    assert!(!other.are_animations_ongoing());
    assert_eq!(other.nodes["n1"].opacity.current(), 1.);
}

#[test]
fn an_enter_prop_that_is_not_a_transitioning_one_is_an_error() {
    let (mut tree, _) = tree();
    let err = error(
        &mut tree,
        json!({ "op": "create", "id": "n1", "kind": "box", "props": { "enter": { "clip": true } } }),
    );
    assert!(err.contains("enter"), "{err}");
    assert!(!tree.contains("n1"));
}
