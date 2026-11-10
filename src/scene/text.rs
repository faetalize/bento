//! Text for the scene: shaped and laid out by parley, as positioned glyphs for the renderer.

use std::borrow::Cow;

use parley::{
    Affinity, Alignment, AlignmentOptions, BoundingBox, Cursor, FontContext, FontStack, FontWeight,
    Layout, LayoutContext, PositionedLayoutItem, Selection, StyleProperty,
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
    /// On one line, cut short with an ellipsis where it doesn't fit, rather than wrapped.
    pub one_line: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            size: 14.,
            weight: 400.,
            family: String::from("sans-serif"),
            align: 0.,
            tabular: false,
            one_line: false,
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
    /// `text` laid out by parley, wrapping at `max_width` if it's given.
    fn build(&mut self, text: &str, style: &TextStyle, max_width: Option<f32>) -> Layout<()> {
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
        layout
    }

    /// Where the caret is at byte `caret` of `text`, and the selection from there to `anchor`,
    /// relative to the text's top left: x, y, width and height each.
    pub fn caret(
        &mut self,
        text: &str,
        style: &TextStyle,
        max_width: Option<f32>,
        caret: usize,
        anchor: usize,
    ) -> ([f32; 4], Vec<[f32; 4]>) {
        let line = style.size * 1.25;
        if text.is_empty() {
            return ([0., 0., 1., line], Vec::new());
        }
        let layout = self.build(text, style, max_width);
        let rect = |b: BoundingBox| {
            [
                b.x0 as f32,
                b.y0 as f32,
                (b.x1 - b.x0) as f32,
                (b.y1 - b.y0) as f32,
            ]
        };
        let at = |index| Cursor::from_byte_index(&layout, index, Affinity::Downstream);
        let caret_rect = rect(at(caret).geometry(&layout, 1.));
        let selection = Selection::new(at(anchor), at(caret))
            .geometry(&layout)
            .into_iter()
            .map(|(b, _)| rect(b))
            .collect();
        (caret_rect, selection)
    }

    /// The byte of `text` nearest the point `x`, `y` relative to its top left.
    pub fn index_at(
        &mut self,
        text: &str,
        style: &TextStyle,
        max_width: Option<f32>,
        x: f32,
        y: f32,
    ) -> usize {
        if text.is_empty() {
            return 0;
        }
        let layout = self.build(text, style, max_width);
        Cursor::from_point(&layout, x, y).index()
    }

    /// Lays `text` out, wrapping at `max_width` if it's given, or on one line cut short with an
    /// ellipsis to fit in it if the style says so.
    pub fn lay_out(&mut self, text: &str, style: &TextStyle, max_width: Option<f32>) -> Laid {
        if !style.one_line {
            return self.lay_out_wrapped(text, style, max_width);
        }
        let whole = self.lay_out_wrapped(text, style, None);
        let Some(room) = max_width.filter(|room| whole.width > *room) else {
            return whole;
        };
        // The most of it that fits with the ellipsis, by its characters.
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let cut = |fonts: &mut Self, keep: usize| {
            let end = chars.get(keep).map_or(text.len(), |(at, _)| *at);
            let shown = format!("{}…", text[..end].trim_end());
            fonts.lay_out_wrapped(&shown, style, None)
        };
        let (mut low, mut high) = (0, chars.len());
        while low < high {
            let middle = (low + high + 1) / 2;
            if cut(self, middle).width <= room {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        cut(self, low)
    }

    fn lay_out_wrapped(&mut self, text: &str, style: &TextStyle, max_width: Option<f32>) -> Laid {
        let layout = self.build(text, style, max_width);
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
