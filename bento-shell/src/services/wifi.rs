//! Wi-Fi, through NetworkManager: whether it's on, the networks around, and joining or leaving
//! one. A network that's new and has a password gets it from the shell, and if joining it fails
//! the connection made for it is removed, so a mistyped password isn't kept.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gio::prelude::*;
use gio::{BusType, DBusConnection, DBusSignalFlags, SignalSubscription};
use glib::{Variant, VariantDict};
use serde_json::{json, Value};

use super::dbus;

const SERVICE: &str = "org.freedesktop.NetworkManager";
const MANAGER: &str = "org.freedesktop.NetworkManager";
const MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";
const DEVICE: &str = "org.freedesktop.NetworkManager.Device";
const WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const ACCESS_POINT: &str = "org.freedesktop.NetworkManager.AccessPoint";
const SETTINGS: &str = "org.freedesktop.NetworkManager.Settings";
const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const CONNECTION: &str = "org.freedesktop.NetworkManager.Settings.Connection";

/// `NM_DEVICE_TYPE_WIFI`.
const WIFI: u32 = 2;
/// `NM_DEVICE_STATE_ACTIVATED` and `NM_DEVICE_STATE_FAILED`; the states between
/// `NM_DEVICE_STATE_PREPARE` and activated are joining.
const ACTIVATED: u32 = 100;
const FAILED: u32 = 120;
const PREPARE: u32 = 40;
/// How many networks the shell is told of, strongest first.
const NETWORKS: usize = 16;
/// How long a join may take before it counts as failed.
const JOIN_TIME: Duration = Duration::from_secs(30);

/// A network around, as the shell shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Network {
    pub ssid: String,
    /// From 0 to 100.
    pub strength: u8,
    pub secure: bool,
    /// Whether there's a saved connection for it.
    pub known: bool,
    pub active: bool,
}

/// What the device and NetworkManager say, before it's turned into what the shell shows.
#[derive(Debug, Default, Clone, PartialEq)]
struct State {
    available: bool,
    enabled: bool,
    /// The device's state, `NM_DEVICE_STATE_*`.
    device_state: u32,
    networks: Vec<Network>,
}

/// A join on its way: the network, the connection made for it if it was new, and when it
/// started.
struct Joining {
    ssid: String,
    added: Option<String>,
    started: std::time::Instant,
    /// Whether the device has been seen joining, so a disconnected device after it means it
    /// gave up.
    seen: bool,
}

pub struct WifiService {
    watch: RefCell<Option<dbus::NameWatch>>,
    bus: RefCell<Option<DBusConnection>>,
    subscriptions: RefCell<Vec<SignalSubscription>>,
    device: RefCell<Option<String>>,
    /// Saved connections by the SSID they're for.
    known: RefCell<HashMap<String, String>>,
    joining: RefCell<Option<Joining>>,
    soon: dbus::Soon,
    generation: Cell<u64>,
    state: RefCell<State>,
    on_change: Box<dyn Fn(&Value)>,
}

