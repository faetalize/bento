//! Glass behind the shell's layer surfaces, in the shapes the shell gives through
//! `bento_glass_v1`.

use std::sync::Mutex;

use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::compositor::{with_states, SurfaceData};
use wayland_server::protocol::wl_surface::WlSurface;

use crate::protocols::bento_shell::{glass_shapes, GlassShape};
use crate::render_helpers::background_effect::RenderParams;
use crate::render_helpers::blur::BlurOptions;
use crate::render_helpers::framebuffer_effect::{
    FramebufferEffect, FramebufferEffectElement, GlassShapes,
};

/// The blur behind glass: light, so the bending stays visible.
const BLUR: BlurOptions = BlurOptions {
    passes: 2,
    offset: 2.5,
};
const NOISE: f32 = 0.015;
const SATURATION: f32 = 1.4;

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

/// Renders the surface's glass, if it has any, for a surface at `location`.
pub fn render(
    ns: Option<usize>,
    surface: &WlSurface,
    location: Point<f64, Logical>,
    scale: f64,
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
            uniforms.radii[i] = shape.radius as f32;
        }

        let params = RenderParams {
            geometry,
            subregion: None,
            clip: None,
            scale,
        };
        push(
            glass
                .effect
                .render_glass(ns, params, BLUR, NOISE, SATURATION, uniforms),
        );
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
