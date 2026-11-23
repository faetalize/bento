//! Which app streams are kept on a device of their own, as PipeWire's "default" metadata says,
//! and making one follow the default again or keeping it where it is.
//!
//! Moving an app to a device leaves a `target.object` for its stream in that metadata, which the
//! session manager (WirePlumber) remembers for the app until it's moved again. A target of `-1`
//! is none: the stream follows the default, and the session manager forgets what it remembered.
//! The metadata is read and written with `pw-metadata`, which comes with PipeWire; without it,
//! or on PulseAudio itself, nothing is known to be kept, and streams can't be made to follow.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gio::prelude::*;

/// How long to wait before watching again after `pw-metadata` ended, as when PipeWire restarts.
const RETRY: Duration = Duration::from_secs(2);

const TARGET_OBJECT: &str = "target.object";
const TARGET_NODE: &str = "target.node";

/// The targets that PipeWire's "default" metadata holds, by the stream's node id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Targets {
    by_node: HashMap<u32, Target>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Target {
    object: Option<String>,
    node: Option<String>,
}

impl Targets {
    /// Whether the stream whose node is `node` is kept on a device, rather than following the
    /// default. `target.object` is what's read first; the older `target.node` only without it.
    pub fn kept(&self, node: u32) -> bool {
        let Some(target) = self.by_node.get(&node) else {
            return false;
        };
        match &target.object {
            Some(object) => object != "-1",
            None => target.node.as_deref().is_some_and(|node| node != "-1"),
        }
    }

    /// Takes in a line that `pw-metadata --monitor` printed. Gives whether it changed anything.
    fn apply(&mut self, line: &str) -> bool {
        let Some(change) = parse(line) else {
            return false;
        };
        let before = self.clone();
        match change {
            Change::Set { id, key, value } => {
                let target = self.by_node.entry(id).or_default();
                match key.as_str() {
                    TARGET_OBJECT => target.object = Some(value),
                    TARGET_NODE => target.node = Some(value),
                    _ => (),
                }
            }
            Change::Remove { id, key: None } => {
                self.by_node.remove(&id);
            }
            Change::Remove { id, key: Some(key) } => {
                if let Some(target) = self.by_node.get_mut(&id) {
                    match key.as_str() {
                        TARGET_OBJECT => target.object = None,
                        TARGET_NODE => target.node = None,
                        _ => (),
                    }
                }
            }
        }
        self.by_node
            .retain(|_, target| target.object.is_some() || target.node.is_some());
        *self != before
    }
}

#[derive(Debug, PartialEq)]
enum Change {
    Set {
        id: u32,
        key: String,
        value: String,
    },
    /// A key taken away, or every key of the subject without one.
    Remove {
        id: u32,
        key: Option<String>,
    },
}

/// Reads a line of `pw-metadata --monitor`, like `update: id:85 key:'target.object'
/// value:'52' type:'Spa:Id'` or `remove: id:85 key:'target.object'`, for the keys of targets.
fn parse(line: &str) -> Option<Change> {
    let (what, rest) = line.trim().split_once(": ")?;
    let id = rest
        .strip_prefix("id:")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let quoted = |name: &str| -> Option<String> {
        let start = rest.find(&format!("{name}:'"))? + name.len() + 2;
        let end = rest[start..].find('\'')? + start;
        Some(rest[start..end].to_owned())
    };
    let key = quoted("key").filter(|key| key != "(null)");
    if key
        .as_deref()
        .is_some_and(|key| key != TARGET_OBJECT && key != TARGET_NODE)
    {
        return None;
    }
    match what {
        "update" => Some(Change::Set {
            id,
            key: key?,
            value: quoted("value")?,
        }),
        "remove" => Some(Change::Remove { id, key }),
        _ => None,
    }
}

/// Watches the targets with `pw-metadata --monitor`, starting it again when it ends.
pub struct TargetWatch {
    targets: RefCell<Targets>,
    /// Whether `pw-metadata` runs, so targets can be read and written.
    available: Cell<bool>,
    process: RefCell<Option<gio::Subprocess>>,
    on_change: Box<dyn Fn()>,
}

impl TargetWatch {
    pub fn start(on_change: impl Fn() + 'static) -> Rc<Self> {
        let this = Rc::new(Self {
            targets: RefCell::default(),
            available: Cell::new(false),
            process: RefCell::default(),
            on_change: Box::new(on_change),
        });
        this.watch();
        this
    }

