//! Draws a search node: the query of the running search, or the node's placeholder while it's
//! empty, with the caret and the selection.
//!
//! The text starts inset from the node's left edge by half a line, is centred vertically, and
//! scrolls sideways when it's wider than the node, so that the caret is always in view. The caret
//! and the selection take the node's text colour, the selection with some transparency, and the
//! placeholder the same colour dimmed. The node's background and border are the usual ones.

use anyhow::{anyhow, bail};
use niri_config::Color;
use pangocairo::cairo::{self, ImageSurface};
use pangocairo::pango::{self, FontDescription};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesTexture;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};

use super::render::{emit, RenderItem, Target};
use super::{Drawn, Node};
use crate::render_helpers::memory::MemoryBuffer;
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};
use crate::utils::to_physical_precise_round;

/// How opaque the selection is, relative to the text.
const SELECTION_ALPHA: f64 = 0.35;

/// How opaque the placeholder is, relative to the text.
const PLACEHOLDER_ALPHA: f64 = 0.5;

/// What a search node draws, between frames: the texture and what it was drawn from.
#[derive(Debug, Default)]
pub(super) struct Cache {
    cached: Option<Cached>,
}

#[derive(Debug)]
struct Cached {
    key: Key,
    /// The distance in physical pixels from the node's left edge to the texture, which is also the
    /// distance from the texture to the node's right edge.
    inset: i32,
    texture: Option<TextureBuffer<GlesTexture>>,
}

#[derive(Debug, PartialEq)]
struct Key {
    text: String,
    placeholder: String,
    font: String,
    color: Color,
    scale: f64,
    caret: usize,
    anchor: usize,
    /// The node's width in physical pixels.
    width: i32,
}

struct Rasterized {
    buffer: MemoryBuffer,
    inset: i32,
}

/// How far to move text left, in pixels, for a caret `caret_width` wide at `caret_x` to be in view
/// in `width` pixels.
fn scroll(caret_x: i32, caret_width: i32, width: i32) -> i32 {
    (caret_x + caret_width - width).max(0)
}

/// The distance from a node's edge to its text, for lines `line_height` pixels high.
fn inset(line_height: i32) -> i32 {
    line_height / 2
}

