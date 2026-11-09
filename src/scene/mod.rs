//! bento's own shell, as a scene that bento lays out, animates and draws itself.
//!
//! The scene is a tree of nodes per output: boxes, shapes with a material, and text. Scripts
//! build and change it (see [`script`]); bento lays it out with flexbox, moves each node's frame,
//! corners and opacity with springs towards where the layout puts them, and draws it in the same
//! frame as everything else, so the shell's motion never waits on anything.
//!
//! A node's frame is where it is on its output, so a node moves only when its own place does,
//! and what's in an island moves with it as their places move together. A node that grows out of a point grows as a circle until it's as wide
//! as its narrower side, then stretches to its proportions, and shrinks back the same way.

pub mod apps;
pub mod host;
pub mod script;
mod spring;
pub mod text;
pub mod vk;

use std::rc::Rc;

use serde_json::Value;
use taffy::prelude::*;

pub use self::spring::Spring;
use self::text::{Fonts, Laid, TextStyle};

pub type NodeId = u32;

/// A colour: a fixed one, or one of the theme's, at an opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Paint {
    Rgba([f32; 4]),
    /// The text colour.
    Fg(f32),
    Accent(f32),
    /// The window background colour.
    Window(f32),
}

impl Paint {
    fn parse(value: &Value) -> Option<Self> {
        let s = value.as_str()?;
        let (name, alpha) = match s.split_once('/') {
            Some((name, alpha)) => (name, alpha.trim().parse().ok()?),
            None => (s, 1.),
        };
        Some(match name.trim() {
            "fg" => Self::Fg(alpha),
            "accent" => Self::Accent(alpha),
            "window" => Self::Window(alpha),
            hex => {
                let hex = hex.strip_prefix('#')?;
                let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
                let a = if hex.len() == 8 { byte(6)? } else { 255 };
                let [r, g, b] = [byte(0)?, byte(2)?, byte(4)?];
                Self::Rgba([r, g, b, a].map(|c| f32::from(c) / 255.))
            }
        })
    }

    /// The colour, straight (not premultiplied), with the theme's colours from `theme`.
    pub fn resolve(self, theme: &Theme) -> [f32; 4] {
        match self {
            Self::Rgba(rgba) => rgba,
            Self::Fg(a) => {
                let [r, g, b, fa] = theme.fg;
                [r, g, b, fa * a]
            }
            Self::Accent(a) => {
                let [r, g, b, _] = theme.accent;
                [r, g, b, a]
            }
            Self::Window(a) => {
                let [r, g, b, _] = theme.window;
                [r, g, b, a]
            }
        }
    }
}

/// The colours the scene's theme colours stand for: libadwaita's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub fg: [f32; 4],
    pub accent: [f32; 4],
    pub window: [f32; 4],
}

impl Theme {
    pub fn new(dark: bool, accent: Option<[f32; 4]>) -> Self {
        let accent = accent.unwrap_or([0.21, 0.52, 0.89, 1.]);
        if dark {
            Self {
                fg: [1., 1., 1., 1.],
                accent,
                window: [0.133, 0.133, 0.149, 1.],
            }
        } else {
            Self {
                fg: [0., 0., 6. / 255., 0.8],
                accent,
                window: [0.98, 0.98, 0.984, 1.],
            }
        }
    }
}

/// What a shape is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Material {
    /// Nothing: only its fill and border, if any.
    #[default]
    None,
    Solid,
    /// The wallpaper alone, blurred and tinted.
    Mica,
    /// What's behind, blurred and tinted.
    Acrylic,
    /// What's behind, blurred and refracted through a bezel.
    Glass,
}

#[derive(Debug, Clone)]
pub enum Kind {
    Box,
    Text {
        text: String,
        style: TextStyle,
        /// The text laid out, at the width it was laid out for.
        laid: Option<(Option<f32>, Rc<Laid>)>,
    },
}

/// How a node comes and goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Enter {
    /// It's just there.
    #[default]
    Appear,
    /// It grows out of a point in its middle, and shrinks back into one.
    Point,
    /// It fades in and out.
    Fade,
}

#[derive(Debug, Clone)]
struct Look {
    fill: Option<Paint>,
    material: Material,
    tint: Option<Paint>,
    radii: [f32; 4],
    border: Option<(f32, Paint)>,
    opacity: f32,
    /// How big it's drawn, around its middle, as a fraction of its size.
    scale: f32,
    /// How blurred it's drawn, in logical pixels.
    blur: f32,
    clip: bool,
    color: Paint,
    /// How the pointer looks over it, like "pointer".
    cursor: Option<String>,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            fill: None,
            material: Material::None,
            tint: None,
            radii: [0.; 4],
            border: None,
            opacity: 1.,
            scale: 1.,
            blur: 0.,
            clip: false,
            color: Paint::Fg(1.),
            cursor: None,
        }
    }
}

/// The events a node takes.
pub const CLICK: u32 = 1;
pub const HOVER: u32 = 2;
pub const SCROLL: u32 = 4;

// A node's springs: its frame on its output, opacity, corners, scale and blur.
const X: usize = 0;
const Y: usize = 1;
const W: usize = 2;
const H: usize = 3;
const OPACITY: usize = 4;
const RADII: usize = 5;
const SCALE: usize = 9;
const BLUR: usize = 10;

#[derive(Debug)]
pub struct Node {
    pub kind: Kind,
    taffy: taffy::NodeId,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    look: Look,
    springs: [Spring; 11],
    placed: bool,
    /// Where it heads instead of its place, while it travels into a node it was matched with.
    travel: Option<[f32; 4]>,
    animate: bool,
    enter: Enter,
    /// Its size when it started growing out of a point or shrinking into one.
    whole: Option<[f32; 2]>,
    leaving: bool,
    pub events: u32,
}

