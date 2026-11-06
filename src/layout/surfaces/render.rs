use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context as _};
use niri_config::{Color, CornerRadius, Gradient, GradientRelativeTo};
use pangocairo::cairo::{self, ImageSurface};
use pangocairo::pango::{EllipsizeMode, FontDescription};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::utils::CropRenderElement;
use smithay::backend::renderer::element::{Element, Kind};
use smithay::backend::renderer::gles::GlesTexture;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};

use super::props::Paint;
use super::{icons, Drawn, Node, NodeKind, WindowSlot};
use crate::niri_render_elements;
use crate::render_helpers::border::BorderRenderElement;
use crate::render_helpers::memory::MemoryBuffer;
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::solid_color::{SolidColorBuffer, SolidColorRenderElement};
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};
use crate::utils::to_physical_precise_round;

niri_render_elements! {
    SurfaceRenderElement => {
        SolidColor = SolidColorRenderElement,
        Border = BorderRenderElement,
        Texture = PrimaryGpuTextureRenderElement,
        // The same, cut off by an ancestor's clip.
        ClippedSolidColor = CropRenderElement<SolidColorRenderElement>,
        ClippedBorder = CropRenderElement<BorderRenderElement>,
        ClippedTexture = CropRenderElement<PrimaryGpuTextureRenderElement>,
    }
}

/// What a tree draws, in draw order.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum RenderItem {
    Element(SurfaceRenderElement),
    /// Where a window goes. The caller draws the window there, with these properties.
    Window(WindowSlot),
}

/// Where and how sharply a tree is drawn.
pub(super) struct Target<'a> {
    /// Where the canvas origin is on the output.
    pub location: Point<f64, Logical>,
    /// The output's scale.
    pub scale: f64,
    /// The workspace's view, which gradients that span it cover, in the same coordinates as
    /// `location`.
    pub view: Rectangle<f64, Logical>,
    pub icon_theme: &'a str,
    pub has_border_shader: bool,
}

/// What a node keeps between frames, so that its elements keep their ids and its textures their
/// contents.
#[derive(Debug, Default)]
pub(super) struct Visuals {
    fill: SolidColorBuffer,
    fill_shader: BorderRenderElement,
    edges: [SolidColorBuffer; 4],
    ring_shader: BorderRenderElement,
    text: Option<CachedText>,
    image: Option<CachedImage>,
}

#[derive(Debug, PartialEq)]
struct TextKey {
    text: String,
    font: String,
    color: Color,
    scale: f64,
}

#[derive(Debug)]
struct CachedText {
    key: TextKey,
    /// The width of the text without ellipsis, in physical pixels.
    natural_width: i32,
    /// The width it was cut to with an ellipsis, in physical pixels.
    limit: Option<i32>,
    texture: Option<TextureBuffer<GlesTexture>>,
}

#[derive(Debug, PartialEq)]
struct ImageKey {
    name: String,
    size: u32,
    theme: String,
}

#[derive(Debug)]
struct CachedImage {
    key: ImageKey,
    texture: Option<TextureBuffer<GlesTexture>>,
}

impl CachedText {
    /// Whether this is still how `key` looks when it has `max_width` pixels.
    fn fits(&self, key: &TextKey, max_width: i32) -> bool {
        self.key == *key
            && match self.limit {
                Some(limit) => limit == max_width,
                None => self.natural_width <= max_width,
            }
    }
}

fn emit<E>(
    elem: E,
    crop: Option<Rectangle<i32, Physical>>,
    scale: f64,
    push: &mut dyn FnMut(RenderItem),
) where
    E: Element,
    SurfaceRenderElement: From<E> + From<CropRenderElement<E>>,
{
    match crop {
        None => push(RenderItem::Element(elem.into())),
        Some(crop) => {
            if let Some(elem) = CropRenderElement::from_element(elem, scale, crop) {
                push(RenderItem::Element(elem.into()));
            }
        }
    }
}

