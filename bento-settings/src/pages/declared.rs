//! Settings that the running engine and shell declare, on the pages they name.
//!
//! A setting, an engine's or a shell's, shows in the app only on the page it names, in the group
//! it names there, like `page: "shell", group: "Taskbar"`. The pages are the app's own, listed in
//! [`niri_config::SETTINGS_PAGES`]. One that names no page, or one the app doesn't have, is the
//! engine's or the module's to change where it shows, like pinned apps, and stays out of the app.
//! Settings with the same group show together, and the rest under their module's name, or
//! "Settings" on the Engine page.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use niri_config::SETTINGS_PAGES;
use niri_ipc::{LayoutEngineOption, LayoutEngineOptionValue as OptionValue};

use super::engine::{option_widget, Change, ShowValue};
use crate::ipc;
use crate::kdl::{option_properties, option_values};
use crate::model::Section;
use crate::page::PageContext;
use crate::search::Entry;
use crate::store::File;

/// Whose a setting is, which says where it's written.
#[derive(Debug, Clone, PartialEq)]
enum Owner {
    /// The running engine's, in its own file's `settings {}`.
    Engine,
    /// A shell module's or the shell's, in its block of the shell's settings file.
    Shell(String),
}

#[derive(Debug, Clone, PartialEq)]
struct Item {
    owner: Owner,
    setting: LayoutEngineOption,
}

impl Item {
    /// The page it shows on, if it shows. One of a type the app has no row for, like pins, which
    /// what declares it changes where it shows, or a type from a newer bento, doesn't.
    fn page(&self) -> Option<&str> {
        use niri_ipc::LayoutEngineOptionType as Type;

        if matches!(self.setting.r#type, Type::Pins | Type::Unknown) {
            return None;
        }
        let page = self.setting.page.as_deref()?;
        SETTINGS_PAGES.contains(&page).then_some(page)
    }

    /// Where its row is on its page, which search goes to.
    fn anchor(&self) -> String {
        match &self.owner {
            Owner::Engine => format!("settings.{}", self.setting.name),
            Owner::Shell(block) => format!("shell.{block}.{}", self.setting.name),
        }
    }

    /// The title of the group it shows in on `page`.
    fn group(&self, page: &str) -> String {
        if let Some(group) = &self.setting.group {
            return group.clone();
        }
        match &self.owner {
            Owner::Engine if page == "engine" => String::from("Settings"),
            Owner::Engine => String::from("Engine"),
            Owner::Shell(name) => crate::page::title_from(name),
        }
    }
}

/// What the running engine and shell declare, engine first.
fn items() -> Vec<Item> {
    let mut items = Vec::new();
    if let Ok(engine) = ipc::layout_engine() {
        let settings = engine.settings.into_iter();
        items.extend(settings.map(|setting| Item {
            owner: Owner::Engine,
            setting,
        }));
    }
    if let Ok(shell) = ipc::shell() {
        for block in shell.settings {
            let settings = block.settings.into_iter();
            items.extend(settings.map(|setting| Item {
                owner: Owner::Shell(block.name.clone()),
                setting,
            }));
        }
    }
    items
}

/// What search finds: the app's own pages and settings, and what the running engine and shell
/// declare, as they're now.
pub fn search_index(sections: &[Section]) -> Vec<Entry> {
    index_with(sections, items())
}

/// The app's own entries, then a setting's group, which goes to its first setting, and the setting,
/// for each declared setting on a page it shows on.
fn index_with(sections: &[Section], declared: Vec<Item>) -> Vec<Entry> {
    let mut entries = crate::search::index(sections);
    let categories: Vec<_> = sections.iter().flat_map(|s| &s.categories).collect();
    for page in SETTINGS_PAGES {
        let Some(category) = categories.iter().position(|c| c.id == *page) else {
            continue;
        };
        let page_title = categories[category].title;
        for (group, items) in groups_on(page, declared.clone()) {
            let Some(first) = items.first() else { continue };
            entries.push(Entry {
                category,
                anchor: Some(first.anchor()),
                title: group.clone(),
                context: String::from(page_title),
                keywords: Vec::new(),
            });
            for item in &items {
                let mut keywords = vec![item.setting.name.replace('-', " ")];
                keywords.extend(item.setting.description.clone());
                entries.push(Entry {
                    category,
                    anchor: Some(item.anchor()),
                    title: crate::page::title_from(&item.setting.name),
                    context: format!("{page_title} › {group}"),
                    keywords,
                });
            }
        }
    }
    entries
}

