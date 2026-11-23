//! The layout engine: which one runs, its settings and its actions.
//!
//! Which engine to run comes from the config. What it offers comes from the running engine over
//! IPC, since an engine declares its settings and actions itself. Each setting gets the field of
//! its declared type: a switch, a number, text, a choice, a size, a color, a gradient, or a list of
//! those.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, glib};
use niri_config::{Action, Color, TilingLayout};
use niri_ipc::{
    LayoutEngine, LayoutEngineGradient, LayoutEngineOption, LayoutEngineOptionType as Type,
    LayoutEngineOptionValue as OptionValue, SizeChange,
};

use super::{migrate, settings_file};
use crate::ipc;
use crate::kdl::Value;
use crate::model::Category;
use crate::page::PageContext;
use crate::store::tilde;

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
            "windows11",
            "floating",
            "snap",
        ])
        .search_item(
            "engine.choice",
            "Layout Engine",
            &[
                "switch engine",
                "bento",
                "niri",
                "windows11",
                "custom",
                "file",
            ],
        )
        .search_item(
            "engine.settings",
            "Engine Settings",
            &["gravity", "configure", "column width", "tab indicator"],
        )
        .search_item(
            "engine.actions",
            "Engine Binds",
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
    /// The configured layout and the running engine with its setting values left out. The page
    /// is rebuilt only when this changes; the settings' rows keep themselves up to date.
    shown: RefCell<Option<Shape>>,
}

/// The configured engine, its settings file and whether that's there, and the running engine with
/// its setting values left out.
type Shape = (
    TilingLayout,
    Option<(PathBuf, bool)>,
    Result<LayoutEngine, String>,
);

/// Shows a new value in an option's row. An option with no value is a size left to the window.
pub(super) type ShowValue = Box<dyn Fn(&Option<OptionValue>)>;

/// Saves a value someone set in a row.
pub(super) type Change = Rc<dyn Fn(Option<OptionValue>)>;

fn shape(ctx: &PageContext, engine: &Result<LayoutEngine, String>) -> Shape {
    let engine = engine.clone().map(|mut engine| {
        for option in &mut engine.settings {
            option.value = option.default.clone();
        }
        engine
    });
    let layout = ctx.store.config().tiling.layout.clone();
    let settings = match layout {
        TilingLayout::File(_) => ctx.store.engine_path().map(|path| {
            let exists = path.exists();
            (path, exists)
        }),
        TilingLayout::BuiltIn(_) => None,
    };
    (layout, settings, engine)
}

