//! The shell's settings and actions: what its modules and the shell itself declare in their
//! `meta`, as engines do. The user sets the settings, and binds the actions, in the shell's file,
//! a block for each by its name: bento's own shells share `~/.config/bento/shell.kdl`, and a shell
//! file's is the one the config names next to it.
//!
//! Settings are declared as engines' are, and are checked and converted the same way. A module
//! reads its settings as they are, and gets them again when the file changes, without the shell
//! loading again. A module can write its own, as when an app is pinned. Settings that nothing
//! declares stay in the file: bento says so rather than failing, so that switching shells loses
//! nothing.
//!
//! An action can say the key it's bound to by default. The binds are bento's while the module
//! runs: they take keys over from the main config, as an engine's file does, and a bind of the
//! action's own in its block replaces its default.

use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{anyhow, bail, Context as _};
use niri_config::binds::{Bind, Key};
use niri_config::edit::{option_properties, option_values, Document};
use niri_config::{bind_set, EngineSettings, ShellSettings};
use niri_ipc::{LayoutEngineAction, LayoutEngineOption, LayoutEngineOptionValue};

use crate::layout::tiling::engine::{
    ipc_settings, is_native_action, parse_actions, resolve_settings, warn_unknown_page, SettingMeta,
};

/// The settings file, and what the running shell declares.
#[derive(Debug, Default)]
pub struct Settings {
    /// The file, or none: in tests, which keep the settings in memory alone, and for a shell file
    /// the config names no file for, which runs on its defaults.
    path: Option<PathBuf>,
    /// When the file last changed, as it was read.
    mtime: Option<SystemTime>,
    /// The file's text as it was read, to edit.
    text: String,
    /// The settings the file sets.
    file: ShellSettings,
    /// Why the file can't be read, while it can't.
    error: Option<String>,
    /// The settings each module and the shell declares, by its name, in the order they did.
    declared: Vec<(String, Vec<SettingMeta>)>,
    /// The actions each declares, with the name of the module or shell that does.
    actions: Vec<(String, LayoutEngineAction)>,
    /// The binds their actions have by default, with the name of the module or shell whose they
    /// are.
    binds: Vec<(String, Bind)>,
    /// What a test writes to the file, which reads as a change.
    #[cfg(test)]
    written: Option<String>,
}

impl Settings {
    /// The user's settings and binds, from the file at `path`.
    pub fn load(path: Option<PathBuf>) -> Self {
        let mut settings = Self {
            path: if cfg!(test) { None } else { path },
            ..Self::default()
        };
        settings.read();
        settings
    }

    /// Reads the file at `path` instead, as when another shell runs. Returns whether it's another
    /// file.
    pub fn set_path(&mut self, path: Option<PathBuf>) -> bool {
        let path = if cfg!(test) { None } else { path };
        if path == self.path {
            return false;
        }
        self.path = path;
        self.mtime = None;
        self.text.clear();
        self.file = ShellSettings::default();
        self.error = None;
        self.read();
        true
    }

    /// The user's binds for the shell's actions, from the file's blocks.
    pub fn user_binds(&self) -> Vec<Bind> {
        self.file
            .binds
            .iter()
            .map(|(_, bind)| bind.clone())
            .collect()
    }

    /// The shell's actions the user turned off in the file's blocks.
    pub fn unbound(&self) -> Vec<niri_config::Action> {
        self.file
            .unbound
            .iter()
            .map(|(_, action)| action.clone())
            .collect()
    }

    /// Settings with `text` as the file, which isn't on disk.
    #[cfg(test)]
    pub fn from_text(text: &str) -> Self {
        let mut settings = Self::default();
        settings.take_text(String::from(text));
        settings
    }

    /// Reads the file again if it changed since it was read. Returns whether it did.
    pub fn reload_if_changed(&mut self) -> bool {
        #[cfg(test)]
        if let Some(text) = self.written.take() {
            self.take_text(text);
            return true;
        }
        let Some(path) = &self.path else {
            return false;
        };
        let mtime = path.metadata().and_then(|m| m.modified()).ok();
        if mtime == self.mtime {
            return false;
        }
        self.read();
        true
    }

