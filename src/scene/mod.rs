//! bento's own shell, as a scene that bento lays out, animates and draws itself.
//!
//! The scene is a tree of nodes per output: surfaces, placed on the output the way layer-shell
//! places other programs' surfaces, and in them boxes, shapes with a material, text and images.
//! Scripts build and change it (see [`script`]); bento lays each surface out with flexbox, moves
//! each node's frame, corners and opacity with springs towards where the layout puts them, and
//! draws it in the same frame as everything else, so the shell's motion never waits on anything.
//! Props are CSS's, by their names in JavaScript, with bento's own for what CSS has no word for.
//!
//! A node's frame is where it is on its output, so a node moves only when its own place does,
//! and what's in an island moves with it as their places move together. A node that grows out of a
//! point grows as a circle until it's as wide as its narrower side, then stretches to its
//! proportions, and shrinks back the same way.

pub mod apps;
pub mod host;
pub mod modules;
pub mod reference;
pub mod script;
pub mod settings;
mod spring;
pub mod text;
pub mod vk;

use std::rc::Rc;

use serde_json::Value;
use taffy::prelude::*;
use taffy::{CompactLength, Overflow, Point};

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

/// Where a surface goes among what else is on its output: one of layer-shell's layers, below or
/// above other programs' surfaces in it. From the back to the front.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Band {
    BackgroundBelow,
    BackgroundAbove,
    BottomBelow,
    BottomAbove,
    TopBelow,
    TopAbove,
    OverlayBelow,
    OverlayAbove,
}

impl Band {
    pub const ALL: [Band; 8] = [
        Band::BackgroundBelow,
        Band::BackgroundAbove,
        Band::BottomBelow,
        Band::BottomAbove,
        Band::TopBelow,
        Band::TopAbove,
        Band::OverlayBelow,
        Band::OverlayAbove,
    ];
}

/// How much of the edge a surface is held to it keeps from the windows.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Reserve {
    #[default]
    None,
    /// Its size there and its margin from the edge.
    Own,
    Pixels(f32),
}

/// Where a surface is on its output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// layer-shell's background, bottom, top and overlay, as 0 to 3.
    layer: u8,
    /// Below other programs' surfaces in its layer when negative, above them otherwise; higher
    /// is in front among the shell's own.
    z: f32,
    /// The edges it's held to: top, right, bottom and left.
    anchor: [bool; 4],
    reserve: Reserve,
    /// The node it's put beside instead, as a flyout over its button is.
    attach: Option<Attach>,
    /// Where it was put: its top left on its output.
    origin: [f32; 2],
}

/// Where a surface goes beside a node: on which of its sides, how far, and lined up with its
/// start, middle or end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Attach {
    /// What it's beside.
    to: AttachTo,
    /// Top, right, bottom or left, as 0 to 3.
    side: usize,
    /// Its start, middle or end along that side, as 0, 0.5 or 1.
    align: f32,
    gap: f32,
}

/// What a surface is put beside.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AttachTo {
    /// A node of the shell's.
    Node(NodeId),
    /// A point on the output, like where a menu was asked for.
    Point([f32; 2]),
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            layer: 2,
            z: 1.,
            anchor: [false; 4],
            reserve: Reserve::None,
            attach: None,
            origin: [0., 0.],
        }
    }
}

impl Placement {
    pub fn band(&self) -> Band {
        let above = self.z >= 0.;
        match (self.layer, above) {
            (0, false) => Band::BackgroundBelow,
            (0, true) => Band::BackgroundAbove,
            (1, false) => Band::BottomBelow,
            (1, true) => Band::BottomAbove,
            (2, false) => Band::TopBelow,
            (2, true) => Band::TopAbove,
            (_, false) => Band::OverlayBelow,
            (_, true) => Band::OverlayAbove,
        }
    }

    /// The edge it keeps from the windows: the one it's held to, alone or with both beside it.
    fn reserved_edge(&self) -> Option<usize> {
        let held = self.anchor.iter().filter(|a| **a).count();
        (0..4).find(|&edge| {
            let opposite = (edge + 2) % 4;
            self.anchor[edge]
                && !self.anchor[opposite]
                && (held == 1 || (held == 3 && self.anchor[(edge + 1) % 4]))
        })
    }
}

/// Text styles a node sets itself, which what it holds inherits.
#[derive(Debug, Clone, Default, PartialEq)]
struct TextOwn {
    size: Option<f32>,
    weight: Option<f32>,
    family: Option<String>,
    align: Option<f32>,
    tabular: Option<bool>,
    one_line: Option<bool>,
}

impl TextOwn {
    fn over(&self, base: &TextStyle) -> TextStyle {
        TextStyle {
            size: self.size.unwrap_or(base.size),
            weight: self.weight.unwrap_or(base.weight),
            family: self.family.clone().unwrap_or_else(|| base.family.clone()),
            align: self.align.unwrap_or(base.align),
            tabular: self.tabular.unwrap_or(base.tabular),
            one_line: self.one_line.unwrap_or(base.one_line),
        }
    }
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
    /// A live copy of a window, by its id, fitted into the node and centred.
    Copy(Option<u64>),
    Image(Option<ImageSource>),
    Text {
        text: String,
        /// How it's set: its own text styles over what it inherits.
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
    /// A border's colour set before its width.
    border_paint: Option<Paint>,
    opacity: f32,
    /// How big it's drawn, around its middle, as a fraction of its size.
    scale: f32,
    /// How blurred it's drawn, in logical pixels.
    blur: f32,
    clip: bool,
    /// Whether what it holds scrolls in it, cut to its shape.
    scroll: bool,
    /// The text colour, which what it holds inherits, unless it's the inherited one.
    color: Option<Paint>,
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
            border_paint: None,
            opacity: 1.,
            scale: 1.,
            blur: 0.,
            clip: false,
            scroll: false,
            color: None,
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
/// Where its layout puts it changing.
pub const LAYOUT: u32 = 64;
/// How far what it holds is scrolled, how far it can be, or how tall it is, changing.
pub const SCROLLED: u32 = 128;

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
    ("translate", &[DX, DY]),
    ("drawSize", &[DW, DH]),
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
    /// The size it's drawn at, each way in pixels and in fractions of its place's, or `None` for
    /// its place's size.
    draw_size: Option<[Offset; 2]>,
    /// A drawn size it jumped to before it was laid out, in fractions of a size it didn't have
    /// yet, which it starts from once it's laid out.
    jumped_size: Option<[Offset; 2]>,
    /// Where in its place a shape drawn at another size is held, as fractions of its width and
    /// height: its middle, `[0.5, 0.5]`, unless it says an edge or a corner.
    draw_from: [f32; 2],
    /// How far it's drawn from its place, each way in pixels and in fractions of its size.
    translate: [Offset; 2],
    /// A translation it jumped to before it was laid out, in fractions of a size it didn't have
    /// yet, which it starts from once it's laid out.
    jumped_translate: Option<[Offset; 2]>,
    animate: bool,
    /// Whether it's left out: neither drawn nor hit.
    hidden: bool,
    /// Whether its drawn size was given a start before it was first laid out.
    placed_size: bool,
    /// Whether it was moving on the last frame, to tell when it settles.
    moving: bool,
    pub events: u32,
    /// Its own text styles.
    text_own: TextOwn,
    /// Where it goes on its output, if it's a surface.
    placement: Option<Placement>,
    /// Where its layout put it when its script last heard, for those that hear it.
    told: Option<[f32; 4]>,
    /// How far what it holds is scrolled up, if it scrolls.
    scrolled: f32,
    /// How far what it holds reaches past its bottom, as far as it scrolls.
    reach: f32,
    /// How far it was scrolled, how far it could be and how tall it was when its script last
    /// heard, for those that hear it.
    told_scroll: Option<[f32; 3]>,
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
        /// Whether its scale is at rest, rather than on its way somewhere.
        still: bool,
        blur: f32,
        clip: Option<Clip>,
    },
}

impl Item {
    /// Whether it draws just as `other` does.
    pub fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Item::Shape {
                    rect,
                    radii,
                    material,
                    fill,
                    border,
                    opacity,
                    blur,
                    clip,
                },
                Item::Shape {
                    rect: rect2,
                    radii: radii2,
                    material: material2,
                    fill: fill2,
                    border: border2,
                    opacity: opacity2,
                    blur: blur2,
                    clip: clip2,
                },
            ) => {
                (rect, radii, material, fill, border, opacity, blur, clip)
                    == (
                        rect2, radii2, material2, fill2, border2, opacity2, blur2, clip2,
                    )
            }
            (
                Item::Image {
                    rect,
                    source,
                    color,
                    opacity,
                    scale,
                    blur,
                    clip,
                },
                Item::Image {
                    rect: rect2,
                    source: source2,
                    color: color2,
                    opacity: opacity2,
                    scale: scale2,
                    blur: blur2,
                    clip: clip2,
                },
            ) => {
                (rect, source, color, opacity, scale, blur, clip)
                    == (rect2, source2, color2, opacity2, scale2, blur2, clip2)
            }
            (
                Item::Text {
                    origin,
                    laid,
                    color,
                    opacity,
                    scale,
                    still,
                    blur,
                    clip,
                },
                Item::Text {
                    origin: origin2,
                    laid: laid2,
                    color: color2,
                    opacity: opacity2,
                    scale: scale2,
                    still: still2,
                    blur: blur2,
                    clip: clip2,
                },
            ) => {
                // The same layout, not one that reads the same: text laid out again is new.
                Rc::ptr_eq(laid, laid2)
                    && (origin, color, opacity, scale, still, blur, clip)
                        == (origin2, color2, opacity2, scale2, still2, blur2, clip2)
            }
            _ => false,
        }
    }

    /// The rectangle it draws within, in the output's logical coordinates: its own, with its
    /// blur's spread, cut to what clips it.
    pub fn bounds(&self) -> [f32; 4] {
        let (rect, blur, clip) = match self {
            Item::Shape {
                rect, blur, clip, ..
            } => (*rect, *blur, clip),
            Item::Image {
                rect, blur, clip, ..
            } => (*rect, *blur, clip),
            Item::Text {
                origin,
                laid,
                scale,
                blur,
                clip,
                ..
            } => (
                [
                    origin[0],
                    origin[1],
                    laid.reach * scale,
                    laid.height * scale,
                ],
                *blur,
                clip,
            ),
        };
        // Smoothed edges, glyphs that reach past their line, and a blur's reach.
        let spread = 4. + blur * 3.;
        let (mut x0, mut y0) = (rect[0] - spread, rect[1] - spread);
        let (mut x1, mut y1) = (rect[0] + rect[2] + spread, rect[1] + rect[3] + spread);
        if let Some(clip) = clip {
            x0 = x0.max(clip.rect[0] - 1.);
            y0 = y0.max(clip.rect[1] - 1.);
            x1 = x1.min(clip.rect[0] + clip.rect[2] + 1.);
            y1 = y1.min(clip.rect[1] + clip.rect[3] + 1.);
        }
        [x0, y0, (x1 - x0).max(0.), (y1 - y0).max(0.)]
    }
}

