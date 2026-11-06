//! The network connection, from NetworkManager.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gio::{BusType, DBusConnection, SignalSubscription};

use super::dbus;

const SERVICE: &str = "org.freedesktop.NetworkManager";
const MANAGER: &str = "org.freedesktop.NetworkManager";
const MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";
const ACTIVE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const ACCESS_POINT: &str = "org.freedesktop.NetworkManager.AccessPoint";

/// `NM_ACTIVE_CONNECTION_STATE_ACTIVATED`.
const ACTIVATED: u32 = 2;

/// What kind of connection the computer is online through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    None,
    Wired,
    Wifi,
    Other,
}

/// The connection NetworkManager routes through, or the one it's bringing up.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Network {
    pub kind: Kind,
    /// The Wi-Fi signal, from 0 to 100.
    pub wifi_strength: Option<u8>,
    /// The Wi-Fi network's name, or the connection's.
    pub name: Option<String>,
    /// Whether the connection is up, rather than still being brought up.
    pub connected: bool,
}

impl Network {
    pub fn icon_name(&self) -> &'static str {
        match (self.kind, self.connected) {
            (Kind::None, _) => "network-offline-symbolic",
            (Kind::Wifi, false) => "network-wireless-acquiring-symbolic",
            (Kind::Wifi, true) => match self.wifi_strength.unwrap_or(0) {
                0..20 => "network-wireless-signal-none-symbolic",
                20..40 => "network-wireless-signal-weak-symbolic",
                40..50 => "network-wireless-signal-ok-symbolic",
                50..80 => "network-wireless-signal-good-symbolic",
                _ => "network-wireless-signal-excellent-symbolic",
            },
            (Kind::Wired | Kind::Other, false) => "network-wired-acquiring-symbolic",
            (Kind::Wired | Kind::Other, true) => "network-wired-symbolic",
        }
    }
}

/// Watches NetworkManager's primary connection.
pub struct NetworkService {
    watch: RefCell<Option<dbus::NameWatch>>,
    bus: RefCell<Option<DBusConnection>>,
    subscription: RefCell<Option<SignalSubscription>>,
    /// The objects whose changes change the state: NetworkManager itself, the connection and its
    /// access point.
    watched: RefCell<Vec<String>>,
    /// Counts reads, so that only the latest one is kept.
    generation: Cell<u64>,
    state: RefCell<Option<Network>>,
    on_change: Box<dyn Fn(&Network)>,
}

impl NetworkService {
    pub fn start(on_change: impl Fn(&Network) + 'static) -> Rc<Self> {
        let this = Rc::new(Self {
            watch: RefCell::default(),
            bus: RefCell::default(),
            subscription: RefCell::default(),
            watched: RefCell::default(),
            generation: Cell::default(),
            state: RefCell::default(),
            on_change: Box::new(on_change),
        });

        let appeared = Rc::downgrade(&this);
        let vanished = Rc::downgrade(&this);
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
                    eprintln!("NetworkManager isn't running");
                    this.vanished();
                }
            },
        );
        *this.watch.borrow_mut() = Some(watch);
        this
    }

    pub fn state(&self) -> Network {
        self.state.borrow().clone().unwrap_or_default()
    }

    fn appeared(self: &Rc<Self>, bus: DBusConnection) {
        let weak = Rc::downgrade(self);
        let subscription = dbus::on_properties_changed(&bus, SERVICE, move |path| {
            let Some(this) = weak.upgrade() else { return };
            if this.watched.borrow().iter().any(|watched| watched == path) {
                this.refresh();
            }
        });
        *self.subscription.borrow_mut() = Some(subscription);
        *self.bus.borrow_mut() = Some(bus);
        self.refresh();
    }

    fn vanished(&self) {
        self.subscription.take();
        self.bus.take();
        self.watched.borrow_mut().clear();
        self.generation.set(self.generation.get() + 1);
        self.set(Network::default());
    }

    fn refresh(self: &Rc<Self>) {
        let Some(bus) = self.bus.borrow().clone() else {
            return;
        };
        let generation = self.generation.get() + 1;
        self.generation.set(generation);

        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let (network, watched) = read(&bus).await;
            let Some(this) = weak.upgrade() else { return };
            if this.generation.get() == generation {
                *this.watched.borrow_mut() = watched;
                this.set(network);
            }
        });
    }

    fn set(&self, network: Network) {
        if self.state.borrow().as_ref() == Some(&network) {
            return;
        }
        *self.state.borrow_mut() = Some(network.clone());
        (self.on_change)(&network);
    }
}

