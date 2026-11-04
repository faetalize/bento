//! The signed-in user's name and picture, from AccountsService like GNOME and login screens.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use gtk::prelude::*;
use gtk::{gio, glib};

const SERVICE: &str = "org.freedesktop.Accounts";
const USER: &str = "org.freedesktop.Accounts.User";

#[derive(Clone)]
pub struct Account {
    pub name: String,
    pub user: String,
    pub picture: Option<PathBuf>,
    /// `None` when AccountsService isn't there to say.
    pub administrator: Option<bool>,
    /// The user's AccountsService object, needed to change anything.
    path: Option<String>,
}

impl Account {
    pub fn can_edit(&self) -> bool {
        self.path.is_some()
    }
}

type Listener = Box<dyn Fn(&Account)>;

thread_local! {
    static LISTENERS: RefCell<Vec<Listener>> = RefCell::default();
}

/// Calls `f` with the account whenever the app changes it.
pub fn subscribe(f: impl Fn(&Account) + 'static) {
    LISTENERS.with(|listeners| listeners.borrow_mut().push(Box::new(f)));
}

fn changed() {
    let account = current();
    LISTENERS.with(|listeners| {
        for listener in listeners.borrow().iter() {
            listener(&account);
        }
    });
}

pub fn current() -> Account {
    let user = glib::user_name().to_string_lossy().into_owned();
    let mut account = Account {
        name: glib::real_name().to_string_lossy().into_owned(),
        user: user.clone(),
        picture: None,
        administrator: None,
        path: None,
    };

    if let Some(bus) = system_bus() {
        if let Some(path) = find_user(&bus, &user) {
            if let Some(name) = property(&bus, &path, "RealName").and_then(|v| v.get::<String>()) {
                account.name = name;
            }
            account.picture = property(&bus, &path, "IconFile")
                .and_then(|v| v.get::<String>())
                .filter(|path| !path.is_empty())
                .map(PathBuf::from);
            account.administrator = property(&bus, &path, "AccountType")
                .and_then(|v| v.get::<i32>())
                .map(|kind| kind == 1);
            account.path = Some(path);
        }
    }

    // GLib says "Unknown" when the system has no full name.
    if account.name.is_empty() || account.name == "Unknown" {
        account.name = user;
    }
    account.picture = account
        .picture
        .or_else(|| Some(glib::home_dir().join(".face")))
        .filter(|path| path.is_file());
    account
}

pub fn set_name(account: &Account, name: &str, done: impl FnOnce(Result<(), String>) + 'static) {
    call_user(account, "SetRealName", (name,).to_variant(), done);
}

/// Sets the picture to a copy of the image at `path`, or removes it for `None`.
pub fn set_picture(
    account: &Account,
    path: Option<&Path>,
    done: impl FnOnce(Result<(), String>) + 'static,
) {
    let path = path
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    call_user(account, "SetIconFile", (path,).to_variant(), done);
}

fn call_user(
    account: &Account,
    method: &str,
    args: glib::Variant,
    done: impl FnOnce(Result<(), String>) + 'static,
) {
    let (Some(bus), Some(path)) = (system_bus(), account.path.as_deref()) else {
        done(Err(String::from("AccountsService isn't running")));
        return;
    };
    bus.call(
        Some(SERVICE),
        path,
        USER,
        method,
        Some(&args),
        None,
        // Changing your own name or picture needs no password, but let polkit ask if a system
        // is set up to.
        gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
        -1,
        None::<&gio::Cancellable>,
        move |result| {
            let result = result.map(|_| ()).map_err(|mut err| {
                gio::DBusError::strip_remote_error(&mut err);
                err.message().to_owned()
            });
            if result.is_ok() {
                changed();
            }
            done(result);
        },
    );
}

fn system_bus() -> Option<gio::DBusConnection> {
    gio::bus_get_sync(gio::BusType::System, None::<&gio::Cancellable>).ok()
}

fn call(
    bus: &gio::DBusConnection,
    path: &str,
    interface: &str,
    method: &str,
    args: glib::Variant,
    reply: &str,
) -> Option<glib::Variant> {
    bus.call_sync(
        Some(SERVICE),
        path,
        interface,
        method,
        Some(&args),
        Some(glib::VariantTy::new(reply).unwrap()),
        gio::DBusCallFlags::NONE,
        500,
        None::<&gio::Cancellable>,
    )
    .ok()
}

fn find_user(bus: &gio::DBusConnection, user: &str) -> Option<String> {
    let reply = call(
        bus,
        "/org/freedesktop/Accounts",
        SERVICE,
        "FindUserByName",
        (user,).to_variant(),
        "(o)",
    )?;
    let (path,) = reply.get::<(glib::variant::ObjectPath,)>()?;
    Some(path.as_str().to_owned())
}

fn property(bus: &gio::DBusConnection, path: &str, name: &str) -> Option<glib::Variant> {
    let reply = call(
        bus,
        path,
        "org.freedesktop.DBus.Properties",
        "Get",
        (USER, name).to_variant(),
        "(v)",
    )?;
    let (value,) = reply.get::<(glib::Variant,)>()?;
    Some(value)
}
