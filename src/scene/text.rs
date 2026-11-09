//! Text for the scene: shaped and laid out by parley, as positioned glyphs for the renderer.

use std::borrow::Cow;

use parley::{
    Alignment, AlignmentOptions, FontContext, FontStack, FontWeight, LayoutContext,
    PositionedLayoutItem, StyleProperty,
};

/// How a piece of text is set.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub weight: f32,
    pub family: String,
    /// Left, centre or right, as 0, 0.5 or 1.
    pub align: f32,
    /// Figures all as wide, for clocks and numbers that change.
    pub tabular: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            size: 14.,
            weight: 400.,
            family: String::from("sans-serif"),
            align: 0.,
            tabular: false,
        }
    }
}

/// A run of glyphs in one font, at positions relative to the text's top left.
#[derive(Debug, Clone)]
pub struct GlyphRun {
    pub font: parley::FontData,
    pub size: f32,
    pub coords: Vec<i16>,
    pub glyphs: Vec<(u32, f32, f32)>,
}

/// Text laid out at a width.
#[derive(Debug, Clone, Default)]
pub struct Laid {
    pub width: f32,
    pub height: f32,
    pub runs: Vec<GlyphRun>,
}

pub struct Fonts {
    fonts: FontContext,
    layouts: LayoutContext<()>,
}

impl std::fmt::Debug for Fonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Fonts")
    }
}

impl Default for Fonts {
    fn default() -> Self {
        Self {
            fonts: FontContext::new(),
            layouts: LayoutContext::new(),
        }
    }
}

impl Fonts {
    /// Lays `text` out, wrapping at `max_width` if it's given.
    pub fn lay_out(&mut self, text: &str, style: &TextStyle, max_width: Option<f32>) -> Laid {
        let mut builder = self.layouts.ranged_builder(&mut self.fonts, text, 1., true);
        builder.push_default(StyleProperty::FontSize(style.size));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(style.weight)));
        builder.push_default(StyleProperty::FontStack(FontStack::Source(Cow::Owned(
            style.family.clone(),
        ))));
        if style.tabular {
            builder.push_default(StyleProperty::FontFeatures(parley::FontSettings::Source(
                Cow::Borrowed("\"tnum\""),
            )));
        }
        let mut layout = builder.build(text);
        layout.break_all_lines(max_width);
        let alignment = if style.align >= 0.75 {
            Alignment::Right
        } else if style.align >= 0.25 {
            Alignment::Center
        } else {
            Alignment::Left
        };
        layout.align(max_width, alignment, AlignmentOptions::default());

        let mut runs = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let font = run.run().font().clone();
                let size = run.run().font_size();
                let coords = run.run().normalized_coords().to_vec();
                let glyphs = run
                    .positioned_glyphs()
                    .map(|glyph| (glyph.id, glyph.x, glyph.y))
                    .collect();
                runs.push(GlyphRun {
                    font,
                    size,
                    coords,
                    glyphs,
                });
            }
        }
        Laid {
            width: layout.width(),
            height: layout.height(),
            runs,
        }
    }
}
