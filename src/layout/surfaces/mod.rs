//! What an engine draws on a workspace: a tree of nodes, described by the engine and drawn by
//! bento.
//!
//! The engine changes the tree with ops (see [`Tree::apply`]), and the tree runs the transitions
//! those ops ask for, draws itself, and answers where the pointer is. Window nodes are in the same
//! tree as the nodes the engine creates; they're drawn by the caller, in the slot the tree
//! reports for them.
//!
//! Coordinates are logical pixels on the workspace's canvas.

use std::cell::RefCell;
use std::collections::HashMap;

use niri_config::{Animations, Color};
use serde::Deserialize;
use serde_json::Value;
use smithay::utils::{Logical, Point, Rectangle, Size};

use self::animated::Animated;
pub use self::props::NodeTransform;
use self::props::{Border, Paint, Transitions, Update};
pub use self::render::{RenderItem, SurfaceRenderElement};
use self::render::{Target, Visuals};
use crate::animation::Clock;
use crate::render_helpers::border::BorderRenderElement;
use crate::render_helpers::renderer::NiriRenderer;
use crate::utils::center_f64;

mod animated;
mod icons;
mod props;
mod render;

#[derive(Debug, Clone, Copy, PartialEq)]
enum NodeKind {
    Root,
    Box,
    Text,
    Image,
    /// A window, by its id.
    Window(u64),
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CreateKind {
    Box,
    Text,
    Image,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum Op {
    Create {
        id: String,
        kind: CreateKind,
        #[serde(default)]
        props: Update,
    },
    Set {
        id: String,
        props: Update,
    },
    Append {
        parent: String,
        child: String,
        #[serde(default)]
        before: Option<String>,
    },
    Remove {
        id: String,
    },
}

#[derive(Debug)]
struct Node {
    kind: NodeKind,
    parent: Option<String>,
    children: Vec<String>,
    rect: Animated<Rectangle<f64, Logical>>,
    opacity: Animated<f64>,
    transform: Animated<NodeTransform>,
    corner_radius: Animated<f64>,
    background: Animated<Option<Paint>>,
    clip: bool,
    border: Option<Border>,
    text: String,
    font: String,
    color: Color,
    image: Option<String>,
    interactive: bool,
    transitions: Transitions,
    visuals: RefCell<Visuals>,
}

/// Where a node is drawn, with everything its ancestors do to it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drawn {
    /// Its own rect right now, before its transform and its ancestors'.
    pub rect: Rectangle<f64, Logical>,
    /// Its own opacity times its ancestors'.
    pub opacity: f64,
    /// Its own transform after its ancestors', as one: a scale around the centre of `rect`, then
    /// a move.
    pub transform: NodeTransform,
    /// The area its ancestors clip it to, where it ends up.
    pub clip: Option<Clip>,
}

/// An area that cuts off what's drawn in it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clip {
    /// Where it ends up.
    pub rect: Rectangle<f64, Logical>,
    pub radius: f64,
}

/// Where a window goes, reported to the caller that draws windows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowSlot {
    pub id: u64,
    pub drawn: Drawn,
}

/// A node with everything under it, taken out of a tree to put in another workspace's.
#[derive(Debug)]
pub struct Subtree(Vec<(String, Node)>);

/// What a node's ancestors do to it, as seen from the node: it ends up at `scale` times its
/// canvas position plus `offset`.
#[derive(Debug, Clone, Copy)]
struct Space {
    scale: f64,
    offset: Point<f64, Logical>,
    opacity: f64,
    clip: Option<Clip>,
}

impl Clip {
    fn intersect(self, other: Clip) -> Clip {
        let rect = self.rect.intersection(other.rect).unwrap_or_default();
        // The corners that are left are the smaller one's.
        let radius = if rect == other.rect {
            other.radius
        } else if rect == self.rect {
            self.radius
        } else {
            0.
        };
        Clip { rect, radius }
    }
}