/// Draws a box, text or image node. Window nodes only get a slot, from the caller.
pub(super) fn draw<R: NiriRenderer>(
    renderer: &mut R,
    node: &Node,
    drawn: &Drawn,
    target: &Target,
    push: &mut dyn FnMut(RenderItem),
) {
    if !matches!(node.kind, NodeKind::Box | NodeKind::Text | NodeKind::Image) {
        return;
    }

    let bounds = drawn.bounds();
    if drawn.opacity <= 0. || bounds.size.w <= 0. || bounds.size.h <= 0. {
        return;
    }

    let scale = target.scale;
    let alpha = drawn.opacity as f32;
    let size = bounds.size;
    let loc = bounds.loc + target.location;
    let crop: Option<Rectangle<i32, Physical>> = drawn.clip.map(|clip| {
        Rectangle::new(clip.rect.loc + target.location, clip.rect.size)
            .to_physical_precise_round(scale)
    });

    let radius = CornerRadius::from((node.corner_radius.current() * drawn.transform.scale) as f32)
        .fit_to(size.w as f32, size.h as f32);
    // Rounded corners and gradients need the shader. Without it a paint is a flat colour.
    let shaded = |paint: Paint| {
        target.has_border_shader
            && (radius != CornerRadius::default() || matches!(paint, Paint::Gradient(_)))
    };

    let mut visuals = node.visuals.borrow_mut();

    if let Some(paint) = node.background.current().filter(|paint| paint.is_visible()) {
        if shaded(paint) {
            update_shader(
                &mut visuals.fill_shader,
                paint.gradient(),
                size,
                loc,
                target,
                0.,
                radius,
                alpha,
            );
            let elem = visuals.fill_shader.clone().with_location(loc);
            emit(elem, crop, scale, push);
        } else {
            visuals.fill.update(size, paint.solid());
            let elem =
                SolidColorRenderElement::from_buffer(&visuals.fill, loc, alpha, Kind::Unspecified);
            emit(elem, crop, scale, push);
        }
    }

    match node.kind {
        NodeKind::Text => draw_text(renderer, node, &mut visuals, drawn, target, crop, push),
        NodeKind::Image => draw_image(renderer, node, &mut visuals, drawn, target, crop, push),
        _ => (),
    }

    if let Some(border) = node.border {
        // The border is drawn inside the rect, so it can't be wider than half of it.
        let width = (border.width * drawn.transform.scale)
            .min(size.w / 2.)
            .min(size.h / 2.);
        if width <= 0. || !border.color.is_visible() {
            return;
        }

        if shaded(border.color) {
            update_shader(
                &mut visuals.ring_shader,
                border.color.gradient(),
                size,
                loc,
                target,
                width as f32,
                radius,
                alpha,
            );
            let elem = visuals.ring_shader.clone().with_location(loc);
            emit(elem, crop, scale, push);
        } else {
            // Four edges that don't overlap, so that a translucent border has no darker corners.
            let side: Size<f64, Logical> = Size::from((width, size.h - width * 2.));
            let edges: [(Point<f64, Logical>, Size<f64, Logical>); 4] = [
                (Point::from((0., 0.)), Size::from((size.w, width))),
                (
                    Point::from((0., size.h - width)),
                    Size::from((size.w, width)),
                ),
                (Point::from((0., width)), side),
                (Point::from((size.w - width, width)), side),
            ];
            for (buffer, (offset, size)) in visuals.edges.iter_mut().zip(edges) {
                buffer.update(size, border.color.solid());
                let elem = SolidColorRenderElement::from_buffer(
                    buffer,
                    loc + offset,
                    alpha,
                    Kind::Unspecified,
                );
                emit(elem, crop, scale, push);
            }
        }
    }
}

/// Where a gradient is spread, for an element at `loc` that is `size` large, relative to the
/// element: the element itself, or the whole view of the workspace.
fn gradient_area(
    gradient: &Gradient,
    size: Size<f64, Logical>,
    loc: Point<f64, Logical>,
    view: Rectangle<f64, Logical>,
) -> Rectangle<f64, Logical> {
    match gradient.relative_to {
        GradientRelativeTo::Window => Rectangle::from_size(size),
        GradientRelativeTo::WorkspaceView => Rectangle::new(view.loc - loc, view.size),
    }
}

/// Sets what a border shader element draws: a node's fill when `border_width` is 0, or its
/// border.
#[allow(clippy::too_many_arguments)]
fn update_shader(
    shader: &mut BorderRenderElement,
    gradient: Gradient,
    size: Size<f64, Logical>,
    loc: Point<f64, Logical>,
    target: &Target,
    border_width: f32,
    radius: CornerRadius,
    alpha: f32,
) {
    shader.update(
        size,
        gradient_area(&gradient, size, loc, target.view),
        gradient.in_,
        gradient.from,
        gradient.to,
        // The shader's angle is from the x axis, niri's from the top.
        (f32::from(gradient.angle) - 90.).to_radians(),
        Rectangle::from_size(size),
        border_width,
        radius,
        target.scale as f32,
        alpha,
    );
}

