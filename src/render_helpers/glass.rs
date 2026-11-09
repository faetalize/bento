//! Glass: the material that blurs what's behind, bends it like a lens near the edges and lights
//! the edges. Background effects use it, and the shell gets it behind its layer surfaces in the
//! shapes it gives through `bento_glass_v1`.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use glam::{Mat3, Vec2};
use niri_config::{Color, CornerRadius, GlassPattern};
use smithay::backend::renderer::gles::{Uniform, UniformValue};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::compositor::{with_states, SurfaceData};
use wayland_server::protocol::wl_surface::WlSurface;

use crate::protocols::bento_shell::{
    glass_shapes, GlassCurve, GlassShape, DEFAULT_MELT, MAX_GLASS_CURVES, MAX_GLASS_PATHS,
    MAX_GLASS_SHAPES,
};
use crate::render_helpers::background_effect::{BackgroundEffectElement, RenderParams};
use crate::render_helpers::blur::BlurOptions;
use crate::render_helpers::framebuffer_effect::FramebufferEffect;
use crate::render_helpers::shaders::{mat3_uniform, GLASS_LIGHT_UNIFORMS};
use crate::utils::region::TransformedRegion;

/// How far from the pointer its light reaches, in logical pixels, as `POINTER_REACH` in
/// `glass.frag` says: glass further away doesn't need drawing again when the pointer moves.
const POINTER_REACH: f64 = 400.;

/// The desktop's accent colour, from the settings portal, which glass can take on. There's one
/// for all of bento, so glass drawn outside an output's frame, like a closing window's last look,
/// has it too.
static ACCENT: Mutex<Option<Color>> = Mutex::new(None);

/// Sets the accent colour, returning whether it changed.
pub fn set_accent(accent: Option<Color>) -> bool {
    let mut current = ACCENT.lock().unwrap();
    let changed = *current != accent;
    *current = accent;
    changed
}

fn accent() -> Option<Color> {
    *ACCENT.lock().unwrap()
}

/// Whether the desktop is in dark style, from the settings portal; light, libadwaita's default,
/// until it says. One for all of bento, like the accent.
static DARK: AtomicBool = AtomicBool::new(false);

/// Sets whether the desktop is in dark style, returning whether it changed.
pub fn set_dark(dark: bool) -> bool {
    DARK.swap(dark, Ordering::Relaxed) != dark
}

/// The shade of an effect blurred with `blur`, as the shaders take it: the window colour for the
/// desktop's style, and how far towards it in alpha.
pub fn shade(blur: &niri_config::Blur) -> [f32; 4] {
    let [r, g, b, _] = blur
        .shade_color(DARK.load(Ordering::Relaxed))
        .to_array_unpremul();
    [r, g, b, blur.shade as f32]
}

/// The commit of an element drawing an effect at `commit`, with its glass, if it's glass, and
/// its shade: it changes too when they do, which the effect doesn't count, like when the glass's
/// shapes change, the pointer moves within its reach, or the accent or the style changes.
pub fn effect_commit(
    commit: CommitCounter,
    glass: Option<&GlassShapes>,
    shade: [f32; 4],
) -> CommitCounter {
    if glass.is_none() && shade[3] == 0. {
        return commit;
    }
    let mut hasher = DefaultHasher::new();
    commit
        .distance(Some(CommitCounter::default()))
        .hash(&mut hasher);
    shade.map(f32::to_bits).hash(&mut hasher);
    if let Some(glass) = glass {
        let count = glass.count;
        for values in [
            glass.rects[..count].as_flattened(),
            glass.radii[..count].as_flattened(),
            &glass.melts[..count],
            glass.curves[..glass.curve_count].as_flattened(),
            &glass.path_ends[..glass.path_count],
        ] {
            for value in values {
                value.to_bits().hash(&mut hasher);
            }
        }
        glass.pointer.map(|p| p.map(f32::to_bits)).hash(&mut hasher);
        let tint = glass.glass.tint(glass.accent);
        tint.to_array_unpremul().map(f32::to_bits).hash(&mut hasher);
    }
    CommitCounter::from(hasher.finish() as usize)
}

