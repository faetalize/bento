use std::sync::{Arc, Mutex};

use niri_config::{CornerRadius, Material};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Point, Rectangle, Scale};
use smithay::wayland::compositor::{with_states, SurfaceData};
use wayland_server::protocol::wl_surface::WlSurface;

use crate::handlers::background_effect::get_cached_blur_region;
use crate::niri_render_elements;
use crate::render_helpers::blur::BlurOptions;
use crate::render_helpers::clipped_surface::rounded_overlap;
use crate::render_helpers::damage::ExtraDamage;
use crate::render_helpers::framebuffer_effect::{FramebufferEffect, FramebufferEffectElement};
use crate::render_helpers::glass::{self, GlassShapes};
use crate::render_helpers::xray::{XrayElement, XrayPos};
use crate::render_helpers::RenderCtx;
use crate::utils::region::TransformedRegion;
use crate::utils::surface_geo;

#[derive(Debug, Clone)]
pub struct BackgroundEffect {
    nonxray: FramebufferEffect,
    /// Damage when options change.
    damage: ExtraDamage,
    /// Corner radius for clipping.
    ///
    /// Stored here in addition to `RenderParams` to damage when it changes.
    // FIXME: would be good to remove this duplication of radius.
    corner_radius: CornerRadius,
    blur_config: niri_config::Blur,
    options: Options,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Options {
    pub blur: bool,
    pub xray: bool,
    pub noise: Option<f64>,
    pub saturation: Option<f64>,
    /// The material when blurred, or `None` for the `blur {}` section's.
    pub material: Option<Material>,
}

impl Options {
    fn is_visible(&self) -> bool {
        self.xray
            || self.blur
            || self.noise.is_some_and(|x| x > 0.)
            || self.saturation.is_some_and(|x| x != 1.)
    }
}

/// Render-time parameters.
#[derive(Debug, Clone)]
pub struct RenderParams {
    /// Geometry of the background effect.
    pub geometry: Rectangle<f64, Logical>,
    /// Effect subregion, will be clipped to `geometry`.
    ///
    /// `subregion.iter()` should return `geometry`-relative rectangles.
    pub subregion: Option<TransformedRegion>,
    /// Geometry and radius for clipping in the same coordinate space as `geometry`.
    pub clip: Option<(Rectangle<f64, Logical>, CornerRadius)>,
    /// Scale to use for rounding to physical pixels.
    pub scale: f64,
}

impl RenderParams {
    fn fit_clip_radius(&mut self) {
        if let Some((geo, radius)) = &mut self.clip {
            // HACK: increase radius to avoid slight bleed on rounded corners.
            *radius = radius.expanded_by(1.);

            *radius = radius.fit_to(geo.size.w as f32, geo.size.h as f32);
        }
    }
}

niri_render_elements! {
    BackgroundEffectElement => {
        FramebufferEffect = FramebufferEffectElement,
        Xray = XrayElement,
        ExtraDamage = ExtraDamage,
    }
}

impl BackgroundEffectElement {
    /// The effect as opaque as `alpha`, for one that fades with the window it's behind.
    pub fn with_alpha(self, alpha: f32) -> Self {
        match self {
            Self::FramebufferEffect(elem) => Self::FramebufferEffect(elem.with_alpha(alpha)),
            Self::Xray(elem) => Self::Xray(elem.with_alpha(alpha)),
            Self::ExtraDamage(elem) => Self::ExtraDamage(elem),
        }
    }
}

impl BackgroundEffect {
    pub fn new() -> Self {
        Self {
            nonxray: FramebufferEffect::new(),
            damage: ExtraDamage::new(),
            corner_radius: CornerRadius::default(),
            blur_config: niri_config::Blur::default(),
            options: Options::default(),
        }
    }

    /// Damage the background effect, for example when a blur subregion changes.
    pub fn damage(&mut self) {
        self.damage.damage_all();
        self.nonxray.damage();
    }

    pub fn update_config(&mut self, config: niri_config::Blur) {
        if self.blur_config == config {
            return;
        }

        self.blur_config = config;
        self.damage.damage_all();
        self.nonxray.damage();
    }

