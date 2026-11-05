//! Finds icons in the icon themes, as the freedesktop icon theme spec describes.
//!
//! Only raster icons (`.png`) are found: bento has no SVG renderer.

use std::collections::{HashSet, VecDeque};
use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, PartialEq)]
enum DirKind {
    Fixed,
    Scalable,
    Threshold,
}

/// A directory of a theme, with the sizes of the icons in it.
#[derive(Debug, PartialEq)]
struct ThemeDir {
    path: String,
    size: u32,
    scale: u32,
    kind: DirKind,
    min_size: u32,
    max_size: u32,
    threshold: u32,
}

#[derive(Debug, Default, PartialEq)]
struct Index {
    inherits: Vec<String>,
    dirs: Vec<ThemeDir>,
}

impl ThemeDir {
    /// How far its icons are from `size` physical pixels, 0 for an exact fit.
    fn distance(&self, size: u32) -> u32 {
        let scaled = |value: u32| value * self.scale;
        let (min, max) = match self.kind {
            DirKind::Fixed => (scaled(self.size), scaled(self.size)),
            DirKind::Scalable => (scaled(self.min_size), scaled(self.max_size)),
            DirKind::Threshold => (
                scaled(self.size.saturating_sub(self.threshold)),
                scaled(self.size + self.threshold),
            ),
        };

        if size < min {
            min - size
        } else if size > max {
            size - max
        } else {
            0
        }
    }
}

fn parse_index(text: &str) -> Index {
    let mut section = "";
    let mut theme = Vec::new();
    let mut sections: Vec<(&str, Vec<(&str, &str)>)> = Vec::new();

    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name;
            if section != "Icon Theme" {
                sections.push((section, Vec::new()));
            }
        } else if let Some((key, value)) = line.split_once('=') {
            let entry = (key.trim(), value.trim());
            if section == "Icon Theme" {
                theme.push(entry);
            } else if let Some((_, entries)) = sections.last_mut() {
                entries.push(entry);
            }
        }
    }

    let list = |key: &str| -> Vec<String> {
        theme
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, value)| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    };

    let dirs = list("Directories")
        .into_iter()
        .filter_map(|path| {
            let (_, entries) = sections.iter().find(|(name, _)| *name == path)?;
            let number = |key: &str| {
                entries
                    .iter()
                    .find(|(k, _)| *k == key)
                    .and_then(|(_, value)| value.parse::<u32>().ok())
            };

            let size = number("Size")?;
            let kind = match entries.iter().find(|(k, _)| *k == "Type").map(|(_, v)| *v) {
                Some("Fixed") => DirKind::Fixed,
                Some("Scalable") => DirKind::Scalable,
                _ => DirKind::Threshold,
            };

            Some(ThemeDir {
                path,
                size,
                scale: number("Scale").unwrap_or(1),
                kind,
                min_size: number("MinSize").unwrap_or(size),
                max_size: number("MaxSize").unwrap_or(size),
                threshold: number("Threshold").unwrap_or(2),
            })
        })
        .collect();

    Index {
        inherits: list("Inherits"),
        dirs,
    }
}

