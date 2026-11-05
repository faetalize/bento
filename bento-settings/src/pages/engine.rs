//! The layout engine: which one runs, its options and its actions.
//!
//! Which engine to run comes from the config. What it offers comes from the running engine over
//! IPC, since an engine declares its options and actions itself. Each option gets the field of its
//! declared type: a switch, a number, text, a choice, a size, or a list of those.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use niri_config::{Action, TilingLayout};
use niri_ipc::{
    LayoutEngine, LayoutEngineOption, LayoutEngineOptionType as Type,
    LayoutEngineOptionValue as OptionValue, SizeChange,
};

use super::migrate;
use crate::ipc;
use crate::kdl::{self, Value};
use crate::model::Category;
use crate::page::PageContext;
use crate::store::{option_values, tilde, File};

pub fn category() -> Category {
    Category::custom("engine", "Engine", build)
        .icon("ph-layout-symbolic")
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
            &["gravity", "configure", "column width", "tab indicator"],
        )
        .search_item(
            "engine.actions",
            "Engine Shortcuts",
            &["layout-action", "actions", "binds", "keybinds"],
        )
        .search_item(
            "engine.migrate",
            "Migrate from niri",
            &["import", "convert", "niri config", "config.kdl"],
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

/// Shows a new value in an option's row. An option with no value is a size left to the window.
type ShowValue = Box<dyn Fn(&Option<OptionValue>)>;

/// Saves a value someone set in a row.
type Change = Rc<dyn Fn(Option<OptionValue>)>;

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

    let column = view.column.clone();
    crate::page::keep_alive(&column, view);
    column.upcast()
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
    view.column.append(&migrate::group(&view.ctx));
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
    ctx.store.edit("tiling.layout", move |doc| {
        doc.set(&["tiling", "layout"], vec![Value::Str(layout)]);
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
        row.add_suffix(&gtk::Image::from_icon_name("ph-folder-open-symbolic"));
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
    let path = format!("options.{}", option.name);
    let title = sentence_case(&option.name.replace(['-', '_'], " "));
    let description = option.description.clone().unwrap_or_default();

    let change: Change = {
        let ctx = view.ctx.clone();
        let path = path.clone();
        let default = option.default.clone();
        Rc::new(move |value: Option<OptionValue>| {
            let (key, path, default) = (path.clone(), path.clone(), default.clone());
            ctx.store.edit_in(File::Engine, &key, move |doc| {
                let parts = kdl::split(&path);
                // An option at its default stays out of the file.
                if value == default {
                    doc.remove(&parts);
                } else {
                    doc.set(&parts, option_values(&value));
                }
            });
        })
    };

    let digits = digits(&option.default);
    let (row, show): (gtk::Widget, ShowValue) = if option.list {
        let list = ListRow::new(option, &title, &description, digits, change);
        let show = {
            let list = list.clone();
            move |value: &Option<OptionValue>| list.show(value)
        };
        (list.row.clone().upcast(), Box::new(show))
    } else {
        let field = Field {
            r#type: option.r#type,
            title: &title,
            subtitle: &description,
            digits,
            choices: &option.choices,
            // Set while rows show new values, so their change signals don't write them back.
            quiet: view.updating.clone(),
        };
        field.build(change)
    };
    view.ctx.anchor(&path, &row);
    view.options.borrow_mut().push(OptionRow { show });
    row
}

/// How many decimals a number option shows: as many as its default needs, up to two.
fn digits(default: &Option<OptionValue>) -> u32 {
    let fractional =
        |value: &OptionValue| matches!(value, OptionValue::Number(x) if x.fract() != 0.);
    match default {
        Some(OptionValue::List(items)) if items.iter().any(fractional) => 2,
        Some(value) if fractional(value) => 2,
        _ => 0,
    }
}

/// A row for one value of an option's type.
struct Field<'a> {
    r#type: Type,
    title: &'a str,
    subtitle: &'a str,
    digits: u32,
    choices: &'a [String],
    /// While set, the row shows a value without calling `change`.
    quiet: Rc<Cell<bool>>,
}

impl Field<'_> {
    /// The row, and how to show another value in it. `change` gets what someone sets.
    fn build(self, change: Change) -> (gtk::Widget, ShowValue) {
        let Self {
            r#type,
            title,
            subtitle,
            digits,
            choices,
            quiet,
        } = self;
        let notify = move |value: Option<OptionValue>| {
            if !quiet.get() {
                change(value);
            }
        };

        match r#type {
            Type::Boolean => {
                let row = adw::SwitchRow::builder()
                    .title(title)
                    .subtitle(subtitle)
                    .build();
                row.connect_active_notify(move |row| {
                    notify(Some(OptionValue::Boolean(row.is_active())))
                });
                let switch = row.clone();
                let show = move |value: &Option<OptionValue>| {
                    if let Some(OptionValue::Boolean(x)) = value {
                        switch.set_active(*x);
                    }
                };
                (row.upcast(), Box::new(show))
            }
            Type::Number => {
                let step = if digits > 0 { 0.1 } else { 1. };
                let row = adw::SpinRow::with_range(-1e9, 1e9, step);
                row.set_title(title);
                row.set_subtitle(subtitle);
                row.set_digits(digits);
                row.connect_value_notify(move |row| notify(Some(OptionValue::Number(row.value()))));
                let spin = row.clone();
                let show = move |value: &Option<OptionValue>| {
                    if let Some(OptionValue::Number(x)) = value {
                        spin.set_value(*x);
                    }
                };
                (row.upcast(), Box::new(show))
            }
            Type::String => {
                let row = entry_row(title, subtitle);
                row.connect_apply(move |row| {
                    notify(Some(OptionValue::String(row.text().to_string())))
                });
                (row.clone().upcast(), show_text(row))
            }
            Type::Choice => {
                let labels: Vec<&str> = choices.iter().map(String::as_str).collect();
                let row = adw::ComboRow::builder()
                    .title(title)
                    .subtitle(subtitle)
                    .model(&gtk::StringList::new(&labels))
                    .build();
                let names = choices.to_vec();
                row.connect_selected_notify(move |row| {
                    if let Some(choice) = names.get(row.selected() as usize) {
                        notify(Some(OptionValue::String(choice.clone())));
                    }
                });
                let combo = row.clone();
                let names = choices.to_vec();
                let show = move |value: &Option<OptionValue>| {
                    if let Some(OptionValue::String(x)) = value {
                        let idx = names.iter().position(|name| name == x);
                        combo.set_selected(idx.map_or(gtk::INVALID_LIST_POSITION, |i| i as u32));
                    }
                };
                (row.upcast(), Box::new(show))
            }
            Type::Size => {
                let row = entry_row(title, subtitle);
                row.connect_apply(move |row| {
                    let text = row.text().trim().to_owned();
                    // Empty leaves the size to the window.
                    if text.is_empty() {
                        row.remove_css_class("error");
                        notify(None);
                    } else if is_size(&text) {
                        row.remove_css_class("error");
                        notify(Some(OptionValue::String(text)));
                    } else {
                        row.add_css_class("error");
                    }
                });
                (row.clone().upcast(), show_text(row))
            }
        }
    }
}