fn build(ctx: &PageContext) -> gtk::Widget {
    let view = Rc::new(View {
        ctx: ctx.clone(),
        column: gtk::Box::new(gtk::Orientation::Vertical, 24),
        shown: RefCell::new(None),
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
    let shape = shape(&view.ctx, &engine);
    if view.shown.borrow().as_ref() == Some(&shape) {
        return;
    }

    while let Some(child) = view.column.first_child() {
        view.column.remove(&child);
    }
    view.column
        .append(&choice_group(view, engine.as_ref().ok()));
    if let Some((path, _)) = &shape.1 {
        if let Some(missing) = settings_file::missing(settings_file::Kind::Engine, path) {
            view.column.append(&missing);
        }
    }
    match &engine {
        Ok(engine) => {
            if let Some(error) = &engine.error {
                view.column.append(&failed_group(error));
            }
            // The engine's settings, but those it puts on another page.
            let settings = crate::pages::placed(&view.ctx, "engine");
            view.ctx.anchor("engine.settings", &settings);
            view.column.append(&settings);
        }
        Err(err) => view.column.append(&offline_group(view, err)),
    }
    view.column.append(&migrate::group(&view.ctx));
    view.shown.replace(Some(shape));
}

fn set_layout(ctx: &PageContext, layout: String) {
    ctx.store.edit("tiling.layout", move |doc| {
        doc.set(&["tiling", "layout"], vec![Value::Str(layout)]);
    });
}

/// Runs the engine file at `script`, with its settings file at `settings`.
fn set_layout_file(ctx: &PageContext, script: &Path, settings: &Path) {
    let (script, settings) = (tilde(script), tilde(settings));
    ctx.store.edit("tiling.layout", move |doc| {
        let properties = vec![(String::from("settings"), Value::Str(settings))];
        doc.set_node(&["tiling", "layout"], vec![Value::Str(script)], properties);
    });
}

/// Engines that ship with bento, as the config names them and as the app shows them.
const BUILT_IN: &[(&str, &str)] = &[
    ("bento", "Bento"),
    ("niri", "Niri"),
    ("windows11", "Windows 11"),
];

/// The engine chosen, its file, and the running engine's binds summed up last.
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
        .use_markup(false)
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
            .use_markup(false)
            .title("File")
            .subtitle(tilde(&path))
            .activatable(true)
            .build();
        row.add_suffix(&gtk::Image::from_icon_name("ph-folder-open-symbolic"));
        let weak = Rc::downgrade(view);
        row.connect_activated(move |row| {
            if let Some(view) = weak.upgrade() {
                choose_file(&view, row);
            }
        });
        group.add(&row);

        // Its settings file, which picks another, the engine staying.
        let settings = view.ctx.store.engine_path();
        let row = settings_file::row(settings.as_deref());
        let weak = Rc::downgrade(view);
        row.connect_activated(move |row| {
            let Some(view) = weak.upgrade() else { return };
            let ctx = view.ctx.clone();
            let script = path.clone();
            let done = move |settings: PathBuf| set_layout_file(&ctx, &script, &settings);
            settings_file::ask(
                &view.ctx,
                row,
                settings_file::Kind::Engine,
                &path,
                done,
                || (),
            );
        });
        group.add(&row);
    }
    if let Some(engine) = running {
        group.add(&binds_row(view, engine));
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
            // Cancelled: rebuild, so the dropdown shows the configured engine again.
            let cancelled = {
                let view = Rc::downgrade(&view);
                move || {
                    if let Some(view) = view.upgrade() {
                        view.shown.replace(None);
                        refresh(&view);
                    }
                }
            };
            let Some(path) = result.ok().and_then(|file| file.path()) else {
                cancelled();
                return;
            };
            // Then where its settings come from; the engine changes only with them.
            let ctx = view.ctx.clone();
            let script = path.clone();
            let done = move |settings: PathBuf| set_layout_file(&ctx, &script, &settings);
            settings_file::ask(
                &view.ctx,
                &view.column,
                settings_file::Kind::Engine,
                &path,
                done,
                cancelled,
            );
        },
    );
}

/// The row for an option of an engine, or a setting of a shell, which are typed alike: the field
/// of its type, or a list of them. `change` gets what someone sets, unless `quiet` is set while
/// the row shows a new value.
pub(super) fn option_widget(
    option: &LayoutEngineOption,
    quiet: Rc<Cell<bool>>,
    change: Change,
) -> (gtk::Widget, ShowValue) {
    let title = crate::page::title_from(&option.name);
    let description = option.description.clone().unwrap_or_default();
    let digits = digits(&option.default);
    if option.list {
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
            quiet,
            item: false,
        };
        field.build(change)
    }
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
    /// Whether it's an item of a list, which a folder is the title of.
    item: bool,
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
            item,
        } = self;
        let notify = move |value: Option<OptionValue>| {
            if !quiet.get() {
                change(value);
            }
        };

        match r#type {
            Type::Boolean => {
                let row = adw::SwitchRow::builder()
                    .use_markup(false)
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
                    .use_markup(false)
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
            Type::Color => {
                let button = color_button();
                let row = adw::ActionRow::builder()
                    .use_markup(false)
                    .title(title)
                    .subtitle(subtitle)
                    .build();
                row.add_suffix(&button);
                row.set_activatable_widget(Some(&button));
                button.connect_rgba_notify(move |button| {
                    notify(Some(OptionValue::String(hex(&button.rgba()))))
                });
                let show = move |value: &Option<OptionValue>| {
                    if let Some(OptionValue::String(x)) = value {
                        show_color(&button, x);
                    }
                };
                (row.upcast(), Box::new(show))
            }
            Type::Gradient => gradient_row(title, subtitle, notify),
            Type::Folder => folder_row(title, subtitle, !item, notify),
        }
    }
}

/// A button that picks a color with its opacity.
fn color_button() -> gtk::ColorDialogButton {
    let dialog = gtk::ColorDialog::builder().with_alpha(true).build();
    let button = gtk::ColorDialogButton::new(Some(dialog));
    button.set_valign(gtk::Align::Center);
    button
}

