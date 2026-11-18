//! Bluetooth, through BlueZ: whether the adapter is on, the devices it knows and finds, and
//! connecting them. A device that isn't paired yet is paired, trusted and connected in one go,
//! which works for the devices that pair without a code, like most headphones, mice and
//! keyboards' receivers.
//!
//! BlueZ is only watched while it runs; this never starts it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gio::prelude::*;
use gio::{BusType, DBusConnection, DBusSignalFlags, SignalSubscription};
use glib::{Variant, VariantDict};
use serde_json::{json, Value};

use super::dbus;

const SERVICE: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
/// How many devices the shell is told of.
const DEVICES: usize = 16;

/// A device, as the shell shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub path: String,
    pub name: String,
    pub icon: String,
    pub paired: bool,
    pub connected: bool,
}

#[derive(Debug, Default, Clone, PartialEq)]
struct State {
    adapter: Option<String>,
    powered: bool,
    discovering: bool,
    devices: Vec<Device>,
}

pub struct BluetoothService {
    watch: RefCell<Option<dbus::NameWatch>>,
    bus: RefCell<Option<DBusConnection>>,
    subscriptions: RefCell<Vec<SignalSubscription>>,
    /// The device being connected, to show it as busy.
    connecting: RefCell<Option<String>>,
    soon: dbus::Soon,
    generation: Cell<u64>,
    state: RefCell<State>,
    on_change: Box<dyn Fn(&Value)>,
}

impl BluetoothService {
    /// Starts watching Bluetooth. `on_change` hears the state, and `{ "type": "bluetooth-error",
    /// message }` values when connecting fails.
    pub fn start(on_change: impl Fn(&Value) + 'static) -> Rc<Self> {
        let this = Rc::new(Self {
            watch: RefCell::default(),
            bus: RefCell::default(),
            subscriptions: RefCell::default(),
            connecting: RefCell::default(),
            soon: dbus::Soon::default(),
            generation: Cell::default(),
            state: RefCell::default(),
            on_change: Box::new(on_change),
        });
        this.publish();
        let (appeared, vanished) = (Rc::downgrade(&this), Rc::downgrade(&this));
        let watch = dbus::watch_name(
            BusType::System,
            SERVICE,
            move |bus| {
                if let Some(this) = appeared.upgrade() {
                    this.appeared(bus);
                }
            },
            move || {
                if let Some(this) = vanished.upgrade() {
                    this.subscriptions.take();
                    this.bus.take();
                    this.generation.set(this.generation.get() + 1);
                    *this.state.borrow_mut() = State::default();
                    this.publish();
                }
            },
        );
        *this.watch.borrow_mut() = Some(watch);
        this
    }

    fn appeared(self: &Rc<Self>, bus: DBusConnection) {
        let weak = Rc::downgrade(self);
        let properties = dbus::on_properties_changed(&bus, SERVICE, move |_| {
            if let Some(this) = weak.upgrade() {
                this.refresh_soon();
            }
        });
        let weak = Rc::downgrade(self);
        let objects = bus.subscribe_to_signal(
            Some(SERVICE),
            Some(OBJECT_MANAGER),
            None,
            None,
            None,
            DBusSignalFlags::NONE,
            move |_| {
                if let Some(this) = weak.upgrade() {
                    this.refresh_soon();
                }
            },
        );
        *self.subscriptions.borrow_mut() = vec![properties, objects];
        *self.bus.borrow_mut() = Some(bus);
        self.refresh();
    }

    fn refresh_soon(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.soon.ask(Duration::from_millis(250), move || {
            if let Some(this) = weak.upgrade() {
                this.refresh();
            }
        });
    }

