//! The layout engine: which one runs, its options and its actions.
//!
//! Which engine to run comes from the config. What it offers comes from the running engine over
//! IPC, since an engine declares its options and actions itself.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib::translate::FromGlib;
use gtk::{gdk, glib};
use niri_config::input::ModKey;
use niri_config::{Action, Key, Modifiers, TilingLayout, Trigger};
use niri_ipc::{LayoutEngine, LayoutEngineOption, LayoutEngineOptionValue};

use crate::ipc;
use crate::kdl::{self, Value};
use crate::model::Category;
use crate::page::{reset_menu, PageContext};
use crate::store::{option_value, tilde};

pub fn category() -> Category {
    Category::custom("engine", "Engine", build)
        .icon("view-dual-symbolic")
        .keywords(&[
            "tiling",
            "layout",
            "scrolling",
            "grid",
            "javascript",
            "bento.js",
            "niri",
        ])
        .search_item(
            "engine.choice",
            "Layout Engine",
            &["switch engine", "bento", "niri", "custom", "file"],
        )
        .search_item(
            "engine.options",
            "Engine Options",
            &["gravity", "configure"],
        )
        .search_item(
            "engine.actions",
            "Engine Shortcuts",
            &["layout-action", "actions", "binds", "keybinds"],
        )
}

struct View {
    ctx: PageContext,
    column: gtk::Box,
    /// The configured layout and the running engine with its option values left out. The page is
    /// rebuilt only when this changes; new option values update the rows in place, so a row isn't
    /// replaced while someone is using it.
    shown: RefCell<Option<Shape>>,
    options: RefCell<Vec<OptionRow>>,
    /// Set while rows show new values, so their change signals don't write them back.
    updating: Rc<Cell<bool>>,
}

type Shape = (TilingLayout, Result<LayoutEngine, String>);

/// Shows a new value in an option's row.
type ShowValue = Box<dyn Fn(&LayoutEngineOptionValue)>;

struct OptionRow {
    show: ShowValue,
}

fn shape(layout: TilingLayout, engine: &Result<LayoutEngine, String>) -> Shape {
    let engine = engine.clone().map(|mut engine| {
        for option in &mut engine.options {
            option.value = option.default.clone();
        }
        engine
    });
    (layout, engine)
}

fn build(ctx: &PageContext) -> gtk::Widget {
    let view = Rc::new(View {
        ctx: ctx.clone(),
        column: gtk::Box::new(gtk::Orientation::Vertical, 24),
        shown: RefCell::new(None),
        options: RefCell::new(Vec::new()),
        updating: Rc::new(Cell::new(false)),
    });
    refresh(&view);

    // bento reloads the config a moment after a write, so ask again a few times.
    let weak = Rc::downgrade(&view);
    ctx.store.subscribe(move || {
        for delay in [200, 700, 2000] {
            let weak = weak.clone();
            glib::timeout_add_local_once(Duration::from_millis(delay), move || {
                if let Some(view) = weak.upgrade() {
                    refresh(&view);
                }
            });
        }
    });

    view.column.clone().upcast()
}

fn refresh(view: &Rc<View>) {
    let engine = ipc::layout_engine();
    let shape = shape(view.ctx.store.config().tiling.layout.clone(), &engine);
    if view.shown.borrow().as_ref() == Some(&shape) {
        if let Ok(engine) = &engine {
            show_values(view, engine);
        }
        return;
    }

    while let Some(child) = view.column.first_child() {
        view.column.remove(&child);
    }
    view.options.borrow_mut().clear();
    view.column
        .append(&choice_group(view, engine.as_ref().ok()));
    match &engine {
        Ok(engine) => {
            view.column.append(&options_group(view, engine));
            view.column.append(&shortcuts_group(view, engine));
            show_values(view, engine);
        }
        Err(err) => view.column.append(&offline_group(view, err)),
    }
    view.shown.replace(Some(shape));
}

fn show_values(view: &View, engine: &LayoutEngine) {
    view.updating.set(true);
    for (row, option) in view.options.borrow().iter().zip(&engine.options) {
        (row.show)(&option.value);
    }
    view.updating.set(false);
}

fn set_layout(ctx: &PageContext, layout: String) {
    ctx.store.edit("tiling.layout", |doc| {
        kdl::set_args(doc, &["tiling", "layout"], vec![Value::Str(layout)]);
        // Options belong to the engine that declared them; bento rejects unknown ones.
        kdl::remove(doc, &["tiling", "options"]);
    });
}

/// Engines that ship with bento, as the config names them and as the app shows them.
const BUILT_IN: &[(&str, &str)] = &[("bento", "Bento"), ("niri", "Niri")];