/// What the renderer draws, in order, back to front, in the output's logical coordinates.
#[derive(Debug, Clone)]
pub enum Item {
    Shape {
        rect: [f32; 4],
        radii: [f32; 4],
        material: Material,
        /// The fill, or for a material, its tint.
        fill: [f32; 4],
        border: (f32, [f32; 4]),
        opacity: f32,
        /// How blurred, in logical pixels.
        blur: f32,
        clip: Option<Clip>,
    },
    Text {
        origin: [f32; 2],
        laid: Rc<Laid>,
        color: [f32; 4],
        opacity: f32,
        /// How big it's drawn, as a fraction of its size.
        scale: f32,
        blur: f32,
        clip: Option<Clip>,
    },
}

/// A rounded rectangle that what's drawn is cut to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clip {
    pub rect: [f32; 4],
    pub radii: [f32; 4],
}

struct Root {
    output: String,
    node: NodeId,
    size: (f32, f32),
}

pub struct Scene {
    nodes: Vec<Option<Node>>,
    free: Vec<NodeId>,
    tree: TaffyTree<NodeId>,
    roots: Vec<Root>,
    fonts: Fonts,
    layout_dirty: bool,
    /// Nodes to match once they're laid out: one grows out of the other.
    matches: Vec<(NodeId, NodeId)>,
    /// What the shell keeps for itself along each output's edges, that windows stay out of:
    /// top, right, bottom and left.
    reserved: std::collections::HashMap<String, [f32; 4]>,
    /// bento's style that the shell lines up with: its gaps and corner radius.
    pub style: Value,
    /// Whether the shell wants the keyboard, which then doesn't go to the apps.
    pub wants_keyboard: bool,
    /// Whether anything changed since it was last drawn.
    changed: bool,
    pub theme: Theme,
}

