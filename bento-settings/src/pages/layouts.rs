//! Keyboard layouts as a list of readable names, with a searchable picker to add more. They're
//! written to `xkb { layout "us,de"; variant ",neo"; }`, one entry per layout in both.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use niri_config::Config;

use crate::kdl::Value;
use crate::page::PageContext;

/// xkb's lists of layouts and variants with readable names.
const RULES: &[&str] = &[
    "/usr/share/X11/xkb/rules/evdev.lst",
    "/usr/share/X11/xkb/rules/base.lst",
];

const LAYOUT: &[&str] = &["input", "keyboard", "xkb", "layout"];
const VARIANT: &[&str] = &["input", "keyboard", "xkb", "variant"];

#[derive(Clone)]
struct Layout {
    code: String,
    variant: String,
    name: String,
}

fn catalog() -> Vec<Layout> {
    let text = RULES
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();
    parse(&text)
}

fn parse(text: &str) -> Vec<Layout> {
    let mut section = "";
    let mut layouts = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("! ") {
            section = name.trim();
            continue;
        }
        let Some((code, description)) = line.trim().split_once(char::is_whitespace) else {
            continue;
        };
        let description = description.trim();
        match section {
            "layout" => layouts.push(Layout {
                code: code.to_owned(),
                variant: String::new(),
                name: description.to_owned(),
            }),
            // Variants read "de: German (Neo 2)".
            "variant" => {
                if let Some((layout, name)) = description.split_once(": ") {
                    layouts.push(Layout {
                        code: layout.to_owned(),
                        variant: code.to_owned(),
                        name: name.to_owned(),
                    });
                }
            }
            _ => (),
        }
    }
    layouts
}

/// The configured layouts as (layout, variant) pairs.
fn configured(config: &Config) -> Vec<(String, String)> {
    let xkb = &config.input.keyboard.xkb;
    if xkb.layout.trim().is_empty() {
        return Vec::new();
    }
    let variants: Vec<&str> = xkb.variant.split(',').collect();
    xkb.layout
        .split(',')
        .enumerate()
        .map(|(i, layout)| {
            let variant = variants.get(i).copied().unwrap_or_default();
            (layout.trim().to_owned(), variant.trim().to_owned())
        })
        .collect()
}

fn write(ctx: &PageContext, layouts: &[(String, String)]) {
    ctx.store.edit(|doc| {
        if layouts.is_empty() {
            doc.remove(LAYOUT);
            doc.remove(VARIANT);
            return;
        }
        let codes: Vec<&str> = layouts.iter().map(|(code, _)| code.as_str()).collect();
        doc.set(LAYOUT, vec![Value::Str(codes.join(","))]);
        if layouts.iter().all(|(_, variant)| variant.is_empty()) {
            doc.remove(VARIANT);
        } else {
            let variants: Vec<&str> = layouts.iter().map(|(_, v)| v.as_str()).collect();
            doc.set(VARIANT, vec![Value::Str(variants.join(","))]);
        }
    });
}

fn name_of(catalog: &[Layout], code: &str, variant: &str) -> String {
    catalog
        .iter()
        .find(|layout| layout.code == code && layout.variant == variant)
        .map(|layout| layout.name.clone())
        .unwrap_or_else(|| {
            if variant.is_empty() {
                code.to_owned()
            } else {
                format!("{code} ({variant})")
            }
        })
}

