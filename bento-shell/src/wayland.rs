//! `bento_shell_v1` on GDK's own Wayland connection: bento's commands for the shell, and glass
//! behind the shell's surfaces.
//!
//! The global is only there when bento started the shell, so without it the shell runs without
//! glass and without bento's commands.

use std::cell::{Cell, RefCell};
use std::os::fd::AsRawFd as _;
use std::rc::Rc;

use gdk_wayland::prelude::*;
use gtk::{gdk, gsk};
use wayland_client::protocol::wl_registry;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};

use self::protocol::bento_glass_v1::BentoGlassV1;
use self::protocol::bento_shell_v1::{self, BentoShellV1};

#[allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
#[allow(non_upper_case_globals, non_snake_case, unused_imports)]
#[allow(missing_docs, clippy::all)]
mod protocol {
    use wayland_client;
    use wayland_client::protocol::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("../resources/bento-shell-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("../resources/bento-shell-v1.xml");
}

/// How bento lays out windows, for the shell to line up with them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub gaps: f64,
    pub corner_radius: f64,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            gaps: 8.,
            corner_radius: 12.,
        }
    }
}

/// A rounded rectangle of glass, in the surface's logical coordinates, to fractions of a pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    /// How close other glass comes for it to melt into it, with the wider join a wider melt
    /// gives; it melts into the shapes before it.
    pub melt: f32,
}

enum Event {
    Style(Style),
    ToggleLauncher,
}

#[derive(Default)]
struct State {
    shell: Option<BentoShellV1>,
    events: Vec<Event>,
}

type Listeners<T> = RefCell<Vec<Box<dyn Fn(T)>>>;

pub struct Bento {
    queue: RefCell<EventQueue<State>>,
    handle: QueueHandle<State>,
    state: RefCell<State>,
    style: Cell<Style>,
    on_style: Listeners<Style>,
    on_toggle_launcher: Listeners<()>,
}

impl Bento {
    pub fn connect(display: &gdk::Display) -> Option<Rc<Self>> {
        let display = display.downcast_ref::<gdk_wayland::WaylandDisplay>()?;
        let wl_display = display.wl_display()?;
        let connection = Connection::from_backend(wl_display.backend().upgrade()?);

        let mut queue = connection.new_event_queue();
        let handle = queue.handle();
        wl_display.get_registry(&handle, ());

        let mut state = State::default();
        if let Err(err) = queue.roundtrip(&mut state) {
            eprintln!("couldn't read bento's globals: {err}");
        }
        if state.shell.is_none() {
            eprintln!("bento didn't start this shell, so it has no glass or commands");
        }

        let bento = Rc::new(Self {
            queue: RefCell::new(queue),
            handle,
            state: RefCell::new(state),
            style: Cell::new(Style::default()),
            on_style: RefCell::default(),
            on_toggle_launcher: RefCell::default(),
        });
        bento.deliver();

        // GDK reads the socket too. Both watches wake on the same poll, so whichever reads it,
        // ours then dispatches what landed on our queue.
        let fd = connection.backend().poll_fd().as_raw_fd();
        let weak = Rc::downgrade(&bento);
        glib::source::unix_fd_add_local(fd, glib::IOCondition::IN, move |_, _| {
            let Some(bento) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            bento.dispatch();
            glib::ControlFlow::Continue
        });

        Some(bento)
    }

    fn dispatch(&self) {
        {
            let mut queue = self.queue.borrow_mut();
            let mut state = self.state.borrow_mut();
            if let Some(guard) = queue.prepare_read() {
                // Nothing to read means GDK read it first.
                let _ = guard.read();
            }
            if let Err(err) = queue.dispatch_pending(&mut state) {
                eprintln!("error reading bento's events: {err}");
            }
        }
        self.deliver();
    }

    /// Runs the callbacks for what bento sent, with nothing borrowed so they can call back in.
    fn deliver(&self) {
        let events = std::mem::take(&mut self.state.borrow_mut().events);
        for event in events {
            match event {
                Event::Style(style) => {
                    self.style.set(style);
                    for f in self.on_style.borrow().iter() {
                        f(style);
                    }
                }
                Event::ToggleLauncher => {
                    for f in self.on_toggle_launcher.borrow().iter() {
                        f(());
                    }
                }
            }
        }
    }

    pub fn style(&self) -> Style {
        self.style.get()
    }

    pub fn connect_style(&self, f: impl Fn(Style) + 'static) {
        self.on_style.borrow_mut().push(Box::new(f));
    }

    pub fn connect_toggle_launcher(&self, f: impl Fn() + 'static) {
        self.on_toggle_launcher
            .borrow_mut()
            .push(Box::new(move |()| f()));
    }

    /// The glass behind a mapped surface. A surface gets a new one each time it's mapped.
    pub fn glass(&self, surface: &gdk::Surface) -> Option<Glass> {
        let shell = self.state.borrow().shell.clone()?;
        let surface = surface.downcast_ref::<gdk_wayland::WaylandSurface>()?;
        let wl_surface: WlSurface = surface.wl_surface()?;
        let glass = shell.get_glass(&wl_surface, &self.handle, ());
        Some(Glass {
            glass,
            shapes: RefCell::default(),
            curves: RefCell::default(),
        })
    }
}

