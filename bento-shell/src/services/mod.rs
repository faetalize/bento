//! The system services the shell shows: each one watches its part of the system on the main loop
//! and calls back with its state.

pub mod audio;
pub mod battery;
mod dbus;
pub mod ipc;
pub mod network;
pub mod notifications;
pub mod power;
pub mod tray;

pub use audio::{Audio, AudioService};
pub use battery::{Battery, BatteryService};
pub use ipc::Workspaces;
pub use network::{Network, NetworkService};
pub use notifications::{Event as NotificationEvent, Notification, Notifications};
pub use tray::{Tray, TrayIcon, TrayItem};