fn entry_row(title: &str, hint: &str) -> adw::EntryRow {
    let row = adw::EntryRow::builder()
        .title(title)
        .show_apply_button(true)
        .build();
    if !hint.is_empty() {
        row.set_tooltip_text(Some(hint));
    }
    row
}

fn show_text(row: adw::EntryRow) -> ShowValue {
    Box::new(move |value: &Option<OptionValue>| {
        // Don't replace what someone is typing.
        if row.has_focus() {
            return;
        }
        match value {
            Some(OptionValue::String(x)) => row.set_text(x),
            None => row.set_text(""),
            Some(_) => (),
        }
    })
}

/// Whether text is a size to set, like `"50%"` or `"800"`, and not a change to one.
fn is_size(text: &str) -> bool {
    matches!(
        text.parse::<SizeChange>(),
        Ok(SizeChange::SetProportion(_) | SizeChange::SetFixed(_))
    )
}

/// An option that is a list: an expander with a row for each item, and one to add another.
struct ListRow {
    row: adw::ExpanderRow,
    r#type: Type,
    digits: u32,
    choices: Vec<String>,
    change: Change,
    items: RefCell<Vec<OptionValue>>,
    /// The rows of the items and the one that adds an item, to replace when the items change.
    children: RefCell<Vec<gtk::Widget>>,
    /// Set while the rows are being made, so that showing the items doesn't write them back.
    building: Rc<Cell<bool>>,
}

impl ListRow {
    fn new(
        option: &LayoutEngineOption,
        title: &str,
        description: &str,
        digits: u32,
        change: Change,
    ) -> Rc<Self> {
        let row = adw::ExpanderRow::builder().title(title).build();
        if !description.is_empty() {
            row.set_tooltip_text(Some(description));
        }
        let list = Rc::new(Self {
            row,
            r#type: option.r#type,
            digits,
            choices: option.choices.clone(),
            change,
            items: RefCell::new(Vec::new()),
            children: RefCell::new(Vec::new()),
            building: Rc::new(Cell::new(false)),
        });
        list.rebuild();
        list
    }

