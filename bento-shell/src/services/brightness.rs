//! How bright the displays are, and making them brighter or dimmer.
//!
//! A laptop's own panel has a backlight the kernel shows under `/sys/class/backlight`, which
//! logind sets for the session. A monitor is asked over its cable, by DDC/CI, with `ddcutil`:
//! each ask takes a moment, so they run one at a time for a display, and only the last of the
//! values asked for while one runs is sent after it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gio::prelude::*;
use gio::{BusType, DBusCallFlags};

use super::dbus;

/// A display whose brightness can be set.
#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    /// The output's name, like `DP-2`, where it's known, or the backlight's.
    pub id: String,
    /// What to call it for the user.
    pub label: String,
    /// From 0 to 1.
    pub value: f64,
    how: How,
    max: u32,
}

#[derive(Debug, Clone, PartialEq)]
enum How {
    /// A backlight, by its name in `/sys/class/backlight`.
    Backlight(String),
    /// A monitor, by its I2C bus.
    Ddc(u32),
}

pub struct BrightnessService {
    displays: RefCell<Vec<Display>>,
    /// The displays being set, each with the value to send once the one on its way is there.
    setting: RefCell<HashMap<String, Option<f64>>>,
    on_change: Box<dyn Fn(&[Display])>,
}

impl BrightnessService {
    pub fn start(on_change: impl Fn(&[Display]) + 'static) -> Rc<Self> {
        let this = Rc::new(Self {
            displays: RefCell::default(),
            setting: RefCell::default(),
            on_change: Box::new(on_change),
        });
        this.detect();
        this
    }

    /// Finds the displays and reads how bright each is.
    fn detect(self: &Rc<Self>) {
        let this = self.clone();
        glib::spawn_future_local(async move {
            let mut displays = backlights();
            if let Some(found) = run(&["ddcutil", "detect", "--brief"]).await {
                for (bus, connector, label) in parse_detect(&found) {
                    let bus_arg = bus.to_string();
                    let read = ["ddcutil", "getvcp", "10", "--brief", "--bus", &bus_arg];
                    let Some((current, max)) = run(&read).await.as_deref().and_then(parse_vcp)
                    else {
                        continue;
                    };
                    displays.push(Display {
                        id: connector.unwrap_or_else(|| format!("i2c-{bus}")),
                        label,
                        value: f64::from(current) / f64::from(max),
                        how: How::Ddc(bus),
                        max,
                    });
                }
            }
            *this.displays.borrow_mut() = displays;
            this.notify();
        });
    }

    /// Reads how bright each display is again, since a monitor's own buttons change it too.
    pub fn refresh(self: &Rc<Self>) {
        let this = self.clone();
        glib::spawn_future_local(async move {
            let displays = this.displays.borrow().clone();
            for display in displays {
                // One that's being set says where it is once it's there.
                if this.setting.borrow().contains_key(&display.id) {
                    continue;
                }
                let value = match &display.how {
                    How::Backlight(name) => read_backlight(name).map(|(now, max)| (now, max)),
                    How::Ddc(bus) => {
                        let bus = bus.to_string();
                        run(&["ddcutil", "getvcp", "10", "--brief", "--bus", &bus])
                            .await
                            .as_deref()
                            .and_then(parse_vcp)
                    }
                };
                if let Some((now, max)) = value {
                    this.took(&display.id, f64::from(now) / f64::from(max.max(1)));
                }
            }
        });
    }

    /// Sets the display `id` to `value`, from 0 to 1.
    pub fn set(self: &Rc<Self>, id: &str, value: f64) {
        let value = if value.is_finite() {
            value.clamp(0., 1.)
        } else {
            return;
        };
        let Some(display) = self.displays.borrow().iter().find(|d| d.id == id).cloned() else {
            return;
        };
        if let Some(waiting) = self.setting.borrow_mut().get_mut(id) {
            *waiting = Some(value);
            return;
        }
        self.setting.borrow_mut().insert(id.to_owned(), None);
        let this = self.clone();
        glib::spawn_future_local(async move {
            let mut value = value;
            loop {
                let raw = (value * f64::from(display.max)).round() as u32;
                match &display.how {
                    How::Backlight(name) => set_backlight(name, raw).await,
                    How::Ddc(bus) => {
                        let (bus, raw) = (bus.to_string(), raw.to_string());
                        let set = ["ddcutil", "setvcp", "10", &raw, "--bus", &bus, "--noverify"];
                        run(&set).await;
                    }
                }
                this.took(&display.id, value);
                // The last of what was asked for meanwhile, if anything.
                let next = this
                    .setting
                    .borrow_mut()
                    .get_mut(&display.id)
                    .and_then(Option::take);
                match next {
                    Some(next) => value = next,
                    None => break,
                }
            }
            this.setting.borrow_mut().remove(&display.id);
        });
    }

    /// The display `id` is at `value` now.
    fn took(&self, id: &str, value: f64) {
        let changed = {
            let mut displays = self.displays.borrow_mut();
            let Some(display) = displays.iter_mut().find(|d| d.id == id) else {
                return;
            };
            let changed = (display.value - value).abs() > 1e-6;
            display.value = value;
            changed
        };
        if changed {
            self.notify();
        }
    }

