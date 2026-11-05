//! Moving from niri: converts a niri `config.kdl` into bento's config and the niri engine's file.
//!
//! The conversion and the writing are `niri_config::convert`'s, which `bento migrate-from-niri`
//! uses too. This is the row that asks for the file and the dialog that says what happens.

use std::path::Path;

use adw::prelude::*;
use gtk::{gio, glib};
use niri_config::convert::{self, Conversion};

use crate::page::PageContext;
use crate::store::describe;

pub fn group(ctx: &PageContext) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    ctx.anchor("engine.migrate", &group);

    let row = adw::ActionRow::builder()
        .title("Migrate from niri…")
        .subtitle("Convert a niri config into bento's config and the niri engine's file")
        .activatable(true)
        .build();
    row.add_suffix(&gtk::Image::from_icon_name("ph-folder-open-symbolic"));
    let ctx = ctx.clone();
    row.connect_activated(move |row| choose(&ctx, row));
    group.add(&row);
    group
}

/// Asks for niri's config, starting at where niri keeps it.
fn choose(ctx: &PageContext, widget: &impl IsA<gtk::Widget>) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("KDL Configs"));
    filter.add_pattern("*.kdl");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title("Choose a niri Config")
        .filters(&filters)
        .build();
    let niri_config = glib::user_config_dir().join("niri").join("config.kdl");
    dialog.set_initial_file(Some(&gio::File::for_path(niri_config)));

    let window = widget.root().and_downcast::<gtk::Window>();
    let ctx = ctx.clone();
    let parent = window.clone();
    dialog.open(window.as_ref(), None::<&gio::Cancellable>, move |result| {
        if let Some(path) = result.ok().and_then(|file| file.path()) {
            convert_file(&ctx, parent.as_ref(), &path);
        }
    });
}

fn convert_file(ctx: &PageContext, window: Option<&gtk::Window>, path: &Path) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            let body = format!("{}: {err}", path.display());
            alert(window, "Couldn't Read the Config", &body);
            return;
        }
    };
    match convert::convert(&text) {
        Ok(conversion) => confirm(ctx, window, conversion),
        Err(err) => alert(
            window,
            "Couldn't Convert the Config",
            &describe(err.as_ref()),
        ),
    }
}

/// Says what will be replaced and what couldn't be converted, and migrates if that's fine.
fn confirm(ctx: &PageContext, window: Option<&gtk::Window>, conversion: Conversion) {
    let mut body = String::from(
        "bento's config and the niri engine's file are replaced with the converted ones. The \
         current files are kept, with .before-migration added to their names.",
    );
    if !conversion.notes.is_empty() {
        body.push_str("\n\nSome things couldn't be converted:");
    }
    let dialog = adw::AlertDialog::new(Some("Migrate from niri?"), Some(&body));

    if !conversion.notes.is_empty() {
        let notes: Vec<String> = conversion.notes.iter().map(|n| format!("• {n}")).collect();
        let label = gtk::Label::builder()
            .label(notes.join("\n"))
            .wrap(true)
            .xalign(0.)
            .selectable(true)
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&label)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(240)
            .build();
        dialog.set_extra_child(Some(&scrolled));
    }

    dialog.add_responses(&[("cancel", "Cancel"), ("migrate", "Migrate")]);
    dialog.set_response_appearance("migrate", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let ctx = ctx.clone();
    let parent = window.cloned();
    dialog.connect_response(Some("migrate"), move |_, _| {
        migrate(&ctx, parent.as_ref(), &conversion);
    });
    dialog.present(window);
}

fn migrate(ctx: &PageContext, window: Option<&gtk::Window>, conversion: &Conversion) {
    // Changes still waiting to be saved go in first, so they're in the backup and not lost.
    ctx.store.flush();
    match convert::write(conversion, ctx.store.config_path()) {
        Ok(_) => {
            // bento reloads the files itself; the app does too, to show the new engine.
            ctx.store.reload();
            ctx.toast("Migrated from niri");
        }
        Err(err) => alert(window, "Couldn't Write the Config", &err.to_string()),
    }
}

fn alert(window: Option<&gtk::Window>, heading: &str, body: &str) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("close", "Close");
    dialog.present(window);
}