/// What glass takes from the output it's drawn on.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GlassFrame {
    /// Where the pointer is while it shows, in the output's logical coordinates.
    pub pointer: Option<Point<f64, Logical>>,
}

/// Shapes and paths of glass, in the coordinates of the geometry the shader clips to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassShapes {
    pub rects: [[f32; 4]; MAX_GLASS_SHAPES],
    /// Top left, top right, bottom right and bottom left.
    pub radii: [[f32; 4]; MAX_GLASS_SHAPES],
    /// How close other glass comes for each to melt into it.
    pub melts: [f32; MAX_GLASS_SHAPES],
    pub count: usize,
    /// The paths' curves, in order of their paths: each its start, control point and end.
    pub curves: [[f32; 6]; MAX_GLASS_CURVES],
    pub curve_count: usize,
    /// How many curves come before the end of each path.
    pub path_ends: [f32; MAX_GLASS_PATHS],
    pub path_count: usize,
    /// How the glass bends and lights what's behind it.
    pub glass: niri_config::Glass,
    /// The accent colour, for glass that follows it.
    pub accent: Option<Color>,
    /// Where the pointer is, in the shapes' coordinates, while its light reaches the glass.
    pub pointer: Option<[f32; 2]>,
}

impl Default for GlassShapes {
    fn default() -> Self {
        Self {
            rects: Default::default(),
            radii: Default::default(),
            melts: [DEFAULT_MELT; MAX_GLASS_SHAPES],
            count: 0,
            curves: [[0.; 6]; MAX_GLASS_CURVES],
            curve_count: 0,
            path_ends: [0.; MAX_GLASS_PATHS],
            path_count: 0,
            glass: Default::default(),
            accent: None,
            pointer: None,
        }
    }
}

impl GlassShapes {
    /// One piece of glass filling a rounded geometry of this size.
    pub fn filling(
        width: f32,
        height: f32,
        radius: CornerRadius,
        glass: niri_config::Glass,
    ) -> Self {
        let mut shapes = Self {
            count: 1,
            glass,
            accent: accent(),
            ..Self::default()
        };
        shapes.rects[0] = [0., 0., width, height];
        shapes.radii[0] = radius.into();
        shapes
    }

    /// Glass in the shape of `region`, an app's region for its background effect, in coordinates
    /// that start at `origin` in the region's: a piece for each part of the region, see
    /// `region_pieces`.
    pub fn following(
        region: &TransformedRegion,
        origin: Point<f64, Logical>,
        glass: niri_config::Glass,
    ) -> Self {
        let scale = region.scale.x.min(region.scale.y);
        let pieces = region_pieces(&region.rects);
        let mut shapes = Self {
            count: pieces.len(),
            glass,
            accent: accent(),
            ..Self::default()
        };
        for (i, (rect, radii)) in pieces.into_iter().enumerate() {
            let loc = rect.loc.to_f64().upscale(region.scale) + region.offset - origin;
            let size = rect.size.to_f64().upscale(region.scale);
            shapes.rects[i] = [loc.x, loc.y, size.w, size.h].map(|x| x as f32);
            shapes.radii[i] = radii.map(|r| (r * scale) as f32);
        }
        shapes
    }

    /// The shapes `zoom` times as big.
    pub fn scaled(mut self, zoom: f32) -> Self {
        for rect in &mut self.rects[..self.count] {
            *rect = rect.map(|x| x * zoom);
        }
        for radii in &mut self.radii[..self.count] {
            *radii = radii.map(|r| r * zoom);
        }
        for melt in &mut self.melts[..self.count] {
            *melt *= zoom;
        }
        for curve in &mut self.curves[..self.curve_count] {
            *curve = curve.map(|x| x * zoom);
        }
        self
    }