    pub fn targets(&self) -> Targets {
        self.targets.borrow().clone()
    }

    pub fn available(&self) -> bool {
        self.available.get()
    }

    /// Makes the stream whose node is `node` follow the default output or input.
    pub fn follow_default(&self, node: u32) {
        self.set(node, "-1");
    }

    /// Keeps the stream whose node is `node` on the device whose `object.serial` is `serial`.
    pub fn keep_on(&self, node: u32, serial: &str) {
        self.set(node, serial);
    }

    fn set(&self, node: u32, value: &str) {
        if !self.available.get() {
            return;
        }
        // `--` so that a value of -1 isn't read as an option.
        let args = [
            "pw-metadata",
            "--",
            &node.to_string(),
            TARGET_OBJECT,
            value,
            "Spa:Id",
        ];
        let args: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
        let flags = gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE;
        if let Ok(process) = gio::Subprocess::newv(&args, flags) {
            process.wait_async(gio::Cancellable::NONE, |_| ());
        }
    }

    fn watch(self: &Rc<Self>) {
        let args = ["pw-metadata", "--monitor", "--name", "default"].map(std::ffi::OsStr::new);
        let flags = gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE;
        let process = match gio::Subprocess::newv(&args, flags) {
            Ok(process) => process,
            // Without pw-metadata there's nothing to watch, now or later.
            Err(_) => return,
        };
        let Some(stdout) = process.stdout_pipe() else {
            return;
        };
        *self.process.borrow_mut() = Some(process);
        let lines = gio::DataInputStream::new(&stdout);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let mut first = true;
            loop {
                let line = lines.read_line_utf8_future(glib::Priority::DEFAULT).await;
                let Some(this) = weak.upgrade() else { return };
                let Ok(Some(line)) = line else { break };
                if first {
                    first = false;
                    this.available.set(true);
                    (this.on_change)();
                }
                if this.targets.borrow_mut().apply(&line) {
                    (this.on_change)();
                }
            }
            let Some(this) = weak.upgrade() else { return };
            this.ended();
        });
    }

    /// `pw-metadata` ended: what it said is stale, and it's watched again in a moment.
    fn ended(self: &Rc<Self>) {
        self.process.borrow_mut().take();
        let was = self.available.replace(false);
        *self.targets.borrow_mut() = Targets::default();
        if was {
            (self.on_change)();
        }
        let weak: Weak<Self> = Rc::downgrade(self);
        glib::timeout_add_local_once(RETRY, move || {
            if let Some(this) = weak.upgrade() {
                this.watch();
            }
        });
    }
}

impl Drop for TargetWatch {
    fn drop(&mut self) {
        if let Some(process) = self.process.borrow_mut().take() {
            process.force_exit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_monitor_says_targets_set_and_taken_away() {
        assert_eq!(
            parse("update: id:158 key:'target.object' value:'5302' type:'Spa:Id'"),
            Some(Change::Set {
                id: 158,
                key: TARGET_OBJECT.into(),
                value: "5302".into()
            })
        );
        assert_eq!(
            parse("remove: id:158 key:'target.node'"),
            Some(Change::Remove {
                id: 158,
                key: Some(TARGET_NODE.into())
            })
        );
        assert_eq!(
            parse("remove: id:158 key:'(null)'"),
            Some(Change::Remove { id: 158, key: None })
        );
        assert_eq!(
            parse("update: id:0 key:'default.audio.sink' value:'{}' type:'Spa:String:JSON'"),
            None,
            "only targets"
        );
        assert_eq!(parse("Found \"default\" metadata 40"), None);
    }

    #[test]
    fn a_stream_is_kept_by_its_target_and_follows_with_none_or_minus_one() {
        let mut targets = Targets::default();
        assert!(!targets.kept(158));
        targets.apply("update: id:158 key:'target.node' value:'128' type:'Spa:Id'");
        assert!(targets.kept(158), "the older key alone");
        targets.apply("update: id:158 key:'target.object' value:'-1' type:'Spa:Id'");
        assert!(
            !targets.kept(158),
            "target.object comes first, and -1 is none"
        );
        targets.apply("update: id:158 key:'target.object' value:'5302' type:'Spa:Id'");
        assert!(targets.kept(158));
        assert!(targets.apply("remove: id:158 key:'(null)'"));
        assert!(!targets.kept(158));
        assert!(
            !targets.apply("remove: id:158 key:'(null)'"),
            "nothing left to change"
        );
    }
}
