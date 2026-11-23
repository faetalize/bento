//! The user and their places, as a start menu shows them: who the user is and their picture, the
//! files they opened lately, and the folders it has buttons for, each with its icon.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gio::prelude::*;
use gio::{BusType, DBusConnection, SignalSubscription};
use serde_json::{json, Value};

use super::{dbus, files};

const ACCOUNTS: &str = "org.freedesktop.Accounts";
const ACCOUNTS_PATH: &str = "/org/freedesktop/Accounts";
const USER: &str = "org.freedesktop.Accounts.User";

/// The most files opened lately that are told.
const RECENT: usize = 20;

/// The icons of the user's standard folders, by the names `user-dirs.dirs` gives them.
const ICONS: [(&str, &str); 8] = [
    ("$XDG_DESKTOP_DIR", "user-desktop-symbolic"),
    ("$XDG_DOCUMENTS_DIR", "folder-documents-symbolic"),
    ("$XDG_DOWNLOAD_DIR", "folder-download-symbolic"),
    ("$XDG_MUSIC_DIR", "folder-music-symbolic"),
    ("$XDG_PICTURES_DIR", "folder-pictures-symbolic"),
    ("$XDG_PUBLICSHARE_DIR", "folder-publicshare-symbolic"),
    ("$XDG_TEMPLATES_DIR", "folder-templates-symbolic"),
    ("$XDG_VIDEOS_DIR", "folder-videos-symbolic"),
];

type Say = fn(&Value);

/// Watches who the user is, through AccountsService when it runs, and the files they opened
/// lately, as GTK lists them.
pub struct Places {
    say: Say,
    watch: RefCell<Option<dbus::NameWatch>>,
    subscription: RefCell<Option<SignalSubscription>>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    /// A reading of the files opened lately that's waiting for their list to settle.
    pending: RefCell<Option<glib::SourceId>>,
}

