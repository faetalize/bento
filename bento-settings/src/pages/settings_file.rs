//! The settings file of an engine or shell of one's own: chosen as it's picked, and named in the
//! config next to it. A new one is empty, since the defaults are the engine's or the shell's, and
//! goes in a folder of bento's; a backup is used where it is.

use std::path::{Path, PathBuf};

use adw::prelude::*;
use gtk::gio;

use crate::page::PageContext;
use crate::store::tilde;

/// What has the settings file: an engine of one's own, or a shell.
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    Engine,
    Shell,
}

impl Kind {
    fn noun(self) -> &'static str {
        match self {
            Kind::Engine => "engine",
            Kind::Shell => "shell",
        }
    }

    /// The folder a new settings file goes in, next to the config.
    fn dir(self, ctx: &PageContext) -> PathBuf {
        let folder = match self {
            Kind::Engine => "engines",
            Kind::Shell => "shells",
        };
        ctx.store.config_dir().join(folder)
    }
}

/// Asks where the settings of the `kind` at `script` come from, and calls `done` with the file:
/// a new, empty one named after the script, or a backup the user picks. Cancelling calls
/// `cancelled` and changes nothing.
pub fn ask(
    ctx: &PageContext,
    parent: &impl IsA<gtk::Widget>,
    kind: Kind,
    script: &Path,
    done: impl Fn(PathBuf) + 'static,
    cancelled: impl Fn() + 'static,
) {
    let alert = adw::AlertDialog::new(
        Some("Settings File"),
        Some(&format!(
            "Start fresh with the {}'s defaults, or use settings you backed up.",
            kind.noun()
        )),
    );
    alert.add_response("cancel", "Cancel");
    alert.add_response("backup", "Use a Backup…");
    alert.add_response("fresh", "Start Fresh");
    alert.set_response_appearance("fresh", adw::ResponseAppearance::Suggested);
    alert.set_default_response(Some("fresh"));
    alert.set_close_response("cancel");

    let stem = script.file_stem().map_or_else(
        || String::from(kind.noun()),
        |s| s.to_string_lossy().into_owned(),
    );
    let (ctx, parent_widget) = (ctx.clone(), parent.clone().upcast::<gtk::Widget>());
    alert.choose(
        Some(parent),
        gio::Cancellable::NONE,
        move |response| match response.as_str() {
            "fresh" => match fresh(&kind.dir(&ctx), &stem) {
                Ok(path) => done(path),
                Err(err) => {
                    ctx.toast(&err);
                    cancelled();
                }
            },
            "backup" => pick_backup(&parent_widget, done, cancelled),
            _ => cancelled(),
        },
    );
}

/// Makes an empty settings file in `dir`, as `<stem>.kdl`, or `<stem>-2.kdl` and so on if that's
/// taken, so that no file is ever written over.
fn fresh(dir: &Path, stem: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("Couldn't make {}: {err}", tilde(dir)))?;
    for n in 1.. {
        let path = match n {
            1 => dir.join(format!("{stem}.kdl")),
            n => dir.join(format!("{stem}-{n}.kdl")),
        };
        match std::fs::File::options()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Ok(path),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(format!("Couldn't make {}: {err}", tilde(&path))),
        }
    }
    unreachable!("a name is free")
}

fn pick_backup(
    parent: &gtk::Widget,
    done: impl Fn(PathBuf) + 'static,
    cancelled: impl Fn() + 'static,
) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Settings"));
    filter.add_pattern("*.kdl");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title("Choose a Settings Backup")
        .filters(&filters)
        .build();
    let window = parent.root().and_downcast::<gtk::Window>();
    dialog.open(
        window.as_ref(),
        gio::Cancellable::NONE,
        move |result| match result.ok().and_then(|file| file.path()) {
            Some(path) => done(path),
            None => cancelled(),
        },
    );
}

/// The row that shows the settings file, which picks another when activated.
pub fn row(path: Option<&Path>) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .use_markup(false)
        .title("Settings File")
        .subtitle(path.map_or_else(|| String::from("None"), tilde))
        .activatable(true)
        .build();
    row.add_suffix(&gtk::Image::from_icon_name("ph-folder-open-symbolic"));
    row
}

/// Says the settings file is missing, when the config names one that isn't there.
pub fn missing(kind: Kind, path: &Path) -> Option<adw::PreferencesGroup> {
    if path.exists() {
        return None;
    }
    let group = adw::PreferencesGroup::new();
    let row = adw::ActionRow::builder()
        .use_markup(false)
        .title("Settings File Missing")
        .subtitle(format!(
            "{} isn't there, so the {} runs on its defaults. Pick it again above.",
            tilde(path),
            kind.noun()
        ))
        .subtitle_selectable(true)
        .build();
    group.add(&row);
    Some(group)
}
