//! An equalizer for each output: ten bands, each output's own, kept in
//! `$XDG_STATE_HOME/bento/equalizers.json`. One that's on runs as a rig of its own, a filter-chain
//! sink named `bento.eq.` and the output's name, which plays on the output; what plays on the
//! output goes through it, and the devices say it as the output itself. Its bands change as it
//! plays.
//!
//! Turned on, one waits for its sink, then what plays on the output goes through it and it takes
//! the output's volume, the output at full under it ("adopted"); turned off, that's undone before
//! it goes. Its volume is kept too, since a sink made again, after a re-login or a PipeWire
//! restart, starts at full: one that comes back gets it again ("resumed").

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};

use super::rigs::{self, Rigs};

/// What an output's equalizer is named, before the output's own name.
pub const PREFIX: &str = "bento.eq.";
/// The bands' frequencies, in Hz, an octave apart.
pub const BANDS: [f64; 10] = [
    31., 62., 125., 250., 500., 1000., 2000., 4000., 8000., 16000.,
];
/// The most a band boosts or cuts, in dB.
const RANGE: f64 = 12.;
/// An octave wide.
const Q: f64 = 1.41;

#[derive(Debug, Clone, PartialEq)]
pub struct Equalizer {
    pub on: bool,
    /// The preset it was set to, which the shell names; "Custom" once a band is moved.
    pub preset: String,
    /// Each band's gain, in dB.
    pub gains: [f64; 10],
    /// The output's title, for the equalizer's description.
    pub title: String,
    /// Its volume, from 0, which is the output's while it's on: kept for when it's made again.
    pub volume: Option<f64>,
}

impl Default for Equalizer {
    fn default() -> Self {
        Self {
            on: false,
            preset: "Flat".into(),
            gains: [0.; 10],
            title: String::new(),
            volume: None,
        }
    }
}

impl Equalizer {
    fn from_value(value: &Value) -> Self {
        let mut gains = [0.; 10];
        for (gain, value) in gains
            .iter_mut()
            .zip(value["gains"].as_array().into_iter().flatten())
        {
            *gain = value.as_f64().unwrap_or(0.).clamp(-RANGE, RANGE);
        }
        Self {
            on: value["on"].as_bool().unwrap_or(false),
            preset: value["preset"].as_str().unwrap_or("Custom").to_owned(),
            gains,
            title: value["title"].as_str().unwrap_or_default().to_owned(),
            volume: value["volume"].as_f64(),
        }
    }

    fn to_value(&self) -> Value {
        json!({
            "on": self.on, "preset": self.preset, "gains": self.gains, "title": self.title,
            "volume": self.volume,
        })
    }
}

/// The name of the equalizer of the output named `device`.
pub fn sink(device: &str) -> String {
    format!("{PREFIX}{device}")
}

/// The config of the rig that makes `device`'s equalizer, which `title` names for the user.
pub fn config(device: &str, title: &str, equalizer: &Equalizer, positions: &[String]) -> String {
    // The output's own channels, so nothing of it is lost; stereo until they're known.
    let positions = if positions.is_empty() {
        vec!["FL".to_owned(), "FR".to_owned()]
    } else {
        positions.to_vec()
    };
    let nodes: Vec<String> = BANDS
        .iter()
        .zip(equalizer.gains)
        .enumerate()
        .map(|(i, (freq, gain))| {
            format!(
                "{{ type = builtin name = band{i} label = bq_peaking control = {{ \"Freq\" = {freq:.1} \"Q\" = {Q} \"Gain\" = {gain:.1} }} }}"
            )
        })
        .collect();
    let links: Vec<String> = (1..BANDS.len())
        .map(|i| {
            format!(
                "{{ output = \"band{}:Out\" input = \"band{i}:In\" }}",
                i - 1
            )
        })
        .collect();
    let mut out = String::from(rigs::HEADER);
    out.push_str(&format!(
        r#"    {{ name = libpipewire-module-filter-chain
        args = {{
            node.description = {description}
            audio.channels = {channels}
            audio.position = [ {position} ]
            filter.graph = {{
                nodes = [
                    {nodes}
                ]
                links = [ {links} ]
            }}
            capture.props = {{
                node.name = {name}
                media.class = Audio/Sink
                priority.session = 0
                priority.driver = 0
            }}
            playback.props = {{
                node.name = {output}
                target.object = {device}
                node.passive = true
                node.dont-move = true
                stream.dont-remix = true
                bento.internal = true
            }}
        }}
    }}
]
"#,
        description = rigs::quoted(&format!("{title} (Equalizer)")),
        channels = positions.len(),
        position = positions.join(" "),
        nodes = nodes.join("\n                    "),
        links = links.join(" "),
        name = rigs::quoted(&sink(device)),
        output = rigs::quoted(&format!("{}.out", sink(device))),
        device = rigs::quoted(device),
    ));
    out
}