pub(super) fn draw<R: NiriRenderer>(
    renderer: &mut R,
    node: &Node,
    cache: &mut Cache,
    drawn: &Drawn,
    target: &Target,
    crop: Option<Rectangle<i32, Physical>>,
    push: &mut dyn FnMut(RenderItem),
) {
    let Some(field) = target.search else {
        return;
    };

    let key = Key {
        text: field.text().to_owned(),
        placeholder: node.placeholder.clone(),
        font: node.font.clone(),
        color: node.color,
        scale: target.scale,
        caret: field.caret(),
        anchor: field.anchor(),
        width: (node.rect.current().size.w * target.scale).round() as i32,
    };

    if cache.cached.as_ref().is_none_or(|cached| cached.key != key) {
        let rasterized = rasterize(&key).and_then(|text| {
            let texture =
                TextureBuffer::from_memory_buffer(renderer.as_gles_renderer(), &text.buffer)?;
            Ok((text.inset, texture))
        });
        let (inset, texture) = match rasterized {
            Ok((inset, texture)) => (inset, Some(texture)),
            Err(err) => {
                warn!("error drawing the search field: {err:?}");
                (0, None)
            }
        };
        cache.cached = Some(Cached {
            key,
            inset,
            texture,
        });
    }

    let Some(cached) = &cache.cached else {
        return;
    };
    let Some(buffer) = cached.texture.clone() else {
        return;
    };

    let bounds = drawn.bounds();
    let natural = buffer.logical_size();
    let size: Size<f64, Logical> = Size::from((
        natural.w * drawn.transform.scale,
        natural.h * drawn.transform.scale,
    ));
    let offset = Point::from((
        f64::from(cached.inset) / target.scale * drawn.transform.scale,
        (bounds.size.h - size.h) / 2.,
    ));

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

/// Draws the field with Pango. Font sizes are in logical pixels, as in bento's other UI.
fn rasterize(key: &Key) -> anyhow::Result<Rasterized> {
    let _span = tracy_client::span!("surfaces::rasterize_search_field");

    let mut font = FontDescription::from_string(&key.font);
    font.set_absolute_size(to_physical_precise_round(key.scale, font.size()));

    let layout = |cr: &cairo::Context, text: &str| {
        let layout = pangocairo::functions::create_layout(cr);
        layout.context().set_round_glyph_positions(false);
        layout.set_font_description(Some(&font));
        layout.set_text(text);
        layout
    };

    let surface = ImageSurface::create(cairo::Format::ARgb32, 0, 0)?;
    let cr = cairo::Context::new(&surface)?;
    let showing_placeholder = key.text.is_empty();
    let shown = if showing_placeholder {
        &key.placeholder
    } else {
        &key.text
    };
    let shown = layout(&cr, shown);
    let (_, height) = shown.pixel_size();
    let inset = inset(height);
    let width = key.width - inset * 2;
    if width <= 0 || height <= 0 {
        bail!("the field has no room for text");
    }

    // Where the caret and the selection's ends are, in pixels from the start of the text.
    let x_of = |index: usize| {
        let (strong, _) = layout(&cr, &key.text).cursor_pos(index as i32);
        strong.x() / pango::SCALE
    };
    let caret_x = x_of(key.caret);
    let caret_width = to_physical_precise_round::<i32>(key.scale, 1.).max(1);
    let scroll = scroll(caret_x, caret_width, width);

    let surface = ImageSurface::create(cairo::Format::ARgb32, width, height)?;
    let cr = cairo::Context::new(&surface)?;
    let [r, g, b, a] = key.color.to_array_unpremul().map(f64::from);

    let (start, end) = (key.caret.min(key.anchor), key.caret.max(key.anchor));
    if start != end {
        let (from, to) = (x_of(start), x_of(end));
        cr.set_source_rgba(r, g, b, a * SELECTION_ALPHA);
        cr.rectangle(
            f64::from(from - scroll),
            0.,
            f64::from(to - from),
            f64::from(height),
        );
        cr.fill()?;
    }

    let text_alpha = if showing_placeholder {
        PLACEHOLDER_ALPHA
    } else {
        1.
    };
    cr.set_source_rgba(r, g, b, a * text_alpha);
    cr.move_to(-f64::from(scroll), 0.);
    pangocairo::functions::show_layout(&cr, &shown);

    cr.set_source_rgba(r, g, b, a);
    cr.rectangle(
        f64::from(caret_x - scroll),
        0.,
        f64::from(caret_width),
        f64::from(height),
    );
    cr.fill()?;
    drop(cr);

    let data = surface
        .take_data()
        .map_err(|err| anyhow!("error reading the field's surface: {err:?}"))?;
    let buffer = MemoryBuffer::new(
        data.to_vec(),
        Fourcc::Argb8888,
        (width, height),
        key.scale,
        smithay::utils::Transform::Normal,
    );

    Ok(Rasterized { buffer, inset })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_scrolls_only_as_far_as_the_caret_needs() {
        // The caret shows without scrolling while there's room for it.
        assert_eq!(scroll(0, 2, 100), 0);
        assert_eq!(scroll(97, 2, 100), 0);
        assert_eq!(scroll(98, 2, 100), 0);
        // Past that, the caret sits at the right edge.
        assert_eq!(scroll(99, 2, 100), 1);
        assert_eq!(scroll(250, 2, 100), 152);
    }

    #[test]
    fn text_starts_half_a_line_from_the_edge() {
        assert_eq!(inset(20), 10);
        assert_eq!(inset(1), 0);
    }
}
