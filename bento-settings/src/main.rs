//! Bento Settings: a GTK app for bento's config.

mod account;
mod ipc;
mod kdl;
mod model;
mod page;
mod pages;
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