impl Places {
    /// Starts watching. `say` hears `{ "type": "user", name, picture }` and
    /// `{ "type": "recent", files }`, and `folders` answers with `{ "type": "folders", folders }`.
    pub fn start(say: Say) -> Rc<Self> {
        let this = Rc::new(Self {
            say,
            watch: RefCell::default(),
            subscription: RefCell::default(),
            monitor: RefCell::default(),
            pending: RefCell::default(),
        });
        say(&fallback_user());

        let (appeared, vanished) = (Rc::downgrade(&this), Rc::downgrade(&this));
        let watch = dbus::watch_name(
            BusType::System,
            ACCOUNTS,
            move |bus| {
                if let Some(this) = appeared.upgrade() {
                    glib::spawn_future_local(async move { this.follow_user(bus).await });
                }
            },
            move || {
                if let Some(this) = vanished.upgrade() {
                    this.subscription.take();
                    (this.say)(&fallback_user());
                }
            },
        );
        *this.watch.borrow_mut() = Some(watch);

        // The list is written anew as a file is opened, often a few times at once.
        let list = gio::File::for_path(recent_list());
        match list.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
            Ok(monitor) => {
                let weak = Rc::downgrade(&this);
                monitor.connect_changed(move |_, _, _, _| {
                    let Some(this) = weak.upgrade() else { return };
                    if let Some(pending) = this.pending.take() {
                        pending.remove();
                    }
                    let weak = Rc::downgrade(&this);
                    let id = glib::timeout_add_local_once(Duration::from_millis(300), move || {
                        if let Some(this) = weak.upgrade() {
                            this.pending.take();
                            (this.say)(&recent());
                        }
                    });
                    *this.pending.borrow_mut() = Some(id);
                });
                *this.monitor.borrow_mut() = Some(monitor);
            }
            Err(err) => eprintln!("bento-shell --data: can't watch the files opened lately: {err}"),
        }
        say(&recent());
        this
    }

    /// Reads the user's name and picture from AccountsService, and again as they change.
    async fn follow_user(self: Rc<Self>, bus: DBusConnection) {
        let name = glib::user_name().to_string_lossy().into_owned();
        let Ok(reply) = dbus::call(
            &bus,
            ACCOUNTS,
            ACCOUNTS_PATH,
            ACCOUNTS,
            "FindUserByName",
            Some((name,).to_variant()),
        )
        .await
        else {
            return;
        };
        let Some(path) = reply.child_value(0).str().map(String::from) else {
            return;
        };
        let weak = Rc::downgrade(&self);
        let (user, listened) = (path.clone(), bus.clone());
        let subscription = dbus::on_properties_changed(&bus, ACCOUNTS, move |changed| {
            if changed != user {
                return;
            }
            let Some(this) = weak.upgrade() else { return };
            let (bus, path) = (listened.clone(), user.clone());
            glib::spawn_future_local(async move { this.read_user(&bus, &path).await });
        });
        *self.subscription.borrow_mut() = Some(subscription);
        self.read_user(&bus, &path).await;
    }

    async fn read_user(&self, bus: &DBusConnection, path: &str) {
        let Ok(props) = dbus::properties(bus, ACCOUNTS, path, USER).await else {
            return;
        };
        let real: Option<String> = dbus::lookup(&props, "RealName");
        let picture: Option<String> = dbus::lookup(&props, "IconFile");
        let mut user = fallback_user();
        if let Some(real) = real.filter(|name| !name.trim().is_empty()) {
            user["name"] = json!(real);
        }
        if let Some(picture) = picture.filter(|path| readable(Path::new(path))) {
            user["picture"] = json!(picture);
        }
        (self.say)(&user);
    }

    /// Tells the folders of `written`, as folder settings write them, that are there, in order,
    /// each with its name and icon.
    pub fn folders(&self, written: &[String]) {
        let home = files::resolve(&glib::home_dir());
        let standard: Vec<(PathBuf, &str)> = files::STANDARD
            .iter()
            .zip(ICONS)
            .filter_map(|((_, dir), (_, icon))| Some((glib::user_special_dir(*dir)?, icon)))
            .map(|(path, icon)| (files::resolve(&path), icon))
            // One set to the home folder is turned off.
            .filter(|(path, _)| *path != home)
            .collect();
        let folders: Vec<Value> = written
            .iter()
            .filter_map(|written| {
                let path = files::expand(written)?;
                let resolved = files::resolve(&path);
                if !resolved.is_dir() {
                    return None;
                }
                let icon = if resolved == home {
                    if written != "~" {
                        // A standard folder turned off, or a folder written as the home one.
                        if files::STANDARD.iter().any(|(name, _)| name == written) {
                            return None;
                        }
                    }
                    "user-home-symbolic"
                } else {
                    standard
                        .iter()
                        .find(|(standard, _)| *standard == resolved)
                        .map_or("folder-symbolic", |(_, icon)| icon)
                };
                let name = if resolved == home {
                    String::from("Home")
                } else {
                    path.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string())
                };
                Some(json!({
                    "written": written,
                    "path": path.to_string_lossy(),
                    "name": name,
                    "icon": icon,
                }))
            })
            .collect();
        (self.say)(&json!({ "type": "folders", "folders": folders }));
    }
}

/// Who the user is without AccountsService: their name from the system, and `~/.face` as their
/// picture, if it's there.
fn fallback_user() -> Value {
    let real = glib::real_name().to_string_lossy().into_owned();
    let name = if real.trim().is_empty() || real == "Unknown" {
        glib::user_name().to_string_lossy().into_owned()
    } else {
        real
    };
    let face = glib::home_dir().join(".face");
    let picture = readable(&face).then(|| face.to_string_lossy().into_owned());
    json!({ "type": "user", "name": name, "picture": picture })
}

fn readable(path: &Path) -> bool {
    std::fs::File::open(path).is_ok_and(|file| file.metadata().is_ok_and(|meta| meta.is_file()))
}

/// The list of files opened lately, which GTK keeps.
fn recent_list() -> PathBuf {
    glib::user_data_dir().join("recently-used.xbel")
}

/// The files opened lately that are still there, the latest first, `RECENT` at most, each with
/// when it was opened, in UTC as ISO 8601.
fn recent() -> Value {
    let list = std::fs::read_to_string(recent_list())
        .map(|xbel| files::read_recent(&xbel))
        .unwrap_or_default();
    let mut list: Vec<(PathBuf, String)> = list.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1));
    let found: Vec<Value> = list
        .into_iter()
        .filter_map(|(path, opened)| {
            let meta = std::fs::metadata(&path).ok()?;
            let mut file = files::describe(&path.to_string_lossy(), meta.is_dir());
            file["opened"] = json!(opened);
            Some(file)
        })
        .take(RECENT)
        .collect();
    json!({ "type": "recent", "files": found })
}
