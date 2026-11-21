//! Binds: every action, bento's, the running engine's and the running shell's, in tabs by who
//! declares them, each with its binds, and dialogs to change an action's binds or to add one.
//!
//! An action is what a bind runs, with its arguments; a bind is a chord that runs it, with
//! settings of its own. A chord runs at most one action, and an action has any number of binds.
//! Defaults stay with whoever declares the action; what the page writes is only what differs:
//! binds of the user's own, in the main config, or the engine's file for the engine's actions, and
//! `unbind` for an action turned off. What the page lists and what a change writes is
//! `bind_list`'s. Binds from included files are shown but can't be changed here, since those files
//! often belong to something else, like a shell that writes them.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib::translate::FromGlib;
use gtk::{gdk, gio, glib};
use niri_config::bind_set::Declarer;
use niri_config::binds::BindFile;
use niri_config::input::ModKey;
use niri_config::{Key, Modifiers, Trigger};
use niri_ipc::{LayoutEngineAction, LayoutEngineActionArgType as ArgType};

use super::bind_list::{
    self, binds_of, general_group, Change, Entry, Section, Source, Sources, Tab, SHELL_ACTION,
};
use crate::ipc;
use crate::kdl::{ActionNode, BindNode, Value};
use crate::model::Category;
use crate::page::PageContext;
use crate::store::{File, Store};

pub fn category() -> Category {
    Category::custom("binds", "Binds", build)
        .icon("ph-command-symbolic")
        .keywords(&[
            "shortcuts",
            "keybinds",
            "hotkeys",
            "keyboard",
            "keys",
            "chords",
        ])
        .search_item("binds.list", "Binds", &["add bind", "reset", "actions"])
}

/// The anchor of `tab`, which links to the page open it on.
pub fn tab_anchor(tab: Tab) -> String {
    format!("binds.{}", tab.name())
}

/// What the page lists from, as it is now.
fn sources(store: &Store) -> Sources {
    let config = store.config();
    let engine = ipc::layout_engine().ok().map(|e| (e.name, e.actions));
    let shell = ipc::shell()
        .map(|s| {
            s.actions
                .into_iter()
                .map(|a| (a.module, a.action))
                .collect()
        })
        .unwrap_or_default();
    let engine_path = store.engine_path();
    let shell_path = store.shell_path();
    let read_if = |path: &Option<std::path::PathBuf>| {
        path.as_ref()
            .filter(|path| path.exists())
            .map(|path| bind_list::read(path))
    };
    let shell_file = read_if(&shell_path);
    let shell_settings = store.shell_settings();
    Sources {
        native: niri_config::actions::native_actions(),
        engine,
        shell,
        main: bind_list::read(store.config_path()),
        engine_file: read_if(&engine_path),
        shell_file,
        main_path: store.config_path().to_owned(),
        engine_path,
        binds: config.binds.0.clone(),
        bind_files: config.bind_files.clone(),
        include_files: include_files(&config.bind_files, store),
        unbound: config.unbound.clone(),
        shell_binds: shell_settings.binds.into_iter().map(|(_, b)| b).collect(),
        shell_unbound: shell_settings.unbound.into_iter().map(|(_, a)| a).collect(),
    }
}

/// The files the config includes binds from, but the config and the engine's file, as written.
fn include_files(
    bind_files: &niri_config::BindFiles,
    store: &Store,
) -> Vec<(std::path::PathBuf, crate::kdl::Document)> {
    let mut files: Vec<(std::path::PathBuf, crate::kdl::Document)> = Vec::new();
    for (_, path) in &bind_files.0 {
        let ours = path == store.config_path() || store.engine_path().as_ref() == Some(path);
        if ours || files.iter().any(|(file, _)| file == path) {
            continue;
        }
        files.push((path.clone(), bind_list::read(path)));
    }
    files
}

/// The title of a section of the page.
fn section_title(section: &Section) -> String {
    match section {
        Section::General(group) => String::from(*group),
        Section::Engine(name) => format!("{} Engine", crate::page::title_from(name)),
        Section::Module(name) => crate::page::title_from(name),
    }
}

/// Who declares an action, as a note says it.
fn whose(declarer: Declarer) -> &'static str {
    match declarer {
        Declarer::Bento => "bento's",
        Declarer::Engine => "the engine's",
        Declarer::Shell => "the shell's",
    }
}

/// Writes `change` to `entry`: what the user wrote for it comes out of every file, and the change
/// goes in its own, the main config, the engine's file or the shell's.
fn apply(store: &Store, entry: &Entry, change: Change) {
    let id = format!("binds.{}", entry.node.text());
    let files = [
        (File::Main, BindFile::Main, true),
        (
            File::Engine,
            BindFile::Engine,
            store.engine_path().is_some_and(|p| p.exists()),
        ),
        (
            File::ShellSettings,
            BindFile::Shell,
            store.shell_path().is_some_and(|p| p.exists()),
        ),
    ];
    for (file, bind_file, exists) in files {
        if exists || entry.file() == file {
            let (one, change) = (entry.clone(), change.clone());
            store.edit_in(file, &id, move |doc| {
                bind_list::apply(doc, &one, &change, bind_file)
            });
        }
    }
}

/// Where a dialog's way to show its binds again is kept, for what it shows to call.
type ShowSlot = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

/// A tab of the page: its toggle with the count of actions it shows, and its groups.
struct TabView {
    tab: Tab,
    toggle: adw::Toggle,
    count: gtk::Label,
    /// The sections' groups, made again on each refresh.
    sections: gtk::Box,
    /// Shown when the tab lists nothing, or the search matches nothing in it.
    empty: adw::PreferencesGroup,
    empty_row: adw::ActionRow,
}

