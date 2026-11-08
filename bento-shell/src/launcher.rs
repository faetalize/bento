//! The launcher: a glass search field in the middle of the screen that finds apps. It grows out of
//! nothing in its middle, and shrinks back into nothing.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, gio};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::stage::{natural_size, Face, IslandId, Place, Rect, Stage};
use crate::Shell;

const WIDTH: i32 = 560;
const RADIUS: f32 = 26.;
/// How close other glass comes for it to melt into it.
const MELT: f32 = 24.;
/// Its faces.
const NOTHING: usize = 0;
const SHOWN: usize = 1;
const MAX_RESULTS: usize = 8;
/// How far down the screen the launcher sits.
const TOP: f64 = 0.22;

pub struct Launcher {
    shell: Weak<Shell>,
    window: gtk::Window,
    stage: Stage,
    island: IslandId,
    entry: gtk::SearchEntry,
    results: gtk::ListBox,
    apps: RefCell<Vec<gio::AppInfo>>,
    shown: RefCell<Vec<gio::AppInfo>>,
}

impl Launcher {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let window = gtk::Window::builder()
            .application(&shell.app)
            .css_classes(["bento-shell"])
            .build();
        window.init_layer_shell();
        window.set_namespace(Some("bento-shell-launcher"));
        window.set_layer(Layer::Overlay);
        // Across the screen, letting the pointer through everywhere but the glass.
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Left, true);
        window.set_anchor(Edge::Right, true);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        window.set_default_size(1, 1);

        let entry = gtk::SearchEntry::builder()
            .placeholder_text("Search apps")
            .hexpand(true)
            .css_classes(["launcher-entry"])
            .build();
        let results = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Browse)
            .css_classes(["launcher-results"])
            .build();
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .width_request(WIDTH)
            .css_classes(["launcher"])
            .build();
        content.append(&entry);
        content.append(&results);

        let stage = Stage::default();
        window.set_child(Some(&stage));

        // In the middle, as large as it is.
        let place: Place = {
            let content = content.clone();
            Rc::new(move |width| {
                let (w, h) = natural_size(&content);
                Rect::new((width - w) / 2., 0., w, h)
            })
        };
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
            entry,
            results,
            apps: RefCell::default(),
            shown: RefCell::default(),
        });

        this.connect_signals();
        this
    }

    fn connect_signals(self: &Rc<Self>) {
        let launcher = Rc::downgrade(self);
        self.window.connect_map(move |window| {
            let Some(launcher) = launcher.upgrade() else {
                return;
            };
            let (Some(shell), Some(surface)) = (launcher.shell.upgrade(), window.surface()) else {
                return;
            };
            launcher
                .stage
                .set_glass(shell.bento.as_ref().and_then(|b| b.glass(&surface)));
        });
        let launcher = Rc::downgrade(self);
        self.window.connect_unmap(move |_| {
            if let Some(launcher) = launcher.upgrade() {
                launcher.stage.set_glass(None);
            }
        });

        let launcher = Rc::downgrade(self);
        self.entry.connect_search_changed(move |_| {
            if let Some(launcher) = launcher.upgrade() {
                launcher.search();
            }
        });
        let launcher = Rc::downgrade(self);
        self.entry.connect_activate(move |_| {
            if let Some(launcher) = launcher.upgrade() {
                let row = launcher
                    .results
                    .selected_row()
                    .or_else(|| launcher.results.row_at_index(0));
                if let Some(row) = row {
                    launcher.launch(row.index());
                }
            }
        });
        let launcher = Rc::downgrade(self);
        self.results.connect_row_activated(move |_, row| {
            if let Some(launcher) = launcher.upgrade() {
                launcher.launch(row.index());
            }
        });

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let launcher = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(launcher) = launcher.upgrade() else {
                return glib::Propagation::Proceed;
            };
            match key {
                gdk::Key::Escape => {
                    launcher.hide();
                    glib::Propagation::Stop
                }
                gdk::Key::Down | gdk::Key::Up => {
                    launcher.move_selection(if key == gdk::Key::Down { 1 } else { -1 });
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        self.window.add_controller(keys);

        // Clicking another window takes the keyboard away from it.
        let launcher = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |window| {
            if !window.is_active() {
                if let Some(launcher) = launcher.upgrade() {
                    launcher.hide();
                }
            }
        });
    }

    pub fn toggle(self: &Rc<Self>) {
        if self.stage.shown(self.island) == Some(SHOWN) {
            self.hide();
        } else {
            self.show();
        }
    }

    fn show(self: &Rc<Self>) {
        let mut apps: Vec<_> = gio::AppInfo::all()
            .into_iter()
            .filter(|app| app.should_show())
            .collect();
        apps.sort_by_key(|app| app.display_name().to_lowercase());
        self.apps.replace(apps);

        self.entry.set_text("");
        self.search();

        // Without a monitor of its own it goes on the focused one, each time it's shown.
        let height = self
            .shell
            .upgrade()
            .and_then(|shell| shell.focused_monitor())
            .map_or(1080, |monitor| monitor.geometry().height());
        self.window
            .set_margin(Edge::Top, (f64::from(height) * TOP).round() as i32);
        self.window.present();
        self.entry.grab_focus();
        self.stage.show(self.island, SHOWN);
    }

    fn hide(self: &Rc<Self>) {
        let window = self.window.downgrade();
        self.stage.show_then(
            self.island,
            NOTHING,
            Some(move || {
                if let Some(window) = window.upgrade() {
                    window.set_visible(false);
                }
            }),
        );
    }

    fn search(&self) {
        let query = self.entry.text().to_lowercase();
        let apps = self.apps.borrow();
        let mut matches: Vec<(u8, &gio::AppInfo)> = apps
            .iter()
            .filter_map(|app| score(app, &query).map(|score| (score, app)))
            .collect();
        // Better matches first, keeping the alphabetical order among equals.
        matches.sort_by_key(|(score, _)| *score);

        while let Some(row) = self.results.row_at_index(0) {
            self.results.remove(&row);
        }
        let shown: Vec<gio::AppInfo> = matches
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, app)| app.clone())
            .collect();
        for app in &shown {
            self.results.append(&result_row(app));
        }
        self.results.set_visible(!shown.is_empty());
        if let Some(row) = self.results.row_at_index(0) {
            self.results.select_row(Some(&row));
        }
        self.shown.replace(shown);
        self.stage.queue_resize();
        self.window.set_default_size(1, 1);
    }

    fn move_selection(&self, by: i32) {
        let count = self.shown.borrow().len() as i32;
        if count == 0 {
            return;
        }
        let current = self.results.selected_row().map_or(-1, |row| row.index());
        let next = (current + by).rem_euclid(count);
        if let Some(row) = self.results.row_at_index(next) {
            self.results.select_row(Some(&row));
        }
    }

    fn launch(self: &Rc<Self>, index: i32) {
        let Some(app) = self.shown.borrow().get(index as usize).cloned() else {
            return;
        };
        let context = WidgetExt::display(&self.window).app_launch_context();
        if let Err(err) = app.launch(&[], Some(&context)) {
            eprintln!("couldn't start {}: {err}", app.display_name());
        }
        self.hide();
    }
}

