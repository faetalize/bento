//! The islands along the top of a monitor: workspaces and the minimap on the left, the clock in
//! the middle, status icons and the tray on the right. Each is a piece of glass that becomes its
//! panel when it's opened: the clock becomes the calendar, the right island quick settings.
//! Notifications show under the right island.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use bento_ipc::{Window, Workspace};
use gtk::gdk;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::osd::Osd;
use crate::services::{Audio, Battery, Network, Notification, TrayIcon, TrayItem};
use crate::spring::Spring;
use crate::stage::{natural_size, Face, IslandId, Place, Rect, Stage};
use crate::wayland::Style;
use crate::Shell;

pub const ISLAND_HEIGHT: f32 = 36.;
/// How close other glass comes for an island to melt into it.
const ISLAND_MELT: f32 = 24.;
const PANEL_RADIUS: f32 = 24.;
const PANEL_WIDTH: f32 = 340.;
const BANNER_WIDTH: i32 = 380;
/// Between an island and the notifications under it, and between notifications.
const STACK_GAP: f32 = 10.;
/// Room kept below the islands for notifications, so the surface doesn't grow when one comes.
const NOTIFICATION_ROOM: f32 = 520.;
/// How long a notification stays when it doesn't say.
const BANNER_TIMEOUT: Duration = Duration::from_secs(6);
/// How long a power button stays armed after the first click.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(3);
/// The minimap's track.
const MINIMAP_WIDTH: i32 = 80;
const MINIMAP_HEIGHT: i32 = 4;

/// The faces of the middle and right islands.
const REST: usize = 0;
const OPEN: usize = 1;

struct Banner {
    id: u32,
    widget: gtk::Box,
    island: IslandId,
    timeout: Option<glib::SourceId>,
    /// Whether it's going away.
    closing: bool,
}

pub struct Islands {
    pub monitor: gdk::Monitor,
    shell: Weak<Shell>,
    window: gtk::Window,
    stage: Stage,
    middle: IslandId,
    right: IslandId,
    workspaces: gtk::Box,
    minimap: Minimap,
    clock: gtk::Label,
    tray: gtk::Box,
    status_button: gtk::Button,
    network_icon: gtk::Image,
    volume_icon: gtk::Image,
    battery_icon: gtk::Image,
    battery_label: gtk::Label,
    calendar_day: gtk::Label,
    calendar_date: gtk::Label,
    calendar_widget: gtk::Calendar,
    quick: gtk::Box,
    volume_scale: gtk::Scale,
    volume_mute: gtk::Button,
    volume_label: gtk::Label,
    /// Set while the slider follows the service, so it doesn't send the value back.
    volume_syncing: Cell<bool>,
    network_row_icon: gtk::Image,
    network_row_label: gtk::Label,
    battery_row: gtk::Box,
    battery_row_icon: gtk::Image,
    battery_row_label: gtk::Label,
    osd: Rc<Osd>,
    banners: RefCell<Vec<Banner>>,
}

impl Islands {
    pub fn new(shell: &Rc<Shell>, monitor: &gdk::Monitor) -> Rc<Self> {
        let window = gtk::Window::builder()
            .application(&shell.app)
            .css_classes(["bento-shell"])
            .build();
        window.init_layer_shell();
        window.set_namespace(Some("bento-shell-islands"));
        window.set_layer(Layer::Top);
        window.set_monitor(Some(monitor));
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Left, true);
        window.set_anchor(Edge::Right, true);
        window.set_exclusive_zone(ISLAND_HEIGHT as i32);
        window.set_keyboard_mode(KeyboardMode::OnDemand);
        window.set_default_size(1, 1);

        let stage = Stage::default();
        stage.set_room_below(NOTIFICATION_ROOM);
        window.set_child(Some(&stage));

        // The left island: workspaces and the minimap.
        let workspaces = gtk::Box::builder()
            .spacing(4)
            .css_classes(["workspaces"])
            .valign(gtk::Align::Center)
            .build();
        let minimap = Minimap::new();
        let left = gtk::Box::builder()
            .spacing(10)
            .css_classes(["island"])
            .build();
        left.append(&workspaces);
        left.append(&minimap.area);

        // The middle island: the clock.
        let clock = gtk::Label::new(None);
        let clock_button = gtk::Button::builder()
            .child(&clock)
            .css_classes(["flat", "island", "island-button"])
            .build();