/// A group of settings on a page: its title and its settings.
type Group = (String, Vec<Item>);

/// The groups on `page`, in the order they first come.
fn groups_on(page: &str, items: Vec<Item>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for item in items.into_iter().filter(|item| item.page() == Some(page)) {
        let title = item.group(page);
        match groups.iter_mut().find(|(t, _)| *t == title) {
            Some((_, list)) => list.push(item),
            None => groups.push((title, vec![item])),
        }
    }
    groups
}

/// Writes what someone sets for `item` to its file. A setting at its default stays out of it.
fn change(ctx: &PageContext, item: &Item) -> Change {
    let ctx = ctx.clone();
    let (owner, name) = (item.owner.clone(), item.setting.name.clone());
    let default = item.setting.default.clone();
    Rc::new(move |value: Option<OptionValue>| {
        let (file, section) = match &owner {
            Owner::Engine => (File::Engine, String::from("settings")),
            Owner::Shell(block) => (File::ShellSettings, block.clone()),
        };
        let key = format!("{section}.{name}");
        let (name, default) = (name.clone(), default.clone());
        ctx.store.edit_in(file, &key, move |doc| {
            let path = [section.as_str(), name.as_str()];
            if value == default {
                doc.remove(&path);
            } else {
                doc.set_node(&path, option_values(&value), option_properties(&value));
            }
        });
    })
}

struct View {
    ctx: PageContext,
    page: &'static str,
    column: gtk::Box,
    /// The groups shown, with their values left out. They're made again only when this changes;
    /// new values update the rows in place, so a row isn't replaced while someone is using it.
    shown: RefCell<Option<Vec<Group>>>,
    rows: RefCell<Vec<ShowValue>>,
    /// Set while rows show new values, so their change signals don't write them back.
    updating: Rc<Cell<bool>>,
}

/// The groups of settings declared for `page`, kept up to date.
pub fn placed(ctx: &PageContext, page: &'static str) -> gtk::Widget {
    let view = Rc::new(View {
        ctx: ctx.clone(),
        page,
        column: gtk::Box::new(gtk::Orientation::Vertical, 24),
        shown: RefCell::new(None),
        rows: RefCell::new(Vec::new()),
        updating: Rc::new(Cell::new(false)),
    });
    refresh(&view);

    // bento reads the files a moment after a write, so ask again a few times.
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
    let groups = groups_on(view.page, items());
    let mut shape = groups.clone();
    for (_, items) in &mut shape {
        for item in items {
            item.setting.value = None;
        }
    }
    if view.shown.borrow().as_ref() != Some(&shape) {
        while let Some(child) = view.column.first_child() {
            view.column.remove(&child);
        }
        view.rows.borrow_mut().clear();
        for (title, items) in &groups {
            let group = adw::PreferencesGroup::builder().title(title).build();
            for item in items {
                let change = change(&view.ctx, item);
                let (row, show) = option_widget(&item.setting, view.updating.clone(), change);
                view.ctx.anchor(&item.anchor(), &row);
                group.add(&row);
                view.rows.borrow_mut().push(show);
            }
            view.column.append(&group);
        }
        view.column.set_visible(!groups.is_empty());
        view.shown.replace(Some(shape));
    }

    view.updating.set(true);
    let settings = groups.iter().flat_map(|(_, items)| items);
    for (show, item) in view.rows.borrow().iter().zip(settings) {
        show(&item.setting.value);
    }
    view.updating.set(false);
}