fn choice_group(view: &Rc<View>, running: Option<&LayoutEngine>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    view.ctx.anchor("engine.choice", &group);

    let configured = view.ctx.store.config().tiling.layout.clone();
    let file = match &configured {
        TilingLayout::File(path) => Some(path.clone()),
        TilingLayout::BuiltIn(_) => None,
    };

    let mut labels: Vec<&str> = BUILT_IN.iter().map(|(_, label)| *label).collect();
    labels.push("Custom File");
    let custom = BUILT_IN.len() as u32;
    let selected = match &configured {
        TilingLayout::BuiltIn(name) => BUILT_IN
            .iter()
            .position(|(n, _)| n == name)
            .map_or(gtk::INVALID_LIST_POSITION, |i| i as u32),
        TilingLayout::File(_) => custom,
    };
    let combo = adw::ComboRow::builder()
        .title("Engine")
        .model(&gtk::StringList::new(&labels))
        .selected(selected)
        .build();
    if let Some(description) = running.and_then(|engine| engine.description.as_deref()) {
        combo.set_subtitle(description);
    }
    {
        let view = Rc::downgrade(view);
        let has_file = file.is_some();
        combo.connect_selected_notify(move |combo| {
            let Some(view) = view.upgrade() else { return };
            let idx = combo.selected();
            if let Some((name, _)) = BUILT_IN.get(idx as usize) {
                set_layout(&view.ctx, (*name).to_owned());
            } else if idx == custom && !has_file {
                choose_file(&view, combo);
            }
        });
    }
    group.add(&combo);

    if let Some(path) = file {
        let row = adw::ActionRow::builder()
            .title("File")
            .subtitle(tilde(&path))
            .activatable(true)
            .build();
        row.add_suffix(&gtk::Image::from_icon_name("document-open-symbolic"));
        let view = Rc::downgrade(view);
        row.connect_activated(move |row| {
            if let Some(view) = view.upgrade() {
                choose_file(&view, row);
            }
        });
        group.add(&row);
    }
    group
}

fn choose_file(view: &Rc<View>, widget: &impl IsA<gtk::Widget>) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Layout Engines"));
    filter.add_pattern("*.js");
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title("Choose a Layout Engine")
        .filters(&filters)
        .build();

    let view = Rc::downgrade(view);
    let window = widget.root().and_downcast::<gtk::Window>();
    dialog.open(
        window.as_ref(),
        None::<&gtk::gio::Cancellable>,
        move |result| {
            let Some(view) = view.upgrade() else { return };
            match result.ok().and_then(|file| file.path()) {
                Some(path) => set_layout(&view.ctx, tilde(&path)),
                // Cancelled: rebuild, so the dropdown shows the configured engine again.
                None => {
                    view.shown.replace(None);
                    refresh(&view);
                }
            }
        },
    );
}

fn options_group(view: &Rc<View>, engine: &LayoutEngine) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Options").build();
    view.ctx.anchor("engine.options", &group);

    if engine.options.is_empty() {
        let row = adw::ActionRow::builder()
            .title("No Options")
            .css_classes(["dim-label"])
            .build();
        group.add(&row);
    }
    for option in &engine.options {
        group.add(&option_row(view, option));
    }
    group
}

fn option_row(view: &Rc<View>, option: &LayoutEngineOption) -> gtk::Widget {
    let path = format!("tiling.options.{}", option.name);
    let title = sentence_case(&option.name.replace(['-', '_'], " "));

    let write = {
        let ctx = view.ctx.clone();
        let updating = view.updating.clone();
        let path = path.clone();
        let default = option.default.clone();
        move |value: LayoutEngineOptionValue| {
            if updating.get() {
                return;
            }
            ctx.store.edit(&path, |doc| {
                let parts = kdl::split(&path);
                // An option at its default stays out of the file.
                if value == default {
                    kdl::remove(doc, &parts);
                } else {
                    kdl::set_args(doc, &parts, vec![option_value(&value)]);
                }
            });
        }
    };

    let description = option.description.clone().unwrap_or_default();
    let (row, show): (gtk::Widget, ShowValue) = match &option.default {
        LayoutEngineOptionValue::Boolean(_) => {
            let row = adw::SwitchRow::builder()
                .title(&title)
                .subtitle(&description)
                .build();
            row.connect_active_notify(move |row| {
                write(LayoutEngineOptionValue::Boolean(row.is_active()))
            });
            let switch = row.clone();
            let show = move |value: &LayoutEngineOptionValue| {
                if let LayoutEngineOptionValue::Boolean(x) = value {
                    switch.set_active(*x);
                }
            };
            (row.upcast(), Box::new(show))
        }
        LayoutEngineOptionValue::Number(default) => {
            let row = adw::SpinRow::with_range(-1e9, 1e9, 1.);
            row.set_title(&title);
            row.set_subtitle(&description);
            row.set_digits(if default.fract() == 0. { 0 } else { 2 });
            row.connect_value_notify(move |row| {
                write(LayoutEngineOptionValue::Number(row.value()))
            });
            let spin = row.clone();
            let show = move |value: &LayoutEngineOptionValue| {
                if let LayoutEngineOptionValue::Number(x) = value {
                    spin.set_value(*x);
                }
            };
            (row.upcast(), Box::new(show))
        }
        LayoutEngineOptionValue::String(_) => {
            let row = adw::EntryRow::builder()
                .title(&title)
                .show_apply_button(true)
                .build();
            row.connect_apply(move |row| {
                write(LayoutEngineOptionValue::String(row.text().to_string()))
            });
            let entry = row.clone();
            let show = move |value: &LayoutEngineOptionValue| {
                if let LayoutEngineOptionValue::String(x) = value {
                    // Don't replace what someone is typing.
                    if !entry.has_focus() {
                        entry.set_text(x);
                    }
                }
            };
            (row.upcast(), Box::new(show))
        }
    };
    {
        let (store_check, store_reset) = (view.ctx.store.clone(), view.ctx.store.clone());
        let (path_check, path_reset) = (path.clone(), path.clone());
        reset_menu(
            &row,
            move || kdl::get(&store_check.doc(), &kdl::split(&path_check)).is_some(),
            move || {
                store_reset.edit(&path_reset, |doc| {
                    kdl::remove(doc, &kdl::split(&path_reset));
                })
            },
        );
    }
    view.ctx.anchor(&path, &row);
    view.options.borrow_mut().push(OptionRow { show });
    row
}

