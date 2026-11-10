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

/// Where an image comes from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImageSource {
    /// An icon from the icon theme, by name.
    Icon(String),
    /// An image file: PNG, JPEG, WebP or SVG.
    File(String),
}

impl ImageSource {
    /// Whether it's a symbolic icon, drawn in the node's colour.
    pub fn is_symbolic(&self) -> bool {
        matches!(self, Self::Icon(name) if name.ends_with("-symbolic"))
    }
}

#[derive(Debug, Clone)]
pub enum Kind {
    Box,
    Image(Option<ImageSource>),
    Text {
        text: String,
        style: TextStyle,
        /// The text laid out, at the width it was laid out for.
        laid: Option<(Option<f32>, Rc<Laid>)>,
        /// For text that's typed into, its caret and selection, which `text` follows.
        edit: Option<crate::layout::search::Field>,
    },
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
/// Its motion settling.
pub const SETTLE: u32 = 8;
/// The pointer coming into its place and leaving it, whatever else is there.
pub const NEAR: u32 = 16;
/// The pointer moving while a press on it is held.
pub const DRAG: u32 = 32;

// A node's springs: its place on its output, as the layout has it, opacity, corners, scale,
// blur, an offset from its place, and the size it's drawn at.
const X: usize = 0;
const Y: usize = 1;
const W: usize = 2;
const H: usize = 3;
const OPACITY: usize = 4;
const RADII: usize = 5;
const SCALE: usize = 9;
const BLUR: usize = 10;
const DX: usize = 11;
const DY: usize = 12;
const DW: usize = 13;
const DH: usize = 14;
const SPRINGS: usize = 15;

/// The props a script sets how fast they move by, and their springs.
const MOTIONS: [(&str, &[usize]); 7] = [
    ("frame", &[X, Y, W, H]),
    ("opacity", &[OPACITY]),
    ("radius", &[RADII, RADII + 1, RADII + 2, RADII + 3]),
    ("scale", &[SCALE]),
    ("blur", &[BLUR]),
    ("offset", &[DX, DY]),
    ("size", &[DW, DH]),
];

#[derive(Debug)]
pub struct Node {
    pub kind: Kind,
    taffy: taffy::NodeId,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    look: Look,
    springs: [Spring; SPRINGS],
    /// How long each spring takes to settle, in seconds, where it isn't bento's usual.
    responses: [Option<f64>; SPRINGS],
    placed: bool,
    /// Whether it was left out of the last layout, hidden or not displayed: shown again, it
    /// comes in at its place, not from where it was.
    left_out: bool,
    /// The size it's drawn at, around the middle of its place, or `None` for its place's size.
    draw_size: Option<[f32; 2]>,
    animate: bool,
    /// Whether it's left out: neither drawn nor hit.
    hidden: bool,
    /// Whether its drawn size was given a start before it was first laid out.
    placed_size: bool,
    /// Whether it was moving on the last frame, to tell when it settles.
    moving: bool,
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
    Image {
        rect: [f32; 4],
        source: ImageSource,
        /// The colour a symbolic icon is drawn in.
        color: [f32; 4],
        opacity: f32,
        /// How big it's drawn, as a fraction of its size.
        scale: f32,
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
    /// Nodes that settled, for their scripts.
    settled: Vec<NodeId>,
    /// How long a spring takes to settle, in seconds, where a node doesn't say: bento's own,
    /// as its windows move.
    pub base_response: f64,
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
            settled: Vec::new(),
            base_response: spring::RESPONSE,
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

    /// What `id` holds, for tests to walk the scene.
    #[cfg(test)]
    pub fn held(&self, id: NodeId) -> Vec<NodeId> {
        self.node(id)
            .map(|node| node.children.clone())
            .unwrap_or_default()
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

    /// Makes a node of `kind`, "box", "text", "input" or "image", with `props`.
    pub fn create(&mut self, kind: &str, props: &Value) -> NodeId {
        let kind = match kind {
            "image" => Kind::Image(None),
            "text" | "input" => Kind::Text {
                text: String::new(),
                style: TextStyle::default(),
                laid: None,
                edit: (kind == "input").then(Default::default),
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
        let mut springs = [Spring::new(0.); SPRINGS];
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
            left_out: false,
            responses: [None; SPRINGS],
            draw_size: None,
            animate: true,
            hidden: false,
            placed_size: false,
            moving: false,
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
            if let Kind::Image(source) = &mut node.kind {
                match key.as_str() {
                    "icon" => *source = value.as_str().map(|v| ImageSource::Icon(v.into())),
                    "src" => *source = value.as_str().map(|v| ImageSource::File(v.into())),
                    "size" => {
                        let size = Dimension::length(value.as_f64().unwrap_or(16.) as f32);
                        style.size = Size {
                            width: size,
                            height: size,
                        };
                        relayout = true;
                    }
                    _ => (),
                }
            }
            if let Kind::Text {
                text,
                style: text_style,
                laid,
                edit,
            } = &mut node.kind
            {
                let changed = match key.as_str() {
                    "text" => value
                        .as_str()
                        .filter(|v| *v != text.as_str())
                        .map(|v| {
                            *text = String::from(v);
                            // Set from outside, typing goes on at its end.
                            if let Some(edit) = edit {
                                *edit = Default::default();
                                edit.apply(&crate::layout::search::Edit::Insert(text.clone()));
                            }
                        })
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
        node.springs[SCALE].target = f64::from(node.look.scale);
        node.springs[BLUR].target = f64::from(node.look.blur);
        for i in 0..4 {
            node.springs[RADII + i].target = f64::from(node.look.radii[i]);
        }
        if let Some(offset) = props.get("offset") {
            let [dx, dy] = pair(offset).unwrap_or([0., 0.]);
            node.springs[DX].target = f64::from(dx);
            node.springs[DY].target = f64::from(dy);
        }
        // Shown or hidden, it's placed again, or left out.
        if props.get("hidden").is_some() {
            self.layout_dirty = true;
        }
        if let Some(size) = props.get("drawSize") {
            node.draw_size = pair(size);
            if let Some([w, h]) = node.draw_size {
                node.springs[DW].target = f64::from(w);
                node.springs[DH].target = f64::from(h);
            } else {
                // Its place's size, which the next layout sets.
                self.layout_dirty = true;
            }
        }
        if !node.placed {
            for spring in &mut node.springs[OPACITY..] {
                spring.jump(spring.target);
            }
        }
        // Where the look starts from, before it springs to its targets.
        if let Some(from) = props.get("jump").and_then(Value::as_object) {
            for (key, value) in from {
                let (springs, values): (&[usize], Vec<f32>) = match key.as_str() {
                    "opacity" => (
                        &[OPACITY],
                        value.as_f64().map(|v| v as f32).into_iter().collect(),
                    ),
                    "scale" => (
                        &[SCALE],
                        value.as_f64().map(|v| v as f32).into_iter().collect(),
                    ),
                    "blur" => (
                        &[BLUR],
                        value.as_f64().map(|v| v as f32).into_iter().collect(),
                    ),
                    "radius" => (
                        &[RADII, RADII + 1, RADII + 2, RADII + 3],
                        sides(value).map(Vec::from).unwrap_or_default(),
                    ),
                    "offset" => (&[DX, DY], pair(value).map(Vec::from).unwrap_or_default()),
                    "drawSize" => (&[DW, DH], pair(value).map(Vec::from).unwrap_or_default()),
                    _ => continue,
                };
                for (spring, value) in springs.iter().zip(values) {
                    node.springs[*spring].value = f64::from(value);
                    node.springs[*spring].velocity = 0.;
                }
                // Jumped before it's laid out, it starts from there.
                if key == "drawSize" && !node.placed {
                    node.placed_size = true;
                }
            }
        }
        self.changed = true;
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

    /// Takes a node away, at once: a script that wants it to go some way first takes it away
    /// once it has.
    pub fn remove(&mut self, id: NodeId) {
        self.destroy(id);
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
    }

    /// Where node `id`'s layout puts it on its output, laying the scene out first if it has to:
    /// x, y, width and height.
    pub fn layout(&mut self, id: NodeId) -> Option<[f32; 4]> {
        self.lay_out();
        // From the layout itself, so that it's known for what's left out too.
        let node = self.node(id)?;
        let own = self.tree.layout(node.taffy).ok()?;
        let (mut x, mut y) = (own.location.x, own.location.y);
        let size = [own.size.width, own.size.height];
        let mut parent = node.parent;
        while let Some(above) = parent.and_then(|id| self.node(id)) {
            let layout = self.tree.layout(above.taffy).ok()?;
            x += layout.location.x;
            y += layout.location.y;
            parent = above.parent;
        }
        Some([x, y, size[0], size[1]])
    }

    /// Sets the targets of `id` and what's in it from the layout, with its parent's top left at
    /// `origin`. Frames are absolute, so a node whose place on the output doesn't change doesn't
    /// move, whatever its parent does.
    fn place(&mut self, id: NodeId, origin: [f32; 2]) {
        let Some(node) = self.node(id) else { return };
        // What's left out has no place: shown again, it comes in at its place, not from where
        // it was.
        if node.hidden || !self.displayed(node) {
            self.leave_out(id);
            return;
        }
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
        // Drawn at its place's size, unless it says another.
        let size = node.draw_size.unwrap_or([target[2], target[3]]);
        if !node.placed {
            node.placed = true;
            for i in 0..4 {
                node.springs[i].jump(f64::from(target[i]));
            }
            if !node.placed_size {
                node.springs[DW].jump(f64::from(size[0]));
                node.springs[DH].jump(f64::from(size[1]));
            }
        } else if node.left_out {
            // Its looks are where they were, or where the script put them as it showed it.
            for i in 0..4 {
                node.springs[i].jump(f64::from(target[i]));
            }
            if node.draw_size.is_none() {
                node.springs[DW].jump(f64::from(size[0]));
                node.springs[DH].jump(f64::from(size[1]));
            }
        } else if !node.animate {
            for i in 0..4 {
                node.springs[i].jump(f64::from(target[i]));
            }
            if node.draw_size.is_none() {
                node.springs[DW].jump(f64::from(size[0]));
                node.springs[DH].jump(f64::from(size[1]));
            }
        }
        for i in 0..4 {
            node.springs[i].target = f64::from(target[i]);
        }
        node.springs[DW].target = f64::from(size[0]);
        node.springs[DH].target = f64::from(size[1]);
        node.left_out = false;
        for child in children {
            self.place(child, [target[0], target[1]]);
        }
    }

    /// Leaves `id` and what's in it out, so the next place they get they're put at.
    fn leave_out(&mut self, id: NodeId) {
        let Some(node) = self.node_mut(id) else {
            return;
        };
        if node.left_out || !node.placed {
            return;
        }
        node.left_out = true;
        for child in node.children.clone() {
            self.leave_out(child);
        }
    }

    /// Moves everything `dt` seconds on, and says whether anything is still moving. Nodes that
    /// hear of it and settled are kept for [`Scene::take_settled`].
    pub fn advance(&mut self, dt: f64, slowdown: Option<f64>) -> bool {
        let mut moving = false;
        for (id, node) in self.nodes.iter_mut().enumerate() {
            let Some(node) = node else { continue };
            let mut settled = true;
            for (i, spring) in node.springs.iter_mut().enumerate() {
                let response = node.responses[i].unwrap_or(self.base_response);
                spring.advance_with(dt, slowdown, response);
                settled &= spring.settle(0.01);
            }
            if settled && node.moving && node.events & SETTLE != 0 {
                self.settled.push(id as NodeId);
            }
            node.moving = !settled;
            moving |= !settled;
        }
        moving
    }

    /// The nodes that hear of their motion settling and settled since this was last asked.
    pub fn take_settled(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.settled)
    }

    /// Keeps `edges` along output `output`, top, right, bottom and left, for the shell.
    pub fn reserve(&mut self, output: &str, edges: [f32; 4]) {
        self.reserved.insert(String::from(output), edges);
    }

    /// What the shell keeps along output `output`'s edges.
    pub fn reserved(&self, output: &str) -> [f32; 4] {
        self.reserved.get(output).copied().unwrap_or_default()
    }

    /// Types `key`, a key event as the shell gets it, into input `id`, or with `{ "point": [x,
    /// y] }` puts its caret nearest that point on its output, with `shift` selecting up to it.
    /// Says what it holds now, with where its caret and selection are relative to it, or `None`
    /// if it isn't an input.
    pub fn edit(&mut self, id: NodeId, key: &Value) -> Option<Value> {
        use crate::layout::search::Edit;

        let frame = self.node(id).map(Self::drawn)?;
        let Self { nodes, fonts, .. } = self;
        let node = nodes.get_mut(id as usize)?.as_mut()?;
        let Kind::Text {
            text,
            style,
            laid,
            edit: Some(field),
        } = &mut node.kind
        else {
            return None;
        };
        let width = laid.as_ref().and_then(|(at, _)| *at);
        let flag = |name: &str| key[name].as_bool().unwrap_or(false);
        let shift = flag("shift");
        let before = (field.text().to_owned(), field.caret(), field.anchor());
        if let Some([x, y]) = key["point"]
            .as_array()
            .map(|p| [0, 1].map(|i| p.get(i).and_then(Value::as_f64).unwrap_or(0.) as f32))
        {
            let at = fonts.index_at(text, style, width, x - frame[0], y - frame[1]);
            field.place(at, shift);
        } else if key["pressed"].as_bool().unwrap_or(true) {
            let select = shift;
            let edit = match key["key"].as_str().unwrap_or_default() {
                "BackSpace" => Some(Edit::Backspace),
                "Delete" | "KP_Delete" => Some(Edit::Delete),
                "Left" | "KP_Left" => Some(Edit::Left { select }),
                "Right" | "KP_Right" => Some(Edit::Right { select }),
                "Home" | "KP_Home" => Some(Edit::Home { select }),
                "End" | "KP_End" => Some(Edit::End { select }),
                "a" | "A" if flag("ctrl") => Some(Edit::SelectAll),
                _ => key["text"]
                    .as_str()
                    .filter(|t| !t.is_empty() && !flag("ctrl") && !flag("alt") && !flag("super"))
                    .map(|t| Edit::Insert(String::from(t))),
            };
            if let Some(edit) = edit {
                field.apply(&edit);
            }
        }
        let changed = before != (field.text().to_owned(), field.caret(), field.anchor());
        if field.text() != text.as_str() {
            *text = field.text().to_owned();
            *laid = None;
            let _ = self.tree.mark_dirty(node.taffy);
            self.layout_dirty = true;
        }
        self.changed |= changed;
        let (caret, selection) = fonts.caret(text, style, width, field.caret(), field.anchor());
        Some(serde_json::json!({
            "text": text,
            "caret": field.caret(),
            "anchor": field.anchor(),
            "caretRect": caret,
            "selection": selection,
            "changed": changed,
        }))
    }

    /// Where node `id` is drawn on its output, with its own scale: x, y, width and height.
    pub fn frame(&self, id: NodeId) -> Option<[f32; 4]> {
        let node = self.node(id)?;
        let [x, y, w, h] = Self::drawn(node);
        let scale = node.springs[SCALE].value.max(0.) as f32;
        let (cx, cy) = (x + w / 2., y + h / 2.);
        Some([
            cx - w * scale / 2.,
            cy - h * scale / 2.,
            w * scale,
            h * scale,
        ])
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

    /// Where a node is drawn on its output: at the size it's drawn at, around the middle of its
    /// place, moved by its offset.
    fn drawn(node: &Node) -> [f32; 4] {
        let s = |i: usize| node.springs[i].value as f32;
        let (w, h) = (s(DW).max(0.), s(DH).max(0.));
        let cx = s(X) + s(W) / 2. + s(DX);
        let cy = s(Y) + s(H) / 2. + s(DY);
        [cx - w / 2., cy - h / 2., w, h]
    }

    /// What to draw for output `name`, back to front.
    pub fn draw(&self, output: &str) -> Vec<Item> {
        let mut items = Vec::new();
        for child in self.top_nodes(output) {
            self.draw_node(child, 1., None, Scaling::NONE, 0., &mut items);
        }
        items
    }

    /// Output `name`'s top nodes, from the bottom up.
    fn top_nodes(&self, output: &str) -> Vec<NodeId> {
        self.root(output)
            .and_then(|root| self.node(root))
            .map(|root| root.children.clone())
            .unwrap_or_default()
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
        if node.hidden || !self.displayed(node) {
            return;
        }
        let rect = Self::drawn(node);
        let opacity = opacity * node.springs[OPACITY].value.clamp(0., 1.) as f32;
        if opacity <= 0.003 {
            return;
        }
        let half = rect[2].min(rect[3]) / 2.;
        let radii: [f32; 4] =
            std::array::from_fn(|i| (node.springs[RADII + i].value as f32).clamp(0., half));
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
            Kind::Image(Some(source)) => {
                items.push(Item::Image {
                    rect,
                    source: source.clone(),
                    color: node.look.color.resolve(&self.theme),
                    opacity,
                    scale: scaling.by,
                    blur,
                    clip,
                });
            }
            Kind::Image(None) => (),
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

        let clip = if node.look.clip {
            Some(Clip { rect, radii })
        } else {
            clip
        };
        for child in &node.children {
            self.draw_node(*child, opacity, clip, scaling, blur, items);
        }
    }

    /// The nodes at `point` on output `output`, from the bottom up: those whose drawn shape,
    /// rounded, scaled and cut to what clips it, holds the point, and that aren't hidden or
    /// leaving.
    fn pick(&self, output: &str, point: [f32; 2]) -> Vec<NodeId> {
        let mut found = Vec::new();
        for child in self.top_nodes(output) {
            self.pick_node(child, point, Scaling::NONE, None, &mut found);
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
        if node.hidden || node.springs[OPACITY].target <= 0. || !self.displayed(node) {
            return;
        }
        let rect = Self::drawn(node);
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

    /// The nodes at `point` on output `output` that hear the pointer near: all of them, whatever
    /// is over or under them, since they take nothing from it.
    pub fn near(&self, output: &str, point: [f32; 2]) -> Vec<NodeId> {
        let mut found = self.pick(output, point);
        found.retain(|id| self.node(*id).is_some_and(|n| n.events & NEAR != 0));
        found
    }

    /// Where the texts that say `wanted`, and the icons called it, show on output `output`: each
    /// one's frame, for tests to find what to press.
    #[cfg(test)]
    pub fn find(&self, output: &str, wanted: &str) -> Vec<[f32; 4]> {
        self.seen(output, wanted)
            .into_iter()
            .filter(|(_, _, shown)| *shown)
            .map(|(frame, _, _)| frame)
            .collect()
    }

    /// The texts that say `wanted`, and the icons called it, on output `output`, whether they
    /// show or not: each one's frame, how opaque it's drawn now, and whether it's on its way to
    /// showing.
    #[cfg(test)]
    pub fn seen(&self, output: &str, wanted: &str) -> Vec<([f32; 4], f32, bool)> {
        fn walk(
            scene: &Scene,
            id: NodeId,
            wanted: &str,
            opacity: f32,
            shown: bool,
            found: &mut Vec<([f32; 4], f32, bool)>,
        ) {
            let Some(node) = scene.node(id) else { return };
            if node.hidden || !scene.displayed(node) {
                return;
            }
            let opacity = opacity * node.springs[OPACITY].value.clamp(0., 1.) as f32;
            let shown = shown && node.springs[OPACITY].target > 0.;
            let is_it = match &node.kind {
                Kind::Text { text, .. } => text == wanted,
                Kind::Image(Some(ImageSource::Icon(name))) => name == wanted,
                _ => false,
            };
            if let (true, Some(frame)) = (is_it, scene.frame(id)) {
                if frame[2] > 0. && frame[3] > 0. {
                    found.push((frame, opacity, shown));
                }
            }
            for child in &node.children {
                walk(scene, *child, wanted, opacity, shown, found);
            }
        }
        let mut found = Vec::new();
        if let Some(root) = self.root(output) {
            walk(self, root, wanted, 1., true, &mut found);
        }
        found
    }

    /// Whether `node` has a place in the layout: with `display: "none"` it has none, and
    /// neither it nor what it holds is drawn or hit.
    fn displayed(&self, node: &Node) -> bool {
        self.tree
            .style(node.taffy)
            .is_ok_and(|style| style.display != Display::None)
    }

    /// Whether node `id` takes any of `events`.
    pub fn takes(&self, id: NodeId, events: u32) -> bool {
        self.node(id).is_some_and(|n| n.events & events != 0)
    }

    /// The topmost node at `point` on output `output` that takes any of `events`.
    pub fn hit(&self, output: &str, point: [f32; 2], events: u32) -> Option<NodeId> {
        self.pick(output, point)
            .into_iter()
            .rev()
            .find(|id| self.node(*id).is_some_and(|n| n.events & events != 0))
    }

    /// Whether the shell is at `point` on output `output`, so it takes the pointer there from
    /// what's below: a shape or something that takes the pointer's events is.
    pub fn covers(&self, output: &str, point: [f32; 2]) -> bool {
        self.pick(output, point).into_iter().any(|id| {
            self.node(id).is_some_and(|n| {
                n.events & (CLICK | HOVER | SCROLL) != 0
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

/// Two numbers, like a size or an offset.
fn pair(value: &Value) -> Option<[f32; 2]> {
    match value.as_array()?.as_slice() {
        [a, b, ..] => Some([a.as_f64()? as f32, b.as_f64()? as f32]),
        _ => None,
    }
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
    let Kind::Text {
        text,
        style,
        laid,
        edit,
    } = &mut node.kind
    else {
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
    // Text that's typed into is a line tall even while it's empty.
    let min_height = if edit.is_some() {
        style.size * 1.25
    } else {
        0.
    };
    taffy::LayoutOutput::from_outer_size(Size {
        width: input.known_dimensions.width.unwrap_or(laid.width.ceil()),
        height: input
            .known_dimensions
            .height
            .unwrap_or(laid.height.ceil().max(min_height)),
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
        "hidden" => node.hidden = value.as_bool().unwrap_or(false),
        // How long its moves take, in seconds: all of them, or with `transition` each kind.
        "response" => {
            let response = value.as_f64().filter(|r| *r > 0.);
            node.responses = [response; SPRINGS];
        }
        "transition" => {
            if let Some(kinds) = value.as_object() {
                for (kind, springs) in MOTIONS {
                    if let Some(response) = kinds.get(kind) {
                        let response = response.as_f64().filter(|r| *r > 0.);
                        for spring in springs {
                            node.responses[*spring] = response;
                        }
                    }
                }
            }
        }
        "cursor" => look.cursor = value.as_str().map(String::from),
        "animate" => node.animate = value.as_bool().unwrap_or(true),
        "events" => {
            node.events = value.as_array().map_or(0, |list| {
                list.iter().fold(0, |events, e| {
                    events
                        | match e.as_str() {
                            Some("click") => CLICK,
                            Some("hover") => HOVER,
                            Some("scroll") => SCROLL,
                            Some("settle") => SETTLE,
                            Some("near") => NEAR,
                            Some("drag") => DRAG,
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
        assert!(opening[3] > 36. && opening[3] < 478., "{opening:?}");
        for _ in 0..120 {
            scene.advance(1. / 60., Some(1.));
        }
        let open = shapes(&scene)[1];
        // As big as the calendar in it, with its room around.
        assert_eq!([open[2], open[3]], [340., 478.]);
        assert!((open[0] + open[2] / 2. - 960.).abs() < 1., "{open:?}");
    }

    #[test]
    fn typing_into_an_input_edits_it_with_its_caret_and_selection() {
        let mut scene = Scene::new();
        let root = scene.set_output("DP-1", (1920., 1080.));
        let input = scene.create("input", &serde_json::json!({ "size": 20 }));
        scene.append(root, input, None);
        scene.lay_out();
        let key = |key: &str, text: &str, shift: bool| serde_json::json!({ "key": key, "text": text, "pressed": true, "shift": shift });
        for c in ["h", "e", "y"] {
            scene.edit(input, &key(c, c, false)).unwrap();
        }
        let state = scene.edit(input, &key("BackSpace", "", false)).unwrap();
        assert_eq!(state["text"], "he");
        assert_eq!(state["caret"], 2);
        let state = scene.edit(input, &key("Left", "", true)).unwrap();
        assert_eq!(
            (state["caret"].as_u64(), state["anchor"].as_u64()),
            (Some(1), Some(2))
        );
        assert_eq!(state["selection"].as_array().unwrap().len(), 1);
        let state = scene.edit(
            input,
            &serde_json::json!({ "key": "a", "text": "a", "ctrl": true }),
        );
        assert_eq!(state.unwrap()["anchor"], 0);
        // The caret goes where a press puts it: the start, at the input's left edge.
        scene.lay_out();
        scene.advance(1., Some(1.));
        let frame = scene.frame(input).unwrap();
        let state = scene.edit(
            input,
            &serde_json::json!({ "point": [frame[0], frame[1] + 5.] }),
        );
        assert_eq!(state.unwrap()["caret"], 0);
        assert!(
            scene.edit(root, &key("x", "x", false)).is_none(),
            "only inputs are typed into"
        );
    }

    fn panel(scene: &mut Scene, props: serde_json::Value) -> NodeId {
        let root = scene.set_output("DP-1", (1920., 1080.));
        let mut all = serde_json::json!({
            "material": "glass", "position": "absolute",
            "top": 200, "left": 680, "width": 560, "height": 120,
        });
        all.as_object_mut()
            .unwrap()
            .extend(props.as_object().unwrap().clone());
        let panel = scene.create("box", &all);
        scene.append(root, panel, None);
        scene.lay_out();
        panel
    }

    fn settle(scene: &mut Scene) {
        for _ in 0..240 {
            scene.advance(1. / 60., Some(1.));
        }
    }

    #[test]
    fn a_drawn_size_and_an_offset_draw_it_around_its_place() {
        let mut scene = Scene::new();
        let node = panel(
            &mut scene,
            serde_json::json!({ "drawSize": [100, 40], "offset": [10, -5] }),
        );
        settle(&mut scene);
        assert_eq!(scene.frame(node).unwrap(), [920., 235., 100., 40.]);
        // Without one, it's drawn at its place's size, and moves there.
        scene.set(
            node,
            &serde_json::json!({ "drawSize": null, "offset": [0, 0] }),
        );
        scene.lay_out();
        scene.advance(1. / 60., Some(1.));
        let moving = scene.frame(node).unwrap();
        assert!(moving[2] > 100. && moving[2] < 560., "{moving:?}");
        settle(&mut scene);
        assert_eq!(scene.frame(node).unwrap(), [680., 200., 560., 120.]);
    }

    #[test]
    fn a_jump_starts_it_somewhere_before_it_moves() {
        let mut scene = Scene::new();
        let node = panel(&mut scene, serde_json::json!({}));
        settle(&mut scene);
        scene.set(node, &serde_json::json!({ "jump": { "drawSize": [0, 0] } }));
        assert_eq!(scene.frame(node).unwrap(), [960., 260., 0., 0.]);
        settle(&mut scene);
        assert_eq!(scene.frame(node).unwrap(), [680., 200., 560., 120.]);
        // Before it's first laid out too.
        let root = scene.root("DP-1").unwrap();
        let other = scene.create(
            "box",
            &serde_json::json!({
                "material": "glass", "width": 50, "height": 50, "jump": { "drawSize": [0, 0] },
            }),
        );
        scene.append(root, other, None);
        scene.lay_out();
        assert_eq!(scene.frame(other).unwrap()[2], 0.);
    }

    #[test]
    fn it_moves_as_fast_as_bentos_windows_do() {
        let mut scene = Scene::new();
        let node = panel(&mut scene, serde_json::json!({ "events": ["settle"] }));
        settle(&mut scene);
        scene.take_settled();
        // bento's window movement: a critically damped spring of stiffness 800.
        scene.base_response = std::f64::consts::TAU / 800f64.sqrt();
        scene.set(node, &serde_json::json!({ "opacity": 0 }));
        for _ in 0..18 {
            scene.advance(1. / 60., Some(1.));
        }
        assert_eq!(
            scene.take_settled(),
            [node],
            "settled in under a third of a second"
        );
    }

    #[test]
    fn hidden_it_is_neither_drawn_nor_hit_at_once() {
        let mut scene = Scene::new();
        let node = panel(&mut scene, serde_json::json!({}));
        settle(&mut scene);
        assert!(scene.covers("DP-1", [960., 260.]));
        scene.set(node, &serde_json::json!({ "hidden": true }));
        assert!(scene.draw("DP-1").is_empty());
        assert!(!scene.covers("DP-1", [960., 260.]));
    }

    #[test]
    fn it_says_when_its_motion_settles_once_and_moves_as_fast_as_it_is_told() {
        let mut scene = Scene::new();
        let node = panel(&mut scene, serde_json::json!({ "events": ["settle"] }));
        settle(&mut scene);
        scene.take_settled();
        scene.set(
            node,
            &serde_json::json!({ "opacity": 0, "transition": { "opacity": 0.05 } }),
        );
        for _ in 0..10 {
            scene.advance(1. / 60., Some(1.));
        }
        assert_eq!(
            scene.take_settled(),
            [node],
            "a fast fade is done in ten frames"
        );
        settle(&mut scene);
        assert!(scene.take_settled().is_empty(), "once");
    }

    #[test]
    fn what_isnt_displayed_is_neither_drawn_nor_hit_with_what_it_holds() {
        let mut scene = Scene::new();
        let root = scene.set_output("DP-1", (1920., 1080.));
        let holder = scene.create("box", &serde_json::json!({ "display": "none" }));
        let held = scene.create(
            "box",
            &serde_json::json!({
                "position": "absolute", "top": 0, "left": 0, "width": 40, "height": 40,
                "material": "glass", "events": ["click"],
            }),
        );
        scene.append(root, holder, None);
        scene.append(holder, held, None);
        scene.lay_out();
        scene.advance(1., Some(1.));
        assert!(scene.draw("DP-1").is_empty(), "{:?}", scene.draw("DP-1"));
        assert!(!scene.covers("DP-1", [0., 0.]));

        // Displayed again, it's there.
        scene.set(holder, &serde_json::json!({ "display": "flex" }));
        scene.lay_out();
        scene.advance(1., Some(1.));
        assert_eq!(scene.draw("DP-1").len(), 1);
        assert!(scene.covers("DP-1", [20., 20.]));
    }

    #[test]
    fn what_shows_again_comes_in_at_its_place_rather_than_from_where_it_was_left_out() {
        let mut scene = Scene::new();
        let root = scene.set_output("DP-1", (1920., 1080.));
        let row = scene.create(
            "box",
            &serde_json::json!({ "position": "absolute", "top": 100, "left": 600, "gap": 10 }),
        );
        let first = scene.create("box", &serde_json::json!({ "width": 50, "height": 20 }));
        let shown_later = scene.create(
            "box",
            &serde_json::json!({ "width": 30, "height": 20, "display": "none", "fill": "fg" }),
        );
        let hidden_later = scene.create(
            "box",
            &serde_json::json!({ "width": 30, "height": 20, "fill": "fg" }),
        );
        scene.append(root, row, None);
        scene.append(row, first, None);
        scene.append(row, shown_later, None);
        scene.append(row, hidden_later, None);
        scene.lay_out();
        scene.advance(1., Some(1.));
        scene.set(hidden_later, &serde_json::json!({ "hidden": true }));
        scene.lay_out();
        scene.advance(1., Some(1.));

        // The row moves while they're left out.
        scene.set(row, &serde_json::json!({ "left": 1200 }));
        scene.lay_out();
        scene.advance(1., Some(1.));

        // Where it goes is known while it's left out.
        assert_eq!(scene.layout(hidden_later), Some([1260., 100., 30., 20.]));

        scene.set(shown_later, &serde_json::json!({ "display": "flex" }));
        scene.set(hidden_later, &serde_json::json!({ "hidden": false }));
        scene.lay_out();
        scene.advance(1. / 60., Some(1.));
        assert_eq!(scene.frame(shown_later).unwrap()[0], 1260.);
        assert_eq!(scene.frame(hidden_later).unwrap()[0], 1300.);
    }

    #[test]
    fn what_hears_the_pointer_near_takes_nothing_from_it() {
        let mut scene = Scene::new();
        let root = scene.set_output("DP-1", (1920., 1080.));
        let zone = scene.create(
            "box",
            &serde_json::json!({
                "position": "absolute", "top": 0, "left": 0, "right": 0, "height": 2,
                "events": ["near", "settle"],
            }),
        );
        scene.append(root, zone, None);
        // Something of the shell's over it doesn't keep the pointer from it.
        let island = scene.create(
            "box",
            &serde_json::json!({
                "position": "absolute", "top": 0, "left": 0, "width": 100, "height": 36,
                "material": "glass",
            }),
        );
        scene.append(root, island, None);
        scene.lay_out();
        scene.advance(1., Some(1.));
        assert_eq!(scene.near("DP-1", [500., 1.]), vec![zone]);
        assert_eq!(scene.near("DP-1", [50., 1.]), vec![zone]);
        assert!(scene.near("DP-1", [500., 5.]).is_empty());
        // What's under it keeps the pointer.
        assert!(!scene.covers("DP-1", [500., 1.]));
        assert!(scene.covers("DP-1", [50., 1.]));
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