    /// Adds the curves of paths, moved by `offset`.
    fn add_paths(&mut self, curves: &[GlassCurve], offset: [f32; 2]) {
        let curves = &curves[..curves.len().min(MAX_GLASS_CURVES)];
        for (i, curve) in curves.iter().enumerate() {
            let [a, b, c] = curve.points.map(|[x, y]| [x + offset[0], y + offset[1]]);
            self.curves[i] = [a[0], a[1], b[0], b[1], c[0], c[1]];
            self.path_ends[curve.path] = (i + 1) as f32;
            self.path_count = self.path_count.max(curve.path + 1);
        }
        self.curve_count = curves.len();
    }

    /// Gives the glass the pointer of `frame`. The shapes' coordinates start at `origin` in the
    /// output's logical coordinates, `zoom` times as big as they show.
    pub fn lit(mut self, frame: GlassFrame, origin: Point<f64, Logical>, zoom: f64) -> Self {
        self.pointer = frame
            .pointer
            .filter(|_| self.glass.pointer_light > 0.)
            .map(|pointer| (pointer - origin).upscale(zoom))
            .filter(|pointer| self.reach(*pointer, POINTER_REACH * zoom))
            .map(|pointer| [pointer.x as f32, pointer.y as f32]);
        self
    }

    /// Whether a light at `point` reaching `reach` falls on any of the shapes or paths. A path's
    /// curves lie inside the box around their points.
    fn reach(&self, point: Point<f64, Logical>, reach: f64) -> bool {
        let path_box = self.curves[..self.curve_count]
            .iter()
            .flat_map(|c| [[c[0], c[1]], [c[2], c[3]], [c[4], c[5]]])
            .fold(None, |bounds: Option<[f32; 4]>, [x, y]| {
                Some(bounds.map_or([x, y, x, y], |[x0, y0, x1, y1]| {
                    [x0.min(x), y0.min(y), x1.max(x), y1.max(y)]
                }))
            })
            .map(|[x0, y0, x1, y1]| [x0, y0, x1 - x0, y1 - y0]);
        let mut boxes = self.rects[..self.count].iter().chain(&path_box);
        boxes.any(|&[x, y, w, h]| {
            let dx = (f64::from(x) - point.x)
                .max(point.x - f64::from(x + w))
                .max(0.);
            let dy = (f64::from(y) - point.y)
                .max(point.y - f64::from(y + h))
                .max(0.);
            dx.hypot(dy) < reach
        })
    }
}

/// The pieces of glass a region makes, each with the radii of its top left, top right, bottom
/// right and bottom left corners.
///
/// Rectangles that touch make one piece: their bounding box, with each corner as round as the
/// region's. An app's region has a rounded corner drawn in whole pixels, as rows that step in,
/// and a corner of radius r leaves out r²(1 − π/4) of the box, so the area left out of each
/// quarter of the box gives its corner's radius. A region in more pieces than glass takes is one
/// piece around them all.
pub fn region_pieces(
    rects: &[Rectangle<i32, Logical>],
) -> Vec<(Rectangle<i32, Logical>, [f64; 4])> {
    // Touching along an edge, or overlapping: sharing more than a point.
    let touch = |a: &Rectangle<i32, Logical>, b: &Rectangle<i32, Logical>| {
        let (a0, a1) = (a.loc, a.loc + a.size.to_point());
        let (b0, b1) = (b.loc, b.loc + b.size.to_point());
        let over_x = a0.x < b1.x && b0.x < a1.x;
        let over_y = a0.y < b1.y && b0.y < a1.y;
        let meet_x = a0.x <= b1.x && b0.x <= a1.x;
        let meet_y = a0.y <= b1.y && b0.y <= a1.y;
        (over_x && meet_y) || (meet_x && over_y)
    };

    let rects: Vec<_> = rects.iter().filter(|r| !r.is_empty()).copied().collect();
    let mut piece: Vec<usize> = (0..rects.len()).collect();
    fn root(piece: &mut [usize], mut i: usize) -> usize {
        while piece[i] != i {
            piece[i] = piece[piece[i]];
            i = piece[i];
        }
        i
    }
    for i in 0..rects.len() {
        for j in i + 1..rects.len() {
            if touch(&rects[i], &rects[j]) {
                let (a, b) = (root(&mut piece, i), root(&mut piece, j));
                piece[a.max(b)] = a.min(b);
            }
        }
    }

    let mut groups: Vec<(usize, Vec<Rectangle<i32, Logical>>)> = Vec::new();
    for (i, rect) in rects.iter().enumerate() {
        let r = root(&mut piece, i);
        match groups.iter_mut().find(|(g, _)| *g == r) {
            Some((_, group)) => group.push(*rect),
            None => groups.push((r, vec![*rect])),
        }
    }
    let mut groups: Vec<_> = groups.into_iter().map(|(_, group)| group).collect();
    if groups.len() > MAX_GLASS_SHAPES {
        groups = vec![rects];
    }

    groups
        .into_iter()
        .filter_map(|group| {
            let bounds = group.iter().copied().reduce(|a, b| a.merge(b))?;
            Some((bounds, corner_radii(bounds, &group)))
        })
        .collect()
}

