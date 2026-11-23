//! What bento builds in PipeWire out of its own modules, like a combination of outputs or an
//! equalizer: each a small PipeWire client of its own, `pipewire -c` with a config of that
//! module, as PipeWire's own filter-chain configs run. It runs while it's wanted, and starts again
//! when it ends by itself; and it ends with this program, which nothing else would end.

use std::cell::RefCell;
use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::{Rc, Weak};
use std::time::Duration;

/// How long to wait before starting one again that ended by itself.
const RETRY: Duration = Duration::from_secs(2);

/// Where PipeWire's modules are, on the distributions that are known.
const MODULE_DIRS: [&str; 5] = [
    "/usr/lib/pipewire-0.3",
    "/usr/lib64/pipewire-0.3",
    "/usr/lib/x86_64-linux-gnu/pipewire-0.3",
    "/usr/lib/aarch64-linux-gnu/pipewire-0.3",
    "/usr/local/lib/pipewire-0.3",
];

/// The file of PipeWire's module `name`, like `libpipewire-module-combine-stream`, when it's
/// installed. `PIPEWIRE_MODULE_DIR` is looked in first, as PipeWire does.
pub fn module(name: &str) -> Option<PathBuf> {
    let file = format!("{name}.so");
    let from_env = std::env::var_os("PIPEWIRE_MODULE_DIR").map(PathBuf::from);
    from_env
        .into_iter()
        .chain(MODULE_DIRS.iter().map(PathBuf::from))
        .map(|dir| dir.join(&file))
        .find(|path| path.is_file())
}

/// Whether the module at `path` knows the option `option`: one a newer PipeWire added is among
/// its strings, as an older one's isn't.
pub fn knows(path: &Path, option: &str) -> bool {
    std::fs::read(path).is_ok_and(|bytes| {
        bytes
            .windows(option.len())
            .any(|window| window == option.as_bytes())
    })
}

/// Whether `pipewire` itself is there to run them with.
pub fn can_run() -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("pipewire").is_file()))
}

struct Running {
    config: String,
    child: Child,
}

/// The rigs that run, by their ids.
pub struct Rigs {
    running: RefCell<HashMap<String, Running>>,
    dir: PathBuf,
}

impl Rigs {
    pub fn new() -> Rc<Self> {
        let dir = glib::user_runtime_dir().join("bento-shell");
        let _ = std::fs::create_dir_all(&dir);
        Rc::new(Self {
            running: RefCell::default(),
            dir,
        })
    }

    /// Runs the rig `id` with `config`, or stops it with none. One that runs with the same
    /// config is left as it is.
    pub fn set(self: &Rc<Self>, id: &str, config: Option<&str>) {
        let same = self
            .running
            .borrow()
            .get(id)
            .is_some_and(|running| Some(running.config.as_str()) == config);
        if same {
            return;
        }
        self.stop(id);
        if let Some(config) = config {
            self.start(id, config);
        }
    }

    /// Notes that the rig `id` runs as `config` says now, as when what changed was set on it as
    /// it runs.
    pub fn remember(&self, id: &str, config: &str) {
        if let Some(running) = self.running.borrow_mut().get_mut(id) {
            running.config = config.to_owned();
        }
        let _ = std::fs::write(self.path(id), config);
    }

    /// The ids of the rigs that run.
    pub fn ids(&self) -> Vec<String> {
        self.running.borrow().keys().cloned().collect()
    }

    fn stop(&self, id: &str) {
        if let Some(mut running) = self.running.borrow_mut().remove(id) {
            let _ = running.child.kill();
            // glib's child watch reaps it; waiting here too makes glib warn.
        }
        let _ = std::fs::remove_file(self.path(id));
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.conf"))
    }

    fn start(self: &Rc<Self>, id: &str, config: &str) {
        let path = self.path(id);
        if std::fs::write(&path, config).is_err() {
            return;
        }
        let mut command = Command::new("pipewire");
        command
            .arg("-c")
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // It ends with this program, even killed, rather than playing on with nobody to stop it.
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let Ok(child) = command.spawn() else {
            eprintln!("bento-shell --data: couldn't run pipewire for {id}");
            return;
        };
        let pid = glib::Pid(child.id() as i32);
        self.running.borrow_mut().insert(
            id.to_owned(),
            Running {
                config: config.to_owned(),
                child,
            },
        );
        let weak: Weak<Self> = Rc::downgrade(self);
        let id = id.to_owned();
        let config = config.to_owned();
        glib::child_watch_add_local(pid, move |pid, _| {
            let Some(this) = weak.upgrade() else { return };
            // Only the one that ended by itself, not one stopped or set again since.
            let ended = this
                .running
                .borrow()
                .get(&id)
                .is_some_and(|running| running.child.id() as i32 == pid.0);
            if !ended {
                return;
            }
            // glib has reaped it already.
            this.running.borrow_mut().remove(&id);
            eprintln!("bento-shell --data: pipewire for {id} ended; starting it again");
            let weak = Rc::downgrade(&this);
            let (id, config) = (id.clone(), config.clone());
            glib::timeout_add_local_once(RETRY, move || {
                if let Some(this) = weak.upgrade() {
                    if !this.running.borrow().contains_key(&id) {
                        this.start(&id, &config);
                    }
                }
            });
        });
    }
}

impl Drop for Rigs {
    fn drop(&mut self) {
        for (_, mut running) in self.running.borrow_mut().drain() {
            let _ = running.child.kill();
            let _ = running.child.wait();
        }
    }
}

/// Writes `text` as a quoted string of PipeWire's config.
pub fn quoted(text: &str) -> String {
    let escaped: String = text
        .chars()
        .filter(|c| !c.is_control())
        .flat_map(|c| match c {
            '"' | '\\' => vec!['\\', c],
            c => vec![c],
        })
        .collect();
    format!("\"{escaped}\"")
}

/// The start of every rig's config: a client of the running PipeWire, with what its modules
/// need.
pub const HEADER: &str = r#"context.properties = {
    log.level = 0
}
context.spa-libs = {
    audio.convert.* = audioconvert/libspa-audioconvert
    support.* = support/libspa-support
}
context.modules = [
    { name = libpipewire-module-rt args = { } flags = [ ifexists nofail ] }
    { name = libpipewire-module-protocol-native }
    { name = libpipewire-module-client-node }
    { name = libpipewire-module-adapter }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_quoted_for_the_config() {
        assert_eq!(quoted("Speakers + Desk"), "\"Speakers + Desk\"");
        assert_eq!(quoted("a \"b\" \\ c\n"), "\"a \\\"b\\\" \\\\ c\"");
    }
}
