use std::collections::HashMap;

use futures_util::StreamExt;
use niri_config::Color;
use zbus::zvariant::{OwnedValue, Value};

const NAMESPACE: &str = "org.gnome.desktop.interface";
const ICON_THEME: &str = "icon-theme";
const APPEARANCE: &str = "org.freedesktop.appearance";
const ACCENT_COLOR: &str = "accent-color";

pub enum PortalSettingsToNiri {
    /// The icon theme from GNOME's interface settings; `None` if the portal has no value for it.
    IconTheme(Option<String>),
    /// The accent colour; `None` if the portal has none or the person didn't pick one.
    AccentColor(Option<Color>),
}

/// The message for a setting the portal gave, if it's one bento follows.
fn message(namespace: &str, key: &str, value: &Value) -> Option<PortalSettingsToNiri> {
    match (namespace, key) {
        (NAMESPACE, ICON_THEME) => Some(PortalSettingsToNiri::IconTheme(string_of(value))),
        (APPEARANCE, ACCENT_COLOR) => Some(PortalSettingsToNiri::AccentColor(color_of(value))),
        _ => None,
    }
}

/// The colour of an accent-color value: red, green and blue from 0 to 1, where any out of that
/// range means no accent was picked.
fn color_of(value: &Value) -> Option<Color> {
    match value {
        Value::Value(inner) => color_of(inner),
        Value::Structure(rgb) => {
            let [Value::F64(r), Value::F64(g), Value::F64(b)] = rgb.fields() else {
                return None;
            };
            let rgb = [*r, *g, *b];
            let [r, g, b] = rgb.map(|x| x as f32);
            rgb.iter()
                .all(|x| (0. ..=1.).contains(x))
                .then(|| Color::from_array_unpremul([r, g, b, 1.]))
        }
        _ => None,
    }
}

/// The text of a setting's value, which some portal backends wrap in one more variant.
fn string_of(value: &Value) -> Option<String> {
    match value {
        Value::Str(text) => Some(text.to_string()),
        Value::Value(inner) => string_of(inner),
        _ => None,
    }
}

/// Follows the settings portal for the icon theme, which apps read the same way.
pub fn start(
    to_niri: calloop::channel::Sender<PortalSettingsToNiri>,
) -> anyhow::Result<zbus::blocking::Connection> {
    let conn = zbus::blocking::Connection::session()?;

    let async_conn = conn.inner().clone();
    let future = async move {
        let proxy = zbus::Proxy::new(
            &async_conn,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
        )
        .await;
        let proxy = match proxy {
            Ok(x) => x,
            Err(err) => {
                warn!("error creating the settings portal Proxy: {err:?}");
                return;
            }
        };

        let mut changed = match proxy.receive_signal("SettingChanged").await {
            Ok(x) => x,
            Err(err) => {
                warn!("error subscribing to SettingChanged: {err:?}");
                return;
            }
        };

        let namespaces: &[&str] = &[NAMESPACE, APPEARANCE];
        let all: zbus::Result<HashMap<String, HashMap<String, OwnedValue>>> =
            proxy.call("ReadAll", &(namespaces,)).await;
        match all {
            Ok(all) => {
                let theme = all
                    .get(NAMESPACE)
                    .and_then(|settings| settings.get(ICON_THEME))
                    .and_then(|value| string_of(value));
                let accent = all
                    .get(APPEARANCE)
                    .and_then(|settings| settings.get(ACCENT_COLOR))
                    .and_then(|value| color_of(value));
                trace!("initial icon theme: {theme:?}, accent colour: {accent:?}");
                for msg in [
                    PortalSettingsToNiri::IconTheme(theme),
                    PortalSettingsToNiri::AccentColor(accent),
                ] {
                    if let Err(err) = to_niri.send(msg) {
                        warn!("error sending message to niri: {err:?}");
                        return;
                    }
                }
            }
            Err(err) => {
                // No portal, or none that knows these settings: the GTK settings file decides.
                debug!("error reading the portal's interface settings: {err:?}");
            }
        }

        while let Some(signal) = changed.next().await {
            let body = signal.body();
            let (namespace, key, value): (String, String, OwnedValue) = match body.deserialize() {
                Ok(x) => x,
                Err(err) => {
                    warn!("error parsing SettingChanged args: {err:?}");
                    return;
                }
            };
            let Some(msg) = message(&namespace, &key, &value) else {
                continue;
            };

            trace!("changed {namespace} {key}: {value:?}");
            if let Err(err) = to_niri.send(msg) {
                warn!("error sending message to niri: {err:?}");
                return;
            }
        }
    };

    let task = conn
        .inner()
        .executor()
        .spawn(future, "monitor settings portal changes");
    task.detach();

    Ok(conn)
}