/// A live copy of a window, as bento draws it over the rest of its band: where, how faded, with
/// what corners, and cut to what.
#[derive(Debug, Clone, PartialEq)]
pub struct CopyItem {
    pub window: u64,
    pub rect: [f32; 4],
    pub radius: f32,
    pub opacity: f32,
    pub clip: Option<Clip>,
}

/// A rounded rectangle that what's drawn is cut to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clip {
    pub rect: [f32; 4],
    pub radii: [f32; 4],
}

impl Clip {
    /// This and `outer` both, or nothing if they don't meet. A corner is rounded as the one it's
    /// the corner of is, and square where one of them cuts across the other.
    fn within(self, outer: Option<Self>) -> Option<Self> {
        let Some(outer) = outer else {
            return Some(self);
        };
        // Left, top, right and bottom.
        let sides = |rect: [f32; 4]| [rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]];
        let (own, other) = (sides(self.rect), sides(outer.rect));
        let both = [
            own[0].max(other[0]),
            own[1].max(other[1]),
            own[2].min(other[2]),
            own[3].min(other[3]),
        ];
        if both[2] <= both[0] || both[3] <= both[1] {
            return None;
        }
        // The sides each corner is between, from the top left on.
        let corners = [[0, 1], [2, 1], [2, 3], [0, 3]];
        let radii = std::array::from_fn(|i| {
            let radius = |sides: [f32; 4], radii: [f32; 4]| {
                let is_its = corners[i].iter().all(|side| sides[*side] == both[*side]);
                if is_its {
                    radii[i]
                } else {
                    0.
                }
            };
            f32::max(radius(own, self.radii), radius(other, outer.radii))
        });
        Some(Self {
            rect: [both[0], both[1], both[2] - both[0], both[3] - both[1]],
            radii,
        })
    }
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
    /// Whether text styles changed, which what's held inherits.
    styles_dirty: bool,
    /// Nodes that settled, for their scripts.
    settled: Vec<NodeId>,
    /// Nodes whose layout put them somewhere else, and where, for their scripts.
    laid_out: Vec<(NodeId, [f32; 4])>,
    /// What hears how far it's scrolled and was scrolled, or reached, otherwise since last asked.
    scrolled_out: Vec<(NodeId, [f32; 3])>,
    /// How long a spring takes to settle, in seconds, where a node doesn't say: bento's own,
    /// as its windows move.
    pub base_response: f64,
    /// What the shell keeps for itself along each output's edges, that windows stay out of:
    /// top, right, bottom and left, from the surfaces that reserve room.
    reserved: std::collections::HashMap<String, [f32; 4]>,
    /// bento's style that the shell lines up with: its gaps and corner radius.
    pub style: Value,
    /// Whether the shell wants the keyboard, which then doesn't go to the apps.
    pub wants_keyboard: bool,
    /// Icons the script will show, with their sizes, to load ahead of their first frame.
    pub preloads: Vec<(ImageSource, f32)>,
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
            styles_dirty: true,
            settled: Vec::new(),
            laid_out: Vec::new(),
            scrolled_out: Vec::new(),
            base_response: spring::RESPONSE,
            reserved: std::collections::HashMap::new(),
            style: Value::Null,
            wants_keyboard: false,
            preloads: Vec::new(),
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

    /// How big output `name` is, in logical pixels.
    pub fn output_size(&self, name: &str) -> Option<(f32, f32)> {
        self.roots.iter().find(|r| r.output == name).map(|r| r.size)
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

    /// Makes a node of `kind`, "surface", "box", "text", "input" or "image", with `props`.
    pub fn create(&mut self, kind: &str, props: &Value) -> NodeId {
        let placement = (kind == "surface").then(Placement::default);
        let kind = match kind {
            "image" => Kind::Image(None),
            "text" | "input" => Kind::Text {
                text: String::new(),
                style: TextStyle::default(),
                laid: None,
                edit: (kind == "input").then(Default::default),
            },
            "copy" => Kind::Copy(None),
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
            .new_leaf_with_context(
                // Every node is what its absolute children are placed and sized in.
                Style {
                    position: Position::Relative,
                    ..Style::default()
                },
                id,
            )
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
            jumped_size: None,
            draw_from: [0.5, 0.5],
            translate: Default::default(),
            jumped_translate: None,
            animate: true,
            hidden: false,
            placed_size: false,
            moving: false,
            events: 0,
            text_own: TextOwn::default(),
            placement,
            told: None,
            told_scroll: None,
            scrolled: 0.,
            reach: 0.,
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
            if let (Kind::Copy(window), "window") = (&mut node.kind, key.as_str()) {
                *window = value.as_u64();
            }
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
            if apply_text(&mut node.text_own, key, value) {
                self.styles_dirty = true;
                relayout = true;
            }
            if let Some(placement) = &mut node.placement {
                if apply_placement(placement, key, value) {
                    relayout = true;
                }
            }
            if let Kind::Text {
                text, laid, edit, ..
            } = &mut node.kind
            {
                if key == "text" {
                    let value = match value {
                        Value::String(v) => v.clone(),
                        Value::Number(n) => n.to_string(),
                        _ => String::new(),
                    };
                    if value != *text {
                        *text = value;
                        // Set from outside, typing goes on at its end.
                        if let Some(edit) = edit {
                            *edit = Default::default();
                            edit.apply(&crate::layout::search::Edit::Insert(text.clone()));
                        }
                        *laid = None;
                        relayout = true;
                        let _ = self.tree.mark_dirty(node.taffy);
                    }
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
        if let Some(offset) = props.get("translate") {
            node.translate = offsets(offset).unwrap_or_default();
            let size = [node.springs[W].target, node.springs[H].target].map(|x| x as f32);
            let [dx, dy] = resolve(node.translate, size);
            node.springs[DX].target = f64::from(dx);
            node.springs[DY].target = f64::from(dy);
        }
        // Shown or hidden, it's placed again, or left out.
        if props.get("hidden").is_some() || props.get("visibility").is_some() {
            self.layout_dirty = true;
        }
        if let Some(from) = props.get("drawFrom") {
            node.draw_from = draw_from(from.as_str().unwrap_or("center"));
        }
        if let Some(size) = props.get("drawSize") {
            node.draw_size = offsets(size);
            let place = [node.springs[W].target, node.springs[H].target].map(|x| x as f32);
            if let Some(size) = node.draw_size {
                let [w, h] = resolve(size, place);
                node.springs[DW].target = f64::from(w);
                node.springs[DH].target = f64::from(h);
            }
            // What follows its place's size takes it from the next layout.
            if follows_place(node.draw_size) {
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
                    "borderRadius" => (
                        &[RADII, RADII + 1, RADII + 2, RADII + 3],
                        sides(value).map(Vec::from).unwrap_or_default(),
                    ),
                    "translate" => {
                        let offset = offsets(value).unwrap_or_default();
                        let size = [node.springs[W].target, node.springs[H].target];
                        let [dx, dy] = resolve(offset, size.map(|x| x as f32));
                        // A fraction of a size it doesn't have yet is taken once it has one.
                        if !node.placed && offset.iter().any(|o| o.of_size != 0.) {
                            node.jumped_translate = Some(offset);
                        }
                        (&[DX, DY], vec![dx, dy])
                    }
                    "drawSize" => {
                        let Some(size) = offsets(value) else { continue };
                        let place = [node.springs[W].target, node.springs[H].target];
                        let [w, h] = resolve(size, place.map(|x| x as f32));
                        // A fraction of a size it doesn't have yet is taken once it has one.
                        if !node.placed && size.iter().any(|o| o.of_size != 0.) {
                            node.jumped_size = Some(size);
                        }
                        (&[DW, DH], vec![w, h])
                    }
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
        // Surfaces are laid out on their own, in their output.
        if !self.is_root(parent) {
            let _ = self
                .tree
                .insert_child_at_index(parent_taffy, index, child_taffy);
        }
        self.styles_dirty = true;
        self.node_mut(child).unwrap().parent = Some(parent);
        self.layout_dirty = true;
        self.changed = true;
    }

    /// Puts `child` in `parent` before the first of `siblings` that `parent` holds, or last if
    /// it holds none of them: the nodes that come after it in the script's tree, some of which,
    /// like those on their way out, bento may not hold.
    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, siblings: &[NodeId]) {
        let Some(node) = self.node(parent) else {
            return;
        };
        let before = siblings
            .iter()
            .filter(|sibling| **sibling != child)
            .find_map(|sibling| node.children.iter().position(|c| c == sibling));
        // Its own place counts from without it.
        let own = node.children.iter().position(|c| *c == child);
        let index = match (before, own) {
            (Some(before), Some(own)) if own < before => Some(before - 1),
            (before, _) => before,
        };
        self.append(parent, child, index);
    }

    fn detach(&mut self, child: NodeId) {
        let Some(parent) = self.node(child).and_then(|n| n.parent) else {
            return;
        };
        let child_taffy = self.node(child).unwrap().taffy;
        if let Some(parent_node) = self.node_mut(parent) {
            parent_node.children.retain(|c| *c != child);
            let parent_taffy = parent_node.taffy;
            // Surfaces aren't in their output's layout.
            let held = self
                .tree
                .children(parent_taffy)
                .is_ok_and(|children| children.contains(&child_taffy));
            if held {
                let _ = self.tree.remove_child(parent_taffy, child_taffy);
            }
        }
        self.styles_dirty = true;
        self.node_mut(child).unwrap().parent = None;
        self.layout_dirty = true;
    }

    /// Takes a node away, at once: a script that wants it to go some way first takes it away
    /// once it has.
    pub fn remove(&mut self, id: NodeId) {
        self.destroy(id);
    }

    /// Takes a node out of what holds it, keeping it and what it holds, to be put back.
    pub fn take_out(&mut self, id: NodeId) {
        if self.node(id).is_some() {
            // Put back, it comes in at its new place, not from where it was.
            self.leave_out(id);
            self.detach(id);
            self.changed = true;
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

    /// Whether `id` is an output's root, which holds its surfaces.
    fn is_root(&self, id: NodeId) -> bool {
        self.roots.iter().any(|root| root.node == id)
    }

    /// Lays the scene out again if anything changed, setting where each node heads: each
    /// surface on its own, placed on its output by its anchors and margins.
    pub fn lay_out(&mut self) {
        if !self.layout_dirty {
            return;
        }
        self.layout_dirty = false;
        if std::mem::take(&mut self.styles_dirty) {
            for i in 0..self.roots.len() {
                self.resolve_text(self.roots[i].node, &TextStyle::default());
            }
        }
        for i in 0..self.roots.len() {
            let (root, size) = (self.roots[i].node, self.roots[i].size);
            let surfaces = self
                .node(root)
                .map(|n| n.children.clone())
                .unwrap_or_default();
            let mut reserved = [0f32; 4];
            // Those put beside a node go once that node has its place.
            let (beside, held): (Vec<NodeId>, Vec<NodeId>) = surfaces.into_iter().partition(|id| {
                self.node(*id)
                    .and_then(|n| n.placement)
                    .is_some_and(|p| p.attach.is_some())
            });
            for surface in held.into_iter().chain(beside) {
                if let Some((edge, room)) = self.lay_out_surface(surface, size) {
                    reserved[edge] += room;
                }
            }
            let output = self.roots[i].output.clone();
            self.reserved.insert(output, reserved);
        }
    }

    /// Sets the text styles of the text in `id` from what it and what's above it set.
    fn resolve_text(&mut self, id: NodeId, inherited: &TextStyle) {
        let Some(node) = self.nodes.get_mut(id as usize).and_then(Option::as_mut) else {
            return;
        };
        let style = node.text_own.over(inherited);
        if let Kind::Text {
            style: own, laid, ..
        } = &mut node.kind
        {
            if *own != style {
                *own = style.clone();
                *laid = None;
                let _ = self.tree.mark_dirty(node.taffy);
            }
        }
        for child in node.children.clone() {
            self.resolve_text(child, &style);
        }
    }

    /// Lays surface `id` out on an output `size` big, and says what it keeps from the windows:
    /// along which edge, and how much.
    fn lay_out_surface(&mut self, id: NodeId, size: (f32, f32)) -> Option<(usize, f32)> {
        let node = self.node(id)?;
        let placement = node.placement.unwrap_or_default();
        let taffy = node.taffy;
        let shown = !node.hidden && self.displayed(node);
        let own = self.tree.style(taffy).cloned().unwrap_or_default();
        let length = |v: LengthPercentageAuto, of: f32| -> f32 {
            let raw = v.into_raw();
            match raw.tag() {
                CompactLength::LENGTH_TAG => raw.value(),
                CompactLength::PERCENT_TAG => raw.value() * of,
                _ => 0.,
            }
        };
        // Its margins are from the output's edges.
        let margin = [
            length(own.margin.top, size.1),
            length(own.margin.right, size.0),
            length(own.margin.bottom, size.1),
            length(own.margin.left, size.0),
        ];
        let room = (
            (size.0 - margin[1] - margin[3]).max(0.),
            (size.1 - margin[0] - margin[2]).max(0.),
        );
        let attach = placement.attach.and_then(|attach| {
            let rect: [f32; 4] = match attach.to {
                AttachTo::Node(node) => {
                    let node = self.node(node)?;
                    std::array::from_fn(|i| node.springs[X + i].target as f32)
                }
                AttachTo::Point([x, y]) => [x, y, 0., 0.],
            };
            Some((attach, rect))
        });
        // Put beside a node, it's as big as what it holds.
        let [top, right, bottom, left] = if attach.is_some() {
            [false; 4]
        } else {
            placement.anchor
        };
        let (wide, tall) = (left && right, top && bottom);
        // Held to opposite edges, it stretches between them unless it says how big it is; else
        // it's as big as what it holds, up to the room there is.
        let mut style = own.clone();
        let zero = LengthPercentageAuto::length(0.);
        style.margin = Rect {
            left: zero,
            right: zero,
            top: zero,
            bottom: zero,
        };
        style.position = Position::Relative;
        let auto = LengthPercentageAuto::auto();
        style.inset = Rect {
            left: auto,
            right: auto,
            top: auto,
            bottom: auto,
        };
        if wide && style.size.width == Dimension::auto() {
            style.size.width = Dimension::length(room.0);
        }
        if tall && style.size.height == Dimension::auto() {
            style.size.height = Dimension::length(room.1);
        }
        if style.max_size.width == LengthPercentageAuto::auto() {
            style.max_size.width = LengthPercentageAuto::length(room.0);
        }
        if style.max_size.height == LengthPercentageAuto::auto() {
            style.max_size.height = LengthPercentageAuto::length(room.1);
        }
        let _ = self.tree.set_style(taffy, style);
        let available = Size {
            width: if wide {
                AvailableSpace::Definite(room.0)
            } else {
                AvailableSpace::MaxContent
            },
            height: if tall {
                AvailableSpace::Definite(room.1)
            } else {
                AvailableSpace::MaxContent
            },
        };
        let Self {
            tree, nodes, fonts, ..
        } = self;
        let _ = tree.compute_layout_with_measure(taffy, available, |input, _, id, _| {
            let mut out = measure(nodes, fonts, input, id.copied());
            // taffy keeps what's out of a box's flow in a list of its own, and only empties it
            // for a box with a positioning area, which it gives a box with nothing in it none of.
            // A box left empty by what was out of its flow would keep it on the list, and taffy
            // would read it once it's gone. With an area, it's emptied.
            out.oof_positioning_area
                .get_or_insert(taffy::OofPositioningArea {
                    size: out.size,
                    offset: taffy::Point::ZERO,
                });
            out
        });
        let laid = self
            .tree
            .layout(taffy)
            .map(|l| l.size)
            .unwrap_or(Size::ZERO);
        // Its own style again, for scripts to change; the layout stays until the next one.
        let _ = self.tree.set_style(taffy, own);

        let x = if left {
            margin[3]
        } else if right {
            size.0 - margin[1] - laid.width
        } else {
            (size.0 - laid.width) / 2.
        };
        let y = if top {
            margin[0]
        } else if bottom {
            size.1 - margin[2] - laid.height
        } else {
            (size.1 - laid.height) / 2.
        };
        // Beside its node, on the output, as far in as its margins say.
        let (x, y) = match attach {
            Some((attach, [nx, ny, nw, nh])) => {
                let (w, h) = (laid.width, laid.height);
                let at = |side: usize, align: f32| match side {
                    0 => (nx + (nw - w) * align, ny - attach.gap - h),
                    1 => (nx + nw + attach.gap, ny + (nh - h) * align),
                    3 => (nx - attach.gap - w, ny + (nh - h) * align),
                    _ => (nx + (nw - w) * align, ny + nh + attach.gap),
                };
                let fits_x = |x: f32| x >= margin[3] && x + w <= size.0 - margin[1];
                let fits_y = |y: f32| y >= margin[0] && y + h <= size.1 - margin[2];
                // Where there's no room on its side, it goes on the other; where there's no room
                // lined up with its start, it lines up with its end, and the other way around, as
                // a menu at the pointer does near the output's edges.
                let (mut side, mut align) = (attach.side, attach.align);
                let (x, y) = at(side, align);
                let across = side % 2 == 0;
                if !(if across { fits_y(y) } else { fits_x(x) }) {
                    let (x, y) = at((side + 2) % 4, align);
                    if if across { fits_y(y) } else { fits_x(x) } {
                        side = (side + 2) % 4;
                    }
                }
                let (x, y) = at(side, align);
                if !(if across { fits_x(x) } else { fits_y(y) }) && align != 0.5 {
                    let (x, y) = at(side, 1. - align);
                    if if across { fits_x(x) } else { fits_y(y) } {
                        align = 1. - align;
                    }
                }
                let (x, y) = at(side, align);
                (
                    x.min(size.0 - margin[1] - w).max(margin[3]),
                    y.min(size.1 - margin[2] - h).max(margin[0]),
                )
            }
            None => (x, y),
        };
        if let Some(placement) = self.node_mut(id).and_then(|n| n.placement.as_mut()) {
            placement.origin = [x, y];
        }
        self.place(id, [x, y]);

        let edge = placement.reserved_edge().filter(|_| shown)?;
        let room = match placement.reserve {
            Reserve::None => return None,
            Reserve::Pixels(px) => px,
            Reserve::Own => {
                margin[edge]
                    + if edge % 2 == 0 {
                        laid.height
                    } else {
                        laid.width
                    }
            }
        };
        Some((edge, room))
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
        let mut at = node;
        loop {
            if let Some(placement) = &at.placement {
                x += placement.origin[0];
                y += placement.origin[1];
                break;
            }
            let Some(above) = at.parent.and_then(|id| self.node(id)) else {
                break;
            };
            if let Some(placement) = &above.placement {
                // A surface's own layout is at its output's top left; its place is its origin.
                x += placement.origin[0];
                y += placement.origin[1];
                break;
            }
            let layout = self.tree.layout(above.taffy).ok()?;
            x += layout.location.x;
            y += layout.location.y;
            at = above;
        }
        Some([x, y, size[0], size[1]])
    }

    /// Sets the targets of `id` and what's in it from the layout, with its parent's top left at
    /// `origin`. Frames are absolute, so a node whose place on the output doesn't change doesn't
    /// move, whatever its parent does.
    fn place(&mut self, id: NodeId, origin: [f32; 2]) {
        let Some(node) = self.node(id) else { return };
        // What's left out has no place: shown again, it comes in at its place, not from where
        // it was. What's hidden is still laid out, and what in it hears where it would be is
        // told, so that a script can size things for it before it shows.
        if node.hidden || !self.displayed(node) {
            if node.hidden && self.displayed(node) {
                self.tell_layout(id, origin);
            }
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
        // Those that hear where they're put are told when it changes.
        let tell = node.events & LAYOUT != 0 && node.told != Some(target);
        let (scrolls, taffy_id) = (node.look.scroll, node.taffy);
        if tell {
            self.laid_out.push((id, target));
        }
        // What scrolls is moved up by as far as it's scrolled, as far as what it holds reaches.
        let reach = if scrolls {
            let bottom = children
                .iter()
                .filter_map(|child| {
                    let child = self.node(*child)?;
                    let l = self.tree.layout(child.taffy).ok()?;
                    Some(l.location.y + l.size.height)
                })
                .fold(0f32, f32::max);
            let padding = self.tree.layout(taffy_id).map_or(0., |l| l.padding.bottom);
            (bottom + padding - target[3]).max(0.)
        } else {
            0.
        };
        let node = self.node_mut(id).unwrap();
        if node.events & LAYOUT != 0 {
            node.told = Some(target);
        }
        node.reach = reach;
        node.scrolled = node.scrolled.clamp(0., reach);
        let scrolled = node.scrolled;
        // Those that hear how far they're scrolled are told when it, or how far it goes, changes.
        let scroll = [scrolled, reach, target[3]];
        if scrolls && node.events & SCROLLED != 0 && node.told_scroll != Some(scroll) {
            node.told_scroll = Some(scroll);
            self.scrolled_out.push((id, scroll));
        }
        let node = self.node_mut(id).unwrap();
        // Drawn at its place's size, unless it says another.
        let place = [target[2], target[3]];
        let size = node.draw_size.map_or(place, |size| resolve(size, place));
        let first = !node.placed;
        if !node.placed {
            node.placed = true;
            for (spring, value) in node.springs.iter_mut().zip(target) {
                spring.jump(f64::from(value));
            }
            if let Some(jumped) = node.jumped_size.take() {
                // It starts from where it jumped to, and moves to its own size.
                let [w, h] = resolve(jumped, place);
                for (spring, value) in [(DW, w), (DH, h)] {
                    node.springs[spring].value = f64::from(value);
                    node.springs[spring].velocity = 0.;
                }
            } else if !node.placed_size {
                node.springs[DW].jump(f64::from(size[0]));
                node.springs[DH].jump(f64::from(size[1]));
            }
        } else if node.left_out {
            // Its looks are where they were, or where the script put them as it showed it.
            for (spring, value) in node.springs.iter_mut().zip(target) {
                spring.jump(f64::from(value));
            }
            if follows_place(node.draw_size) {
                node.springs[DW].jump(f64::from(size[0]));
                node.springs[DH].jump(f64::from(size[1]));
            }
        } else if !node.animate {
            for (spring, value) in node.springs.iter_mut().zip(target) {
                spring.jump(f64::from(value));
            }
            if follows_place(node.draw_size) {
                node.springs[DW].jump(f64::from(size[0]));
                node.springs[DH].jump(f64::from(size[1]));
            }
        }
        for (spring, value) in node.springs.iter_mut().zip(target) {
            spring.target = f64::from(value);
        }
        node.springs[DW].target = f64::from(size[0]);
        node.springs[DH].target = f64::from(size[1]);
        // A translation by a fraction of its size follows its size.
        let [dx, dy] = resolve(node.translate, [target[2], target[3]]);
        node.springs[DX].target = f64::from(dx);
        node.springs[DY].target = f64::from(dy);
        let by_size = node.translate.iter().any(|o| o.of_size != 0.);
        if let Some(jumped) = node.jumped_translate.take() {
            // It starts from where it jumped to, and moves to its own translation.
            let [x, y] = resolve(jumped, [target[2], target[3]]);
            for (spring, value) in [(DX, x), (DY, y)] {
                node.springs[spring].value = f64::from(value);
                node.springs[spring].velocity = 0.;
            }
        } else if by_size && (first || !node.animate) {
            node.springs[DX].jump(f64::from(dx));
            node.springs[DY].jump(f64::from(dy));
        }
        node.left_out = false;
        for child in children {
            self.place(child, [target[0], target[1] - scrolled]);
        }
    }

    /// Tells what hears where its layout puts it, in `id` and what it holds, with `id`'s parent's
    /// top left at `origin`, without moving anything.
    fn tell_layout(&mut self, id: NodeId, origin: [f32; 2]) {
        let Some(node) = self.node(id) else { return };
        if !self.displayed(node) {
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
        if node.events & LAYOUT != 0 && node.told != Some(target) {
            self.laid_out.push((id, target));
            self.node_mut(id).unwrap().told = Some(target);
        }
        for child in children {
            self.tell_layout(child, [target[0], target[1]]);
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

    /// The nodes that hear where their layout puts them and were put somewhere else since this
    /// was last asked, with where.
    pub fn take_laid_out(&mut self) -> Vec<(NodeId, [f32; 4])> {
        std::mem::take(&mut self.laid_out)
    }

    /// What hears how far it's scrolled, with how far it is, how far it can be and how tall it
    /// is, since this was last asked.
    pub fn take_scrolled(&mut self) -> Vec<(NodeId, [f32; 3])> {
        std::mem::take(&mut self.scrolled_out)
    }

    /// Scrolls what `id` holds to `top` logical pixels from its top, as far as it goes, and says
    /// whether it scrolls at all.
    pub fn scroll_to(&mut self, id: NodeId, top: f32) -> bool {
        let Some(scrolled) = self.node(id).map(|node| node.scrolled) else {
            return false;
        };
        self.scroll(id, top - scrolled)
    }

    /// Scrolls what `id` holds by `by` logical pixels, as far as it goes, and says whether it
    /// scrolls at all.
    pub fn scroll(&mut self, id: NodeId, by: f32) -> bool {
        let Some(node) = self.node_mut(id) else {
            return false;
        };
        if !node.look.scroll {
            return false;
        }
        let scrolled = (node.scrolled + by).clamp(0., node.reach);
        if scrolled != node.scrolled {
            node.scrolled = scrolled;
            self.layout_dirty = true;
            self.changed = true;
        }
        true
    }

    /// The nodes that hear of their motion settling and settled since this was last asked.
    pub fn take_settled(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.settled)
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
        let [fx, fy] = node.draw_from;
        let x = s(X) + s(DX) + (s(W) - w) * fx;
        let y = s(Y) + s(DY) + (s(H) - h) * fy;
        [x, y, w, h]
    }

    /// What to draw of output `name`'s surfaces in `band`, back to front.
    pub fn draw(&self, output: &str, band: Band) -> Vec<Item> {
        self.draw_with_copies(output, band).0
    }

    /// What to draw of output `name`'s surfaces in `band`, back to front, and the window copies
    /// in them, which bento draws over it.
    pub fn draw_with_copies(&self, output: &str, band: Band) -> (Vec<Item>, Vec<CopyItem>) {
        let mut items = Vec::new();
        let mut copies = Vec::new();
        for child in self.surfaces(output, Some(band)) {
            let mut out = (&mut items, &mut copies);
            self.draw_node(child, 1., None, Scaling::NONE, 0., None, &mut out);
        }
        (items, copies)
    }

    /// What to draw of output `name`'s surfaces in every band, back to front.
    #[cfg(test)]
    pub fn draw_all(&self, output: &str) -> Vec<Item> {
        Band::ALL
            .into_iter()
            .flat_map(|band| self.draw(output, band))
            .collect()
    }

    /// The bands output `name` has surfaces in.
    pub fn bands(&self, output: &str) -> Vec<Band> {
        let mut bands: Vec<Band> = self
            .surfaces(output, None)
            .into_iter()
            .filter_map(|id| self.node(id)?.placement.map(|p| p.band()))
            .collect();
        bands.sort();
        bands.dedup();
        bands
    }

    /// Output `name`'s surfaces in `band`, or all of them, from the back to the front.
    fn surfaces(&self, output: &str, band: Option<Band>) -> Vec<NodeId> {
        let Some(root) = self.root(output).and_then(|root| self.node(root)) else {
            return Vec::new();
        };
        let mut surfaces: Vec<(Band, f32, usize, NodeId)> = root
            .children
            .iter()
            .enumerate()
            .filter_map(|(i, id)| {
                let placement = self.node(*id)?.placement?;
                let own = placement.band();
                band.is_none_or(|band| band == own)
                    .then_some((own, placement.z, i, *id))
            })
            .collect();
        surfaces.sort_by(|a, b| {
            (a.0, a.1, a.2)
                .partial_cmp(&(b.0, b.1, b.2))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        surfaces.into_iter().map(|(_, _, _, id)| id).collect()
    }

    /// The topmost surface of output `output` in `band`, or any, that covers `point`.
    pub fn surface_at(&self, output: &str, point: [f32; 2], band: Option<Band>) -> Option<NodeId> {
        self.surfaces(output, band).into_iter().rev().find(|id| {
            let mut found = Vec::new();
            self.pick_node(*id, point, Scaling::NONE, None, &mut found);
            found.iter().any(|id| self.takes_pointer(*id))
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_node(
        &self,
        id: NodeId,
        opacity: f32,
        clip: Option<Clip>,
        scaling: Scaling,
        blur: f32,
        color: Option<Paint>,
        out: &mut (&mut Vec<Item>, &mut Vec<CopyItem>),
    ) {
        let items = &mut *out.0;
        let Some(node) = self.node(id) else { return };
        if node.hidden || !self.displayed(node) {
            return;
        }
        // The text colour, as it's inherited.
        let color = node.look.color.or(color);
        let ink = color.unwrap_or(Paint::Fg(1.)).resolve(&self.theme);
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
        let spring = &node.springs[SCALE];
        let scaling = scaling.then(Scaling {
            by: own,
            offset: [middle[0] * (1. - own), middle[1] * (1. - own)],
            still: spring.velocity == 0. && (spring.value - spring.target).abs() < 1e-6,
        });
        let rect = scaling.rect(rect);
        let radii = radii.map(|r| r * scaling.by);
        let blur = blur + node.springs[BLUR].value.max(0.) as f32;

        match &node.kind {
            Kind::Box | Kind::Copy(_) => {
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
                if let Kind::Copy(Some(window)) = node.kind {
                    out.1.push(CopyItem {
                        window,
                        rect,
                        radius: radii[0],
                        opacity,
                        clip,
                    });
                }
            }
            Kind::Image(Some(source)) => {
                items.push(Item::Image {
                    rect,
                    source: source.clone(),
                    color: ink,
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
                        color: ink,
                        opacity,
                        scale: scaling.by,
                        still: scaling.still,
                        blur,
                        clip,
                    });
                }
            }
        }

        let clip = if node.look.clip {
            let Some(clip) = (Clip { rect, radii }).within(clip) else {
                return;
            };
            Some(clip)
        } else {
            clip
        };
        // What it holds is moved with it.
        let scaling = scaling.then(Scaling::moved(node));
        for child in &node.children {
            self.draw_node(*child, opacity, clip, scaling, blur, color, out);
        }
    }

    /// The nodes at `point` on output `output`, from the bottom up: those whose drawn shape,
    /// rounded, scaled and cut to what clips it, holds the point, and that aren't hidden or
    /// leaving.
    fn pick(&self, output: &str, point: [f32; 2], band: Option<Band>) -> Vec<NodeId> {
        let mut found = Vec::new();
        for child in self.surfaces(output, band) {
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
            still: true,
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
            let Some(clip) = (Clip { rect, radii }).within(clip) else {
                return;
            };
            Some(clip)
        } else {
            clip
        };
        let scaling = scaling.then(Scaling::moved(node));
        for child in &node.children {
            self.pick_node(*child, point, scaling, clip, found);
        }
    }

    /// The nodes at `point` on output `output` that hear the pointer near: all of them, whatever
    /// is over or under them, since they take nothing from it.
    pub fn near(&self, output: &str, point: [f32; 2]) -> Vec<NodeId> {
        let mut found = self.pick(output, point, None);
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

    /// Where the texts that say `wanted`, and the icons called it, show on output `output`, on
    /// surfaces in `band` alone.
    #[cfg(test)]
    pub fn find_in(&self, output: &str, wanted: &str, band: Band) -> Vec<[f32; 4]> {
        let Some(root) = self.root(output).and_then(|root| self.node(root)) else {
            return Vec::new();
        };
        let surfaces = root.children.iter().filter(|id| {
            self.node(**id)
                .and_then(|node| node.placement)
                .is_some_and(|placement| placement.band() == band)
        });
        let found = surfaces.flat_map(|surface| {
            let all = self.find(output, wanted);
            let frame = self.frame(*surface);
            // What's on this surface is within its frame.
            all.into_iter().filter(move |f| {
                frame.is_some_and(|s| {
                    f[0] >= s[0]
                        && f[1] >= s[1]
                        && f[0] + f[2] <= s[0] + s[2] + 0.5
                        && f[1] + f[3] <= s[1] + s[3] + 0.5
                })
            })
        });
        found.collect()
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

    /// The topmost node at `point` on output `output`, in `band` or any, that takes any of
    /// `events`.
    pub fn hit(
        &self,
        output: &str,
        point: [f32; 2],
        events: u32,
        band: Option<Band>,
    ) -> Option<NodeId> {
        self.pick(output, point, band)
            .into_iter()
            .rev()
            .find(|id| self.node(*id).is_some_and(|n| n.events & events != 0))
    }

    /// Whether the shell's surfaces in `band`, or any, are at `point` on output `output`, so
    /// they take the pointer there from what's below: a shape or something that takes the
    /// pointer's events is.
    pub fn covers(&self, output: &str, point: [f32; 2], band: Option<Band>) -> bool {
        self.pick(output, point, band)
            .into_iter()
            .any(|id| self.takes_pointer(id))
    }

    /// Whether node `id` takes the pointer from what's below it: a shape does, and so does what
    /// hears presses, the pointer coming or scrolling.
    fn takes_pointer(&self, id: NodeId) -> bool {
        self.node(id).is_some_and(|n| {
            n.events & (CLICK | HOVER | SCROLL | DRAG) != 0
                || n.look.scroll
                || n.look.material != Material::None
                || n.look.fill.is_some()
                || n.look.border.is_some()
        })
    }

    /// What the pointer is on at `point` on output `output`, in `band` or any, as CSS's `:hover`
    /// has it: the topmost node that takes the pointer and what holds it, those of them that
    /// hear it come and go.
    pub fn hovered(&self, output: &str, point: [f32; 2], band: Option<Band>) -> Vec<NodeId> {
        let picked = self.pick(output, point, band);
        let Some(mut at) = picked.into_iter().rev().find(|id| self.takes_pointer(*id)) else {
            return Vec::new();
        };
        let mut chain = Vec::new();
        while let Some(node) = self.node(at) {
            if node.events & HOVER != 0 {
                chain.push(at);
            }
            match node.parent {
                Some(parent) if !self.is_root(parent) => at = parent,
                _ => break,
            }
        }
        chain
    }

    /// What scrolling at `point` on output `output`, in `band` or any, scrolls: the topmost node
    /// that hears it, or that holds what scrolls in it, which says so with `true`.
    pub fn scroll_target(
        &self,
        output: &str,
        point: [f32; 2],
        band: Option<Band>,
    ) -> Option<(NodeId, bool)> {
        self.pick(output, point, band)
            .into_iter()
            .rev()
            .find_map(|id| {
                let node = self.node(id)?;
                if node.events & SCROLL != 0 {
                    Some((id, false))
                } else {
                    node.look.scroll.then_some((id, true))
                }
            })
    }

    /// The pointer's look at `point` on output `output`, in `band` or any: the topmost node's
    /// that says one, like "pointer" over a button.
    pub fn cursor(&self, output: &str, point: [f32; 2], band: Option<Band>) -> Option<String> {
        self.pick(output, point, band)
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
    /// Whether it's at rest, rather than on its way somewhere.
    still: bool,
}

impl Scaling {
    const NONE: Self = Self {
        by: 1.,
        offset: [0., 0.],
        still: true,
    };

    /// How far `node` is drawn from its place, which moves what it holds with it.
    fn moved(node: &Node) -> Self {
        let (dx, dy) = (&node.springs[DX], &node.springs[DY]);
        let at_rest = |s: &Spring| s.velocity == 0. && (s.value - s.target).abs() < 1e-6;
        Self {
            by: 1.,
            offset: [dx.value as f32, dy.value as f32],
            still: at_rest(dx) && at_rest(dy),
        }
    }

    /// This scaling, applied after `inner`.
    fn then(self, inner: Self) -> Self {
        Self {
            still: self.still && inner.still,
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

/// Where in its place a shape is held, from a style's `drawFrom`: `"center"`, an edge, or a corner
/// like `"bottom right"`, as fractions of its width and height.
fn draw_from(value: &str) -> [f32; 2] {
    let mut at = [0.5, 0.5];
    for word in value.split_whitespace() {
        match word {
            "left" => at[0] = 0.,
            "right" => at[0] = 1.,
            "top" => at[1] = 0.,
            "bottom" => at[1] = 1.,
            _ => (),
        }
    }
    at
}

/// A length one way, as `translate` takes it: pixels, and a fraction of the node's own size, which
/// a percentage like `"100%"` is.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Offset {
    px: f32,
    of_size: f32,
}

/// A translation, `[x, y]`, each a number of pixels or a percentage of the node's size.
fn offsets(value: &Value) -> Option<[Offset; 2]> {
    let one = |value: &Value| match value {
        Value::Number(n) => Some(Offset {
            px: n.as_f64()? as f32,
            of_size: 0.,
        }),
        Value::String(text) => Some(Offset {
            px: 0.,
            of_size: text.strip_suffix('%')?.trim().parse::<f32>().ok()? / 100.,
        }),
        _ => None,
    };
    match value.as_array()?.as_slice() {
        [x, y] => Some([one(x)?, one(y)?]),
        _ => None,
    }
}

/// Whether a drawn size follows its place's: none, which is its place's, or a fraction of it.
fn follows_place(size: Option<[Offset; 2]>) -> bool {
    size.is_none_or(|size| size.iter().any(|o| o.of_size != 0.))
}

/// `offset` in pixels, for a node of `size`.
fn resolve(offset: [Offset; 2], size: [f32; 2]) -> [f32; 2] {
    [0, 1].map(|i| offset[i].px + offset[i].of_size * size[i])
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
    // Text on one line can be cut down to its ellipsis, which is as narrow as it gets.
    if style.one_line
        && input.known_dimensions.width.is_none()
        && input.available_space.width == AvailableSpace::MinContent
    {
        let ellipsis = fonts.lay_out("…", style, None);
        return taffy::LayoutOutput::from_outer_size(Size {
            width: ellipsis.width.ceil(),
            height: input
                .known_dimensions
                .height
                .unwrap_or(ellipsis.height.ceil()),
        });
    }
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
        "start" | "flex-start" => AlignItems::FLEX_START,
        "center" => AlignItems::CENTER,
        "end" | "flex-end" => AlignItems::FLEX_END,
        "stretch" => AlignItems::STRETCH,
        "baseline" => AlignItems::BASELINE,
        _ => return None,
    })
}

fn justify(value: &Value) -> Option<JustifyContent> {
    Some(match value.as_str()? {
        "start" | "flex-start" => JustifyContent::FLEX_START,
        "center" => JustifyContent::CENTER,
        "end" | "flex-end" => JustifyContent::FLEX_END,
        "space-between" => JustifyContent::SPACE_BETWEEN,
        "space-around" => JustifyContent::SPACE_AROUND,
        "space-evenly" => JustifyContent::SPACE_EVENLY,
        "stretch" => JustifyContent::STRETCH,
        _ => return None,
    })
}

/// A length or a percentage, as padding and gaps take.
fn length_percentage(value: &Value) -> LengthPercentage {
    if let Some(v) = value.as_f64() {
        return LengthPercentage::length(v as f32);
    }
    value
        .as_str()
        .and_then(|s| s.strip_suffix('%'))
        .and_then(|s| s.parse::<f32>().ok())
        .map_or(LengthPercentage::length(0.), |p| {
            LengthPercentage::percent(p / 100.)
        })
}

/// Up to four lengths, percentages or `"auto"`, CSS-style, as margins and insets take.
fn sides_auto(value: &Value) -> Option<[LengthPercentageAuto; 4]> {
    let one = |v: &Value| inset(v).unwrap_or(LengthPercentageAuto::length(0.));
    if !value.is_array() {
        let v = one(value);
        return Some([v; 4]);
    }
    let list: Vec<LengthPercentageAuto> = value.as_array()?.iter().map(one).collect();
    Some(match list.as_slice() {
        [v] => [*v; 4],
        [v, h] => [*v, *h, *v, *h],
        [t, h, b] => [*t, *h, *b, *h],
        [t, r, b, l, ..] => [*t, *r, *b, *l],
        [] => return None,
    })
}

/// Applies one layout prop, by its CSS name, and says whether it was one.
fn apply_layout(style: &mut Style, key: &str, value: &Value) -> bool {
    match key {
        "display" => {
            style.display = match value.as_str() {
                Some("none") => Display::None,
                _ => Display::Flex,
            }
        }
        // A box that scrolls is as small as its style lets it be, not as big as what it holds.
        "overflow" => {
            let overflow = match value.as_str() {
                Some("scroll") => Overflow::Scroll,
                Some("hidden") => Overflow::Hidden,
                _ => Overflow::Visible,
            };
            style.overflow = Point {
                x: overflow,
                y: overflow,
            };
            return true;
        }
        "flexDirection" => {
            style.flex_direction = match value.as_str() {
                Some("column") => FlexDirection::Column,
                Some("column-reverse") => FlexDirection::ColumnReverse,
                Some("row-reverse") => FlexDirection::RowReverse,
                _ => FlexDirection::Row,
            }
        }
        "flexWrap" => {
            style.flex_wrap = match value.as_str() {
                Some("wrap") => FlexWrap::Wrap,
                _ => FlexWrap::NoWrap,
            }
        }
        "gap" => {
            let gap = length_percentage(value);
            style.gap = Size {
                width: gap,
                height: gap,
            };
        }
        "rowGap" => style.gap.height = length_percentage(value),
        "columnGap" => style.gap.width = length_percentage(value),
        "padding" => {
            let [t, r, b, l] = sides(value).unwrap_or([0.; 4]);
            style.padding = Rect {
                left: LengthPercentage::length(l),
                right: LengthPercentage::length(r),
                top: LengthPercentage::length(t),
                bottom: LengthPercentage::length(b),
            };
        }
        "paddingTop" => style.padding.top = length_percentage(value),
        "paddingRight" => style.padding.right = length_percentage(value),
        "paddingBottom" => style.padding.bottom = length_percentage(value),
        "paddingLeft" => style.padding.left = length_percentage(value),
        "margin" => {
            let zero = LengthPercentageAuto::length(0.);
            let [t, r, b, l] = sides_auto(value).unwrap_or([zero; 4]);
            style.margin = Rect {
                left: l,
                right: r,
                top: t,
                bottom: b,
            };
        }
        "marginTop" => style.margin.top = inset(value).unwrap_or(LengthPercentageAuto::length(0.)),
        "marginRight" => {
            style.margin.right = inset(value).unwrap_or(LengthPercentageAuto::length(0.))
        }
        "marginBottom" => {
            style.margin.bottom = inset(value).unwrap_or(LengthPercentageAuto::length(0.))
        }
        "marginLeft" => {
            style.margin.left = inset(value).unwrap_or(LengthPercentageAuto::length(0.))
        }
        "width" => style.size.width = length_or_auto(value).unwrap_or(Dimension::auto()),
        "height" => style.size.height = length_or_auto(value).unwrap_or(Dimension::auto()),
        "minWidth" => style.min_size.width = inset(value).unwrap_or(LengthPercentageAuto::auto()),
        "minHeight" => style.min_size.height = inset(value).unwrap_or(LengthPercentageAuto::auto()),
        "maxWidth" => style.max_size.width = inset(value).unwrap_or(LengthPercentageAuto::auto()),
        "maxHeight" => style.max_size.height = inset(value).unwrap_or(LengthPercentageAuto::auto()),
        "aspectRatio" => style.aspect_ratio = value.as_f64().map(|v| v as f32),
        "flexGrow" => style.flex_grow = value.as_f64().unwrap_or(0.) as f32,
        "flexShrink" => style.flex_shrink = value.as_f64().unwrap_or(1.) as f32,
        "flexBasis" => style.flex_basis = length_or_auto(value).unwrap_or(Dimension::auto()),
        "alignItems" => style.align_items = align_items(value).unwrap_or(AlignItems::NORMAL),
        "alignSelf" => style.align_self = align_items(value),
        "alignContent" => style.align_content = justify(value).unwrap_or(JustifyContent::NORMAL),
        "justifyContent" => {
            style.justify_content = justify(value).unwrap_or(JustifyContent::NORMAL)
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
        "inset" => {
            let auto = LengthPercentageAuto::auto();
            let [t, r, b, l] = sides_auto(value).unwrap_or([auto; 4]);
            style.inset = Rect {
                left: l,
                right: r,
                top: t,
                bottom: b,
            };
        }
        _ => return false,
    }
    true
}

/// Applies one text style, which what a node holds inherits, and says whether it was one.
fn apply_text(own: &mut TextOwn, key: &str, value: &Value) -> bool {
    match key {
        "fontSize" => own.size = value.as_f64().map(|v| v as f32),
        "fontWeight" => {
            own.weight = value.as_f64().map(|v| v as f32).or(match value.as_str() {
                Some("bold") => Some(700.),
                Some("normal") => Some(400.),
                _ => None,
            })
        }
        "fontFamily" => own.family = value.as_str().map(String::from),
        "textAlign" => {
            own.align = value.as_str().map(|v| match v {
                "center" => 0.5,
                "end" | "right" => 1.,
                _ => 0.,
            })
        }
        "fontVariantNumeric" => own.tabular = value.as_str().map(|v| v == "tabular-nums"),
        "lineClamp" => own.one_line = (!value.is_null()).then(|| value.as_u64() == Some(1)),
        _ => return false,
    }
    true
}

/// Applies one of a surface's props, and says whether it was one.
fn apply_placement(placement: &mut Placement, key: &str, value: &Value) -> bool {
    match key {
        "layer" => {
            placement.layer = match value.as_str() {
                Some("background") => 0,
                Some("bottom") => 1,
                Some("overlay") => 3,
                _ => 2,
            }
        }
        "z" => placement.z = value.as_f64().unwrap_or(1.) as f32,
        "anchor" => {
            let edges: Vec<&str> = value
                .as_array()
                .map(|list| list.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            placement.anchor = ["top", "right", "bottom", "left"].map(|e| edges.contains(&e));
        }
        "attach" => {
            let to = match (value.get("node"), value.get("point")) {
                (Some(node), _) => node.as_u64().map(|node| AttachTo::Node(node as NodeId)),
                (_, Some(point)) => serde_json::from_value(point.clone())
                    .ok()
                    .map(AttachTo::Point),
                _ => None,
            };
            placement.attach = to.map(|to| Attach {
                to,
                side: match value["side"].as_str() {
                    Some("top") => 0,
                    Some("right") => 1,
                    Some("left") => 3,
                    _ => 2,
                },
                align: match value["align"].as_str() {
                    Some("center") => 0.5,
                    Some("end") => 1.,
                    _ => 0.,
                },
                gap: value["gap"].as_f64().unwrap_or(0.) as f32,
            })
        }
        "reserve" => {
            placement.reserve = match value {
                Value::Bool(true) => Reserve::Own,
                Value::Number(n) => Reserve::Pixels(n.as_f64().unwrap_or(0.) as f32),
                _ => Reserve::None,
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
        "background" => look.fill = Paint::parse(value),
        "tint" => look.tint = Paint::parse(value),
        "color" => look.color = Paint::parse(value),
        "material" => {
            look.material = match value.as_str() {
                Some("solid") => Material::Solid,
                Some("mica") => Material::Mica,
                Some("acrylic") => Material::Acrylic,
                Some("glass") => Material::Glass,
                _ => Material::None,
            }
        }
        "borderRadius" => {
            // Top left, top right, bottom right and bottom left.
            look.radii = sides(value).unwrap_or([0.; 4]);
        }
        "borderWidth" => {
            let width = value.as_f64().unwrap_or(0.) as f32;
            let paint = look
                .border
                .map(|(_, p)| p)
                .or(look.border_paint)
                .unwrap_or(Paint::Fg(0.2));
            look.border = (width > 0.).then_some((width, paint));
        }
        "borderColor" => {
            if let Some(paint) = Paint::parse(value) {
                let width = look.border.map_or(0., |(w, _)| w);
                look.border = Some((width, paint)).filter(|(w, _)| *w > 0.);
                // The colour is kept for when a width comes.
                if look.border.is_none() {
                    look.border_paint = Some(paint);
                }
            }
        }
        "opacity" => look.opacity = value.as_f64().unwrap_or(1.).clamp(0., 1.) as f32,
        "scale" => look.scale = value.as_f64().unwrap_or(1.).max(0.) as f32,
        "blur" => look.blur = value.as_f64().unwrap_or(0.).max(0.) as f32,
        "overflow" => {
            look.clip = matches!(value.as_str(), Some("hidden" | "scroll"));
            look.scroll = value.as_str() == Some("scroll");
        }
        "visibility" => node.hidden = value.as_str() == Some("hidden"),
        "hidden" => node.hidden = value.as_bool().unwrap_or(false),
        // How long its moves take, in seconds: all of them, or each kind's.
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
            } else {
                let response = value.as_f64().filter(|r| *r > 0.);
                node.responses = [response; SPRINGS];
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
                            Some("layout") => LAYOUT,
                            Some("scrolled") => SCROLLED,
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
            crate::scene::host::UNIX,
            serde_json::json!({ "gaps": 8 }),
            Default::default(),
        )
        .unwrap();
        let mut scene = shell.scene.borrow_mut();
        scene.lay_out();
        scene.advance(1., Some(1.));
        let shapes = |scene: &Scene| -> Vec<[f32; 4]> {
            scene
                .draw_all("DP-1")
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
            .hit("DP-1", [960., 20.], CLICK, None)
            .expect("the clock takes clicks");
        drop(scene);
        shell.send(&serde_json::json!({ "type": "click", "id": clock, "button": 1 }));
        // The calendar is laid out, and the island hears how big it is, as bento tells it.
        let laid = {
            let mut scene = shell.scene.borrow_mut();
            scene.lay_out();
            scene.take_laid_out()
        };
        for (id, [x, y, width, height]) in laid {
            shell.send(&serde_json::json!({
                "type": "layout", "id": id, "x": x, "y": y, "width": width, "height": height,
            }));
        }
        let mut scene = shell.scene.borrow_mut();
        scene.lay_out();
        scene.advance(1. / 60., Some(1.));
        let opening = shapes(&scene)[1];
        assert!(opening[3] > 36. && opening[3] < 478., "{opening:?}");
        for _ in 0..120 {
            scene.advance(1. / 60., Some(1.));
        }
        let open = shapes(&scene)[1];
        // As big as the calendar in it, with its room around. The calendar is as tall as its
        // text, which is a pixel or two taller or shorter with the fonts one machine has and
        // another doesn't.
        assert_eq!(open[2], 340.);
        assert!((open[3] - 478.).abs() <= 4., "{open:?}");
        assert!((open[0] + open[2] / 2. - 960.).abs() < 1., "{open:?}");
    }

    #[test]
    fn typing_into_an_input_edits_it_with_its_caret_and_selection() {
        let mut scene = Scene::new();
        let root = screen(&mut scene);
        let input = scene.create("input", &serde_json::json!({ "fontSize": 20 }));
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

    /// A surface over the whole of output DP-1, to put things in.
    fn screen(scene: &mut Scene) -> NodeId {
        let root = scene.set_output("DP-1", (1920., 1080.));
        let surface = scene.create(
            "surface",
            &serde_json::json!({ "anchor": ["top", "right", "bottom", "left"], "layer": "overlay" }),
        );
        scene.append(root, surface, None);
        surface
    }

    fn panel(scene: &mut Scene, props: serde_json::Value) -> NodeId {
        let root = screen(scene);
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
            serde_json::json!({ "drawSize": [100, 40], "translate": [10, -5] }),
        );
        settle(&mut scene);
        assert_eq!(scene.frame(node).unwrap(), [920., 235., 100., 40.]);
        // Without one, it's drawn at its place's size, and moves there.
        scene.set(
            node,
            &serde_json::json!({ "drawSize": null, "translate": [0, 0] }),
        );
        scene.lay_out();
        scene.advance(1. / 60., Some(1.));
        let moving = scene.frame(node).unwrap();
        assert!(moving[2] > 100. && moving[2] < 560., "{moving:?}");
        settle(&mut scene);
        assert_eq!(scene.frame(node).unwrap(), [680., 200., 560., 120.]);
    }

    #[test]
    fn a_drawn_size_is_held_at_the_edge_or_corner_it_says() {
        let mut scene = Scene::new();
        let node = panel(
            &mut scene,
            serde_json::json!({ "drawSize": [100, 40], "drawFrom": "bottom right" }),
        );
        settle(&mut scene);
        assert_eq!(scene.frame(node).unwrap(), [1140., 280., 100., 40.]);
        scene.set(node, &serde_json::json!({ "drawFrom": "bottom" }));
        assert_eq!(scene.frame(node).unwrap(), [910., 280., 100., 40.]);
        scene.set(node, &serde_json::json!({ "drawFrom": "top left" }));
        assert_eq!(scene.frame(node).unwrap(), [680., 200., 100., 40.]);
    }

    #[test]
    fn a_translation_can_be_a_fraction_of_its_size() {
        let mut scene = Scene::new();
        // The panel is 560 by 120.
        let node = panel(&mut scene, serde_json::json!({ "translate": [0, "50%"] }));
        settle(&mut scene);
        assert_eq!(scene.frame(node).unwrap(), [680., 260., 560., 120.]);
        // A jump by a fraction of its size before it's laid out starts it from there.
        let other = scene.create(
            "box",
            &serde_json::json!({ "width": 50, "height": 80, "jump": { "translate": [0, "100%"] } }),
        );
        scene.append(node, other, None);
        scene.lay_out();
        let start = scene.frame(other).unwrap();
        settle(&mut scene);
        let end = scene.frame(other).unwrap();
        assert_eq!(start[1] - end[1], 80., "{start:?} {end:?}");
    }

    #[test]
    fn a_box_left_empty_by_what_was_out_of_its_flow_lays_out() {
        let mut scene = Scene::new();
        let holder = panel(&mut scene, serde_json::json!({}));
        let absolute = scene.create(
            "box",
            &serde_json::json!({ "position": "absolute", "width": 20, "height": 20 }),
        );
        scene.append(holder, absolute, None);
        scene.lay_out();
        // taffy keeps what's out of a box's flow in a list of its own, which it doesn't empty
        // for a box laid out with nothing in it, and then reads what's gone from it.
        scene.remove(absolute);
        scene.lay_out();
        assert_eq!(scene.frame(holder).unwrap()[2], 560.);
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
        assert!(scene.covers("DP-1", [960., 260.], None));
        scene.set(node, &serde_json::json!({ "hidden": true }));
        assert!(scene.draw_all("DP-1").is_empty());
        assert!(!scene.covers("DP-1", [960., 260.], None));
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
        let root = screen(&mut scene);
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
        assert!(
            scene.draw_all("DP-1").is_empty(),
            "{:?}",
            scene.draw_all("DP-1")
        );
        assert!(!scene.covers("DP-1", [0., 0.], None));

        // Displayed again, it's there.
        scene.set(holder, &serde_json::json!({ "display": "flex" }));
        scene.lay_out();
        scene.advance(1., Some(1.));
        assert_eq!(scene.draw_all("DP-1").len(), 1);
        assert!(scene.covers("DP-1", [20., 20.], None));
    }

    #[test]
    fn what_shows_again_comes_in_at_its_place_rather_than_from_where_it_was_left_out() {
        let mut scene = Scene::new();
        let root = screen(&mut scene);
        let row = scene.create(
            "box",
            &serde_json::json!({ "position": "absolute", "top": 100, "left": 600, "gap": 10 }),
        );
        let first = scene.create("box", &serde_json::json!({ "width": 50, "height": 20 }));
        let shown_later = scene.create(
            "box",
            &serde_json::json!({ "width": 30, "height": 20, "display": "none", "background": "fg" }),
        );
        let hidden_later = scene.create(
            "box",
            &serde_json::json!({ "width": 30, "height": 20, "background": "fg" }),
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
    fn one_line_text_is_cut_short_to_its_room_and_lets_what_is_beside_it_be() {
        let mut scene = Scene::new();
        let root = screen(&mut scene);
        let row = scene.create(
            "box",
            &serde_json::json!({ "position": "absolute", "top": 0, "left": 0, "width": 200, "gap": 10 }),
        );
        let icon = scene.create(
            "box",
            &serde_json::json!({ "width": 32, "height": 32, "flexShrink": 0 }),
        );
        let name = scene.create(
            "text",
            &serde_json::json!({
                "text": "Navi 21/23 HDMI/DP Audio Controller Digital Stereo (HDMI 3)",
                "lineClamp": 1,
            }),
        );
        let pin = scene.create(
            "box",
            &serde_json::json!({ "width": 24, "height": 24, "flexShrink": 0 }),
        );
        scene.append(root, row, None);
        scene.append(row, icon, None);
        scene.append(row, name, None);
        scene.append(row, pin, None);
        scene.lay_out();
        // The icon and the pin keep their size, and the name takes what's left, on one line.
        assert_eq!(scene.layout(icon).unwrap()[2], 32.);
        assert_eq!(scene.layout(pin).unwrap()[2], 24.);
        let place = scene.layout(name).unwrap();
        assert!(place[2] <= 200. - 32. - 24. - 20. + 0.5, "{place:?}");
        let Kind::Text {
            laid: Some((_, laid)),
            ..
        } = &scene.node(name).unwrap().kind
        else {
            panic!("not laid out");
        };
        assert!(laid.width <= place[2] + 0.5, "{} in {place:?}", laid.width);
        // One line of 14 pixel text.
        assert!(laid.height < 28., "{}", laid.height);
    }

    #[test]
    fn what_is_absolute_is_placed_and_sized_in_what_holds_it() {
        let mut scene = Scene::new();
        let root = screen(&mut scene);
        // A bar across the output, a column at its end, and a pill in that, none of which say
        // how they're positioned but the bar.
        let bar = scene.create(
            "box",
            &serde_json::json!({ "position": "absolute", "top": 8, "left": 8, "right": 8 }),
        );
        let end = scene.create(
            "box",
            &serde_json::json!({ "flexGrow": 1, "width": 0, "flexDirection": "column", "alignItems": "end" }),
        );
        let pill = scene.create("box", &serde_json::json!({ "width": 372, "height": 36 }));
        let holder = scene.create(
            "box",
            &serde_json::json!({ "position": "absolute", "top": 16, "left": 0, "right": 0 }),
        );
        scene.append(root, bar, None);
        scene.append(bar, end, None);
        scene.append(end, pill, None);
        scene.append(pill, holder, None);
        assert_eq!(scene.layout(pill), Some([1540., 8., 372., 36.]));
        // From the pill's left to its right, not the bar's.
        assert_eq!(scene.layout(holder), Some([1540., 24., 372., 0.]));
    }

    #[test]
    fn what_overflows_its_holder_is_held_to_the_side_it_says() {
        let mut scene = Scene::new();
        let root = screen(&mut scene);
        let pill = scene.create(
            "box",
            &serde_json::json!({ "position": "absolute", "top": 8, "left": 800, "width": 100, "height": 36 }),
        );
        scene.append(root, pill, None);
        let mut held = |justify: &str| {
            let holder = scene.create(
                "box",
                &serde_json::json!({
                    "position": "absolute", "top": 16, "left": 0, "right": 0, "justifyContent": justify,
                }),
            );
            let content = scene.create(
                "box",
                &serde_json::json!({ "width": 340, "height": 50, "flexShrink": 0 }),
            );
            scene.append(pill, holder, None);
            scene.append(holder, content, None);
            scene.layout(content).unwrap()
        };
        // Wider than the pill, it reaches past it on the sides it isn't held to.
        assert_eq!(held("start"), [800., 24., 340., 50.]);
        assert_eq!(held("end"), [560., 24., 340., 50.]);
        assert_eq!(held("center"), [680., 24., 340., 50.]);
    }

    #[test]
    fn what_hears_the_pointer_near_takes_nothing_from_it() {
        let mut scene = Scene::new();
        let root = screen(&mut scene);
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
        assert!(!scene.covers("DP-1", [500., 1.], None));
        assert!(scene.covers("DP-1", [50., 1.], None));
    }

    /// A surface on output DP-1 with `props`, holding a box with `inner`.
    fn surface(scene: &mut Scene, props: Value, inner: Value) -> (NodeId, NodeId) {
        let root = scene.set_output("DP-1", (1920., 1080.));
        let surface = scene.create("surface", &props);
        let held = scene.create("box", &inner);
        scene.append(root, surface, None);
        scene.append(surface, held, None);
        (surface, held)
    }

    #[test]
    fn a_child_put_before_another_comes_before_it() {
        let mut scene = Scene::new();
        let (_, row) = surface(
            &mut scene,
            serde_json::json!({ "anchor": ["top", "left"] }),
            serde_json::json!({ "flexDirection": "row" }),
        );
        let style = serde_json::json!({ "width": 40, "height": 40 });
        let a = scene.create("box", &style);
        let b = scene.create("box", &style);
        scene.append(row, a, None);
        scene.append(row, b, None);
        scene.lay_out();
        assert_eq!(scene.layout(a).map(|r| r[0]), Some(0.));
        scene.append(row, b, Some(0));
        scene.lay_out();
        assert_eq!(scene.layout(b).map(|r| r[0]), Some(0.));
        assert_eq!(scene.layout(a).map(|r| r[0]), Some(40.));

        // Before the first of the nodes after it that's there: one that isn't is passed over.
        scene.insert_before(row, a, &[9999, b]);
        scene.lay_out();
        assert_eq!(scene.layout(a).map(|r| r[0]), Some(0.));
        scene.insert_before(row, a, &[9999]);
        scene.lay_out();
        assert_eq!(scene.layout(a).map(|r| r[0]), Some(40.));
    }

    #[test]
    fn a_surface_goes_where_its_anchors_and_margins_say_and_keeps_room_from_the_windows() {
        let mut scene = Scene::new();
        // A taskbar: along the bottom, stretched between the sides, as tall as it holds.
        let (bar, _) = surface(
            &mut scene,
            serde_json::json!({
                "anchor": ["bottom", "left", "right"], "reserve": true, "margin": [0, 4, 6, 4],
            }),
            serde_json::json!({ "height": 48 }),
        );
        // A panel in the middle, as big as it holds.
        let (panel, _) = surface(
            &mut scene,
            serde_json::json!({}),
            serde_json::json!({ "width": 400, "height": 200 }),
        );
        scene.lay_out();
        assert_eq!(scene.layout(bar), Some([4., 1080. - 6. - 48., 1912., 48.]));
        assert_eq!(scene.layout(panel), Some([760., 440., 400., 200.]));
        assert_eq!(scene.reserved("DP-1"), [0., 0., 48. + 6., 0.]);
        // Held to one corner, it keeps no room.
        scene.set(bar, &serde_json::json!({ "anchor": ["bottom", "left"] }));
        scene.lay_out();
        assert_eq!(scene.reserved("DP-1"), [0.; 4]);
    }

    #[test]
    fn a_surface_attached_to_a_node_goes_beside_it_on_the_output() {
        let mut scene = Scene::new();
        let (_, button) = surface(
            &mut scene,
            serde_json::json!({ "anchor": ["bottom", "right"], "margin": [0, 8, 8, 0] }),
            serde_json::json!({ "width": 40, "height": 40 }),
        );
        let (flyout, _) = surface(
            &mut scene,
            serde_json::json!({
                "attach": { "node": button, "side": "top", "align": "center", "gap": 10 },
                "margin": 8,
            }),
            serde_json::json!({ "width": 300, "height": 100 }),
        );
        scene.lay_out();
        // Over the button, its middle on the button's, but kept on the output by its margin.
        assert_eq!(scene.layout(button), Some([1872., 1032., 40., 40.]));
        assert_eq!(
            scene.layout(flyout),
            Some([1920. - 8. - 300., 1032. - 10. - 100., 300., 100.])
        );
    }

    #[test]
    fn a_surface_attached_to_a_point_goes_where_there_is_room_for_it() {
        let mut scene = Scene::new();
        let (menu, _) = surface(
            &mut scene,
            serde_json::json!({
                "attach": { "point": [100, 200], "side": "bottom", "align": "start" },
                "margin": 8,
            }),
            serde_json::json!({ "width": 200, "height": 300 }),
        );
        scene.lay_out();
        // Below the point and to its right.
        assert_eq!(scene.layout(menu), Some([100., 200., 200., 300.]));

        // Near the bottom right corner, there's room only above it and to its left.
        scene.set(
            menu,
            &serde_json::json!({ "attach": { "point": [1900, 1000], "side": "bottom", "align": "start" } }),
        );
        scene.lay_out();
        assert_eq!(scene.layout(menu), Some([1700., 700., 200., 300.]));
    }

    #[test]
    fn surfaces_are_drawn_in_the_band_of_their_layer_and_z() {
        let mut scene = Scene::new();
        let style = serde_json::json!({ "width": 10, "height": 10, "background": "fg" });
        let (wallpaper, _) = surface(
            &mut scene,
            serde_json::json!({ "layer": "background", "z": -1 }),
            style.clone(),
        );
        let (bar, _) = surface(
            &mut scene,
            serde_json::json!({ "layer": "overlay", "z": -1 }),
            style,
        );
        scene.lay_out();
        scene.advance(1., Some(1.));
        assert_eq!(
            scene.bands("DP-1"),
            [Band::BackgroundBelow, Band::OverlayBelow]
        );
        assert_eq!(scene.draw("DP-1", Band::BackgroundBelow).len(), 1);
        assert_eq!(scene.draw("DP-1", Band::TopAbove).len(), 0);
        assert_eq!(scene.surface_at("DP-1", [960., 540.], None), Some(bar));
        assert_eq!(
            scene.surface_at("DP-1", [960., 540.], Some(Band::BackgroundBelow)),
            Some(wallpaper)
        );
    }

    #[test]
    fn what_is_cut_to_a_box_in_another_is_cut_to_both() {
        let mut scene = Scene::new();
        let (_, outer) = surface(
            &mut scene,
            serde_json::json!({}),
            serde_json::json!({ "height": 100, "width": 100, "overflow": "hidden", "borderRadius": 20 }),
        );
        // Reaching 50 past the outer one, and what it holds 100 past that.
        let inner = scene.create(
            "box",
            &serde_json::json!({
                "position": "absolute", "left": 10, "top": 50, "width": 80, "height": 100,
                "overflow": "hidden", "borderRadius": 6,
            }),
        );
        scene.append(outer, inner, None);
        let held = scene.create(
            "box",
            &serde_json::json!({ "width": 80, "height": 200, "flexShrink": 0, "background": "fg" }),
        );
        scene.append(inner, held, None);
        let shown = scene.create(
            "copy",
            &serde_json::json!({ "position": "absolute", "left": 0, "top": 60, "width": 80, "height": 30, "window": 4 }),
        );
        scene.append(inner, shown, None);
        scene.lay_out();
        settle(&mut scene);
        let [x, y, ..] = scene.frame(outer).unwrap();
        let (items, copies) = Band::ALL
            .into_iter()
            .map(|band| scene.draw_with_copies("DP-1", band))
            .fold((Vec::new(), Vec::new()), |mut all, (items, copies)| {
                all.0.extend(items);
                all.1.extend(copies);
                all
            });
        let clips: Vec<Clip> = items
            .iter()
            .filter_map(|item| match item {
                Item::Shape { clip, .. } => *clip,
                _ => None,
            })
            .collect();
        assert_eq!(clips.len(), 1, "{items:?}");
        assert_eq!(clips[0].rect, [x + 10., y + 50., 80., 50.]);
        // Its own corners at the top, and none where the outer one cuts it across.
        assert_eq!(clips[0].radii, [6., 6., 0., 0.]);
        // What's wholly past the outer one shows nothing.
        assert!(
            copies
                .iter()
                .all(|copy| copy.rect[1] + copy.rect[3] <= y + 100.
                    || copy.clip.unwrap().rect[1] + copy.clip.unwrap().rect[3] <= y + 100.),
            "{copies:?}"
        );
        // Nor does it take the pointer there.
        assert!(scene.covers("DP-1", [x + 40., y + 80.], None));
        assert!(!scene.covers("DP-1", [x + 40., y + 120.], None));
    }

    #[test]
    fn what_scrolls_moves_what_it_holds_as_far_as_it_reaches_and_no_further() {
        let mut scene = Scene::new();
        let (_, list) = surface(
            &mut scene,
            serde_json::json!({}),
            serde_json::json!({ "height": 100, "width": 50, "overflow": "scroll", "flexDirection": "column" }),
        );
        let rows: Vec<NodeId> = (0..5)
            .map(|_| {
                let row =
                    scene.create("box", &serde_json::json!({ "height": 50, "flexShrink": 0 }));
                scene.append(list, row, None);
                row
            })
            .collect();
        scene.lay_out();
        let top = scene.layout(list).unwrap()[1];
        assert!(scene.scroll(list, 80.));
        scene.lay_out();
        settle(&mut scene);
        assert_eq!(scene.frame(rows[0]).unwrap()[1], top - 80.);
        // Five rows of fifty reach 150 past a hundred.
        scene.scroll(list, 500.);
        scene.lay_out();
        settle(&mut scene);
        assert_eq!(scene.frame(rows[4]).unwrap()[1], top + 50.);
    }

    #[test]
    fn what_scrolls_tells_how_far_it_is_and_goes_and_is_scrolled_to_a_place() {
        let mut scene = Scene::new();
        let (_, list) = surface(
            &mut scene,
            serde_json::json!({}),
            serde_json::json!({
                "height": 100, "width": 50, "overflow": "scroll", "flexDirection": "column",
                "events": ["scrolled"],
            }),
        );
        for _ in 0..4 {
            let row = scene.create("box", &serde_json::json!({ "height": 50, "flexShrink": 0 }));
            scene.append(list, row, None);
        }
        scene.lay_out();
        // Scrolled to its top, reaching a hundred past its bottom, and a hundred tall.
        assert_eq!(scene.take_scrolled(), [(list, [0., 100., 100.])]);
        scene.lay_out();
        assert!(
            scene.take_scrolled().is_empty(),
            "told again with nothing new"
        );

        assert!(scene.scroll_to(list, 60.));
        scene.lay_out();
        assert_eq!(scene.take_scrolled(), [(list, [60., 100., 100.])]);
        scene.scroll_to(list, 400.);
        scene.lay_out();
        assert_eq!(scene.take_scrolled(), [(list, [100., 100., 100.])]);
    }

    #[test]
    fn hovering_is_on_what_the_pointer_is_on_and_what_holds_it() {
        let mut scene = Scene::new();
        let (_, card) = surface(
            &mut scene,
            serde_json::json!({}),
            serde_json::json!({ "width": 200, "height": 100, "background": "fg", "events": ["hover"] }),
        );
        let button = scene.create(
            "box",
            &serde_json::json!({ "width": 40, "height": 40, "events": ["hover", "click"] }),
        );
        scene.append(card, button, None);
        scene.lay_out();
        scene.advance(1., Some(1.));
        let at = scene.layout(button).unwrap();
        let on_button = scene.hovered("DP-1", [at[0] + 5., at[1] + 5.], None);
        assert_eq!(on_button, [button, card]);
        let beside = scene.hovered("DP-1", [at[0] + 100., at[1] + 5.], None);
        assert_eq!(beside, [card]);
    }

    #[test]
    fn what_hears_where_its_put_is_told_when_that_changes_hidden_or_not() {
        let mut scene = Scene::new();
        let (_, held) = surface(
            &mut scene,
            serde_json::json!({}),
            serde_json::json!({ "width": 100, "height": 20, "events": ["layout"], "hidden": true }),
        );
        scene.lay_out();
        assert_eq!(scene.take_laid_out(), [(held, [910., 530., 100., 20.])]);
        scene.set(held, &serde_json::json!({ "width": 120 }));
        scene.lay_out();
        assert_eq!(scene.take_laid_out(), [(held, [900., 530., 120., 20.])]);
        scene.set(held, &serde_json::json!({ "background": "fg" }));
        scene.lay_out();
        assert!(scene.take_laid_out().is_empty());
    }

    #[test]
    fn the_shell_keeps_the_top_for_its_islands_and_bentos_gaps() {
        let style = serde_json::json!({ "gaps": 16 });
        let shell = Shell::load(
            &[(String::from("DP-1"), (1920., 1080.))],
            None,
            crate::scene::host::UNIX,
            style,
            Default::default(),
        )
        .unwrap();
        shell.scene.borrow_mut().lay_out();
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
            crate::scene::host::UNIX,
            serde_json::json!({ "gaps": 8 }),
            Default::default(),
        )
        .unwrap();
        let glass = |scene: &Scene| -> Vec<[f32; 4]> {
            scene
                .draw_all("DP-1")
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
        let clock = scene.hit("DP-1", [960., 20.], CLICK, None).unwrap();
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