        // The right island: the tray and the status icons.
        let tray = gtk::Box::builder()
            .spacing(2)
            .valign(gtk::Align::Center)
            .build();
        let network_icon = gtk::Image::new();
        let volume_icon = gtk::Image::new();
        let battery_icon = gtk::Image::new();
        let battery_label = gtk::Label::new(None);
        let status = gtk::Box::builder().spacing(8).build();
        status.append(&network_icon);
        status.append(&volume_icon);
        status.append(&battery_icon);
        status.append(&battery_label);
        let status_button = gtk::Button::builder()
            .child(&status)
            .css_classes(["flat", "island-button"])
            .valign(gtk::Align::Center)
            .build();
        let right = gtk::Box::builder()
            .spacing(4)
            .css_classes(["island"])
            .build();
        right.append(&tray);
        right.append(&status_button);

        // The calendar.
        let calendar_day = gtk::Label::builder()
            .xalign(0.)
            .css_classes(["title-2"])
            .build();
        let calendar_date = gtk::Label::builder()
            .xalign(0.)
            .css_classes(["dim-label"])
            .build();
        let calendar_widget = gtk::Calendar::new();
        let calendar = panel_box();
        calendar.append(&calendar_day);
        calendar.append(&calendar_date);
        calendar.append(&calendar_widget);