    fn notify(&self) {
        (self.on_change)(&self.displays.borrow());
    }
}

/// Runs a program and gives what it printed, if it ran and succeeded.
async fn run(command: &[&str]) -> Option<String> {
    let flags = gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE;
    let args: Vec<&std::ffi::OsStr> = command.iter().map(std::ffi::OsStr::new).collect();
    let process = gio::Subprocess::newv(&args, flags).ok()?;
    let (out, _) = process.communicate_utf8_future(None).await.ok()?;
    process
        .is_successful()
        .then(|| out.unwrap_or_default().to_string())
}

/// The monitors `ddcutil detect --brief` found: each one's I2C bus, its output's name where it
/// says, and what it's called.
fn parse_detect(text: &str) -> Vec<(u32, Option<String>, String)> {
    let mut found = Vec::new();
    let mut current: Option<(Option<u32>, Option<String>, Option<String>)> = None;
    let mut push = |display: Option<(Option<u32>, Option<String>, Option<String>)>| {
        if let Some((Some(bus), connector, label)) = display {
            let label = label.unwrap_or_else(|| String::from("Monitor"));
            found.push((bus, connector, label));
        }
    };
    for line in text.lines() {
        let trimmed = line.trim();
        // A display that answers starts a block; one that doesn't is "Invalid display".
        if !line.starts_with(char::is_whitespace) {
            push(current.take());
            if trimmed.starts_with("Display ") {
                current = Some((None, None, None));
            }
            continue;
        }
        let Some(display) = current.as_mut() else {
            continue;
        };
        if let Some(bus) = trimmed.strip_prefix("I2C bus:") {
            display.0 = bus.trim().rsplit('-').next().and_then(|n| n.parse().ok());
        } else if let Some(connector) = trimmed.strip_prefix("DRM connector:") {
            // `card1-DP-2` is output `DP-2`.
            let connector = connector.trim();
            display.1 = connector
                .split_once('-')
                .map(|(_, name)| name.to_owned())
                .filter(|name| !name.is_empty());
        } else if let Some(monitor) = trimmed.strip_prefix("Monitor:") {
            // Maker, model and serial number, between colons.
            let mut parts = monitor.trim().split(':');
            let (_maker, model) = (parts.next(), parts.next());
            display.2 = model
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .map(String::from);
        }
    }
    push(current.take());
    found
}

/// The current and the largest value in `ddcutil getvcp --brief`'s `VCP 10 C 35 100`.
fn parse_vcp(text: &str) -> Option<(u32, u32)> {
    let mut words = text.split_whitespace();
    if words.next()? != "VCP" {
        return None;
    }
    let (_code, _kind) = (words.next()?, words.next()?);
    let current = words.next()?.parse().ok()?;
    let max: u32 = words.next()?.parse().ok()?;
    (max > 0).then_some((current, max))
}

fn backlight_dir() -> PathBuf {
    PathBuf::from("/sys/class/backlight")
}

fn read_backlight(name: &str) -> Option<(u32, u32)> {
    let read = |file: &str| -> Option<u32> {
        std::fs::read_to_string(backlight_dir().join(name).join(file))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    let max = read("max_brightness")?;
    (max > 0).then_some((read("brightness")?, max))
}

/// The backlights the kernel shows.
fn backlights() -> Vec<Display> {
    let Ok(entries) = std::fs::read_dir(backlight_dir()) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().to_string_lossy().into_owned();
            let (now, max) = read_backlight(&name)?;
            Some(Display {
                id: format!("backlight-{name}"),
                label: String::from("Built-in display"),
                value: f64::from(now) / f64::from(max),
                how: How::Backlight(name),
                max,
            })
        })
        .collect()
}

/// Sets a backlight through logind, which lets the session's user do it.
async fn set_backlight(name: &str, value: u32) {
    let Some(bus) = dbus::bus(BusType::System).await else {
        return;
    };
    let reply = bus
        .call_future(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1/session/auto",
            "org.freedesktop.login1.Session",
            "SetBrightness",
            Some(&("backlight", name, value).to_variant()),
            None,
            DBusCallFlags::NONE,
            -1,
        )
        .await;
    if let Err(err) = reply {
        eprintln!("Couldn't set the backlight {name}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_reads_the_monitors_ddcutil_finds() {
        let text = "\
Display 1
   I2C bus:          /dev/i2c-7
   DRM connector:    card1-DP-2
   drm_connector_id: 520
   Monitor:          ACM:Example 27Q:

Invalid display
   I2C bus:          /dev/i2c-3
   DRM connector:    card1-eDP-1
   Monitor:          AAA:Panel:

Display 2
   I2C bus:          /dev/i2c-8
   Monitor:          BBB:Other 24:SERIAL123
";
        assert_eq!(
            parse_detect(text),
            vec![
                (7, Some(String::from("DP-2")), String::from("Example 27Q")),
                (8, None, String::from("Other 24")),
            ]
        );
    }

    #[test]
    fn it_reads_a_brightness_and_its_largest() {
        assert_eq!(parse_vcp("VCP 10 C 35 100\n"), Some((35, 100)));
        assert_eq!(parse_vcp("VCP 10 ERR\n"), None);
        assert_eq!(parse_vcp(""), None);
    }
}
