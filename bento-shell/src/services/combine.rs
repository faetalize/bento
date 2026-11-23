//! Combinations of outputs: one output that plays on several, as PipeWire's combine-stream module
//! makes it, each of its outputs with a role, its level and its delay. They're kept in
//! `$XDG_STATE_HOME/bento/combinations.json`, and each runs as a rig of its own.
//!
//! Their own streams can't be moved, so an app that moves every stream to its own sink, like
//! EasyEffects, leaves them going where they're made to.
//!
//! A role picks what an output plays: the same as the rest, the front or the back of four
//! channels, or the lows or the highs, split by a crossover. With PipeWire's filter-chain, each
//! output plays through a filter made for it, with its level and its delay, which change as it
//! plays; a change of roles, or of keeping in sync, starts the combination again.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};

use super::rigs::{self, Rigs};

/// What the combined output's node is named, after `bento.combine.`, by its id.
pub const PREFIX: &str = "bento.combine.";
/// What the filter-chain sinks made for its outputs are named, after their own prefix.
pub const CHAIN_PREFIX: &str = "bento.chain.";
/// Where the lows and the highs are split, in Hz.
const CROSSOVER: f64 = 200.;
/// The longest delay an output can have, in milliseconds.
const MAX_DELAY: f64 = 500.;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Same,
    Front,
    Back,
    Lows,
    Highs,
}

impl Role {
    fn parse(text: &str) -> Self {
        match text {
            "front" => Self::Front,
            "back" => Self::Back,
            "lows" => Self::Lows,
            "highs" => Self::Highs,
            _ => Self::Same,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::Front => "front",
            Self::Back => "back",
            Self::Lows => "lows",
            Self::Highs => "highs",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    /// The output's name in the sound server.
    pub device: String,
    pub role: Role,
    /// Its share of the combination's sound, from 0 to 1: the balance between the outputs.
    pub level: f64,
    /// In milliseconds, when it isn't kept in sync by itself.
    pub delay: f64,
    /// The output's own volume before the combination set it to full, to put back when it's
    /// removed.
    pub restore: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Combination {
    pub id: String,
    pub name: String,
    /// Whether the quicker outputs wait for the slowest by themselves.
    pub auto: bool,
    pub members: Vec<Member>,
}

impl Combination {
    /// Reads one from what the shell or the file says, keeping ids to letters, digits and `-`
    /// for node names.
    pub fn from_value(value: &Value) -> Option<Self> {
        let id = value["id"].as_str()?;
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return None;
        }
        let members = value["members"]
            .as_array()?
            .iter()
            .filter_map(|member| {
                Some(Member {
                    device: member["device"]
                        .as_str()
                        .filter(|d| !d.is_empty())?
                        .to_owned(),
                    role: Role::parse(member["role"].as_str().unwrap_or_default()),
                    level: member["level"].as_f64().unwrap_or(1.).clamp(0., 1.),
                    restore: member["restore"].as_f64(),
                    delay: member["delay"].as_f64().unwrap_or(0.).clamp(0., MAX_DELAY),
                })
            })
            .collect();
        Some(Self {
            id: id.to_owned(),
            name: value["name"]
                .as_str()
                .unwrap_or("Combined Output")
                .to_owned(),
            auto: value["auto"].as_bool().unwrap_or(true),
            members,
        })
    }

    pub fn to_value(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "device": format!("{PREFIX}{}", self.id),
            "auto": self.auto,
            "members": self.members.iter().map(|member| json!({
                "device": member.device,
                "role": member.role.name(),
                "level": member.level,
                "restore": member.restore,
                "delay": member.delay,
            })).collect::<Vec<_>>(),
        })
    }
}