        // Quick settings.
        let volume_mute = gtk::Button::builder()
            .css_classes(["flat", "circular"])
            .tooltip_text("Mute")
            .build();
        let volume_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0., 1., 0.01);
        volume_scale.set_hexpand(true);
        let volume_label = gtk::Label::builder()
            .width_chars(4)
            .xalign(1.)
            .css_classes(["numeric"])
            .build();
        let volume_row = row();
        volume_row.append(&volume_mute);
        volume_row.append(&volume_scale);
        volume_row.append(&volume_label);

        let network_row_icon = gtk::Image::new();
        let network_row_label = gtk::Label::builder()
            .xalign(0.)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let network_row = row();
        network_row.append(&network_row_icon);
        network_row.append(&network_row_label);

        let battery_row_icon = gtk::Image::new();
        let battery_row_label = gtk::Label::builder().xalign(0.).hexpand(true).build();
        let battery_row = row();
        battery_row.append(&battery_row_icon);
        battery_row.append(&battery_row_label);

        let power_row = gtk::Box::builder()
            .spacing(12)
            .halign(gtk::Align::Center)
            .margin_top(6)
            .build();

        let quick = panel_box();
        quick.append(&volume_row);
        quick.append(&network_row);
        quick.append(&battery_row);
        quick.append(&power_row);

        let radius = shell.bento_style().corner_radius as f32;
        let left_island = Face::new(&left, radius, at_left(&left)).align(0., 0.5);
        stage.add_island(vec![left_island], REST, ISLAND_MELT);

        // The middle island grows down from where it is, as wide as the calendar, and the right
        // one down and to the left.
        let middle = stage.add_island(
            vec![
                Face::new(&clock_button, radius, in_middle(&clock_button, None)).align(0.5, 0.),
                Face::new(&calendar, PANEL_RADIUS, in_middle(&calendar, Some(0.))).align(0.5, 0.),
            ],
            REST,
            ISLAND_MELT,
        );
        let right_island = stage.add_island(
            vec![
                Face::new(&right, radius, at_right(&right, None)).align(1., 0.),
                Face::new(&quick, PANEL_RADIUS, at_right(&quick, Some(PANEL_WIDTH))).align(1., 0.),
            ],
            REST,
            ISLAND_MELT,
        );

        let osd = Osd::new(shell, monitor);

        let this = Rc::new(Self {
            monitor: monitor.clone(),
            shell: Rc::downgrade(shell),
            window,
            stage,
            middle,
            right: right_island,
            workspaces,
            minimap,
            clock,
            tray,
            status_button,
            network_icon,
            volume_icon,
            battery_icon,
            battery_label,
            calendar_day,
            calendar_date,
            calendar_widget,
            quick,
            volume_scale,
            volume_mute,
            volume_label,
            volume_syncing: Cell::new(false),
            network_row_icon,
            network_row_label,
            battery_row,
            battery_row_icon,
            battery_row_label,
            osd,
            banners: RefCell::default(),
        });

        this.setup_power_buttons(&power_row);
        this.connect_signals(&clock_button);
        this.set_style(shell.bento_style());
        this.update_clock();
        this.window.present();
        this
    }

    fn connect_signals(self: &Rc<Self>, clock_button: &gtk::Button) {
        let islands = Rc::downgrade(self);
        self.window.connect_map(move |window| {
            let Some(islands) = islands.upgrade() else {
                return;
            };
            let (Some(shell), Some(surface)) = (islands.shell.upgrade(), window.surface()) else {
                return;
            };
            islands
                .stage
                .set_glass(shell.bento.as_ref().and_then(|b| b.glass(&surface)));
        });
        let islands = Rc::downgrade(self);
        self.window.connect_unmap(move |_| {
            if let Some(islands) = islands.upgrade() {
                islands.stage.set_glass(None);
            }
        });

        // Panels close when the islands lose the keyboard, like when another window is clicked.
        let islands = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |window| {
            if !window.is_active() {
                if let Some(islands) = islands.upgrade() {
                    islands.close_panels();
                }
            }
        });
        let keys = gtk::EventControllerKey::new();
        let islands = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                if let Some(islands) = islands.upgrade() {
                    islands.close_panels();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.window.add_controller(keys);

        let islands = Rc::downgrade(self);
        clock_button.connect_clicked(move |_| {
            if let Some(islands) = islands.upgrade() {
                islands.toggle(islands.middle);
            }
        });
        let islands = Rc::downgrade(self);
        self.status_button.connect_clicked(move |_| {
            if let Some(islands) = islands.upgrade() {
                islands.toggle(islands.right);
            }
        });

        let shell = self.shell.clone();
        let islands = Rc::downgrade(self);
        self.volume_scale.connect_value_changed(move |scale| {
            let (Some(shell), Some(islands)) = (shell.upgrade(), islands.upgrade()) else {
                return;
            };
            if !islands.volume_syncing.get() {
                shell.audio.set_volume(scale.value());
            }
        });
        let shell = self.shell.clone();
        self.volume_mute.connect_clicked(move |_| {
            if let Some(shell) = shell.upgrade() {
                let audio = shell.audio.state();
                shell.audio.set_muted(!audio.muted);
            }
        });
    }

    fn setup_power_buttons(&self, row: &gtk::Box) {
        use crate::services::power;

        /// The icon, the label, whether it asks to be clicked twice, and what it does.
        type PowerAction = (&'static str, &'static str, bool, fn());
        let actions: [PowerAction; 4] = [
            (
                "weather-clear-night-symbolic",
                "Suspend",
                false,
                power::suspend,
            ),
            ("system-log-out-symbolic", "Log Out", true, log_out),
            ("system-reboot-symbolic", "Restart", true, power::reboot),
            (
                "system-shutdown-symbolic",
                "Power Off",
                true,
                power::power_off,
            ),
        ];

        for (icon, label, confirm, action) in actions {
            let button = gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(label)
                .css_classes(["circular", "power"])
                .build();
            let armed: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
            button.connect_clicked(move |button| {
                if confirm && armed.borrow().is_none() {
                    // The first click arms it; a second one within a few seconds does it.
                    button.add_css_class("destructive-action");
                    button.set_tooltip_text(Some("Click again to confirm"));
                    let armed_ = armed.clone();
                    let button_ = button.downgrade();
                    let label = label.to_owned();
                    let id = glib::timeout_add_local_once(CONFIRM_TIMEOUT, move || {
                        armed_.take();
                        if let Some(button) = button_.upgrade() {
                            button.remove_css_class("destructive-action");
                            button.set_tooltip_text(Some(&label));
                        }
                    });
                    armed.replace(Some(id));
                    return;
                }
                if let Some(id) = armed.take() {
                    id.remove();
                }
                button.remove_css_class("destructive-action");
                action();
            });
            row.append(&button);
        }
    }

    pub fn set_style(&self, style: Style) {
        let gaps = style.gaps.round() as i32;
        self.window.set_margin(Edge::Top, gaps);
        self.window.set_margin(Edge::Left, gaps);
        self.window.set_margin(Edge::Right, gaps);
        self.osd.set_style(style);
        self.stage.relayout();
    }

    /// Opens an island into its panel, closing the other, or closes it.
    fn toggle(&self, island: IslandId) {
        if self.stage.shown(island) == Some(OPEN) {
            self.stage.show(island, REST);
            self.relayout_banners();
            return;
        }
        for other in [self.middle, self.right] {
            if other != island {
                self.stage.show(other, REST);
            }
        }
        if island == self.middle {
            if let Ok(now) = glib::DateTime::now_local() {
                self.calendar_widget.select_day(&now);
            }
        }
        self.stage.show(island, OPEN);
        self.relayout_banners();
    }

    pub fn close_panels(&self) {
        for island in [self.middle, self.right] {
            self.stage.show(island, REST);
        }
        self.relayout_banners();
    }

    fn quick_open(&self) -> bool {
        self.stage.shown(self.right) == Some(OPEN)
    }

    pub fn update_clock(&self) {
        let Ok(now) = glib::DateTime::now_local() else {
            return;
        };
        if let Ok(text) = now.format("%a %e %b  %H:%M") {
            let text = text.split_whitespace().collect::<Vec<_>>();
            // Day, date and month together, then the time.
            let (date, time) = text.split_at(text.len().saturating_sub(1));
            self.clock
                .set_label(&format!("{}   {}", date.join(" "), time.join("")));
        }
        if let Ok(day) = now.format("%A") {
            self.calendar_day.set_label(&day);
        }
        if let Ok(date) = now.format("%e %B %Y") {
            self.calendar_date.set_label(date.trim());
        }
    }

    pub fn set_workspaces(self: &Rc<Self>, workspaces: &[Workspace]) {
        while let Some(child) = self.workspaces.first_child() {
            self.workspaces.remove(&child);
        }

        let connector = self.monitor.connector();
        for workspace in workspaces
            .iter()
            .filter(|ws| ws.monitor.as_deref() == connector.as_deref())
        {
            let label = workspace
                .name
                .clone()
                .unwrap_or_else(|| (workspace.index + 1).to_string());
            let button = gtk::Button::builder()
                .label(label)
                .css_classes(["flat", "workspace"])
                .build();
            if workspace.active {
                button.add_css_class("active");
            }
            if workspace.urgent {
                button.add_css_class("urgent");
            }
            let id = workspace.id;
            let shell = self.shell.clone();
            button.connect_clicked(move |_| {
                if let Some(shell) = shell.upgrade() {
                    shell.workspaces.focus(id);
                }
            });
            self.workspaces.append(&button);
        }
        self.update_minimap();
    }

    /// Shows where the view is on the monitor's active workspace.
    pub fn update_minimap(&self) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        let connector = self.monitor.connector();
        let workspaces = shell.workspaces.current();
        let workspace = workspaces
            .iter()
            .find(|ws| ws.active && ws.monitor.as_deref() == connector.as_deref());
        let view_width = f64::from(self.monitor.geometry().width());
        let span = workspace.map(|ws| minimap_span(ws, &shell.workspaces.windows(), view_width));
        self.minimap.set(span.unwrap_or_default());
    }

    pub fn set_tray(&self, items: &[TrayItem]) {
        while let Some(child) = self.tray.first_child() {
            self.tray.remove(&child);
        }

        for item in items {
            let image = match &item.icon {
                None => gtk::Image::from_icon_name("application-x-executable-symbolic"),
                Some(TrayIcon::Name { name, theme_path }) => {
                    if let Some(path) = theme_path.as_deref().filter(|p| !p.is_empty()) {
                        let theme = gtk::IconTheme::for_display(&WidgetExt::display(&self.window));
                        if !theme.search_path().iter().any(|p| p.as_os_str() == path) {
                            theme.add_search_path(path);
                        }
                    }
                    gtk::Image::from_icon_name(name)
                }
                Some(TrayIcon::Texture(texture)) => gtk::Image::from_paintable(Some(texture)),
            };
            image.set_pixel_size(16);

            let button = gtk::Button::builder()
                .child(&image)
                .css_classes(["flat", "island-button", "tray-item"])
                .valign(gtk::Align::Center)
                .tooltip_text(&item.title)
                .build();

            let id = item.id.clone();
            let shell = self.shell.clone();
            let is_menu = item.item_is_menu;
            button.connect_clicked(move |button| {
                if let Some(shell) = shell.upgrade() {
                    if is_menu {
                        tray_menu(&shell, &id, button);
                    } else {
                        shell.tray.activate(&id, 0, 0);
                    }
                }
            });

            let right = gtk::GestureClick::builder()
                .button(gdk::BUTTON_SECONDARY)
                .build();
            let id = item.id.clone();
            let shell = self.shell.clone();
            let anchor = button.downgrade();
            right.connect_pressed(move |_, _, _, _| {
                if let (Some(shell), Some(button)) = (shell.upgrade(), anchor.upgrade()) {
                    tray_menu(&shell, &id, &button);
                }
            });
            button.add_controller(right);

            self.tray.append(&button);
        }
    }

    pub fn set_audio(&self, audio: &Audio, show_osd: bool) {
        let icon = volume_icon(audio);
        self.volume_icon.set_icon_name(Some(icon));
        self.volume_icon.set_visible(audio.available);
        self.volume_mute.set_icon_name(icon);
        self.volume_mute
            .set_tooltip_text(Some(if audio.muted { "Unmute" } else { "Mute" }));

        self.volume_syncing.set(true);
        self.volume_scale.set_value(audio.volume.min(1.));
        self.volume_syncing.set(false);
        self.volume_label
            .set_label(&format!("{}%", (audio.volume * 100.).round()));

        let level = if audio.muted {
            0.
        } else {
            audio.volume.min(1.)
        };
        // Quick settings shows the volume itself while it's open.
        if show_osd && audio.available && !self.quick_open() {
            self.osd.show(icon, level);
        }
    }

    pub fn set_network(&self, network: &Network) {
        let icon = network.icon_name();
        self.network_icon.set_icon_name(Some(icon));
        self.network_row_icon.set_icon_name(Some(icon));
        let label = match (&network.name, network.connected) {
            (Some(name), true) => name.clone(),
            (None, true) => String::from("Connected"),
            (_, false) => String::from("Offline"),
        };
        self.network_row_label.set_label(&label);
        self.status_button.set_tooltip_text(Some(&label));
    }

    pub fn set_battery(&self, battery: &Battery) {
        self.battery_icon.set_visible(battery.present);
        self.battery_label.set_visible(battery.present);
        self.battery_row.set_visible(battery.present);
        if !battery.present {
            return;
        }
        let percentage = format!("{}%", battery.percentage.round());
        self.battery_icon.set_icon_name(Some(&battery.icon_name));
        self.battery_label.set_label(&percentage);
        self.battery_row_icon
            .set_icon_name(Some(&battery.icon_name));
        self.battery_row_label.set_label(&if battery.charging {
            format!("{percentage}, charging")
        } else {
            percentage
        });
    }

    /// Shows a notification under the right island, or updates it if it's showing.
    pub fn show_notification(self: &Rc<Self>, notification: &Notification) {
        let mut banners = self.banners.borrow_mut();
        if let Some(banner) = banners
            .iter_mut()
            .find(|b| b.id == notification.id && !b.closing)
        {
            fill_banner(&banner.widget, notification, &self.shell);
            if let Some(id) = banner.timeout.take() {
                id.remove();
            }
            banner.timeout = self.banner_timeout(notification);
            return;
        }
        drop(banners);

        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .width_request(BANNER_WIDTH)
            .css_classes(["banner"])
            .build();
        fill_banner(&widget, notification, &self.shell);

        // It grows out of nothing in its place.
        let place = self.banner_place(notification.id, &widget);
        let island = self.stage.add_island(
            vec![
                Face::nothing(place.clone()),
                Face::new(&widget, PANEL_RADIUS, place),
            ],
            0,
            ISLAND_MELT,
        );
        let timeout = self.banner_timeout(notification);
        self.banners.borrow_mut().push(Banner {
            id: notification.id,
            widget,
            island,
            timeout,
            closing: false,
        });
        self.stage.show(island, 1);
    }

    /// Where a notification goes: under the right island, or quick settings while it's open,
    /// below the notifications before it.
    fn banner_place(self: &Rc<Self>, id: u32, widget: &gtk::Box) -> Place {
        let islands = Rc::downgrade(self);
        let widget = widget.clone();
        Rc::new(move |width| {
            let (w, h) = natural_size(&widget);
            let Some(islands) = islands.upgrade() else {
                return Rect::new(width - w, ISLAND_HEIGHT + STACK_GAP, w, h);
            };
            let top = if islands.quick_open() {
                natural_size(&islands.quick).1
            } else {
                ISLAND_HEIGHT
            };
            let above: f32 = islands
                .banners
                .borrow()
                .iter()
                .take_while(|banner| banner.id != id)
                .filter(|banner| !banner.closing)
                .map(|banner| natural_size(&banner.widget).1 + STACK_GAP)
                .sum();
            Rect::new(width - w, top + STACK_GAP + above, w, h)
        })
    }

    fn relayout_banners(&self) {
        self.stage.relayout();
    }

    fn banner_timeout(self: &Rc<Self>, notification: &Notification) -> Option<glib::SourceId> {
        let timeout = match notification.expire_timeout {
            0 => return None,
            ms if ms > 0 => Duration::from_millis(ms as u64),
            // Critical notifications stay until they're dismissed.
            _ if notification.urgency >= 2 => return None,
            _ => BANNER_TIMEOUT,
        };
        let islands = Rc::downgrade(self);
        let id = notification.id;
        Some(glib::timeout_add_local_once(timeout, move || {
            let Some(islands) = islands.upgrade() else {
                return;
            };
            if let Some(banner) = islands.banners.borrow_mut().iter_mut().find(|b| b.id == id) {
                banner.timeout = None;
            }
            if let Some(shell) = islands.shell.upgrade() {
                shell.notifications.expire(id);
            }
        }))
    }

    pub fn has_notification(&self, id: u32) -> bool {
        self.banners
            .borrow()
            .iter()
            .any(|banner| banner.id == id && !banner.closing)
    }

    /// Takes the islands away, when their monitor went.
    pub fn destroy(&self) {
        self.osd.destroy();
        self.window.destroy();
    }

    /// Takes a notification away, once it closed: it shrinks back into nothing, and the ones
    /// under it move up.
    pub fn hide_notification(self: &Rc<Self>, id: u32) {
        let island = {
            let mut banners = self.banners.borrow_mut();
            let Some(banner) = banners.iter_mut().find(|b| b.id == id && !b.closing) else {
                return;
            };
            banner.closing = true;
            if let Some(timeout) = banner.timeout.take() {
                timeout.remove();
            }
            banner.island
        };

        let islands = Rc::downgrade(self);
        self.stage.show_then(
            island,
            0,
            Some(move || {
                let Some(islands) = islands.upgrade() else {
                    return;
                };
                islands.banners.borrow_mut().retain(|b| b.island != island);
                islands.stage.remove(island);
            }),
        );
        self.relayout_banners();
    }
}

