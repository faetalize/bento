//! bento's shell: the islands, the launcher and notifications around the workspaces.

mod islands;
mod launcher;
mod osd;
mod services;
mod spring;
mod stage;
mod wayland;

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio};

use crate::islands::Islands;
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
    islands: RefCell<Vec<Rc<Islands>>>,
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
            let w_ = weak.clone();
            let workspaces = Workspaces::start(
                move |workspaces| {
                    if let Some(shell) = w.upgrade() {
                        for islands in shell.islands() {
                            islands.set_workspaces(workspaces);
                        }
                    }
                },
                move || {
                    if let Some(shell) = w_.upgrade() {
                        for islands in shell.islands() {
                            islands.update_minimap();
                        }
                    }
                },
            );
            let w = weak.clone();
            let audio = AudioService::start(move |audio| {
                if let Some(shell) = w.upgrade() {
                    shell.set_audio(audio);
                }
            });
            let w = weak.clone();
            let network = NetworkService::start(move |network| {
                if let Some(shell) = w.upgrade() {
                    for islands in shell.islands() {
                        islands.set_network(network);
                    }
                }
            });
            let w = weak.clone();
            let battery = BatteryService::start(move |battery| {
                if let Some(shell) = w.upgrade() {
                    for islands in shell.islands() {
                        islands.set_battery(battery);
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
                    for islands in shell.islands() {
                        islands.set_tray(items);
                    }
                }
            });

            Self {
                app: app.clone(),
                bento,
                islands: RefCell::default(),
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
                    for islands in shell.islands() {
                        islands.set_style(style);
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
                shell.update_islands(monitors);
            }
        });
        shell.update_islands(&monitors);
        shell.tick_clock();

        // The launcher reads the apps once it's idle, rather than the first time it opens.
        let weak = Rc::downgrade(&shell);
        glib::idle_add_local_once(move || {
            if let Some(shell) = weak.upgrade() {
                shell.launcher().prepare();
            }
        });

        shell
    }

    fn islands(&self) -> Vec<Rc<Islands>> {
        self.islands.borrow().clone()
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
        self.islands()
            .into_iter()
            .map(|islands| islands.monitor.clone())
            .find(|monitor| monitor.connector().as_deref() == Some(name.as_str()))
    }

    /// The islands where notifications and indicators show: the focused monitor's.
    fn focused_islands(&self) -> Option<Rc<Islands>> {
        let all = self.islands();
        let focused = self.focused_monitor();
        all.iter()
            .find(|islands| Some(&islands.monitor) == focused.as_ref())
            .or(all.first())
            .cloned()
    }

    /// One islands per monitor.
    fn update_islands(self: &Rc<Self>, monitors: &gio::ListModel) {
        let monitors: Vec<gdk::Monitor> = (0..monitors.n_items())
            .filter_map(|i| monitors.item(i).and_downcast::<gdk::Monitor>())
            .collect();

        let mut all = self.islands.borrow_mut();
        all.retain(|islands| {
            let keep = monitors.contains(&islands.monitor);
            if !keep {
                islands.destroy();
            }
            keep
        });
        let new: Vec<_> = monitors
            .iter()
            .filter(|monitor| !all.iter().any(|islands| &islands.monitor == *monitor))
            .cloned()
            .collect();
        drop(all);

        for monitor in new {
            let islands = Islands::new(self, &monitor);
            islands.set_workspaces(&self.workspaces.current());
            islands.set_network(&self.network.state());
            islands.set_battery(&self.battery.state());
            islands.set_audio(&self.audio.state(), false);
            islands.set_tray(&self.tray.items());
            self.islands.borrow_mut().push(islands);
        }
    }

    fn set_audio(&self, audio: &Audio) {
        let last = self.last_audio.replace(Some(*audio));
        // The sound server tells of many changes to the speaker that don't touch its volume.
        if last == Some(*audio) {
            return;
        }
        let changed = last.is_some_and(|last| {
            last.available && (last.volume != audio.volume || last.muted != audio.muted)
        });
        let focused = self.focused_islands();
        for islands in self.islands() {
            let show = changed && focused.as_ref().is_some_and(|f| Rc::ptr_eq(f, &islands));
            islands.set_audio(audio, show);
        }
    }

    fn notification(&self, event: &NotificationEvent) {
        match event {
            NotificationEvent::Shown(notification) => {
                if let Some(islands) = self.focused_islands() {
                    islands.show_notification(notification);
                }
            }
            NotificationEvent::Replaced(notification) => {
                // It's updated where it shows, or shown again if it went.
                let all = self.islands();
                let showing = all
                    .iter()
                    .find(|islands| islands.has_notification(notification.id));
                if let Some(islands) = showing.cloned().or_else(|| self.focused_islands()) {
                    islands.show_notification(notification);
                }
            }
            NotificationEvent::Closed(id) => {
                for islands in self.islands() {
                    islands.hide_notification(*id);
                }
            }
        }
    }

    /// Updates the clocks now and then at the start of every minute.
    fn tick_clock(self: &Rc<Self>) {
        for islands in self.islands() {
            islands.update_clock();
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

        let display = gdk::Display::default().expect("the shell needs a display");
        let css = gtk::CssProvider::new();
        css.load_from_string(include_str!("style.css"));
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        // Above the user's own CSS, which may give every window a background, as that would
        // cover the glass.
        let bare = gtk::CssProvider::new();
        bare.load_from_string(include_str!("bare.css"));
        gtk::style_context_add_provider_for_display(
            &display,
            &bare,
            gtk::STYLE_PROVIDER_PRIORITY_USER + 1,
        );

        // The shell runs as long as the session, with or without windows.
        std::mem::forget(app.hold());
        shell.replace(Some(Shell::new(app)));
    });

    app.run()
}
