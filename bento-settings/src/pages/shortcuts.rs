//! Shortcuts: every bind in one list, bento's actions and the running engine's together, and a
//! dialog to add or change one.
//!
//! Binds to an action the running engine declares go in the engine's own file, and every other
//! bind in the main config; the page doesn't show which. Binds from included files are shown but
//! can't be changed here, since those files often belong to something else, like a shell that
//! writes them.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib::translate::FromGlib;
use gtk::{gdk, gio, glib};
use niri_config::input::ModKey;
use niri_config::{Action, Key, Modifiers, TilingLayout, Trigger};
use niri_ipc::{LayoutEngineAction, LayoutEngineActionArgType as ArgType};

use crate::ipc;
use crate::kdl::{ActionNode, BindNode, Document, Value};
use crate::model::Category;
use crate::page::PageContext;
use crate::store::{File, Store};

pub fn category() -> Category {
    Category::custom("shortcuts", "Shortcuts", build)
        .icon("ph-command-symbolic")
        .keywords(&["binds", "keybinds", "hotkeys", "keyboard", "keys"])
        .search_item("shortcuts.list", "Shortcuts", &["add shortcut", "reset"])
}

/// An action to bind: bento's own, or one the running engine declares.
#[derive(Clone)]
struct Choice {
    action: LayoutEngineAction,
    engine: bool,
}

impl Choice {
    fn title(&self) -> String {
        title(&self.action)
    }
}

fn title(action: &LayoutEngineAction) -> String {
    let description = action.description.as_deref().unwrap_or(&action.name);
    description.trim_end_matches('.').to_owned()
}

/// bento's actions, then the running engine's.
fn choices() -> Vec<Choice> {
    let native = niri_config::actions::native_actions().into_iter();
    let mut choices: Vec<Choice> = native
        .map(|action| Choice {
            action,
            engine: false,
        })
        .collect();
    if let Ok(engine) = ipc::layout_engine() {
        choices.extend(engine.actions.into_iter().map(|action| Choice {
            action,
            engine: true,
        }));
    }
    choices
}

/// A bind as the page shows it.
struct Entry {
    key: Key,
    /// Where it's written: the main config, the engine's file, or `None` for an included file.
    file: Option<File>,
    /// The included file it comes from.
    include: Option<PathBuf>,
    /// How its file writes it, for binds in the main config or the engine's file.
    node: Option<BindNode>,
    /// The action's name, from the file or the parsed config.
    action: String,
}

fn entries(store: &Store) -> Vec<Entry> {
    let config = store.config();
    let main = store.config_path().to_owned();
    let engine = store.engine_path();
    let main_binds = read_binds(&main);
    let engine_binds = read_binds(&engine);

    let mut entries = Vec::new();
    for bind in &config.binds.0 {
        let path = config.bind_files.get(&bind.key).map(Path::to_owned);
        let (file, nodes) = match &path {
            Some(path) if *path == main => (Some(File::Main), &main_binds),
            Some(path) if *path == engine => (Some(File::Engine), &engine_binds),
            _ => (None, &Vec::new()),
        };
        let node = nodes
            .iter()
            .find(|node| parse_key(&node.key) == Some(bind.key));
        let action = match node.and_then(|node| node.action.as_ref()) {
            Some(action) => action.name.clone(),
            None => bind.press_action().map_or_else(String::new, action_name),
        };
        entries.push(Entry {
            key: bind.key,
            file,
            include: if file.is_none() { path } else { None },
            node: node.cloned(),
            action,
        });
    }
    entries
}

fn read_binds(path: &Path) -> Vec<BindNode> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    Document::parse(text)
        .map(|doc| doc.binds())
        .unwrap_or_default()
}

fn parse_key(text: &str) -> Option<Key> {
    text.parse().ok()
}

/// Name of a parsed action as configs write it, like `FocusMonitorLeft` as `focus-monitor-left`.
fn action_name(action: &Action) -> String {
    if let Action::LayoutAction(name, _) = action {
        return name.clone();
    }
    // `Debug` starts with the variant name.
    let debug = format!("{action:?}");
    let variant = debug.split(['(', ' ', '{']).next().unwrap_or_default();
    let mut name = String::new();
    for (i, c) in variant.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                name.push('-');
            }
            name.push(c.to_ascii_lowercase());
        } else {
            name.push(c);
        }
    }
    name
}

struct View {
    ctx: PageContext,
    group: adw::PreferencesGroup,
    search: gtk::SearchEntry,
    /// Each bind's row, with the words search matches it on.
    rows: RefCell<Vec<(gtk::Widget, String)>>,
    /// Shown when the search matches nothing.
    empty: adw::ActionRow,
}