/// The left island: at the left, as wide as its content.
fn at_left(content: &impl IsA<gtk::Widget>) -> Place {
    let content = content.clone().upcast::<gtk::Widget>();
    Rc::new(move |_| {
        let (w, _) = natural_size(&content);
        Rect::new(0., 0., w.max(ISLAND_HEIGHT), ISLAND_HEIGHT)
    })
}

/// In the middle, as wide as the content, and as tall as an island or, with `Some`, as the
/// content.
fn in_middle(content: &impl IsA<gtk::Widget>, panel: Option<f32>) -> Place {
    let content = content.clone().upcast::<gtk::Widget>();
    Rc::new(move |width| {
        let (w, h) = natural_size(&content);
        let w = w.max(ISLAND_HEIGHT).min(width);
        let h = if panel.is_some() { h } else { ISLAND_HEIGHT };
        Rect::new((width - w) / 2., 0., w, h)
    })
}

/// At the right, as wide as the content, or at least `panel` wide and as tall as the content for
/// a panel.
fn at_right(content: &impl IsA<gtk::Widget>, panel: Option<f32>) -> Place {
    let content = content.clone().upcast::<gtk::Widget>();
    Rc::new(move |width| {
        let (w, h) = natural_size(&content);
        let (w, h) = match panel {
            Some(min) => (w.max(min), h),
            None => (w.max(ISLAND_HEIGHT), ISLAND_HEIGHT),
        };
        let w = w.min(width);
        Rect::new(width - w, 0., w, h)
    })
}

