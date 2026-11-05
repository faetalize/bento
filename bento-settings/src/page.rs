//! Building pages: the shared frame, and a row for each kind of setting.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio};
use niri_config::Color;

use crate::model::{Control, Group, Input, Setting};
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

/// Keeps a page's state alive as long as `widget`. Pages hand their state to signal handlers as
/// weak references, so that the handlers don't keep it alive in a cycle; this is what does.
pub fn keep_alive<T: 'static>(widget: &impl IsA<gtk::Widget>, state: Rc<T>) {
    // SAFETY: the key is only ever set here, with this type, and never read back.
    unsafe {
        widget.set_data(
            &format!("bento-page-state-{}", std::any::type_name::<T>()),
            state,
        )
    };
}

/// Lays out a page's groups the way `AdwPreferencesPage` does. Custom pages rebuild their groups
/// in place, which `AdwPreferencesPage` doesn't make easy.
pub fn frame(content: &impl IsA<gtk::Widget>) -> gtk::Widget {
    let clamp = adw::Clamp::builder()
        .child(content)
        .css_classes(["page-column"])
        .build();
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&clamp)
        .build()
        .upcast()
}

pub fn settings_page(groups: &'static [Group], ctx: &PageContext) -> gtk::Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 24);
    for group in groups {
        if let Some(build) = group.build {
            column.append(&build(ctx));
            continue;
        }
        let widget = adw::PreferencesGroup::new();
        if let Some(title) = group.title {
            widget.set_title(title);
        }
        for setting in &group.settings {
            widget.add(&setting_row(setting, ctx));
        }
        column.append(&widget);
    }
    column.upcast()
}

pub fn open_file(widget: &impl IsA<gtk::Widget>, path: &std::path::Path) {
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    let window = widget.root().and_downcast::<gtk::Window>();
    launcher.launch(window.as_ref(), None::<&gio::Cancellable>, |_| ());
}

pub fn open_uri(widget: &impl IsA<gtk::Widget>, uri: &str) {
    let launcher = gtk::UriLauncher::new(uri);
    let window = widget.root().and_downcast::<gtk::Window>();
    launcher.launch(window.as_ref(), None::<&gio::Cancellable>, |_| ());
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
                store.edit(setting.path, move |doc| setting.write(doc, input));
            }
        }
    };

    let (row, show): (adw::PreferencesRow, Box<dyn Fn()>) = match &setting.control {
        Control::Switch { get, inverted, .. } if !setting.children.is_empty() => {
            let row = adw::ExpanderRow::builder()
                .title(setting.title)
                .show_enable_switch(true)
                .build();
            for child in &setting.children {
                row.add_row(&setting_row(child, ctx));
            }
            row.connect_enable_expansion_notify(move |row| {
                edit(Input::Bool(row.enables_expansion()))
            });
            let (get, inverted, store) = (*get, *inverted, store.clone());
            let expander = row.clone();
            let show = move || expander.set_enable_expansion(get(&store.config()) != inverted);
            (row.upcast(), Box::new(show))
        }
        Control::Switch { get, inverted, .. } => {
            let row = adw::SwitchRow::builder().title(setting.title).build();
            row.connect_active_notify(move |row| edit(Input::Bool(row.is_active())));
            let (get, inverted, store) = (*get, *inverted, store.clone());
            let switch = row.clone();
            let show = move || switch.set_active(get(&store.config()) != inverted);
            (row.upcast(), Box::new(show))
        }
        Control::Number {
            get,
            min,
            max,
            step,
            slider: true,
            format,
            ..
        } => {
            let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, *min, *max, *step);
            scale.set_draw_value(false);
            scale.set_hexpand(true);
            scale.set_valign(gtk::Align::Center);
            scale.set_width_request(200);
            let value = gtk::Label::builder()
                .xalign(1.)
                .width_chars(8)
                .css_classes(["dim-label", "numeric"])
                .build();
            let row = adw::ActionRow::builder().title(setting.title).build();
            row.add_suffix(&value);
            row.add_suffix(&scale);
            let format = format.unwrap_or(|x| format!("{x}"));
            {
                let value = value.clone();
                scale.connect_value_changed(move |scale| {
                    value.set_label(&format(scale.value()));
                    edit(Input::Number(scale.value()));
                });
            }
            let (get, store) = (*get, store.clone());
            let show = move || {
                // Set the label too: the scale doesn't signal when the value stays the same.
                let current = get(&store.config());
                scale.set_value(current);
                value.set_label(&format(current));
            };
            (row.upcast(), Box::new(show))
        }
        Control::Number {
            get,
            min,
            max,
            step,
            digits,
            slider: false,
            ..
        } => {
            let row = adw::SpinRow::with_range(*min, *max, *step);
            row.set_title(setting.title);
            row.set_digits(*digits);
            row.connect_value_notify(move |row| edit(Input::Number(row.value())));
            let (get, store) = (*get, store.clone());
            let spin = row.clone();
            let show = move || spin.set_value(get(&store.config()));
            (row.upcast(), Box::new(show))
        }
        Control::Choice { get, choices } => {
            let labels: Vec<&str> = choices.iter().map(|(_, label)| *label).collect();
            let row = adw::ComboRow::builder()
                .title(setting.title)
                .model(&gtk::StringList::new(&labels))
                .build();
            row.connect_selected_notify(move |row| {
                if let Some((value, _)) = choices.get(row.selected() as usize) {
                    edit(Input::Choice(*value));
                }
            });
            let (get, store) = (*get, store.clone());
            let combo = row.clone();
            let show = move || {
                let current = get(&store.config());
                let idx = choices.iter().position(|(value, _)| *value == current);
                combo.set_selected(idx.map_or(gtk::INVALID_LIST_POSITION, |i| i as u32));
            };
            (row.upcast(), Box::new(show))
        }
        Control::Text { get } => {
            let row = adw::EntryRow::builder()
                .title(setting.title)
                .show_apply_button(true)
                .build();
            row.connect_apply(move |row| edit(Input::Text(row.text().trim().to_owned())));
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
            let row = adw::ActionRow::builder().title(setting.title).build();
            row.add_suffix(&button);
            row.set_activatable_widget(Some(&button));
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
                button.set_rgba(&gdk::RGBA::new(r, g, b, a));
            };
            (row.upcast(), Box::new(show))
        }
    };
    ctx.anchor(setting.path, &row);

    let refresh = move || {
        // A change still on its way to the config would be undone by showing the old value.
        if store.is_pending(setting.path) {
            return;
        }
        updating.set(true);
        show();
        updating.set(false);
    };
    refresh();
    ctx.store.subscribe(refresh);

    row.upcast()
}