    pub fn update_render_elements(
        &mut self,
        corner_radius: CornerRadius,
        effect: niri_config::BackgroundEffect,
        has_blur_region: bool,
    ) {
        // If the surface explicitly requests a blur region, default blur to true.
        let blur = if has_blur_region {
            effect.blur != Some(false)
        } else {
            effect.blur == Some(true)
        };

        let mut options = Options {
            blur,
            xray: effect.xray == Some(true),
            noise: effect.noise,
            saturation: effect.saturation,
            material: effect.material,
        };

        // If we have some background effect but xray wasn't explicitly set, default it to true
        // since it's cheaper.
        if options.is_visible() && effect.xray.is_none() {
            options.xray = true;
        }

        if self.options == options && self.corner_radius == corner_radius {
            return;
        }

        self.options = options;
        self.corner_radius = corner_radius;
        self.damage.damage_all();
        self.nonxray.damage();
    }

    pub fn is_visible(&self) -> bool {
        self.options.is_visible()
    }

    /// Renders the effect, cut to `outer_clip` too if it's given, like a window's is to the clip
    /// that a layout engine draws it in.
    pub fn render(
        &self,
        ctx: RenderCtx<GlesRenderer>,
        ns: Option<usize>,
        mut params: RenderParams,
        outer_clip: Option<(Rectangle<f64, Logical>, CornerRadius)>,
        xray_pos: XrayPos,
        push: &mut dyn FnMut(BackgroundEffectElement),
    ) {
        if !self.is_visible() {
            return;
        }

        if let Some(clip) = &mut params.clip {
            clip.1 = self.corner_radius;
        }
        params.fit_clip_radius();

        if let Some(outer) = outer_clip {
            let own = params
                .clip
                .unwrap_or((params.geometry, CornerRadius::default()));
            let Some(clip) = rounded_overlap(own, outer) else {
                return;
            };
            params.clip = Some(clip);
        }

        let damage = self.damage.render(params.geometry);

        // Use noise/saturation from options, falling back to blur defaults if blurred, and
        // to no effect if not blurred.
        let blur = self.options.blur && !self.blur_config.off;
        let blur_options = blur
            .then(|| BlurOptions::reaching(self.blur_config))
            .flatten();
        let noise = if blur { self.blur_config.noise } else { 0. };
        let noise = self.options.noise.unwrap_or(noise) as f32;
        let saturation = if blur {
            self.blur_config.saturation
        } else {
            1.
        };
        let saturation = self.options.saturation.unwrap_or(saturation) as f32;
        // Only what's blurred is shaded: what shows through as it is reads as it is.
        let shade = if blur {
            glass::shade(&self.blur_config)
        } else {
            [0.; 4]
        };

        // Glass is a material for blur; without blur, what's behind shows as it is.
        let material = self.options.material.unwrap_or(self.blur_config.material);
        let glass = (blur && material == Material::Glass)
            .then(|| glass_shapes(&params, self.blur_config.glass));

        if self.options.xray {
            let Some(xray) = ctx.xray else {
                return;
            };

            push(damage.into());
            xray.render(
                ctx,
                params,
                xray_pos,
                blur_options.is_some(),
                noise,
                saturation,
                shade,
                glass,
                &mut |elem| push(elem.into()),
            );
        } else if let Some(shapes) = glass {
            let origin = params.clip.map_or(params.geometry.loc, |(geo, _)| geo.loc);
            let shapes = shapes.lit(ctx.glass, origin, 1.);
            let elem = self.nonxray.render_glass(
                ns,
                params,
                blur_options,
                noise,
                saturation,
                shade,
                shapes,
            );
            if let Some(look) = elem.look() {
                push(look.into());
            }
            push(elem.into());
        } else {
            // Render non-xray effect.
            let elem = self
                .nonxray
                .render(ns, params, blur_options, noise, saturation, shade);
            if let Some(look) = elem.look() {
                push(look.into());
            }
            push(elem.into());
        }
    }
}

/// The glass for an effect drawn with `params`, in the coordinates of its clip geometry, or of its
/// geometry without one.
fn glass_shapes(params: &RenderParams, glass: niri_config::Glass) -> GlassShapes {
    // An app's own region, unclipped, is the glass's shape: the surface around it can hold more,
    // like the shadow GTK draws, where an edge would never show.
    if let (None, Some(region)) = (&params.clip, &params.subregion) {
        return GlassShapes::following(region, params.geometry.loc, glass);
    }

    let (geo, radius) = params
        .clip
        .unwrap_or((params.geometry, CornerRadius::default()));
    GlassShapes::filling(geo.size.w as f32, geo.size.h as f32, radius, glass)
}

fn render_params_for_tile(
    geometry: Rectangle<f64, Logical>,
    scale: f64,
    clip_to_geometry: bool,
    block_out: bool,
    blur_region: Option<Arc<Vec<Rectangle<i32, Logical>>>>,
    surface_geo: Rectangle<f64, Logical>,
    surface_anim_scale: Scale<f64>,
) -> Option<RenderParams> {
    // Effects not requested by the surface itself are drawn to match the geometry.
    let mut clip = true;

    let mut effect_geometry = geometry;
    let mut subregion = None;
    if let Some(rects) = blur_region {
        if rects.is_empty() {
            // Surface has a set, but empty blur region.
            return None;
        } else {
            // If the surface itself requests the effects, apply different defaults.
            clip = clip_to_geometry;

            // Use geometry-shaped blur for blocked-out windows to avoid unintentionally
            // leaking any surface shapes. We render those windows as geometry-shaped solid
            // rectangles anyway.
            if block_out {
                clip = true;
            } else {
                let mut surface_geo = surface_geo.upscale(surface_anim_scale);
                surface_geo.loc += geometry.loc;

                subregion = Some(TransformedRegion {
                    rects,
                    scale: surface_anim_scale,
                    offset: surface_geo.loc,
                });

                surface_geo = surface_geo
                    .to_physical_precise_round(scale)
                    .to_logical(scale);
                effect_geometry = surface_geo;
            }
        }
    }

    // This corner radius is reset to self.corner_radius in render().
    let clip = clip.then_some((geometry, CornerRadius::default()));

    Some(RenderParams {
        geometry: effect_geometry,
        subregion,
        clip,
        scale,
    })
}

/// Per-surface background effect stored in its data map.
struct SurfaceBackgroundEffect(Mutex<BackgroundEffect>);

impl SurfaceBackgroundEffect {
    fn get(states: &SurfaceData) -> &Self {
        states
            .data_map
            .get_or_insert(|| SurfaceBackgroundEffect(Mutex::new(BackgroundEffect::new())))
    }
}

pub fn damage_surface(states: &SurfaceData) {
    if let Some(effect) = states.data_map.get::<SurfaceBackgroundEffect>() {
        effect.0.lock().unwrap().damage();
    }
}

/// A window's background effect, ready to draw: the effect with its settings, and where it
/// goes. It's taken from the window's surface each frame, and kept when the window closes, so its
/// closing animation goes on drawing it after the surface is gone.
#[derive(Debug, Clone)]
pub struct CapturedEffect {
    effect: BackgroundEffect,
    params: RenderParams,
    outer_clip: Option<(Rectangle<f64, Logical>, CornerRadius)>,
    /// Where the window's geometry starts, which `xray_pos` is for.
    origin: Point<f64, Logical>,
}

impl CapturedEffect {
    /// Where the window's geometry starts.
    pub fn origin(&self) -> Point<f64, Logical> {
        self.origin
    }