impl std::fmt::Debug for Scene {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scene")
            .field("nodes", &self.nodes.iter().flatten().count())
            .finish()
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            free: Vec::new(),
            tree: TaffyTree::new(),
            roots: Vec::new(),
            fonts: Fonts::default(),
            layout_dirty: true,
            matches: Vec::new(),
            reserved: std::collections::HashMap::new(),
            style: Value::Null,
            wants_keyboard: false,
            changed: true,
            theme: Theme::new(true, None),
        }
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id as usize)?.as_ref()
    }

    fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id as usize)?.as_mut()
    }

    /// The root node of output `name`, `size` big, made if it isn't there yet.
    pub fn set_output(&mut self, name: &str, size: (f32, f32)) -> NodeId {
        if let Some(root) = self.roots.iter_mut().find(|r| r.output == name) {
            if root.size != size {
                root.size = size;
                self.layout_dirty = true;
            }
            return root.node;
        }
        let node = self.create("box", &Value::Null);
        self.roots.push(Root {
            output: String::from(name),
            node,
            size,
        });
        self.layout_dirty = true;
        node
    }

    pub fn root(&self, output: &str) -> Option<NodeId> {
        self.roots
            .iter()
            .find(|r| r.output == output)
            .map(|r| r.node)
    }

    pub fn outputs(&self) -> impl Iterator<Item = &str> {
        self.roots.iter().map(|r| r.output.as_str())
    }

    /// Takes away output `name`'s nodes.
    pub fn remove_output(&mut self, name: &str) {
        self.reserved.remove(name);
        if let Some(i) = self.roots.iter().position(|r| r.output == name) {
            let root = self.roots.remove(i);
            self.destroy(root.node);
        }
    }

    /// Makes a node of `kind`, "box" or "text", with `props`.
    pub fn create(&mut self, kind: &str, props: &Value) -> NodeId {
        let kind = match kind {
            "text" => Kind::Text {
                text: String::new(),
                style: TextStyle::default(),
                laid: None,
            },
            _ => Kind::Box,
        };
        let id = match self.free.pop() {
            Some(id) => id,
            None => {
                self.nodes.push(None);
                (self.nodes.len() - 1) as NodeId
            }
        };
        let taffy = self
            .tree
            .new_leaf_with_context(Style::default(), id)
            .expect("taffy never fails to add a leaf");
        let mut springs = [Spring::new(0.); 11];
        springs[OPACITY] = Spring::new(1.);
        springs[SCALE] = Spring::new(1.);
        self.nodes[id as usize] = Some(Node {
            kind,
            taffy,
            parent: None,
            children: Vec::new(),
            look: Look::default(),
            springs,
            placed: false,
            travel: None,
            animate: true,
            enter: Enter::Appear,
            whole: None,
            leaving: false,
            events: 0,
        });
        self.set(id, props);
        id
    }

    /// Changes a node's props. Layout props lay the scene out again; the look moves there with
    /// springs.
    pub fn set(&mut self, id: NodeId, props: &Value) {
        let Some(props) = props.as_object() else {
            return;
        };
        self.changed = true;
        let Some(node) = self.nodes.get_mut(id as usize).and_then(Option::as_mut) else {
            return;
        };
        let mut style = self.tree.style(node.taffy).cloned().unwrap_or_default();
        let mut relayout = false;
        for (key, value) in props {
            relayout |= apply_layout(&mut style, key, value);
            apply_look(node, key, value);
            if let Kind::Text {
                text,
                style: text_style,
                laid,
            } = &mut node.kind
            {
                let changed = match key.as_str() {
                    "text" => value
                        .as_str()
                        .filter(|v| *v != text.as_str())
                        .map(|v| *text = String::from(v))
                        .is_some(),
                    "size" => value.as_f64().map(|v| text_style.size = v as f32).is_some(),
                    "weight" => value
                        .as_f64()
                        .map(|v| text_style.weight = v as f32)
                        .is_some(),
                    "family" => value
                        .as_str()
                        .map(|v| text_style.family = String::from(v))
                        .is_some(),
                    "textAlign" => value
                        .as_str()
                        .map(|v| {
                            text_style.align = match v {
                                "center" => 0.5,
                                "end" | "right" => 1.,
                                _ => 0.,
                            }
                        })
                        .is_some(),
                    "tabular" => value.as_bool().map(|v| text_style.tabular = v).is_some(),
                    _ => false,
                };
                if changed {
                    *laid = None;
                    relayout = true;
                    let _ = self.tree.mark_dirty(node.taffy);
                }
            }
        }
        if relayout {
            let _ = self.tree.set_style(node.taffy, style);
            self.layout_dirty = true;
        }
        // The look's targets.
        let node = self.nodes[id as usize].as_mut().unwrap();
        node.springs[OPACITY].target = f64::from(node.look.opacity);
        // Shown again, a node that travelled into another goes back to its own place and size.
        if node.travel.is_some() && node.look.opacity > 0. {
            node.travel = None;
            self.layout_dirty = true;
        }
        if node.travel.is_none() {
            node.springs[SCALE].target = f64::from(node.look.scale);
        }
        node.springs[BLUR].target = f64::from(node.look.blur);
        for i in 0..4 {
            node.springs[RADII + i].target = f64::from(node.look.radii[i]);
        }
        if !node.placed {
            for spring in &mut node.springs[OPACITY..] {
                spring.jump(spring.target);
            }
        }
        // Where the look starts from, before it springs to its targets.
        if let Some(from) = props.get("jump").and_then(Value::as_object) {
            for (key, value) in from {
                let spring = match key.as_str() {
                    "opacity" => OPACITY,
                    "scale" => SCALE,
                    "blur" => BLUR,
                    _ => continue,
                };
                if let Some(value) = value.as_f64() {
                    node.springs[spring].value = value;
                    node.springs[spring].velocity = 0.;
                }
            }
        }
    }

    /// Puts `child` into `parent`, at `index`, or last.
    pub fn append(&mut self, parent: NodeId, child: NodeId, index: Option<usize>) {
        if parent == child || self.node(parent).is_none() || self.node(child).is_none() {
            return;
        }
        self.detach(child);
        let (parent_taffy, child_taffy) = (
            self.node(parent).unwrap().taffy,
            self.node(child).unwrap().taffy,
        );
        let children = &mut self.node_mut(parent).unwrap().children;
        let index = index.unwrap_or(children.len()).min(children.len());
        children.insert(index, child);
        let _ = self
            .tree
            .insert_child_at_index(parent_taffy, index, child_taffy);
        self.node_mut(child).unwrap().parent = Some(parent);
        self.layout_dirty = true;
        self.changed = true;
    }

    fn detach(&mut self, child: NodeId) {
        let Some(parent) = self.node(child).and_then(|n| n.parent) else {
            return;
        };
        let child_taffy = self.node(child).unwrap().taffy;
        if let Some(parent_node) = self.node_mut(parent) {
            parent_node.children.retain(|c| *c != child);
            let parent_taffy = parent_node.taffy;
            let _ = self.tree.remove_child(parent_taffy, child_taffy);
        }
        self.node_mut(child).unwrap().parent = None;
        self.layout_dirty = true;
    }

    /// Takes a node away: at once, or after it went the way it came, as `enter` says.
    pub fn remove(&mut self, id: NodeId) {
        let Some(node) = self.node_mut(id) else {
            return;
        };
        if node.enter == Enter::Appear || !node.placed {
            self.destroy(id);
            return;
        }
        node.leaving = true;
        match node.enter {
            Enter::Point => {
                node.whole = Some([node.springs[W].value as f32, node.springs[H].value as f32]);
                let (cx, cy) = (
                    node.springs[X].value + node.springs[W].value / 2.,
                    node.springs[Y].value + node.springs[H].value / 2.,
                );
                node.springs[X].target = cx;
                node.springs[Y].target = cy;
                node.springs[W].target = 0.;
                node.springs[H].target = 0.;
            }
            _ => node.springs[OPACITY].target = 0.,
        }
    }

    fn destroy(&mut self, id: NodeId) {
        self.detach(id);
        self.changed = true;
        let Some(node) = self.nodes.get_mut(id as usize).and_then(Option::take) else {
            return;
        };
        let _ = self.tree.remove(node.taffy);
        self.free.push(id);
        for child in node.children {
            if let Some(child_node) = self.node_mut(child) {
                child_node.parent = None;
            }
            self.destroy(child);
        }
    }

    /// Lays the scene out again if anything changed, setting where each node heads.
    pub fn lay_out(&mut self) {
        if !self.layout_dirty {
            return;
        }
        self.layout_dirty = false;
        for i in 0..self.roots.len() {
            let (node, size) = (self.roots[i].node, self.roots[i].size);
            let Some(taffy) = self.node(node).map(|n| n.taffy) else {
                continue;
            };
            let mut style = self.tree.style(taffy).cloned().unwrap_or_default();
            style.size = Size {
                width: Dimension::length(size.0),
                height: Dimension::length(size.1),
            };
            let _ = self.tree.set_style(taffy, style);

            let Self {
                tree, nodes, fonts, ..
            } = self;
            let _ =
                tree.compute_layout_with_measure(taffy, Size::MAX_CONTENT, |input, _, id, _| {
                    measure(nodes, fonts, input, id.copied())
                });
            self.place(node, [0., 0.]);
        }
        for (from, to) in std::mem::take(&mut self.matches) {
            self.resolve_match(from, to);
        }
    }

    /// Makes `to` grow out of where `from` is, and `from` travel into where `to` goes, so they
    /// read as one thing moving to its new place and size.
    pub fn r#match(&mut self, from: NodeId, to: NodeId) {
        self.matches.push((from, to));
        self.layout_dirty = true;
        self.changed = true;
    }

    fn resolve_match(&mut self, from: NodeId, to: NodeId) {
        let (Some(f), Some(t)) = (self.node(from), self.node(to)) else {
            return;
        };
        let (fr, _) = Self::drawn(f);
        let f_scale = f.springs[SCALE].value as f32;
        let tt = [X, Y, W, H].map(|i| t.springs[i].target as f32);
        if fr[3] <= 0. || tt[3] <= 0. {
            return;
        }
        let fc = [fr[0] + fr[2] / 2., fr[1] + fr[3] / 2.];
        let tc = [tt[0] + tt[2] / 2., tt[1] + tt[3] / 2.];

        let t = self.node_mut(to).unwrap();
        t.travel = None;
        t.placed = true;
        let start = [fc[0] - tt[2] / 2., fc[1] - tt[3] / 2., tt[2], tt[3]];
        for i in 0..4 {
            t.springs[i].value = f64::from(start[i]);
            t.springs[i].velocity = 0.;
        }
        t.springs[SCALE].value = f64::from(fr[3] * f_scale / tt[3]);
        t.springs[SCALE].velocity = 0.;

        let f = self.node_mut(from).unwrap();
        let travel = [tc[0] - fr[2] / 2., tc[1] - fr[3] / 2., fr[2], fr[3]];
        f.travel = Some(travel);
        for i in 0..4 {
            f.springs[i].target = f64::from(travel[i]);
        }
        f.springs[SCALE].target = f64::from(tt[3] / fr[3]);
    }

    /// Sets the targets of `id` and what's in it from the layout, with its parent's top left at
    /// `origin`. Frames are absolute, so a node whose place on the output doesn't change doesn't
    /// move, whatever its parent does.
    fn place(&mut self, id: NodeId, origin: [f32; 2]) {
        let Some(node) = self.node(id) else { return };
        let Ok(layout) = self.tree.layout(node.taffy) else {
            return;
        };
        let target = [
            origin[0] + layout.location.x,
            origin[1] + layout.location.y,
            layout.size.width,
            layout.size.height,
        ];
        let children = node.children.clone();
        let node = self.node_mut(id).unwrap();
        let own = target;
        let target = node.travel.unwrap_or(target);
        if !node.leaving {
            if !node.placed {
                node.placed = true;
                let start = match node.enter {
                    Enter::Point => {
                        node.whole = Some([target[2], target[3]]);
                        [
                            target[0] + target[2] / 2.,
                            target[1] + target[3] / 2.,
                            0.,
                            0.,
                        ]
                    }
                    _ => target,
                };
                for i in 0..4 {
                    node.springs[i].jump(f64::from(start[i]));
                }
                if node.enter == Enter::Fade {
                    node.springs[OPACITY].jump(0.);
                }
            } else if !node.animate {
                for i in 0..4 {
                    node.springs[i].jump(f64::from(target[i]));
                }
            }
            for i in 0..4 {
                node.springs[i].target = f64::from(target[i]);
            }
        }
        for child in children {
            self.place(child, [own[0], own[1]]);
        }
    }

    /// Moves everything `dt` seconds on, and takes away what finished leaving. Says whether
    /// anything is still moving.
    pub fn advance(&mut self, dt: f64, slowdown: Option<f64>) -> bool {
        let mut moving = false;
        let mut gone = Vec::new();
        for (id, node) in self.nodes.iter_mut().enumerate() {
            let Some(node) = node else { continue };
            let mut settled = true;
            for spring in &mut node.springs {
                spring.advance(dt, slowdown);
                settled &= spring.settle(0.01);
            }
            if settled {
                if node.leaving {
                    gone.push(id as NodeId);
                } else {
                    node.whole = None;
                }
            } else {
                moving = true;
            }
        }
        for id in gone {
            self.destroy(id);
        }
        moving
    }

    /// Keeps `edges` along output `output`, top, right, bottom and left, for the shell.
    pub fn reserve(&mut self, output: &str, edges: [f32; 4]) {
        self.reserved.insert(String::from(output), edges);
    }

    /// What the shell keeps along output `output`'s edges.
    pub fn reserved(&self, output: &str) -> [f32; 4] {
        self.reserved.get(output).copied().unwrap_or_default()
    }

    /// Whether anything needs laying out.
    pub fn is_dirty(&self) -> bool {
        self.layout_dirty
    }

    /// Whether anything changed since this was last asked, so it needs drawing again.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    pub fn set_theme(&mut self, theme: Theme) {
        if self.theme != theme {
            self.theme = theme;
            self.changed = true;
        }
    }

    /// Where a node is drawn on its output, with its size; and the fraction of its whole size it
    /// has grown to, while it grows out of a point or shrinks into one.
    fn drawn(node: &Node) -> ([f32; 4], f32) {
        let s = &node.springs;
        let mut rect = [
            s[X].value as f32,
            s[Y].value as f32,
            s[W].value.max(0.) as f32,
            s[H].value.max(0.) as f32,
        ];
        let mut grown = 1.;
        if let (Enter::Point, Some([ww, wh])) = (node.enter, node.whole) {
            if ww > 0. && wh > 0. {
                let g = (rect[2] / ww).max(rect[3] / wh).clamp(0., 1.);
                grown = g;
                let narrow = ww.min(wh);
                let (w, h) = if g < 0.5 {
                    let d = narrow * g / 0.5;
                    (d, d)
                } else {
                    let t = (g - 0.5) / 0.5;
                    (narrow + (ww - narrow) * t, narrow + (wh - narrow) * t)
                };
                let (cx, cy) = (rect[0] + rect[2] / 2., rect[1] + rect[3] / 2.);
                rect = [cx - w / 2., cy - h / 2., w, h];
            }
        }
        (rect, grown)
    }

    /// What to draw for output `name`, back to front.
    pub fn draw(&self, output: &str) -> Vec<Item> {
        let mut items = Vec::new();
        if let Some(root) = self.root(output) {
            self.draw_node(root, 1., None, Scaling::NONE, 0., &mut items);
        }
        items
    }

    fn draw_node(
        &self,
        id: NodeId,
        opacity: f32,
        clip: Option<Clip>,
        scaling: Scaling,
        blur: f32,
        items: &mut Vec<Item>,
    ) {
        let Some(node) = self.node(id) else { return };
        let (rect, grown) = Self::drawn(node);
        let opacity = opacity * node.springs[OPACITY].value.clamp(0., 1.) as f32;
        if opacity <= 0.003 {
            return;
        }
        let half = rect[2].min(rect[3]) / 2.;
        let radii: [f32; 4] = std::array::from_fn(|i| {
            let r = node.springs[RADII + i].value as f32;
            // Growing out of a point, it's round until it stretches.
            let r = if node.whole.is_some() && grown < 0.5 {
                half
            } else {
                r
            };
            r.clamp(0., half)
        });
        // Scaled around its middle, inside whatever its parents are scaled by.
        let own = node.springs[SCALE].value.max(0.) as f32;
        let middle = [rect[0] + rect[2] / 2., rect[1] + rect[3] / 2.];
        let scaling = scaling.then(Scaling {
            by: own,
            offset: [middle[0] * (1. - own), middle[1] * (1. - own)],
        });
        let rect = scaling.rect(rect);
        let radii = radii.map(|r| r * scaling.by);
        let blur = blur + node.springs[BLUR].value.max(0.) as f32;

        match &node.kind {
            Kind::Box => {
                let fill = node.look.fill.or(node.look.tint);
                if node.look.material != Material::None
                    || fill.is_some()
                    || node.look.border.is_some()
                {
                    items.push(Item::Shape {
                        rect,
                        radii,
                        material: node.look.material,
                        fill: fill.map_or([0.; 4], |p| p.resolve(&self.theme)),
                        border: node
                            .look
                            .border
                            .map_or((0., [0.; 4]), |(w, p)| (w, p.resolve(&self.theme))),
                        opacity,
                        blur,
                        clip,
                    });
                }
            }
            Kind::Text { laid, .. } => {
                if let Some((_, laid)) = laid {
                    items.push(Item::Text {
                        origin: [rect[0], rect[1]],
                        laid: laid.clone(),
                        color: node.look.color.resolve(&self.theme),
                        opacity,
                        scale: scaling.by,
                        blur,
                        clip,
                    });
                }
            }
        }

        // What's in something growing out of a point fades in late and out early.
        let content = if node.whole.is_some() {
            if node.leaving {
                1. - smoothstep(0., 0.35, 1. - grown)
            } else {
                smoothstep(0.35, 0.85, grown)
            }
        } else {
            1.
        };
        let clip = if node.look.clip {
            Some(Clip { rect, radii })
        } else {
            clip
        };
        for child in &node.children {
            self.draw_node(*child, opacity * content, clip, scaling, blur, items);
        }
    }

    /// The nodes at `point` on output `output`, from the bottom up: those whose drawn shape,
    /// rounded, scaled and cut to what clips it, holds the point, and that aren't hidden or
    /// leaving.
    fn pick(&self, output: &str, point: [f32; 2]) -> Vec<NodeId> {
        let mut found = Vec::new();
        if let Some(root) = self.root(output) {
            self.pick_node(root, point, Scaling::NONE, None, &mut found);
        }
        found
    }

    fn pick_node(
        &self,
        id: NodeId,
        point: [f32; 2],
        scaling: Scaling,
        clip: Option<Clip>,
        found: &mut Vec<NodeId>,
    ) {
        let Some(node) = self.node(id) else { return };
        if node.leaving || node.springs[OPACITY].target <= 0. {
            return;
        }
        let (rect, _) = Self::drawn(node);
        let own = node.springs[SCALE].value.max(0.) as f32;
        let middle = [rect[0] + rect[2] / 2., rect[1] + rect[3] / 2.];
        let scaling = scaling.then(Scaling {
            by: own,
            offset: [middle[0] * (1. - own), middle[1] * (1. - own)],
        });
        let rect = scaling.rect(rect);
        let half = rect[2].min(rect[3]) / 2.;
        let radii: [f32; 4] = std::array::from_fn(|i| {
            (node.springs[RADII + i].value as f32 * scaling.by).clamp(0., half)
        });
        let in_clip = clip.is_none_or(|c| inside_rounded(point, c.rect, c.radii));
        if in_clip && inside_rounded(point, rect, radii) {
            found.push(id);
        }
        let clip = if node.look.clip {
            Some(Clip { rect, radii })
        } else {
            clip
        };
        for child in &node.children {
            self.pick_node(*child, point, scaling, clip, found);
        }
    }

    /// The topmost node at `point` on output `output` that takes any of `events`.
    pub fn hit(&self, output: &str, point: [f32; 2], events: u32) -> Option<NodeId> {
        self.pick(output, point)
            .into_iter()
            .rev()
            .find(|id| self.node(*id).is_some_and(|n| n.events & events != 0))
    }

    /// Whether the shell is at `point` on output `output`, so it takes the pointer there from
    /// what's below: a shape or something that takes events is.
    pub fn covers(&self, output: &str, point: [f32; 2]) -> bool {
        self.pick(output, point).into_iter().any(|id| {
            self.node(id).is_some_and(|n| {
                n.events != 0
                    || n.look.material != Material::None
                    || n.look.fill.is_some()
                    || n.look.border.is_some()
            })
        })
    }

    /// The pointer's look at `point` on output `output`: the topmost node's that says one, like
    /// "pointer" over a button.
    pub fn cursor(&self, output: &str, point: [f32; 2]) -> Option<String> {
        self.pick(output, point)
            .into_iter()
            .rev()
            .find_map(|id| self.node(id).and_then(|n| n.look.cursor.clone()))
    }
}

