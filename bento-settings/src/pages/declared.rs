//! Settings that the running engine and shell declare, on the pages they name.
//!
//! An engine's or a shell's setting can name a page of the app and a group on it, like
//! `page: "sound", group: "Output Switcher"`. The pages are the app's own, listed in
//! [`niri_config::SETTINGS_PAGES`]; a setting that names none of them shows on the Engine page
//! for an engine's, or the Shell page for a shell's. Settings with the same group show together,
//! and the rest under their module's name, or "Settings" on the Engine page.

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
use crate::page::PageContext;
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
    /// The page it shows on.
    fn page(&self) -> &str {
        match self.setting.page.as_deref() {
            Some(page) if SETTINGS_PAGES.contains(&page) => page,
            _ => match self.owner {
                Owner::Engine => "engine",
                Owner::Shell(_) => "shell",
            },
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

/// A group of settings on a page: its title and its settings.
type Group = (String, Vec<Item>);

/// The groups on `page`, in the order they first come.
fn groups_on(page: &str, items: Vec<Item>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for item in items.into_iter().filter(|item| item.page() == page) {
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
                let anchor = match &item.owner {
                    Owner::Engine => format!("settings.{}", item.setting.name),
                    Owner::Shell(block) => format!("shell.{block}.{}", item.setting.name),
                };
                view.ctx.anchor(&anchor, &row);
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
    fn settings_go_on_the_page_and_in_the_group_they_name() {
        let sound = || Owner::Shell(String::from("sound"));
        let items = vec![
            item(Owner::Engine, "gravity", None, None),
            item(
                sound(),
                "pinned",
                Some("appearance"),
                Some("Output Switcher"),
            ),
            item(Owner::Shell(String::from("taskbar")), "pinned", None, None),
            // A page the app doesn't have is the Shell page.
            item(sound(), "volume", Some("nowhere"), None),
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
        assert_eq!(
            titles("shell"),
            [pair("Taskbar", &["pinned"]), pair("Sound", &["volume"])]
        );
    }
}