/// Draws a text node's text, centred in its rect, cut with an ellipsis where it's too wide.
fn draw_text<R: NiriRenderer>(
    renderer: &mut R,
    node: &Node,
    visuals: &mut Visuals,
    drawn: &Drawn,
    target: &Target,
    crop: Option<Rectangle<i32, Physical>>,
    push: &mut dyn FnMut(RenderItem),
) {
    if node.text.is_empty() {
        return;
    }

    let key = TextKey {
        text: node.text.clone(),
        font: node.font.clone(),
        color: node.color,
        scale: target.scale,
    };
    let max_width = (node.rect.current().size.w * target.scale).round() as i32;

    if !visuals
        .text
        .as_ref()
        .is_some_and(|cached| cached.fits(&key, max_width))
    {
        let rasterized = rasterize_text(&key, max_width).and_then(|text| {
            let texture =
                TextureBuffer::from_memory_buffer(renderer.as_gles_renderer(), &text.buffer)?;
            Ok((text.natural_width, text.limit, texture))
        });
        let (natural_width, limit, texture) = match rasterized {
            Ok((natural_width, limit, texture)) => (natural_width, limit, Some(texture)),
            Err(err) => {
                warn!("error drawing the text {:?}: {err:?}", key.text);
                (0, None, None)
            }
        };
        visuals.text = Some(CachedText {
            key,
            natural_width,
            limit,
            texture,
        });
    }

    let Some(buffer) = visuals
        .text
        .as_ref()
        .and_then(|cached| cached.texture.clone())
    else {
        return;
    };

    let bounds = drawn.bounds();
    let natural = buffer.logical_size();
    let size = Size::from((
        natural.w * drawn.transform.scale,
        natural.h * drawn.transform.scale,
    ));
    let offset = Point::from(((bounds.size.w - size.w) / 2., (bounds.size.h - size.h) / 2.));

    let elem = TextureRenderElement::from_texture_buffer(
        buffer,
        bounds.loc + target.location + offset,
        drawn.opacity as f32,
        None,
        Some(size),
        Kind::Unspecified,
    );
    emit(
        PrimaryGpuTextureRenderElement(elem),
        crop,
        target.scale,
        push,
    );
}

/// Draws an image node's image, as large as fits its rect, centred.
fn draw_image<R: NiriRenderer>(
    renderer: &mut R,
    node: &Node,
    visuals: &mut Visuals,
    drawn: &Drawn,
    target: &Target,
    crop: Option<Rectangle<i32, Physical>>,
    push: &mut dyn FnMut(RenderItem),
) {
    let Some(name) = &node.image else {
        return;
    };

    let rect = node.rect.current();
    let key = ImageKey {
        name: name.clone(),
        size: (rect.size.w.max(rect.size.h) * target.scale).round() as u32,
        theme: target.icon_theme.to_owned(),
    };

    if visuals
        .image
        .as_ref()
        .is_none_or(|cached| cached.key != key)
    {
        let texture = load_image(&key).and_then(|buffer| {
            Ok(TextureBuffer::from_memory_buffer(
                renderer.as_gles_renderer(),
                &buffer,
            )?)
        });
        let texture = match texture {
            Ok(texture) => Some(texture),
            Err(err) => {
                warn!("error loading the image {name:?}: {err:?}");
                None
            }
        };
        visuals.image = Some(CachedImage { key, texture });
    }

    let Some(buffer) = visuals
        .image
        .as_ref()
        .and_then(|cached| cached.texture.clone())
    else {
        return;
    };

    let bounds = drawn.bounds();
    let natural = buffer.logical_size();
    let fit = (bounds.size.w / natural.w).min(bounds.size.h / natural.h);
    let size = Size::from((natural.w * fit, natural.h * fit));
    let offset = Point::from(((bounds.size.w - size.w) / 2., (bounds.size.h - size.h) / 2.));

    let elem = TextureRenderElement::from_texture_buffer(
        buffer,
        bounds.loc + target.location + offset,
        drawn.opacity as f32,
        None,
        Some(size),
        Kind::Unspecified,
    );
    emit(
        PrimaryGpuTextureRenderElement(elem),
        crop,
        target.scale,
        push,
    );
}

struct RasterizedText {
    buffer: MemoryBuffer,
    natural_width: i32,
    limit: Option<i32>,
}