/// Whether `point` is inside the rectangle `rect` with corners `radii`.
fn inside_rounded(point: [f32; 2], rect: [f32; 4], radii: [f32; 4]) -> bool {
    let half = [rect[2] / 2., rect[3] / 2.];
    let p = [point[0] - rect[0] - half[0], point[1] - rect[1] - half[1]];
    let r = match (p[0] < 0., p[1] < 0.) {
        (true, true) => radii[0],
        (false, true) => radii[1],
        (false, false) => radii[2],
        (true, false) => radii[3],
    };
    let q = [p[0].abs() - half[0] + r, p[1].abs() - half[1] + r];
    let outside = (q[0].max(0.).powi(2) + q[1].max(0.).powi(2)).sqrt();
    q[0].max(q[1]).min(0.) + outside - r <= 0.
}

/// A scaling of what's drawn: a point `p` is drawn at `p * by + offset`.
#[derive(Debug, Clone, Copy)]
struct Scaling {
    by: f32,
    offset: [f32; 2],
}

impl Scaling {
    const NONE: Self = Self {
        by: 1.,
        offset: [0., 0.],
    };

    /// This scaling, applied after `inner`.
    fn then(self, inner: Self) -> Self {
        Self {
            by: self.by * inner.by,
            offset: [
                self.by * inner.offset[0] + self.offset[0],
                self.by * inner.offset[1] + self.offset[1],
            ],
        }
    }