impl Drawn {
    /// Where it ends up, with its transform applied.
    pub fn bounds(&self) -> Rectangle<f64, Logical> {
        let centre = center_f64(self.rect);
        let size = Size::from((
            self.rect.size.w * self.transform.scale,
            self.rect.size.h * self.transform.scale,
        ));
        Rectangle::new(
            Point::from((
                centre.x + self.transform.x - size.w / 2.,
                centre.y + self.transform.y - size.h / 2.,
            )),
            size,
        )
    }

    /// Its transform as a scale around the centre of `rect` instead of its own rect, for a caller
    /// that places the node somewhere else than the tree thinks. It moves points the same way.
    pub fn transform_around(&self, rect: Rectangle<f64, Logical>) -> NodeTransform {
        let own = center_f64(self.rect);
        let centre = center_f64(rect);
        let scale = self.transform.scale;
        NodeTransform {
            scale,
            x: self.transform.x + (own.x - centre.x) * (1. - scale),
            y: self.transform.y + (own.y - centre.y) * (1. - scale),
        }
    }

    /// Whether `point` is on it, and not cut off by a clip.
    pub fn contains(&self, point: Point<f64, Logical>) -> bool {
        self.bounds().contains(point) && self.clip.is_none_or(|clip| clip.rect.contains(point))
    }
}

impl Space {
    fn root() -> Self {
        Self {
            scale: 1.,
            offset: Point::from((0., 0.)),
            opacity: 1.,
            clip: None,
        }
    }

    /// Where `node` is drawn, and the space its children are in.
    fn enter(&self, node: &Node) -> (Drawn, Space) {
        let rect = node.rect.current();
        let own = node.transform.current();
        let centre = center_f64(rect);

        // Its own transform moves a point `p` to `own.scale * (p - centre) + centre + (own.x,
        // own.y)`, which is `own.scale * p + b`. Its ancestors then move that.
        let b: Point<f64, Logical> = Point::from((
            centre.x * (1. - own.scale) + own.x,
            centre.y * (1. - own.scale) + own.y,
        ));
        let scale = self.scale * own.scale;
        let offset = Point::from((
            self.scale * b.x + self.offset.x,
            self.scale * b.y + self.offset.y,
        ));

        let drawn = Drawn {
            rect,
            opacity: self.opacity * node.opacity.current().clamp(0., 1.),
            // The same move, as a scale around the centre of the rect and then a translation.
            transform: NodeTransform {
                scale,
                x: offset.x - centre.x * (1. - scale),
                y: offset.y - centre.y * (1. - scale),
            },
            clip: self.clip,
        };

        let mut clip = self.clip;
        if node.clip {
            let area = Clip {
                rect: drawn.bounds(),
                radius: node.corner_radius.current().max(0.) * scale,
            };
            clip = Some(clip.map_or(area, |outer| outer.intersect(area)));
        }

        let inner = Space {
            scale,
            offset,
            opacity: drawn.opacity,
            clip,
        };
        (drawn, inner)
    }
}

impl Node {
    fn new(kind: NodeKind) -> Self {
        Self {
            kind,
            parent: None,
            children: Vec::new(),
            rect: Animated::new(Rectangle::default()),
            opacity: Animated::new(1.),
            transform: Animated::new(NodeTransform::IDENTITY),
            corner_radius: Animated::new(0.),
            background: Animated::new(None),
            clip: false,
            border: None,
            text: String::new(),
            font: String::from("sans 11"),
            color: Color::from_rgba8_unpremul(255, 255, 255, 255),
            image: None,
            interactive: false,
            transitions: Transitions::default(),
            visuals: RefCell::default(),
        }
    }