/// Where the view is along a workspace's tiles, and the focused tile, as fractions of the whole
/// stretch they and the view cover.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Span {
    view: (f64, f64),
    focused: Option<(f64, f64)>,
}

impl Default for Span {
    fn default() -> Self {
        Self {
            view: (0., 1.),
            focused: None,
        }
    }
}

/// The span of `workspace`'s tiles, from bento's windows, with a view `view_width` wide.
fn minimap_span(workspace: &Workspace, windows: &[Window], view_width: f64) -> Span {
    let tiles: Vec<&Window> = windows
        .iter()
        .filter(|win| win.workspace == Some(workspace.id) && !win.floating)
        .filter(|win| win.rect.is_some())
        .collect();
    let view = (workspace.camera.x, workspace.camera.x + view_width);
    let (start, end) = tiles
        .iter()
        .filter_map(|win| win.rect.as_ref())
        .fold(view, |(start, end), rect| {
            (start.min(rect.x), end.max(rect.x + rect.w))
        });
    let length = (end - start).max(1.);
    let fraction = |(a, b): (f64, f64)| ((a - start) / length, (b - a) / length);
    Span {
        view: fraction(view),
        focused: tiles
            .iter()
            .find(|win| win.focused)
            .and_then(|win| win.rect.as_ref())
            .map(|rect| fraction((rect.x, rect.x + rect.w))),
    }
}

