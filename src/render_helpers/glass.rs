//! Glass: the material that blurs what's behind, bends it like a lens near the edges and lights
//! the edges. Background effects use it, and the shell gets it behind its layer surfaces in the
//! shapes it gives through `bento_glass_v1`.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Mutex;

use glam::{Mat3, Vec2};
use niri_config::{Color, CornerRadius, GlassPattern};
use smithay::backend::renderer::gles::Uniform;
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::compositor::{with_states, SurfaceData};
use wayland_server::protocol::wl_surface::WlSurface;

use crate::protocols::bento_shell::{glass_shapes, GlassShape, MAX_GLASS_SHAPES};
use crate::render_helpers::background_effect::RenderParams;
use crate::render_helpers::blur::BlurOptions;
use crate::render_helpers::framebuffer_effect::{FramebufferEffect, FramebufferEffectElement};
use crate::render_helpers::shaders::{
    mat3_uniform, GLASS_LIGHT_UNIFORMS, GLASS_RADII_UNIFORMS, GLASS_SHAPE_UNIFORMS,
};
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

/// What glass takes from the output it's drawn on.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GlassFrame {
    /// Where the pointer is while it shows, in the output's logical coordinates.
    pub pointer: Option<Point<f64, Logical>>,
}

/// Shapes of glass, in the coordinates of the geometry the shader clips to.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GlassShapes {
    pub rects: [[f32; 4]; MAX_GLASS_SHAPES],
    /// Top left, top right, bottom right and bottom left.
    pub radii: [[f32; 4]; MAX_GLASS_SHAPES],
    pub count: usize,
    /// How the glass bends and lights what's behind it.
    pub glass: niri_config::Glass,
    /// The accent colour, for glass that follows it.
    pub accent: Option<Color>,
    /// Where the pointer is, in the shapes' coordinates, while its light reaches the glass.
    pub pointer: Option<[f32; 2]>,
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
        self
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

    /// Whether a light at `point` reaching `reach` falls on any of the shapes.
    fn reach(&self, point: Point<f64, Logical>, reach: f64) -> bool {
        self.rects[..self.count].iter().any(|&[x, y, w, h]| {
            let dx = (f64::from(x) - point.x)
                .max(point.x - f64::from(x + w))
                .max(0.);
            let dy = (f64::from(y) - point.y)
                .max(point.y - f64::from(y + h))
                .max(0.);
            dx.hypot(dy) < reach
        })
    }

    /// The commit of an element drawing this glass for an effect at `commit`: it changes too
    /// when the pointer moves within reach or the accent changes, which the effect doesn't
    /// count.
    pub fn commit(&self, commit: CommitCounter) -> CommitCounter {
        let mut hasher = DefaultHasher::new();
        commit
            .distance(Some(CommitCounter::default()))
            .hash(&mut hasher);
        self.pointer.map(|p| p.map(f32::to_bits)).hash(&mut hasher);
        let tint = self.glass.tint(self.accent);
        tint.to_array_unpremul().map(f32::to_bits).hash(&mut hasher);
        CommitCounter::from(hasher.finish() as usize)
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
    uniforms.extend(
        GLASS_SHAPE_UNIFORMS
            .iter()
            .zip(shapes.rects)
            .chain(GLASS_RADII_UNIFORMS.iter().zip(shapes.radii))
            .map(|(name, values)| Uniform::new(*name, values)),
    );
    uniforms
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

#[derive(Debug)]
struct Glass {
    effect: FramebufferEffect,
    shapes: Vec<GlassShape>,
}

/// Per-surface glass stored in its data map.
struct SurfaceGlass(Mutex<Glass>);

impl SurfaceGlass {
    fn get(states: &SurfaceData) -> &Self {
        states.data_map.get_or_insert_threadsafe(|| {
            SurfaceGlass(Mutex::new(Glass {
                effect: FramebufferEffect::new(),
                shapes: Vec::new(),
            }))
        })
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
    push: &mut dyn FnMut(FramebufferEffectElement),
) {
    let mut shapes = glass_shapes(surface);
    shapes.retain(|shape| !shape.rect.is_empty());
    let Some(bounds) = bounds(&shapes) else {
        return;
    };

    with_states(surface, |states| {
        let glass = SurfaceGlass::get(states);
        let mut glass = glass.0.lock().unwrap();
        if glass.shapes != shapes {
            glass.effect.damage();
            glass.shapes = shapes.clone();
        }

        let geometry = Rectangle::new(location + bounds.loc.to_f64(), bounds.size.to_f64())
            .to_physical_precise_round(scale)
            .to_logical(scale);

        let mut uniforms = GlassShapes {
            count: shapes.len(),
            glass: blur.glass,
            accent: accent(),
            ..GlassShapes::default()
        };
        for (i, shape) in shapes.iter().enumerate() {
            let rect = shape.rect;
            let loc = rect.loc - bounds.loc;
            uniforms.rects[i] = [
                loc.x as f32,
                loc.y as f32,
                rect.size.w as f32,
                rect.size.h as f32,
            ];
            uniforms.radii[i] = [shape.radius as f32; 4];
        }
        let uniforms = uniforms.lit(frame, geometry.loc, 1.);

        let params = RenderParams {
            geometry,
            subregion: None,
            clip: None,
            scale,
        };
        push(glass.effect.render_glass(
            ns,
            params,
            BlurOptions::reaching(blur),
            blur.noise as f32,
            blur.saturation as f32,
            uniforms,
        ));
    });
}

/// The area the glass covers, in surface coordinates.
fn bounds(shapes: &[GlassShape]) -> Option<Rectangle<i32, Logical>> {
    let bounds = shapes
        .iter()
        .map(|shape| shape.rect)
        .reduce(|a, b| a.merge(b))?;
    Some(Rectangle::new(
        bounds.loc - Point::from((MELT_REACH, MELT_REACH)),
        bounds.size + (MELT_REACH * 2, MELT_REACH * 2).into(),
    ))
}