/// Reads the state, with the objects it came from.
async fn read(bus: &DBusConnection) -> (Network, Vec<String>) {
    let mut watched = vec![String::from(MANAGER_PATH)];
    let mut network = Network::default();

    let manager = match dbus::properties(bus, SERVICE, MANAGER_PATH, MANAGER).await {
        Ok(manager) => manager,
        Err(err) => {
            eprintln!("Couldn't read NetworkManager's state: {err}");
            return (network, watched);
        }
    };
    let Some(connection) = ["PrimaryConnection", "ActivatingConnection"]
        .into_iter()
        .filter_map(|key| dbus::lookup::<String>(&manager, key))
        .find(|path| is_object(path))
    else {
        return (network, watched);
    };
    watched.push(connection.clone());

    let Ok(active) = dbus::properties(bus, SERVICE, &connection, ACTIVE).await else {
        return (network, watched);
    };
    network.kind = kind(&dbus::lookup::<String>(&active, "Type").unwrap_or_default());
    network.connected = dbus::lookup::<u32>(&active, "State") == Some(ACTIVATED);
    network.name = dbus::lookup::<String>(&active, "Id").filter(|id| !id.is_empty());

    if network.kind == Kind::Wifi {
        let access_point =
            dbus::lookup::<String>(&active, "SpecificObject").filter(|p| is_object(p));
        if let Some(access_point) = access_point {
            watched.push(access_point.clone());
            if let Ok(ap) = dbus::properties(bus, SERVICE, &access_point, ACCESS_POINT).await {
                network.wifi_strength = dbus::lookup::<u8>(&ap, "Strength");
                if let Some(ssid) = dbus::lookup::<Vec<u8>>(&ap, "Ssid").and_then(|s| ssid(&s)) {
                    network.name = Some(ssid);
                }
            }
        }
    }

    (network, watched)
}

/// NetworkManager says "/" for no object.
fn is_object(path: &str) -> bool {
    !path.is_empty() && path != "/"
}

/// The kind of a connection, from its NetworkManager type.
fn kind(r#type: &str) -> Kind {
    match r#type {
        "802-11-wireless" => Kind::Wifi,
        "802-3-ethernet" => Kind::Wired,
        "" => Kind::None,
        _ => Kind::Other,
    }
}

/// A Wi-Fi network's name, which is bytes that are almost always UTF-8.
fn ssid(bytes: &[u8]) -> Option<String> {
    let name = String::from_utf8_lossy(bytes);
    let name = name.trim_end_matches('\0');
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_types_map_to_kinds() {
        assert_eq!(kind("802-11-wireless"), Kind::Wifi);
        assert_eq!(kind("802-3-ethernet"), Kind::Wired);
        assert_eq!(kind("wireguard"), Kind::Other);
        assert_eq!(kind(""), Kind::None);
    }

    #[test]
    fn ssids_are_text() {
        assert_eq!(ssid(b"home").as_deref(), Some("home"));
        assert_eq!(ssid(b"caf\xc3\xa9\0").as_deref(), Some("café"));
        assert_eq!(ssid(b""), None);
    }

    #[test]
    fn no_object_is_a_slash() {
        assert!(!is_object("/"));
        assert!(is_object(
            "/org/freedesktop/NetworkManager/ActiveConnection/3"
        ));
    }

    #[test]
    fn icons_follow_kind_signal_and_state() {
        let wifi = |strength, connected| Network {
            kind: Kind::Wifi,
            wifi_strength: Some(strength),
            name: None,
            connected,
        };
        assert_eq!(
            wifi(90, true).icon_name(),
            "network-wireless-signal-excellent-symbolic"
        );
        assert_eq!(
            wifi(60, true).icon_name(),
            "network-wireless-signal-good-symbolic"
        );
        assert_eq!(
            wifi(45, true).icon_name(),
            "network-wireless-signal-ok-symbolic"
        );
        assert_eq!(
            wifi(25, true).icon_name(),
            "network-wireless-signal-weak-symbolic"
        );
        assert_eq!(
            wifi(5, true).icon_name(),
            "network-wireless-signal-none-symbolic"
        );
        assert_eq!(
            wifi(90, false).icon_name(),
            "network-wireless-acquiring-symbolic"
        );

        let wired = Network {
            kind: Kind::Wired,
            connected: true,
            ..Network::default()
        };
        assert_eq!(wired.icon_name(), "network-wired-symbolic");
        assert_eq!(Network::default().icon_name(), "network-offline-symbolic");
    }
}