/// Keeps each output's equalizer, and runs those that are on.
pub struct EqualizerService {
    equalizers: RefCell<BTreeMap<String, Equalizer>>,
    rigs: Rc<Rigs>,
    /// Whether PipeWire's filter-chain is there to make them.
    available: bool,
    /// Whether the sound server is PipeWire's, as the devices say once they're read: until
    /// then, and on PulseAudio itself, nothing runs.
    on_pipewire: std::cell::Cell<bool>,
    on_change: Box<dyn Fn(&Value)>,
    /// Hears an output whose equalizer turns off, before it goes, to undo its adoption.
    on_release: RefCell<Option<Box<dyn Fn(&str)>>>,
    /// Gives an output's channels, by PipeWire's names.
    positions: RefCell<Option<Box<dyn Fn(&str) -> Vec<String>>>>,
    /// The outputs whose equalizer is on but not there yet: `true` for one just turned on, to
    /// be adopted, `false` for one coming back, to be resumed.
    pending: RefCell<BTreeMap<String, bool>>,
    /// The outputs whose equalizer has taken over, with when, since its volume is only its own
    /// once it has settled.
    adopted: RefCell<BTreeMap<String, std::time::Instant>>,
}

impl EqualizerService {
    pub fn start(on_change: impl Fn(&Value) + 'static) -> Rc<Self> {
        let this = Rc::new(Self {
            equalizers: RefCell::new(load()),
            rigs: Rigs::new(),
            available: rigs::can_run() && rigs::module("libpipewire-module-filter-chain").is_some(),
            on_pipewire: std::cell::Cell::new(false),
            on_change: Box::new(on_change),
            on_release: RefCell::default(),
            positions: RefCell::default(),
            pending: RefCell::default(),
            adopted: RefCell::default(),
        });
        this.apply();
        this
    }

    /// Whether equalizers can run: filter-chain is there, and the sound server is PipeWire's.
    fn usable(&self) -> bool {
        self.available && self.on_pipewire.get()
    }

    /// Notes whether the sound server is PipeWire's, running the equalizers once it is.
    pub fn set_pipewire(&self, on_pipewire: bool) {
        if self.on_pipewire.replace(on_pipewire) != on_pipewire {
            self.apply();
        }
    }

    pub fn on_release(&self, heard: impl Fn(&str) + 'static) {
        *self.on_release.borrow_mut() = Some(Box::new(heard));
    }

    pub fn set_positions(&self, positions: impl Fn(&str) -> Vec<String> + 'static) {
        *self.positions.borrow_mut() = Some(Box::new(positions));
    }

    fn positions(&self, device: &str) -> Vec<String> {
        self.positions
            .borrow()
            .as_ref()
            .map(|positions| positions(device))
            .unwrap_or_default()
    }

    /// The outputs whose equalizer is on and waits for its sink: `true` to adopt, `false` to
    /// resume, with the volume to resume at.
    pub fn pending(&self) -> Vec<(String, bool, Option<f64>)> {
        let equalizers = self.equalizers.borrow();
        let pending = self.pending.borrow();
        let pending = pending.iter();
        pending
            .map(|(device, fresh)| {
                let volume = equalizers.get(device).and_then(|eq| eq.volume);
                (device.clone(), *fresh, volume)
            })
            .collect()
    }

    /// Notes that the equalizer of `device` has taken over.
    pub fn adopted(&self, device: &str) {
        self.pending.borrow_mut().remove(device);
        self.adopted
            .borrow_mut()
            .insert(device.to_owned(), std::time::Instant::now());
    }

    /// The outputs whose equalizer has taken over.
    pub fn adopted_outputs(&self) -> Vec<String> {
        self.adopted.borrow().keys().cloned().collect()
    }

    /// The equalizer of `device` went, though it's on, as when PipeWire restarts: it's resumed
    /// once it's there again.
    pub fn lost(&self, device: &str) {
        if self.adopted.borrow_mut().remove(device).is_some() {
            self.pending.borrow_mut().insert(device.to_owned(), false);
        }
    }

