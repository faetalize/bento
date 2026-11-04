use adw::prelude::*;

use crate::ipc;
use crate::model::Category;
use crate::page::{open_file, open_uri, PageContext};
use crate::store::tilde;

const REPOSITORY: &str = "https://github.com/faetalize/bento";
const NIRI_DOCS: &str = "https://niri-wm.github.io/niri/Configuration:-Introduction";

pub fn category() -> Category {
    Category::custom("about", "About", build)
        .icon("help-about-symbolic")
        .keywords(&["version", "license", "help", "info"])
        .search_item(
            "about.version",
            "Version",
            &["bento version", "compositor", "update"],
        )
        .search_item("about.files", "Config Files", &["config.kdl", "folder"])
        .search_item(
            "about.links",
            "Source Code",
            &["github", "issues", "bugs", "report"],
        )
}

fn build(ctx: &PageContext) -> gtk::Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 24);

    let versions = adw::PreferencesGroup::new();
    ctx.anchor("about.version", &versions);
    let version = ipc::version().unwrap_or_else(|_| String::from(env!("CARGO_PKG_VERSION")));
    versions.add(&info_row("Version", &version));
    versions.add(&info_row("License", env!("CARGO_PKG_LICENSE")));
    column.append(&versions);

    let files = adw::PreferencesGroup::new();
    ctx.anchor("about.files", &files);
    let config = ctx.store.config_path().to_owned();
    let folder = config.parent().map(|p| p.to_owned()).unwrap_or_default();
    files.add(&file_row("Config", &config));
    files.add(&file_row("Config Folder", &folder));
    column.append(&files);

    let links = adw::PreferencesGroup::new();
    ctx.anchor("about.links", &links);
    links.add(&link_row("Source Code", REPOSITORY));
    links.add(&link_row(
        "Report a Problem",
        &format!("{REPOSITORY}/issues"),
    ));
    links.add(&link_row("niri Documentation", NIRI_DOCS));
    column.append(&links);

    column.upcast()
}

fn info_row(title: &str, value: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).build();
    let label = gtk::Label::builder()
        .label(value)
        .selectable(true)
        .valign(gtk::Align::Center)
        .css_classes(["dim-label"])
        .build();
    row.add_suffix(&label);
    row
}

fn file_row(title: &str, path: &std::path::Path) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(tilde(path))
        .activatable(true)
        .build();
    row.add_suffix(&gtk::Image::from_icon_name("document-open-symbolic"));
    let path = path.to_owned();
    row.connect_activated(move |row| open_file(row, &path));
    row
}

fn link_row(title: &str, uri: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .activatable(true)
        .build();
    row.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
    let uri = uri.to_owned();
    row.connect_activated(move |row| open_uri(row, &uri));
    row
}