/// The config of the rig that makes `combination`. `compensates` says whether PipeWire's
/// combine-stream keeps outputs in sync by itself; without, delays are what keep them. With
/// `filters`, each output plays through a filter of its own; without, there are no crossovers,
/// levels or delays.
pub fn config(combination: &Combination, compensates: bool, filters: bool) -> String {
    let id = &combination.id;
    let auto = combination.auto && compensates;
    let four = combination
        .members
        .iter()
        .any(|member| matches!(member.role, Role::Front | Role::Back));
    let position = if four { "[ FL FR RL RR ]" } else { "[ FL FR ]" };
    let mut out = String::from(rigs::HEADER);
    let mut rules = Vec::new();
    for (i, member) in combination.members.iter().enumerate() {
        let channels = match member.role {
            Role::Back => "[ RL RR ]",
            _ => "[ FL FR ]",
        };
        let delay = if auto { 0. } else { member.delay };
        let mut nodes = Vec::new();
        match member.role {
            Role::Lows | Role::Highs if filters => {
                let label = if member.role == Role::Lows {
                    "bq_lowpass"
                } else {
                    "bq_highpass"
                };
                // Two in a row make a Linkwitz-Riley crossover, whose halves add back up.
                for n in 0..2 {
                    nodes.push(format!(
                        "{{ type = builtin name = cross{n} label = {label} control = {{ \"Freq\" = {CROSSOVER:.1} \"Q\" = 0.707 }} }}"
                    ));
                }
            }
            _ => (),
        }
        // Always there, so a level or a delay is set as it plays.
        if filters {
            nodes.push(format!(
                "{{ type = builtin name = delay label = delay config = {{ \"max-delay\" = {:.3} }} control = {{ \"Delay (s)\" = {:.4} }} }}",
                MAX_DELAY / 1000.,
                delay / 1000.
            ));
            nodes.push(format!(
                "{{ type = builtin name = level label = linear control = {{ \"Mult\" = {:.3} \"Add\" = 0.0 }} }}",
                member.level
            ));
        }
        let target = if nodes.is_empty() {
            member.device.clone()
        } else {
            let names: Vec<&str> = nodes
                .iter()
                .map(|node| {
                    let start = node.find("name = ").unwrap() + 7;
                    let end = node[start..].find(' ').unwrap() + start;
                    &node[start..end]
                })
                .collect();
            let links: Vec<String> = names
                .windows(2)
                .map(|pair| {
                    format!(
                        "{{ output = \"{}:Out\" input = \"{}:In\" }}",
                        pair[0], pair[1]
                    )
                })
                .collect();
            let chain = format!("{CHAIN_PREFIX}{id}.{i}");
            out.push_str(&format!(
                r#"    {{ name = libpipewire-module-filter-chain
        args = {{
            node.description = {description}
            audio.channels = 2
            audio.position = [ FL FR ]
            filter.graph = {{
                nodes = [ {nodes} ]
                links = [ {links} ]
            }}
            capture.props = {{
                node.name = {chain}
                media.class = Audio/Sink
                priority.session = 0
                priority.driver = 0
            }}
            playback.props = {{
                node.name = {chain_out}
                target.object = {device}
                node.passive = true
                node.dont-move = true
                stream.dont-remix = true
                bento.internal = true
            }}
        }}
    }}
"#,
                description = rigs::quoted(&format!("{} {}", combination.name, i + 1)),
                nodes = nodes.join(" "),
                links = links.join(" "),
                chain = rigs::quoted(&chain),
                chain_out = rigs::quoted(&format!("{chain}.out")),
                device = rigs::quoted(&member.device),
            ));
            chain
        };
        rules.push(format!(
            "{{ matches = [ {{ node.name = {} }} ] actions = {{ create-stream = {{ combine.audio.position = {channels} audio.position = [ FL FR ] }} }} }}",
            rigs::quoted(&target)
        ));
    }
    out.push_str(&format!(
        r#"    {{ name = libpipewire-module-combine-stream
        args = {{
            combine.mode = sink
            node.name = {name}
            node.description = {description}
{compensate}            combine.props = {{
                audio.position = {position}
                priority.session = 0
            }}
            stream.props = {{
                node.dont-move = true
                stream.dont-remix = true
                bento.internal = true
            }}
            stream.rules = [
                {rules}
            ]
        }}
    }}
]
"#,
        name = rigs::quoted(&format!("{PREFIX}{id}")),
        description = rigs::quoted(&combination.name),
        compensate = if auto {
            "            combine.latency-compensate = true\n"
        } else {
            ""
        },
        rules = rules.join("\n                "),
    ));
    out
}

/// Keeps the combinations, and runs them.
pub struct CombineService {
    combinations: RefCell<Vec<Combination>>,
    rigs: Rc<Rigs>,
    /// Whether PipeWire's combine-stream is there to make them.
    available: bool,
    /// Whether it keeps outputs in sync by itself.
    compensates: bool,
    /// Whether filter-chain is there, for crossovers, levels and delays.
    filters: bool,
    on_change: Box<dyn Fn(&Value)>,
    /// Hears the name of a combined output that starts again, so apps kept on it go back.
    on_restart: RefCell<Option<Box<dyn Fn(&str)>>>,
}

impl CombineService {
    pub fn start(on_change: impl Fn(&Value) + 'static) -> Rc<Self> {
        let combine = rigs::module("libpipewire-module-combine-stream");
        let can_run = rigs::can_run();
        let this = Rc::new(Self {
            combinations: RefCell::new(load()),
            rigs: Rigs::new(),
            available: can_run && combine.is_some(),
            compensates: combine
                .as_deref()
                .is_some_and(|path| rigs::knows(path, "combine.latency-compensate")),
            filters: can_run && rigs::module("libpipewire-module-filter-chain").is_some(),
            on_change: Box::new(on_change),
            on_restart: RefCell::default(),
        });
        this.apply();
        this
    }

