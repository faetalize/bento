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

/// The rigs, by their ids: what each is wanted to run as, and the process of each that runs.
/// One that ended by itself starts again as it's wanted then, not as it first started, and not
/// at all once it's stopped.
pub struct Rigs {
    wanted: RefCell<HashMap<String, String>>,
    running: RefCell<HashMap<String, Child>>,
    dir: PathBuf,
    /// What runs them: `pipewire`, or a stand-in in tests.
    program: String,
    /// How long to wait before starting one again that ended by itself.
    retry: Duration,
}

impl Rigs {
    pub fn new() -> Rc<Self> {
        Self::with(
            glib::user_runtime_dir().join("bento-shell"),
            "pipewire",
            RETRY,
        )
    }

    /// Rigs whose configs are in `dir`, run by `program`, starting again after `retry`.
    fn with(dir: PathBuf, program: &str, retry: Duration) -> Rc<Self> {
        let _ = std::fs::create_dir_all(&dir);
        Rc::new(Self {
            wanted: RefCell::default(),
            running: RefCell::default(),
            dir,
            program: program.to_owned(),
            retry,
        })
    }

    /// Runs the rig `id` with `config`, or stops it with none. One that runs with the same
    /// config is left as it is.
    pub fn set(self: &Rc<Self>, id: &str, config: Option<&str>) {
        let same = self.wanted.borrow().get(id).map(String::as_str) == config
            && self.running.borrow().contains_key(id);
        if same {
            return;
        }
        self.stop(id);
        match config {
            Some(config) => {
                self.wanted
                    .borrow_mut()
                    .insert(id.to_owned(), config.to_owned());
                self.start(id);
            }
            None => {
                self.wanted.borrow_mut().remove(id);
            }
        }
    }

    /// Notes that the rig `id` runs as `config` says now, as when what changed was set on it as
    /// it runs.
    pub fn remember(&self, id: &str, config: &str) {
        if let Some(wanted) = self.wanted.borrow_mut().get_mut(id) {
            *wanted = config.to_owned();
            let _ = std::fs::write(self.path(id), config);
        }
    }

    /// The ids of the rigs that are wanted, running or about to start again.
    pub fn ids(&self) -> Vec<String> {
        self.wanted.borrow().keys().cloned().collect()
    }

    fn stop(&self, id: &str) {
        if let Some(mut child) = self.running.borrow_mut().remove(id) {
            let _ = child.kill();
            // glib's child watch reaps it; waiting here too makes glib warn.
        }
        let _ = std::fs::remove_file(self.path(id));
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.conf"))
    }

    /// Starts the rig `id` as it's wanted.
    fn start(self: &Rc<Self>, id: &str) {
        let Some(config) = self.wanted.borrow().get(id).cloned() else {
            return;
        };
        let path = self.path(id);
        if std::fs::write(&path, &config).is_err() {
            return;
        }
        let mut command = Command::new(&self.program);
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
        self.running.borrow_mut().insert(id.to_owned(), child);
        let weak: Weak<Self> = Rc::downgrade(self);
        let id = id.to_owned();
        glib::child_watch_add_local(pid, move |pid, _| {
            let Some(this) = weak.upgrade() else { return };
            // Only the one that ended by itself, not one stopped or set again since.
            let ended = this
                .running
                .borrow()
                .get(&id)
                .is_some_and(|child| child.id() as i32 == pid.0);
            if !ended {
                return;
            }
            // glib has reaped it already.
            this.running.borrow_mut().remove(&id);
            eprintln!("bento-shell --data: pipewire for {id} ended; starting it again");
            let weak = Rc::downgrade(&this);
            let id = id.clone();
            glib::timeout_add_local_once(this.retry, move || {
                if let Some(this) = weak.upgrade() {
                    if !this.running.borrow().contains_key(&id) {
                        this.start(&id);
                    }
                }
            });
        });
    }
}

impl Drop for Rigs {
    fn drop(&mut self) {
        for (_, mut child) in self.running.borrow_mut().drain() {
            let _ = child.kill();
            let _ = child.wait();
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

    /// Runs `context` until `done` or `within` passed.
    fn run_until(context: &glib::MainContext, within: Duration, done: impl Fn() -> bool) {
        let end = std::time::Instant::now() + within;
        while !done() && std::time::Instant::now() < end {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Rigs run by `false`, which ends at once, as a rig that keeps failing does, starting
    /// again after 50 ms, in a directory of their own.
    fn failing(dir: &Path) -> Rc<Rigs> {
        Rigs::with(dir.to_owned(), "false", Duration::from_millis(50))
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bento-rigs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // One test, since glib watches children from the default main context, which only one
    // thread at a time can run.
    #[test]
    fn a_rig_that_ends_starts_again_as_last_set_and_not_once_stopped() {
        let context = glib::MainContext::default();
        let _running = context.acquire().unwrap();

        let dir = scratch("remember");
        let rigs = failing(&dir);
        rigs.set("eq", Some("flat"));
        // A band changed as it ran.
        rigs.remember("eq", "bass");
        // It ends, and starts again: with the bass it had, not the flat it started with.
        run_until(&context, Duration::from_millis(400), || false);
        let file = dir.join("eq.conf");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "bass");
        rigs.set("eq", None);

        let dir = scratch("stop");
        let rigs = failing(&dir);
        rigs.set("combine", Some("pair"));
        // It has ended, and waits to start again.
        run_until(&context, Duration::from_millis(30), || false);
        rigs.set("combine", None);
        run_until(&context, Duration::from_millis(300), || false);
        assert!(!dir.join("combine.conf").exists(), "started again");
        assert!(rigs.ids().is_empty());
    }

    #[test]
    fn names_are_quoted_for_the_config() {
        assert_eq!(quoted("Speakers + Desk"), "\"Speakers + Desk\"");
        assert_eq!(quoted("a \"b\" \\ c\n"), "\"a \\\"b\\\" \\\\ c\"");
    }
}