    fn rect(self, rect: [f32; 4]) -> [f32; 4] {
        [
            rect[0] * self.by + self.offset[0],
            rect[1] * self.by + self.offset[1],
            rect[2] * self.by,
            rect[3] * self.by,
        ]
    }
}

fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}

/// Measures a text node for taffy.
fn measure(
    nodes: &mut [Option<Node>],
    fonts: &mut Fonts,
    input: taffy::LayoutInput,
    id: Option<NodeId>,
) -> taffy::LayoutOutput {
    let Some(Some(node)) = id.and_then(|id| nodes.get_mut(id as usize)) else {
        return taffy::LayoutOutput::HIDDEN;
    };
    let Kind::Text { text, style, laid } = &mut node.kind else {
        let size = Size {
            width: input.known_dimensions.width.unwrap_or(0.),
            height: input.known_dimensions.height.unwrap_or(0.),
        };
        return taffy::LayoutOutput::from_outer_size(size);
    };
    let max_width = input
        .known_dimensions
        .width
        .or(match input.available_space.width {
            AvailableSpace::Definite(width) => Some(width),
            _ => None,
        });
    let reuse = laid.as_ref().filter(|(at, laid)| {
        *at == max_width || (at.is_none() && max_width.is_some_and(|w| w >= laid.width))
    });
    let laid = match reuse {
        Some((_, laid)) => laid.clone(),
        None => {
            let new = Rc::new(fonts.lay_out(text, style, max_width));
            *laid = Some((max_width, new.clone()));
            new
        }
    };
    taffy::LayoutOutput::from_outer_size(Size {
        width: input.known_dimensions.width.unwrap_or(laid.width.ceil()),
        height: input.known_dimensions.height.unwrap_or(laid.height.ceil()),
    })
}