/// A track of the active workspace, with the part the view shows highlighted, and the focused
/// tile in it, moving smoothly as the view does.
struct Minimap {
    area: gtk::DrawingArea,
    /// The view's start and width, and the focused tile's, as fractions of the track.
    springs: Rc<RefCell<[Spring; 4]>>,
    has_focus: Rc<Cell<bool>>,
    tick: Rc<RefCell<Option<gtk::TickCallbackId>>>,
}

impl Minimap {
    fn new() -> Self {
        let area = gtk::DrawingArea::builder()
            .content_width(MINIMAP_WIDTH)
            .content_height(MINIMAP_HEIGHT)
            .valign(gtk::Align::Center)
            .build();
        let springs = Rc::new(RefCell::new([
            Spring::new(0.),
            Spring::new(1.),
            Spring::new(0.),
            Spring::new(0.),
        ]));
        let has_focus = Rc::new(Cell::new(false));
        {
            let (springs, has_focus) = (springs.clone(), has_focus.clone());
            area.set_draw_func(move |area, cr, width, height| {
                let [view_start, view_width, focus_start, focus_width] =
                    springs.borrow().map(|s| s.value);
                let (width, height) = (f64::from(width), f64::from(height));
                let radius = height / 2.;
                let rounded = |x: f64, w: f64| {
                    let w = w.max(height);
                    cr.new_sub_path();
                    cr.arc(
                        x + w - radius,
                        radius,
                        radius,
                        -90f64.to_radians(),
                        90f64.to_radians(),
                    );
                    cr.arc(
                        x + radius,
                        radius,
                        radius,
                        90f64.to_radians(),
                        270f64.to_radians(),
                    );
                    cr.close_path();
                };
                let fg = area.color();
                cr.set_source_rgba(
                    f64::from(fg.red()),
                    f64::from(fg.green()),
                    f64::from(fg.blue()),
                    0.22,
                );
                rounded(0., width);
                let _ = cr.fill();

                let accent = adw::StyleManager::default().accent_color_rgba();
                cr.set_source_rgba(
                    f64::from(accent.red()),
                    f64::from(accent.green()),
                    f64::from(accent.blue()),
                    1.,
                );
                rounded(view_start * width, view_width * width);
                let _ = cr.fill();

                if has_focus.get() {
                    cr.set_source_rgba(1., 1., 1., 0.7);
                    rounded(focus_start * width, focus_width * width);
                    let _ = cr.fill();
                }
            });
        }
        Self {
            area,
            springs,
            has_focus,
            tick: Rc::default(),
        }
    }