    /// Changes properties. Those with a transition animate, unless `animate` is `None`.
    fn update(&mut self, update: Update, animate: Option<(&Clock, &Animations)>) {
        if let Some(transitions) = update.transitions {
            self.transitions = transitions;
        }

        let start = |transition: &Option<props::Transition>| {
            let (clock, animations) = animate?;
            Some((clock, transition.as_ref()?.resolve(animations)?))
        };

        if let Some(rect) = update.rect {
            self.rect.set(rect, start(&self.transitions.rect));
        }
        if let Some(opacity) = update.opacity {
            self.opacity.set(opacity, start(&self.transitions.opacity));
        }
        if let Some(transform) = update.transform {
            self.transform
                .set(transform, start(&self.transitions.transform));
        }
        if let Some(radius) = update.corner_radius {
            self.corner_radius
                .set(radius, start(&self.transitions.corner_radius));
        }
        if let Some(background) = update.background {
            self.background
                .set(background, start(&self.transitions.background));
        }

        if let Some(clip) = update.clip {
            self.clip = clip;
        }
        if let Some(border) = update.border {
            self.border = border;
        }
        if let Some(text) = update.text {
            self.text = text;
        }
        if let Some(font) = update.font {
            self.font = font;
        }
        if let Some(color) = update.color {
            self.color = color;
        }
        if let Some(image) = update.image {
            self.image = Some(image);
        }
        if let Some(interactive) = update.interactive {
            self.interactive = interactive;
        }
    }

    fn advance_animations(&mut self) {
        self.rect.advance();
        self.opacity.advance();
        self.transform.advance();
        self.corner_radius.advance();
        self.background.advance();
    }

    fn are_animations_ongoing(&self) -> bool {
        self.rect.is_animating()
            || self.opacity.is_animating()
            || self.transform.is_animating()
            || self.corner_radius.is_animating()
            || self.background.is_animating()
    }

    /// Whether the pointer can be on it. Windows get the pointer from bento, not from the tree.
    fn takes_pointer(&self) -> bool {
        self.interactive && !matches!(self.kind, NodeKind::Root | NodeKind::Window(_))
    }
}

/// Which properties a kind of node has.
fn check_props(kind: NodeKind, update: &Update) -> Result<(), String> {
    let only = |prop: &str, wanted: &str| -> Result<(), String> {
        Err(format!("`{prop}` is only for {wanted} nodes"))
    };

    if update.text.is_some() && kind != NodeKind::Text {
        return only("text", "text");
    }
    if update.font.is_some() && kind != NodeKind::Text {
        return only("font", "text");
    }
    if update.color.is_some() && kind != NodeKind::Text {
        return only("color", "text");
    }
    if update.image.is_some() && kind != NodeKind::Image {
        return only("image", "image");
    }
    if (update.background.is_some() || update.border.is_some())
        && matches!(kind, NodeKind::Root | NodeKind::Window(_))
    {
        return Err("window nodes draw no background or border".into());
    }
    Ok(())
}

fn window_key(id: u64) -> String {
    format!("w{id}")
}

/// One workspace's tree of drawn nodes.
///
/// Nodes are named by strings: `"n<number>"` for the ones an engine creates, `"w<window id>"` for
/// windows and `"r<workspace id>"` for the root.
#[derive(Debug)]
pub struct Tree {
    root: String,
    nodes: HashMap<String, Node>,
    /// The nodes the pointer is on, innermost first.
    hovered: Vec<String>,
    icon_theme: String,
    clock: Clock,
    animations: Animations,
}

impl Tree {
    pub fn new(workspace: u64, clock: Clock, animations: Animations) -> Self {
        let root = format!("r{workspace}");
        Self {
            nodes: HashMap::from([(root.clone(), Node::new(NodeKind::Root))]),
            root,
            hovered: Vec::new(),
            icon_theme: String::from("hicolor"),
            clock,
            animations,
        }
    }

    pub fn root_id(&self) -> &str {
        &self.root
    }

    /// Whether the tree has a node, in it or created by the engine and not put in yet.
    pub fn contains(&self, id: &str) -> bool {
        self.nodes.contains_key(id)
    }

    /// Changes what the names of transitions mean.
    pub fn set_animations(&mut self, animations: Animations) {
        self.animations = animations;
    }

