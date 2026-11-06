//! bento's shell: the bar, the launcher and notifications around the workspaces.

mod bar;
mod launcher;
mod services;
mod stage;
mod wayland;

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio};

use crate::bar::Bar;
use crate::launcher::Launcher;
use crate::services::{
    Audio, AudioService, BatteryService, NetworkService, NotificationEvent, Notifications, Tray,
    Workspaces,
};
use crate::wayland::{Bento, Style};

const APP_ID: &str = "io.github.faetalize.bento.Shell";

pub struct Shell {
    pub app: adw::Application,
    /// bento's private connection, when bento started the shell.
    pub bento: Option<Rc<Bento>>,
    bars: RefCell<Vec<Rc<Bar>>>,
    launcher: RefCell<Option<Rc<Launcher>>>,
    pub workspaces: Rc<Workspaces>,
    pub audio: Rc<AudioService>,
    pub network: Rc<NetworkService>,
    pub battery: Rc<BatteryService>,
    pub notifications: Rc<Notifications>,
    pub tray: Rc<Tray>,
    /// The audio as last shown, to tell a change from the first report.
    last_audio: Cell<Option<Audio>>,
}

impl Shell {
    fn new(app: &adw::Application) -> Rc<Self> {
        let display = gdk::Display::default().expect("the shell needs a display");
        let bento = Bento::connect(&display);

        let shell = Rc::new_cyclic(|weak: &Weak<Self>| {
            let w = weak.clone();
            let workspaces = Workspaces::start(move |workspaces| {
                if let Some(shell) = w.upgrade() {
                    for bar in shell.bars() {
                        bar.set_workspaces(workspaces);
                    }
                }
            });
            let w = weak.clone();
            let audio = AudioService::start(move |audio| {
                if let Some(shell) = w.upgrade() {
                    shell.set_audio(audio);
                }
            });
            let w = weak.clone();
            let network = NetworkService::start(move |network| {
                if let Some(shell) = w.upgrade() {
                    for bar in shell.bars() {
                        bar.set_network(network);
                    }
                }
            });
            let w = weak.clone();
            let battery = BatteryService::start(move |battery| {
                if let Some(shell) = w.upgrade() {
                    for bar in shell.bars() {
                        bar.set_battery(battery);
                    }
                }
            });
            let w = weak.clone();
            let notifications = Notifications::start(move |event| {
                if let Some(shell) = w.upgrade() {
                    shell.notification(event);
                }
            });
            let w = weak.clone();
            let tray = Tray::start(move |items| {
                if let Some(shell) = w.upgrade() {
                    for bar in shell.bars() {
                        bar.set_tray(items);
                    }
                }
            });

            Self {
                app: app.clone(),
                bento,
                bars: RefCell::default(),
                launcher: RefCell::default(),
                workspaces,
                audio,
                network,
                battery,
                notifications,
                tray,
                last_audio: Cell::new(None),
            }
        });

        if let Some(bento) = &shell.bento {
            let weak = Rc::downgrade(&shell);
            bento.connect_style(move |style| {
                if let Some(shell) = weak.upgrade() {
                    for bar in shell.bars() {
                        bar.set_style(style);
                    }
                }
            });
            let weak = Rc::downgrade(&shell);
            bento.connect_toggle_launcher(move || {
                if let Some(shell) = weak.upgrade() {
                    shell.launcher().toggle();
                }
            });
        }

        let monitors = display.monitors();
        let weak = Rc::downgrade(&shell);
        monitors.connect_items_changed(move |monitors, _, _, _| {
            if let Some(shell) = weak.upgrade() {
                shell.update_bars(monitors);
            }
        });
        shell.update_bars(&monitors);
        shell.tick_clock();

        shell
    }

    fn bars(&self) -> Vec<Rc<Bar>> {
        self.bars.borrow().clone()
    }

    pub fn bento_style(&self) -> Style {
        self.bento.as_ref().map(|b| b.style()).unwrap_or_default()
    }