impl WifiService {
    /// Starts watching Wi-Fi. `on_change` hears the state, and `{ "type": "wifi-error", ssid,
    /// message }` values when joining fails.
    pub fn start(on_change: impl Fn(&Value) + 'static) -> Rc<Self> {
        let this = Rc::new(Self {
            watch: RefCell::default(),
            bus: RefCell::default(),
            subscriptions: RefCell::default(),
            device: RefCell::default(),
            known: RefCell::default(),
            joining: RefCell::default(),
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
                    this.device.take();
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
        // Any of NetworkManager's objects changing may change what's shown: the manager, the
        // device, its access points and the saved connections.
        let weak = Rc::downgrade(self);
        let properties = dbus::on_properties_changed(&bus, SERVICE, move |_| {
            if let Some(this) = weak.upgrade() {
                this.refresh_soon();
            }
        });
        let weak = Rc::downgrade(self);
        let signals = bus.subscribe_to_signal(
            Some(SERVICE),
            None,
            None,
            None,
            None,
            DBusSignalFlags::NONE,
            move |signal| {
                let interesting = matches!(
                    signal.signal_name,
                    "AccessPointAdded"
                        | "AccessPointRemoved"
                        | "NewConnection"
                        | "ConnectionRemoved"
                        | "StateChanged"
                        | "DeviceAdded"
                        | "DeviceRemoved"
                );
                if let (true, Some(this)) = (interesting, weak.upgrade()) {
                    this.refresh_soon();
                }
            },
        );
        *self.subscriptions.borrow_mut() = vec![properties, signals];
        *self.bus.borrow_mut() = Some(bus);
        self.refresh();
    }

    fn refresh_soon(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.soon.ask(Duration::from_millis(400), move || {
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
            let read = read(&bus).await;
            let Some(this) = weak.upgrade() else { return };
            if this.generation.get() != generation {
                return;
            }
            let (state, device, known) = read;
            *this.device.borrow_mut() = device;
            *this.known.borrow_mut() = known;
            this.follow_join(&state);
            let changed = *this.state.borrow() != state;
            *this.state.borrow_mut() = state;
            if changed {
                this.publish();
            }
        });
    }

    /// Ends a join that worked or failed, removing a new network's connection if it failed.
    fn follow_join(self: &Rc<Self>, state: &State) {
        let mut joining = self.joining.borrow_mut();
        let Some(join) = joining.as_mut() else { return };
        let active = state
            .networks
            .iter()
            .any(|n| n.active && n.ssid == join.ssid);
        if active && state.device_state == ACTIVATED {
            *joining = None;
            return;
        }
        let busy = (PREPARE..ACTIVATED).contains(&state.device_state);
        join.seen |= busy;
        let failed = state.device_state == FAILED
            || (join.seen && !busy && !active)
            || join.started.elapsed() > JOIN_TIME;
        if !failed {
            return;
        }
        let join = joining.take().unwrap();
        drop(joining);
        if let (Some(added), Some(bus)) = (join.added, self.bus.borrow().clone()) {
            glib::spawn_future_local(async move {
                let _ = dbus::call(&bus, SERVICE, &added, CONNECTION, "Delete", None).await;
            });
        }
        (self.on_change)(&json!({
            "type": "wifi-error",
            "ssid": join.ssid,
            "message": "Couldn't join the network. Check its password.",
        }));
    }

    fn publish(&self) {
        let state = self.state.borrow();
        let networks: Vec<Value> = state
            .networks
            .iter()
            .map(|n| {
                json!({
                    "ssid": n.ssid, "strength": n.strength, "secure": n.secure,
                    "known": n.known, "active": n.active,
                })
            })
            .collect();
        let joining = self.joining.borrow().as_ref().map(|join| join.ssid.clone());
        (self.on_change)(&json!({
            "type": "wifi",
            "available": state.available,
            "enabled": state.enabled,
            "joining": joining,
            "networks": networks,
        }));
    }

    fn with_bus(self: &Rc<Self>, f: impl AsyncFnOnce(DBusConnection, Rc<Self>) + 'static) {
        let Some(bus) = self.bus.borrow().clone() else {
            return;
        };
        let this = self.clone();
        glib::spawn_future_local(async move { f(bus, this).await });
    }

    /// Turns Wi-Fi on or off.
    pub fn enable(self: &Rc<Self>, on: bool) {
        self.with_bus(async move |bus, _| {
            let value = on.to_variant();
            let _ = dbus::set_property(
                &bus,
                SERVICE,
                MANAGER_PATH,
                MANAGER,
                "WirelessEnabled",
                value,
            )
            .await;
        });
    }

    /// Looks for networks again.
    pub fn scan(self: &Rc<Self>) {
        let Some(device) = self.device.borrow().clone() else {
            return;
        };
        self.with_bus(async move |bus, _| {
            let options = VariantDict::new(None).end();
            // NetworkManager refuses scans too close together, which is fine.
            let _ = bus
                .call_future(
                    Some(SERVICE),
                    &device,
                    WIRELESS,
                    "RequestScan",
                    Some(&(options,).to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    -1,
                )
                .await;
        });
    }

    /// Joins the network `ssid`, with `password` if it's new and has one.
    pub fn join(self: &Rc<Self>, ssid: String, password: Option<String>) {
        let Some(device) = self.device.borrow().clone() else {
            return;
        };
        let known = self.known.borrow().get(&ssid).cloned();
        let secure = self
            .state
            .borrow()
            .networks
            .iter()
            .any(|n| n.ssid == ssid && n.secure);
        *self.joining.borrow_mut() = Some(Joining {
            ssid: ssid.clone(),
            added: None,
            started: std::time::Instant::now(),
            seen: false,
        });
        self.publish();
        self.with_bus(async move |bus, this| {
            let reply = match known {
                Some(connection) => {
                    let args = (object(&connection), object(&device), object("/"));
                    dbus::call(
                        &bus,
                        SERVICE,
                        MANAGER_PATH,
                        MANAGER,
                        "ActivateConnection",
                        Some(args.to_variant()),
                    )
                    .await
                    .map(|_| None)
                }
                None => {
                    let settings = new_connection(&ssid, secure.then_some(password).flatten());
                    let args = Variant::tuple_from_iter([settings, object(&device), object("/")]);
                    dbus::call(
                        &bus,
                        SERVICE,
                        MANAGER_PATH,
                        MANAGER,
                        "AddAndActivateConnection",
                        Some(args),
                    )
                    .await
                    .map(|reply| reply.child_value(0).str().map(String::from))
                }
            };
            match reply {
                Ok(added) => {
                    if let Some(join) = this.joining.borrow_mut().as_mut() {
                        join.added = added;
                    }
                }
                Err(err) => {
                    this.joining.take();
                    this.publish();
                    (this.on_change)(&json!({
                        "type": "wifi-error", "ssid": ssid, "message": err.message(),
                    }));
                }
            }
            this.refresh_soon();
        });
    }

    /// Leaves the network the device is on.
    pub fn disconnect(self: &Rc<Self>) {
        let Some(device) = self.device.borrow().clone() else {
            return;
        };
        self.with_bus(async move |bus, _| {
            let _ = dbus::call(&bus, SERVICE, &device, DEVICE, "Disconnect", None).await;
        });
    }

    /// Forgets the saved connection for `ssid`.
    pub fn forget(self: &Rc<Self>, ssid: &str) {
        let Some(connection) = self.known.borrow().get(ssid).cloned() else {
            return;
        };
        self.with_bus(async move |bus, _| {
            let _ = dbus::call(&bus, SERVICE, &connection, CONNECTION, "Delete", None).await;
        });
    }
}

fn object(path: &str) -> Variant {
    glib::variant::ObjectPath::try_from(path.to_owned())
        .map(|path| path.to_variant())
        .unwrap_or_else(|_| {
            glib::variant::ObjectPath::try_from(String::from("/"))
                .unwrap()
                .to_variant()
        })
}

/// The settings of a connection for the network `ssid`: WPA with `password` if it has one, else
/// open.
fn new_connection(ssid: &str, password: Option<String>) -> Variant {
    let section = |pairs: Vec<(&str, Variant)>| -> HashMap<String, Variant> {
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect()
    };
    let mut settings: HashMap<String, HashMap<String, Variant>> = HashMap::new();
    settings.insert(
        String::from("connection"),
        section(vec![
            ("id", ssid.to_variant()),
            ("type", "802-11-wireless".to_variant()),
        ]),
    );
    settings.insert(
        String::from("802-11-wireless"),
        section(vec![("ssid", ssid.as_bytes().to_vec().to_variant())]),
    );
    if let Some(password) = password {
        settings.insert(
            String::from("802-11-wireless-security"),
            section(vec![
                ("key-mgmt", "wpa-psk".to_variant()),
                ("psk", password.to_variant()),
            ]),
        );
    }
    // `a{sa{sv}}`.
    settings.to_variant()
}

/// Reads Wi-Fi's state, the device's path, and the saved connections by SSID.
async fn read(bus: &DBusConnection) -> (State, Option<String>, HashMap<String, String>) {
    let mut state = State::default();
    let Ok(manager) = dbus::properties(bus, SERVICE, MANAGER_PATH, MANAGER).await else {
        return (state, None, HashMap::new());
    };
    state.enabled = dbus::lookup::<bool>(&manager, "WirelessEnabled").unwrap_or(false);
    let devices: Vec<String> = dbus::lookup::<Vec<glib::variant::ObjectPath>>(&manager, "Devices")
        .unwrap_or_default()
        .into_iter()
        .map(|path| path.to_string())
        .collect();
    let mut device = None;
    for path in devices {
        let kind = dbus::property(bus, SERVICE, &path, DEVICE, "DeviceType").await;
        if kind.ok().and_then(|v| v.get::<u32>()) == Some(WIFI) {
            device = Some(path);
            break;
        }
    }
    let known = known(bus).await;
    let Some(device) = device else {
        return (state, None, known);
    };
    state.available = true;
    state.device_state = dbus::property(bus, SERVICE, &device, DEVICE, "State")
        .await
        .ok()
        .and_then(|v| v.get::<u32>())
        .unwrap_or(0);
    let Ok(wireless) = dbus::properties(bus, SERVICE, &device, WIRELESS).await else {
        return (state, Some(device), known);
    };
    let active_path = dbus::lookup::<glib::variant::ObjectPath>(&wireless, "ActiveAccessPoint")
        .map(|path| path.to_string());
    let points: Vec<String> =
        dbus::lookup::<Vec<glib::variant::ObjectPath>>(&wireless, "AccessPoints")
            .unwrap_or_default()
            .into_iter()
            .map(|path| path.to_string())
            .collect();
    let mut seen = Vec::new();
    for path in points {
        let Ok(point) = dbus::properties(bus, SERVICE, &path, ACCESS_POINT).await else {
            continue;
        };
        let Some(ssid) = dbus::lookup::<Vec<u8>>(&point, "Ssid").and_then(|s| ssid(&s)) else {
            // A hidden network has no name to show.
            continue;
        };
        let flags = |key| dbus::lookup::<u32>(&point, key).unwrap_or(0);
        seen.push(Network {
            known: known.contains_key(&ssid),
            active: active_path.as_deref() == Some(path.as_str()),
            ssid,
            strength: dbus::lookup::<u8>(&point, "Strength").unwrap_or(0),
            secure: flags("Flags") & 1 != 0 || flags("WpaFlags") != 0 || flags("RsnFlags") != 0,
        });
    }
    state.networks = networks(seen);
    (state, Some(device), known)
}

/// The networks to show: one for each name, the strongest of its access points, the one in use
/// first, then the saved ones, then the strongest.
fn networks(seen: Vec<Network>) -> Vec<Network> {
    let mut by_name: Vec<Network> = Vec::new();
    for network in seen {
        match by_name.iter_mut().find(|n| n.ssid == network.ssid) {
            Some(kept) => {
                kept.active |= network.active;
                if network.strength > kept.strength {
                    kept.strength = network.strength;
                }
            }
            None => by_name.push(network),
        }
    }
    by_name.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(b.known.cmp(&a.known))
            .then(b.strength.cmp(&a.strength))
            .then(a.ssid.cmp(&b.ssid))
    });
    by_name.truncate(NETWORKS);
    by_name
}

/// The saved Wi-Fi connections, by the SSID they're for.
async fn known(bus: &DBusConnection) -> HashMap<String, String> {
    let mut known = HashMap::new();
    let Ok(reply) = dbus::call(
        bus,
        SERVICE,
        SETTINGS_PATH,
        SETTINGS,
        "ListConnections",
        None,
    )
    .await
    else {
        return known;
    };
    let paths: Vec<glib::variant::ObjectPath> = reply.child_value(0).get().unwrap_or_default();
    for path in paths {
        let Ok(reply) =
            dbus::call(bus, SERVICE, path.as_str(), CONNECTION, "GetSettings", None).await
        else {
            continue;
        };
        let ssid = section(&reply.child_value(0), "802-11-wireless")
            .and_then(|wireless| dbus::lookup::<Vec<u8>>(&wireless, "ssid"))
            .and_then(|bytes| ssid_from(&bytes));
        if let Some(ssid) = ssid {
            known.insert(ssid, path.to_string());
        }
    }
    known
}

fn ssid(bytes: &[u8]) -> Option<String> {
    ssid_from(bytes)
}

/// The section `name` of a connection's settings, `a{sa{sv}}`.
fn section(settings: &Variant, name: &str) -> Option<VariantDict> {
    settings
        .iter()
        .find(|entry| entry.child_value(0).str() == Some(name))
        .map(|entry| VariantDict::new(Some(&entry.child_value(1))))
}

/// A Wi-Fi network's name, which is bytes that are almost always UTF-8.
fn ssid_from(bytes: &[u8]) -> Option<String> {
    let name = String::from_utf8_lossy(bytes);
    let name = name.trim_end_matches('\0');
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network(ssid: &str, strength: u8, known: bool, active: bool) -> Network {
        Network {
            ssid: ssid.to_owned(),
            strength,
            secure: true,
            known,
            active,
        }
    }

    #[test]
    fn networks_are_one_per_name_in_use_then_saved_then_strongest() {
        let shown = networks(vec![
            network("cafe", 40, false, false),
            network("home", 30, true, false),
            network("cafe", 80, false, false),
            network("office", 90, false, false),
            network("phone", 20, false, true),
        ]);
        let names: Vec<&str> = shown.iter().map(|n| n.ssid.as_str()).collect();
        assert_eq!(names, ["phone", "home", "office", "cafe"]);
        assert_eq!(shown[3].strength, 80);
    }

    #[test]
    fn a_new_networks_connection_has_its_password_only_when_it_has_one() {
        let open = new_connection("cafe", None);
        assert!(section(&open, "802-11-wireless-security").is_none());
        let secure = new_connection("home", Some(String::from("hunter22")));
        let security = section(&secure, "802-11-wireless-security").unwrap();
        assert_eq!(
            dbus::lookup::<String>(&security, "psk").as_deref(),
            Some("hunter22")
        );
        let wireless = section(&secure, "802-11-wireless").unwrap();
        assert_eq!(dbus::lookup::<Vec<u8>>(&wireless, "ssid").unwrap(), b"home");
    }
}