fn build(ctx: &PageContext) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder().title("Shortcuts").build();
    ctx.anchor("shortcuts.list", &group);
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search shortcuts")
        .build();
    let empty = adw::ActionRow::builder()
        .title("No Matching Shortcuts")
        .css_classes(["dim-label"])
        .visible(false)
        .build();
    group.add(&empty);

    let view = Rc::new(View {
        ctx: ctx.clone(),
        group: group.clone(),
        search: search.clone(),
        rows: RefCell::new(Vec::new()),
        empty,
    });
    let weak = Rc::downgrade(&view);
    search.connect_search_changed(move |_| {
        if let Some(view) = weak.upgrade() {
            filter(&view);
        }
    });

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let reset = gtk::Button::builder()
        .label("Reset All…")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let add = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Add Shortcut")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    buttons.append(&reset);
    buttons.append(&add);
    group.set_header_suffix(Some(&buttons));

    let ctx = view.ctx.clone();
    add.connect_clicked(move |button| open(&ctx, button, Edit::New));
    let ctx = view.ctx.clone();
    reset.connect_clicked(move |button| reset_all(&ctx, button));

    refresh(&view);
    let weak = Rc::downgrade(&view);
    view.ctx.store.subscribe(move || {
        if let Some(view) = weak.upgrade() {
            refresh(&view);
        }
    });

    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.append(&search);
    column.append(&group);
    column.upcast()
}

/// Shows the rows that have every word of the search.
fn filter(view: &View) {
    let query = view.search.text().to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    let mut any = false;
    for (row, text) in view.rows.borrow().iter() {
        let visible = words.iter().all(|word| text.contains(word));
        row.set_visible(visible);
        any |= visible;
    }
    view.empty.set_visible(!any);
}

fn refresh(view: &Rc<View>) {
    for (row, _) in view.rows.take() {
        view.group.remove(&row);
    }
    let choices = choices();
    let mod_key = mod_key(&view.ctx.store);
    let mut rows = Vec::new();
    for entry in entries(&view.ctx.store) {
        let choice = choices.iter().find(|c| c.action.name == entry.action);
        let title = choice.map_or_else(|| entry.action.clone(), Choice::title);
        let row = adw::ActionRow::builder().title(&title).build();
        let args = entry.node.as_ref().and_then(|node| node.action.as_ref());
        let args = args.map(ActionNode::arguments).unwrap_or_default();
        let subtitle = match (&entry.include, &entry.node) {
            (Some(path), _) => format!("From {}", tilde(path)),
            (None, Some(node)) if node.action.is_none() => {
                String::from("Press and release actions, set in the config file")
            }
            _ => args,
        };
        if !subtitle.is_empty() {
            row.set_subtitle(&subtitle);
        }
        row.add_suffix(&shortcut_label(&entry.key, mod_key));

        let editable =
            entry.file.is_some() && entry.node.as_ref().is_some_and(|n| n.action.is_some());
        if editable {
            row.set_activatable(true);
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            let ctx = view.ctx.clone();
            let key = entry.key;
            row.connect_activated(move |row| open(&ctx, row, Edit::Existing(key)));
        }
        view.group.add(&row);
        let text = format!("{title} {subtitle} {}", key_label(&entry.key, mod_key));
        rows.push((row.upcast(), text.to_lowercase()));
    }
    view.rows.replace(rows);
    filter(view);
}

fn mod_key(store: &Store) -> Modifiers {
    let config = store.config();
    config.input.mod_key.unwrap_or(ModKey::Super).to_modifiers()
}

fn tilde(path: &Path) -> String {
    crate::store::tilde(path)
}

/// What the dialog changes.
pub enum Edit {
    New,
    /// The shortcut on this key.
    Existing(Key),
}

