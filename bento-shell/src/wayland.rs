//! `bento_shell_v1` on GDK's own Wayland connection: bento's commands for the shell, and glass
//! behind the shell's surfaces.
//!
//! The global is only there when bento started the shell, so without it the shell runs without
//! glass and without bento's commands.

use std::cell::{Cell, RefCell};
use std::os::fd::AsRawFd as _;
use std::rc::Rc;

use gdk_wayland::prelude::*;
use gtk::gdk;
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

/// A rounded rectangle of glass, in the surface's logical coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub radius: i32,
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
        })
    }
}

/// Glass behind one surface.
pub struct Glass {
    glass: BentoGlassV1,
    shapes: RefCell<Vec<Shape>>,
}

impl Glass {
    /// Sets the shapes, for the surface's next commit.
    pub fn set_shapes(&self, shapes: &[Shape]) {
        if *self.shapes.borrow() == shapes {
            return;
        }
        *self.shapes.borrow_mut() = shapes.to_vec();

        let bytes = shapes
            .iter()
            .flat_map(|s| [s.x, s.y, s.width.max(0), s.height.max(0), s.radius.max(0)])
            .flat_map(i32::to_ne_bytes)
            .collect();
        self.glass.set_shapes(bytes);
    }
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
            name, interface, ..
        } = event
        {
            if interface == BentoShellV1::interface().name {
                state.shell = Some(registry.bind(name, 1, handle, ()));
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
