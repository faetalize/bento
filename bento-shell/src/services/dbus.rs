//! Small helpers for reading other programs' D-Bus objects.

use gio::prelude::*;
use gio::{
    BusNameWatcherFlags, BusType, DBusCallFlags, DBusConnection, DBusSignalFlags,
    SignalSubscription,
};
use glib::{Variant, VariantDict};

pub const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// Connects to a message bus, logging why when it can't.
pub async fn bus(bus_type: BusType) -> Option<DBusConnection> {
    match gio::bus_get_future(bus_type).await {
        Ok(bus) => Some(bus),
        Err(err) => {
            eprintln!("Couldn't connect to the {bus_type:?} bus: {err}");
            None
        }
    }
}

/// Reads all of an object's properties on `interface`.
pub async fn properties(
    bus: &DBusConnection,
    name: &str,
    path: &str,
    interface: &str,
) -> Result<VariantDict, glib::Error> {
    let reply = bus
        .call_future(
            Some(name),
            path,
            PROPERTIES,
            "GetAll",
            Some(&(interface,).to_variant()),
            Some(glib::VariantTy::new("(a{sv})").unwrap()),
            DBusCallFlags::NONE,
            -1,
        )
        .await?;
    Ok(VariantDict::new(Some(&reply.child_value(0))))
}

/// Reads one of an object's properties.
pub async fn property(
    bus: &DBusConnection,
    name: &str,
    path: &str,
    interface: &str,
    property: &str,
) -> Result<Variant, glib::Error> {
    let reply = bus
        .call_future(
            Some(name),
            path,
            PROPERTIES,
            "Get",
            Some(&(interface, property).to_variant()),
            Some(glib::VariantTy::new("(v)").unwrap()),
            DBusCallFlags::NONE,
            -1,
        )
        .await?;
    Ok(reply.child_value(0).as_variant().unwrap_or(reply))
}

/// Calls `changed` with the object path whenever `name` says that properties of one of its
/// objects changed.
pub fn on_properties_changed(
    bus: &DBusConnection,
    name: &str,
    changed: impl Fn(&str) + 'static,
) -> SignalSubscription {
    bus.subscribe_to_signal(
        Some(name),
        Some(PROPERTIES),
        Some("PropertiesChanged"),
        None,
        None,
        DBusSignalFlags::NONE,
        move |signal| changed(signal.object_path),
    )
}

/// Reads a value of type `T` from `dict`, or `None` when it's missing or another type.
pub fn lookup<T: glib::variant::FromVariant>(dict: &VariantDict, key: &str) -> Option<T> {
    dict.lookup_value(key, None).and_then(|value| value.get())
}

/// Follows whether a name has an owner on a bus, until it's dropped.
pub struct NameWatch(Option<Box<dyn FnOnce()>>);

/// Calls `appeared` with the bus whenever `name` gets an owner, and `vanished` when it has none,
/// which is also when the bus can't be reached.
pub fn watch_name(
    bus_type: BusType,
    name: &str,
    appeared: impl Fn(DBusConnection) + 'static,
    vanished: impl Fn() + 'static,
) -> NameWatch {
    let id = gio::bus_watch_name(
        bus_type,
        name,
        BusNameWatcherFlags::NONE,
        move |bus, _, _| appeared(bus),
        move |_, _| vanished(),
    );
    NameWatch(Some(Box::new(move || gio::bus_unwatch_name(id))))
}

/// Like [`watch_name`], on a connection that's already open.
pub fn watch_name_on(
    bus: &DBusConnection,
    name: &str,
    appeared: impl Fn(DBusConnection) + 'static,
    vanished: impl Fn() + 'static,
) -> NameWatch {
    let id = gio::bus_watch_name_on_connection(
        bus,
        name,
        BusNameWatcherFlags::NONE,
        move |bus, _, _| appeared(bus),
        move |_, _| vanished(),
    );
    NameWatch(Some(Box::new(move || gio::bus_unwatch_name(id))))
}

impl Drop for NameWatch {
    fn drop(&mut self) {
        if let Some(unwatch) = self.0.take() {
            unwatch();
        }
    }
}
