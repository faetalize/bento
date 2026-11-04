//! Bento Settings: a GTK app for bento's config.

mod account;
mod ipc;
mod kdl;
mod model;
mod page;
mod pages;
mod password;
mod search;
mod store;
mod window;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::store::Store;

const APP_ID: &str = "io.github.faetalize.bento.Settings";

const USAGE: &str = "\
Usage: bento-settings [PAGE] [--search QUERY]

Opens bento's settings, on PAGE if given, like `bento-settings engine`.
--search starts a search for QUERY.
";

type OpenWindow = (adw::ApplicationWindow, Rc<window::Ui>);

fn main() -> glib::ExitCode {
    gio::resources_register_include!("resources.gresource").expect("resources are built in");

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    app.connect_startup(|app| {
        let css = gtk::CssProvider::new();
        css.load_from_string(include_str!("style.css"));
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            // Phosphor icons, bundled in the binary (MIT, see data/icons/LICENSE).
            gtk::IconTheme::for_display(&display)
                .add_resource_path("/io/github/faetalize/bento/Settings/icons");
            follow_user_css(&display);
        }

        let quit = gio::ActionEntry::builder("quit")
            .activate(|app: &adw::Application, _, _| app.quit())
            .build();
        app.add_action_entries([quit]);
        app.set_accels_for_action("app.quit", &["<Control>q"]);
    });

    let open: Rc<RefCell<Option<OpenWindow>>> = Rc::default();
    app.connect_command_line(move |app, command_line| {
        let mut page = None;
        let mut query = None;
        let mut args = command_line
            .arguments()
            .into_iter()
            .skip(1)
            .map(|arg| arg.to_string_lossy().into_owned());
        while let Some(arg) = args.next() {
            if arg == "-h" || arg == "--help" {
                command_line.print_literal(USAGE);
                return glib::ExitCode::SUCCESS;
            } else if arg == "--search" {
                query = args.next();
            } else if let Some(rest) = arg.strip_prefix("--search=") {
                query = Some(rest.to_owned());
            } else {
                page = Some(arg);
            }
        }

        // A second launch reuses the open window.
        let (window, ui) = open
            .borrow_mut()
            .get_or_insert_with(|| {
                // Pages live as long as the app, so the catalog does too.
                let sections = Box::leak(pages::sections().into_boxed_slice());
                window::build(app, sections, Store::load())
            })
            .clone();
        window.present();

        if let Some(page) = page {
            if !ui.open_page(&page) {
                command_line.printerr_literal(&format!("There's no page called {page}.\n"));
            }
        }
        if let Some(query) = query {
            ui.start_search(&query);
        }
        glib::ExitCode::SUCCESS
    });

    app.run()
}

/// Reloads the user's GTK stylesheet when it changes. GTK reads it only at startup, but shells like
/// Noctalia rewrite it to switch between light and dark.
fn follow_user_css(display: &gdk::Display) {
    let dir = glib::user_config_dir().join("gtk-4.0");
    let css = dir.join("gtk.css");
    // Empty until the file changes, since GTK already loaded what's there now. Above GTK's own
    // copy, so the new rules win.
    let provider = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_USER + 1,
    );

    let Ok(monitor) = gio::File::for_path(&dir).monitor_directory(
        gio::FileMonitorFlags::WATCH_MOVES,
        None::<&gio::Cancellable>,
    ) else {
        return;
    };
    // Shells write several files at once; reload once they're done.
    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    monitor.connect_changed(move |_, _, _, _| {
        if let Some(source) = pending.take() {
            source.remove();
        }
        let (provider, css, pending_done) = (provider.clone(), css.clone(), pending.clone());
        let source =
            glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
                pending_done.take();
                if css.exists() {
                    provider.load_from_path(&css);
                }
            });
        pending.replace(Some(source));
    });
    // The monitor lives as long as the app.
    std::mem::forget(monitor);
}