/// A color as the `#rrggbb` or `#rrggbbaa` that engines take.
fn hex(rgba: &gdk::RGBA) -> String {
    let byte = |x: f32| (x.clamp(0., 1.) * 255.).round() as u8;
    let (r, g, b, a) = (
        byte(rgba.red()),
        byte(rgba.green()),
        byte(rgba.blue()),
        byte(rgba.alpha()),
    );
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

/// Shows a color written `#rgb`, `#rrggbb` or `#rrggbbaa` in a button.
fn show_color(button: &gtk::ColorDialogButton, text: &str) {
    if let Ok(color) = Color::from_hex(text) {
        let [r, g, b, a] = color.to_array_unpremul();
        button.set_rgba(&gdk::RGBA::new(r, g, b, a));
    }
}

/// A gradient: a row with a button for each of its colors, the angle it runs at, and a button to
/// take it away. Where it spans and the color space it's mixed in are kept as they're written.
fn gradient_row(
    title: &str,
    subtitle: &str,
    notify: impl Fn(Option<OptionValue>) + 'static,
) -> (gtk::Widget, ShowValue) {
    let from = color_button();
    from.set_tooltip_text(Some("Start Color"));
    let to = color_button();
    to.set_tooltip_text(Some("End Color"));
    let angle = gtk::SpinButton::with_range(-360., 360., 1.);
    angle.set_valign(gtk::Align::Center);
    angle.set_tooltip_text(Some("Angle in Degrees"));
    let clear = gtk::Button::builder()
        .icon_name("edit-clear-symbolic")
        .tooltip_text("No Gradient")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();

    let row = adw::ActionRow::builder()
        .use_markup(false)
        .title(title)
        .subtitle(subtitle)
        .build();
    for widget in [
        clear.upcast_ref::<gtk::Widget>(),
        from.upcast_ref(),
        to.upcast_ref(),
    ] {
        row.add_suffix(widget);
    }
    row.add_suffix(&angle);

    // The properties that the row has no field for, from the gradient it shows.
    let kept = Rc::new(RefCell::new((None::<String>, None::<String>)));
    let notify = Rc::new(notify);
    let write: Rc<dyn Fn()> = {
        let (from, to, angle, kept, notify) = (
            from.clone(),
            to.clone(),
            angle.clone(),
            kept.clone(),
            notify.clone(),
        );
        Rc::new(move || {
            let (relative_to, color_space) = kept.borrow().clone();
            notify(Some(OptionValue::Gradient(LayoutEngineGradient {
                from: hex(&from.rgba()),
                to: hex(&to.rgba()),
                angle: Some(angle.value()),
                relative_to,
                color_space,
            })));
        })
    };
    {
        let write = write.clone();
        from.connect_rgba_notify(move |_| write());
    }
    {
        let write = write.clone();
        to.connect_rgba_notify(move |_| write());
    }
    angle.connect_value_changed(move |_| write());
    clear.connect_clicked(move |_| notify(None));

    let show = move |value: &Option<OptionValue>| match value {
        Some(OptionValue::Gradient(gradient)) => {
            show_color(&from, &gradient.from);
            show_color(&to, &gradient.to);
            // niri's default direction, top to bottom.
            angle.set_value(gradient.angle.unwrap_or(180.));
            kept.replace((gradient.relative_to.clone(), gradient.color_space.clone()));
            clear.set_sensitive(true);
        }
        _ => {
            // What a new gradient starts from.
            show_color(&from, "#000000");
            show_color(&to, "#ffffff");
            angle.set_value(180.);
            kept.replace((None, None));
            clear.set_sensitive(false);
        }
    };
    (row.upcast(), Box::new(show))
}

/// A folder's row: its path, and a button that picks another with the folder chooser.
/// The path shows under the title, unless the row is an item of a list, which it's the title of.
fn folder_row(
    title: &str,
    hint: &str,
    path_under: bool,
    notify: impl Fn(Option<OptionValue>) + 'static,
) -> (gtk::Widget, ShowValue) {
    let row = adw::ActionRow::builder()
        .use_markup(false)
        .title(title)
        .build();
    if !hint.is_empty() {
        row.set_tooltip_text(Some(hint));
    }
    let choose = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Choose Folder…")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let notify = Rc::new(notify);
    choose.connect_clicked(move |button| {
        let notify = notify.clone();
        choose_folder(button.upcast_ref(), move |folder| {
            notify(Some(OptionValue::String(folder)))
        });
    });
    row.add_suffix(&choose);
    row.set_activatable_widget(Some(&choose));
    let shown = row.clone();
    let show = move |value: &Option<OptionValue>| match value {
        _ if !path_under => (),
        Some(OptionValue::String(path)) if !path.is_empty() => shown.set_subtitle(path),
        _ => shown.set_subtitle("None"),
    };
    (row.upcast(), Box::new(show))
}

/// Asks for a folder with the folder chooser, over `from`'s window, and gives it to `picked` as
/// it's written: with `~` for the home folder.
fn choose_folder(from: &gtk::Widget, picked: impl Fn(String) + 'static) {
    let dialog = gtk::FileDialog::builder()
        .title("Choose a Folder")
        .modal(true)
        .build();
    let window = from.root().and_downcast::<gtk::Window>();
    dialog.select_folder(
        window.as_ref(),
        gtk::gio::Cancellable::NONE,
        move |folder| {
            if let Some(path) = folder.ok().and_then(|folder| folder.path()) {
                picked(written_folder(&path));
            }
        },
    );
}

/// A folder's path as a setting writes it, with `~` for the home folder.
fn written_folder(path: &std::path::Path) -> String {
    let home = glib::home_dir();
    match path.strip_prefix(&home) {
        Ok(inside) if inside.as_os_str().is_empty() => String::from("~"),
        Ok(inside) => format!("~/{}", inside.display()),
        Err(_) => path.display().to_string(),
    }
}

fn entry_row(title: &str, hint: &str) -> adw::EntryRow {
    let row = adw::EntryRow::builder()
        .use_markup(false)
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
        if crate::page::typing_in(&row) {
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
        let row = adw::ExpanderRow::builder()
            .use_markup(false)
            .title(title)
            .build();
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
        let weak = Rc::downgrade(&list);
        list.row.connect_expanded_notify(move |_| {
            if let Some(list) = weak.upgrade() {
                list.show_summary();
            }
        });
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
                let row = self.row.clone();
                Rc::new(move |value: Option<OptionValue>| {
                    let Some(list) = weak.upgrade() else { return };
                    if let Some(value) = value {
                        // A folder that's listed already isn't listed twice.
                        let elsewhere = |(j, item): (usize, &OptionValue)| j != i && *item == value;
                        if list.r#type == Type::Folder
                            && list.items.borrow().iter().enumerate().any(elsewhere)
                        {
                            toast(
                                row.upcast_ref(),
                                &format!("“{}” is already in the list", summarize(&value)),
                            );
                            list.rebuild();
                            return;
                        }
                        if let Some(item) = list.items.borrow_mut().get_mut(i) {
                            *item = value;
                        }
                        list.commit();
                        // A folder's row is titled with it. Other rows stay, as one may be typed
                        // in.
                        if list.r#type == Type::Folder {
                            list.rebuild();
                        }
                    }
                })
            };
            // A folder is its own title.
            let title = match (self.r#type, item) {
                (Type::Folder, OptionValue::String(path)) => path.clone(),
                _ => format!("Item {}", i + 1),
            };
            let field = Field {
                r#type: self.r#type,
                title: &title,
                subtitle: "",
                digits: self.digits,
                choices: &self.choices,
                quiet: self.building.clone(),
                item: true,
            };
            let (row, show) = field.build(change);
            show(&Some(item.clone()));

            let remove = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
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

        let folders = self.r#type == Type::Folder;
        let add = adw::ActionRow::builder()
            .title(if folders { "Add Folder…" } else { "Add Item" })
            .activatable(true)
            .build();
        add.add_prefix(&gtk::Image::from_icon_name("list-add-symbolic"));
        let weak = Rc::downgrade(self);
        add.connect_activated(move |add| {
            let Some(list) = weak.upgrade() else { return };
            if !folders {
                let item = list.new_item();
                list.items.borrow_mut().push(item);
                list.rebuild();
                list.commit();
                return;
            }
            // A folder is picked first, and added once.
            let weak = Rc::downgrade(&list);
            let from = add.clone().upcast::<gtk::Widget>();
            choose_folder(&from.clone(), move |folder| {
                let Some(list) = weak.upgrade() else { return };
                let item = OptionValue::String(folder.clone());
                if list.items.borrow().contains(&item) {
                    toast(&from, &format!("“{folder}” is already in the list"));
                    return;
                }
                list.items.borrow_mut().push(item);
                list.rebuild();
                list.commit();
            });
        });
        self.row.add_row(&add);
        self.children.borrow_mut().push(add.upcast());

        self.show_summary();
        self.building.set(false);
    }

    /// What it holds, under its title, while it's closed: open, its items show it.
    fn show_summary(&self) {
        let summary: Vec<String> = self.items.borrow().iter().map(summarize).collect();
        self.row.set_subtitle(&if self.row.is_expanded() {
            String::new()
        } else if summary.is_empty() {
            String::from("No Items")
        } else {
            summary.join(", ")
        });
    }

    /// What a new item starts as: a value its field accepts.
    fn new_item(&self) -> OptionValue {
        match self.r#type {
            Type::Boolean => OptionValue::Boolean(false),
            Type::Number => OptionValue::Number(0.),
            Type::String => OptionValue::String(String::new()),
            Type::Choice => OptionValue::String(self.choices.first().cloned().unwrap_or_default()),
            Type::Size => OptionValue::String(String::from("50%")),
            Type::Color => OptionValue::String(String::from("#ffffff")),
            Type::Folder => OptionValue::String(String::from("~")),
            // A gradient can't be a list: it's written with the properties of its one node.
            Type::Gradient => OptionValue::Gradient(LayoutEngineGradient {
                from: String::from("#000000"),
                to: String::from("#ffffff"),
                angle: None,
                relative_to: None,
                color_space: None,
            }),
        }
    }
}

