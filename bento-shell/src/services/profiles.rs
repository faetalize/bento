//! The power mode, through power-profiles-daemon: power saver, balanced or performance, of those
//! the computer has.

use std::cell::RefCell;
use std::rc::Rc;

use gio::prelude::*;
use gio::{BusType, DBusConnection, SignalSubscription};
use glib::VariantDict;
use serde_json::{json, Value};

use super::dbus;

const SERVICE: &str = "net.hadess.PowerProfiles";
const PATH: &str = "/net/hadess/PowerProfiles";
const INTERFACE: &str = "net.hadess.PowerProfiles";

pub struct ProfileService {
    watch: RefCell<Option<dbus::NameWatch>>,
    bus: RefCell<Option<DBusConnection>>,
    subscription: RefCell<Option<SignalSubscription>>,
    on_change: Box<dyn Fn(&Value)>,
}

impl ProfileService {
    /// Starts watching the power mode. `on_change` hears `{ "type": "power-profile", available,
    /// active, profiles }`.
    pub fn start(on_change: impl Fn(&Value) + 'static) -> Rc<Self> {
        let this = Rc::new(Self {
            watch: RefCell::default(),
            bus: RefCell::default(),
            subscription: RefCell::default(),
            on_change: Box::new(on_change),
        });
        (this.on_change)(&gone());
        let (appeared, vanished) = (Rc::downgrade(&this), Rc::downgrade(&this));
        let watch = dbus::watch_name(
            BusType::System,
            SERVICE,
            move |bus| {
                if let Some(this) = appeared.upgrade() {
                    let weak = Rc::downgrade(&this);
                    let subscription = dbus::on_properties_changed(&bus, SERVICE, move |_| {
                        if let Some(this) = weak.upgrade() {
                            this.refresh();
                        }
                    });
                    *this.subscription.borrow_mut() = Some(subscription);
                    *this.bus.borrow_mut() = Some(bus);
                    this.refresh();
                }
            },
            move || {
                if let Some(this) = vanished.upgrade() {
                    this.subscription.take();
                    this.bus.take();
                    (this.on_change)(&gone());
                }
            },
        );
        *this.watch.borrow_mut() = Some(watch);
        this
    }

    fn refresh(self: &Rc<Self>) {
        let Some(bus) = self.bus.borrow().clone() else {
            return;
        };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(properties) = dbus::properties(&bus, SERVICE, PATH, INTERFACE).await else {
                return;
            };
            if let Some(this) = weak.upgrade() {
                (this.on_change)(&read(&properties));
            }
        });
    }

    /// Picks the power mode `name`.
    pub fn set(self: &Rc<Self>, name: String) {
        let Some(bus) = self.bus.borrow().clone() else {
            return;
        };
        glib::spawn_future_local(async move {
            let value = name.to_variant();
            let _ =
                dbus::set_property(&bus, SERVICE, PATH, INTERFACE, "ActiveProfile", value).await;
        });
    }
}

fn gone() -> Value {
    json!({ "type": "power-profile", "available": false, "active": null, "profiles": [] })
}

fn read(properties: &VariantDict) -> Value {
    let profiles: Vec<String> = properties
        .lookup_value("Profiles", None)
        .map(|list| {
            list.iter()
                .filter_map(|profile| {
                    dbus::lookup::<String>(&VariantDict::new(Some(&profile)), "Profile")
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "type": "power-profile",
        "available": !profiles.is_empty(),
        "active": dbus::lookup::<String>(properties, "ActiveProfile"),
        "profiles": profiles,
    })
}
