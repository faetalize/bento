//! Glass: the material that blurs what's behind, bends it like a lens near the edges and lights
//! the edges. Background effects use it, and the shell gets it behind its layer surfaces in the
//! shapes it gives through `bento_glass_v1`.

use std::sync::Mutex;

use glam::{Mat3, Vec2};
use niri_config::CornerRadius;
use smithay::backend::renderer::gles::{
    ffi, GlesError, GlesFrame, GlesTexture, Uniform, UniformValue,
};
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::compositor::{with_states, SurfaceData};
use wayland_server::protocol::wl_surface::WlSurface;

use crate::protocols::bento_shell::{glass_shapes, GlassShape, MAX_GLASS_SHAPES};
use crate::render_helpers::background_effect::RenderParams;
use crate::render_helpers::blur::BlurOptions;
use crate::render_helpers::framebuffer_effect::{FramebufferEffect, FramebufferEffectElement};
use crate::render_helpers::shaders::{
    mat3_uniform, GLASS_RADII_UNIFORMS, GLASS_SHAPE_UNIFORMS, GLASS_SHARP_UNIT,
};

/// Shapes of glass, in the coordinates of the geometry the shader clips to.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GlassShapes {
    pub rects: [[f32; 4]; MAX_GLASS_SHAPES],
    /// Top left, top right, bottom right and bottom left.
    pub radii: [[f32; 4]; MAX_GLASS_SHAPES],
    pub count: usize,
    /// How the glass bends and lights what's behind it.
    pub glass: niri_config::Glass,
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
            ..Self::default()
        };
        shapes.rects[0] = [0., 0., width, height];
        shapes.radii[0] = radius.into();
        shapes
    }
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
    let angle = glass.light_angle.to_radians();
    let light_direction = [angle.sin() as f32, -angle.cos() as f32];
    let mut uniforms = vec![
        Uniform::new("niri_scale", scale),
        Uniform::new("geo_size", <[f32; 2]>::from(geo_size)),
        mat3_uniform("input_to_geo", input_to_geo),
        mat3_uniform("geo_to_input", input_to_geo.inverse()),
        Uniform::new("shape_count", shapes.count as f32),
        Uniform::new("sharp", UniformValue::_1i(GLASS_SHARP_UNIT as i32)),
        Uniform::new("clarity", glass.clarity as f32),
        Uniform::new("noise", noise),
        Uniform::new("saturation", saturation),
        Uniform::new("bg_color", bg_color),
        Uniform::new("bezel", glass.bezel as f32),
        Uniform::new("thickness", glass.thickness as f32),
        Uniform::new("refractive_index", glass.refractive_index as f32),
        Uniform::new("dispersion", glass.dispersion as f32),
        Uniform::new("highlight", glass.highlight as f32),
        Uniform::new("light_direction", light_direction),
    ];
    uniforms.extend(
        GLASS_SHAPE_UNIFORMS
            .iter()
            .zip(shapes.rects)
            .chain(GLASS_RADII_UNIFORMS.iter().zip(shapes.radii))
            .map(|(name, values)| Uniform::new(*name, values)),
    );
    uniforms
}

/// Binds the sharp copy of what's behind where the glass shader reads it, for one draw.
pub fn bind_sharp(frame: &mut GlesFrame<'_, '_>, sharp: &GlesTexture) -> Result<(), GlesError> {
    frame.with_context(|gl| unsafe {
        gl.ActiveTexture(ffi::TEXTURE0 + GLASS_SHARP_UNIT);
        gl.BindTexture(ffi::TEXTURE_2D, sharp.tex_id());
        gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, ffi::LINEAR as i32);
        gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
        gl.TexParameteri(
            ffi::TEXTURE_2D,
            ffi::TEXTURE_WRAP_S,
            ffi::CLAMP_TO_EDGE as i32,
        );
        gl.TexParameteri(
            ffi::TEXTURE_2D,
            ffi::TEXTURE_WRAP_T,
            ffi::CLAMP_TO_EDGE as i32,
        );
        gl.ActiveTexture(ffi::TEXTURE0);
    })
}

/// Unbinds what [`bind_sharp`] bound, once the draw is done.
pub fn unbind_sharp(frame: &mut GlesFrame<'_, '_>) -> Result<(), GlesError> {
    frame.with_context(|gl| unsafe {
        gl.ActiveTexture(ffi::TEXTURE0 + GLASS_SHARP_UNIT);
        gl.BindTexture(ffi::TEXTURE_2D, 0);
        gl.ActiveTexture(ffi::TEXTURE0);
    })
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

        let params = RenderParams {
            geometry,
            subregion: None,
            clip: None,
            scale,
        };
        push(glass.effect.render_glass(
            ns,
            params,
            BlurOptions::from(blur),
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
