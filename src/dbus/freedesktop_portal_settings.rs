use std::collections::HashMap;

use futures_util::StreamExt;
use zbus::zvariant::{OwnedValue, Value};

const NAMESPACE: &str = "org.gnome.desktop.interface";
const ICON_THEME: &str = "icon-theme";

pub enum PortalSettingsToNiri {
    /// The icon theme from GNOME's interface settings; `None` if the portal has no value for it.
    IconTheme(Option<String>),
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

        let namespaces: &[&str] = &[NAMESPACE];
        let all: zbus::Result<HashMap<String, HashMap<String, OwnedValue>>> =
            proxy.call("ReadAll", &(namespaces,)).await;
        match all {
            Ok(all) => {
                let theme = all
                    .get(NAMESPACE)
                    .and_then(|settings| settings.get(ICON_THEME))
                    .and_then(|value| string_of(value));
                trace!("initial icon theme: {theme:?}");
                if let Err(err) = to_niri.send(PortalSettingsToNiri::IconTheme(theme)) {
                    warn!("error sending message to niri: {err:?}");
                    return;
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
            if namespace != NAMESPACE || key != ICON_THEME {
                continue;
            }

            trace!("changed icon theme: {value:?}");
            let msg = PortalSettingsToNiri::IconTheme(string_of(&value));
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
