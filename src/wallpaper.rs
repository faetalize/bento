//! Images drawn behind workspaces, from `layout { background-image }`.
//!
//! A large image takes long enough to decode that it would drop frames, so it decodes on a thread.
//! Workspaces show their background color until it's ready. Wallpapers from layer-shell clients
//! draw above it, so a shell's own wallpaper still wins.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::thread;

use anyhow::Context as _;
use calloop::LoopHandle;
use niri_config::Config;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesTexture;
use smithay::utils::{Buffer, Logical, Point, Rectangle, Size, Transform};

use crate::niri::State;
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};
use crate::utils::expand_home;

/// Longest side an image keeps. Bigger ones are scaled down as they load, to bound memory.
const MAX_SIDE: u32 = 4096;

pub struct Wallpapers {
    /// Keyed by the path as the config writes it.
    images: HashMap<String, Entry>,
    to_main: calloop::channel::Sender<(String, anyhow::Result<Decoded>)>,
}

enum Entry {
    Loading,
    Failed,
    Ready(Image),
}

struct Image {
    pixels: Vec<u8>,
    size: Size<i32, Buffer>,
    opaque: bool,
    /// Uploaded on first use, on the primary GPU.
    buffer: RefCell<Option<TextureBuffer<GlesTexture>>>,
}

pub struct Decoded {
    /// RGBA, 8 bits per channel.
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    opaque: bool,
}

impl Wallpapers {
    pub fn new(event_loop: &LoopHandle<'static, State>) -> Self {
        let (to_main, from_threads) = calloop::channel::channel();
        event_loop
            .insert_source(from_threads, |event, _, state| {
                if let calloop::channel::Event::Msg((path, result)) = event {
                    state.niri.wallpapers.loaded(path, result);
                    state.niri.queue_redraw_all();
                }
            })
            .unwrap();
        Self {
            images: HashMap::new(),
            to_main,
        }
    }

    /// Starts loading the images the config uses, and drops the ones it no longer does.
    pub fn update(&mut self, config: &Config) {
        let wanted = wanted(config);
        self.images.retain(|path, _| wanted.contains(path));
        for path in wanted {
            if self.images.contains_key(&path) {
                continue;
            }
            self.images.insert(path.clone(), Entry::Loading);
            let to_main = self.to_main.clone();
            let res = thread::Builder::new()
                .name(String::from("Wallpaper Loader"))
                .spawn(move || {
                    let result = decode(&path);
                    let _ = to_main.send((path, result));
                });
            if let Err(err) = res {
                warn!("error spawning a thread to load the wallpaper: {err:?}");
            }
        }
    }

    fn loaded(&mut self, path: String, result: anyhow::Result<Decoded>) {
        // The config may have moved on while the image loaded.
        let Some(entry) = self.images.get_mut(&path) else {
            return;
        };
        *entry = match result {
            Ok(decoded) => Entry::Ready(Image {
                size: Size::from((decoded.width as i32, decoded.height as i32)),
                pixels: decoded.pixels,
                opaque: decoded.opaque,
                buffer: RefCell::new(None),
            }),
            Err(err) => {
                warn!("error loading the wallpaper {path:?}: {err:?}");
                Entry::Failed
            }
        };
    }

    /// The image at `path`, scaled to cover a workspace of `size` and cropped to its middle.
    /// `None` until it has loaded, or when it can't.
    pub fn render<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        path: &str,
        size: Size<f64, Logical>,
    ) -> Option<PrimaryGpuTextureRenderElement> {
        let Some(Entry::Ready(image)) = self.images.get(path) else {
            return None;
        };

        let mut buffer = image.buffer.borrow_mut();
        if buffer.is_none() {
            let opaque_regions = if image.opaque {
                vec![Rectangle::from_size(image.size)]
            } else {
                Vec::new()
            };
            let result = TextureBuffer::from_memory(
                renderer.as_gles_renderer(),
                &image.pixels,
                Fourcc::Abgr8888,
                image.size,
                false,
                1.,
                Transform::Normal,
                opaque_regions,
            );
            match result {
                Ok(uploaded) => *buffer = Some(uploaded),
                Err(err) => {
                    warn!("error uploading the wallpaper: {err:?}");
                    return None;
                }
            }
        }
        let buffer = buffer.clone()?;

        let src = cover(image.size.to_f64().to_logical(1., Transform::Normal), size);
        let elem = TextureRenderElement::from_texture_buffer(
            buffer,
            Point::from((0., 0.)),
            1.,
            Some(src),
            Some(size),
            Kind::Unspecified,
        );
        Some(PrimaryGpuTextureRenderElement(elem))
    }
}

/// Paths of every background image the config sets, for all monitors.
fn wanted(config: &Config) -> Vec<String> {
    let mut paths: Vec<String> = config.layout.background_image.iter().cloned().collect();
    for output in &config.outputs.0 {
        if let Some(path) = output
            .layout
            .as_ref()
            .and_then(|l| l.background_image.clone())
        {
            paths.push(path);
        }
    }
    paths.sort_unstable();
    paths.dedup();
    paths
}

/// The part of an image that fills `target` without stretching: scaled to cover it, centered.
fn cover(image: Size<f64, Logical>, target: Size<f64, Logical>) -> Rectangle<f64, Logical> {
    let scale = (target.w / image.w).max(target.h / image.h);
    let size = Size::from((target.w / scale, target.h / scale));
    let loc = Point::from(((image.w - size.w) / 2., (image.h - size.h) / 2.));
    Rectangle::new(loc, size)
}

fn decode(path: &str) -> anyhow::Result<Decoded> {
    let path = expand_home(path.as_ref())?.unwrap_or_else(|| PathBuf::from(path));
    let image = image::ImageReader::open(&path)
        .with_context(|| format!("error opening {path:?}"))?
        .with_guessed_format()?
        .decode()
        .context("error decoding the image")?;

    let image = if image.width().max(image.height()) > MAX_SIDE {
        image.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let image = image.into_rgba8();
    let (width, height) = image.dimensions();
    let pixels = image.into_raw();
    let opaque = pixels.chunks_exact(4).all(|pixel| pixel[3] == 255);
    Ok(Decoded {
        pixels,
        width,
        height,
        opaque,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_by_cropping_the_middle() {
        // A wide image on a narrower screen keeps its full height and loses its sides.
        let src = cover(Size::from((2000., 1000.)), Size::from((1000., 1000.)));
        assert_eq!(
            src,
            Rectangle::new(Point::from((500., 0.)), Size::from((1000., 1000.)))
        );

        // A small image scales up: the crop is in image pixels, smaller than the screen.
        let src = cover(Size::from((800., 600.)), Size::from((1600., 900.)));
        assert_eq!(
            src,
            Rectangle::new(Point::from((0., 75.)), Size::from((800., 450.)))
        );
    }
}