struct View {
    ctx: PageContext,
    search: gtk::SearchEntry,
    tabs: Vec<TabView>,
    /// What the page lists now, in the rows' order, for a row to open its dialog by its index.
    entries: RefCell<Vec<Entry>>,
    /// Each action's row, its section's group and its tab, with the words search matches it on.
    rows: RefCell<Vec<(gtk::Widget, adw::PreferencesGroup, Tab, String)>>,
}

fn build(ctx: &PageContext) -> gtk::Widget {
    let header = adw::PreferencesGroup::builder().title("Binds").build();
    ctx.anchor("binds.list", &header);
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search binds")
        .build();

    // The tabs, as a toggle group over a stack, each toggle with its count.
    let switcher = adw::ToggleGroup::builder().homogeneous(true).build();
    let stack = gtk::Stack::builder()
        .vhomogeneous(false)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    let mut tabs = Vec::new();
    for tab in Tab::ALL {
        let count = gtk::Label::builder()
            .css_classes(["dim-label", "numeric"])
            .build();
        let label = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .halign(gtk::Align::Center)
            .build();
        label.append(&gtk::Label::new(Some(tab.title())));
        label.append(&count);
        let toggle = adw::Toggle::builder()
            .name(tab.name())
            .child(&label)
            .build();
        switcher.add(toggle.clone());

        let empty_row = adw::ActionRow::builder()
            .title("No Matching Binds")
            .css_classes(["dim-label"])
            .build();
        let empty = adw::PreferencesGroup::new();
        empty.add(&empty_row);
        let sections = gtk::Box::new(gtk::Orientation::Vertical, 24);
        let page = gtk::Box::new(gtk::Orientation::Vertical, 24);
        page.append(&empty);
        page.append(&sections);
        stack.add_named(&page, Some(tab.name()));
        ctx.anchor(&tab_anchor(tab), &page);
        tabs.push(TabView {
            tab,
            toggle,
            count,
            sections,
            empty,
            empty_row,
        });
    }
    switcher
        .bind_property("active-name", &stack, "visible-child-name")
        .bidirectional()
        .sync_create()
        .build();
    switcher.set_active_name(Some(Tab::General.name()));

    let view = Rc::new(View {
        ctx: ctx.clone(),
        search: search.clone(),
        tabs,
        entries: RefCell::new(Vec::new()),
        rows: RefCell::new(Vec::new()),
    });
    // A row opens its entry's dialog through `binds.open`, with the entry's index.
    let actions = gio::SimpleActionGroup::new();
    let open = gio::SimpleAction::new("open", Some(glib::VariantTy::UINT32));
    let weak = Rc::downgrade(&view);
    open.connect_activate(move |_, index| {
        let Some(view) = weak.upgrade() else { return };
        let index = index.and_then(|i| i.get::<u32>()).unwrap_or(u32::MAX) as usize;
        let entry = view.entries.borrow().get(index).cloned();
        if let Some(entry) = entry {
            open_entry(&view.ctx, &view.search, entry);
        }
    });
    actions.add_action(&open);
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
        .tooltip_text("Add Bind")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    buttons.append(&reset);
    buttons.append(&add);
    header.set_header_suffix(Some(&buttons));

    let ctx = view.ctx.clone();
    add.connect_clicked(move |button| open_new(&ctx, button));
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
    column.append(&header);
    column.append(&search);
    column.append(&switcher);
    column.append(&stack);
    column.insert_action_group("binds", Some(&actions));
    crate::page::keep_alive(&column, view);
    column.upcast()
}

/// Shows the rows that have every word of the search, in every tab, and the sections that have
/// any. Each tab's count follows what it shows, and a tab with nothing is greyed out; the tab
/// shown stays.
fn filter(view: &View) {
    let query = view.search.text().to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    let mut shown: Vec<adw::PreferencesGroup> = Vec::new();
    let mut counts = [0usize; 3];
    for (row, group, tab, text) in view.rows.borrow().iter() {
        let visible = words.iter().all(|word| text.contains(word));
        row.set_visible(visible);
        if visible {
            counts[Tab::ALL.iter().position(|t| t == tab).unwrap_or(0)] += 1;
            if !shown.contains(group) {
                shown.push(group.clone());
            }
        }
    }
    for (tab, count) in view.tabs.iter().zip(counts) {
        let mut child = tab.sections.first_child();
        while let Some(widget) = child {
            let group = widget.clone().downcast::<adw::PreferencesGroup>().ok();
            widget.set_visible(group.is_some_and(|g| shown.contains(&g)));
            child = widget.next_sibling();
        }
        tab.count.set_label(&count.to_string());
        tab.toggle.set_enabled(count > 0);
        tab.empty.set_visible(count == 0);
        tab.empty_row.set_title(if !words.is_empty() {
            "No Matching Binds"
        } else {
            match tab.tab {
                Tab::General => "No Actions",
                Tab::Engine => "The Engine Isn't Running",
                Tab::Shell => "No Shell Actions",
            }
        });
    }
}

