//! Turning the computer off, restarting and suspending it, through logind.
//!
//! logind asks for a password when policy wants one. Logging out is bento's `quit` action:
//! `ipc::run_action("quit", vec![], {"skip-confirmation": true})`.

use gio::prelude::*;
use gio::{BusType, DBusCallFlags};

use super::dbus;

const SERVICE: &str = "org.freedesktop.login1";
const PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";

pub fn power_off() {
    call("PowerOff");
}

pub fn reboot() {
    call("Reboot");
}

pub fn suspend() {
    call("Suspend");
}

/// Calls one of logind's power methods, letting it ask for authentication.
fn call(method: &'static str) {
    glib::spawn_future_local(async move {
        let Some(bus) = dbus::bus(BusType::System).await else {
            return;
        };
        let reply = bus
            .call_future(
                Some(SERVICE),
                PATH,
                MANAGER,
                method,
                Some(&(true,).to_variant()),
                None,
                DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
                -1,
            )
            .await;
        if let Err(err) = reply {
            eprintln!("Couldn't ask logind to {method}: {err}");
        }
    });
}
