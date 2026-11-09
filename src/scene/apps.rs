//! The apps the shell can start: the desktop entries in the XDG data dirs, read again whenever one
//! of their folders changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct App {
    /// The desktop entry's id, like `org.gnome.Nautilus.desktop`.
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub keywords: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip)]
    exec: String,
    #[serde(skip)]
    terminal: bool,
}

#[derive(Default)]
pub struct Apps {
    apps: Vec<App>,
    /// When each folder last changed, when the apps were read.
    read: Vec<(PathBuf, Option<SystemTime>)>,
}

/// The folders desktop entries are in, the user's first.
fn dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/share")));
    let system = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| String::from("/usr/local/share:/usr/share"));
    home.into_iter()
        .chain(
            system
                .split(':')
                .filter(|d| !d.is_empty())
                .map(PathBuf::from),
        )
        .map(|dir| dir.join("applications"))
        .collect()
}

fn mtime(dir: &Path) -> Option<SystemTime> {
    dir.metadata().ok()?.modified().ok()
}

impl Apps {
    /// The apps that show in launchers, by name, read again if their folders changed.
    pub fn list(&mut self) -> &[App] {
        let dirs = dirs();
        let read: Vec<_> = dirs.iter().map(|d| (d.clone(), mtime(d))).collect();
        if read != self.read {
            self.read = read;
            self.apps = load(&dirs);
        }
        &self.apps
    }

    /// Starts app `id`, and says whether there's one.
    pub fn launch(&mut self, id: &str) -> bool {
        let Some(app) = self.list().iter().find(|app| app.id == id).cloned() else {
            return false;
        };
        let mut command = exec_args(&app.exec);
        if command.is_empty() {
            return false;
        }
        if app.terminal {
            // The terminal that the desktop's default `x-terminal-emulator` or `xdg-terminal-exec`
            // gives.
            command.insert(0, String::from("xdg-terminal-exec"));
        }
        crate::utils::spawning::spawn(command, None);
        true
    }
}

fn load(dirs: &[PathBuf]) -> Vec<App> {
    // An id in an earlier folder hides the same id in later ones.
    let mut by_id: HashMap<String, Option<App>> = HashMap::new();
    for dir in dirs {
        let Ok(entries) = walk(dir) else { continue };
        for (id, path) in entries {
            if by_id.contains_key(&id) {
                continue;
            }
            let app = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| parse(&id, &text));
            by_id.insert(id, app);
        }
    }
    let mut apps: Vec<App> = by_id.into_values().flatten().collect();
    apps.sort_by_key(|app| app.name.to_lowercase());
    apps
}

/// The desktop entries under `dir`, with their ids: a path under it with `/` as `-`.
fn walk(dir: &Path) -> std::io::Result<Vec<(String, PathBuf)>> {
    let mut found = Vec::new();
    let mut queue = vec![dir.to_path_buf()];
    while let Some(current) = queue.pop() {
        for entry in std::fs::read_dir(&current)? {
            let path = entry?.path();
            if path.is_dir() {
                queue.push(path);
            } else if path.extension().is_some_and(|e| e == "desktop") {
                let Ok(relative) = path.strip_prefix(dir) else {
                    continue;
                };
                let id = relative.to_string_lossy().replace('/', "-");
                found.push((id, path));
            }
        }
    }
    Ok(found)
}

/// The app a desktop entry describes, if it's one that shows in launchers.
fn parse(id: &str, text: &str) -> Option<App> {
    let mut in_entry = false;
    let mut fields: HashMap<&str, &str> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            fields.entry(key.trim()).or_insert(value.trim());
        }
    }
    let yes = |key: &str| fields.get(key).is_some_and(|v| *v == "true");
    if fields.get("Type") != Some(&"Application") || yes("NoDisplay") || yes("Hidden") {
        return None;
    }
    let field = |key: &str| {
        fields
            .get(key)
            .map(|v| unescape(v))
            .filter(|v| !v.is_empty())
    };
    Some(App {
        id: String::from(id),
        name: field("Name")?,
        generic: field("GenericName"),
        comment: field("Comment"),
        keywords: field("Keywords")
            .map(|k| {
                k.split(';')
                    .filter(|k| !k.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default(),
        icon: field("Icon"),
        exec: field("Exec")?,
        terminal: yes("Terminal"),
    })
}

fn unescape(value: &str) -> String {
    value
        .replace("\\s", " ")
        .replace("\\n", "\n")
        .replace("\\t", "\t")
        .replace("\\\\", "\\")
}

/// An `Exec` line as the command's words, without the field codes for files and URLs, which a
/// launcher has none of.
fn exec_args(exec: &str) -> Vec<String> {
    shlex::split(exec)
        .unwrap_or_default()
        .into_iter()
        .filter(|arg| !(arg.len() == 2 && arg.starts_with('%')))
        .map(|arg| arg.replace("%%", "%"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_reads_as_an_app_with_its_command_without_field_codes() {
        let text = "[Desktop Entry]\nType=Application\nName=Files\nKeywords=folder;manager;\n\
                    Icon=org.gnome.Nautilus\nExec=nautilus --new-window %U\n\
                    [Desktop Action new]\nName=New\n";
        let app = parse("org.gnome.Nautilus.desktop", text).unwrap();
        assert_eq!(app.name, "Files");
        assert_eq!(app.keywords, ["folder", "manager"]);
        assert_eq!(exec_args(&app.exec), ["nautilus", "--new-window"]);
        assert!(parse(
            "x.desktop",
            "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true"
        )
        .is_none());
    }
}