fn refresh(view: &Rc<View>) {
    for tab in &view.tabs {
        while let Some(child) = tab.sections.first_child() {
            tab.sections.remove(&child);
        }
    }
    let mod_key = mod_key(&view.ctx.store);
    let entries = bind_list::entries(&sources(&view.ctx.store));
    let mut rows = Vec::new();
    let mut groups: Vec<(Section, adw::PreferencesGroup)> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let tab = entry.section.tab();
        let group = match groups.iter().find(|(s, _)| *s == entry.section) {
            Some((_, group)) => group.clone(),
            None => {
                let group = adw::PreferencesGroup::builder()
                    .title(section_title(&entry.section))
                    .build();
                if let Some(tab_view) = view.tabs.iter().find(|t| t.tab == tab) {
                    tab_view.sections.append(&group);
                }
                groups.push((entry.section.clone(), group.clone()));
                group
            }
        };

        let title = entry.title();
        let mut notes = Vec::new();
        let args = entry.arguments();
        if !args.is_empty() {
            notes.push(args);
        }
        for key in &entry.binds {
            if let Source::Include(path) = &key.source {
                notes.push(format!("From {}", crate::store::tilde(path)));
            }
            if let Some(by) = key.taken {
                notes.push(format!(
                    "{} not in use: {} bind takes it",
                    key_label(&key.key, mod_key),
                    whose(by)
                ));
            }
        }
        let chords: Vec<gtk::Widget> = entry
            .binds
            .iter()
            .map(|key| {
                let label = chord_label(&key.key, mod_key);
                if key.taken.is_some() {
                    label.add_css_class("dim-label");
                }
                label
            })
            .collect();
        let row = entry_row(&title, &notes.join("\n"), chords);
        if entry.editable() {
            row.set_activatable(true);
            row.set_action_name(Some("binds.open"));
            row.set_action_target_value(Some(&(i as u32).to_variant()));
            let arrow = gtk::Image::from_icon_name("go-next-symbolic");
            arrow.set_valign(gtk::Align::Center);
            if let Some(content) = row.child().and_downcast::<gtk::Box>() {
                content.append(&arrow);
            }
        }
        group.add(&row);

        let key_words: Vec<String> = entry
            .binds
            .iter()
            .map(|k| key_label(&k.key, mod_key))
            .collect();
        let text = format!(
            "{title} {} {} {}",
            notes.join(" "),
            key_words.join(" "),
            section_title(&entry.section)
        );
        rows.push((row.upcast(), group, tab, text.to_lowercase()));
    }
    view.entries.replace(entries);
    view.rows.replace(rows);
    filter(view);
}

/// A row of the list: the action's title, its notes, and below them its chords on a line of
/// their own, a dot between each, or "Disabled".
fn entry_row(title: &str, notes: &str, chords: Vec<gtk::Widget>) -> adw::PreferencesRow {
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    let title_label = gtk::Label::builder()
        .label(title)
        .xalign(0.)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes(["title"])
        .build();
    text.append(&title_label);
    if !notes.is_empty() {
        text.append(
            &gtk::Label::builder()
                .label(notes)
                .xalign(0.)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .css_classes(["subtitle", "dim-label", "caption"])
                .build(),
        );
    }
    if chords.is_empty() {
        let disabled = dim_label("Disabled");
        disabled.set_halign(gtk::Align::Start);
        text.append(&disabled);
    } else {
        text.append(&super::chord_line::ChordLine::new(chords));
    }

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&text);
    let row = adw::PreferencesRow::builder()
        .title(title)
        .activatable(false)
        .build();
    row.set_child(Some(&content));
    row
}

fn mod_key(store: &Store) -> Modifiers {
    let config = store.config();
    config.input.mod_key.unwrap_or(ModKey::Super).to_modifiers()
}

/// The entry other than `except`'s whose bind runs the chord `key` now, if any.
fn running(store: &Store, key: &Key, except: &niri_config::Action) -> Option<Entry> {
    let list = bind_list::entries(&sources(store));
    list.into_iter().find(|s| {
        s.action != *except
            && s.binds
                .iter()
                .any(|k| k.taken.is_none() && k.key.conflicts_with(key))
    })
}

/// Asks before taking the chord `key` from `other`, the entry whose bind runs it now, and calls
/// `then` with whether to go on. Taking it leaves `other` its other binds, or turns it off.
fn take_key(
    ctx: &PageContext,
    parent: &impl IsA<gtk::Widget>,
    key: Key,
    other: Entry,
    then: impl Fn(Option<(Entry, Key)>) + 'static,
) {
    if !other.editable() {
        let alert = adw::AlertDialog::new(
            Some("Chord Taken"),
            Some(&format!(
                "It runs “{}”, from a file that can't be changed here.",
                other.title()
            )),
        );
        alert.add_response("close", "Close");
        alert.present(Some(parent));
        return;
    }
    let alert = adw::AlertDialog::new(
        Some("Replace Bind?"),
        Some(&format!("This chord runs “{}”.", other.title())),
    );
    alert.add_response("cancel", "Cancel");
    alert.add_response("replace", "Replace");
    alert.set_response_appearance("replace", adw::ResponseAppearance::Destructive);
    alert.set_default_response(Some("cancel"));
    let _ = ctx;
    alert.choose(Some(parent), gio::Cancellable::NONE, move |response| {
        if response == "replace" {
            then(Some((other.clone(), key)));
        }
    });
}

/// Takes the chord `key` off `other`: its other binds stay, and with none left it's turned off.
fn give_up(store: &Store, other: &Entry, key: Key) {
    let rest: Vec<BindNode> = binds_of(other)
        .into_iter()
        .filter(|b| {
            b.key
                .parse::<Key>()
                .map_or(true, |k| !k.conflicts_with(&key))
        })
        .collect();
    let change = if rest.is_empty() {
        Change::Disable
    } else {
        Change::Keys(rest)
    };
    apply(store, other, change);
}