    /// Hears the name of each combined output that starts again for a change.
    pub fn on_restart(&self, heard: impl Fn(&str) + 'static) {
        *self.on_restart.borrow_mut() = Some(Box::new(heard));
    }

    /// Makes or changes a combination, by its id. A change of levels or delays alone is made
    /// as it plays; any other starts it again.
    pub fn set(&self, value: &Value) {
        let Some(combination) = Combination::from_value(value) else {
            return;
        };
        let old = self
            .combinations
            .borrow()
            .iter()
            .find(|c| c.id == combination.id)
            .cloned();
        {
            let mut combinations = self.combinations.borrow_mut();
            match combinations.iter_mut().find(|c| c.id == combination.id) {
                Some(old) => *old = combination.clone(),
                None => combinations.push(combination.clone()),
            }
        }
        let restarts = self.available && old.is_some();
        if let Some(old) = old.filter(|old| self.filters && shape(old) == shape(&combination)) {
            self.tune(&old, &combination);
            save(&self.combinations.borrow());
            self.rigs.remember(
                &rig_id(&combination.id),
                &config(&combination, self.compensates, self.filters),
            );
            self.publish();
            return;
        }
        self.changed();
        if restarts {
            if let Some(heard) = self.on_restart.borrow().as_ref() {
                heard(&format!("{PREFIX}{}", combination.id));
            }
        }
    }

    /// Sets the levels and delays that changed from `old` to `new` on their filters as they
    /// play.
    fn tune(&self, old: &Combination, new: &Combination) {
        let auto = new.auto && self.compensates;
        for (i, (was, is)) in old.members.iter().zip(&new.members).enumerate() {
            let mut params = Vec::new();
            if was.level != is.level {
                params.push(format!("\"level:Mult\" {:.3}", is.level));
            }
            if was.delay != is.delay && !auto {
                params.push(format!("\"delay:Delay (s)\" {:.4}", is.delay / 1000.));
            }
            if params.is_empty() {
                continue;
            }
            let node = format!("{CHAIN_PREFIX}{}.{i}", new.id);
            let props = format!("{{ params = [ {} ] }}", params.join(" "));
            let args = ["pw-cli", "set-param", &node, "Props", &props];
            let args: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
            let flags = gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE;
            if let Ok(process) = gio::Subprocess::newv(&args, flags) {
                process.wait_async(gio::Cancellable::NONE, |_| ());
            }
        }
    }

    /// The combination `id`, if there's one.
    pub fn get(&self, id: &str) -> Option<Combination> {
        self.combinations
            .borrow()
            .iter()
            .find(|c| c.id == id)
            .cloned()
    }

    pub fn remove(&self, id: &str) {
        self.combinations.borrow_mut().retain(|c| c.id != id);
        self.changed();
    }

    fn changed(&self) {
        save(&self.combinations.borrow());
        self.apply();
    }

    fn publish(&self) {
        let combinations = self.combinations.borrow();
        (self.on_change)(&json!({
            "type": "combinations",
            "available": self.available,
            "autoSync": self.compensates,
            "filters": self.filters,
            "combinations": combinations.iter().map(Combination::to_value).collect::<Vec<_>>(),
        }));
    }

    /// Runs what's kept, stops what isn't, and says so.
    fn apply(&self) {
        {
            let combinations = self.combinations.borrow();
            if self.available {
                let wanted: Vec<String> = combinations.iter().map(|c| rig_id(&c.id)).collect();
                for id in self.rigs.ids() {
                    if !wanted.contains(&id) {
                        self.rigs.set(&id, None);
                    }
                }
                for combination in combinations.iter() {
                    let mut combination = combination.clone();
                    if !self.filters {
                        // Without filter-chain, what needs one plays as it is.
                        for member in &mut combination.members {
                            if matches!(member.role, Role::Lows | Role::Highs) {
                                member.role = Role::Same;
                            }
                            member.level = 1.;
                            member.delay = 0.;
                        }
                    }
                    let config = config(&combination, self.compensates, self.filters);
                    self.rigs.set(&rig_id(&combination.id), Some(&config));
                }
            }
        }
        self.publish();
    }
}

/// What a change of starts a combination again: all but its outputs' levels and delays.
fn shape(combination: &Combination) -> (String, bool, Vec<(String, Role)>) {
    let members = combination.members.iter();
    (
        combination.name.clone(),
        combination.auto,
        members.map(|m| (m.device.clone(), m.role)).collect(),
    )
}