    /// Changes the icon theme that image nodes look icons up in.
    pub fn set_icon_theme(&mut self, theme: &str) {
        self.icon_theme = theme.to_owned();
    }

    /// Applies ops, in order:
    ///
    /// - `{"op":"create","id":"n4","kind":"box"|"text"|"image","props":{...}}` makes a node, which
    ///   is drawn once it's added to the tree;
    /// - `{"op":"set","id":"n4","props":{...}}` changes properties, and those with a transition
    ///   animate;
    /// - `{"op":"append","parent":"r2","child":"n4","before":"w7"|null}` puts a node in the tree,
    ///   or moves it, before one of the parent's children or last;
    /// - `{"op":"remove","id":"n4"}` drops a node and its subtree. Window nodes can't be removed,
    ///   only moved.
    ///
    /// On an error the ops before the one that failed stay applied.
    pub fn apply(&mut self, ops: &[Value]) -> Result<(), String> {
        for (index, op) in ops.iter().enumerate() {
            let op = Op::deserialize(op).map_err(|err| format!("op {index}: {err}"))?;
            self.apply_op(op)
                .map_err(|err| format!("op {index}: {err}"))?;
        }
        Ok(())
    }

    fn apply_op(&mut self, op: Op) -> Result<(), String> {
        match op {
            Op::Create { id, kind, props } => self.create(id, kind, props),
            Op::Set { id, props } => self.set(&id, props),
            Op::Append {
                parent,
                child,
                before,
            } => self.append(&parent, &child, before.as_deref()),
            Op::Remove { id } => self.remove(&id),
        }
    }

    fn create(&mut self, id: String, kind: CreateKind, props: Update) -> Result<(), String> {
        let numbered = id
            .strip_prefix('n')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if !numbered {
            return Err(format!(
                "`{id}` can't be a created node's id, those are `n` and a number"
            ));
        }
        if self.nodes.contains_key(&id) {
            return Err(format!("node `{id}` exists already"));
        }

        let kind = match kind {
            CreateKind::Box => NodeKind::Box,
            CreateKind::Text => NodeKind::Text,
            CreateKind::Image => NodeKind::Image,
        };
        check_props(kind, &props)?;

        let mut node = Node::new(kind);
        node.update(props, None);
        self.nodes.insert(id, node);
        Ok(())
    }

    fn set(&mut self, id: &str, props: Update) -> Result<(), String> {
        let node = self
            .nodes
            .get_mut(id)
            .ok_or_else(|| format!("there's no node `{id}`"))?;
        if node.kind == NodeKind::Root {
            return Err("the root can't be changed".into());
        }
        check_props(node.kind, &props)?;

        node.update(props, Some((&self.clock, &self.animations)));
        Ok(())
    }

    fn append(&mut self, parent: &str, child: &str, before: Option<&str>) -> Result<(), String> {
        if !self.nodes.contains_key(parent) {
            return Err(format!("there's no node `{parent}`"));
        }
        let Some(node) = self.nodes.get(child) else {
            return Err(format!("there's no node `{child}`"));
        };
        if node.kind == NodeKind::Root {
            return Err("the root can't be moved".into());
        }
        if self.path(parent).iter().any(|id| id == child) {
            return Err(format!("`{child}` can't go inside itself"));
        }
        if let Some(before) = before {
            let sibling = before != child
                && self.nodes.get(before).and_then(|n| n.parent.as_deref()) == Some(parent);
            if !sibling {
                return Err(format!("`{before}` isn't a child of `{parent}`"));
            }
        }

        self.detach(child);
        self.attach(parent, child, before);
        Ok(())
    }

