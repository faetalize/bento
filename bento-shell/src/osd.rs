//! On-screen indicators, like the volume: a piece of glass at the bottom middle of the screen
//! that grows out of nothing, stays a moment, and shrinks back into nothing.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use gtk::gdk;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::stage::{natural_size, Face, IslandId, Place, Rect, Stage};
use crate::wayland::Style;
use crate::Shell;

const RADIUS: f32 = 24.;
/// How close other glass comes for it to melt into it.
const MELT: f32 = 24.;
/// How long it stays after the last change.
const TIMEOUT: Duration = Duration::from_millis(1500);
/// How far above the bottom of the screen it sits, past bento's gaps.
const LIFT: i32 = 48;

/// Its faces.
const NOTHING: usize = 0;
const SHOWN: usize = 1;

pub struct Osd {
    shell: Weak<Shell>,
    window: gtk::Window,
    stage: Stage,
    island: IslandId,
    icon: gtk::Image,
    level: gtk::LevelBar,
    timeout: RefCell<Option<glib::SourceId>>,
}

impl Osd {
    pub fn new(shell: &Rc<Shell>, monitor: &gdk::Monitor) -> Rc<Self> {
        let window = gtk::Window::builder()
            .application(&shell.app)
            .css_classes(["bento-shell"])
            .build();
        window.init_layer_shell();
        window.set_namespace(Some("bento-shell-osd"));
        window.set_layer(Layer::Overlay);
        window.set_monitor(Some(monitor));
        // A strip along the bottom, letting the pointer through everywhere but the glass.
        window.set_anchor(Edge::Bottom, true);
        window.set_anchor(Edge::Left, true);
        window.set_anchor(Edge::Right, true);
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_default_size(1, 1);

        let icon = gtk::Image::builder().pixel_size(20).build();
        let level = gtk::LevelBar::builder()
            .min_value(0.)
            .max_value(1.)
            .width_request(160)
            .valign(gtk::Align::Center)
            .build();
        let content = gtk::Box::builder().spacing(12).css_classes(["osd"]).build();
        content.append(&icon);
        content.append(&level);

        let stage = Stage::default();
        window.set_child(Some(&stage));
        let place = centered(&content);
        let island = stage.add_island(
            vec![
                Face::nothing(place.clone()),
                Face::new(&content, RADIUS, place).align(0.5, 0.5),
            ],
            NOTHING,
            MELT,
        );

        let this = Rc::new(Self {
            shell: Rc::downgrade(shell),
            window,
            stage,
            island,
            icon,
            level,
            timeout: RefCell::default(),
        });

        let osd = Rc::downgrade(&this);
        this.window.connect_map(move |window| {
            let Some(osd) = osd.upgrade() else { return };
            let (Some(shell), Some(surface)) = (osd.shell.upgrade(), window.surface()) else {
                return;
            };
            osd.stage
                .set_glass(shell.bento.as_ref().and_then(|b| b.glass(&surface)));
        });
        let osd = Rc::downgrade(&this);
        this.window.connect_unmap(move |_| {
            if let Some(osd) = osd.upgrade() {
                osd.stage.set_glass(None);
            }
        });

        this.set_style(shell.bento_style());
        this
    }

    pub fn set_style(&self, style: Style) {
        self.window
            .set_margin(Edge::Bottom, style.gaps.round() as i32 + LIFT);
    }

    /// Shows `icon` with `level`, from 0 to 1, and hides again a moment after the last change.
    pub fn show(self: &Rc<Self>, icon: &str, level: f64) {
        self.icon.set_icon_name(Some(icon));
        self.level.set_value(level);
        if !self.window.is_visible() {
            self.window.present();
        }
        self.stage.show(self.island, SHOWN);

        if let Some(id) = self.timeout.take() {
            id.remove();
        }
        let osd = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(TIMEOUT, move || {
            let Some(osd) = osd.upgrade() else { return };
            osd.timeout.take();
            let window = osd.window.downgrade();
            osd.stage.show_then(
                osd.island,
                NOTHING,
                Some(move || {
                    if let Some(window) = window.upgrade() {
                        window.set_visible(false);
                    }
                }),
            );
        });
        self.timeout.replace(Some(id));
    }

    pub fn destroy(&self) {
        if let Some(id) = self.timeout.take() {
            id.remove();
        }
        self.window.destroy();
    }
}

/// In the middle of the strip, as large as the content.
fn centered(content: &impl IsA<gtk::Widget>) -> Place {
    let content = content.clone().upcast::<gtk::Widget>();
    Rc::new(move |width| {
        let (w, h) = natural_size(&content);
        Rect::new((width - w) / 2., 0., w, h)
    })
}