/// Opens the dialog to add a shortcut or change one.
pub fn open(ctx: &PageContext, parent: &impl IsA<gtk::Widget>, edit: Edit) {
    let choices = Rc::new(choices());
    let existing = match &edit {
        Edit::Existing(key) => entries(&ctx.store).into_iter().find(|e| e.key == *key),
        Edit::New => None,
    };
    let node = existing.as_ref().and_then(|e| e.node.clone());

    let dialog = adw::Dialog::builder()
        .title(if existing.is_some() {
            "Edit Shortcut"
        } else {
            "Add Shortcut"
        })
        .content_width(480)
        .build();
    let page = adw::PreferencesPage::new();

    // The key.
    let key = Rc::new(RefCell::new(node.as_ref().map(|n| n.key.clone())));
    let key_group = adw::PreferencesGroup::new();
    let key_row = adw::ActionRow::builder().title("Shortcut").build();
    let key_view = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let record = gtk::Button::builder()
        .label("Record…")
        .valign(gtk::Align::Center)
        .build();
    key_row.add_suffix(&key_view);
    key_row.add_suffix(&record);
    key_group.add(&key_row);
    page.add(&key_group);
    let mod_key = mod_key(&ctx.store);
    let show_key = {
        let key = key.clone();
        let key_view = key_view.clone();
        move || {
            while let Some(child) = key_view.first_child() {
                key_view.remove(&child);
            }
            match key.borrow().as_deref().and_then(parse_key) {
                Some(parsed) => key_view.append(&shortcut_label(&parsed, mod_key)),
                None => key_view.append(&dim_label("Not Set")),
            }
        }
    };
    show_key();
    let recording = Rc::new(Recording::default());
    {
        let (key, show_key, recording) = (key.clone(), show_key.clone(), recording.clone());
        let key_view = key_view.clone();
        let dialog_widget = dialog.clone();
        record.connect_clicked(move |_| {
            while let Some(child) = key_view.first_child() {
                key_view.remove(&child);
            }
            key_view.append(&dim_label("Press a shortcut…"));
            let (key, show_key) = (key.clone(), show_key.clone());
            recording.start(&dialog_widget, mod_key, move |recorded| {
                if let Some(recorded) = recorded {
                    key.replace(Some(recorded));
                }
                show_key();
            });
        });
    }

    // The action.
    let action_group = adw::PreferencesGroup::new();
    let titles: Vec<String> = choices.iter().map(Choice::title).collect();
    let model = gtk::StringList::new(&titles.iter().map(String::as_str).collect::<Vec<_>>());
    let action_row = adw::ComboRow::builder()
        .title("Action")
        .model(&model)
        .enable_search(true)
        .build();
    action_row.set_expression(Some(gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        None::<gtk::Expression>,
        "string",
    )));
    action_group.add(&action_row);
    page.add(&action_group);

    let picked = node
        .as_ref()
        .and_then(|n| n.action.as_ref())
        .map(|a| a.name.clone());
    if let Some(idx) = picked.and_then(|name| choices.iter().position(|c| c.action.name == name)) {
        action_row.set_selected(idx as u32);
    }

    // The action's arguments, rebuilt when the action changes.
    let args_group = adw::PreferencesGroup::builder().title("Arguments").build();
    page.add(&args_group);
    let fields: Rc<RefCell<Vec<Field>>> = Rc::default();
    let existing_action = node.as_ref().and_then(|n| n.action.clone());
    let show_args = {
        let (choices, fields, args_group) = (choices.clone(), fields.clone(), args_group.clone());
        let action_row = action_row.clone();
        move || {
            for field in fields.take() {
                args_group.remove(&field.row);
            }
            let Some(choice) = choices.get(action_row.selected() as usize) else {
                return;
            };
            // Keep the values the bind has, when it's the same action.
            let current = existing_action
                .as_ref()
                .filter(|action| action.name == choice.action.name);
            let mut new = Vec::new();
            // Where each positional argument is in the bind; properties have names instead.
            let mut position = 0;
            for arg in &choice.action.args {
                let field = Field::new(arg, current, position);
                if !arg.property {
                    position += 1;
                }
                args_group.add(&field.row);
                new.push(field);
            }
            args_group.set_visible(!new.is_empty());
            fields.replace(new);
        }
    };
    show_args();
    action_row.connect_selected_notify(move |_| show_args());

    // Settings most people never need.
    let advanced_group = adw::PreferencesGroup::new();
    let advanced = Advanced::new(node.as_ref());
    advanced_group.add(&advanced.expander);
    page.add(&advanced_group);

    // Removing it and putting back the default.
    if let Some(existing) = &existing {
        let group = adw::PreferencesGroup::new();
        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .halign(gtk::Align::Center)
            .build();
        let action = existing.node.as_ref().and_then(|n| n.action.as_ref());
        let default = action.and_then(|action| default_bind(&ctx.store, existing.key, action));
        if let (Some((default, file)), Some(from)) = (default, existing.file) {
            let reset = gtk::Button::builder()
                .label("Reset to Default")
                .css_classes(["pill"])
                .build();
            let (ctx, dialog, key) = (ctx.clone(), dialog.clone(), existing.key);
            reset.connect_clicked(move |_| {
                let Some(default_key) = parse_key(&default.key) else {
                    return;
                };
                let from = Some((key, from));
                save_bind(&ctx, &dialog, default_key, default.clone(), file, from);
            });
            buttons.append(&reset);
        }
        let delete = gtk::Button::builder()
            .label("Remove Shortcut")
            .css_classes(["pill", "destructive-action"])
            .build();
        let (ctx_, dialog_, key, file) = (ctx.clone(), dialog.clone(), existing.key, existing.file);
        delete.connect_clicked(move |_| {
            if let Some(file) = file {
                remove(&ctx_.store, file, key);
            }
            dialog_.close();
        });
        buttons.append(&delete);
        group.add(&buttons);
        page.add(&group);
    }

    // The header: cancel and save.
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::builder()
        .label(if existing.is_some() { "Save" } else { "Add" })
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&cancel);
    header.pack_end(&save);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));

    let dialog_ = dialog.clone();
    cancel.connect_clicked(move |_| {
        dialog_.close();
    });
    let recording_ = recording.clone();
    dialog.connect_closed(move |_| recording_.stop());

    let ctx = ctx.clone();
    let dialog_ = dialog.clone();
    save.connect_clicked(move |_| {
        let Some(key_text) = key.borrow().clone() else {
            ctx.toast("Record a shortcut first");
            return;
        };
        let Some(parsed) = parse_key(&key_text) else {
            ctx.toast("That shortcut can't be bound");
            return;
        };
        let Some(choice) = choices.get(action_row.selected() as usize) else {
            ctx.toast("Pick an action first");
            return;
        };
        let action = match action_node(&choice.action, &fields.borrow()) {
            Ok(action) => action,
            Err(message) => {
                ctx.toast(&message);
                return;
            }
        };
        let bind = BindNode {
            key: key_text,
            properties: advanced.properties(),
            action: Some(action),
        };
        let file = if choice.engine {
            File::Engine
        } else {
            File::Main
        };
        let from = existing
            .as_ref()
            .and_then(|e| e.file.map(|file| (e.key, file)));
        save_bind(&ctx, &dialog_, parsed, bind, file, from);
    });

    dialog.present(Some(parent));
}

