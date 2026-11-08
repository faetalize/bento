//! The bar along the top of a monitor, and everything that grows out of it: the calendar, quick
//! settings, the volume indicator and notifications.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use bento_ipc::Workspace;
use gtk::gdk;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::services::{Audio, Battery, Network, Notification, TrayIcon, TrayItem};
use crate::stage::{natural_size, Place, Rect, Stage};
use crate::wayland::Style;
use crate::Shell;

pub const BAR_HEIGHT: f32 = 40.;
/// Between the bar and what grows out of it: close enough that the glass stays one piece.
const PANEL_GAP: f32 = 6.;
/// Between pieces stacked under the bar.
const STACK_GAP: f32 = 10.;
const PANEL_RADIUS: f32 = 24.;
const PANEL_WIDTH: i32 = 340;
const BANNER_WIDTH: i32 = 380;
/// How long a notification stays when it doesn't say.
const BANNER_TIMEOUT: Duration = Duration::from_secs(6);
const OSD_TIMEOUT: Duration = Duration::from_millis(1500);
/// How long a power button stays armed after the first click.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(3);

struct Banner {
    id: u32,
    widget: gtk::Box,
    timeout: Option<glib::SourceId>,
}

pub struct Bar {
    pub monitor: gdk::Monitor,
    shell: Weak<Shell>,
    window: gtk::Window,
    stage: Stage,
    workspaces: gtk::Box,
    clock_button: gtk::Button,
    clock: gtk::Label,
    tray: gtk::Box,
    status_button: gtk::Button,
    network_icon: gtk::Image,
    volume_icon: gtk::Image,
    battery_icon: gtk::Image,
    battery_label: gtk::Label,
    calendar: gtk::Box,
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
    osd: gtk::Box,
    osd_icon: gtk::Image,
    osd_level: gtk::LevelBar,
    osd_timeout: RefCell<Option<glib::SourceId>>,
    banners: RefCell<Vec<Banner>>,
}

impl Bar {
    pub fn new(shell: &Rc<Shell>, monitor: &gdk::Monitor) -> Rc<Self> {
        let window = gtk::Window::builder()
            .application(&shell.app)
            .css_classes(["bento-shell"])
            .build();
        window.init_layer_shell();
        window.set_namespace(Some("bento-shell-bar"));
        window.set_layer(Layer::Top);
        window.set_monitor(Some(monitor));
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Left, true);
        window.set_anchor(Edge::Right, true);
        window.set_exclusive_zone(BAR_HEIGHT as i32);
        window.set_keyboard_mode(KeyboardMode::OnDemand);
        window.set_default_size(1, 1);

        let stage = Stage::default();
        window.set_child(Some(&stage));

        // The bar.
        let workspaces = gtk::Box::builder()
            .spacing(4)
            .css_classes(["workspaces"])
            .valign(gtk::Align::Center)
            .build();

        let clock = gtk::Label::new(None);
        let clock_button = gtk::Button::builder()
            .child(&clock)
            .css_classes(["flat", "bar-item"])
            .valign(gtk::Align::Center)
            .build();

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
            .css_classes(["flat", "bar-item"])
            .valign(gtk::Align::Center)
            .build();

        let end = gtk::Box::builder().spacing(6).build();
        end.append(&tray);
        end.append(&status_button);

        let bar = gtk::CenterBox::builder()
            .css_classes(["bar"])
            .start_widget(&workspaces)
            .center_widget(&clock_button)
            .end_widget(&end)
            .build();
        let place_bar: Place = Rc::new(|_, width| {
            let rect = Rect::new(0., 0., width, BAR_HEIGHT);
            (rect, rect)
        });

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
        quick.set_width_request(PANEL_WIDTH);
        quick.append(&volume_row);
        quick.append(&network_row);
        quick.append(&battery_row);
        quick.append(&power_row);

        // The volume indicator.
        let osd_icon = gtk::Image::builder().pixel_size(20).build();
        let osd_level = gtk::LevelBar::builder()
            .min_value(0.)
            .max_value(1.)
            .width_request(160)
            .valign(gtk::Align::Center)
            .build();
        let osd = gtk::Box::builder().spacing(12).css_classes(["osd"]).build();
        osd.append(&osd_icon);
        osd.append(&osd_level);

        let this = Rc::new(Self {
            monitor: monitor.clone(),
            shell: Rc::downgrade(shell),
            window,
            stage,
            workspaces,
            clock_button,
            clock,
            tray,
            status_button,
            network_icon,
            volume_icon,
            battery_icon,
            battery_label,
            calendar,
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
            osd_icon,
            osd_level,
            osd_timeout: RefCell::default(),
            banners: RefCell::default(),
        });