    fn show(self: &Rc<Self>, value: &Option<OptionValue>) {
        let items = match value {
            Some(OptionValue::List(items)) => items.clone(),
            _ => Vec::new(),
        };
        if *self.items.borrow() != items {
            self.items.replace(items);
            self.rebuild();
        }
    }

    /// Writes the items as they are.
    fn commit(&self) {
        let items = self.items.borrow().clone();
        (self.change)(Some(OptionValue::List(items)));
    }

    fn rebuild(self: &Rc<Self>) {
        self.building.set(true);
        for child in self.children.take() {
            self.row.remove(&child);
        }

        let items = self.items.borrow().clone();
        for (i, item) in items.iter().enumerate() {
            let weak = Rc::downgrade(self);
            let change: Change = {
                let weak = weak.clone();
                Rc::new(move |value: Option<OptionValue>| {
                    let Some(list) = weak.upgrade() else { return };
                    if let Some(value) = value {
                        if let Some(item) = list.items.borrow_mut().get_mut(i) {
                            *item = value;
                        }
                        list.commit();
                    }
                })
            };
            let title = format!("Item {}", i + 1);
            let field = Field {
                r#type: self.r#type,
                title: &title,
                subtitle: "",
                digits: self.digits,
                choices: &self.choices,
                quiet: self.building.clone(),
            };
            let (row, show) = field.build(change);
            show(&Some(item.clone()));

            let remove = gtk::Button::builder()
                .icon_name("list-remove-symbolic")
                .tooltip_text("Remove")
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            remove.connect_clicked(move |_| {
                let Some(list) = weak.upgrade() else { return };
                if i < list.items.borrow().len() {
                    list.items.borrow_mut().remove(i);
                    list.rebuild();
                    list.commit();
                }
            });
            if let Some(row) = row.downcast_ref::<adw::EntryRow>() {
                row.add_suffix(&remove);
            } else if let Some(row) = row.downcast_ref::<adw::ActionRow>() {
                row.add_suffix(&remove);
            }
            self.row.add_row(&row);
            self.children.borrow_mut().push(row);
        }

        let add = adw::ActionRow::builder()
            .title("Add Item")
            .activatable(true)
            .build();
        add.add_prefix(&gtk::Image::from_icon_name("list-add-symbolic"));
        let weak = Rc::downgrade(self);
        add.connect_activated(move |_| {
            let Some(list) = weak.upgrade() else { return };
            let item = list.new_item();
            list.items.borrow_mut().push(item);
            list.rebuild();
            list.commit();
        });
        self.row.add_row(&add);
        self.children.borrow_mut().push(add.upcast());

        let summary: Vec<String> = items.iter().map(summarize).collect();
        self.row.set_subtitle(&if summary.is_empty() {
            String::from("No items")
        } else {
            summary.join(", ")
        });
        self.building.set(false);
    }

    /// What a new item starts as: a value its field accepts.
    fn new_item(&self) -> OptionValue {
        match self.r#type {
            Type::Boolean => OptionValue::Boolean(false),
            Type::Number => OptionValue::Number(0.),
            Type::String => OptionValue::String(String::new()),
            Type::Choice => OptionValue::String(self.choices.first().cloned().unwrap_or_default()),
            Type::Size => OptionValue::String(String::from("50%")),
        }
    }
}

fn summarize(value: &OptionValue) -> String {
    match value {
        OptionValue::Boolean(x) => String::from(if *x { "on" } else { "off" }),
        OptionValue::Number(x) => x.to_string(),
        OptionValue::String(x) => x.clone(),
        OptionValue::List(items) => items.iter().map(summarize).collect::<Vec<_>>().join(", "),
    }
}

/// How many actions the engine has and how many have a shortcut, with a link to the Shortcuts page,
/// which lists them with bento's own.
fn shortcuts_group(view: &Rc<View>, engine: &LayoutEngine) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    view.ctx.anchor("engine.actions", &group);

    let config = view.ctx.store.config();
    let bound = engine
        .actions
        .iter()
        .filter(|action| {
            config.binds.0.iter().any(|bind| {
                [bind.press_action(), bind.release_action()]
                    .into_iter()
                    .flatten()
                    .any(|a| matches!(a, Action::LayoutAction(name, ..) if *name == action.name))
            })
        })
        .count();
    let total = engine.actions.len();
    let row = adw::ActionRow::builder()
        .title("Shortcuts")
        .subtitle(format!(
            "This engine has {total} {}, {bound} with a shortcut",
            if total == 1 { "action" } else { "actions" }
        ))
        .activatable(true)
        .build();
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    row.connect_activated(|row| {
        let _ = row.activate_action("win.open-page", Some(&"shortcuts".to_variant()));
    });
    group.add(&row);
    group
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