    fn read(&mut self) {
        let Some(path) = &self.path else { return };
        self.mtime = path.metadata().and_then(|m| m.modified()).ok();
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => {
                warn!("error reading the shell's settings: {err}");
                self.error = Some(err.to_string());
                return;
            }
        };
        self.take_text(text);
    }

    fn take_text(&mut self, text: String) {
        match ShellSettings::parse(&text) {
            Ok(file) => {
                self.file = file;
                self.text = text;
                self.error = None;
            }
            Err(err) => {
                // The last settings that read stay until the file is fixed.
                let report = format!("{:?}", miette::Report::new(err));
                warn!("error reading the shell's settings: {report}");
                self.error = Some(report);
            }
        }
    }

    /// Writes `text` as the file, as someone editing it does.
    #[cfg(test)]
    pub fn write_file(&mut self, text: &str) {
        self.written = Some(String::from(text));
    }

    /// Forgets what the shell declared, for a shell loading again to declare it anew.
    pub fn forget_declarations(&mut self) {
        self.declared.clear();
        self.actions.clear();
        self.binds.clear();
    }

    /// Takes what module or shell `name` declares, its settings and actions as the JSON arrays
    /// that `checkSettings` and `checkActions` give, and returns its settings' values, by name.
    pub fn declare(
        &mut self,
        name: &str,
        settings: &str,
        actions: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let declared: Vec<SettingMeta> = serde_json::from_str(settings)
            .with_context(|| format!("{name}'s settings are declared wrong"))?;
        for setting in &declared {
            warn_unknown_page(name, setting);
            if let Err(problem) = setting.convert(&setting.default_values()) {
                bail!(
                    "{name}'s default for its setting {:?} is wrong: it {problem}",
                    setting.name
                );
            }
        }
        let actions = parse_actions(actions)
            .with_context(|| format!("{name}'s actions are declared wrong"))?;
        for action in &actions {
            if is_native_action(&action.name) {
                bail!(
                    "{name} declares {:?}, which is one of bento's own actions",
                    action.name
                );
            }
            let theirs = self
                .actions
                .iter()
                .find(|(owner, a)| a.name == action.name && owner != name);
            if let Some((owner, _)) = theirs {
                bail!(
                    "{name} declares the action {:?}, which {owner} declares too",
                    action.name
                );
            }
        }
        let mut binds = Vec::new();
        for action in &actions {
            for bind in &action.binds {
                let text = &bind.key;
                let key: Key = text.parse().map_err(|err| {
                    anyhow!(
                        "{name} binds its action {:?} to {text:?}, which isn't a key: {err}",
                        action.name
                    )
                })?;
                let theirs = self
                    .binds
                    .iter()
                    .find(|(owner, bind)| owner != name && bind.key.conflicts_with(&key));
                if let Some((owner, _)) = theirs {
                    bail!(
                        "{name} binds {text} to its action {:?}, which {owner} binds too",
                        action.name
                    );
                }
                let runs = bind_set::shell_action(&action.name, &bind.args);
                binds.push((String::from(name), bind_set::default_bind(key, runs)));
            }
        }
        match self.declared.iter_mut().find(|(n, _)| n == name) {
            Some((_, settings)) => *settings = declared,
            None => self.declared.push((String::from(name), declared)),
        }
        self.actions.retain(|(owner, _)| owner != name);
        let owned = actions
            .into_iter()
            .map(|action| (String::from(name), action));
        self.actions.extend(owned);
        self.binds.retain(|(owner, _)| owner != name);
        self.binds.extend(binds);
        Ok(self.values(name))
    }

    /// The binds the running shell's actions have by default.
    pub fn binds(&self) -> Vec<Bind> {
        self.binds.iter().map(|(_, bind)| bind.clone()).collect()
    }

    /// The actions the running shell declares, in the order they're declared, each with the
    /// module or shell that declares it.
    pub fn owned_actions(&self) -> Vec<(String, LayoutEngineAction)> {
        self.actions.clone()
    }

    /// The actions the running shell declares, in the order they're declared.
    pub fn actions(&self) -> Vec<LayoutEngineAction> {
        self.actions
            .iter()
            .map(|(_, action)| action.clone())
            .collect()
    }

    /// The values of `name`'s settings, by name: what the file sets, if it fits, or the default.
    pub fn values(&self, name: &str) -> serde_json::Value {
        let Some((_, declared)) = self.declared.iter().find(|(n, _)| n == name) else {
            return serde_json::json!({});
        };
        resolve_settings(declared, &[&self.file.of(name)])
    }

    /// The values of every module's and shell's settings, by its name.
    pub fn all_values(&self) -> serde_json::Value {
        let all = self
            .declared
            .iter()
            .map(|(name, _)| (name.clone(), self.values(name)));
        serde_json::Value::Object(all.collect())
    }

    /// Writes `name`'s setting `setting` to the file: `value` as it's written, or the default
    /// when it's the default or `null`, which stays out of the file.
    pub fn set(&mut self, name: &str, setting: &str, value: &str) -> anyhow::Result<()> {
        let declared = self
            .declared
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, declared)| declared.iter().find(|s| s.name == setting))
            .ok_or_else(|| anyhow!("{name} declares no setting {setting:?}"))?;
        let value: Option<LayoutEngineOptionValue> =
            serde_json::from_str(value).context("the value isn't JSON")?;

        // Check it the way the file is read.
        let mut doc = Document::parse(format!("{name} {{\n}}\n"))?;
        let path = [name, setting];
        doc.set_node(&path, option_values(&value), option_properties(&value));
        let check = ShellSettings::parse(doc.text())?;
        let written = check
            .of(name)
            .pop()
            .map(|(_, values)| values)
            .unwrap_or_default();
        if let Err(problem) = declared.convert(&written) {
            bail!("{name}'s setting {setting:?} {problem}");
        }

        let mut doc = Document::parse(self.text.clone())
            .map_err(|_| anyhow!("the settings file has an error, so it's left as it is"))?;
        if value.is_none() || value == declared.default {
            doc.remove(&path);
        } else {
            doc.set_node(&path, option_values(&value), option_properties(&value));
        }
        let text = String::from(doc.text());
        if self.path.is_none() && !cfg!(test) {
            bail!("the shell has no settings file, so its settings can't change");
        }
        if let Some(file) = &self.path {
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(file, &text)
                .with_context(|| format!("error writing {}", file.display()))?;
            self.mtime = file.metadata().and_then(|m| m.modified()).ok();
        }
        self.take_text(text);
        Ok(())
    }

    /// What the file sets that nothing running declares, as `name` for a whole block or
    /// `name.setting`, and the settings it sets to what doesn't fit, with why.
    pub fn problems(&self) -> (Vec<String>, Vec<String>) {
        let mut unused = Vec::new();
        let mut wrong = Vec::new();
        for (name, settings) in &self.file.blocks {
            let Some((_, declared)) = self.declared.iter().find(|(n, _)| n == name) else {
                if !unused.contains(name) {
                    unused.push(name.clone());
                }
                continue;
            };
            for (setting, values) in settings {
                match declared.iter().find(|s| s.name == *setting) {
                    None => unused.push(format!("{name}.{setting}")),
                    Some(meta) => {
                        if let Err(problem) = meta.convert(values) {
                            wrong.push(format!("{name}.{setting} {problem}"));
                        }
                    }
                }
            }
        }
        (unused, wrong)
    }

    /// Says in the log what the file sets that isn't used, and what doesn't fit.
    pub fn warn_problems(&self) {
        let (unused, wrong) = self.problems();
        if !unused.is_empty() {
            warn!(
                "the shell's settings file sets what the running shell doesn't declare, which \
                 stays in the file: {}",
                unused.join(", ")
            );
        }
        for problem in wrong {
            warn!("the shell's setting {problem}, so it's left at its default");
        }
    }

    /// Each module's and shell's settings with their values, for bento's settings.
    pub fn ipc(&self) -> Vec<(String, Vec<LayoutEngineOption>)> {
        let blocks = self
            .declared
            .iter()
            .filter(|(_, declared)| !declared.is_empty());
        let blocks = blocks.map(|(name, declared)| {
            let values: EngineSettings = self.file.of(name);
            (name.clone(), ipc_settings(declared, &values))
        });
        blocks.collect()
    }

    /// Why the file can't be read, if it can't.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const TASKBAR: &str = r#"[
        { "name": "pinned", "type": "string", "list": true, "default": [] },
        { "name": "size", "type": "number", "default": 40 }
    ]"#;

    #[test]
    fn a_module_gets_what_the_file_sets_or_its_defaults() {
        let mut settings = Settings::from_text("taskbar {\n    pinned \"firefox\" \"foot\"\n}\n");
        let values = settings.declare("taskbar", TASKBAR, "[]").unwrap();
        assert_eq!(
            values,
            json!({ "pinned": ["firefox", "foot"], "size": 40.0 })
        );
    }

    #[test]
    fn a_setting_that_does_not_fit_is_its_default_and_said_so() {
        let mut settings =
            Settings::from_text("taskbar {\n    size \"big\"\n    gone 1\n}\nold {\n}\n");
        let values = settings.declare("taskbar", TASKBAR, "[]").unwrap();
        assert_eq!(values["size"], json!(40.0));
        let (unused, wrong) = settings.problems();
        assert_eq!(unused, ["taskbar.gone", "old"]);
        assert_eq!(wrong, ["taskbar.size must be a number"]);
    }

    #[test]
    fn a_module_writes_its_settings_into_the_file_and_leaves_the_rest() {
        let text = "// Mine.\nclock {\n    seconds true\n}\n";
        let mut settings = Settings::from_text(text);
        settings.declare("taskbar", TASKBAR, "[]").unwrap();
        settings.set("taskbar", "pinned", r#"["firefox"]"#).unwrap();
        assert_eq!(settings.values("taskbar")["pinned"], json!(["firefox"]));
        assert!(settings.text.starts_with(text));

        // The default stays out of the file.
        settings.set("taskbar", "pinned", "[]").unwrap();
        assert!(!settings.text.contains("pinned"));

        let wrong = settings.set("taskbar", "size", r#""big""#);
        assert!(wrong.is_err());
        assert!(settings.set("taskbar", "nothing", "1").is_err());
    }

    #[test]
    fn modules_declare_actions_that_no_other_module_or_bento_has() {
        let mut settings = Settings::from_text("");
        let switch = r#"[{ "name": "switch-audio-output", "description": "Switch.", "args": [] }]"#;
        settings.declare("sound", "[]", switch).unwrap();
        let names: Vec<String> = settings.actions().into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["switch-audio-output"]);
        // Declaring again, as a shell loading again does, replaces its own.
        settings.declare("sound", "[]", switch).unwrap();
        assert_eq!(settings.actions().len(), 1);

        let err = settings.declare("mixer", "[]", switch).unwrap_err();
        assert!(format!("{err:#}").contains("sound declares too"), "{err:#}");
        let native = r#"[{ "name": "close-window", "args": [] }]"#;
        assert!(settings.declare("mixer", "[]", native).is_err());
    }

    #[test]
    fn an_action_can_have_a_key_by_default() {
        let mut settings = Settings::from_text("");
        let start = r#"[{ "name": "toggle-start-menu", "args": [], "binds": [{ "key": "Mod" }] }]"#;
        let clipboard =
            r#"[{ "name": "toggle-clipboard", "args": [], "binds": [{ "key": "Mod+V" }] }]"#;
        settings.declare("start-menu", "[]", start).unwrap();
        settings.declare("clipboard", "[]", clipboard).unwrap();
        let binds = settings.binds();
        use niri_config::binds::{Action, BoundAction};
        let shell = |name: &str| Action::ShellAction(String::from(name), Vec::new());
        // Mod alone runs it as it's let go, so Mod still works with other keys.
        assert_eq!(binds[0].key, "Mod".parse().unwrap());
        assert_eq!(
            binds[0].action,
            BoundAction::Release(shell("toggle-start-menu"))
        );
        assert_eq!(binds[1].key, "Mod+V".parse().unwrap());
        assert_eq!(
            binds[1].action,
            BoundAction::Press(shell("toggle-clipboard"))
        );

        // A key another module binds, or one that isn't a key, is a wrong declaration.
        let taken = r#"[{ "name": "toggle-launcher", "args": [], "binds": [{ "key": "Mod" }] }]"#;
        let err = settings.declare("launcher", "[]", taken).unwrap_err();
        assert!(
            format!("{err:#}").contains("start-menu binds too"),
            "{err:#}"
        );
        let wrong =
            r#"[{ "name": "toggle-launcher", "args": [], "binds": [{ "key": "Mod+Nothing" }] }]"#;
        assert!(settings.declare("launcher", "[]", wrong).is_err());
        // Loading again, a module's binds are its new ones.
        settings.declare("start-menu", "[]", "[]").unwrap();
        assert_eq!(settings.binds().len(), 1);
    }

    #[test]
    fn a_wrong_default_is_a_wrong_declaration() {
        let mut settings = Settings::from_text("");
        let declared = r#"[{ "name": "size", "type": "number", "default": "big" }]"#;
        assert!(settings.declare("taskbar", declared, "[]").is_err());
    }
}