/// The directories that hold icon themes, in order of precedence.
fn base_dirs() -> Vec<PathBuf> {
    let home = env::var_os("HOME").map(PathBuf::from);
    let non_empty = |name: &str| env::var_os(name).filter(|value| !value.is_empty());

    let data_home = non_empty("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".local/share")));
    let data_dirs =
        non_empty("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());

    home.iter()
        .map(|home| home.join(".icons"))
        .chain(data_home.map(|dir| dir.join("icons")))
        .chain(env::split_paths(&data_dirs).map(|dir| dir.join("icons")))
        .chain([PathBuf::from("/usr/share/pixmaps")])
        .collect()
}

/// The `.png` of the icon `name` that best fits `size` physical pixels, from `theme` or the
/// themes it inherits from, with `hicolor` as the last resort.
pub fn find(theme: &str, name: &str, size: u32) -> Option<PathBuf> {
    find_in(&base_dirs(), theme, name, size)
}

fn find_in(base_dirs: &[PathBuf], theme: &str, name: &str, size: u32) -> Option<PathBuf> {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::from([theme.to_owned()]);

    loop {
        let theme = match queue.pop_front() {
            Some(theme) => theme,
            None if visited.contains("hicolor") => break,
            None => "hicolor".to_owned(),
        };
        if !visited.insert(theme.clone()) {
            continue;
        }

        let Some(index) = base_dirs.iter().find_map(|base| {
            let text = fs::read_to_string(base.join(&theme).join("index.theme")).ok()?;
            Some(parse_index(&text))
        }) else {
            continue;
        };

        let best = index
            .dirs
            .iter()
            .filter_map(|dir| {
                let file = base_dirs
                    .iter()
                    .map(|base| {
                        base.join(&theme)
                            .join(&dir.path)
                            .join(format!("{name}.png"))
                    })
                    .find(|file| file.is_file())?;
                Some((dir.distance(size), file))
            })
            .min_by_key(|(distance, _)| *distance);
        if let Some((_, file)) = best {
            return Some(file);
        }

        queue.extend(index.inherits);
    }

    // Icons that aren't in a theme.
    base_dirs
        .iter()
        .map(|base| base.join(format!("{name}.png")))
        .find(|file| file.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = "\
[Icon Theme]
Name=Test
Inherits=parent, other
Directories=16x16/apps,48x48/apps,scalable/apps

[16x16/apps]
Size=16
Type=Fixed

[48x48/apps]
Size=48
Type=Threshold
Threshold=4

[scalable/apps]
Size=128
Scale=2
Type=Scalable
MinSize=16
MaxSize=512
";

    #[test]
    fn an_index_lists_parents_and_directories() {
        let index = parse_index(INDEX);

        assert_eq!(index.inherits, ["parent", "other"]);
        assert_eq!(index.dirs.len(), 3);
        assert_eq!(
            index.dirs[0],
            ThemeDir {
                path: "16x16/apps".into(),
                size: 16,
                scale: 1,
                kind: DirKind::Fixed,
                min_size: 16,
                max_size: 16,
                threshold: 2,
            }
        );
        assert_eq!(index.dirs[1].kind, DirKind::Threshold);
        assert_eq!(index.dirs[1].threshold, 4);
        assert_eq!(index.dirs[2].kind, DirKind::Scalable);
        assert_eq!(index.dirs[2].scale, 2);
    }

    #[test]
    fn a_directory_without_a_section_is_skipped() {
        let index = parse_index("[Icon Theme]\nDirectories=a,b\n\n[a]\nSize=32\n");
        assert_eq!(index.dirs.len(), 1);
        assert_eq!(index.dirs[0].path, "a");
    }

    #[test]
    fn distance_follows_the_directory_type() {
        let index = parse_index(INDEX);
        let [fixed, threshold, scalable] = &index.dirs[..] else {
            panic!("expected three directories");
        };

        assert_eq!(fixed.distance(16), 0);
        assert_eq!(fixed.distance(24), 8);
        assert_eq!(fixed.distance(10), 6);

        assert_eq!(threshold.distance(44), 0);
        assert_eq!(threshold.distance(52), 0);
        assert_eq!(threshold.distance(64), 12);

        // Its icons are 32 to 1024 physical pixels, because of the scale.
        assert_eq!(scalable.distance(32), 0);
        assert_eq!(scalable.distance(2000), 976);
        assert_eq!(scalable.distance(8), 24);
    }

    #[test]
    fn the_closest_size_wins_and_parents_and_hicolor_are_searched() {
        let root = env::temp_dir().join(format!("bento-icons-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        let write = |path: &str, text: &str| {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let index = |inherits: &str| {
            format!(
                "[Icon Theme]\nInherits={inherits}\nDirectories=16,48\n\
                 [16]\nSize=16\nType=Fixed\n[48]\nSize=48\nType=Fixed\n"
            )
        };

        write("child/index.theme", &index("parent"));
        write("child/16/small.png", "");
        write("child/48/small.png", "");
        write("parent/index.theme", &index(""));
        write("parent/48/inherited.png", "");
        write("hicolor/index.theme", &index(""));
        write("hicolor/16/fallback.png", "");
        write("loose.png", "");

        let dirs = [root.clone()];

        assert_eq!(
            find_in(&dirs, "child", "small", 20),
            Some(root.join("child/16/small.png"))
        );
        assert_eq!(
            find_in(&dirs, "child", "small", 40),
            Some(root.join("child/48/small.png"))
        );
        assert_eq!(
            find_in(&dirs, "child", "inherited", 16),
            Some(root.join("parent/48/inherited.png"))
        );
        assert_eq!(
            find_in(&dirs, "child", "fallback", 16),
            Some(root.join("hicolor/16/fallback.png"))
        );
        assert_eq!(
            find_in(&dirs, "child", "loose", 16),
            Some(root.join("loose.png"))
        );
        assert_eq!(find_in(&dirs, "child", "missing", 16), None);
        // A theme that isn't installed still falls back to hicolor.
        assert_eq!(
            find_in(&dirs, "nope", "fallback", 16),
            Some(root.join("hicolor/16/fallback.png"))
        );

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn inheritance_loops_end() {
        let root = env::temp_dir().join(format!("bento-icons-loop-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (theme, parent) in [("a", "b"), ("b", "a")] {
            let dir = root.join(theme);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("index.theme"),
                format!("[Icon Theme]\nInherits={parent}\nDirectories=\n"),
            )
            .unwrap();
        }

        assert_eq!(find_in(&[root.clone()], "a", "missing", 16), None);
        fs::remove_dir_all(&root).unwrap();
    }
}