/// The radii of the corners of `bounds` that `rects` round off, from the area each quarter of it
/// leaves out.
fn corner_radii(bounds: Rectangle<i32, Logical>, rects: &[Rectangle<i32, Logical>]) -> [f64; 4] {
    let (w, h) = (bounds.size.w / 2, bounds.size.h / 2);
    let (x0, y0) = (bounds.loc.x, bounds.loc.y);
    let (x1, y1) = (x0 + bounds.size.w - w, y0 + bounds.size.h - h);
    let largest = f64::from(w.min(h));
    [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| {
        let quarter = Rectangle::<i32, Logical>::new((x, y).into(), (w, h).into());
        let covered: i64 = rects
            .iter()
            .filter_map(|rect| rect.intersection(quarter))
            .map(|r| i64::from(r.size.w) * i64::from(r.size.h))
            .sum();
        let left_out = (i64::from(w) * i64::from(h) - covered).max(0) as f64;
        (left_out / (1. - std::f64::consts::FRAC_PI_4))
            .sqrt()
            .min(largest)
    })
}

/// The glass shader's uniforms.
#[allow(clippy::too_many_arguments)]
pub fn uniforms(
    shapes: &GlassShapes,
    scale: f32,
    geo_size: Vec2,
    input_to_geo: Mat3,
    noise: f32,
    saturation: f32,
    shade: [f32; 4],
    bg_color: [f32; 4],
) -> Vec<Uniform<'static>> {
    let glass = shapes.glass;
    let (lights, sizes) = light_uniforms(&glass);
    let mut uniforms = vec![
        Uniform::new("niri_scale", scale),
        Uniform::new("geo_size", <[f32; 2]>::from(geo_size)),
        mat3_uniform("input_to_geo", input_to_geo),
        mat3_uniform("geo_to_input", input_to_geo.inverse()),
        Uniform::new("shape_count", shapes.count as f32),
        Uniform::new("noise", noise),
        Uniform::new("saturation", saturation),
        Uniform::new("shade", shade),
        Uniform::new("bg_color", bg_color),
        Uniform::new("bezel", glass.bezel as f32),
        Uniform::new("thickness", glass.thickness as f32),
        Uniform::new("refractive_index", glass.refractive_index as f32),
        Uniform::new("dispersion", glass.dispersion as f32),
        Uniform::new("reflection", glass.reflection as f32),
        Uniform::new("sheen", glass.sheen as f32),
        Uniform::new("light_count", lights.len() as f32),
        Uniform::new("light_sizes", sizes),
        Uniform::new(
            "absorbance",
            glass.absorbance(shapes.accent).map(|x| x as f32),
        ),
        Uniform::new("pattern", pattern_uniform(glass.pattern)),
        Uniform::new("pattern_depth", glass.pattern_depth as f32),
        Uniform::new(
            "pointer",
            shapes
                .pointer
                .map_or([0.; 3], |[x, y]| [x, y, glass.pointer_light as f32]),
        ),
    ];
    uniforms.extend(
        GLASS_LIGHT_UNIFORMS
            .iter()
            .enumerate()
            .map(|(i, name)| Uniform::new(*name, lights.get(i).copied().unwrap_or_default())),
    );
    // Each shape a matrix: its rectangle, its corners' radii, then its melt.
    let shape_matrices = (0..MAX_GLASS_SHAPES)
        .map(|i| {
            let ([x, y, w, h], [r0, r1, r2, r3]) = (shapes.rects[i], shapes.radii[i]);
            [
                x,
                y,
                w,
                h,
                r0,
                r1,
                r2,
                r3,
                shapes.melts[i],
                0.,
                0.,
                0.,
                0.,
                0.,
                0.,
                0.,
            ]
        })
        .collect();
    uniforms.push(Uniform::new(
        "shapes",
        UniformValue::Matrix4x4 {
            matrices: shape_matrices,
            transpose: false,
        },
    ));
    let [e0, e1, e2, e3, e4, e5, e6, e7] = shapes.path_ends;
    uniforms.extend([
        Uniform::new("path_count", shapes.path_count as f32),
        Uniform::new("curve_count", shapes.curve_count as f32),
        Uniform::new("path_ends0", [e0, e1, e2, e3]),
        Uniform::new("path_ends1", [e4, e5, e6, e7]),
        Uniform::new(
            "curves",
            UniformValue::Matrix4x4 {
                matrices: curve_matrices(&shapes.curves),
                transpose: false,
            },
        ),
    ]);
    uniforms
}

