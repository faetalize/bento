//! Building pages: the shared frame with its header, and a row for each kind of setting.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use niri_config::Color;

use crate::model::{Category, Control, Group, Input, Setting};
use crate::store::Store;

/// What a page gets to build itself.
#[derive(Clone)]
pub struct PageContext {
    pub store: Store,
    pub toasts: adw::ToastOverlay,
    anchors: Rc<RefCell<HashMap<String, gtk::Widget>>>,
}

impl PageContext {
    pub fn new(store: Store, toasts: adw::ToastOverlay) -> Self {
        Self {
            store,
            toasts,
            anchors: Rc::default(),
        }
    }

    /// Marks a widget as the place search jumps to for `anchor`.
    pub fn anchor(&self, anchor: &str, widget: &impl IsA<gtk::Widget>) {
        self.anchors
            .borrow_mut()
            .insert(anchor.to_owned(), widget.clone().upcast());
    }

    pub fn find_anchor(&self, anchor: &str) -> Option<gtk::Widget> {
        self.anchors.borrow().get(anchor).cloned()
    }

    pub fn toast(&self, title: &str) {
        self.toasts.add_toast(adw::Toast::new(title));
    }
}

/// Wraps a page's groups the way `AdwPreferencesPage` lays them out, with the category's
/// description on top. Custom pages need their own widgets between groups, which
/// `AdwPreferencesPage` doesn't allow.
pub fn frame(category: &Category, content: &impl IsA<gtk::Widget>) -> gtk::Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 24);
    column.add_css_class("page-column");
    if !category.description.is_empty() {
        let description = gtk::Label::builder()
            .label(category.description)
            .xalign(0.)
            .wrap(true)
            .css_classes(["dim-label"])
            .build();
        column.append(&description);
    }
    column.append(content);

    let clamp = adw::Clamp::builder().child(&column).build();
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&clamp)
        .build()
        .upcast()
}

pub fn settings_page(groups: &'static [Group], ctx: &PageContext) -> gtk::Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 24);
    for group in groups {
        let widget = adw::PreferencesGroup::builder().title(group.title).build();
        if let Some(description) = group.description {
            widget.set_description(Some(description));
        }
        for setting in &group.settings {
            let row = setting_row(setting, ctx);
            widget.add(&row);
            ctx.anchor(setting.path, &row);
        }
        column.append(&widget);
    }
    column.upcast()
}

/// Shown for pages that aren't in the app yet.
pub fn planned_page(category: &Category, ctx: &PageContext) -> gtk::Widget {
    let button = gtk::Button::builder()
        .label("Open config.kdl")
        .halign(gtk::Align::Center)
        .css_classes(["pill", "suggested-action"])
        .build();
    let path = ctx.store.config_path().to_owned();
    button.connect_clicked(move |button| open_file(button, &path));

    let status = adw::StatusPage::builder()
        .icon_name(category.icon)
        .title(format!("{} settings are coming", category.title))
        .description(format!(
            "{}\n\nUntil this page is ready, set them in config.kdl.",
            category.description
        ))
        .child(&button)
        .vexpand(true)
        .build();
    status.add_css_class("planned");
    status.upcast()
}

pub fn open_file(widget: &impl IsA<gtk::Widget>, path: &std::path::Path) {
    let launcher = gtk::FileLauncher::new(Some(&gtk::gio::File::for_path(path)));
    let window = widget.root().and_downcast::<gtk::Window>();
    launcher.launch(window.as_ref(), None::<&gtk::gio::Cancellable>, |_| ());
}

pub fn open_uri(widget: &impl IsA<gtk::Widget>, uri: &str) {
    let launcher = gtk::UriLauncher::new(uri);
    let window = widget.root().and_downcast::<gtk::Window>();
    launcher.launch(window.as_ref(), None::<&gtk::gio::Cancellable>, |_| ());
}

/// A flat round button that undoes a setting, shown only while settings.kdl sets it.
pub fn reset_button() -> gtk::Button {
    gtk::Button::builder()
        .icon_name("edit-undo-symbolic")
        .tooltip_text("Reset to the value from config.kdl, or the default")
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular", "reset"])
        .visible(false)
        .build()
}

