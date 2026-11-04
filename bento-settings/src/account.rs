//! The signed-in user, shown at the top of the sidebar.

use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{gio, glib};

pub struct Account {
    pub name: String,
    pub user: String,
    pub picture: Option<PathBuf>,
}

pub fn current() -> Account {
    let user = glib::user_name().to_string_lossy().into_owned();
    let mut name = glib::real_name().to_string_lossy().into_owned();
    let mut picture = None;

    // AccountsService has the name and picture that login screens and GNOME show.
    if let Some((real_name, icon)) = accounts_service(&user) {
        if !real_name.is_empty() {
            name = real_name;
        }
        picture = icon;
    }
    // GLib says "Unknown" when the system has no full name.
    if name.is_empty() || name == "Unknown" {
        name = user.clone();
    }
    let picture = picture
        .or_else(|| Some(glib::home_dir().join(".face")))
        .filter(|path| path.is_file());

    Account {
        name,
        user,
        picture,
    }
}

fn accounts_service(user: &str) -> Option<(String, Option<PathBuf>)> {
    const SERVICE: &str = "org.freedesktop.Accounts";
    let bus = gio::bus_get_sync(gio::BusType::System, None::<&gio::Cancellable>).ok()?;
    let call = |path: &str, interface: &str, method: &str, args: glib::Variant, reply: &str| {
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
    };

    let reply = call(
        "/org/freedesktop/Accounts",
        SERVICE,
        "FindUserByName",
        (user,).to_variant(),
        "(o)",
    )?;
    let (path,) = reply.get::<(glib::variant::ObjectPath,)>()?;
    let property = |name: &str| {
        let reply = call(
            path.as_str(),
            "org.freedesktop.DBus.Properties",
            "Get",
            ("org.freedesktop.Accounts.User", name).to_variant(),
            "(v)",
        )?;
        let (value,) = reply.get::<(glib::Variant,)>()?;
        value.get::<String>()
    };

    let name = property("RealName").unwrap_or_default();
    let picture = property("IconFile")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    Some((name, picture))
}