/// The curves as the shader takes them: two to a matrix, each in two columns, its start and
/// control point in the first and its end in the second.
pub fn curve_matrices(curves: &[[f32; 6]; MAX_GLASS_CURVES]) -> Vec<[f32; 16]> {
    curves
        .chunks_exact(2)
        .map(|pair| {
            let [a, b] = [pair[0], pair[1]];
            [
                a[0], a[1], a[2], a[3], a[4], a[5], 0., 0., //
                b[0], b[1], b[2], b[3], b[4], b[5], 0., 0.,
            ]
        })
        .collect()
}

/// The pattern, as the shader numbers it.
pub fn pattern_uniform(pattern: GlassPattern) -> f32 {
    match pattern {
        GlassPattern::Smooth => 0.,
        GlassPattern::Reeded => 1.,
        GlassPattern::Hammered => 2.,
        GlassPattern::Frosted => 3.,
    }
}

/// The lights that shine, each towards it over the screen with its intensity, and their sizes.
fn light_uniforms(glass: &niri_config::Glass) -> (Vec<[f32; 4]>, [f32; 4]) {
    let mut sizes = [0.; 4];
    let lights = glass
        .lights
        .iter()
        .flatten()
        .filter(|light| light.on && light.intensity > 0.)
        .enumerate()
        .map(|(i, light)| {
            sizes[i] = light.size as f32;
            let (angle, height) = (light.angle.to_radians(), light.height.to_radians());
            [
                (angle.sin() * height.cos()) as f32,
                (-angle.cos() * height.cos()) as f32,
                height.sin() as f32,
                light.intensity as f32,
            ]
        })
        .collect();
    (lights, sizes)
}

/// How far past its shapes the glass reaches where nearby shapes melt together.
const MELT_REACH: i32 = 6;

/// The steps the glass's area grows and shrinks in, in logical pixels. What's behind it is
/// captured again, into new textures, whenever the area changes, so a morph that changed it on
/// every frame would blur on every frame and make new textures for it; in steps, it does that only
/// now and then, and in between only draws the glass again from what it captured.
const AREA_STEP: i32 = 64;

/// Per-surface glass stored in its data map.
struct SurfaceGlass(Mutex<FramebufferEffect>);

impl SurfaceGlass {
    fn get(states: &SurfaceData) -> &Self {
        states
            .data_map
            .get_or_insert_threadsafe(|| SurfaceGlass(Mutex::new(FramebufferEffect::new())))
    }
}