    fn refresh(self: &Rc<Self>) {
        let Some(bus) = self.bus.borrow().clone() else {
            return;
        };
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let reply = dbus::call(
                &bus,
                SERVICE,
                "/",
                OBJECT_MANAGER,
                "GetManagedObjects",
                None,
            )
            .await;
            let Some(this) = weak.upgrade() else { return };
            if this.generation.get() != generation {
                return;
            }
            let state = reply
                .map(|reply| read(&reply.child_value(0)))
                .unwrap_or_default();
            if *this.state.borrow() != state {
                *this.state.borrow_mut() = state;
                this.publish();
            }
        });
    }

    fn publish(&self) {
        let state = self.state.borrow();
        let connecting = self.connecting.borrow().clone();
        let devices: Vec<Value> = state
            .devices
            .iter()
            .map(|d| {
                json!({
                    "path": d.path, "name": d.name, "icon": d.icon, "paired": d.paired,
                    "connected": d.connected,
                    "connecting": connecting.as_deref() == Some(d.path.as_str()),
                })
            })
            .collect();
        (self.on_change)(&json!({
            "type": "bluetooth",
            "available": state.adapter.is_some(),
            "powered": state.powered,
            "discovering": state.discovering,
            "devices": devices,
        }));
    }

    fn with_adapter(
        self: &Rc<Self>,
        f: impl AsyncFnOnce(DBusConnection, String, Rc<Self>) + 'static,
    ) {
        let (Some(bus), Some(adapter)) = (
            self.bus.borrow().clone(),
            self.state.borrow().adapter.clone(),
        ) else {
            return;
        };
        let this = self.clone();
        glib::spawn_future_local(async move { f(bus, adapter, this).await });
    }

    /// Turns the adapter on or off.
    pub fn power(self: &Rc<Self>, on: bool) {
        self.with_adapter(async move |bus, adapter, this| {
            let set =
                dbus::set_property(&bus, SERVICE, &adapter, ADAPTER, "Powered", on.to_variant());
            if let Err(err) = set.await {
                this.error(err.message());
            }
        });
    }

    /// Looks for devices nearby, or stops.
    pub fn scan(self: &Rc<Self>, on: bool) {
        self.with_adapter(async move |bus, adapter, _| {
            let method = if on {
                "StartDiscovery"
            } else {
                "StopDiscovery"
            };
            let _ = dbus::call(&bus, SERVICE, &adapter, ADAPTER, method, None).await;
        });
    }

    /// Connects the device at `path`, pairing and trusting it first if it's new.
    pub fn connect(self: &Rc<Self>, path: String) {
        let paired = self
            .state
            .borrow()
            .devices
            .iter()
            .any(|d| d.path == path && d.paired);
        *self.connecting.borrow_mut() = Some(path.clone());
        self.publish();
        self.with_adapter(async move |bus, _, this| {
            let mut result = Ok(());
            if !paired {
                result = dbus::call(&bus, SERVICE, &path, DEVICE, "Pair", None)
                    .await
                    .map(|_| ());
                if result.is_ok() {
                    let trust = dbus::set_property(
                        &bus,
                        SERVICE,
                        &path,
                        DEVICE,
                        "Trusted",
                        true.to_variant(),
                    );
                    let _ = trust.await;
                }
            }
            if result.is_ok() {
                result = dbus::call(&bus, SERVICE, &path, DEVICE, "Connect", None)
                    .await
                    .map(|_| ());
            }
            this.connecting.take();
            this.publish();
            if let Err(err) = result {
                this.error(err.message());
            }
        });
    }

    /// Disconnects the device at `path`.
    pub fn disconnect(self: &Rc<Self>, path: String) {
        self.with_adapter(async move |bus, _, _| {
            let _ = dbus::call(&bus, SERVICE, &path, DEVICE, "Disconnect", None).await;
        });
    }

    /// Forgets the device at `path`.
    pub fn forget(self: &Rc<Self>, path: String) {
        self.with_adapter(async move |bus, adapter, _| {
            let Ok(device) = glib::variant::ObjectPath::try_from(path) else {
                return;
            };
            let args = (device,).to_variant();
            let _ = dbus::call(&bus, SERVICE, &adapter, ADAPTER, "RemoveDevice", Some(args)).await;
        });
    }

    fn error(&self, message: &str) {
        (self.on_change)(&json!({ "type": "bluetooth-error", "message": message }));
    }
}

/// The state, from `GetManagedObjects`'s `a{oa{sa{sv}}}`.
fn read(objects: &Variant) -> State {
    let mut state = State::default();
    for object in objects.iter() {
        let path = object.child_value(0).str().unwrap_or_default().to_owned();
        let interfaces = object.child_value(1);
        let interface = |name: &str| {
            interfaces
                .iter()
                .find(|entry| entry.child_value(0).str() == Some(name))
                .map(|entry| VariantDict::new(Some(&entry.child_value(1))))
        };
        if let (Some(adapter), None) = (interface(ADAPTER), &state.adapter) {
            state.powered = dbus::lookup::<bool>(&adapter, "Powered").unwrap_or(false);
            state.discovering = dbus::lookup::<bool>(&adapter, "Discovering").unwrap_or(false);
            state.adapter = Some(path.clone());
        }
        if let Some(device) = interface(DEVICE) {
            let address = dbus::lookup::<String>(&device, "Address").unwrap_or_default();
            let name = dbus::lookup::<String>(&device, "Alias")
                .or_else(|| dbus::lookup::<String>(&device, "Name"))
                .unwrap_or_default();
            let paired = dbus::lookup::<bool>(&device, "Paired").unwrap_or(false);
            // A device found nearby that only says its address isn't worth listing.
            if !paired && (name.is_empty() || unnamed(&name, &address)) {
                continue;
            }
            state.devices.push(Device {
                path,
                name,
                icon: icon(&dbus::lookup::<String>(&device, "Icon").unwrap_or_default()),
                paired,
                connected: dbus::lookup::<bool>(&device, "Connected").unwrap_or(false),
            });
        }
    }
    state.devices.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.paired.cmp(&a.paired))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    state.devices.truncate(DEVICES);
    state
}

/// Whether `name` is only the device's address, as BlueZ names a device that says no name.
fn unnamed(name: &str, address: &str) -> bool {
    name.replace(['-', ':'], "")
        .eq_ignore_ascii_case(&address.replace(':', ""))
}

/// A symbolic icon for BlueZ's icon name, like `audio-headset`.
fn icon(name: &str) -> String {
    let known = [
        "audio-headset",
        "audio-headphones",
        "audio-speakers",
        "audio-card",
        "input-mouse",
        "input-keyboard",
        "input-gaming",
        "input-tablet",
        "phone",
        "computer",
        "camera-photo",
        "printer",
    ];
    if known.contains(&name) {
        format!("{name}-symbolic")
    } else {
        String::from("bluetooth-symbolic")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_that_only_say_their_address_are_left_out() {
        assert!(unnamed("A1-B2-C3-D4-E5-F6", "A1:B2:C3:D4:E5:F6"));
        assert!(!unnamed("Headphones", "A1:B2:C3:D4:E5:F6"));
    }

    #[test]
    fn a_devices_icon_is_symbolic_or_bluetooths() {
        assert_eq!(icon("audio-headset"), "audio-headset-symbolic");
        assert_eq!(icon("something-else"), "bluetooth-symbolic");
    }
}