fn length_or_auto(value: &Value) -> Option<Dimension> {
    if let Some(v) = value.as_f64() {
        return Some(Dimension::length(v as f32));
    }
    let s = value.as_str()?;
    if s == "auto" {
        return Some(Dimension::auto());
    }
    let percent: f32 = s.strip_suffix('%')?.parse().ok()?;
    Some(Dimension::percent(percent / 100.))
}

fn inset(value: &Value) -> Option<LengthPercentageAuto> {
    if let Some(v) = value.as_f64() {
        return Some(LengthPercentageAuto::length(v as f32));
    }
    let s = value.as_str()?;
    if s == "auto" {
        return Some(LengthPercentageAuto::auto());
    }
    let percent: f32 = s.strip_suffix('%')?.parse().ok()?;
    Some(LengthPercentageAuto::percent(percent / 100.))
}

/// Up to four numbers, CSS-style: all, vertical and horizontal, or top, right, bottom and left.
fn sides(value: &Value) -> Option<[f32; 4]> {
    if let Some(v) = value.as_f64() {
        return Some([v as f32; 4]);
    }
    let list: Vec<f32> = value
        .as_array()?
        .iter()
        .filter_map(|v| v.as_f64().map(|v| v as f32))
        .collect();
    Some(match list.as_slice() {
        [v] => [*v; 4],
        [v, h] => [*v, *h, *v, *h],
        [t, h, b] => [*t, *h, *b, *h],
        [t, r, b, l, ..] => [*t, *r, *b, *l],
        [] => return None,
    })
}

