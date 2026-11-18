//! `bento-shell --data`: the shell's data, for bento's own shell, without a window.
//!
//! bento starts it and reads what it says from its stdout, a JSON object per line: the speaker's
//! volume, the sound devices, the displays' brightness, the light or dark style, the weather, the
//! network, the battery, notifications, the clipboard's history and the tray's items, each as it
//! changes. Pictures, like a
//! notification's image or a tray icon an app draws itself, are written as PNG files under
//! `$XDG_RUNTIME_DIR/bento-shell` and named by their path. It takes commands on its stdin, a JSON
//! object per line, like `{ "cmd": "volume", "value": 0.5 }`.

use std::cell::RefCell;
use std::io::{BufRead as _, Write as _};
use std::path::PathBuf;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use serde_json::{json, Value};

use crate::services::bluetooth::BluetoothService;
use crate::services::brightness::BrightnessService;
use crate::services::clipboard::{ClipboardService, Content};
use crate::services::devices::{Device, DeviceService, Kind};
use crate::services::media::MediaService;
use crate::services::profiles::ProfileService;
use crate::services::weather::{Place, WeatherService};
use crate::services::wifi::WifiService;
use crate::services::{
    power, AudioService, BatteryService, NetworkService, NotificationEvent, Notifications, Tray,
    TrayIcon, TrayItem,
};

/// GNOME's interface settings, which hold the light or dark style apps follow.
const INTERFACE: &str = "org.gnome.desktop.interface";

/// The interface settings, if they're there with a style to set.
fn interface_settings() -> Option<gio::Settings> {
    let schema = gio::SettingsSchemaSource::default()?.lookup(INTERFACE, true)?;
    schema
        .has_key("color-scheme")
        .then(|| gio::Settings::new(INTERFACE))
}

fn say_theme(settings: &gio::Settings) {
    let dark = settings.string("color-scheme") == "prefer-dark";
    say(&json!({ "type": "theme", "dark": dark }));
}

fn device(device: &Device) -> Value {
    json!({
        "name": device.name,
        "label": device.label,
        "icon": device.icon,
        "active": device.active,
        "volume": device.volume,
        "muted": device.muted,
    })
}

/// Runs `command` with `sh -c`, apart from this program's own pipes, and says how it ended as
/// `{ "type": "ran", "id": id, "code": its exit code }`.
fn run_command(id: Value, command: &str) {
    let flags = gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE;
    let args = ["sh", "-c", command].map(std::ffi::OsStr::new);
    let process = match gio::Subprocess::newv(&args, flags) {
        Ok(process) => process,
        Err(err) => {
            eprintln!("bento-shell --data: couldn't run {command:?}: {err}");
            say(&json!({ "type": "ran", "id": id, "code": -1 }));
            return;
        }
    };
    glib::spawn_future_local(async move {
        let _ = process.wait_future().await;
        let code = if process.has_exited() {
            process.exit_status()
        } else {
            -1
        };
        say(&json!({ "type": "ran", "id": id, "code": code }));
    });
}