    fn remove(&mut self, id: &str) -> Result<(), String> {
        let Some(node) = self.nodes.get(id) else {
            return Err(format!("there's no node `{id}`"));
        };
        if matches!(node.kind, NodeKind::Root | NodeKind::Window(_)) {
            return Err(format!("`{id}` can't be removed, only moved"));
        }

        let subtree = self.subtree_ids(id);
        if let Some(window) = subtree
            .iter()
            .find(|id| matches!(self.nodes[id.as_str()].kind, NodeKind::Window(_)))
        {
            return Err(format!(
                "`{id}` has the window node `{window}` in it, which can't be removed, only moved \
                 out first"
            ));
        }

        self.detach(id);
        for id in &subtree {
            self.nodes.remove(id);
        }
        self.hovered.retain(|id| self.nodes.contains_key(id));
        Ok(())
    }

    /// Takes a node out of its parent.
    fn detach(&mut self, id: &str) {
        let Some(parent) = self.nodes.get_mut(id).and_then(|node| node.parent.take()) else {
            return;
        };
        if let Some(parent) = self.nodes.get_mut(&parent) {
            parent.children.retain(|child| child != id);
        }
    }

    /// Puts a node that has no parent into `parent`, before its child `before` or last.
    fn attach(&mut self, parent: &str, child: &str, before: Option<&str>) {
        if let Some(parent) = self.nodes.get_mut(parent) {
            let index = before
                .and_then(|before| parent.children.iter().position(|c| c == before))
                .unwrap_or(parent.children.len());
            parent.children.insert(index, child.to_owned());
        }
        if let Some(child) = self.nodes.get_mut(child) {
            child.parent = Some(parent.to_owned());
        }
    }

    /// The node and everything in it, the node first.
    fn subtree_ids(&self, id: &str) -> Vec<String> {
        let mut subtree = vec![id.to_owned()];
        let mut next = 0;
        while let Some(id) = subtree.get(next) {
            if let Some(node) = self.nodes.get(id) {
                let children = node.children.clone();
                subtree.extend(children);
            }
            next += 1;
        }
        subtree
    }

    fn is_window(&self, id: &str) -> bool {
        self.nodes
            .get(id)
            .is_some_and(|node| matches!(node.kind, NodeKind::Window(_)))
    }

    /// Adds a window's node, last in the root.
    pub fn add_window(&mut self, id: u64) {
        let key = window_key(id);
        if self.nodes.contains_key(&key) {
            warn!("the surface tree has the window node {key} already");
            return;
        }

        self.nodes
            .insert(key.clone(), Node::new(NodeKind::Window(id)));
        let root = self.root.clone();
        self.attach(&root, &key, None);
    }

    /// Drops a window's node, and what the engine put in it. Windows that were in it go to the
    /// end of the root.
    pub fn remove_window(&mut self, id: u64) {
        let key = window_key(id);
        if !self.nodes.contains_key(&key) {
            warn!("the surface tree has no window node {key}");
            return;
        }

        self.move_nested_windows_to_root(&key);
        drop(self.take_subtree(&key));
    }

    /// Takes a window's node out, with what the engine put in it, to put in the tree of the
    /// workspace that the window moves to. Windows that were in it go to the end of the root.
    pub fn take_window(&mut self, id: u64) -> Option<Subtree> {
        let key = window_key(id);
        if !self.nodes.contains_key(&key) {
            warn!("the surface tree has no window node {key}");
            return None;
        }

        self.move_nested_windows_to_root(&key);
        Some(self.take_subtree(&key))
    }

    /// Puts a window's node that was taken out, last in the root.
    pub fn put_window(&mut self, subtree: Subtree) {
        let root = self.root.clone();
        self.put_subtree(subtree, &root, None);
    }

    /// Takes a node and what's under it out, to put in another workspace's tree with
    /// [`Tree::put`]. Nothing is taken if it's the root or has a window node in it: windows move
    /// with their workspace, not as part of a node.
    pub fn take(&mut self, id: &str) -> Option<Subtree> {
        if self.nodes.get(id)?.kind == NodeKind::Root {
            return None;
        }
        if self.subtree_ids(id).iter().any(|id| self.is_window(id)) {
            return None;
        }
        Some(self.take_subtree(id))
    }