/// Saves a bind, asking first when its key is already taken by another one.
fn save_bind(
    ctx: &PageContext,
    dialog: &adw::Dialog,
    key: Key,
    bind: BindNode,
    file: File,
    from: Option<(Key, File)>,
) {
    let taken = entries(&ctx.store)
        .into_iter()
        .find(|e| e.key.conflicts_with(&key) && from.is_none_or(|(old, _)| old != e.key));

    let commit = {
        let (ctx, dialog) = (ctx.clone(), dialog.clone());
        move |replace: Option<(Key, File)>| {
            if let Some((taken, taken_file)) = replace {
                remove(&ctx.store, taken_file, taken);
            }
            match from {
                // In the same file, the bind changes where it is.
                Some((old, old_file)) if old_file == file => {
                    write(&ctx.store, file, old, bind.clone());
                }
                Some((old, old_file)) => {
                    remove(&ctx.store, old_file, old);
                    write(&ctx.store, file, key, bind.clone());
                }
                None => write(&ctx.store, file, key, bind.clone()),
            }
            dialog.close();
        }
    };

    let Some(taken) = taken else {
        commit(None);
        return;
    };
    let choices = choices();
    let title = choices
        .iter()
        .find(|c| c.action.name == taken.action)
        .map_or_else(|| taken.action.clone(), Choice::title);
    let Some(taken_file) = taken.file else {
        let file = taken.include.as_deref().map(tilde).unwrap_or_default();
        let alert = adw::AlertDialog::new(
            Some("Shortcut Taken"),
            Some(&format!(
                "It's set to “{title}” in {file}, which can't be changed here."
            )),
        );
        alert.add_response("close", "Close");
        alert.present(Some(dialog));
        return;
    };
    let alert = adw::AlertDialog::new(
        Some("Replace Shortcut?"),
        Some(&format!("This shortcut is set to “{title}”.")),
    );
    alert.add_response("cancel", "Cancel");
    alert.add_response("replace", "Replace");
    alert.set_response_appearance("replace", adw::ResponseAppearance::Destructive);
    alert.set_default_response(Some("cancel"));
    alert.choose(Some(dialog), gio::Cancellable::NONE, move |response| {
        if response == "replace" {
            commit(Some((taken.key, taken_file)));
        }
    });
}

/// Writes `bind` in place of the one on `key`, or adds it.
fn write(store: &Store, file: File, key: Key, bind: BindNode) {
    let id = format!("binds.{file:?}.{}", bind.key);
    store.edit_in(file, &id, move |doc| doc.set_bind(&key, &bind));
}

fn remove(store: &Store, file: File, key: Key) {
    let id = format!("binds.{file:?}.remove.{key:?}");
    store.edit_in(file, &id, move |doc| {
        doc.remove_bind(&key);
    });
}

/// The shipped bind for `action`, and the file it belongs in, when the bind on `key` differs from
/// it. Like GNOME's shortcuts, resetting puts the action back on its default key.
fn default_bind(store: &Store, key: Key, action: &ActionNode) -> Option<(BindNode, File)> {
    let main = Document::parse(niri_config::DEFAULT_CONFIG.to_owned()).ok()?;
    let main = main.binds().into_iter().map(|bind| (bind, File::Main));
    let engine = engine_defaults(store)
        .map(|doc| doc.binds())
        .unwrap_or_default();
    let engine = engine.into_iter().map(|bind| (bind, File::Engine));
    let defaults: Vec<_> = main
        .chain(engine)
        .filter(|(bind, _)| bind.action.as_ref() == Some(action))
        .collect();
    let current = entries(store);
    // An action can have several default keys, like Mod+Left and Mod+H.
    if let Some(on_key) = defaults
        .iter()
        .find(|(b, _)| parse_key(&b.key) == Some(key))
    {
        // Same key: only reset what differs, like its properties.
        let node = current
            .iter()
            .find(|e| e.key == key)
            .and_then(|e| e.node.as_ref());
        return (node != Some(&on_key.0)).then(|| on_key.clone());
    }
    let free = |(bind, _): &&(BindNode, File)| {
        let default_key = parse_key(&bind.key);
        !current.iter().any(|e| Some(e.key) == default_key)
    };
    defaults.iter().find(free).or(defaults.first()).cloned()
}

/// The running built-in engine's default file.
fn engine_defaults(store: &Store) -> Option<Document> {
    let layout = store.config().tiling.layout.clone();
    let TilingLayout::BuiltIn(name) = layout else {
        return None;
    };
    let text = niri_config::default_engine_config(&name)?;
    Document::parse(text.to_owned()).ok()
}

