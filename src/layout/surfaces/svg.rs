//! Draws SVG icons and images to pixels.

use anyhow::{anyhow, Context as _};
use resvg::{tiny_skia, usvg};

/// An image drawn to pixels.
pub struct Raster {
    /// Premultiplied RGBA, row by row.
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Draws an SVG so that its longer side takes `size` pixels, keeping its proportions, and neither
/// side takes more than `max_size`.
pub fn rasterize(svg: &[u8], size: u32, max_size: u32) -> anyhow::Result<Raster> {
    let mut options = usvg::Options::default();
    // An engine names the SVG it wants drawn, not the files that SVG pulls in.
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let tree = usvg::Tree::from_data(svg, &options).context("error parsing the SVG")?;

    let natural = tree.size();
    let (natural_w, natural_h) = (natural.width(), natural.height());
    let fit = size.clamp(1, max_size) as f32 / natural_w.max(natural_h);
    let side = |natural: f32| ((natural * fit).round() as u32).clamp(1, max_size);
    let (width, height) = (side(natural_w), side(natural_h));

    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| anyhow!("can't draw an SVG {width} × {height} large"))?;
    let transform =
        tiny_skia::Transform::from_scale(width as f32 / natural_w, height as f32 / natural_h);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    Ok(Raster {
        data: pixmap.take(),
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED_SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="24" viewBox="0 0 48 24">
        <rect width="24" height="24" fill="#ff0000"/>
    </svg>"##;

    fn pixel(raster: &Raster, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * raster.width + x) * 4) as usize;
        raster.data[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn the_longer_side_takes_the_size_and_the_proportions_stay() {
        let raster = rasterize(RED_SQUARE.as_bytes(), 96, 4096).unwrap();
        assert_eq!((raster.width, raster.height), (96, 48));
        assert_eq!(raster.data.len(), 96 * 48 * 4);
    }

    #[test]
    fn what_the_svg_paints_is_drawn_and_the_rest_is_transparent() {
        let raster = rasterize(RED_SQUARE.as_bytes(), 48, 4096).unwrap();
        assert_eq!(pixel(&raster, 10, 10), [255, 0, 0, 255]);
        assert_eq!(pixel(&raster, 40, 10), [0, 0, 0, 0]);
    }

    #[test]
    fn pixels_are_premultiplied() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">
            <rect width="8" height="8" fill="#ff0000" fill-opacity="0.5"/>
        </svg>"##;
        let raster = rasterize(svg.as_bytes(), 8, 4096).unwrap();
        let [r, g, b, a] = pixel(&raster, 4, 4);
        assert!((127..=129).contains(&a));
        assert!(r.abs_diff(a) <= 1 && g == 0 && b == 0);
    }

    #[test]
    fn the_size_stays_within_the_limit() {
        let raster = rasterize(RED_SQUARE.as_bytes(), 100_000, 256).unwrap();
        assert_eq!((raster.width, raster.height), (256, 128));

        // An empty size still draws something.
        let raster = rasterize(RED_SQUARE.as_bytes(), 0, 256).unwrap();
        assert_eq!((raster.width, raster.height), (1, 1));
    }

    #[test]
    fn a_file_that_isnt_an_svg_is_an_error() {
        assert!(rasterize(b"not an svg", 16, 4096).is_err());
    }
}
