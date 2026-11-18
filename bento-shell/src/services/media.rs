//! What's playing, from the apps that say so over MPRIS, and playing, pausing and skipping.
//!
//! playerctld is left out: it only repeats another player. Cover art is shown when the app gives
//! it as a file; one only on the web isn't fetched.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gio::prelude::*;
use gio::{BusType, DBusConnection, DBusSignalFlags, SignalSubscription};
use glib::VariantDict;
use serde_json::{json, Value};

use super::dbus;

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT: &str = "org.mpris.MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const BUS: &str = "org.freedesktop.DBus";

pub struct MediaService {
    bus: RefCell<Option<DBusConnection>>,
    subscriptions: RefCell<Vec<SignalSubscription>>,
    soon: dbus::Soon,
    generation: Cell<u64>,
    last: RefCell<Option<Value>>,
    on_change: Box<dyn Fn(&Value)>,
}

impl MediaService {
    /// Starts watching the players. `on_change` hears `{ "type": "media", players: [{ id, name,
    /// icon, playing, title, artist, art, canNext, canPrevious, canPlay }] }`, the playing ones
    /// first.
    pub fn start(on_change: impl Fn(&Value) + 'static) -> Rc<Self> {
        let this = Rc::new(Self {
            bus: RefCell::default(),
            subscriptions: RefCell::default(),
            soon: dbus::Soon::default(),
            generation: Cell::default(),
            last: RefCell::default(),
            on_change: Box::new(on_change),
        });
        let weak = Rc::downgrade(&this);
        glib::spawn_future_local(async move {
            let Some(bus) = dbus::bus(BusType::Session).await else {
                return;
            };
            let Some(this) = weak.upgrade() else { return };
            // Players coming and going, and anything about one changing.
            let weak = Rc::downgrade(&this);
            let owners = bus.subscribe_to_signal(
                Some(BUS),
                Some(BUS),
                Some("NameOwnerChanged"),
                None,
                Some(PREFIX),
                DBusSignalFlags::MATCH_ARG0_NAMESPACE,
                move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.refresh_soon();
                    }
                },
            );
            let weak = Rc::downgrade(&this);
            let changes = bus.subscribe_to_signal(
                None,
                Some(dbus::PROPERTIES),
                Some("PropertiesChanged"),
                Some(PATH),
                None,
                DBusSignalFlags::NONE,
                move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.refresh_soon();
                    }
                },
            );
            *this.subscriptions.borrow_mut() = vec![owners, changes];
            *this.bus.borrow_mut() = Some(bus);
            this.refresh();
        });
        this
    }

    fn refresh_soon(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.soon.ask(Duration::from_millis(150), move || {
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
            let players = read(&bus).await;
            let Some(this) = weak.upgrade() else { return };
            if this.generation.get() != generation {
                return;
            }
            let value = json!({ "type": "media", "players": players });
            if this.last.borrow().as_ref() != Some(&value) {
                (this.on_change)(&value);
                *this.last.borrow_mut() = Some(value);
            }
        });
    }

    /// Asks the player `id` to `action`: `PlayPause`, `Next` or `Previous`.
    pub fn control(&self, id: &str, action: &str) {
        if !id.starts_with(PREFIX) || !matches!(action, "PlayPause" | "Next" | "Previous") {
            return;
        }
        let Some(bus) = self.bus.borrow().clone() else {
            return;
        };
        let (id, action) = (id.to_owned(), action.to_owned());
        glib::spawn_future_local(async move {
            let _ = dbus::call(&bus, &id, PATH, PLAYER, &action, None).await;
        });
    }
}

async fn read(bus: &DBusConnection) -> Vec<Value> {
    let Ok(reply) = dbus::call(bus, BUS, "/org/freedesktop/DBus", BUS, "ListNames", None).await
    else {
        return Vec::new();
    };
    let names: Vec<String> = reply.child_value(0).get().unwrap_or_default();
    let mut players = Vec::new();
    for id in names.into_iter().filter(|name| shown(name)) {
        let Ok(player) = dbus::properties(bus, &id, PATH, PLAYER).await else {
            continue;
        };
        let root = dbus::properties(bus, &id, PATH, ROOT).await.ok();
        let metadata = player
            .lookup_value("Metadata", None)
            .map(|metadata| VariantDict::new(Some(&metadata)));
        let text = |key: &str| {
            metadata
                .as_ref()
                .and_then(|m| dbus::lookup::<String>(m, key))
                .filter(|text| !text.is_empty())
        };
        let artist = metadata
            .as_ref()
            .and_then(|m| dbus::lookup::<Vec<String>>(m, "xesam:artist"))
            .map(|artists| artists.join(", "))
            .filter(|artist| !artist.is_empty());
        let status = dbus::lookup::<String>(&player, "PlaybackStatus").unwrap_or_default();
        let name = root
            .as_ref()
            .and_then(|root| dbus::lookup::<String>(root, "Identity"))
            .unwrap_or_else(|| id.trim_start_matches(PREFIX).to_owned());
        let icon = root
            .as_ref()
            .and_then(|root| dbus::lookup::<String>(root, "DesktopEntry"));
        let can = |key: &str| dbus::lookup::<bool>(&player, key).unwrap_or(false);
        // A player with nothing loaded has nothing to show.
        let Some(title) = text("xesam:title") else {
            continue;
        };
        players.push(json!({
            "id": id,
            "name": name,
            "icon": icon,
            "playing": status == "Playing",
            "title": title,
            "artist": artist,
            "art": text("mpris:artUrl").and_then(|url| art(&url)),
            "canNext": can("CanGoNext"),
            "canPrevious": can("CanGoPrevious"),
            "canPlay": can("CanPlay") || can("CanPause"),
        }));
    }
    // Playing ones first; otherwise as the bus lists them.
    players.sort_by_key(|player| !player["playing"].as_bool().unwrap_or(false));
    players
}

/// Whether the bus name `name` is a player to show.
fn shown(name: &str) -> bool {
    name.strip_prefix(PREFIX)
        .is_some_and(|rest| !rest.is_empty() && rest != "playerctld")
}

/// The file cover art `url` names, if it's a file that's there.
fn art(url: &str) -> Option<String> {
    let path = gio::File::for_uri(url).path()?;
    path.is_file().then(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_are_mpris_names_but_playerctld() {
        assert!(shown("org.mpris.MediaPlayer2.firefox.instance_1_84"));
        assert!(shown("org.mpris.MediaPlayer2.spotify"));
        assert!(!shown("org.mpris.MediaPlayer2.playerctld"));
        assert!(!shown("org.mpris.MediaPlayer2."));
        assert!(!shown("org.freedesktop.Notifications"));
    }

    #[test]
    fn only_art_in_a_file_is_shown() {
        assert_eq!(art("https://example.org/cover.jpg"), None);
        assert_eq!(art("file:///nowhere/at/all.png"), None);
    }
}