    /// Keeps `volume` as the volume of the equalizer of `device`, which has taken over, once it
    /// has settled there.
    pub fn note_volume(&self, device: &str, volume: f64) {
        let settled = self
            .adopted
            .borrow()
            .get(device)
            .is_some_and(|at| at.elapsed() > std::time::Duration::from_secs(1));
        if !settled {
            return;
        }
        let mut equalizers = self.equalizers.borrow_mut();
        let Some(equalizer) = equalizers.get_mut(device) else {
            return;
        };
        if equalizer.volume.is_some_and(|v| (v - volume).abs() < 0.001) {
            return;
        }
        equalizer.volume = Some(volume);
        save(&equalizers);
    }

    /// The outputs whose equalizer is on.
    pub fn on_outputs(&self) -> std::collections::BTreeSet<String> {
        let equalizers = self.equalizers.borrow();
        let on = equalizers.iter().filter(|(_, eq)| eq.on);
        on.map(|(device, _)| device.clone()).collect()
    }

    /// Turns off the equalizer of `device`, and forgets it, as for a combination removed.
    pub fn forget(self: &Rc<Self>, device: &str) {
        let Some(equalizer) = self.equalizers.borrow().get(device).cloned() else {
            return;
        };
        if equalizer.on {
            let mut off = equalizer.to_value();
            off["on"] = false.into();
            self.set(device, &off);
        }
        self.equalizers.borrow_mut().remove(device);
        save(&self.equalizers.borrow());
        self.publish();
    }

    /// Sets the equalizer of the output named `device`. Bands change as it plays; turning it on
    /// makes it, to be adopted once it's there, and turning it off undoes its adoption before
    /// it goes.
    pub fn set(self: &Rc<Self>, device: &str, value: &Value) {
        if device.is_empty() || device.starts_with(PREFIX) {
            return;
        }
        let old = self
            .equalizers
            .borrow()
            .get(device)
            .cloned()
            .unwrap_or_default();
        let mut new = Equalizer::from_value(value);
        // The volume is the data program's to keep, not the shell's.
        new.volume = old.volume;
        self.equalizers
            .borrow_mut()
            .insert(device.to_owned(), new.clone());
        save(&self.equalizers.borrow());
        if !self.usable() {
            self.publish();
            return;
        }
        match (old.on, new.on) {
            (true, true) => {
                self.tune(device, &old, &new);
                let config = config(device, &self.title(device), &new, &self.positions(device));
                self.rigs.remember(&rig_id(device), &config);
                self.publish();
            }
            (false, true) => {
                self.pending.borrow_mut().insert(device.to_owned(), true);
                self.apply();
            }
            (true, false) => {
                self.pending.borrow_mut().remove(device);
                if self.adopted.borrow_mut().remove(device).is_some() {
                    // What plays through it goes back to the output before it goes.
                    if let Some(heard) = self.on_release.borrow().as_ref() {
                        heard(device);
                    }
                    let this = Rc::downgrade(self);
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(400),
                        move || {
                            if let Some(this) = this.upgrade() {
                                this.apply();
                            }
                        },
                    );
                } else {
                    // Never taken over: nothing to undo.
                    self.apply();
                }
            }
            (false, false) => self.publish(),
        }
    }

    fn title(&self, device: &str) -> String {
        let equalizers = self.equalizers.borrow();
        let title = equalizers.get(device).map(|eq| eq.title.clone());
        title
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| device.to_owned())
    }

    /// Sets the bands that changed on the running equalizer.
    fn tune(&self, device: &str, old: &Equalizer, new: &Equalizer) {
        let params: Vec<String> = old
            .gains
            .iter()
            .zip(new.gains)
            .enumerate()
            .filter(|(_, (was, is))| *was != is)
            .map(|(i, (_, gain))| format!("\"band{i}:Gain\" {gain:.1}"))
            .collect();
        if params.is_empty() {
            return;
        }
        let node = sink(device);
        let props = format!("{{ params = [ {} ] }}", params.join(" "));
        let args = ["pw-cli", "set-param", &node, "Props", &props];
        let args: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
        let flags = gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE;
        if let Ok(process) = gio::Subprocess::newv(&args, flags) {
            process.wait_async(gio::Cancellable::NONE, |_| ());
        }
    }

    /// Runs the equalizers that are on, stops the rest, and says so.
    fn apply(&self) {
        if self.usable() {
            let equalizers = self.equalizers.borrow();
            let wanted: Vec<String> = equalizers
                .iter()
                .filter(|(_, eq)| eq.on)
                .map(|(device, _)| rig_id(device))
                .collect();
            for id in self.rigs.ids() {
                if !wanted.contains(&id) {
                    self.rigs.set(&id, None);
                }
            }
            for (device, equalizer) in equalizers.iter().filter(|(_, eq)| eq.on) {
                let config = config(
                    device,
                    &self.title(device),
                    equalizer,
                    &self.positions(device),
                );
                self.rigs.set(&rig_id(device), Some(&config));
                // One on that hasn't taken over, as at the start, comes back as it was.
                let adopted = self.adopted.borrow().contains_key(device);
                let mut pending = self.pending.borrow_mut();
                if !adopted && !pending.contains_key(device) {
                    pending.insert(device.clone(), false);
                }
            }
            let mut pending = self.pending.borrow_mut();
            pending.retain(|device, _| equalizers.get(device).is_some_and(|eq| eq.on));
            let mut adopted = self.adopted.borrow_mut();
            adopted.retain(|device, _| equalizers.get(device).is_some_and(|eq| eq.on));
        }
        self.publish();
    }

    fn publish(&self) {
        let equalizers = self.equalizers.borrow();
        let devices: serde_json::Map<String, Value> = equalizers
            .iter()
            .map(|(device, eq)| (device.clone(), eq.to_value()))
            .collect();
        (self.on_change)(&json!({
            "type": "equalizers",
            "available": self.usable(),
            "bands": BANDS,
            "range": RANGE,
            "devices": devices,
        }));
    }
}

