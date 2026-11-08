//! `bento_shell_v1`: the shell's private protocol, on the shell's connection only.

use std::sync::Mutex;

use bento_glass_v1::BentoGlassV1;
use bento_shell_v1::BentoShellV1;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::compositor::{with_states, Cacheable};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use super::raw::bento_shell::v1::server::{bento_glass_v1, bento_shell_v1};

const VERSION: u32 = 2;

/// The most shapes a surface's glass has.
pub const MAX_GLASS_SHAPES: usize = 8;
/// The most paths a surface's glass has, and the most curves they have in all.
pub const MAX_GLASS_PATHS: usize = 8;
pub const MAX_GLASS_CURVES: usize = 64;

/// One shape of a surface's glass: a rounded rectangle in surface-local logical coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassShape {
    pub rect: Rectangle<i32, Logical>,
    pub radius: i32,
}

/// One curve of a path of a surface's glass: a quadratic Bézier curve from `points[0]` to
/// `points[2]` with `points[1]` its control point, in surface-local logical coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassCurve {
    pub points: [[f32; 2]; 3],
    /// The number of the path it's in.
    pub path: usize,
}

/// A surface's glass shapes and paths, double-buffered with its commits. Neither means no
/// glass.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct GlassCachedState {
    pub shapes: Vec<GlassShape>,
    pub paths: Vec<GlassCurve>,
}

impl Cacheable for GlassCachedState {
    fn commit(&mut self, _dh: &DisplayHandle) -> Self {
        self.clone()
    }

    fn merge_into(self, into: &mut Self, _dh: &DisplayHandle) {
        *into = self;
    }
}

/// Marks a surface that has a glass object.
#[derive(Default)]
struct HasGlass(Mutex<bool>);

pub struct BentoShellState {
    shells: Vec<BentoShellV1>,
}

pub struct BentoShellGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub struct ShellData;

pub struct GlassData {
    surface: WlSurface,
}

pub trait BentoShellHandler {
    fn bento_shell_state(&mut self) -> &mut BentoShellState;
    /// The gaps between windows and their corner radius.
    fn shell_style(&self) -> (f64, f64);
}

impl BentoShellState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<BentoShellV1, BentoShellGlobalData>,
        D: Dispatch<BentoShellV1, ShellData>,
        D: BentoShellHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = BentoShellGlobalData {
            filter: Box::new(filter),
        };
        display.create_global::<D, BentoShellV1, _>(VERSION, global_data);

        Self { shells: Vec::new() }
    }

    pub fn send_style(&self, gaps: f64, corner_radius: f64) {
        for shell in &self.shells {
            shell.style(gaps, corner_radius);
        }
    }

    pub fn toggle_launcher(&self) {
        for shell in &self.shells {
            shell.toggle_launcher();
        }
    }
}

impl<D> GlobalDispatch2<BentoShellV1, D> for BentoShellGlobalData
where
    D: Dispatch<BentoShellV1, ShellData>,
    D: BentoShellHandler,
{
    fn bind(
        &self,
        state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<BentoShellV1>,
        data_init: &mut DataInit<'_, D>,
    ) {
        let shell = data_init.init(resource, ShellData);
        let (gaps, corner_radius) = state.shell_style();
        shell.style(gaps, corner_radius);
        state.bento_shell_state().shells.push(shell);
    }

    fn can_view(&self, client: &Client) -> bool {
        (self.filter)(client)
    }
}

impl<D> Dispatch2<BentoShellV1, D> for ShellData
where
    D: Dispatch<BentoGlassV1, GlassData>,
    D: BentoShellHandler,
{
    fn request(
        &self,
        _state: &mut D,
        _client: &Client,
        resource: &BentoShellV1,
        request: <BentoShellV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            bento_shell_v1::Request::Destroy => (),
            bento_shell_v1::Request::GetGlass { id, surface } => {
                let taken = with_states(&surface, |states| {
                    let has = states.data_map.get_or_insert_threadsafe(HasGlass::default);
                    let mut has = has.0.lock().unwrap();
                    std::mem::replace(&mut *has, true)
                });
                if taken {
                    resource.post_error(
                        bento_shell_v1::Error::GlassExists,
                        "the surface already has a glass object",
                    );
                    return;
                }

                data_init.init(id, GlassData { surface });
            }
        }
    }

    fn destroyed(&self, state: &mut D, _client: ClientId, resource: &BentoShellV1) {
        state
            .bento_shell_state()
            .shells
            .retain(|shell| shell != resource);
    }
}