/// The dialog of one entry: its binds, each with its settings and to take off, one more to
/// record, and turning the action off or putting back its defaults.
fn open_entry(ctx: &PageContext, parent: &impl IsA<gtk::Widget>, entry: Entry) {
    let mod_key = mod_key(&ctx.store);
    let dialog = adw::Dialog::builder()
        .title(entry.title())
        .content_width(480)
        .build();
    let page = adw::PreferencesPage::new();
    let binds: Rc<RefCell<Vec<EditedBind>>> = Rc::new(RefCell::new(
        binds_of(&entry)
            .into_iter()
            .map(|node| EditedBind::new(node, mod_key))
            .collect(),
    ));
    // Chords taken from other actions on saving.
    let taken: Rc<RefCell<Vec<(Entry, Key)>>> = Rc::default();

    let binds_group = adw::PreferencesGroup::builder().title("Binds").build();
    if !entry.arguments().is_empty() {
        binds_group.set_description(Some(&entry.arguments()));
    }
    page.add(&binds_group);
    let shown: Rc<RefCell<Vec<gtk::Widget>>> = Rc::default();
    let recording = Rc::new(Recording::default());
    let record_row = adw::ActionRow::builder()
        .title("Add Bind")
        .subtitle("Backspace turns the action off")
        .build();
    let record = gtk::Button::builder()
        .label("Record…")
        .valign(gtk::Align::Center)
        .build();
    record_row.add_suffix(&record);

    // The bind rows, each with its settings, put back in order as binds come and go.
    let show: Rc<dyn Fn()> = {
        let (binds, shown, binds_group) = (binds.clone(), shown.clone(), binds_group.clone());
        let record_row = record_row.clone();
        let show_slot: ShowSlot = Rc::default();
        let slot = show_slot.clone();
        let f: Rc<dyn Fn()> = Rc::new(move || {
            for row in shown.take() {
                binds_group.remove(&row);
            }
            binds_group.remove(&record_row);
            let mut rows: Vec<gtk::Widget> = Vec::new();
            if binds.borrow().is_empty() {
                let row = adw::ActionRow::builder().title("Disabled").build();
                row.add_css_class("dim-label");
                binds_group.add(&row);
                rows.push(row.upcast());
            }
            for bind in binds.borrow().iter() {
                if !bind.connected.replace(true) {
                    let (binds, slot, row) = (binds.clone(), slot.clone(), bind.row.clone());
                    bind.remove.connect_clicked(move |_| {
                        binds.borrow_mut().retain(|b| b.row != row);
                        if let Some(show) = slot.borrow().clone() {
                            show();
                        }
                    });
                }
                binds_group.add(&bind.row);
                rows.push(bind.row.clone().upcast());
            }
            binds_group.add(&record_row);
            shown.replace(rows);
        });
        show_slot.replace(Some(f.clone()));
        f
    };
    show();

    {
        let (ctx, binds, taken, show) = (ctx.clone(), binds.clone(), taken.clone(), show.clone());
        let (recording, dialog_widget, one) = (recording.clone(), dialog.clone(), entry.clone());
        record.connect_clicked(move |button| {
            button.set_label("Press a chord…");
            let button = button.clone();
            let (ctx, binds, taken, show, dialog) = (
                ctx.clone(),
                binds.clone(),
                taken.clone(),
                show.clone(),
                dialog_widget.clone(),
            );
            let one = one.clone();
            recording.start(&dialog_widget, mod_key, move |recorded| {
                button.set_label("Record…");
                let Some(text) = recorded else { return };
                // Backspace alone turns the action off, as in GNOME's shortcuts.
                if text == "BackSpace" {
                    apply(&ctx.store, &one, Change::Disable);
                    dialog.close();
                    return;
                }
                let Some(key) = parse_key(&text) else {
                    ctx.toast("That chord can't be bound");
                    return;
                };
                if binds.borrow().iter().any(|b| b.node.key == text) {
                    return;
                }
                let node = BindNode {
                    key: text,
                    properties: Vec::new(),
                    action: Some(one.node.clone()),
                };
                let add = {
                    let (binds, taken, show) = (binds.clone(), taken.clone(), show.clone());
                    move |take: Option<(Entry, Key)>| {
                        taken.borrow_mut().extend(take);
                        binds
                            .borrow_mut()
                            .push(EditedBind::new(node.clone(), mod_key));
                        show();
                    }
                };
                match running(&ctx.store, &key, &one.action) {
                    Some(other) => take_key(&ctx, &dialog, key, other, add),
                    None => add(None),
                }
            });
        });
    }

    // Turning it off, putting back its defaults, or removing one of the user's own.
    let buttons = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .halign(gtk::Align::Center)
        .build();
    let button = |label: &str, destructive: bool| {
        let classes: &[&str] = if destructive {
            &["pill", "destructive-action"]
        } else {
            &["pill"]
        };
        gtk::Button::builder()
            .label(label)
            .css_classes(classes)
            .build()
    };
    let mut any = false;
    let has_defaults = !entry.defaults.is_empty();
    if entry.changed() && has_defaults {
        let reset = button("Reset to Default", false);
        let (ctx, dialog, one) = (ctx.clone(), dialog.clone(), entry.clone());
        reset.connect_clicked(move |_| {
            apply(&ctx.store, &one, Change::Reset);
            dialog.close();
        });
        buttons.append(&reset);
        any = true;
    }
    if has_defaults && !entry.disabled {
        let disable = button("Disable", true);
        let (ctx, dialog, one) = (ctx.clone(), dialog.clone(), entry.clone());
        disable.connect_clicked(move |_| {
            apply(&ctx.store, &one, Change::Disable);
            dialog.close();
        });
        buttons.append(&disable);
        any = true;
    }
    if !has_defaults && entry.changed() {
        let remove = button("Remove Binds", true);
        let (ctx, dialog, one) = (ctx.clone(), dialog.clone(), entry.clone());
        remove.connect_clicked(move |_| {
            apply(&ctx.store, &one, Change::Reset);
            dialog.close();
        });
        buttons.append(&remove);
        any = true;
    }
    if any {
        let group = adw::PreferencesGroup::new();
        group.add(&buttons);
        page.add(&group);
    }

    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::builder()
        .label("Save")
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

    let (ctx, dialog_) = (ctx.clone(), dialog.clone());
    save.connect_clicked(move |_| {
        for (other, key) in taken.take() {
            give_up(&ctx.store, &other, key);
        }
        let binds: Vec<BindNode> = binds.borrow().iter().map(EditedBind::written).collect();
        apply(&ctx.store, &entry, Change::Keys(binds));
        dialog_.close();
    });

    dialog.present(Some(parent));
}

