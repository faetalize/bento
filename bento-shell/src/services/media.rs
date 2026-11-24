//! What's playing, from the apps that say so over MPRIS, and playing, pausing, skipping,
//! seeking, setting its volume and raising its window.
//!
//! playerctld is left out: it only repeats another player. Cover art is shown when the app gives
//! it as a file; one only on the web isn't fetched.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gio::prelude::*;
use gio::{BusType, DBusConnection, DBusSignalFlags, SignalSubscription};
use glib::variant::ObjectPath;
use glib::{Variant, VariantDict};
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
    /// icon, playing, title, artist, art, artAspect, canNext, canPrevious, canPlay, canSeek,
    /// position, length, rate, at, volume, canRaise }] }`, the playing ones first. `position` and
    /// `length` are in seconds, `position` as it was at `at`, in milliseconds since 1970:
    /// players don't say as it moves, so it's read again only when something else changes or
    /// they seek.
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
            // Players say when they jump, not as they play on.
            let weak = Rc::downgrade(&this);
            let seeks = bus.subscribe_to_signal(
                None,
                Some(PLAYER),
                Some("Seeked"),
                Some(PATH),
                None,
                DBusSignalFlags::NONE,
                move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.refresh_soon();
                    }
                },
            );
            *this.subscriptions.borrow_mut() = vec![owners, changes, seeks];
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
            // Told again only when something besides when it was read changed.
            let value = json!({ "type": "media", "players": players });
            let unstamped = unstamped(&value);
            if this.last.borrow().as_ref() != Some(&unstamped) {
                (this.on_change)(&value);
                *this.last.borrow_mut() = Some(unstamped);
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

    /// Takes the player `id` to `position` seconds into what it plays.
    pub fn seek(&self, id: &str, position: Option<f64>) {
        let Some(position) = position.filter(|p| p.is_finite() && *p >= 0.) else {
            return;
        };
        let Some(bus) = self.player_bus(id) else {
            return;
        };
        let id = id.to_owned();
        let micros = (position * 1e6) as i64;
        glib::spawn_future_local(async move {
            let Ok(player) = dbus::properties(&bus, &id, PATH, PLAYER).await else {
                return;
            };
            // SetPosition goes to a place; it needs the track's id, which some players don't
            // give, so they're moved by as far as it is from where they are.
            let (method, args) = match track(&player) {
                Some(track) => ("SetPosition", (track, micros).to_variant()),
                None => {
                    let now = dbus::lookup::<i64>(&player, "Position").unwrap_or(0);
                    ("Seek", (micros - now,).to_variant())
                }
            };
            let _ = dbus::call(&bus, &id, PATH, PLAYER, method, Some(args)).await;
        });
    }

    /// Sets the player `id`'s own volume, from 0.
    pub fn volume(&self, id: &str, value: Option<f64>) {
        let Some(value) = value.filter(|v| v.is_finite()) else {
            return;
        };
        let Some(bus) = self.player_bus(id) else {
            return;
        };
        let id = id.to_owned();
        let value = value.max(0.).to_variant();
        glib::spawn_future_local(async move {
            let _ = dbus::set_property(&bus, &id, PATH, PLAYER, "Volume", value).await;
        });
    }

    /// Asks the player `id` to bring its window to the front.
    pub fn raise(&self, id: &str) {
        let Some(bus) = self.player_bus(id) else {
            return;
        };
        let id = id.to_owned();
        glib::spawn_future_local(async move {
            let _ = dbus::call(&bus, &id, PATH, ROOT, "Raise", None).await;
        });
    }

    /// The bus, when `id` names a player.
    fn player_bus(&self, id: &str) -> Option<DBusConnection> {
        shown(id).then(|| self.bus.borrow().clone()).flatten()
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
        let length = metadata
            .as_ref()
            .and_then(|m| m.lookup_value("mpris:length", None))
            .and_then(|length| micros(&length))
            .filter(|length| *length > 0);
        let position = player
            .lookup_value("Position", None)
            .and_then(|position| micros(&position))
            .unwrap_or(0);
        // Players that can't be told anything only show what they play.
        let volume = dbus::lookup::<f64>(&player, "Volume").filter(|_| can("CanControl"));
        // A player with nothing loaded has nothing to show.
        let Some(title) = text("xesam:title") else {
            continue;
        };
        let art = text("mpris:artUrl").and_then(|url| art(&url));
        players.push(json!({
            "id": id,
            "name": name,
            "icon": icon,
            "playing": status == "Playing",
            "title": title,
            "artist": artist,
            "artAspect": art.as_deref().and_then(aspect),
            "art": art,
            "canNext": can("CanGoNext"),
            "canPrevious": can("CanGoPrevious"),
            "canPlay": can("CanPlay") || can("CanPause"),
            // A place to go to needs a length to go along.
            "canSeek": can("CanSeek") && length.is_some(),
            "position": seconds(position),
            "length": length.map(seconds),
            "rate": dbus::lookup::<f64>(&player, "Rate").unwrap_or(1.),
            "at": stamp(),
            "volume": volume,
            "canRaise": root
                .as_ref()
                .and_then(|root| dbus::lookup::<bool>(root, "CanRaise"))
                .unwrap_or(false),
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

/// The width over the height of the picture at `path`, from its header alone: covers aren't
/// all square, and the shell crops them to fit.
fn aspect(path: &str) -> Option<f64> {
    let (_, width, height) = gtk::gdk_pixbuf::Pixbuf::file_info(path)?;
    (width > 0 && height > 0).then(|| f64::from(width) / f64::from(height))
}

/// The id of the track `player` plays, to go to a place in it: none when it plays nothing, or
/// gives it as a plain string, which SetPosition won't take.
fn track(player: &VariantDict) -> Option<ObjectPath> {
    let metadata = VariantDict::new(Some(&player.lookup_value("Metadata", None)?));
    let track = dbus::lookup::<ObjectPath>(&metadata, "mpris:trackid")?;
    (track.as_str() != "/org/mpris/MediaPlayer2/TrackList/NoTrack").then_some(track)
}

/// A time in microseconds, which players give as 64-bit numbers signed or not, and some as
/// 32-bit ones.
fn micros(value: &Variant) -> Option<i64> {
    value
        .get::<i64>()
        .or_else(|| value.get::<u64>().and_then(|v| i64::try_from(v).ok()))
        .or_else(|| value.get::<i32>().map(i64::from))
        .or_else(|| value.get::<u32>().map(i64::from))
}

fn seconds(micros: i64) -> f64 {
    micros as f64 / 1e6
}

/// Now, in milliseconds since 1970, as the shell's `Date.now()` has it.
fn stamp() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0., |since| since.as_secs_f64() * 1000.)
}

/// `value` without when each player was read, to tell whether anything else changed. A playing
/// player's place moves on between readings, so it's told again then.
fn unstamped(value: &Value) -> Value {
    let mut value = value.clone();
    for player in value["players"].as_array_mut().into_iter().flatten() {
        if let Some(player) = player.as_object_mut() {
            player.remove("at");
        }
    }
    value
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
    fn times_come_in_any_width_of_number() {
        assert_eq!(micros(&5_000_000i64.to_variant()), Some(5_000_000));
        assert_eq!(micros(&5_000_000u64.to_variant()), Some(5_000_000));
        assert_eq!(micros(&5_000_000i32.to_variant()), Some(5_000_000));
        assert_eq!(micros(&"5".to_variant()), None);
    }

    #[test]
    fn a_place_to_go_to_needs_a_real_track() {
        let with = |id: Variant| {
            let metadata = VariantDict::new(None);
            metadata.insert_value("mpris:trackid", &id);
            let player = VariantDict::new(None);
            player.insert_value("Metadata", &metadata.end());
            track(&player).map(|track| track.as_str().to_owned())
        };
        let path = |p: &str| ObjectPath::try_from(p.to_owned()).unwrap().to_variant();
        assert_eq!(
            with(path("/org/mpris/MediaPlayer2/firefox")).as_deref(),
            Some("/org/mpris/MediaPlayer2/firefox")
        );
        assert_eq!(
            with(path("/org/mpris/MediaPlayer2/TrackList/NoTrack")),
            None
        );
        assert_eq!(
            with("/a/string".to_variant()),
            None,
            "a string isn't a track id"
        );
    }

    #[test]
    fn a_reading_is_the_same_whenever_it_was() {
        let at = |at: f64| json!({ "players": [{ "id": "a", "position": 3.0, "at": at }] });
        assert_eq!(unstamped(&at(1.)), unstamped(&at(2.)));
        assert_ne!(
            unstamped(&at(1.)),
            unstamped(&json!({ "players": [{ "id": "a", "position": 4.0, "at": 1. }] }))
        );
    }

    #[test]
    fn only_art_in_a_file_is_shown() {
        assert_eq!(art("https://example.org/cover.jpg"), None);
        assert_eq!(art("file:///nowhere/at/all.png"), None);
    }
}