/// Glass behind one surface.
pub struct Glass {
    glass: BentoGlassV1,
    shapes: RefCell<Vec<Shape>>,
    curves: RefCell<Vec<[f32; 7]>>,
}

/// The most curves bento takes for a surface's paths, and the most paths.
const MAX_CURVES: usize = 64;
const MAX_PATHS: usize = 8;

impl Glass {
    /// Sets the glass's rounded rectangles and paths of any shape, for the surface's next
    /// commit. They melt together where they come close. A bento that doesn't take paths gets
    /// only the rectangles.
    pub fn set_shapes(&self, shapes: &[Shape], paths: &[gsk::Path]) {
        if *self.shapes.borrow() != shapes {
            *self.shapes.borrow_mut() = shapes.to_vec();
            if self.glass.version() >= 3 {
                let bytes = shapes
                    .iter()
                    .flat_map(|s| {
                        [s.x, s.y, s.width, s.height, s.radius, s.melt].map(|x| x.max(0.))
                    })
                    .flat_map(f32::to_ne_bytes)
                    .collect();
                self.glass.set_rects(bytes);
            } else {
                // Whole pixels, melting at bento's own distance.
                let bytes = shapes
                    .iter()
                    .flat_map(|s| {
                        [
                            s.x,
                            s.y,
                            s.width.max(0.),
                            s.height.max(0.),
                            s.radius.max(0.),
                        ]
                        .map(|x| x.round() as i32)
                    })
                    .flat_map(i32::to_ne_bytes)
                    .collect();
                self.glass.set_shapes(bytes);
            }
        }

        if self.glass.version() < 2 {
            return;
        }
        let curves = path_curves(paths);
        if *self.curves.borrow() != curves {
            let bytes = curves
                .iter()
                .flatten()
                .flat_map(|x| x.to_ne_bytes())
                .collect();
            *self.curves.borrow_mut() = curves;
            self.glass.set_paths(bytes);
        }
    }
}

/// The paths as bento takes them: each curve a quadratic one, its start, control point and end,
/// and the number of its path. GTK turns cubic curves and arcs into quadratic ones, and a line
/// is a curve with its control point in its middle. Each path is closed. Paths past bento's
/// limits are left out whole.
fn path_curves(paths: &[gsk::Path]) -> Vec<[f32; 7]> {
    let mut curves: Vec<[f32; 7]> = Vec::new();
    for path in paths.iter().take(MAX_PATHS) {
        let number = curves.last().map_or(0., |last| last[6] + 1.);
        let mut own: Vec<[f32; 7]> = Vec::new();
        let line = |own: &mut Vec<[f32; 7]>, a: [f32; 2], b: [f32; 2]| {
            if a != b {
                let mid = [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.];
                own.push([a[0], a[1], mid[0], mid[1], b[0], b[1], number]);
            }
        };
        let mut start = [0f32; 2];
        let mut at = [0f32; 2];
        path.foreach(gsk::PathForeachFlags::QUAD, |op, first, n, _| {
            // SAFETY: GTK hands `n` points in a row, and `graphene::Point` is laid out as
            // graphene's point.
            let points =
                unsafe { std::slice::from_raw_parts(first as *const gtk::graphene::Point, n) };
            let point = |i: usize| [points[i].x(), points[i].y()];
            match op {
                gsk::PathOperation::Move => {
                    line(&mut own, at, start);
                    start = point(0);
                    at = start;
                }
                gsk::PathOperation::Line => {
                    line(&mut own, point(0), point(1));
                    at = point(1);
                }
                gsk::PathOperation::Quad => {
                    let [a, b, c] = [point(0), point(1), point(2)];
                    own.push([a[0], a[1], b[0], b[1], c[0], c[1], number]);
                    at = c;
                }
                gsk::PathOperation::Close => {
                    line(&mut own, at, start);
                    at = start;
                }
                _ => (),
            }
            gtk::glib::ControlFlow::Continue
        });
        line(&mut own, at, start);

        if curves.len() + own.len() > MAX_CURVES {
            break;
        }
        curves.extend(own);
    }
    curves
}

impl Drop for Glass {
    fn drop(&mut self) {
        if self.glass.is_alive() {
            self.glass.destroy();
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == BentoShellV1::interface().name {
                // Version 2 takes glass's paths, and 3 its shapes to fractions of a pixel with
                // their melts.
                state.shell = Some(registry.bind(name, version.min(3), handle, ()));
            }
        }
    }
}

impl Dispatch<BentoShellV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &BentoShellV1,
        event: bento_shell_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            bento_shell_v1::Event::Style {
                gaps,
                corner_radius,
            } => state.events.push(Event::Style(Style {
                gaps,
                corner_radius,
            })),
            bento_shell_v1::Event::ToggleLauncher => state.events.push(Event::ToggleLauncher),
        }
    }
}

impl Dispatch<BentoGlassV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &BentoGlassV1,
        event: <BentoGlassV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {}
    }
}