/// A bind in an action's dialog: as it's written, its settings, and its row, which shows its chord
/// and opens to its settings.
struct EditedBind {
    node: BindNode,
    settings: Rc<BindSettings>,
    row: adw::ExpanderRow,
    remove: gtk::Button,
    /// Whether `remove` is connected yet.
    connected: Cell<bool>,
}

impl EditedBind {
    fn new(node: BindNode, mod_key: Modifiers) -> Self {
        let settings = Rc::new(BindSettings::new(&node.properties));
        let row = adw::ExpanderRow::builder().use_markup(false).build();
        match node.key.parse::<Key>() {
            Ok(key) => row.add_prefix(&chord_label(&key, mod_key)),
            Err(_) => row.set_title(&node.key),
        }
        for settings_row in settings.rows() {
            row.add_row(&settings_row);
        }
        row.set_subtitle(&settings.summary());
        let (weak, row_) = (Rc::downgrade(&settings), row.downgrade());
        settings.connect_changed(move || {
            if let (Some(settings), Some(row)) = (weak.upgrade(), row_.upgrade()) {
                row.set_subtitle(&settings.summary());
            }
        });
        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Remove Bind")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        row.add_suffix(&remove);
        Self {
            node,
            settings,
            row,
            remove,
            connected: Cell::new(false),
        }
    }

    /// The bind as it's written now, with its settings.
    fn written(&self) -> BindNode {
        BindNode {
            properties: self.settings.properties(),
            ..self.node.clone()
        }
    }
}

/// An action to bind: bento's own, or one the running engine or shell declares.
#[derive(Clone)]
struct Choice {
    action: LayoutEngineAction,
    declarer: Declarer,
    section: Section,
}

impl Choice {
    fn title(&self) -> String {
        let description = self
            .action
            .description
            .as_deref()
            .unwrap_or(&self.action.name);
        description.trim_end_matches('.').to_owned()
    }
}

/// bento's actions by their groups, then the running engine's, then the running shell's.
fn choices() -> Vec<Choice> {
    // bento's in the General tab's groups, without `shell-action`: the shell's actions are
    // picked under their modules.
    let native = niri_config::actions::native_actions().into_iter();
    let mut choices: Vec<Choice> = native
        .filter(|action| action.name != SHELL_ACTION)
        .map(|action| Choice {
            section: Section::General(general_group(&action.name)),
            action,
            declarer: Declarer::Bento,
        })
        .collect();
    let order = |c: &Choice| match c.section {
        Section::General(group) => bind_list::GENERAL_GROUPS.iter().position(|g| *g == group),
        _ => None,
    };
    choices.sort_by_key(order);
    if let Ok(engine) = ipc::layout_engine() {
        let name = engine.name.clone();
        choices.extend(engine.actions.into_iter().map(|action| Choice {
            action,
            declarer: Declarer::Engine,
            section: Section::Engine(name.clone()),
        }));
    }
    if let Ok(shell) = ipc::shell() {
        choices.extend(shell.actions.into_iter().map(|a| Choice {
            action: a.action,
            declarer: Declarer::Shell,
            section: Section::Module(a.module),
        }));
    }
    choices
}

/// The action `choice` writes, with its arguments from `fields`.
fn chosen_action(choice: &Choice, fields: &[Field]) -> Result<ActionNode, String> {
    let mut action = action_node(&choice.action, fields)?;
    if choice.declarer == Declarer::Shell {
        action.args.insert(0, Value::Str(action.name));
        action.name = String::from(SHELL_ACTION);
    }
    Ok(action)
}