fn setting_row(setting: &'static Setting, ctx: &PageContext) -> gtk::Widget {
    let store = ctx.store.clone();
    // Set while the row shows a new value, so its own change signal doesn't write it back.
    let updating = Rc::new(Cell::new(false));

    let edit = {
        let store = store.clone();
        let updating = updating.clone();
        move |input: Input| {
            if !updating.get() {
                store.edit(setting.path, |doc| setting.write(doc, input));
            }
        }
    };

    let reset = reset_button();
    reset.connect_clicked({
        let store = store.clone();
        move |_| store.edit(setting.path, |doc| setting.reset(doc))
    });

    let warning = gtk::Image::builder()
        .icon_name("dialog-warning-symbolic")
        .tooltip_text("Something after the include in config.kdl overrides this")
        .css_classes(["warning"])
        .visible(false)
        .build();

    let (row, show): (adw::PreferencesRow, Box<dyn Fn()>) = match &setting.control {
        Control::Switch { get, inverted, .. } => {
            let switch = gtk::Switch::builder().valign(gtk::Align::Center).build();
            let row = action_row(
                setting,
                &[
                    warning.upcast_ref(),
                    reset.upcast_ref(),
                    switch.upcast_ref(),
                ],
            );
            row.set_activatable_widget(Some(&switch));
            switch.connect_active_notify(move |switch| edit(Input::Bool(switch.is_active())));
            let (get, inverted, store) = (*get, *inverted, store.clone());
            let show = move || switch.set_active(get(&store.config()) != inverted);
            (row.upcast(), Box::new(show))
        }
        Control::Number {
            get,
            min,
            max,
            step,
            digits,
        } => {
            let spin = gtk::SpinButton::with_range(*min, *max, *step);
            spin.set_digits(*digits);
            // Equal widths keep the fields lined up whatever their range.
            spin.set_width_chars(5);
            spin.set_valign(gtk::Align::Center);
            let row = action_row(
                setting,
                &[warning.upcast_ref(), reset.upcast_ref(), spin.upcast_ref()],
            );
            spin.connect_value_changed(move |spin| edit(Input::Number(spin.value())));
            let (get, store) = (*get, store.clone());
            let show = move || spin.set_value(get(&store.config()));
            (row.upcast(), Box::new(show))
        }
        Control::Choice { get, choices } => {
            let labels: Vec<&str> = choices.iter().map(|(_, label)| *label).collect();
            let dropdown = gtk::DropDown::from_strings(&labels);
            dropdown.set_valign(gtk::Align::Center);
            let row = action_row(
                setting,
                &[
                    warning.upcast_ref(),
                    reset.upcast_ref(),
                    dropdown.upcast_ref(),
                ],
            );
            dropdown.connect_selected_notify(move |dropdown| {
                if let Some((value, _)) = choices.get(dropdown.selected() as usize) {
                    edit(Input::Choice(*value));
                }
            });
            let (get, store) = (*get, store.clone());
            let show = move || {
                let current = get(&store.config());
                let idx = choices.iter().position(|(value, _)| *value == current);
                dropdown.set_selected(idx.map_or(gtk::INVALID_LIST_POSITION, |i| i as u32));
            };
            (row.upcast(), Box::new(show))
        }
        Control::Text { get } => {
            let row = adw::EntryRow::builder()
                .title(setting.title)
                .show_apply_button(true)
                .build();
            row.add_suffix(&warning);
            row.add_suffix(&reset);
            {
                let edit = edit.clone();
                row.connect_apply(move |row| edit(Input::Text(row.text().trim().to_owned())));
            }
            let (get, store) = (*get, store.clone());
            let entry = row.clone();
            let show = move || {
                // Don't replace what someone is typing.
                if !entry.has_focus() || entry.text().is_empty() {
                    entry.set_text(&get(&store.config()));
                }
            };
            (row.upcast(), Box::new(show))
        }
        Control::Color { get } => {
            let dialog = gtk::ColorDialog::builder().with_alpha(true).build();
            let button = gtk::ColorDialogButton::new(Some(dialog));
            button.set_valign(gtk::Align::Center);
            let row = action_row(
                setting,
                &[
                    warning.upcast_ref(),
                    reset.upcast_ref(),
                    button.upcast_ref(),
                ],
            );
            button.connect_rgba_notify(move |button| {
                let rgba = button.rgba();
                edit(Input::Color(Color::from_array_unpremul([
                    rgba.red(),
                    rgba.green(),
                    rgba.blue(),
                    rgba.alpha(),
                ])));
            });
            let (get, store) = (*get, store.clone());
            let show = move || {
                let [r, g, b, a] = get(&store.config()).to_array_unpremul();
                button.set_rgba(&gtk::gdk::RGBA::new(r, g, b, a));
            };
            (row.upcast(), Box::new(show))
        }
    };

    let refresh = {
        let row = row.clone();
        let store = store.clone();
        move || {
            updating.set(true);
            show();
            updating.set(false);

            let doc = store.doc();
            let config = store.config();
            let is_set = setting.is_set(&doc);
            reset.set_visible(is_set && setting.can_reset());
            warning.set_visible(is_set && !setting.applied(&doc, &config));
            if let Some(enabled) = setting.enabled_if {
                row.set_sensitive(enabled(&config));
            }
        }
    };
    refresh();
    store.subscribe(refresh);

    row.upcast()
}

fn action_row(setting: &Setting, suffixes: &[&gtk::Widget]) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(setting.title).build();
    if let Some(subtitle) = setting.subtitle {
        row.set_subtitle(subtitle);
    }
    for suffix in suffixes {
        row.add_suffix(*suffix);
    }
    row
}