fn reset_all(ctx: &PageContext, parent: &gtk::Button) {
    let alert = adw::AlertDialog::new(
        Some("Reset All Shortcuts?"),
        Some(
            "Every shortcut goes back to the one bento ships, in the config and in the layout \
             engine's file. Shortcuts from included files stay as they are.",
        ),
    );
    alert.add_response("cancel", "Cancel");
    alert.add_response("reset", "Reset All");
    alert.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
    alert.set_default_response(Some("cancel"));
    let ctx = ctx.clone();
    alert.choose(Some(parent), gio::Cancellable::NONE, move |response| {
        if response != "reset" {
            return;
        }
        let main = Document::parse(niri_config::DEFAULT_CONFIG.to_owned()).ok();
        if let Some(section) = main.and_then(|doc| doc.binds_section()) {
            ctx.store.edit_in(File::Main, "binds.reset", move |doc| {
                doc.replace_binds(&section)
            });
        }
        if let Some(section) = engine_defaults(&ctx.store).and_then(|doc| doc.binds_section()) {
            ctx.store.edit_in(File::Engine, "binds.reset", move |doc| {
                doc.replace_binds(&section)
            });
        }
    });
}

/// A row for one of an action's arguments.
struct Field {
    row: gtk::Widget,
    name: String,
    property: bool,
    /// The values to write, or `None` to leave it out.
    value: Box<dyn Fn() -> Result<Option<Vec<Value>>, String>>,
}

impl Field {
    fn new(
        arg: &niri_ipc::LayoutEngineActionArg,
        current: Option<&ActionNode>,
        index: usize,
    ) -> Self {
        let name = arg.name.clone();
        let title = sentence_case(&name.replace(['-', '_'], " "));
        let subtitle = arg.description.clone().unwrap_or_default();
        // The value the bind has now, else the declared default.
        let current: Vec<Value> = match current {
            Some(action) if arg.property => {
                let found = action.properties.iter().find(|(n, _)| *n == name);
                found.map(|(_, v)| vec![v.clone()]).unwrap_or_default()
            }
            Some(action) if arg.r#type == Some(ArgType::Command) => {
                action.args.get(index..).unwrap_or_default().to_vec()
            }
            Some(action) => action.args.get(index).cloned().into_iter().collect(),
            None => Vec::new(),
        };
        let default = arg.default.as_ref().map(crate::store::option_value);
        let first = current.first().cloned().or(default.clone());
        let required = arg.default.is_none() && !arg.optional;

        let (row, value): (gtk::Widget, Box<dyn Fn() -> _>) = match arg.r#type {
            Some(ArgType::Boolean) => {
                let row = adw::SwitchRow::builder()
                    .title(&title)
                    .subtitle(&subtitle)
                    .active(matches!(first, Some(Value::Bool(true))))
                    .build();
                let r = row.clone();
                let default = default.clone();
                (
                    row.upcast(),
                    Box::new(move || {
                        let value = Value::Bool(r.is_active());
                        // A value at its default stays out of the config.
                        Ok((Some(&value) != default.as_ref()).then(|| vec![value]))
                    }),
                )
            }
            Some(ArgType::Number) => {
                let row = adw::SpinRow::with_range(-100_000., 100_000., 1.);
                row.set_title(&title);
                row.set_subtitle(&subtitle);
                row.set_value(match first {
                    Some(Value::Int(x)) => x as f64,
                    Some(Value::Float(x)) => x,
                    _ => 1.,
                });
                let r = row.clone();
                let default = default.clone();
                (
                    row.upcast(),
                    Box::new(move || {
                        let value = number(r.value());
                        Ok((Some(&value) != default.as_ref()).then(|| vec![value]))
                    }),
                )
            }
            Some(ArgType::Choice) => {
                let choices: Vec<&str> = arg.choices.iter().map(String::as_str).collect();
                let row = adw::ComboRow::builder()
                    .title(&title)
                    .subtitle(&subtitle)
                    .model(&gtk::StringList::new(&choices))
                    .build();
                if let Some(Value::Str(current)) = &first {
                    if let Some(idx) = arg.choices.iter().position(|c| c == current) {
                        row.set_selected(idx as u32);
                    }
                }
                let r = row.clone();
                let choices = arg.choices.clone();
                (
                    row.upcast(),
                    Box::new(move || {
                        let choice = choices.get(r.selected() as usize).cloned();
                        Ok(choice.map(|c| vec![Value::Str(c)]))
                    }),
                )
            }
            // Text: strings, sizes, workspaces, monitors and commands.
            r#type => {
                let text = match r#type {
                    Some(ArgType::Command) => {
                        let words: Vec<String> = current.iter().map(text_of).collect();
                        let quoted = words.iter().map(glib::shell_quote);
                        let quoted: Vec<String> =
                            quoted.map(|w| w.to_string_lossy().into_owned()).collect();
                        // Plain words don't need the quotes the shell would add.
                        let plain = words
                            .iter()
                            .all(|w| !w.is_empty() && !w.contains([' ', '\'', '"']));
                        if plain {
                            words.join(" ")
                        } else {
                            quoted.join(" ")
                        }
                    }
                    _ => first.as_ref().map(text_of).unwrap_or_default(),
                };
                let row = adw::EntryRow::builder().title(&title).text(&text).build();
                if !subtitle.is_empty() {
                    row.set_tooltip_text(Some(&subtitle));
                }
                let r = row.clone();
                let title_ = title.clone();
                (
                    row.upcast(),
                    Box::new(move || {
                        let text = r.text().trim().to_owned();
                        if text.is_empty() {
                            return if required {
                                Err(format!("Fill in “{title_}”"))
                            } else {
                                Ok(None)
                            };
                        }
                        Ok(Some(match r#type {
                            Some(ArgType::Command) => {
                                let words = glib::shell_parse_argv(&text)
                                    .map_err(|_| format!("Check the quotes in “{title_}”"))?;
                                let words =
                                    words.into_iter().map(|w| w.to_string_lossy().into_owned());
                                words.map(Value::Str).collect()
                            }
                            Some(ArgType::Workspace) => match text.parse::<i64>() {
                                Ok(index) => vec![Value::Int(index)],
                                Err(_) => vec![Value::Str(text)],
                            },
                            _ => vec![Value::Str(text)],
                        }))
                    }),
                )
            }
        };
        Self {
            row,
            name,
            property: arg.property,
            value,
        }
    }
}