    /// The effect moved by `offset`, then scaled by `zoom` about `middle`, for a window drawn
    /// moved and scaled so.
    pub fn transformed(
        &self,
        offset: Point<f64, Logical>,
        middle: Point<f64, Logical>,
        zoom: f64,
    ) -> Self {
        let point = |p: Point<f64, Logical>| middle + (p + offset - middle).upscale(zoom);
        let rect = |r: Rectangle<f64, Logical>| Rectangle::new(point(r.loc), r.size.upscale(zoom));
        let clip = |(r, radius): (Rectangle<f64, Logical>, CornerRadius)| {
            (rect(r), radius.scaled_by(zoom as f32))
        };
        let mut captured = self.clone();
        captured.params.geometry = rect(self.params.geometry);
        captured.params.clip = self.params.clip.map(clip);
        if let Some(region) = &mut captured.params.subregion {
            region.offset = point(region.offset);
            region.scale = Scale::from((region.scale.x * zoom, region.scale.y * zoom));
        }
        captured.outer_clip = self.outer_clip.map(clip);
        captured.origin = point(self.origin);
        captured
    }

    /// Draws the effect, with `xray_pos` where the window's geometry starts in the backdrop.
    pub fn render(
        &self,
        ctx: RenderCtx<GlesRenderer>,
        ns: Option<usize>,
        xray_pos: XrayPos,
        push: &mut dyn FnMut(BackgroundEffectElement),
    ) {
        let xray_pos = xray_pos.offset(self.params.geometry.loc - self.origin);
        self.effect.render(
            ctx,
            ns,
            self.params.clone(),
            self.outer_clip,
            xray_pos,
            push,
        );
    }
}

/// A window's background effect as it is now, if it shows.
#[allow(clippy::too_many_arguments)]
pub fn capture_for_tile(
    geometry: Rectangle<f64, Logical>,
    scale: f64,
    clip_to_geometry: bool,
    surface: &WlSurface,
    surface_off: Point<f64, Logical>,
    surface_anim_scale: Scale<f64>,
    blur_config: niri_config::Blur,
    radius: CornerRadius,
    effect: niri_config::BackgroundEffect,
    should_block_out: bool,
    outer_clip: Option<(Rectangle<f64, Logical>, CornerRadius)>,
) -> Option<CapturedEffect> {
    with_states(surface, |states| {
        let background_effect = SurfaceBackgroundEffect::get(states);
        let mut background_effect = background_effect.0.lock().unwrap();

        let blur_region = get_cached_blur_region(states);
        let has_blur_region = blur_region.as_ref().is_some_and(|r| !r.is_empty());

        background_effect.update_config(blur_config);
        background_effect.update_render_elements(radius, effect, has_blur_region);

        if !background_effect.is_visible() {
            return None;
        }

        let mut surface_geo = surface_geo(states).unwrap_or_default().to_f64();
        surface_geo.loc += surface_off;

        let params = render_params_for_tile(
            geometry,
            scale,
            clip_to_geometry,
            should_block_out,
            blur_region,
            surface_geo,
            surface_anim_scale,
        )?;

        Some(CapturedEffect {
            effect: background_effect.clone(),
            params,
            outer_clip,
            origin: geometry.loc,
        })
    })
}

#[allow(clippy::too_many_arguments)]
pub fn render_for_tile(
    ctx: RenderCtx<GlesRenderer>,
    ns: Option<usize>,
    geometry: Rectangle<f64, Logical>,
    scale: f64,
    clip_to_geometry: bool,
    surface: &WlSurface,
    surface_off: Point<f64, Logical>,
    surface_anim_scale: Scale<f64>,
    blur_config: niri_config::Blur,
    radius: CornerRadius,
    effect: niri_config::BackgroundEffect,
    should_block_out: bool,
    outer_clip: Option<(Rectangle<f64, Logical>, CornerRadius)>,
    xray_pos: XrayPos,
    push: &mut dyn FnMut(BackgroundEffectElement),
) {
    let captured = capture_for_tile(
        geometry,
        scale,
        clip_to_geometry,
        surface,
        surface_off,
        surface_anim_scale,
        blur_config,
        radius,
        effect,
        should_block_out,
        outer_clip,
    );
    if let Some(captured) = captured {
        captured.render(ctx, ns, xray_pos, push);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rounded rectangle's region as GTK gives it, from the rounded rectangle drawn in whole
    /// pixels: a row for each pixel row of the corners, and one rectangle between.
    fn rounded_region(rect: Rectangle<i32, Logical>, radius: i32) -> Vec<Rectangle<i32, Logical>> {
        let (x, y, w, h) = (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h);
        let r = f64::from(radius);
        let inset = |row: i32| {
            let dy = r - f64::from(row) - 0.5;
            (r - (r * r - dy * dy).sqrt()).round() as i32
        };
        let mut rects = Vec::new();
        for row in 0..radius {
            let i = inset(row);
            rects.push(Rectangle::new(
                (x + i, y + row).into(),
                (w - 2 * i, 1).into(),
            ));
        }
        rects.push(Rectangle::new(
            (x, y + radius).into(),
            (w, h - 2 * radius).into(),
        ));
        for row in (0..radius).rev() {
            let i = inset(row);
            rects.push(Rectangle::new(
                (x + i, y + h - 1 - row).into(),
                (w - 2 * i, 1).into(),
            ));
        }
        rects
    }

    #[test]
    fn glass_an_app_asks_for_takes_its_regions_shape() {
        // A GTK window with rounded corners, in a surface with room for its shadow around it.
        let geometry = Rectangle::new((100., 50.).into(), (440., 340.).into());
        let rects = rounded_region(Rectangle::new((20, 20).into(), (400, 300).into()), 12);
        let params = RenderParams {
            geometry,
            subregion: Some(TransformedRegion {
                rects: Arc::new(rects),
                scale: Scale::from(1.),
                offset: geometry.loc,
            }),
            clip: None,
            scale: 1.,
        };

        let shapes = glass_shapes(&params, niri_config::Glass::default());
        assert_eq!(shapes.count, 1);
        assert_eq!(shapes.rects[0], [20., 20., 400., 300.]);
        for radius in shapes.radii[0] {
            assert!((radius - 12.).abs() < 0.75, "{:?}", shapes.radii[0]);
        }
    }
}
