//! What a shell can use, for editors: bento keeps a copy of its modules, its shells, its API's
//! types and Solid's in `~/.local/share/bento/shell/`, written again when bento changes, with a
//! `tsconfig.json` there that a shell's own extends for an editor to know them. Nothing is
//! written next to the user's files. The types are the one place that says what there is.

use std::path::Path;

use anyhow::Context as _;

use super::modules::STOCK;

/// Types for editors, bento's and Solid's, by their path under `resources/shell`.
const TYPES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/shell_types.rs"));

/// The first line of the `tsconfig.json` bento writes.
const TSCONFIG_MARK: &str = "// Written by bento";

/// Writes the copy and its `tsconfig.json`, if they aren't the ones this bento writes.
pub fn write() {
    let Some(dirs) = directories::BaseDirs::new() else {
        return;
    };
    let copy = dirs.data_dir().join("bento/shell");
    if let Err(err) = write_copy(&copy) {
        warn!("error writing the shell's reference copy: {err:?}");
    }
    let path = copy.join("tsconfig.json");
    let text = tsconfig(&copy);
    if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
        return;
    }
    if let Err(err) = std::fs::write(&path, text) {
        warn!("error writing {}: {err:?}", path.display());
    }
}

/// The files of the copy, by their path in it.
fn files() -> impl Iterator<Item = (&'static str, String)> {
    let scripts = STOCK
        .iter()
        .filter(|(name, _)| name.starts_with("modules/") || name.starts_with("shells/"));
    let written = scripts
        .chain(TYPES)
        .map(|(name, text)| (*name, String::from(*text)));
    written.chain([(PAGES, pages())])
}

/// Where the pages of bento Settings are, as a type.
const PAGES: &str = "types/pages.d.ts";

/// The pages of bento Settings that a setting can name, as the type `page` takes, from the list
/// bento Settings has them in.
fn pages() -> String {
    let mut text = String::from(
        "// Written by bento: the pages of its settings that a setting can name to show on.\n\n\
         declare module \"bento\" {\n  interface SettingsPages {\n",
    );
    for page in niri_config::SETTINGS_PAGES {
        text.push_str(&format!("    {page:?}: true;\n"));
    }
    text.push_str("  }\n}\n");
    text
}

/// What tells this copy from another bento's.
fn stamp() -> String {
    // FNV-1a, which stays the same from one build to the next.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (name, text) in files() {
        for byte in name.bytes().chain([0]).chain(text.bytes()).chain([0]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{hash:016x}\n")
}

fn write_copy(copy: &Path) -> anyhow::Result<()> {
    let stamp_path = copy.join(".stamp");
    let stamp = stamp();
    if std::fs::read_to_string(&stamp_path).is_ok_and(|old| old == stamp) {
        return Ok(());
    }
    // What's there is an older bento's: its files go, so that none that's gone stays.
    for dir in ["modules", "shells", "types"] {
        let dir = copy.join(dir);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .with_context(|| format!("error removing {}", dir.display()))?;
        }
    }
    for (name, text) in files() {
        // Types sit under `types/` in the copy as they do in bento's sources.
        let path = copy.join(name);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, text).with_context(|| format!("error writing {}", path.display()))?;
    }
    std::fs::write(&stamp_path, stamp)?;
    Ok(())
}

/// The `tsconfig.json` of `copy`, which a shell's own extends for an editor to know bento's API
/// and modules, including the types with its own files:
///
/// ```json
/// {
///   "extends": "/home/you/.local/share/bento/shell/tsconfig.json",
///   "include": ["**/*", "/home/you/.local/share/bento/shell/types"]
/// }
/// ```
fn tsconfig(copy: &Path) -> String {
    let copy = copy.to_string_lossy();
    let json = |s: &str| serde_json::to_string(s).unwrap();
    let at = |rest: &str| json(&format!("{copy}/{rest}"));
    format!(
        r#"{TSCONFIG_MARK}, for editors to know bento's shell API and modules, and written again as
// it changes. A shell's own tsconfig.json extends it, and includes its types with its files:
// {{ "extends": "{copy}/tsconfig.json", "include": ["**/*", "{copy}/types"] }}
{{
  "compilerOptions": {{
    "target": "es2023",
    "lib": ["es2023"],
    "module": "esnext",
    "moduleResolution": "bundler",
    "jsx": "preserve",
    "jsxImportSource": "bento",
    "strict": true,
    "noEmit": true,
    "skipLibCheck": true,
    "types": [],
    "paths": {{
      "solid-js": [{solid}],
      "solid-js/*": [{solid_any}],
      "@solidjs/signals": [{signals}],
      "@solidjs/signals/*": [{signals_any}],
      "bento:modules/*": [{modules}],
      "bento:shells/*": [{shells}]
    }}
  }},
  "include": [{bento}, {pages}]
}}
"#,
        solid = at("types/solid-js/index.d.ts"),
        solid_any = at("types/solid-js/*"),
        signals = at("types/@solidjs/signals/index.d.ts"),
        signals_any = at("types/@solidjs/signals/*"),
        modules = at("modules/*"),
        shells = at("shells/*"),
        bento = at("types/bento.d.ts"),
        pages = at(PAGES),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_copy_has_the_modules_shells_and_types_an_editor_needs() {
        let names: Vec<&str> = files().map(|(name, _)| name).collect();
        for name in [
            "modules/taskbar.tsx",
            "shells/windows.tsx",
            "types/bento.d.ts",
            "types/solid-js/index.d.ts",
            "types/@solidjs/signals/index.d.ts",
            "types/pages.d.ts",
        ] {
            assert!(names.contains(&name), "{name} isn't in the copy");
        }
        // bento's runtime and its bundle of Solid are what runs, not what's used.
        assert!(!names.iter().any(|name| name.starts_with("runtime/")));
        assert!(!names.iter().any(|name| name.starts_with("vendor/")));
        assert!(!names.contains(&"runtime/declare.js"));
    }

    #[test]
    fn the_pages_a_setting_can_name_are_a_type() {
        let text = pages();
        assert!(text.contains("interface SettingsPages"));
        for page in niri_config::SETTINGS_PAGES {
            assert!(
                text.contains(&format!("{page:?}: true;")),
                "{page} is missing"
            );
        }
    }

    #[test]
    fn the_tsconfig_points_at_the_copy() {
        let text = tsconfig(Path::new("/home/someone/.local/share/bento/shell"));
        assert!(text.starts_with(TSCONFIG_MARK));
        let json: String = text
            .lines()
            .filter(|line| !line.starts_with("//"))
            .collect();
        let json: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            json["compilerOptions"]["paths"]["bento:modules/*"],
            serde_json::json!(["/home/someone/.local/share/bento/shell/modules/*"])
        );
    }
}