    fn set(&self, span: Span) {
        let (focus_start, focus_width) = span.focused.unwrap_or_default();
        self.has_focus.set(span.focused.is_some());
        {
            let mut springs = self.springs.borrow_mut();
            let targets = [span.view.0, span.view.1, focus_start, focus_width];
            for (spring, target) in springs.iter_mut().zip(targets) {
                spring.target = target;
            }
        }
        if self.tick.borrow().is_some() {
            return;
        }
        let (springs, tick) = (self.springs.clone(), self.tick.clone());
        let last = Cell::new(None::<i64>);
        let id = self.area.add_tick_callback(move |area, clock| {
            let now = clock.frame_time();
            let dt = last
                .replace(Some(now))
                .map_or(1. / 60., |last| ((now - last) as f64 / 1e6).clamp(0., 0.05));
            let mut springs = springs.borrow_mut();
            let mut settled = true;
            for spring in springs.iter_mut() {
                spring.advance(dt);
                settled &= spring.settle(0.001);
            }
            area.queue_draw();
            if settled {
                tick.take();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        self.tick.replace(Some(id));
    }
}

fn log_out() {
    let mut properties = serde_json::Map::new();
    properties.insert("skip-confirmation".into(), true.into());
    if let Err(err) = crate::services::ipc::run_action("quit", Vec::new(), properties) {
        eprintln!("couldn't log out: {err}");
    }
}

fn panel_box() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["panel"])
        .build()
}

fn row() -> gtk::Box {
    gtk::Box::builder().spacing(12).css_classes(["row"]).build()
}

pub fn volume_icon(audio: &Audio) -> &'static str {
    if audio.muted || audio.volume <= 0. {
        "audio-volume-muted-symbolic"
    } else if audio.volume < 0.34 {
        "audio-volume-low-symbolic"
    } else if audio.volume < 0.67 {
        "audio-volume-medium-symbolic"
    } else {
        "audio-volume-high-symbolic"
    }
}