/// The action picker's model: the choices' titles in sections by who declares them, with a
/// heading for each. Each item is the section's title, then the choice's, apart by `\u{1f}`.
fn picker(
    choices: &[Choice],
) -> (
    gtk::SortListModel,
    gtk::SignalListItemFactory,
    gtk::SignalListItemFactory,
) {
    let items: Vec<String> = choices
        .iter()
        .map(|c| format!("{}\u{1f}{}", section_title(&c.section), c.title()))
        .collect();
    let list = gtk::StringList::new(&items.iter().map(String::as_str).collect::<Vec<_>>());
    let section = |item: &glib::Object| {
        let text = item.downcast_ref::<gtk::StringObject>().map(|s| s.string());
        text.map(|t| t.split('\u{1f}').next().unwrap_or_default().to_owned())
            .unwrap_or_default()
    };
    // The choices come grouped already: the sorter only says where a section starts.
    let order: Vec<String> = choices.iter().map(|c| section_title(&c.section)).collect();
    let sorter = gtk::CustomSorter::new(move |a, b| {
        let at = |s: String| order.iter().position(|o| *o == s).unwrap_or(usize::MAX);
        at(section(a)).cmp(&at(section(b))).into()
    });
    let model = gtk::SortListModel::new(Some(list), None::<gtk::Sorter>);
    model.set_section_sorter(Some(&sorter));

    let label_factory = |part: usize, heading: bool| {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(move |_, item| {
            let label = gtk::Label::builder().xalign(0.).build();
            if heading {
                label.add_css_class("heading");
                label.add_css_class("dim-label");
            }
            if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
                item.set_child(Some(&label));
            } else if let Some(item) = item.downcast_ref::<gtk::ListHeader>() {
                item.set_child(Some(&label));
            }
        });
        factory.connect_bind(move |_, item| {
            let (object, child) = if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
                (item.item(), item.child())
            } else if let Some(item) = item.downcast_ref::<gtk::ListHeader>() {
                (item.item(), item.child())
            } else {
                return;
            };
            let text = object
                .and_downcast::<gtk::StringObject>()
                .map(|s| {
                    s.string()
                        .split('\u{1f}')
                        .nth(part)
                        .unwrap_or_default()
                        .to_owned()
                })
                .unwrap_or_default();
            if let Some(label) = child.and_downcast::<gtk::Label>() {
                label.set_label(&text);
            }
        });
        factory
    };
    (model, label_factory(1, false), label_factory(0, true))
}

/// The dialog to add a bind: its chord, the action and its arguments, and the bind's settings.
/// An action that's listed already gets the bind besides its others.
fn open_new(ctx: &PageContext, parent: &impl IsA<gtk::Widget>) {
    let choices = Rc::new(choices());
    let dialog = adw::Dialog::builder()
        .title("Add Bind")
        .content_width(480)
        .build();
    let page = adw::PreferencesPage::new();

    // The chord.
    let key: Rc<RefCell<Option<String>>> = Rc::default();
    let key_group = adw::PreferencesGroup::new();
    let key_row = adw::ActionRow::builder().title("Chord").build();
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
        let (key, key_view) = (key.clone(), key_view.clone());
        move || {
            while let Some(child) = key_view.first_child() {
                key_view.remove(&child);
            }
            match key.borrow().as_deref().and_then(parse_key) {
                Some(parsed) => key_view.append(&chord_label(&parsed, mod_key)),
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
            key_view.append(&dim_label("Press a chord…"));
            let (key, show_key) = (key.clone(), show_key.clone());
            recording.start(&dialog_widget, mod_key, move |recorded| {
                if let Some(recorded) = recorded {
                    key.replace(Some(recorded));
                }
                show_key();
            });
        });
    }

    // The action, in sections by who declares it, found by any word of its title.
    let action_group = adw::PreferencesGroup::new();
    let (model, factory, header_factory) = picker(&choices);
    let action_row = adw::ComboRow::builder()
        .title("Action")
        .model(&model)
        .factory(&factory)
        .enable_search(true)
        .search_match_mode(gtk::StringFilterMatchMode::Substring)
        .build();
    action_row.set_header_factory(Some(&header_factory));
    action_row.set_expression(Some(gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        None::<gtk::Expression>,
        "string",
    )));
    action_group.add(&action_row);
    page.add(&action_group);

    // The action's arguments, made again when the action changes.
    let args_group = adw::PreferencesGroup::builder().title("Arguments").build();
    page.add(&args_group);
    let fields: Rc<RefCell<Vec<Field>>> = Rc::default();
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
            let mut new = Vec::new();
            let mut position = 0;
            for arg in &choice.action.args {
                let field = Field::new(arg, None, position);
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

    let settings_group = adw::PreferencesGroup::new();
    let settings = BindSettings::new(&[]);
    let expander = adw::ExpanderRow::builder().title("Bind Settings").build();
    for row in settings.rows() {
        expander.add_row(&row);
    }
    settings_group.add(&expander);
    page.add(&settings_group);

    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::builder()
        .label("Add")
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
            ctx.toast("Record a chord first");
            return;
        };
        let Some(parsed) = parse_key(&key_text) else {
            ctx.toast("That chord can't be bound");
            return;
        };
        let Some(choice) = choices.get(action_row.selected() as usize) else {
            ctx.toast("Pick an action first");
            return;
        };
        let node = match chosen_action(choice, &fields.borrow()) {
            Ok(node) => node,
            Err(message) => {
                ctx.toast(&message);
                return;
            }
        };
        // As the main config writes it: the shell's actions with `shell-action`.
        let file = match choice.declarer {
            Declarer::Engine => BindFile::Engine,
            Declarer::Bento | Declarer::Shell => BindFile::Main,
        };
        let Some(action) = bind_list::parse(&node, file) else {
            ctx.toast("That action can't be bound");
            return;
        };
        // The entry it joins, if it's listed already, with the new bind besides its others.
        let list = bind_list::entries(&sources(&ctx.store));
        let entry = list
            .into_iter()
            .find(|s| s.action == action)
            .unwrap_or_else(|| Entry {
                section: choice.section.clone(),
                declarer: choice.declarer,
                action: action.clone(),
                node: node.clone(),
                declaration: Some(choice.action.clone()),
                defaults: Vec::new(),
                binds: Vec::new(),
                disabled: false,
            });
        let mut binds = binds_of(&entry);
        binds.push(BindNode {
            key: key_text,
            properties: settings.properties(),
            action: Some(node),
        });
        let commit = {
            let (ctx, dialog) = (ctx.clone(), dialog_.clone());
            move |take: Option<(Entry, Key)>| {
                if let Some((other, key)) = &take {
                    give_up(&ctx.store, other, *key);
                }
                apply(&ctx.store, &entry, Change::Keys(binds.clone()));
                dialog.close();
            }
        };
        match running(&ctx.store, &parsed, &action) {
            Some(other) => take_key(&ctx, &dialog_, parsed, other, commit),
            None => commit(None),
        }
    });

    dialog.present(Some(parent));
}

