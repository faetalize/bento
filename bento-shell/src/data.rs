//! `bento-shell --data`: the shell's data, for bento's own shell, without a window.
//!
//! bento starts it and reads what it says from its stdout, a JSON object per line: the speaker's
//! volume, the network, the battery, notifications and the tray's items, each as it changes.
//! Pictures, like a notification's image or a tray icon an app draws itself, are written as PNG
//! files under `$XDG_RUNTIME_DIR/bento-shell` and named by their path. It takes commands on its
//! stdin, a JSON object per line, like `{ "cmd": "volume", "value": 0.5 }`.

use std::cell::RefCell;
use std::io::{BufRead as _, Write as _};
use std::path::PathBuf;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use serde_json::{json, Value};

use crate::services::{
    power, AudioService, BatteryService, NetworkService, NotificationEvent, Notifications, Tray,
    TrayIcon, TrayItem,
};

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
    let path = pictures().join(format!("{name}.png"));
    texture.save_to_png(&path).ok()?;
    Some(path.to_string_lossy().into_owned())
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
            "image": image,
        }));
    });
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
            match command["cmd"].as_str().unwrap_or_default() {
                "volume" => audio.set_volume(command["value"].as_f64().unwrap_or(0.)),
                "mute" => audio.set_muted(command["value"].as_bool().unwrap_or(true)),
                "dismiss" => notifications.dismiss(id()),
                "expire" => notifications.expire(id()),
                "invoke" => {
                    notifications.invoke(id(), command["key"].as_str().unwrap_or("default"))
                }
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