pub fn build(ctx: &PageContext) -> gtk::Widget {
    let catalog = Rc::new(catalog());
    let group = adw::PreferencesGroup::builder().title("Layouts").build();
    ctx.anchor("keyboard.layouts", &group);

    let add = gtk::Button::builder()
        .icon_name("ph-plus-symbolic")
        .tooltip_text("Add Layout")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    group.set_header_suffix(Some(&add));
    {
        let (ctx, catalog) = (ctx.clone(), catalog.clone());
        add.connect_clicked(move |button| {
            let ctx = ctx.clone();
            picker(button, &catalog, move |layout| {
                let mut layouts = configured(&ctx.store.config());
                let entry = (layout.code.clone(), layout.variant.clone());
                if !layouts.contains(&entry) {
                    layouts.push(entry);
                    write(&ctx, &layouts);
                }
            });
        });
    }

    let rows: Rc<RefCell<Vec<gtk::Widget>>> = Rc::default();
    let refresh = {
        let (ctx, group) = (ctx.clone(), group.clone());
        move || {
            for row in rows.borrow_mut().drain(..) {
                group.remove(&row);
            }
            let layouts = configured(&ctx.store.config());
            if layouts.is_empty() {
                let row = adw::ActionRow::builder()
                    .title("System Default")
                    .css_classes(["dim-label"])
                    .build();
                group.add(&row);
                rows.borrow_mut().push(row.upcast());
            }
            for (idx, (code, variant)) in layouts.iter().enumerate() {
                let row = adw::ActionRow::builder()
                    .title(name_of(&catalog, code, variant))
                    .build();
                let remove = gtk::Button::builder()
                    .icon_name("ph-x-symbolic")
                    .tooltip_text("Remove")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat", "circular"])
                    .build();
                {
                    let (ctx, layouts) = (ctx.clone(), layouts.clone());
                    remove.connect_clicked(move |_| {
                        let mut layouts = layouts.clone();
                        layouts.remove(idx);
                        write(&ctx, &layouts);
                    });
                }
                row.add_suffix(&remove);
                group.add(&row);
                rows.borrow_mut().push(row.upcast());
            }
        }
    };
    refresh();
    ctx.store.subscribe(refresh);

    group.upcast()
}

/// A dialog listing every layout and variant, filtered as you type.
fn picker(
    parent: &impl IsA<gtk::Widget>,
    catalog: &Rc<Vec<Layout>>,
    pick: impl Fn(&Layout) + 'static,
) {
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search Layouts")
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(6)
        .build();
    let list = gtk::ListBox::builder()
        .css_classes(["navigation-sidebar"])
        .activate_on_single_click(true)
        .build();
    for layout in catalog.iter() {
        let row = adw::ActionRow::builder().title(&layout.name).build();
        list.append(&row);
    }

    let query = Rc::new(RefCell::new(String::new()));
    {
        let (catalog, query) = (catalog.clone(), query.clone());
        list.set_filter_func(move |row| {
            let layout = &catalog[row.index() as usize];
            let name = layout.name.to_lowercase();
            query
                .borrow()
                .split_whitespace()
                .all(|word| name.contains(word) || layout.code == word || layout.variant == word)
        });
    }
    {
        let (list, query) = (list.clone(), query.clone());
        search.connect_search_changed(move |entry| {
            *query.borrow_mut() = entry.text().to_lowercase();
            list.invalidate_filter();
        });
    }

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.add_top_bar(&search);
    toolbar.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build(),
    ));
    let dialog = adw::Dialog::builder()
        .title("Add Layout")
        .content_width(420)
        .content_height(560)
        .child(&toolbar)
        .focus_widget(&search)
        .build();
    {
        let (catalog, dialog) = (catalog.clone(), dialog.clone());
        list.connect_row_activated(move |_, row| {
            pick(&catalog[row.index() as usize]);
            dialog.close();
        });
    }
    dialog.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_layouts_and_variants() {
        let text = "! model\n  pc105           Generic 105-key PC\n\n! layout\n  us              English (US)\n  de              German\n\n! variant\n  neo             de: German (Neo 2)\n\n! option\n  grp             Switching\n";
        let layouts = parse(text);
        let names: Vec<(&str, &str, &str)> = layouts
            .iter()
            .map(|l| (l.code.as_str(), l.variant.as_str(), l.name.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("us", "", "English (US)"),
                ("de", "", "German"),
                ("de", "neo", "German (Neo 2)")
            ]
        );
    }
}