fn number(value: f64) -> Value {
    if value.fract() == 0. {
        Value::Int(value as i64)
    } else {
        Value::Float(value)
    }
}

fn text_of(value: &Value) -> String {
    match value {
        Value::Str(x) => x.clone(),
        Value::Int(x) => x.to_string(),
        Value::Float(x) => x.to_string(),
        Value::Bool(x) => x.to_string(),
    }
}

/// The action as a bind writes it, from the fields.
fn action_node(action: &LayoutEngineAction, fields: &[Field]) -> Result<ActionNode, String> {
    let mut node = ActionNode {
        name: action.name.clone(),
        args: Vec::new(),
        properties: Vec::new(),
    };
    // Positional arguments left out can only be at the end.
    let mut positional: Vec<Option<Vec<Value>>> = Vec::new();
    for field in fields {
        let value = (field.value)()?;
        if field.property {
            if let Some(mut values) = value {
                node.properties.push((field.name.clone(), values.remove(0)));
            }
        } else {
            positional.push(value);
        }
    }
    while positional.last().is_some_and(Option::is_none) {
        positional.pop();
    }
    let declared = action.args.iter().filter(|arg| !arg.property);
    for (value, arg) in positional.into_iter().zip(declared) {
        match value {
            Some(values) => node.args.extend(values),
            // A default left out before a given argument has to be written after all.
            None => match &arg.default {
                Some(default) => node.args.push(crate::store::option_value(default)),
                None => return Err(format!("Fill in “{}”", sentence_case(&arg.name))),
            },
        }
    }
    Ok(node)
}

/// A bind's own settings, under "Advanced".
struct Advanced {
    expander: adw::ExpanderRow,
    repeat: adw::SwitchRow,
    cooldown: adw::SpinRow,
    when_locked: adw::SwitchRow,
    inhibiting: adw::SwitchRow,
    title: adw::EntryRow,
    /// Properties the dialog doesn't show, kept as they are.
    other: Vec<(String, Value)>,
}

impl Advanced {
    fn new(node: Option<&BindNode>) -> Self {
        let props: Vec<(String, Value)> = node.map(|n| n.properties.clone()).unwrap_or_default();
        let get = |name: &str| {
            props
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        };
        let flag = |name: &str, default: bool| match get(name) {
            Some(Value::Bool(x)) => x,
            _ => default,
        };

        let repeat = adw::SwitchRow::builder()
            .title("Repeat While Held")
            .active(flag("repeat", true))
            .build();
        let cooldown = adw::SpinRow::with_range(0., 10_000., 50.);
        cooldown.set_title("Cooldown");
        cooldown.set_subtitle("Milliseconds before it can run again, for scroll wheels");
        if let Some(Value::Int(ms)) = get("cooldown-ms") {
            cooldown.set_value(ms as f64);
        }
        let when_locked = adw::SwitchRow::builder()
            .title("Works When Locked")
            .active(flag("allow-when-locked", false))
            .build();
        let inhibiting = adw::SwitchRow::builder()
            .title("Apps Can Take It")
            .subtitle("When an app asks for all keys, like a remote desktop")
            .active(flag("allow-inhibiting", true))
            .build();
        let title = adw::EntryRow::builder()
            .title("Title in the Shortcut Overlay")
            .build();
        if let Some(Value::Str(text)) = get("hotkey-overlay-title") {
            title.set_text(&text);
        }

        let expander = adw::ExpanderRow::builder().title("Advanced").build();
        for row in [
            repeat.upcast_ref::<gtk::Widget>(),
            cooldown.upcast_ref(),
            when_locked.upcast_ref(),
            inhibiting.upcast_ref(),
            title.upcast_ref(),
        ] {
            expander.add_row(row);
        }

        let shown = [
            "repeat",
            "cooldown-ms",
            "allow-when-locked",
            "allow-inhibiting",
            "hotkey-overlay-title",
        ];
        let other = props
            .iter()
            .filter(|(n, _)| !shown.contains(&n.as_str()))
            .cloned()
            .collect();
        Self {
            expander,
            repeat,
            cooldown,
            when_locked,
            inhibiting,
            title,
            other,
        }
    }

