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

const VERSION: u32 = 1;

/// The most shapes a surface's glass has.
pub const MAX_GLASS_SHAPES: usize = 8;

/// One shape of a surface's glass: a rounded rectangle in surface-local logical coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassShape {
    pub rect: Rectangle<i32, Logical>,
    pub radius: i32,
}

/// A surface's glass shapes, double-buffered with its commits. Empty means no glass.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct GlassCachedState {
    pub shapes: Vec<GlassShape>,
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
        }
    }

    fn destroyed(&self, _state: &mut D, _client: ClientId, _resource: &BentoGlassV1) {
        if !self.surface.is_alive() {
            return;
        }

        with_states(&self.surface, |states| {
            states
                .cached_state
                .get::<GlassCachedState>()
                .pending()
                .shapes
                .clear();
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

/// A surface's current glass shapes.
pub fn glass_shapes(surface: &WlSurface) -> Vec<GlassShape> {
    with_states(surface, |states| {
        states
            .cached_state
            .get::<GlassCachedState>()
            .current()
            .shapes
            .clone()
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
}