/// Renders the shell surface's glass, if it has any, for a surface at `location`.
pub fn render(
    ns: Option<usize>,
    surface: &WlSurface,
    location: Point<f64, Logical>,
    scale: f64,
    blur: niri_config::Blur,
    frame: GlassFrame,
    push: &mut dyn FnMut(BackgroundEffectElement),
) {
    let (mut shapes, paths) = glass_shapes(surface);
    shapes.retain(|shape| !shape.is_empty());
    let Some(bounds) = bounds(&shapes, &paths) else {
        return;
    };
    let bounds = in_steps(bounds);

    with_states(surface, |states| {
        let effect = SurfaceGlass::get(states).0.lock().unwrap();

        let geometry = Rectangle::new(location + bounds.loc.to_f64(), bounds.size.to_f64())
            .to_physical_precise_round(scale)
            .to_logical(scale);

        let mut uniforms = GlassShapes {
            count: shapes.len(),
            glass: blur.glass,
            accent: accent(),
            ..GlassShapes::default()
        };
        let origin = [bounds.loc.x as f32, bounds.loc.y as f32];
        for (i, shape) in shapes.iter().enumerate() {
            let [x, y, w, h] = shape.rect;
            uniforms.rects[i] = [x - origin[0], y - origin[1], w, h];
            uniforms.radii[i] = [shape.radius; 4];
            uniforms.melts[i] = shape.melt;
        }
        uniforms.add_paths(&paths, [-bounds.loc.x as f32, -bounds.loc.y as f32]);
        let uniforms = uniforms.lit(frame, geometry.loc, 1.);

        let params = RenderParams {
            geometry,
            subregion: None,
            clip: None,
            scale,
        };
        let elem = effect.render_glass(
            ns,
            params,
            BlurOptions::reaching(blur),
            blur.noise as f32,
            blur.saturation as f32,
            if blur.off { [0.; 4] } else { shade(&blur) },
            uniforms,
        );
        if let Some(look) = elem.look() {
            push(look.into());
        }
        push(elem.into());
    });
}

/// The desktop's style, as bento's own shell takes it: whether it's dark, and its accent.
pub fn style() -> (bool, Option<[f32; 4]>) {
    (
        DARK.load(Ordering::Relaxed),
        accent().map(|c| c.to_array_unpremul()),
    )
}

/// Renders glass for bento's own shell: rounded rectangles, each its rectangle and its corners'
/// radii, in an output's logical coordinates, melting together where they come `melt` close.
#[allow(clippy::too_many_arguments)]
pub fn render_rects(
    effect: &FramebufferEffect,
    ns: Option<usize>,
    rects: &[([f32; 4], [f32; 4])],
    melt: f32,
    scale: f64,
    blur: niri_config::Blur,
    frame: GlassFrame,
    push: &mut dyn FnMut(BackgroundEffectElement),
) {
    let rects = &rects[..rects.len().min(MAX_GLASS_SHAPES)];
    let shapes: Vec<GlassShape> = rects
        .iter()
        .map(|(rect, radii)| GlassShape {
            rect: *rect,
            radius: radii[0],
            melt,
        })
        .filter(|shape| !shape.is_empty())
        .collect();
    let Some(bounds) = bounds(&shapes, &[]) else {
        return;
    };
    let bounds = in_steps(bounds);
    let geometry = Rectangle::new(bounds.loc.to_f64(), bounds.size.to_f64())
        .to_physical_precise_round(scale)
        .to_logical(scale);

    let mut uniforms = GlassShapes {
        count: rects.len(),
        glass: blur.glass,
        accent: accent(),
        ..GlassShapes::default()
    };
    let origin = [bounds.loc.x as f32, bounds.loc.y as f32];
    for (i, (rect, radii)) in rects.iter().enumerate() {
        let [x, y, w, h] = *rect;
        uniforms.rects[i] = [x - origin[0], y - origin[1], w, h];
        uniforms.radii[i] = *radii;
        uniforms.melts[i] = melt;
    }
    let uniforms = uniforms.lit(frame, geometry.loc, 1.);
    let params = RenderParams {
        geometry,
        subregion: None,
        clip: None,
        scale,
    };
    let elem = effect.render_glass(
        ns,
        params,
        BlurOptions::reaching(blur),
        blur.noise as f32,
        blur.saturation as f32,
        if blur.off { [0.; 4] } else { shade(&blur) },
        uniforms,
    );
    if let Some(look) = elem.look() {
        push(look.into());
    }
    push(elem.into());
}