    /// The properties to write: only the ones away from their defaults, and the others as they
    /// were.
    fn properties(&self) -> Vec<(String, Value)> {
        let mut out = Vec::new();
        if !self.repeat.is_active() {
            out.push((String::from("repeat"), Value::Bool(false)));
        }
        let cooldown = self.cooldown.value() as i64;
        if cooldown > 0 {
            out.push((String::from("cooldown-ms"), Value::Int(cooldown)));
        }
        if self.when_locked.is_active() {
            out.push((String::from("allow-when-locked"), Value::Bool(true)));
        }
        if !self.inhibiting.is_active() {
            out.push((String::from("allow-inhibiting"), Value::Bool(false)));
        }
        let title = self.title.text();
        if !title.is_empty() {
            out.push((
                String::from("hotkey-overlay-title"),
                Value::Str(title.into()),
            ));
        }
        out.extend(self.other.iter().cloned());
        out
    }
}

/// Recording a shortcut: keys, and clicks and scrolls with modifiers held. While it records, the
/// window inhibits bento's shortcuts, so keys bento has bound reach it instead of running.
#[derive(Default)]
struct Recording {
    controllers: RefCell<Vec<(gtk::Widget, gtk::EventController)>>,
    toplevel: RefCell<Option<gdk::Toplevel>>,
    active: Cell<bool>,
}

impl Recording {
    fn start(
        self: &Rc<Self>,
        widget: &impl IsA<gtk::Widget>,
        mod_key: Modifiers,
        done: impl Fn(Option<String>) + 'static,
    ) {
        self.stop();
        self.active.set(true);
        let done: Rc<dyn Fn(Option<String>)> = Rc::new(done);
        let widget = widget.upcast_ref::<gtk::Widget>();

        let finish = {
            let this = Rc::downgrade(self);
            let done = done.clone();
            move |key: Option<String>| {
                if let Some(this) = this.upgrade() {
                    if !this.active.get() {
                        return;
                    }
                    // Let the event that finished it go before tearing the controllers down.
                    let this = this.clone();
                    glib::idle_add_local_once(move || this.stop());
                }
                done(key);
            }
        };

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let finish_ = finish.clone();
        keys.connect_key_pressed(move |_, keyval, _, state| {
            if keyval == gdk::Key::Escape && modifiers(state).is_empty() {
                finish_(None);
                return glib::Propagation::Stop;
            }
            let Some(name) = keyval
                .to_lower()
                .name()
                .filter(|_| !is_modifier_key(keyval))
            else {
                return glib::Propagation::Stop;
            };
            // Letters are written in capitals, like `Mod+T`.
            let name = if name.len() == 1 {
                name.to_uppercase()
            } else {
                name.to_string()
            };
            finish_(Some(key_text(modifiers(state), &name, mod_key)));
            glib::Propagation::Stop
        });

        let scroll = gtk::EventControllerScroll::new(
            gtk::EventControllerScrollFlags::BOTH_AXES | gtk::EventControllerScrollFlags::DISCRETE,
        );
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let finish_ = finish.clone();
        scroll.connect_scroll(move |controller, dx, dy| {
            let mods = modifiers(controller.current_event_state());
            if mods.is_empty() || (dx == 0. && dy == 0.) {
                return glib::Propagation::Proceed;
            }
            let touchpad = controller
                .current_event_device()
                .is_some_and(|d| d.source() == gdk::InputSource::Touchpad);
            let source = if touchpad {
                "TouchpadScroll"
            } else {
                "WheelScroll"
            };
            let direction = if dy.abs() >= dx.abs() {
                if dy > 0. {
                    "Down"
                } else {
                    "Up"
                }
            } else if dx > 0. {
                "Right"
            } else {
                "Left"
            };
            finish_(Some(key_text(
                mods,
                &format!("{source}{direction}"),
                mod_key,
            )));
            glib::Propagation::Stop
        });

        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let finish_ = finish.clone();
        click.connect_pressed(move |gesture, _, _, _| {
            // Without modifiers it's a click on the dialog, not a shortcut.
            let mods = modifiers(gesture.current_event_state());
            if mods.is_empty() {
                return;
            }
            let button = match gesture.current_button() {
                1 => "MouseLeft",
                2 => "MouseMiddle",
                3 => "MouseRight",
                8 => "MouseBack",
                9 => "MouseForward",
                _ => return,
            };
            gesture.set_state(gtk::EventSequenceState::Claimed);
            finish_(Some(key_text(mods, button, mod_key)));
        });

        let mut controllers = self.controllers.borrow_mut();
        for controller in [
            keys.upcast::<gtk::EventController>(),
            scroll.upcast(),
            click.upcast(),
        ] {
            widget.add_controller(controller.clone());
            controllers.push((widget.clone(), controller));
        }

        // The dialog sits inside the app's window, so that's the one to inhibit shortcuts on.
        let toplevel = widget
            .native()
            .and_then(|native| native.surface())
            .and_then(|surface| surface.downcast::<gdk::Toplevel>().ok());
        if let Some(toplevel) = &toplevel {
            toplevel.inhibit_system_shortcuts(None::<&gdk::Event>);
        }
        self.toplevel.replace(toplevel);
    }

    fn stop(&self) {
        self.active.set(false);
        for (widget, controller) in self.controllers.take() {
            widget.remove_controller(&controller);
        }
        if let Some(toplevel) = self.toplevel.take() {
            toplevel.restore_system_shortcuts();
        }
    }
}