        this.stage
            .add_fixed(&bar, shell.bento_style().corner_radius as f32, place_bar);
        this.stage.add_piece(
            &this.calendar,
            PANEL_RADIUS,
            this.below(&this.clock_button, &this.calendar, |_| 0.),
        );
        this.stage.add_piece(
            &this.quick,
            PANEL_RADIUS,
            this.below(&this.status_button, &this.quick, |_| 0.),
        );
        this.stage.add_piece(
            &this.osd,
            PANEL_RADIUS,
            this.below(&this.status_button, &this.osd, |bar| {
                bar.stacked_height(&bar.quick)
            }),
        );

        this.setup_power_buttons(&power_row);
        this.connect_signals();
        this.set_style(shell.bento_style());
        this.update_clock();
        this.window.present();
        this
    }

    /// Places a piece under the bar, growing out of `anchor`: centred under it, `offset` further
    /// down.
    fn below(
        self: &Rc<Self>,
        anchor: &impl IsA<gtk::Widget>,
        content: &impl IsA<gtk::Widget>,
        offset: impl Fn(&Self) -> f32 + 'static,
    ) -> Place {
        let anchor = anchor.clone().upcast::<gtk::Widget>();
        let content = content.clone().upcast::<gtk::Widget>();
        let bar = Rc::downgrade(self);
        Rc::new(move |stage, width| {
            let (content_width, height) = natural_size(&content);
            let content_width = content_width.min(width);
            let source =
                stage
                    .rect_of(&anchor)
                    .unwrap_or(Rect::new(width / 2., 0., 0., BAR_HEIGHT));
            let center = source.x + source.width / 2.;
            let x = (center - content_width / 2.).clamp(0., (width - content_width).max(0.));
            let offset = bar.upgrade().map_or(0., |bar| offset(&bar));
            let to = Rect::new(x, BAR_HEIGHT + PANEL_GAP + offset, content_width, height);
            (source, to)
        })
    }

    /// The height a piece takes in the stack under the bar while it's showing.
    fn stacked_height(&self, piece: &impl IsA<gtk::Widget>) -> f32 {
        if !self.stage.is_open(piece) {
            return 0.;
        }
        natural_size(piece).1 + STACK_GAP
    }

    fn connect_signals(self: &Rc<Self>) {
        let bar = Rc::downgrade(self);
        self.stage.connect_resize(move || {
            if let Some(bar) = bar.upgrade() {
                // The window takes the stage's size, so it shrinks as well as grows.
                bar.window.set_default_size(1, 1);
            }
        });

        let bar = Rc::downgrade(self);
        self.window.connect_map(move |window| {
            let Some(bar) = bar.upgrade() else { return };
            let (Some(shell), Some(surface)) = (bar.shell.upgrade(), window.surface()) else {
                return;
            };
            bar.stage
                .set_glass(shell.bento.as_ref().and_then(|b| b.glass(&surface)));
        });
        let bar = Rc::downgrade(self);
        self.window.connect_unmap(move |_| {
            if let Some(bar) = bar.upgrade() {
                bar.stage.set_glass(None);
            }
        });

        // Panels close when the bar loses the keyboard, like when another window is clicked.
        let bar = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |window| {
            if !window.is_active() {
                if let Some(bar) = bar.upgrade() {
                    bar.close_panels();
                }
            }
        });
        let keys = gtk::EventControllerKey::new();
        let bar = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                if let Some(bar) = bar.upgrade() {
                    bar.close_panels();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.window.add_controller(keys);

        let bar = Rc::downgrade(self);
        self.clock_button.connect_clicked(move |_| {
            if let Some(bar) = bar.upgrade() {
                bar.toggle_panel(&bar.calendar.clone());
            }
        });
        let bar = Rc::downgrade(self);
        self.status_button.connect_clicked(move |_| {
            if let Some(bar) = bar.upgrade() {
                bar.toggle_panel(&bar.quick.clone());
            }
        });

        let shell = self.shell.clone();
        let bar = Rc::downgrade(self);
        self.volume_scale.connect_value_changed(move |scale| {
            let (Some(shell), Some(bar)) = (shell.upgrade(), bar.upgrade()) else {
                return;
            };
            if !bar.volume_syncing.get() {
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
        self.stage.queue_resize();
    }

    fn toggle_panel(&self, panel: &gtk::Box) {
        if self.stage.is_open(panel) {
            self.stage.close(panel, None::<fn()>);
            return;
        }
        for other in [&self.calendar, &self.quick] {
            if other != panel {
                self.stage.close(other, None::<fn()>);
            }
        }
        if panel == &self.calendar {
            let now = glib::DateTime::now_local().ok();
            if let Some(now) = now {
                self.calendar_widget.select_day(&now);
            }
        }
        // The volume indicator makes room for quick settings, which shows the volume itself.
        if panel == &self.quick {
            self.hide_osd();
        }
        self.stage.open(panel);
        self.stage.queue_resize();
    }

    pub fn close_panels(&self) {
        for panel in [&self.calendar, &self.quick] {
            self.stage.close(panel, None::<fn()>);
        }
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
                .css_classes(["flat", "bar-item", "tray-item"])
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

        self.osd_icon.set_icon_name(Some(icon));
        self.osd_level.set_value(if audio.muted {
            0.
        } else {
            audio.volume.min(1.)
        });

        if show_osd && audio.available && !self.stage.is_open(&self.quick) {
            self.show_osd();
        }
    }

    fn show_osd(&self) {
        if let Some(id) = self.osd_timeout.take() {
            id.remove();
        }
        self.stage.open(&self.osd);
        let stage = self.stage.downgrade();
        let osd = self.osd.downgrade();
        let id = glib::timeout_add_local_once(OSD_TIMEOUT, move || {
            if let (Some(stage), Some(osd)) = (stage.upgrade(), osd.upgrade()) {
                stage.close(&osd, None::<fn()>);
            }
        });
        // The timeout clears itself once it ran.
        self.osd_timeout.replace(Some(id));
    }

    fn hide_osd(&self) {
        if let Some(id) = self.osd_timeout.take() {
            id.remove();
        }
        self.stage.close(&self.osd, None::<fn()>);
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

    /// Shows a notification under the bar, or updates it if it's showing.
    pub fn show_notification(self: &Rc<Self>, notification: &Notification) {
        let mut banners = self.banners.borrow_mut();
        if let Some(banner) = banners.iter_mut().find(|b| b.id == notification.id) {
            fill_banner(&banner.widget, notification, &self.shell);
            if let Some(id) = banner.timeout.take() {
                id.remove();
            }
            banner.timeout = self.banner_timeout(notification);
            return;
        }

        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .width_request(BANNER_WIDTH)
            .css_classes(["banner"])
            .build();
        fill_banner(&widget, notification, &self.shell);
        let timeout = self.banner_timeout(notification);
        banners.push(Banner {
            id: notification.id,
            widget: widget.clone(),
            timeout,
        });
        drop(banners);

        let id = notification.id;
        self.stage.add_piece(
            &widget,
            PANEL_RADIUS,
            self.below(&self.clock_button, &widget, move |bar| {
                bar.stacked_height(&bar.calendar) + bar.banners_above(id)
            }),
        );
        self.stage.open(&widget);
    }

    /// The height of the notifications shown before this one.
    fn banners_above(&self, id: u32) -> f32 {
        self.banners
            .borrow()
            .iter()
            .take_while(|banner| banner.id != id)
            .map(|banner| natural_size(&banner.widget).1 + STACK_GAP)
            .sum()
    }

    fn banner_timeout(self: &Rc<Self>, notification: &Notification) -> Option<glib::SourceId> {
        let timeout = match notification.expire_timeout {
            0 => return None,
            ms if ms > 0 => Duration::from_millis(ms as u64),
            // Critical notifications stay until they're dismissed.
            _ if notification.urgency >= 2 => return None,
            _ => BANNER_TIMEOUT,
        };
        let bar = Rc::downgrade(self);
        let id = notification.id;
        Some(glib::timeout_add_local_once(timeout, move || {
            let Some(bar) = bar.upgrade() else { return };
            if let Some(banner) = bar.banners.borrow_mut().iter_mut().find(|b| b.id == id) {
                banner.timeout = None;
            }
            if let Some(shell) = bar.shell.upgrade() {
                shell.notifications.expire(id);
            }
        }))
    }

    pub fn has_notification(&self, id: u32) -> bool {
        self.banners.borrow().iter().any(|banner| banner.id == id)
    }

    /// Takes the bar away, when its monitor went.
    pub fn destroy(&self) {
        self.window.destroy();
    }

    /// Takes a notification away, once it closed.
    pub fn hide_notification(self: &Rc<Self>, id: u32) {
        let banners = self.banners.borrow();
        let Some(banner) = banners.iter().find(|b| b.id == id) else {
            return;
        };
        let widget = banner.widget.clone();
        drop(banners);

        let bar = Rc::downgrade(self);
        let widget_ = widget.clone();
        self.stage.close(
            &widget,
            Some(move || {
                let Some(bar) = bar.upgrade() else { return };
                let mut banners = bar.banners.borrow_mut();
                if let Some(i) = banners.iter().position(|b| b.id == id) {
                    let banner = banners.remove(i);
                    if let Some(timeout) = banner.timeout {
                        timeout.remove();
                    }
                }
                drop(banners);
                bar.stage.remove(&widget_);
            }),
        );
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

fn volume_icon(audio: &Audio) -> &'static str {
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