    fn launcher(self: &Rc<Self>) -> Rc<Launcher> {
        self.launcher
            .borrow_mut()
            .get_or_insert_with(|| Launcher::new(self))
            .clone()
    }

    /// The monitor that has the keyboard focus, as bento last said.
    pub fn focused_monitor(&self) -> Option<gdk::Monitor> {
        let name = self.workspaces.focused_monitor()?;
        self.bars()
            .into_iter()
            .map(|bar| bar.monitor.clone())
            .find(|monitor| monitor.connector().as_deref() == Some(name.as_str()))
    }

    /// The bar where notifications and indicators show: the focused monitor's.
    fn focused_bar(&self) -> Option<Rc<Bar>> {
        let bars = self.bars();
        let focused = self.focused_monitor();
        bars.iter()
            .find(|bar| Some(&bar.monitor) == focused.as_ref())
            .or(bars.first())
            .cloned()
    }

    /// One bar per monitor.
    fn update_bars(self: &Rc<Self>, monitors: &gio::ListModel) {
        let monitors: Vec<gdk::Monitor> = (0..monitors.n_items())
            .filter_map(|i| monitors.item(i).and_downcast::<gdk::Monitor>())
            .collect();

        let mut bars = self.bars.borrow_mut();
        bars.retain(|bar| {
            let keep = monitors.contains(&bar.monitor);
            if !keep {
                bar.destroy();
            }
            keep
        });
        let new: Vec<_> = monitors
            .iter()
            .filter(|monitor| !bars.iter().any(|bar| &bar.monitor == *monitor))
            .cloned()
            .collect();
        drop(bars);

        for monitor in new {
            let bar = Bar::new(self, &monitor);
            bar.set_workspaces(&self.workspaces.current());
            bar.set_network(&self.network.state());
            bar.set_battery(&self.battery.state());
            bar.set_audio(&self.audio.state(), false);
            bar.set_tray(&self.tray.items());
            self.bars.borrow_mut().push(bar);
        }
    }

    fn set_audio(&self, audio: &Audio) {
        let last = self.last_audio.replace(Some(*audio));
        let changed = last.is_some_and(|last| {
            last.available && (last.volume != audio.volume || last.muted != audio.muted)
        });
        let focused = self.focused_bar();
        for bar in self.bars() {
            let show = changed && focused.as_ref().is_some_and(|f| Rc::ptr_eq(f, &bar));
            bar.set_audio(audio, show);
        }
    }

    fn notification(&self, event: &NotificationEvent) {
        match event {
            NotificationEvent::Shown(notification) => {
                if let Some(bar) = self.focused_bar() {
                    bar.show_notification(notification);
                }
            }
            NotificationEvent::Replaced(notification) => {
                // It's updated where it shows, or shown again if it went.
                let bars = self.bars();
                let showing = bars
                    .iter()
                    .find(|bar| bar.has_notification(notification.id));
                if let Some(bar) = showing.cloned().or_else(|| self.focused_bar()) {
                    bar.show_notification(notification);
                }
            }
            NotificationEvent::Closed(id) => {
                for bar in self.bars() {
                    bar.hide_notification(*id);
                }
            }
        }
    }

    /// Updates the clocks now and then at the start of every minute.
    fn tick_clock(self: &Rc<Self>) {
        for bar in self.bars() {
            bar.update_clock();
        }
        let seconds = glib::DateTime::now_local().map_or(0., |now| now.seconds());
        let wait = Duration::from_secs_f64((60. - seconds).max(0.05));
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(wait, move || {
            if let Some(shell) = weak.upgrade() {
                shell.tick_clock();
            }
        });
    }
}

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();

    let shell: Rc<RefCell<Option<Rc<Shell>>>> = Rc::default();
    app.connect_activate(move |app| {
        if shell.borrow().is_some() {
            return;
        }

        let css = gtk::CssProvider::new();
        css.load_from_string(include_str!("style.css"));
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().expect("the shell needs a display"),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        // The shell runs as long as the session, with or without windows.
        std::mem::forget(app.hold());
        shell.replace(Some(Shell::new(app)));
    });

    app.run()
}