    /// Puts a node that was taken out of another tree in `parent`, before its child `before` or
    /// last. Returns whether `parent` exists.
    pub fn put(&mut self, subtree: Subtree, parent: &str, before: Option<&str>) -> bool {
        if !self.nodes.contains_key(parent) {
            return false;
        }
        self.put_subtree(subtree, parent, before);
        true
    }

    fn move_nested_windows_to_root(&mut self, id: &str) {
        let nested: Vec<_> = self
            .subtree_ids(id)
            .into_iter()
            .skip(1)
            .filter(|id| self.is_window(id))
            .collect();
        let root = self.root.clone();
        for window in nested {
            self.detach(&window);
            self.attach(&root, &window, None);
        }
    }

    fn take_subtree(&mut self, id: &str) -> Subtree {
        let ids = self.subtree_ids(id);
        self.detach(id);
        let nodes = ids
            .into_iter()
            .filter_map(|id| Some((id.clone(), self.nodes.remove(&id)?)))
            .collect();
        self.hovered.retain(|id| self.nodes.contains_key(id));
        Subtree(nodes)
    }

    fn put_subtree(&mut self, subtree: Subtree, parent: &str, before: Option<&str>) {
        let Some((id, _)) = subtree.0.first() else {
            return;
        };
        let id = id.clone();
        self.nodes.extend(subtree.0);
        let before = before.filter(|before| {
            *before != id
                && self.nodes.get(*before).and_then(|n| n.parent.as_deref()) == Some(parent)
        });
        self.attach(parent, &id, before);
    }

    /// Starts over for a new engine: its nodes are gone, and the windows are in the root in the
    /// order given, from the bottom to the top.
    pub fn reset(&mut self, windows: impl IntoIterator<Item = u64>) {
        self.nodes.retain(|_, node| node.kind == NodeKind::Root);
        if let Some(root) = self.nodes.get_mut(&self.root) {
            root.children.clear();
        }
        self.hovered.clear();
        for window in windows {
            self.add_window(window);
        }
    }

    /// Ids of the windows in the tree, from the bottom to the top.
    pub fn window_order(&self) -> Vec<u64> {
        let mut windows = Vec::new();
        let mut pending = vec![self.root.as_str()];
        while let Some(id) = pending.pop() {
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            if let NodeKind::Window(window) = node.kind {
                windows.push(window);
            }
            pending.extend(node.children.iter().rev().map(String::as_str));
        }
        windows
    }

    pub fn advance_animations(&mut self) {
        for node in self.nodes.values_mut() {
            node.advance_animations();
        }
    }

    pub fn are_animations_ongoing(&self) -> bool {
        self.nodes.values().any(Node::are_animations_ongoing)
    }

    /// The ids of a node's children, from the bottom to the top.
    pub fn children(&self, id: &str) -> &[String] {
        self.nodes
            .get(id)
            .map_or(&[][..], |node| node.children.as_slice())
    }

    /// The ids from a node up to the root, for events to bubble along. Empty if there's no such
    /// node.
    pub fn path(&self, id: &str) -> Vec<String> {
        let mut path = Vec::new();
        let mut next = self.nodes.get(id).map(|_| id);
        while let Some(id) = next {
            path.push(id.to_owned());
            next = self.nodes[id].parent.as_deref();
        }
        path
    }

    /// Calls `f` for every node in the order they're drawn: a node, then its children, with later
    /// siblings above earlier ones.
    fn walk(&self, f: &mut dyn FnMut(&str, &Node, &Drawn)) {
        self.walk_from(&self.root, &Space::root(), f);
    }

