//! The layout engine: which one runs, its options and its actions.
//!
//! Which engine to run comes from the config. What it offers comes from the running engine over
//! IPC, since an engine declares its options and actions itself.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use niri_config::TilingLayout;
use niri_ipc::{LayoutEngine, LayoutEngineOption, LayoutEngineOptionValue, LayoutEngineSource};

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
            "Engine Actions",
            &["layout-action", "binds", "keybinds"],
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
            view.column.append(&actions_group(view, engine));
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

fn choice_group(view: &Rc<View>, running: Option<&LayoutEngine>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    view.ctx.anchor("engine.choice", &group);

    let configured = view.ctx.store.config().tiling.layout.clone();
    let updating = Rc::new(Cell::new(true));

    let mut first: Option<gtk::CheckButton> = None;
    let mut radio = |title: &str, subtitle: Option<String>, active: bool| {
        let check = gtk::CheckButton::builder()
            .valign(gtk::Align::Center)
            .active(active)
            .build();
        if let Some(first) = &first {
            check.set_group(Some(first));
        } else {
            first = Some(check.clone());
        }
        let row = adw::ActionRow::builder()
            .title(title)
            .subtitle(subtitle.unwrap_or_default())
            .activatable_widget(&check)
            .build();
        row.add_prefix(&check);
        group.add(&row);
        (row, check)
    };

    for name in ["bento", "niri"] {
        let active = configured == TilingLayout::BuiltIn(name.to_owned());
        // Only the running engine can describe itself.
        let subtitle = running
            .filter(|e| e.name == name && matches!(e.source, LayoutEngineSource::BuiltIn {}))
            .and_then(|e| e.description.clone());
        let (_, check) = radio(name, subtitle, active);
        let ctx = view.ctx.clone();
        let updating = updating.clone();
        check.connect_toggled(move |check| {
            if check.is_active() && !updating.get() {
                set_layout(&ctx, name.to_owned());
            }
        });
    }

    let file = match &configured {
        TilingLayout::File(path) => Some(path.clone()),
        TilingLayout::BuiltIn(_) => None,
    };
    let (row, check) = radio("Custom", file.as_deref().map(tilde), file.is_some());
    let choose = gtk::Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text("Choose File")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    row.add_suffix(&choose);
    {
        let ctx = view.ctx.clone();
        choose.connect_clicked(move |button| choose_file(&ctx, button));
    }
    {
        let ctx = view.ctx.clone();
        let updating = updating.clone();
        let file = file.clone();
        check.connect_toggled(move |check| {
            if !check.is_active() || updating.get() {
                return;
            }
            // Without a file picked yet, picking one is the only way to switch.
            match &file {
                Some(path) => set_layout(&ctx, tilde(path)),
                None => choose_file(&ctx, check),
            }
        });
    }

    updating.set(false);
    group
}

fn choose_file(ctx: &PageContext, widget: &impl IsA<gtk::Widget>) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Layout engines"));
    filter.add_pattern("*.js");
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title("Choose a Layout Engine")
        .filters(&filters)
        .build();

    let ctx = ctx.clone();
    let window = widget.root().and_downcast::<gtk::Window>();
    dialog.open(
        window.as_ref(),
        None::<&gtk::gio::Cancellable>,
        move |result| match result {
            Ok(file) => match file.path() {
                Some(path) => set_layout(&ctx, tilde(&path)),
                None => ctx.toast("Pick a file on this computer"),
            },
            // Cancelled: put the radio buttons back the way the config has them.
            Err(_) => ctx.store.reload(),
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

fn actions_group(view: &Rc<View>, engine: &LayoutEngine) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Actions")
        .description("Bind with layout-action.")
        .build();
    view.ctx.anchor("engine.actions", &group);

    if engine.actions.is_empty() {
        let row = adw::ActionRow::builder()
            .title("No Actions")
            .css_classes(["dim-label"])
            .build();
        group.add(&row);
    }
    for action in &engine.actions {
        let mut bind = format!("layout-action \"{}\"", action.name);
        for arg in &action.args {
            bind.push_str(&format!(" <{arg}>"));
        }
        let row = adw::ActionRow::builder()
            .title(&action.name)
            .subtitle(action.description.as_deref().unwrap_or_default())
            .build();
        let copy = gtk::Button::builder()
            .icon_name("edit-copy-symbolic")
            .tooltip_text(format!("Copy {bind}"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let ctx = view.ctx.clone();
        copy.connect_clicked(move |button| {
            button.clipboard().set_text(&bind);
            ctx.toast("Copied");
        });
        row.add_suffix(&copy);
        group.add(&row);
    }
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