/// Draws text with Pango. Font sizes are in logical pixels, as in bento's other UI.
fn rasterize_text(key: &TextKey, max_width: i32) -> anyhow::Result<RasterizedText> {
    let _span = tracy_client::span!("surfaces::rasterize_text");

    let mut font = FontDescription::from_string(&key.font);
    font.set_absolute_size(to_physical_precise_round(key.scale, font.size()));

    let layout = |cr: &cairo::Context, limit: Option<i32>| {
        let layout = pangocairo::functions::create_layout(cr);
        layout.context().set_round_glyph_positions(false);
        layout.set_font_description(Some(&font));
        layout.set_text(&key.text);
        if let Some(limit) = limit {
            layout.set_width(limit * pangocairo::pango::SCALE);
            layout.set_ellipsize(EllipsizeMode::End);
        }
        layout
    };

    let surface = ImageSurface::create(cairo::Format::ARgb32, 0, 0)?;
    let cr = cairo::Context::new(&surface)?;
    let (natural_width, _) = layout(&cr, None).pixel_size();
    let limit = (natural_width > max_width).then_some(max_width.max(1));
    let (width, height) = layout(&cr, limit).pixel_size();
    if width <= 0 || height <= 0 {
        bail!("the text has no size");
    }

    let surface = ImageSurface::create(cairo::Format::ARgb32, width, height)?;
    let cr = cairo::Context::new(&surface)?;
    let [r, g, b, a] = key.color.to_array_unpremul().map(f64::from);
    cr.set_source_rgba(r, g, b, a);
    pangocairo::functions::show_layout(&cr, &layout(&cr, limit));
    drop(cr);

    let data = surface
        .take_data()
        .map_err(|err| anyhow!("error reading the text surface: {err:?}"))?;
    let buffer = MemoryBuffer::new(
        data.to_vec(),
        Fourcc::Argb8888,
        (width, height),
        key.scale,
        smithay::utils::Transform::Normal,
    );

    Ok(RasterizedText {
        buffer,
        natural_width,
        limit,
    })
}

/// Largest width and height of an image that a node draws.
const MAX_IMAGE_SIZE: u32 = 4096;

/// Loads an icon from the icon theme, or an image from an absolute path.
fn load_image(key: &ImageKey) -> anyhow::Result<MemoryBuffer> {
    let _span = tracy_client::span!("surfaces::load_image");

    let path = if Path::new(&key.name).is_absolute() {
        PathBuf::from(&key.name)
    } else {
        icons::find(&key.theme, &key.name, key.size)
            .with_context(|| format!("no icon named {:?} in the icon theme", key.name))?
    };

    // An engine names any file it likes, so a huge image mustn't take bento's memory.
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIZE);
    limits.max_image_height = Some(MAX_IMAGE_SIZE);
    limits.max_alloc = Some(u64::from(MAX_IMAGE_SIZE) * u64::from(MAX_IMAGE_SIZE) * 4);
    let mut reader = image::ImageReader::open(&path)
        .and_then(|reader| reader.with_guessed_format())
        .with_context(|| format!("error reading {path:?}"))?;
    reader.limits(limits);
    let image = reader
        .decode()
        .with_context(|| format!("error reading {path:?}"))?
        .into_rgba8();
    let (width, height) = image.dimensions();
    let mut data = image.into_raw();

    // The renderer blends premultiplied colours.
    for pixel in data.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }

    Ok(MemoryBuffer::new(
        data,
        Fourcc::Abgr8888,
        (width as i32, height as i32),
        1.,
        smithay::utils::Transform::Normal,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gradient_spans_its_node_or_the_view() {
        let gradient = Gradient::from(Color::from_rgba8_unpremul(0, 0, 0, 255));
        let size = Size::from((100., 50.));
        let loc = Point::from((30., 20.));
        let view = Rectangle::from_size(Size::from((1280., 720.)));

        assert_eq!(
            gradient_area(&gradient, size, loc, view),
            Rectangle::from_size(size)
        );

        let spanning_view = Gradient {
            relative_to: GradientRelativeTo::WorkspaceView,
            ..gradient
        };
        // The view starts where the output does, which is -30, -20 from the node.
        assert_eq!(
            gradient_area(&spanning_view, size, loc, view),
            Rectangle::new(Point::from((-30., -20.)), view.size)
        );
    }

    fn key(text: &str) -> TextKey {
        TextKey {
            text: text.into(),
            font: "sans 11".into(),
            color: Color::from_rgba8_unpremul(255, 255, 255, 255),
            scale: 1.,
        }
    }

    fn cached(limit: Option<i32>, natural_width: i32) -> CachedText {
        CachedText {
            key: key("hello"),
            natural_width,
            limit,
            texture: None,
        }
    }

    #[test]
    fn text_that_fits_is_not_drawn_again_when_its_box_resizes() {
        let text = cached(None, 80);
        assert!(text.fits(&key("hello"), 80));
        assert!(text.fits(&key("hello"), 400));
        assert!(!text.fits(&key("hello"), 79));
    }

    #[test]
    fn cut_text_is_drawn_again_when_its_width_changes() {
        let text = cached(Some(60), 80);
        assert!(text.fits(&key("hello"), 60));
        assert!(!text.fits(&key("hello"), 61));
        assert!(!text.fits(&key("hello"), 400));
    }

    #[test]
    fn text_is_drawn_again_when_it_changes() {
        let text = cached(None, 80);
        assert!(!text.fits(&key("goodbye"), 400));

        let mut other = key("hello");
        other.scale = 2.;
        assert!(!text.fits(&other, 400));
    }
}
