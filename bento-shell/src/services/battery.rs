//! The computer's battery, from UPower.
//!
//! UPower's display device combines the batteries that power the computer, and leaves out those
//! of mice, keyboards and other devices.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gio::{BusType, DBusConnection, SignalSubscription};
use glib::VariantDict;

use super::dbus;

const SERVICE: &str = "org.freedesktop.UPower";
const DEVICE: &str = "org.freedesktop.UPower.Device";
const DISPLAY_DEVICE: &str = "/org/freedesktop/UPower/devices/DisplayDevice";

/// UPower's `Type` for a battery.
const TYPE_BATTERY: u32 = 2;
/// UPower's `State` while charging.
const STATE_CHARGING: u32 = 1;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Battery {
    /// Whether the computer has a battery at all.
    pub present: bool,
    /// From 0 to 100.
    pub percentage: f64,
    pub charging: bool,
    /// UPower's symbolic icon for the charge and state.
    pub icon_name: String,
}

impl Battery {
    fn from_properties(properties: &VariantDict) -> Self {
        let present = dbus::lookup::<bool>(properties, "IsPresent").unwrap_or(false)
            && dbus::lookup::<u32>(properties, "Type") == Some(TYPE_BATTERY);
        if !present {
            return Self::default();
        }
        Self {
            present,
            percentage: dbus::lookup::<f64>(properties, "Percentage")
                .unwrap_or(0.)
                .clamp(0., 100.),
            charging: dbus::lookup::<u32>(properties, "State") == Some(STATE_CHARGING),
            icon_name: dbus::lookup::<String>(properties, "IconName")
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| String::from("battery-missing-symbolic")),
        }
    }
}

/// Watches UPower's display device.
pub struct BatteryService {
    watch: RefCell<Option<dbus::NameWatch>>,
    bus: RefCell<Option<DBusConnection>>,
    subscription: RefCell<Option<SignalSubscription>>,
    /// Counts reads, so that only the latest one is kept.
    generation: Cell<u64>,
    state: RefCell<Option<Battery>>,
    on_change: Box<dyn Fn(&Battery)>,
}

impl BatteryService {
    pub fn start(on_change: impl Fn(&Battery) + 'static) -> Rc<Self> {
        let this = Rc::new(Self {
            watch: RefCell::default(),
            bus: RefCell::default(),
            subscription: RefCell::default(),
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
                    eprintln!("UPower isn't running");
                    this.vanished();
                }
            },
        );
        *this.watch.borrow_mut() = Some(watch);
        this
    }

    pub fn state(&self) -> Battery {
        self.state.borrow().clone().unwrap_or_default()
    }

    fn appeared(self: &Rc<Self>, bus: DBusConnection) {
        let weak = Rc::downgrade(self);
        let subscription = dbus::on_properties_changed(&bus, SERVICE, move |path| {
            if let Some(this) = weak.upgrade() {
                if path == DISPLAY_DEVICE {
                    this.refresh();
                }
            }
        });
        *self.subscription.borrow_mut() = Some(subscription);
        *self.bus.borrow_mut() = Some(bus);
        self.refresh();
    }

    fn vanished(&self) {
        self.subscription.take();
        self.bus.take();
        self.generation.set(self.generation.get() + 1);
        self.set(Battery::default());
    }

    fn refresh(self: &Rc<Self>) {
        let Some(bus) = self.bus.borrow().clone() else {
            return;
        };
        let generation = self.generation.get() + 1;
        self.generation.set(generation);

        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let battery = match dbus::properties(&bus, SERVICE, DISPLAY_DEVICE, DEVICE).await {
                Ok(properties) => Battery::from_properties(&properties),
                Err(err) => {
                    eprintln!("Couldn't read the battery: {err}");
                    Battery::default()
                }
            };
            let Some(this) = weak.upgrade() else { return };
            if this.generation.get() == generation {
                this.set(battery);
            }
        });
    }

    fn set(&self, battery: Battery) {
        if self.state.borrow().as_ref() == Some(&battery) {
            return;
        }
        *self.state.borrow_mut() = Some(battery.clone());
        (self.on_change)(&battery);
    }
}

#[cfg(test)]
mod tests {
    use glib::prelude::*;

    use super::*;

    fn properties(entries: &[(&str, glib::Variant)]) -> VariantDict {
        let dict = VariantDict::new(None);
        for (key, value) in entries {
            dict.insert_value(key, value);
        }
        dict
    }

    #[test]
    fn a_charging_battery_is_present() {
        let battery = Battery::from_properties(&properties(&[
            ("IsPresent", true.to_variant()),
            ("Type", 2u32.to_variant()),
            ("Percentage", 42.5f64.to_variant()),
            ("State", 1u32.to_variant()),
            ("IconName", "battery-good-charging-symbolic".to_variant()),
        ]));
        assert_eq!(
            battery,
            Battery {
                present: true,
                percentage: 42.5,
                charging: true,
                icon_name: String::from("battery-good-charging-symbolic"),
            }
        );
    }

    #[test]
    fn a_desktop_has_no_battery() {
        let battery = Battery::from_properties(&properties(&[
            ("IsPresent", false.to_variant()),
            ("Type", 0u32.to_variant()),
            ("IconName", "battery-missing-symbolic".to_variant()),
        ]));
        assert!(!battery.present);
    }

    #[test]
    fn a_discharging_battery_isnt_charging() {
        let battery = Battery::from_properties(&properties(&[
            ("IsPresent", true.to_variant()),
            ("Type", 2u32.to_variant()),
            ("Percentage", 80f64.to_variant()),
            ("State", 2u32.to_variant()),
        ]));
        assert!(battery.present && !battery.charging);
        assert_eq!(battery.icon_name, "battery-missing-symbolic");
    }
}