impl<D> Dispatch2<BentoGlassV1, D> for GlassData
where
    D: BentoShellHandler,
{
    fn request(
        &self,
        _state: &mut D,
        _client: &Client,
        resource: &BentoGlassV1,
        request: <BentoGlassV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            bento_glass_v1::Request::Destroy => (),
            bento_glass_v1::Request::SetShapes { shapes } => {
                let Some(shapes) = parse_shapes(&shapes) else {
                    resource.post_error(
                        bento_glass_v1::Error::InvalidShapes,
                        "shapes must be at most 8 whole quintuples with no negative sizes",
                    );
                    return;
                };

                with_states(&self.surface, |states| {
                    states
                        .cached_state
                        .get::<GlassCachedState>()
                        .pending()
                        .shapes = shapes;
                });
            }
            bento_glass_v1::Request::SetPaths { paths } => {
                let Some(paths) = parse_paths(&paths) else {
                    resource.post_error(
                        bento_glass_v1::Error::InvalidPaths,
                        "paths must be at most 64 whole septuples of finite numbers, \
                         in at most 8 paths numbered from 0 up in order",
                    );
                    return;
                };

                with_states(&self.surface, |states| {
                    states
                        .cached_state
                        .get::<GlassCachedState>()
                        .pending()
                        .paths = paths;
                });
            }
        }
    }

    fn destroyed(&self, _state: &mut D, _client: ClientId, _resource: &BentoGlassV1) {
        if !self.surface.is_alive() {
            return;
        }

        with_states(&self.surface, |states| {
            let mut cached = states.cached_state.get::<GlassCachedState>();
            let pending = cached.pending();
            pending.shapes.clear();
            pending.paths.clear();
            drop(cached);
            if let Some(has) = states.data_map.get::<HasGlass>() {
                *has.0.lock().unwrap() = false;
            }
        });
    }
}

/// Reads `set_shapes`' array: native-endian int32 quintuples.
fn parse_shapes(bytes: &[u8]) -> Option<Vec<GlassShape>> {
    const QUINTUPLE: usize = 5 * 4;
    if bytes.len() % QUINTUPLE != 0 || bytes.len() / QUINTUPLE > MAX_GLASS_SHAPES {
        return None;
    }

    bytes
        .chunks_exact(QUINTUPLE)
        .map(|chunk| {
            let mut ints = chunk
                .chunks_exact(4)
                .map(|int| i32::from_ne_bytes(int.try_into().unwrap()));
            let mut next = || ints.next().unwrap();
            let (x, y, w, h, radius) = (next(), next(), next(), next(), next());
            if w < 0 || h < 0 || radius < 0 {
                return None;
            }
            Some(GlassShape {
                rect: Rectangle::new((x, y).into(), (w, h).into()),
                radius,
            })
        })
        .collect()
}

/// Reads `set_paths`' array: native-endian float32 septuples.
fn parse_paths(bytes: &[u8]) -> Option<Vec<GlassCurve>> {
    const SEPTUPLE: usize = 7 * 4;
    if bytes.len() % SEPTUPLE != 0 || bytes.len() / SEPTUPLE > MAX_GLASS_CURVES {
        return None;
    }

    let mut last_path = None;
    bytes
        .chunks_exact(SEPTUPLE)
        .map(|chunk| {
            let floats: Vec<f32> = chunk
                .chunks_exact(4)
                .map(|float| f32::from_ne_bytes(float.try_into().unwrap()))
                .collect();
            if !floats.iter().all(|x| x.is_finite()) {
                return None;
            }
            // Numbered from 0 up: each curve is in the path before's or the next one.
            let path = floats[6];
            let in_order = match last_path {
                None => path == 0.,
                Some(last) => path == last as f32 || path == last as f32 + 1.,
            };
            if !in_order {
                return None;
            }
            let path = path as usize;
            if path >= MAX_GLASS_PATHS {
                return None;
            }
            last_path = Some(path);
            Some(GlassCurve {
                points: [
                    [floats[0], floats[1]],
                    [floats[2], floats[3]],
                    [floats[4], floats[5]],
                ],
                path,
            })
        })
        .collect()
}