fn align_items(value: &Value) -> Option<AlignItems> {
    Some(match value.as_str()? {
        "start" => AlignItems::FLEX_START,
        "center" => AlignItems::CENTER,
        "end" => AlignItems::FLEX_END,
        "stretch" => AlignItems::STRETCH,
        "baseline" => AlignItems::BASELINE,
        _ => return None,
    })
}

/// Applies one layout prop, and says whether it was one.
fn apply_layout(style: &mut Style, key: &str, value: &Value) -> bool {
    match key {
        "direction" => {
            style.flex_direction = match value.as_str() {
                Some("column") => FlexDirection::Column,
                _ => FlexDirection::Row,
            }
        }
        "gap" => {
            let gap = value.as_f64().unwrap_or(0.) as f32;
            style.gap = Size {
                width: LengthPercentage::length(gap),
                height: LengthPercentage::length(gap),
            };
        }
        "padding" => {
            let [t, r, b, l] = sides(value).unwrap_or([0.; 4]);
            style.padding = Rect {
                left: LengthPercentage::length(l),
                right: LengthPercentage::length(r),
                top: LengthPercentage::length(t),
                bottom: LengthPercentage::length(b),
            };
        }
        "margin" => {
            let [t, r, b, l] = sides(value).unwrap_or([0.; 4]);
            style.margin = Rect {
                left: LengthPercentageAuto::length(l),
                right: LengthPercentageAuto::length(r),
                top: LengthPercentageAuto::length(t),
                bottom: LengthPercentageAuto::length(b),
            };
        }
        "width" => style.size.width = length_or_auto(value).unwrap_or(Dimension::auto()),
        "height" => style.size.height = length_or_auto(value).unwrap_or(Dimension::auto()),
        "minWidth" => style.min_size.width = inset(value).unwrap_or(LengthPercentageAuto::auto()),
        "minHeight" => style.min_size.height = inset(value).unwrap_or(LengthPercentageAuto::auto()),
        "maxWidth" => style.max_size.width = inset(value).unwrap_or(LengthPercentageAuto::auto()),
        "maxHeight" => style.max_size.height = inset(value).unwrap_or(LengthPercentageAuto::auto()),
        "grow" => style.flex_grow = value.as_f64().unwrap_or(0.) as f32,
        "shrink" => style.flex_shrink = value.as_f64().unwrap_or(1.) as f32,
        "align" => style.align_items = align_items(value).unwrap_or(AlignItems::NORMAL),
        "self" => style.align_self = align_items(value),
        "justify" => {
            style.justify_content = match value.as_str() {
                Some("center") => JustifyContent::CENTER,
                Some("end") => JustifyContent::FLEX_END,
                Some("between") => JustifyContent::SPACE_BETWEEN,
                Some("around") => JustifyContent::SPACE_AROUND,
                Some("start") => JustifyContent::FLEX_START,
                _ => JustifyContent::NORMAL,
            }
        }
        "position" => {
            style.position = match value.as_str() {
                Some("absolute") => Position::Absolute,
                _ => Position::Relative,
            }
        }
        "top" => style.inset.top = inset(value).unwrap_or(LengthPercentageAuto::auto()),
        "left" => style.inset.left = inset(value).unwrap_or(LengthPercentageAuto::auto()),
        "right" => style.inset.right = inset(value).unwrap_or(LengthPercentageAuto::auto()),
        "bottom" => style.inset.bottom = inset(value).unwrap_or(LengthPercentageAuto::auto()),
        "display" => {
            style.display = match value.as_str() {
                Some("none") => Display::None,
                _ => Display::Flex,
            }
        }
        _ => return false,
    }
    true
}

/// Applies one look or motion prop, if it's one.
fn apply_look(node: &mut Node, key: &str, value: &Value) {
    let look = &mut node.look;
    match key {
        "fill" => look.fill = Paint::parse(value),
        "tint" => look.tint = Paint::parse(value),
        "color" => look.color = Paint::parse(value).unwrap_or(Paint::Fg(1.)),
        "material" => {
            look.material = match value.as_str() {
                Some("solid") => Material::Solid,
                Some("mica") => Material::Mica,
                Some("acrylic") => Material::Acrylic,
                Some("glass") => Material::Glass,
                _ => Material::None,
            }
        }
        "radius" => {
            // Top left, top right, bottom right, bottom left.
            look.radii = sides(value).unwrap_or([0.; 4]);
        }
        "border" => {
            look.border = value.as_array().and_then(|list| {
                let width = list.first()?.as_f64()? as f32;
                Some((width, Paint::parse(list.get(1)?)?))
            })
        }
        "opacity" => look.opacity = value.as_f64().unwrap_or(1.).clamp(0., 1.) as f32,
        "scale" => look.scale = value.as_f64().unwrap_or(1.).max(0.) as f32,
        "blur" => look.blur = value.as_f64().unwrap_or(0.).max(0.) as f32,
        "clip" => look.clip = value.as_bool().unwrap_or(false),
        "cursor" => look.cursor = value.as_str().map(String::from),
        "animate" => node.animate = value.as_bool().unwrap_or(true),
        "enter" => {
            node.enter = match value.as_str() {
                Some("point") => Enter::Point,
                Some("fade") => Enter::Fade,
                _ => Enter::Appear,
            }
        }
        "events" => {
            node.events = value.as_array().map_or(0, |list| {
                list.iter().fold(0, |events, e| {
                    events
                        | match e.as_str() {
                            Some("click") => CLICK,
                            Some("hover") => HOVER,
                            Some("scroll") => SCROLL,
                            _ => 0,
                        }
                })
            })
        }
        _ => (),
    }
}