/// Fills a banner with a notification: its picture, app, summary, body and actions.
fn fill_banner(banner: &gtk::Box, notification: &Notification, shell: &Weak<Shell>) {
    while let Some(child) = banner.first_child() {
        banner.remove(&child);
    }
    for controller in banner.observe_controllers().snapshot() {
        if let Ok(controller) = controller.downcast::<gtk::EventController>() {
            banner.remove_controller(&controller);
        }
    }

    let picture = match (&notification.image, notification.app_icon.as_str()) {
        (Some(texture), _) => gtk::Image::from_paintable(Some(texture)),
        (None, icon) if icon.starts_with('/') || icon.starts_with("file://") => {
            let path = icon.trim_start_matches("file://");
            gtk::Image::from_file(path)
        }
        (None, "") => gtk::Image::from_icon_name("preferences-system-notifications-symbolic"),
        (None, icon) => gtk::Image::from_icon_name(icon),
    };
    picture.set_pixel_size(40);
    picture.set_valign(gtk::Align::Start);
    picture.add_css_class("banner-picture");

    let app = gtk::Label::builder()
        .label(&notification.app_name)
        .xalign(0.)
        .css_classes(["caption", "dim-label"])
        .build();
    let summary = gtk::Label::builder()
        .label(&notification.summary)
        .xalign(0.)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .lines(2)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["heading"])
        .build();
    let body = gtk::Label::builder()
        .xalign(0.)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .lines(4)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(40)
        .build();
    let markup = notification.body_markup();
    if gtk::pango::parse_markup(&markup, '\0').is_ok() {
        body.set_markup(&markup);
    } else {
        body.set_label(&notification.body);
    }
    body.set_visible(!notification.body.is_empty());

    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    text.append(&app);
    text.append(&summary);
    text.append(&body);

    let close = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .css_classes(["flat", "circular"])
        .valign(gtk::Align::Start)
        .tooltip_text("Dismiss")
        .build();
    let id = notification.id;
    let shell_ = shell.clone();
    close.connect_clicked(move |_| {
        if let Some(shell) = shell_.upgrade() {
            shell.notifications.dismiss(id);
        }
    });

    let top = gtk::Box::builder().spacing(12).build();
    top.append(&picture);
    top.append(&text);
    top.append(&close);
    banner.append(&top);

    let buttons: Vec<_> = notification
        .actions
        .iter()
        .filter(|(key, _)| key != "default")
        .collect();
    if !buttons.is_empty() {
        let actions = gtk::Box::builder()
            .spacing(6)
            .homogeneous(true)
            .css_classes(["banner-actions"])
            .build();
        for (key, label) in buttons {
            let button = gtk::Button::with_label(label);
            let key = key.clone();
            let shell_ = shell.clone();
            button.connect_clicked(move |_| {
                if let Some(shell) = shell_.upgrade() {
                    shell.notifications.invoke(id, &key);
                }
            });
            actions.append(&button);
        }
        banner.append(&actions);
    }

    // Clicking the notification itself runs its default action, or dismisses it.
    let has_default = notification.actions.iter().any(|(key, _)| key == "default");
    let click = gtk::GestureClick::new();
    let shell_ = shell.clone();
    click.connect_released(move |gesture, _, _, _| {
        let Some(shell) = shell_.upgrade() else {
            return;
        };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if has_default {
            shell.notifications.invoke(id, "default");
        } else {
            shell.notifications.dismiss(id);
        }
    });
    banner.add_controller(click);
}

/// Shows a tray item's menu under it, or asks the app to show its own.
fn tray_menu(shell: &Rc<Shell>, id: &str, button: &gtk::Button) {
    let id_ = id.to_owned();
    let shell_ = Rc::downgrade(shell);
    let anchor = button.downgrade();
    shell.tray.menu(id, move |menu| {
        let Some(button) = anchor.upgrade() else {
            return;
        };
        match menu {
            Some((model, actions)) => show_menu(&button, &model, &actions),
            None => {
                if let Some(shell) = shell_.upgrade() {
                    shell.tray.context_menu(&id_, 0, 0);
                }
            }
        }
    });
}

fn show_menu(anchor: &gtk::Button, model: &gio::MenuModel, actions: &gio::SimpleActionGroup) {
    anchor.insert_action_group("tray", Some(actions));
    let popover = gtk::PopoverMenu::from_model(Some(model));
    popover.set_parent(anchor);
    popover.set_has_arrow(false);
    popover.connect_closed(|popover| {
        let popover = popover.clone();
        // Unparenting inside `closed` would cut its own signal short.
        glib::idle_add_local_once(move || popover.unparent());
    });
    popover.popup();
}