/// A surface's current glass shapes and paths.
pub fn glass_shapes(surface: &WlSurface) -> (Vec<GlassShape>, Vec<GlassCurve>) {
    with_states(surface, |states| {
        let mut cached = states.cached_state.get::<GlassCachedState>();
        let current = cached.current();
        (current.shapes.clone(), current.paths.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(ints: &[i32]) -> Vec<u8> {
        ints.iter().flat_map(|i| i.to_ne_bytes()).collect()
    }

    #[test]
    fn shapes_are_read_as_quintuples() {
        let shapes = parse_shapes(&bytes(&[1, 2, 30, 40, 5, 0, 50, 10, 10, 0])).unwrap();
        assert_eq!(
            shapes,
            [
                GlassShape {
                    rect: Rectangle::new((1, 2).into(), (30, 40).into()),
                    radius: 5,
                },
                GlassShape {
                    rect: Rectangle::new((0, 50).into(), (10, 10).into()),
                    radius: 0,
                },
            ]
        );
    }

    #[test]
    fn broken_shapes_are_refused() {
        assert_eq!(parse_shapes(&bytes(&[1, 2, 3, 4])), None);
        assert_eq!(parse_shapes(&bytes(&[0, 0, -1, 4, 0])), None);
        assert_eq!(parse_shapes(&bytes(&[0, 0, 1, 4, -2])), None);
        assert_eq!(parse_shapes(&bytes(&[0, 0, 1, 1, 0].repeat(9))), None);
        assert_eq!(parse_shapes(&[]), Some(Vec::new()));
    }

    fn float_bytes(floats: &[f32]) -> Vec<u8> {
        floats.iter().flat_map(|f| f.to_ne_bytes()).collect()
    }

    #[test]
    fn paths_are_read_as_septuples() {
        let curves = parse_paths(&float_bytes(&[
            0., 0., 5., 0., 10., 0., 0., //
            10., 0., 5., 5., 0., 0., 0., //
            20., 0., 25., 0., 30., 10., 1., //
        ]))
        .unwrap();
        assert_eq!(curves.len(), 3);
        assert_eq!(curves[1].points, [[10., 0.], [5., 5.], [0., 0.]]);
        assert_eq!(curves.iter().map(|c| c.path).collect::<Vec<_>>(), [0, 0, 1]);
    }

    #[test]
    fn broken_paths_are_refused() {
        let curve = |path: f32| [0., 0., 1., 1., 2., 0., path];
        // Not whole septuples.
        assert_eq!(parse_paths(&float_bytes(&[0.; 6])), None);
        // Not finite.
        assert_eq!(
            parse_paths(&float_bytes(&[0., f32::NAN, 0., 0., 0., 0., 0.])),
            None
        );
        // Not from 0, skipping a number, or going back.
        assert_eq!(parse_paths(&float_bytes(&curve(1.))), None);
        assert_eq!(
            parse_paths(&float_bytes(&[curve(0.), curve(2.)].concat())),
            None
        );
        assert_eq!(
            parse_paths(&float_bytes(&[curve(0.), curve(1.), curve(0.)].concat())),
            None
        );
        // Too many paths or curves.
        let nine: Vec<f32> = (0..9).flat_map(|i| curve(i as f32)).collect();
        assert_eq!(parse_paths(&float_bytes(&nine)), None);
        assert_eq!(parse_paths(&float_bytes(&curve(0.).repeat(65))), None);
        assert_eq!(parse_paths(&[]), Some(Vec::new()));
    }
}