/// Shows `message` in the toasts of the window `from` is in.
fn toast(from: &gtk::Widget, message: &str) {
    if let Some(overlay) = from.ancestor(adw::ToastOverlay::static_type()) {
        overlay
            .downcast::<adw::ToastOverlay>()
            .unwrap()
            .add_toast(adw::Toast::new(message));
    }
}

fn summarize(value: &OptionValue) -> String {
    match value {
        OptionValue::Boolean(x) => String::from(if *x { "on" } else { "off" }),
        OptionValue::Number(x) => x.to_string(),
        OptionValue::String(x) => x.clone(),
        OptionValue::Gradient(x) => format!("{} to {}", x.from, x.to),
        OptionValue::List(items) => items.iter().map(summarize).collect::<Vec<_>>().join(", "),
    }
}

/// How many actions the engine has and how many have a bind, with a link to the Binds page's
/// Engine tab, which lists them.
fn binds_row(view: &Rc<View>, engine: &LayoutEngine) -> adw::ActionRow {
    // With its default binds and the user's, as bento settles which runs each chord.
    let mut defaults = niri_config::bind_set::bento_defaults().to_vec();
    defaults.extend(niri_config::bind_set::declared_binds(
        &engine.actions,
        niri_config::bind_set::engine_action,
    ));
    let resolved = {
        let (user, unbound) = view.ctx.store.user_binds();
        niri_config::bind_set::resolve(&user, &unbound, &defaults)
    };
    let bound = engine
        .actions
        .iter()
        .filter(|action| {
            resolved.in_use.iter().any(|bind| {
                matches!(
                    niri_config::bind_set::action_of(bind),
                    Some(Action::LayoutAction(name, ..)) if *name == action.name
                )
            })
        })
        .count();
    let total = engine.actions.len();
    let row = adw::ActionRow::builder()
        .use_markup(false)
        .title("Binds")
        .subtitle(format!(
            "This engine has {total} {}, {bound} with a bind",
            if total == 1 { "action" } else { "actions" }
        ))
        .activatable(true)
        .build();
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    row.connect_activated(|row| {
        let _ = row.activate_action("win.open-page", Some(&"binds#binds.engine".to_variant()));
    });
    view.ctx.anchor("engine.actions", &row);
    row
}

/// Says why the configured engine isn't running, when bento runs the fallback one instead.
fn failed_group(error: &str) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    let row = adw::ActionRow::builder()
        .use_markup(false)
        .title("Engine Failed")
        .subtitle(format!(
            "bento runs a fallback engine until the engine's file changes.\n{error}"
        ))
        .subtitle_selectable(true)
        .build();
    group.add(&row);
    group
}

fn offline_group(view: &Rc<View>, err: &str) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    view.ctx.anchor("engine.settings", &group);
    view.ctx.anchor("engine.actions", &group);

    let row = adw::ActionRow::builder()
        .use_markup(false)
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
