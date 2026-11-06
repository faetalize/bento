//! The icon theme that apps on the session use, which engines' image nodes look icons up in.
//!
//! GTK apps, bento Settings included, take it from the `icon-theme` key of GNOME's interface
//! settings, which the settings portal serves, and otherwise from `gtk-icon-theme-name` in the
//! GTK 4 settings file. The portal's value comes in through [`IconTheme::set_portal`].

use std::path::{Path, PathBuf};
use std::time::SystemTime;
use std::{env, fs};

/// The theme that's used when nothing names one, and the one every theme falls back to.
pub const DEFAULT: &str = "hicolor";

/// Where the icon theme name comes from: the portal if it has one, else the GTK settings file.
#[derive(Debug)]
pub struct IconTheme {
    portal: Option<String>,
    file: GtkSettings,
}

/// The theme name in the GTK settings file, and when the file last changed.
#[derive(Debug)]
struct GtkSettings {
    path: Option<PathBuf>,
    mtime: Option<SystemTime>,
    theme: Option<String>,
}

impl IconTheme {
    pub fn new() -> Self {
        let mut file = GtkSettings {
            path: settings_path(),
            mtime: None,
            theme: None,
        };
        file.reload();
        Self { portal: None, file }
    }

    /// Sets the theme the settings portal reports, or clears it.
    pub fn set_portal(&mut self, theme: Option<String>) {
        self.portal = theme.filter(|theme| !theme.is_empty());
    }

    /// Reads the GTK settings file again if it changed.
    pub fn poll(&mut self) {
        self.file.poll();
    }

    /// The theme to use.
    pub fn name(&self) -> &str {
        self.portal
            .as_deref()
            .or(self.file.theme.as_deref())
            .unwrap_or(DEFAULT)
    }
}

impl GtkSettings {
    fn poll(&mut self) {
        let mtime = self
            .path
            .as_deref()
            .and_then(|path| fs::metadata(path).ok())
            .and_then(|meta| meta.modified().ok());
        if mtime != self.mtime {
            self.reload();
        }
    }

    fn reload(&mut self) {
        let Some(path) = &self.path else { return };
        self.mtime = fs::metadata(path)
            .ok()
            .and_then(|meta| meta.modified().ok());
        self.theme = fs::read_to_string(path)
            .ok()
            .and_then(|text| parse_gtk_settings(&text));
    }
}

fn settings_path() -> Option<PathBuf> {
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| Path::new(&home).join(".config")))?;
    Some(config_home.join("gtk-4.0/settings.ini"))
}

/// `gtk-icon-theme-name` from a GTK settings file.
fn parse_gtk_settings(text: &str) -> Option<String> {
    let mut in_settings = false;
    for line in text.lines().map(str::trim) {
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_settings = section == "Settings";
        } else if in_settings {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if key.trim() == "gtk-icon-theme-name" {
                let value = value.trim();
                return (!value.is_empty()).then(|| value.to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_theme_is_read_from_the_settings_section() {
        let text = "\
[Other]
gtk-icon-theme-name=Wrong

[Settings]
gtk-theme-name=Adwaita
gtk-icon-theme-name = Papirus-Dark
";
        assert_eq!(parse_gtk_settings(text).as_deref(), Some("Papirus-Dark"));
    }

    #[test]
    fn a_settings_file_without_a_theme_names_none() {
        assert_eq!(parse_gtk_settings("[Settings]\ngtk-theme-name=x\n"), None);
        assert_eq!(
            parse_gtk_settings("[Settings]\ngtk-icon-theme-name=\n"),
            None
        );
        assert_eq!(parse_gtk_settings(""), None);
    }

    fn with_file(theme: Option<&str>) -> IconTheme {
        IconTheme {
            portal: None,
            file: GtkSettings {
                path: None,
                mtime: None,
                theme: theme.map(String::from),
            },
        }
    }

    #[test]
    fn the_portal_wins_over_the_file_and_hicolor_is_the_last_resort() {
        let mut theme = with_file(Some("FromFile"));
        assert_eq!(theme.name(), "FromFile");

        theme.set_portal(Some("FromPortal".into()));
        assert_eq!(theme.name(), "FromPortal");

        // A portal that has no theme leaves it to the file.
        theme.set_portal(Some(String::new()));
        assert_eq!(theme.name(), "FromFile");
        theme.set_portal(None);
        assert_eq!(theme.name(), "FromFile");

        assert_eq!(with_file(None).name(), DEFAULT);
    }

    #[test]
    fn the_file_is_read_again_when_it_changes() {
        let dir = env::temp_dir().join(format!("bento-icon-theme-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.ini");

        fs::write(&path, "[Settings]\ngtk-icon-theme-name=First\n").unwrap();
        let mut file = GtkSettings {
            path: Some(path.clone()),
            mtime: None,
            theme: None,
        };
        file.reload();
        assert_eq!(file.theme.as_deref(), Some("First"));

        fs::write(&path, "[Settings]\ngtk-icon-theme-name=Second\n").unwrap();
        // Writes in the same instant leave the mtime alone, so the test sets it apart.
        let later = file.mtime.unwrap() + std::time::Duration::from_secs(5);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
        file.poll();
        assert_eq!(file.theme.as_deref(), Some("Second"));

        fs::remove_file(&path).unwrap();
        file.poll();
        assert_eq!(file.theme, None);

        fs::remove_dir_all(&dir).unwrap();
    }
}