#[cfg(test)]
mod tests {
    use niri_ipc::LayoutEngineOptionType;

    use super::*;

    fn item(owner: Owner, name: &str, page: Option<&str>, group: Option<&str>) -> Item {
        Item {
            owner,
            setting: LayoutEngineOption {
                name: String::from(name),
                r#type: LayoutEngineOptionType::Boolean,
                list: false,
                choices: Vec::new(),
                default: None,
                description: None,
                value: None,
                page: page.map(String::from),
                group: group.map(String::from),
            },
        }
    }

    #[test]
    fn search_finds_what_the_engine_and_shell_declare_where_it_shows() {
        let sections = super::super::sections();
        let taskbar = || Owner::Shell(String::from("taskbar"));
        let items = vec![
            item(taskbar(), "height", Some("shell"), Some("Taskbar")),
            item(Owner::Engine, "gravity", Some("engine"), None),
            item(taskbar(), "pinned", None, None),
        ];
        let entries = index_with(&sections, items);
        let shell = sections
            .iter()
            .flat_map(|s| &s.categories)
            .position(|c| c.id == "shell")
            .unwrap();
        let best = |query: &str| {
            let hits = crate::search::search(&entries, query);
            hits.first().map(|hit| {
                let entry = &entries[hit.entry];
                (
                    entry.category,
                    entry.title.clone(),
                    entry.context.clone(),
                    entry.anchor.clone(),
                )
            })
        };
        // A group goes to its first setting, and a setting to itself.
        let height = Some(String::from("shell.taskbar.height"));
        assert_eq!(
            best("taskbar"),
            Some((
                shell,
                String::from("Taskbar"),
                String::from("Shell"),
                height.clone()
            ))
        );
        assert_eq!(
            best("height"),
            Some((
                shell,
                String::from("Height"),
                String::from("Shell › Taskbar"),
                height
            ))
        );
        assert_eq!(
            best("gravity").unwrap().3,
            Some(String::from("settings.gravity"))
        );
        // What shows on no page isn't found.
        let pinned = Some(String::from("shell.taskbar.pinned"));
        assert!(entries.iter().all(|entry| entry.anchor != pinned));
    }

    #[test]
    fn settings_show_only_on_the_page_they_name_in_the_group_they_name() {
        let sound = || Owner::Shell(String::from("sound"));
        let taskbar = || Owner::Shell(String::from("taskbar"));
        let items = vec![
            item(Owner::Engine, "gravity", Some("engine"), None),
            item(
                sound(),
                "pinned",
                Some("appearance"),
                Some("Output Switcher"),
            ),
            item(taskbar(), "height", Some("shell"), None),
            // What names no page, or one the app doesn't have, isn't in the app.
            item(taskbar(), "pinned", None, None),
            item(sound(), "volume", Some("nowhere"), None),
            item(Owner::Engine, "order", None, None),
            item(Owner::Engine, "tint", Some("appearance"), None),
        ];
        let titles = |page| -> Vec<(String, Vec<String>)> {
            groups_on(page, items.clone())
                .into_iter()
                .map(|(title, items)| (title, items.into_iter().map(|i| i.setting.name).collect()))
                .collect()
        };
        let pair = |title: &str, names: &[&str]| {
            (
                String::from(title),
                names.iter().map(|n| String::from(*n)).collect(),
            )
        };
        assert_eq!(titles("engine"), [pair("Settings", &["gravity"])]);
        assert_eq!(
            titles("appearance"),
            [
                pair("Output Switcher", &["pinned"]),
                pair("Engine", &["tint"])
            ]
        );
        assert_eq!(titles("shell"), [pair("Taskbar", &["height"])]);
        let shown: usize = SETTINGS_PAGES
            .iter()
            .flat_map(|page| titles(page))
            .map(|(_, names)| names.len())
            .sum();
        assert_eq!(shown, 4);
    }
}