fn reset_all(ctx: &PageContext, parent: &gtk::Button) {
    let alert = adw::AlertDialog::new(
        Some("Reset All Binds?"),
        Some(
            "Every action goes back to its default binds: the binds of your own and the \
             actions you turned off are taken out of the config, the engine's file and the \
             shell's. Binds from included files stay as they are.",
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
        let clear = |doc: &mut crate::kdl::Document| {
            for bind in doc.binds() {
                if let Ok(key) = bind.key.parse::<Key>() {
                    doc.remove_bind(&key);
                }
            }
            for node in doc.unbinds() {
                doc.remove_unbind(&node);
            }
        };
        ctx.store.edit_in(File::Main, "binds.reset", clear);
        if ctx.store.engine_path().is_some_and(|path| path.exists()) {
            ctx.store.edit_in(File::Engine, "binds.reset", clear);
        }
        // The shell's binds are in its modules' blocks, next to their settings, which stay.
        if ctx.store.shell_path().is_some_and(|path| path.exists()) {
            ctx.store
                .edit_in(File::ShellSettings, "binds.reset", move |doc| {
                    for name in doc.block_names() {
                        doc.edit_block(&name, clear);
                    }
                });
        }
    });
}

fn parse_key(text: &str) -> Option<Key> {
    text.parse().ok()
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
        let title = crate::page::title_from(&name);
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
        let default = arg.default.as_ref().and_then(crate::kdl::option_value);
        let first = current.first().cloned().or(default.clone());
        let required = arg.default.is_none() && !arg.optional;

        let (row, value): (gtk::Widget, Box<dyn Fn() -> _>) = match arg.r#type {
            Some(ArgType::Boolean) => {
                let row = adw::SwitchRow::builder()
                    .use_markup(false)
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
                    .use_markup(false)
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
                let row = adw::EntryRow::builder()
                    .use_markup(false)
                    .title(&title)
                    .text(&text)
                    .build();
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
                Some(default) => node.args.extend(crate::kdl::option_value(default)),
                None => return Err(format!("Fill in “{}”", crate::page::title_from(&arg.name))),
            },
        }
    }
    Ok(node)
}

/// A bind's own settings, as rows: shown under their bind in an action's dialog, and under "Bind
/// Settings" when adding one.
struct BindSettings {
    repeat: adw::SwitchRow,
    cooldown: adw::SpinRow,
    when_locked: adw::SwitchRow,
    inhibiting: adw::SwitchRow,
    title: adw::EntryRow,
    /// Properties the rows don't show, kept as they are.
    other: Vec<(String, Value)>,
}

impl BindSettings {
    fn new(props: &[(String, Value)]) -> Self {
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
            .title("Title in the Bind Overlay")
            .build();
        if let Some(Value::Str(text)) = get("hotkey-overlay-title") {
            title.set_text(&text);
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
            repeat,
            cooldown,
            when_locked,
            inhibiting,
            title,
            other,
        }
    }

    fn rows(&self) -> [gtk::Widget; 5] {
        [
            self.repeat.clone().upcast(),
            self.cooldown.clone().upcast(),
            self.when_locked.clone().upcast(),
            self.inhibiting.clone().upcast(),
            self.title.clone().upcast(),
        ]
    }

    /// Calls `f` whenever one of the settings changes.
    fn connect_changed(&self, f: impl Fn() + 'static) {
        let f = Rc::new(f);
        for switch in [&self.repeat, &self.when_locked, &self.inhibiting] {
            let f = f.clone();
            switch.connect_active_notify(move |_| f());
        }
        let f_ = f.clone();
        self.cooldown.connect_value_notify(move |_| f_());
        self.title.connect_changed(move |_| f());
    }

