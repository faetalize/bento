//! An equalizer for each output: ten bands, each output's own, kept in
//! `$XDG_STATE_HOME/bento/equalizers.json`. One that's on runs as a rig of its own, a filter-chain
//! sink named `bento.eq.` and the output's name, which plays on the output; what plays on the
//! output goes through it, and the devices say it as the output itself. Its bands change as it
//! plays.

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
}

impl Default for Equalizer {
    fn default() -> Self {
        Self {
            on: false,
            preset: "Flat".into(),
            gains: [0.; 10],
            title: String::new(),
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
        }
    }

    fn to_value(&self) -> Value {
        json!({ "on": self.on, "preset": self.preset, "gains": self.gains, "title": self.title })
    }
}

/// The name of the equalizer of the output named `device`.
pub fn sink(device: &str) -> String {
    format!("{PREFIX}{device}")
}

/// The config of the rig that makes `device`'s equalizer, which `title` names for the user.
pub fn config(device: &str, title: &str, equalizer: &Equalizer) -> String {
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
            audio.channels = 2
            audio.position = [ FL FR ]
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
    /// Hears an output whose equalizer turns on, once it's there, or off, before it goes.
    on_turn: RefCell<Option<Box<dyn Fn(&str, bool)>>>,
}

impl EqualizerService {
    pub fn start(on_change: impl Fn(&Value) + 'static) -> Rc<Self> {
        let this = Rc::new(Self {
            equalizers: RefCell::new(load()),
            rigs: Rigs::new(),
            available: rigs::can_run() && rigs::module("libpipewire-module-filter-chain").is_some(),
            on_pipewire: std::cell::Cell::new(false),
            on_change: Box::new(on_change),
            on_turn: RefCell::default(),
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

    pub fn on_turn(&self, heard: impl Fn(&str, bool) + 'static) {
        *self.on_turn.borrow_mut() = Some(Box::new(heard));
    }

    /// Sets the equalizer of the output named `device`. Bands change as it plays; turning it on
    /// or off makes it or takes it away.
    pub fn set(self: &Rc<Self>, device: &str, value: &Value) {
        if device.is_empty() || device.starts_with(PREFIX) {
            return;
        }
        let new = Equalizer::from_value(value);
        let old = self
            .equalizers
            .borrow()
            .get(device)
            .cloned()
            .unwrap_or_default();
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
                self.rigs
                    .remember(&rig_id(device), &config(device, &self.title(device), &new));
                self.publish();
            }
            (false, true) => {
                self.apply();
                // What plays there goes through it once it's there.
                let (this, device) = (Rc::downgrade(self), device.to_owned());
                glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                    if let Some(this) = this.upgrade() {
                        if let Some(heard) = this.on_turn.borrow().as_ref() {
                            heard(&device, true);
                        }
                    }
                });
            }
            (true, false) => {
                // What plays through it goes back to the output before it goes.
                if let Some(heard) = self.on_turn.borrow().as_ref() {
                    heard(device, false);
                }
                let this = Rc::downgrade(self);
                glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                    if let Some(this) = this.upgrade() {
                        this.apply();
                    }
                });
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
                let config = config(device, &self.title(device), equalizer);
                self.rigs.set(&rig_id(device), Some(&config));
            }
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
        let config = config("alsa_output.a", "Line Out", &equalizer);
        assert_eq!(config.matches("label = bq_peaking").count(), 10);
        assert!(config.contains("name = band0 label = bq_peaking control = { \"Freq\" = 31.0 \"Q\" = 1.41 \"Gain\" = 6.0 }"));
        assert!(config.contains("\"Gain\" = -3.0"));
        assert!(config.contains("{ output = \"band8:Out\" input = \"band9:In\" }"));
        assert!(config.contains("node.name = \"bento.eq.alsa_output.a\""));
        assert!(config.contains("target.object = \"alsa_output.a\""));
        assert!(config.contains("node.description = \"Line Out (Equalizer)\""));
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