fn modifiers(state: gdk::ModifierType) -> Modifiers {
    let mut mods = Modifiers::empty();
    for (gdk_modifier, modifier) in [
        (gdk::ModifierType::CONTROL_MASK, Modifiers::CTRL),
        (gdk::ModifierType::SHIFT_MASK, Modifiers::SHIFT),
        (gdk::ModifierType::ALT_MASK, Modifiers::ALT),
        (gdk::ModifierType::SUPER_MASK, Modifiers::SUPER),
    ] {
        if state.contains(gdk_modifier) {
            mods |= modifier;
        }
    }
    mods
}

fn is_modifier_key(keyval: gdk::Key) -> bool {
    use gdk::Key;
    [
        Key::Shift_L,
        Key::Shift_R,
        Key::Control_L,
        Key::Control_R,
        Key::Alt_L,
        Key::Alt_R,
        Key::Super_L,
        Key::Super_R,
        Key::Meta_L,
        Key::Meta_R,
        Key::ISO_Level3_Shift,
        Key::Caps_Lock,
    ]
    .contains(&keyval)
}

/// A key as configs write it, like `Mod+Shift+T`, with the compositor's Mod key as `Mod`.
fn key_text(mut mods: Modifiers, trigger: &str, mod_key: Modifiers) -> String {
    let mut text = String::new();
    if mods.contains(mod_key) {
        mods.remove(mod_key);
        text.push_str("Mod+");
    }
    for (modifier, name) in [
        (Modifiers::SUPER, "Super"),
        (Modifiers::CTRL, "Ctrl"),
        (Modifiers::ALT, "Alt"),
        (Modifiers::SHIFT, "Shift"),
    ] {
        if mods.contains(modifier) {
            text.push_str(name);
            text.push('+');
        }
    }
    text.push_str(trigger);
    text
}

fn dim_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .valign(gtk::Align::Center)
        .css_classes(["dim-label"])
        .build()
}

fn sentence_case(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The modifiers a key shows, with the compositor's Mod key spelled out, and the same as GDK's.
fn shown_modifiers(key: &Key, mod_key: Modifiers) -> (Modifiers, gdk::ModifierType) {
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
    (modifiers, mask)
}

fn gdk_key(trigger: Trigger) -> Option<gdk::Key> {
    match trigger {
        // SAFETY: GDK key values are X keysyms, which is what xkb gives bento.
        Trigger::Keysym(keysym) => Some(unsafe { gdk::Key::from_glib(keysym.raw()) }),
        _ => None,
    }
}

/// A key as text, like `Shift+Super+T`.
fn key_label(key: &Key, mod_key: Modifiers) -> String {
    let (modifiers, mask) = shown_modifiers(key, mod_key);
    if let Some(gdk_key) = gdk_key(key.trigger) {
        return gtk::accelerator_get_label(gdk_key, mask).to_string();
    }
    // Mouse buttons and scrolling have no keycap; spell them the way the config does.
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
    text.push_str(&format!("{:?}", key.trigger));
    text
}

/// Draws a bind's keys as keycaps, with the compositor's Mod key spelled out.
fn shortcut_label(key: &Key, mod_key: Modifiers) -> gtk::Widget {
    match gdk_key(key.trigger) {
        Some(gdk_key) => {
            let (_, mask) = shown_modifiers(key, mod_key);
            let label = gtk::ShortcutLabel::new(&gtk::accelerator_name(gdk_key, mask));
            label.set_valign(gtk::Align::Center);
            label.upcast()
        }
        None => dim_label(&key_label(key, mod_key)).upcast(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn close_window() -> ActionNode {
        ActionNode {
            name: String::from("close-window"),
            args: Vec::new(),
            properties: Vec::new(),
        }
    }

    #[test]
    fn resetting_puts_the_action_back_on_its_default_key() {
        let dir = std::env::temp_dir().join(format!("bento-settings-reset-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.kdl");

        let moved = "binds {\n    Mod+Ctrl+Y repeat=false { close-window; }\n}\n";
        fs::write(&path, moved).unwrap();
        let store = Store::load_from(path.clone());
        let key = parse_key("Mod+Ctrl+Y").unwrap();
        let (bind, file) = default_bind(&store, key, &close_window()).unwrap();
        assert_eq!((bind.key.as_str(), file), ("Mod+Q", File::Main));

        // Already on its default key, as shipped: nothing to reset.
        let shipped = "binds {\n    Mod+Q repeat=false { close-window; }\n}\n";
        fs::write(&path, shipped).unwrap();
        let store = Store::load_from(path);
        let key = parse_key("Mod+Q").unwrap();
        assert_eq!(default_bind(&store, key, &close_window()), None);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keys_are_written_like_configs_write_them() {
        let mods = Modifiers::SUPER | Modifiers::SHIFT;
        assert_eq!(key_text(mods, "T", Modifiers::SUPER), "Mod+Shift+T");
        assert_eq!(key_text(mods, "T", Modifiers::ALT), "Super+Shift+T");
        let mods = Modifiers::SUPER | Modifiers::CTRL;
        assert_eq!(
            key_text(mods, "WheelScrollDown", Modifiers::SUPER),
            "Mod+Ctrl+WheelScrollDown"
        );
    }
}