#[cfg(test)]
mod tests {
    use super::script::Shell;
    use super::*;

    #[test]
    fn bentos_own_shell_lays_out_islands_and_the_clock_opens() {
        let shell = Shell::load(
            &[(String::from("DP-1"), (1920., 1080.))],
            None,
            serde_json::json!({ "gaps": 8 }),
        )
        .unwrap();
        let mut scene = shell.scene.borrow_mut();
        scene.lay_out();
        scene.advance(1., Some(1.));
        let shapes = |scene: &Scene| -> Vec<[f32; 4]> {
            scene
                .draw("DP-1")
                .into_iter()
                .filter_map(|item| match item {
                    Item::Shape {
                        rect,
                        material: Material::Glass,
                        ..
                    } => Some(rect),
                    _ => None,
                })
                .collect()
        };
        let islands = shapes(&scene);
        assert_eq!(islands.len(), 3, "{islands:?}");
        let middle = islands[1];
        assert!((middle[0] + middle[2] / 2. - 960.).abs() < 1., "{middle:?}");
        assert_eq!(middle[3], 36.);

        let clock = scene
            .hit("DP-1", [960., 20.], CLICK)
            .expect("the clock takes clicks");
        drop(scene);
        shell.send(&serde_json::json!({ "type": "click", "id": clock, "button": 1 }));
        let mut scene = shell.scene.borrow_mut();
        scene.lay_out();
        scene.advance(1. / 60., Some(1.));
        let opening = shapes(&scene)[1];
        assert!(opening[3] > 36. && opening[3] < 104., "{opening:?}");
        for _ in 0..120 {
            scene.advance(1. / 60., Some(1.));
        }
        let open = shapes(&scene)[1];
        assert_eq!([open[2], open[3]], [320., 104.]);
        assert!((open[0] + open[2] / 2. - 960.).abs() < 1., "{open:?}");
    }

    #[test]
    fn the_shell_keeps_the_top_for_its_islands_and_bentos_gaps() {
        let style = serde_json::json!({ "gaps": 16 });
        let shell = Shell::load(&[(String::from("DP-1"), (1920., 1080.))], None, style).unwrap();
        assert_eq!(
            shell.scene.borrow().reserved("DP-1"),
            [16. + 36., 0., 0., 0.]
        );
    }

    #[test]
    fn the_clocks_time_travels_into_the_calendars() {
        let shell = Shell::load(
            &[(String::from("DP-1"), (1920., 1080.))],
            None,
            serde_json::json!({ "gaps": 8 }),
        )
        .unwrap();
        let texts = |scene: &Scene| -> Vec<([f32; 2], f32, f32)> {
            scene
                .draw("DP-1")
                .into_iter()
                .filter_map(|item| match item {
                    Item::Text {
                        origin,
                        laid,
                        scale,
                        opacity,
                        ..
                    } => Some(([origin[0], laid.height * scale], scale, opacity)),
                    _ => None,
                })
                .collect()
        };
        let mut scene = shell.scene.borrow_mut();
        scene.lay_out();
        scene.advance(1., Some(1.));
        let clock = scene.hit("DP-1", [960., 20.], CLICK).unwrap();
        drop(scene);
        shell.send(&serde_json::json!({ "type": "click", "id": clock, "button": 1 }));

        let mut scene = shell.scene.borrow_mut();
        scene.lay_out();
        for _ in 0..3 {
            scene.advance(1. / 60., Some(1.));
        }
        // The big time starts as tall as the small one, scaled down, and grows.
        let starting: Vec<_> = texts(&scene)
            .into_iter()
            .filter(|(_, s, _)| *s < 0.9)
            .collect();
        assert!(!starting.is_empty(), "{:?}", texts(&scene));
        for _ in 0..120 {
            scene.advance(1. / 60., Some(1.));
        }
        for (_, scale, _) in texts(&scene) {
            assert!((scale - 1.).abs() < 0.01, "{:?}", texts(&scene));
        }
    }

    #[test]
    fn opening_the_clock_leaves_the_other_islands_where_they_are() {
        let shell = Shell::load(
            &[(String::from("DP-1"), (1920., 1080.))],
            None,
            serde_json::json!({ "gaps": 8 }),
        )
        .unwrap();
        let glass = |scene: &Scene| -> Vec<[f32; 4]> {
            scene
                .draw("DP-1")
                .into_iter()
                .filter_map(|item| match item {
                    Item::Shape {
                        rect,
                        material: Material::Glass,
                        ..
                    } => Some(rect),
                    _ => None,
                })
                .collect()
        };
        let mut scene = shell.scene.borrow_mut();
        scene.lay_out();
        scene.advance(1., Some(1.));
        let before = glass(&scene);
        let clock = scene.hit("DP-1", [960., 20.], CLICK).unwrap();
        drop(scene);
        shell.send(&serde_json::json!({ "type": "click", "id": clock, "button": 1 }));

        let mut scene = shell.scene.borrow_mut();
        scene.lay_out();
        for _ in 0..90 {
            scene.advance(1. / 60., Some(1.));
            let now = glass(&scene);
            for i in [0, 2] {
                assert_eq!(now[i], before[i], "island {i} moved");
            }
        }
    }
}