fn rig_id(id: &str) -> String {
    format!("combine-{id}")
}

fn file() -> PathBuf {
    glib::user_state_dir()
        .join("bento")
        .join("combinations.json")
}

fn load() -> Vec<Combination> {
    let Ok(text) = std::fs::read_to_string(file()) else {
        return Vec::new();
    };
    let value: Value = serde_json::from_str(&text).unwrap_or_default();
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Combination::from_value)
        .collect()
}

fn save(combinations: &[Combination]) {
    let path = file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let value = Value::Array(combinations.iter().map(Combination::to_value).collect());
    let _ = std::fs::write(
        path,
        serde_json::to_string_pretty(&value).unwrap_or_default(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(roles: [Role; 2], auto: bool) -> Combination {
        Combination {
            id: "c1".into(),
            name: "Speakers + Desk".into(),
            auto,
            members: vec![
                Member {
                    device: "alsa_output.a".into(),
                    role: roles[0],
                    level: 1.,
                    delay: 0.,
                    restore: None,
                },
                Member {
                    device: "alsa_output.b".into(),
                    role: roles[1],
                    level: 1.,
                    delay: 40.,
                    restore: Some(0.3),
                },
            ],
        }
    }

    #[test]
    fn the_same_sound_on_two_outputs_is_two_streams_of_one_sink() {
        let config = config(&pair([Role::Same, Role::Same], true), true, false);
        assert!(config.contains("node.name = \"bento.combine.c1\""));
        assert!(config.contains("node.description = \"Speakers + Desk\""));
        assert!(config.contains("combine.latency-compensate = true"));
        assert!(config.contains("audio.position = [ FL FR ]\n"));
        assert!(config.contains("node.name = \"alsa_output.a\""));
        assert!(config.contains("node.name = \"alsa_output.b\""));
        assert!(
            !config.contains("filter-chain"),
            "without filter-chain, played as it is"
        );
    }

    #[test]
    fn front_and_back_split_four_channels() {
        let config = config(&pair([Role::Front, Role::Back], true), true, true);
        assert!(config.contains("audio.position = [ FL FR RL RR ]"));
        assert!(config.contains("combine.audio.position = [ RL RR ]"));
    }

    #[test]
    fn lows_and_highs_and_delays_play_through_filters_made_for_them() {
        let mut combination = pair([Role::Lows, Role::Highs], false);
        combination.members[0].level = 0.5;
        let config = config(&combination, true, true);
        assert!(
            !config.contains("latency-compensate"),
            "the user's own delays"
        );
        assert_eq!(config.matches("libpipewire-module-filter-chain").count(), 2);
        assert_eq!(config.matches("label = bq_lowpass").count(), 2);
        assert_eq!(config.matches("label = bq_highpass").count(), 2);
        assert!(config.contains("\"Mult\" = 0.500"));
        assert!(config.contains("\"Delay (s)\" = 0.0400"));
        assert!(config.contains("{ output = \"cross0:Out\" input = \"cross1:In\" }"));
        assert!(config.contains("{ output = \"cross1:Out\" input = \"delay:In\" }"));
        assert!(config.contains("{ output = \"delay:Out\" input = \"level:In\" }"));
        // The combination's streams go to the filters, which play on the outputs.
        assert!(config.contains("node.name = \"bento.chain.c1.0\""));
        assert!(config.contains("target.object = \"alsa_output.a\""));
    }

    #[test]
    fn without_sync_of_its_own_delays_are_kept() {
        let config = config(&pair([Role::Same, Role::Same], true), false, true);
        assert!(!config.contains("latency-compensate"));
        assert!(
            config.contains("\"Delay (s)\" = 0.0400"),
            "the older PipeWire's way"
        );
    }

    #[test]
    fn levels_and_delays_change_as_it_plays_and_roles_start_it_again() {
        let old = pair([Role::Same, Role::Same], false);
        let mut louder = old.clone();
        louder.members[1].level = 0.4;
        louder.members[0].delay = 25.;
        assert_eq!(shape(&old), shape(&louder));
        let mut back = old.clone();
        back.members[1].role = Role::Back;
        assert_ne!(shape(&old), shape(&back));
        let mut synced = old.clone();
        synced.auto = true;
        assert_ne!(shape(&old), shape(&synced));
    }

    #[test]
    fn a_combination_is_read_back_as_written() {
        let combination = pair([Role::Front, Role::Highs], false);
        assert_eq!(
            Combination::from_value(&combination.to_value()),
            Some(combination)
        );
        assert_eq!(
            Combination::from_value(&json!({ "id": "a b", "members": [] })),
            None,
            "an id goes in node names"
        );
    }
}