    /// What differs from the defaults, as a row's subtitle says it, like "Works when locked".
    fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.repeat.is_active() {
            parts.push(String::from("Doesn't repeat"));
        }
        let cooldown = self.cooldown.value() as i64;
        if cooldown > 0 {
            parts.push(format!("{cooldown} ms cooldown"));
        }
        if self.when_locked.is_active() {
            parts.push(String::from("Works when locked"));
        }
        if !self.inhibiting.is_active() {
            parts.push(String::from("Apps can't take it"));
        }
        let title = self.title.text();
        if !title.is_empty() {
            parts.push(format!("“{title}”"));
        }
        parts.join(", ")
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

/// Recording a chord: keys, and clicks and scrolls with modifiers held. While it records, the
/// window inhibits bento's binds, so chords bento has bound reach it instead of running.
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
        keys.connect_key_pressed(move |controller, keyval, keycode, state| {
            if keyval == gdk::Key::Escape && modifiers(state).is_empty() {
                finish_(None);
                return glib::Propagation::Stop;
            }
            // The key's own symbol, without what Shift makes of it: Shift+/ is `Shift+Slash`, as
            // bento's defaults write it, so a key taken by another bind is seen as taken.
            let unshifted = gdk::Display::default()
                .and_then(|display| {
                    let group = controller.group() as i32;
                    display.translate_key(keycode, gdk::ModifierType::empty(), group)
                })
                .map_or(keyval, |(key, ..)| key);
            let Some(name) = unshifted
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
            // Without modifiers it's a click on the dialog, not a chord.
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

        // The dialog sits inside the app's window, so that's the one to inhibit binds on.
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

/// The modifiers a key shows, with the compositor's Mod key spelled out, and the same as GDK's.
fn shown_modifiers(key: &Key, mod_key: Modifiers) -> (Modifiers, gdk::ModifierType) {
    let mut modifiers = key.modifiers;
    if modifiers.contains(Modifiers::COMPOSITOR) {
        modifiers.remove(Modifiers::COMPOSITOR);
        modifiers |= mod_key;
    }
    // A modifier pressed alone is the key itself, not one held with it.
    match key.trigger {
        Trigger::CompositorMod => modifiers.remove(mod_key),
        Trigger::Modifier(modifier) => modifiers.remove(modifier.to_modifiers()),
        _ => (),
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
    // Mouse buttons, scrolling and modifiers alone have no key of their own.
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
    text.push_str(&trigger_name(key.trigger, mod_key));
    text
}

/// What a trigger with no key of its own is called: a mouse button, a scroll, a modifier alone.
fn trigger_name(trigger: Trigger, mod_key: Modifiers) -> String {
    let modifier = |m: Modifiers| {
        [
            (Modifiers::SUPER, "Super"),
            (Modifiers::CTRL, "Ctrl"),
            (Modifiers::ALT, "Alt"),
            (Modifiers::SHIFT, "Shift"),
            (Modifiers::ISO_LEVEL3_SHIFT, "AltGr"),
            (Modifiers::ISO_LEVEL5_SHIFT, "Level 5 Shift"),
        ]
        .into_iter()
        .find(|(flag, _)| m.contains(*flag))
        .map_or("Mod", |(_, name)| name)
    };
    let name = match trigger {
        Trigger::Keysym(keysym) => return format!("{keysym:?}"),
        Trigger::CompositorMod => modifier(mod_key),
        Trigger::Modifier(key) => modifier(key.to_modifiers()),
        Trigger::MouseLeft => "Left Click",
        Trigger::MouseRight => "Right Click",
        Trigger::MouseMiddle => "Middle Click",
        Trigger::MouseBack => "Back Button",
        Trigger::MouseForward => "Forward Button",
        Trigger::WheelScrollDown => "Scroll Down",
        Trigger::WheelScrollUp => "Scroll Up",
        Trigger::WheelScrollLeft => "Scroll Left",
        Trigger::WheelScrollRight => "Scroll Right",
        Trigger::TouchpadScrollDown => "Touchpad Scroll Down",
        Trigger::TouchpadScrollUp => "Touchpad Scroll Up",
        Trigger::TouchpadScrollLeft => "Touchpad Scroll Left",
        Trigger::TouchpadScrollRight => "Touchpad Scroll Right",
        Trigger::TabletStylusButton1 => "Stylus Button 1",
        Trigger::TabletStylusButton2 => "Stylus Button 2",
        Trigger::TabletStylusButton3 => "Stylus Button 3",
    };
    String::from(name)
}

/// Draws a chord as keycaps, with the compositor's Mod key spelled out.
fn chord_label(key: &Key, mod_key: Modifiers) -> gtk::Widget {
    let (_, mask) = shown_modifiers(key, mod_key);
    // A trigger with no key of its own, like a scroll, is drawn on a stand-in key's keycap, so
    // it looks like the rest: Super, then Scroll Down.
    let (gdk_key, name) = match gdk_key(key.trigger) {
        Some(gdk_key) => (gdk_key, None),
        None => (gdk::Key::a, Some(trigger_name(key.trigger, mod_key))),
    };
    let label = gtk::ShortcutLabel::new(&gtk::accelerator_name(gdk_key, mask));
    label.set_valign(gtk::Align::Center);
    // The keycaps read on their own; hide the "+" GTK puts between them. If GTK ever draws
    // them differently, the worst case is that the "+" shows again.
    let mut last = None;
    let mut child = label.first_child();
    while let Some(widget) = child {
        if let Some(text) = widget.downcast_ref::<gtk::Label>() {
            if text.label() == "+" {
                widget.set_visible(false);
            } else {
                last = Some(text.clone());
            }
        }
        child = widget.next_sibling();
    }
    if let (Some(name), Some(last)) = (name, last) {
        last.set_label(&name);
    }
    label.upcast()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn a_change_to_a_bind_of_the_main_config_is_saved_beside_an_engine_file() {
        crate::store::on_own_context(|| {
            let dir =
                std::env::temp_dir().join(format!("bento-settings-binds-{}", std::process::id()));
            fs::create_dir_all(dir.join("engines")).unwrap();
            let config_path = dir.join("config.kdl");
            fs::write(&config_path, "tiling {\n    layout \"niri\"\n}\n").unwrap();
            // The engine has a file of its own, as it does once it has run.
            fs::write(dir.join("engines").join("niri.kdl"), "").unwrap();

            let store = Store::load_from(config_path.clone());
            let list = bind_list::entries(&sources(&store));
            let entry = list
                .iter()
                .find(|e| e.node.text() == "power-off-monitors")
                .unwrap();
            let bind = BindNode {
                key: String::from("Mod+F9"),
                properties: Vec::new(),
                action: Some(entry.node.clone()),
            };
            apply(&store, entry, Change::Keys(vec![bind]));
            store.flush();

            let written = fs::read_to_string(&config_path).unwrap();
            fs::remove_dir_all(&dir).unwrap();
            assert!(written.contains("Mod+F9"), "{written}");
        });
    }

    #[test]
    fn a_modifier_alone_is_named_once() {
        let key: Key = "Mod".parse().unwrap();
        assert_eq!(key_label(&key, Modifiers::SUPER), "Super");
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
