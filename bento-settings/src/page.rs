//! Building pages: the shared frame, and a row for each kind of setting.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio};
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

/// Shown for pages that aren't in the app yet.
pub fn planned_page(category: &Category, ctx: &PageContext) -> gtk::Widget {
    let button = gtk::Button::builder()
        .label("Open config.kdl")
        .halign(gtk::Align::Center)
        .css_classes(["pill"])
        .build();
    let path = ctx.store.config_path().to_owned();
    button.connect_clicked(move |button| open_file(button, &path));

    adw::StatusPage::builder()
        .icon_name(category.icon)
        .title("Coming Soon")
        .description("Set these in config.kdl for now.")
        .child(&button)
        .vexpand(true)
        .build()
        .upcast()
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

/// Gives a row a right-click and long-press menu with "Reset", enabled while `can_reset` says so.
///
/// Native rows have no good place for a visible reset button: suffixes land after the switch or
/// spin button.
pub fn reset_menu(
    row: &impl IsA<gtk::Widget>,
    can_reset: impl Fn() -> bool + 'static,
    reset: impl Fn() + 'static,
) {
    let row = row.as_ref();
    let action = gio::SimpleAction::new("reset", None);
    action.connect_activate(move |_, _| reset());
    let actions = gio::SimpleActionGroup::new();
    actions.add_action(&action);
    row.insert_action_group("row", Some(&actions));

    let menu = gio::Menu::new();
    menu.append(Some("Reset"), Some("row.reset"));
    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(row);
    popover.set_has_arrow(false);
    popover.set_halign(gtk::Align::Start);

    let show = move |x: f64, y: f64| {
        action.set_enabled(can_reset());
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
    };
    let show = Rc::new(show);

    let click = gtk::GestureClick::builder()
        .button(gdk::BUTTON_SECONDARY)
        .build();
    {
        let show = show.clone();
        click.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            show(x, y);
        });
    }
    row.add_controller(click);

    let press = gtk::GestureLongPress::builder().touch_only(true).build();
    press.connect_pressed(move |gesture, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        show(x, y);
    });
    row.add_controller(press);
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
            digits,
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

    if setting.can_reset() {
        let store_check = store.clone();
        let store_reset = store.clone();
        reset_menu(
            &row,
            move || setting.is_set(&store_check.doc()),
            move || store_reset.edit(setting.path, |doc| setting.reset(doc)),
        );
    }

    let refresh = {
        let row = row.clone();
        move || {
            updating.set(true);
            show();
            updating.set(false);

            // Something after the include in config.kdl wins over settings.kdl.
            let doc = store.doc();
            let overridden = setting.is_set(&doc) && !setting.applied(&doc, &store.config());
            set_subtitle(&row, overridden.then_some("Overridden in config.kdl"));
            if overridden {
                row.add_css_class("overridden");
            } else {
                row.remove_css_class("overridden");
            }
        }
    };
    refresh();
    ctx.store.subscribe(refresh);

    row.upcast()
}

fn set_subtitle(row: &adw::PreferencesRow, subtitle: Option<&str>) {
    let subtitle = subtitle.unwrap_or_default();
    if let Some(row) = row.downcast_ref::<adw::ActionRow>() {
        row.set_subtitle(subtitle);
    } else if let Some(row) = row.downcast_ref::<adw::ExpanderRow>() {
        row.set_subtitle(subtitle);
    }
}