/// `bounds` grown out to whole steps of [`AREA_STEP`].
fn in_steps(bounds: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    let down = |x: i32| x.div_euclid(AREA_STEP) * AREA_STEP;
    let up = |x: i32| down(x + AREA_STEP - 1);
    let (x0, y0) = (down(bounds.loc.x), down(bounds.loc.y));
    let (x1, y1) = (
        up(bounds.loc.x + bounds.size.w),
        up(bounds.loc.y + bounds.size.h),
    );
    Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into())
}

/// The area the glass covers, in whole surface coordinates. A path's curves lie inside the box
/// around their points, and where shapes melt, the glass swells past them by up to a quarter of
/// their melt.
fn bounds(shapes: &[GlassShape], paths: &[GlassCurve]) -> Option<Rectangle<i32, Logical>> {
    let shape_boxes = shapes.iter().map(|shape| {
        let [x, y, w, h] = shape.rect;
        [x, y, x + w, y + h]
    });
    let path_boxes = paths.iter().map(|curve| {
        curve.points.iter().fold(
            [
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ],
            |[x0, y0, x1, y1], &[x, y]| [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        )
    });
    let [x0, y0, x1, y1] =
        shape_boxes
            .chain(path_boxes)
            .reduce(|[a0, b0, a1, b1], [c0, d0, c1, d1]| {
                [a0.min(c0), b0.min(d0), a1.max(c1), b1.max(d1)]
            })?;
    let melt = shapes
        .iter()
        .map(|shape| shape.melt)
        .fold(DEFAULT_MELT, f32::max);
    let reach = MELT_REACH.max((melt / 4.).ceil() as i32);
    let (x0, y0) = (x0.floor() as i32 - reach, y0.floor() as i32 - reach);
    let (x1, y1) = (x1.ceil() as i32 + reach, y1.ceil() as i32 + reach);
    Some(Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into()))
}

#[cfg(test)]
mod tests {
    use smithay::backend::renderer::element::Element as _;

    use super::*;
    use crate::render_helpers::framebuffer_effect::FramebufferEffectElement;

    fn glass_at(
        effect: &FramebufferEffect,
        pointer: Option<[f32; 2]>,
        width: f32,
    ) -> FramebufferEffectElement {
        let mut shapes = GlassShapes {
            count: 1,
            pointer,
            ..GlassShapes::default()
        };
        shapes.rects[0] = [0., 0., width, 36.];
        let params = RenderParams {
            geometry: Rectangle::new((0., 0.).into(), (600., 100.).into()),
            subregion: None,
            clip: None,
            scale: 1.,
        };
        effect.render_glass(None, params, None, 0., 1., [0.; 4], shapes)
    }

    #[test]
    fn the_pointer_and_the_shapes_change_how_glass_looks_without_capturing_again() {
        let effect = FramebufferEffect::new();
        let still = glass_at(&effect, Some([10., 10.]), 200.);
        for moved in [
            glass_at(&effect, Some([20., 10.]), 200.),
            glass_at(&effect, Some([10., 10.]), 300.),
        ] {
            // What's behind is captured again only when the effect itself changes.
            assert_eq!(moved.current_commit(), still.current_commit());
            let commit = |elem: &FramebufferEffectElement| elem.look().unwrap().current_commit();
            assert_ne!(commit(&moved), commit(&still));
        }
    }

    #[test]
    fn growing_glass_changes_its_area_only_now_and_then() {
        let mut areas = Vec::new();
        for width in 0..=560 {
            let bounds = Rectangle::new((1000 - width / 2, 300).into(), (width, 120).into());
            let area = in_steps(bounds);
            assert!(area.contains_rect(bounds));
            if areas.last() != Some(&area) {
                areas.push(area);
            }
        }
        assert!(areas.len() <= 12, "{} areas", areas.len());
    }
}