fn shortcuts_group(view: &Rc<View>, engine: &LayoutEngine) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Shortcuts").build();
    view.ctx.anchor("engine.actions", &group);

    if engine.actions.is_empty() {
        let row = adw::ActionRow::builder()
            .title("No Actions")
            .css_classes(["dim-label"])
            .build();
        group.add(&row);
    }

    let config = view.ctx.store.config();
    let mod_key = config.input.mod_key.unwrap_or(ModKey::Super).to_modifiers();
    for action in &engine.actions {
        let title = action
            .description
            .as_deref()
            .map_or(action.name.as_str(), |d| d.trim_end_matches('.'));
        let row = adw::ActionRow::builder()
            .title(title)
            .tooltip_text(format!("layout-action \"{}\"", action.name))
            .build();

        let keys: Vec<&Key> = config
            .binds
            .0
            .iter()
            .filter(|bind| {
                [bind.press_action(), bind.release_action()]
                    .into_iter()
                    .flatten()
                    .any(|a| matches!(a, Action::LayoutAction(name, _) if *name == action.name))
            })
            .map(|bind| &bind.key)
            .collect();
        if keys.is_empty() {
            let unset = gtk::Label::builder()
                .label("Not Set")
                .valign(gtk::Align::Center)
                .css_classes(["dim-label"])
                .build();
            row.add_suffix(&unset);
        }
        for key in keys {
            row.add_suffix(&shortcut_label(key, mod_key));
        }
        group.add(&row);
    }
    group
}

/// Draws a bind's keys as keycaps, with the compositor's Mod key spelled out.
fn shortcut_label(key: &Key, mod_key: Modifiers) -> gtk::Widget {
    let mut modifiers = key.modifiers;
    if modifiers.contains(Modifiers::COMPOSITOR) {
        modifiers.remove(Modifiers::COMPOSITOR);
        modifiers |= mod_key;
    }
    let mut mask = gdk::ModifierType::empty();
    for (modifier, gdk_modifier) in [
        (Modifiers::CTRL, gdk::ModifierType::CONTROL_MASK),
        (Modifiers::SHIFT, gdk::ModifierType::SHIFT_MASK),
        (Modifiers::ALT, gdk::ModifierType::ALT_MASK),
        (Modifiers::SUPER, gdk::ModifierType::SUPER_MASK),
    ] {
        if modifiers.contains(modifier) {
            mask |= gdk_modifier;
        }
    }

    match key.trigger {
        Trigger::Keysym(keysym) => {
            // SAFETY: GDK key values are X keysyms, which is what xkb gives bento.
            let key = unsafe { gdk::Key::from_glib(keysym.raw()) };
            let label = gtk::ShortcutLabel::new(&gtk::accelerator_name(key, mask));
            label.set_valign(gtk::Align::Center);
            label.upcast()
        }
        // Mouse buttons and scrolling have no keycap; spell them the way the config does.
        trigger => {
            let mut text = String::new();
            for (modifier, name) in [
                (Modifiers::SUPER, "Super"),
                (Modifiers::CTRL, "Ctrl"),
                (Modifiers::ALT, "Alt"),
                (Modifiers::SHIFT, "Shift"),
            ] {
                if modifiers.contains(modifier) {
                    text.push_str(name);
                    text.push('+');
                }
            }
            text.push_str(&format!("{trigger:?}"));
            gtk::Label::builder()
                .label(text)
                .valign(gtk::Align::Center)
                .css_classes(["dim-label"])
                .build()
                .upcast()
        }
    }
}

fn offline_group(view: &Rc<View>, err: &str) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    view.ctx.anchor("engine.options", &group);
    view.ctx.anchor("engine.actions", &group);

    let row = adw::ActionRow::builder()
        .title("Couldn't Reach bento")
        .subtitle(err)
        .build();
    let retry = gtk::Button::builder()
        .label("Try Again")
        .valign(gtk::Align::Center)
        .build();
    let weak = Rc::downgrade(view);
    retry.connect_clicked(move |_| {
        if let Some(view) = weak.upgrade() {
            view.shown.replace(None);
            refresh(&view);
        }
    });
    row.add_suffix(&retry);
    group.add(&row);
    group
}

fn sentence_case(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