    fn walk_from(&self, id: &str, space: &Space, f: &mut dyn FnMut(&str, &Node, &Drawn)) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };

        let (drawn, inner) = space.enter(node);
        f(id, node, &drawn);
        for child in &node.children {
            self.walk_from(child, &inner, f);
        }
    }

    /// Where a node is drawn right now, with its ancestors' opacity, transforms and clips.
    pub fn drawn(&self, id: &str) -> Option<Drawn> {
        let mut space = Space::root();
        let mut drawn = None;
        for id in self.path(id).iter().rev() {
            let (node_drawn, inner) = space.enter(&self.nodes[id.as_str()]);
            drawn = Some(node_drawn);
            space = inner;
        }
        drawn
    }

    /// Hands what the tree draws to `push`, in draw order, with the canvas' origin at `location`
    /// on the output, which has `scale`. The workspace's view starts at the output's origin and is
    /// `view_size` large, which gradients that span the view cover. Windows come as slots, in
    /// canvas coordinates.
    pub fn render_elements<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        location: Point<f64, Logical>,
        scale: f64,
        view_size: Size<f64, Logical>,
        push: &mut dyn FnMut(RenderItem),
    ) {
        let target = Target {
            location,
            scale,
            view: Rectangle::from_size(view_size),
            icon_theme: &self.icon_theme,
            has_border_shader: BorderRenderElement::has_shader(renderer),
        };

        self.walk(&mut |_, node, drawn| {
            if let NodeKind::Window(id) = node.kind {
                if drawn.opacity > 0. {
                    push(RenderItem::Window(WindowSlot { id, drawn: *drawn }));
                }
            } else {
                render::draw(renderer, node, drawn, &target, push);
            }
        });
    }

    /// The topmost interactive node at `point` on the canvas. Windows aren't hit.
    pub fn hit(&self, point: Point<f64, Logical>) -> Option<String> {
        self.hit_with(point, &|_| false)
    }

    /// The topmost interactive node at `point` on the canvas, where the windows for which
    /// `covers` is true also take the pointer from the nodes below them.
    pub fn hit_with(
        &self,
        point: Point<f64, Logical>,
        covers: &dyn Fn(u64) -> bool,
    ) -> Option<String> {
        let mut hit = None;
        self.walk(&mut |id, node, drawn| match node.kind {
            NodeKind::Window(window) if covers(window) => hit = None,
            _ if node.takes_pointer() && drawn.contains(point) => hit = Some(id.to_owned()),
            _ => (),
        });
        hit
    }

    /// The interactive nodes from `id` up to the root.
    fn pointer_path(&self, id: &str) -> Vec<String> {
        let mut path = self.path(id);
        path.retain(|id| self.nodes[id.as_str()].takes_pointer());
        path
    }

    /// Moves the pointer to `point` on the canvas. Returns the `pointerleave`s of the nodes it
    /// left, innermost first, then the `pointerenter`s of the nodes it entered, outermost first.
    pub fn pointer_moved(&mut self, point: Point<f64, Logical>) -> Vec<(String, &'static str)> {
        let hit = self.hit(point);
        self.pointer_over(hit.as_deref())
    }

    /// Moves the pointer onto the node `hit`, or off all of them, like [`Tree::pointer_moved`]
    /// for a caller that decided what's under the pointer.
    pub fn pointer_over(&mut self, hit: Option<&str>) -> Vec<(String, &'static str)> {
        let now = hit.map(|id| self.pointer_path(id)).unwrap_or_default();
        self.set_hovered(now)
    }

    /// The pointer left the workspace.
    pub fn pointer_left(&mut self) -> Vec<(String, &'static str)> {
        self.set_hovered(Vec::new())
    }

    fn set_hovered(&mut self, now: Vec<String>) -> Vec<(String, &'static str)> {
        let left = self
            .hovered
            .iter()
            .filter(|id| !now.contains(id))
            .map(|id| (id.clone(), "pointerleave"));
        let entered = now
            .iter()
            .rev()
            .filter(|id| !self.hovered.contains(id))
            .map(|id| (id.clone(), "pointerenter"));
        let events = left.chain(entered).collect();

        self.hovered = now;
        events
    }
}

#[cfg(test)]
mod tests;