/// Shows a notification, through whichever program shows them. A `transient` one, like saying
/// which sound output plays now, isn't kept once it's been seen.
fn notify(summary: String, body: String, icon: String, transient: bool) {
    glib::spawn_future_local(async move {
        let Ok(bus) = gio::bus_get_future(gio::BusType::Session).await else {
            return;
        };
        let actions: Vec<String> = Vec::new();
        let mut hints: std::collections::HashMap<String, glib::Variant> = Default::default();
        if transient {
            hints.insert(String::from("transient"), true.to_variant());
        }
        let message = ("bento", 0u32, icon, summary, body, actions, hints, -1i32);
        let reply = bus
            .call_future(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
                "Notify",
                Some(&message.to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                -1,
            )
            .await;
        if let Err(err) = reply {
            eprintln!("bento-shell --data: couldn't show a notification: {err}");
        }
    });
}

/// Says `value` on stdout, as a line.
fn say(value: &Value) {
    let mut out = std::io::stdout().lock();
    if writeln!(out, "{value}").and_then(|()| out.flush()).is_err() {
        // bento went away.
        std::process::exit(0);
    }
}

/// Where pictures are written.
fn pictures() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("bento-shell");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Writes `texture` as `name`.png, and gives its path.
fn save(texture: &gdk::Texture, name: &str) -> Option<String> {
    let png = texture.save_to_png_bytes();
    let path = save_png(&pictures(), name, &png)?;
    Some(path.to_string_lossy().into_owned())
}

/// Writes the PNG `png` for `name` in `dir`, and gives its path. The path is named after what's
/// in it too, since the shell keeps each picture it loaded by its path: a notification that
/// replaces another keeps its id but has a picture of its own, and a tray icon changes. The
/// earlier picture for `name` goes.
fn save_png(dir: &std::path::Path, name: &str, png: &[u8]) -> Option<PathBuf> {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    png.hash(&mut hasher);
    let file = format!("{name}-{:016x}.png", hasher.finish());
    let path = dir.join(&file);
    forget_pictures(dir, name, Some(&file));
    if !path.exists() {
        std::fs::write(&path, png).ok()?;
    }
    Some(path)
}

/// Removes the pictures written for `name` in `dir`, but `keep`.
fn forget_pictures(dir: &std::path::Path, name: &str, keep: Option<&str>) {
    let Ok(files) = std::fs::read_dir(dir) else {
        return;
    };
    let prefix = format!("{name}-");
    for file in files.flatten() {
        let file_name = file.file_name();
        let file_name = file_name.to_string_lossy();
        // The rest is the hash alone, so `notification-7-` doesn't take `notification-70-…`.
        let ours = file_name
            .strip_prefix(&prefix)
            .and_then(|rest| rest.strip_suffix(".png"))
            .is_some_and(|hash| hash.len() == 16 && hash.chars().all(|c| c.is_ascii_hexdigit()));
        if ours && Some(&*file_name) != keep {
            let _ = std::fs::remove_file(file.path());
        }
    }
}

fn tray_item(item: &TrayItem, index: usize) -> Value {
    let icon = match &item.icon {
        None => None,
        Some(TrayIcon::Name { name, theme_path }) => {
            // An icon the app ships itself, in its own folder.
            let own = theme_path
                .as_deref()
                .filter(|p| !p.is_empty())
                .and_then(|dir| {
                    ["png", "svg"]
                        .iter()
                        .map(|ext| PathBuf::from(dir).join(format!("{name}.{ext}")))
                        .find(|path| path.is_file())
                        .map(|path| path.to_string_lossy().into_owned())
                });
            Some(own.unwrap_or_else(|| name.clone()))
        }
        Some(TrayIcon::Texture(texture)) => save(texture, &format!("tray-{index}")),
    };
    json!({
        "id": item.id,
        "title": item.title,
        "icon": icon,
        "menu": item.item_is_menu,
        "attention": item.status == crate::services::tray::Status::NeedsAttention,
    })
}

pub fn run() -> glib::ExitCode {
    let main_loop = glib::MainLoop::new(None, false);

    let audio = AudioService::start(|audio| {
        say(&json!({
            "type": "audio",
            "available": audio.available,
            "volume": audio.volume,
            "muted": audio.muted,
        }));
    });
    let devices = DeviceService::start(|devices| {
        say(&json!({
            "type": "audio-devices",
            "outputs": devices.outputs.iter().map(device).collect::<Vec<_>>(),
            "inputs": devices.inputs.iter().map(device).collect::<Vec<_>>(),
        }));
    });
    let brightness = BrightnessService::start(|displays| {
        let displays: Vec<Value> = displays
            .iter()
            .map(|d| json!({ "id": d.id, "label": d.label, "value": d.value }))
            .collect();
        say(&json!({ "type": "brightness", "displays": displays }));
    });
    let weather = WeatherService::start(|weather| {
        let mut weather = weather.clone();
        weather["type"] = json!("weather");
        say(&weather);
    });
    let theme = interface_settings();
    if let Some(theme) = &theme {
        say_theme(theme);
        theme.connect_changed(Some("color-scheme"), |settings, _| say_theme(settings));
    }
    let network = NetworkService::start(|network| {
        say(&json!({
            "type": "network",
            "connected": network.connected,
            "name": network.name,
            "wifiStrength": network.wifi_strength,
            "icon": network.icon_name(),
        }));
    });
    let battery = BatteryService::start(|battery| {
        say(&json!({
            "type": "battery",
            "present": battery.present,
            "percentage": battery.percentage,
            "charging": battery.charging,
            "icon": battery.icon_name,
        }));
    });
    let notifications = Notifications::start(|event| {
        let (kind, notification) = match event {
            NotificationEvent::Shown(n) => ("shown", n),
            NotificationEvent::Replaced(n) => ("replaced", n),
            NotificationEvent::Closed(id) => {
                forget_pictures(&pictures(), &format!("notification-{id}"), None);
                say(&json!({ "type": "notification", "event": "closed", "id": id }));
                return;
            }
        };
        let image = notification
            .image
            .as_ref()
            .and_then(|texture| save(texture, &format!("notification-{}", notification.id)));
        let app_icon = notification.app_icon.trim_start_matches("file://");
        say(&json!({
            "type": "notification",
            "event": kind,
            "id": notification.id,
            "app": notification.app_name,
            "appIcon": (!app_icon.is_empty()).then_some(app_icon),
            "summary": notification.summary,
            "body": notification.body,
            "actions": notification.actions,
            "urgency": notification.urgency,
            "timeout": notification.expire_timeout,
            "transient": notification.transient,
            "image": image,
        }));
    });
    let clipboard = ClipboardService::start(pictures(), |available, entries| {
        let entries: Vec<Value> = entries
            .iter()
            .map(|entry| match &entry.content {
                Content::Text(_) => json!({
                    "id": entry.id, "kind": "text", "text": entry.preview(),
                }),
                Content::Image { path, .. } => json!({
                    "id": entry.id, "kind": "image", "image": path.to_string_lossy(),
                }),
            })
            .collect();
        say(&json!({ "type": "clipboard", "available": available, "entries": entries }));
    });
    let wifi = WifiService::start(say);
    let bluetooth = BluetoothService::start(say);
    let profiles = ProfileService::start(say);
    let media = MediaService::start(say);
    let tray = Tray::start(|items| {
        let items: Vec<Value> = items
            .iter()
            .enumerate()
            .map(|(i, item)| tray_item(item, i))
            .collect();
        say(&json!({ "type": "tray", "items": items }));
    });

    // Commands, a line each, read on a thread of their own and handled on the main loop.
    let (sender, receiver) = async_channel::unbounded::<Value>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if let Ok(command) = serde_json::from_str(&line) {
                if sender.send_blocking(command).is_err() {
                    break;
                }
            }
        }
        // bento closed our stdin: it's gone.
        std::process::exit(0);
    });
    let services = Rc::new(RefCell::new((audio, notifications, tray)));
    glib::spawn_future_local(async move {
        while let Ok(command) = receiver.recv().await {
            let (audio, notifications, tray) = &*services.borrow();
            let id = || command["id"].as_u64().unwrap_or(0) as u32;
            let item = || command["id"].as_str().unwrap_or_default().to_owned();
            let text = |key: &str| command[key].as_str().unwrap_or_default().to_owned();
            let kind = || match command["kind"].as_str() {
                Some("input") => Kind::Input,
                _ => Kind::Output,
            };
            match command["cmd"].as_str().unwrap_or_default() {
                "volume" => audio.set_volume(command["value"].as_f64().unwrap_or(0.)),
                "mute" => audio.set_muted(command["value"].as_bool().unwrap_or(true)),
                "device" => devices.set_default(kind(), &text("name")),
                "device-volume" => devices.set_volume(
                    kind(),
                    &text("name"),
                    command["value"].as_f64().unwrap_or(0.),
                ),
                "device-mute" => devices.set_muted(
                    kind(),
                    &text("name"),
                    command["value"].as_bool().unwrap_or(true),
                ),
                "brightness" => {
                    brightness.set(&text("id"), command["value"].as_f64().unwrap_or(f64::NAN))
                }
                "brightness-refresh" => brightness.refresh(),
                "theme" => {
                    if let Some(theme) = &theme {
                        let dark = command["dark"].as_bool().unwrap_or(false);
                        let style = if dark { "prefer-dark" } else { "default" };
                        let _ = theme.set_string("color-scheme", style);
                    }
                }
                "weather" => {
                    let place = command["latitude"]
                        .as_f64()
                        .zip(command["longitude"].as_f64())
                        .map(|(latitude, longitude)| Place {
                            latitude,
                            longitude,
                            fahrenheit: command["fahrenheit"].as_bool().unwrap_or(false),
                        });
                    weather.set_place(place);
                }
                "run" => run_command(command["id"].clone(), &text("command")),
                "notify" => notify(
                    text("summary"),
                    text("body"),
                    text("icon"),
                    command["transient"].as_bool().unwrap_or(false),
                ),
                "wifi-enable" => wifi.enable(command["on"].as_bool().unwrap_or(true)),
                "wifi-scan" => wifi.scan(),
                "wifi-join" => {
                    wifi.join(text("ssid"), command["password"].as_str().map(String::from))
                }
                "wifi-disconnect" => wifi.disconnect(),
                "wifi-forget" => wifi.forget(&text("ssid")),
                "bluetooth-power" => bluetooth.power(command["on"].as_bool().unwrap_or(true)),
                "bluetooth-scan" => bluetooth.scan(command["on"].as_bool().unwrap_or(true)),
                "bluetooth-connect" => bluetooth.connect(text("path")),
                "bluetooth-disconnect" => bluetooth.disconnect(text("path")),
                "bluetooth-forget" => bluetooth.forget(text("path")),
                "power-profile" => profiles.set(text("name")),
                "media" => media.control(&text("player"), &text("action")),
                "dismiss" => notifications.dismiss(id()),
                "expire" => notifications.expire(id()),
                "invoke" => notifications.invoke(
                    id(),
                    command["key"].as_str().unwrap_or("default"),
                    command["token"].as_str(),
                ),
                "clipboard-copy" => clipboard.copy(command["id"].as_u64().unwrap_or(0)),
                "clipboard-delete" => clipboard.delete(command["id"].as_u64().unwrap_or(0)),
                "clipboard-clear" => clipboard.clear(),
                "activate" => tray.activate(&item(), 0, 0),
                "secondary" => tray.secondary_activate(&item(), 0, 0),
                "context" => tray.context_menu(&item(), 0, 0),
                "suspend" => power::suspend(),
                "reboot" => power::reboot(),
                "power-off" => power::power_off(),
                other => eprintln!("bento-shell --data: no command {other:?}"),
            }
        }
    });

    // The services hold on to what they report through.
    let _keep = (network, battery);
    main_loop.run();
    glib::ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn another_picture_for_the_same_thing_gets_another_path() {
        let dir = std::env::temp_dir().join(format!("bento-pictures-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A notification that replaces another keeps its id, with a picture of its own: a path
        // that showed one picture never shows another, since the shell keeps what it loaded.
        let first = save_png(&dir, "notification-7", b"one").unwrap();
        let second = save_png(&dir, "notification-7", b"two").unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&second).unwrap(), b"two");
        // The earlier one goes, and another notification's stays.
        let other = save_png(&dir, "notification-70", b"one").unwrap();
        save_png(&dir, "notification-7", b"three").unwrap();
        assert!(!first.exists() && !second.exists() && other.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