/// A rig's id, from its output's name, kept to what a file's name can be.
fn rig_id(device: &str) -> String {
    let safe: String = device
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("eq-{safe}")
}

fn file() -> PathBuf {
    glib::user_state_dir().join("bento").join("equalizers.json")
}

fn load() -> BTreeMap<String, Equalizer> {
    let Ok(text) = std::fs::read_to_string(file()) else {
        return BTreeMap::new();
    };
    let value: Value = serde_json::from_str(&text).unwrap_or_default();
    value
        .as_object()
        .into_iter()
        .flatten()
        .map(|(device, eq)| (device.clone(), Equalizer::from_value(eq)))
        .collect()
}

fn save(equalizers: &BTreeMap<String, Equalizer>) {
    let path = file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let value: serde_json::Map<String, Value> = equalizers
        .iter()
        .map(|(device, eq)| (device.clone(), eq.to_value()))
        .collect();
    let _ = std::fs::write(
        path,
        serde_json::to_string_pretty(&value).unwrap_or_default(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_equalizer_is_ten_bands_in_a_row_that_play_on_its_output() {
        let mut equalizer = Equalizer::default();
        equalizer.gains[0] = 6.;
        equalizer.gains[9] = -3.;
        let config = config("alsa_output.a", "Line Out", &equalizer, &[]);
        assert_eq!(config.matches("label = bq_peaking").count(), 10);
        assert!(config.contains("name = band0 label = bq_peaking control = { \"Freq\" = 31.0 \"Q\" = 1.41 \"Gain\" = 6.0 }"));
        assert!(config.contains("\"Gain\" = -3.0"));
        assert!(config.contains("{ output = \"band8:Out\" input = \"band9:In\" }"));
        assert!(config.contains("node.name = \"bento.eq.alsa_output.a\""));
        assert!(config.contains("target.object = \"alsa_output.a\""));
        assert!(config.contains("node.description = \"Line Out (Equalizer)\""));
    }

    #[test]
    fn an_equalizer_has_its_outputs_channels() {
        let positions: Vec<String> = ["FL", "FR", "RL", "RR"].map(String::from).into();
        let four = config(
            "bento.combine.c1",
            "Front + Back",
            &Equalizer::default(),
            &positions,
        );
        assert!(four.contains("audio.channels = 4"), "{four}");
        assert!(four.contains("audio.position = [ FL FR RL RR ]"));
        // Without the output's channels, as before it's read: stereo.
        let two = config("alsa_output.a", "Line Out", &Equalizer::default(), &[]);
        assert!(two.contains("audio.channels = 2"));
        assert!(two.contains("audio.position = [ FL FR ]"));
    }

    #[test]
    fn gains_are_kept_to_the_range() {
        let equalizer = Equalizer::from_value(&json!({ "on": true, "gains": [20, -20, 3] }));
        assert_eq!(equalizer.gains[..3], [12., -12., 3.]);
        assert_eq!(equalizer.gains[3], 0.);
        assert_eq!(rig_id("bento.combine.c1"), "eq-bento.combine.c1");
        assert_eq!(rig_id("a b/c"), "eq-a_b_c");
    }
}