/// How well an app matches a query, lower is better, or `None` if it doesn't.
fn score(app: &gio::AppInfo, query: &str) -> Option<u8> {
    if query.is_empty() {
        return Some(0);
    }
    let name = app.display_name().to_lowercase();
    if name.starts_with(query) {
        return Some(0);
    }
    if name.split_whitespace().any(|word| word.starts_with(query)) {
        return Some(1);
    }
    if name.contains(query) {
        return Some(2);
    }
    let desktop = app.downcast_ref::<gio::DesktopAppInfo>();
    let extra = desktop
        .map(|app| {
            let mut words: Vec<String> = app.keywords().iter().map(|k| k.to_lowercase()).collect();
            if let Some(generic) = app.generic_name() {
                words.push(generic.to_lowercase());
            }
            words
        })
        .unwrap_or_default();
    let executable = app.executable().to_string_lossy().to_lowercase();
    if extra.iter().any(|word| word.contains(query)) || executable.contains(query) {
        return Some(3);
    }
    None
}

fn result_row(app: &gio::AppInfo) -> gtk::ListBoxRow {
    let icon = match app.icon() {
        Some(icon) => gtk::Image::from_gicon(&icon),
        None => gtk::Image::from_icon_name("application-x-executable"),
    };
    icon.set_pixel_size(32);

    let name = gtk::Label::builder()
        .label(app.display_name())
        .xalign(0.)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let description = gtk::Label::builder()
        .label(app.description().unwrap_or_default())
        .xalign(0.)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["caption", "dim-label"])
        .build();
    description.set_visible(!description.label().is_empty());

    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .build();
    text.append(&name);
    text.append(&description);

    let row = gtk::Box::builder().spacing(12).build();
    row.append(&icon);
    row.append(&text);

    gtk::ListBoxRow::builder()
        .child(&row)
        .css_classes(["launcher-row"])
        .build()
}
