//! Light or dark style and the accent color, from GNOME's interface settings. libadwaita apps
//! follow these through the settings portal, so they apply to every app right away.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;

use crate::page::PageContext;

const SCHEMA: &str = "org.gnome.desktop.interface";

const STYLES: &[(&str, &str)] = &[
    ("default", "Default"),
    ("prefer-light", "Light"),
    ("prefer-dark", "Dark"),
];

const ACCENTS: &[(&str, &str)] = &[
    ("blue", "Blue"),
    ("teal", "Teal"),
    ("green", "Green"),
    ("yellow", "Yellow"),
    ("orange", "Orange"),
    ("red", "Red"),
    ("pink", "Pink"),
    ("purple", "Purple"),
    ("slate", "Slate"),
];

pub fn build(ctx: &PageContext) -> gtk::Widget {
    let group = adw::PreferencesGroup::new();
    // Without the schema there's nothing to change; the group stays empty and out of the way.
    let Some((settings, schema)) = interface_settings() else {
        group.set_visible(false);
        return group.upcast();
    };

    if schema.has_key("color-scheme") {
        group.add(&style_row(ctx, &settings));
    }
    if schema.has_key("accent-color") {
        group.add(&accent_row(ctx, &settings));
    }
    group.upcast()
}

fn interface_settings() -> Option<(gio::Settings, gio::SettingsSchema)> {
    let schema = gio::SettingsSchemaSource::default()?.lookup(SCHEMA, true)?;
    Some((gio::Settings::new(SCHEMA), schema))
}

fn style_row(ctx: &PageContext, settings: &gio::Settings) -> adw::ComboRow {
    let labels: Vec<&str> = STYLES.iter().map(|(_, label)| *label).collect();
    let row = adw::ComboRow::builder()
        .title("Style")
        .model(&gtk::StringList::new(&labels))
        .build();
    ctx.anchor("appearance.style", &row);

    let show = {
        let row = row.clone();
        move |settings: &gio::Settings| {
            let current = settings.string("color-scheme");
            let idx = STYLES.iter().position(|(value, _)| *value == current);
            row.set_selected(idx.unwrap_or(0) as u32);
        }
    };
    show(settings);
    settings.connect_changed(Some("color-scheme"), move |settings, _| show(settings));
    {
        let settings = settings.clone();
        row.connect_selected_notify(move |row| {
            if let Some((value, _)) = STYLES.get(row.selected() as usize) {
                if settings.string("color-scheme") != *value {
                    let _ = settings.set_string("color-scheme", value);
                }
            }
        });
    }
    row
}

fn accent_row(ctx: &PageContext, settings: &gio::Settings) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title("Accent Color").build();
    ctx.anchor("appearance.accent", &row);

    let swatches = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    swatches.set_valign(gtk::Align::Center);
    let mut buttons = Vec::new();
    for (value, label) in ACCENTS {
        let button = gtk::ToggleButton::builder()
            .tooltip_text(*label)
            .css_classes(["accent-swatch", &format!("accent-{value}")])
            .build();
        if let Some(first) = buttons.first() {
            button.set_group(Some(first));
        }
        swatches.append(&button);
        buttons.push(button);
    }
    row.add_suffix(&swatches);

    // Set while the buttons show the setting, so they don't write it back.
    let updating = Rc::new(Cell::new(false));
    let show = {
        let (buttons, updating) = (buttons.clone(), updating.clone());
        move |settings: &gio::Settings| {
            let current = settings.string("accent-color");
            updating.set(true);
            for ((value, _), button) in ACCENTS.iter().zip(&buttons) {
                button.set_active(*value == current);
            }
            updating.set(false);
        }
    };
    show(settings);
    settings.connect_changed(Some("accent-color"), move |settings, _| show(settings));
    for ((value, _), button) in ACCENTS.iter().zip(&buttons) {
        let (settings, updating) = (settings.clone(), updating.clone());
        button.connect_toggled(move |button| {
            if button.is_active() && !updating.get() {
                let _ = settings.set_string("accent-color", value);
            }
        });
    }
    row
}
