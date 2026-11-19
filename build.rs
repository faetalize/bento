use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(have_libinput_plugin_system)");
    if pkg_config::Config::new()
        .atleast_version("1.30.0")
        .probe("libinput")
        .is_ok()
    {
        println!("cargo:rustc-cfg=have_libinput_plugin_system")
    }

    shell_files();
}

/// bento's own shell, built in: every script under `resources/shell`, by its path there, so that
/// a file added there is in bento without being listed anywhere.
fn shell_files() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/shell");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();

    let mut out = String::from("&[\n");
    for path in files {
        let name = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        // Types are for editors.
        if name.starts_with("types/") {
            continue;
        }
        println!("cargo:rerun-if-changed={}", path.display());
        out.push_str(&format!(
            "    ({name:?}, include_str!({:?})),\n",
            path.display().to_string()
        ));
    }
    // What engines and shells declare is checked by the engines' prelude's code, for both.
    let declare =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/layouts/prelude/declare.js");
    println!("cargo:rerun-if-changed={}", declare.display());
    out.push_str(&format!(
        "    (\"runtime/declare.js\", include_str!({:?})),\n",
        declare.display().to_string()
    ));
    out.push_str("]\n");
    let dest = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("shell_files.rs");
    std::fs::write(dest, out).unwrap();

    // Types for editors, bento's and Solid's, which bento copies where an editor finds them.
    let mut types = Vec::new();
    collect_all(&root.join("types"), &mut types);
    types.sort();
    let mut out = String::from("&[\n");
    for path in types {
        let name = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        println!("cargo:rerun-if-changed={}", path.display());
        out.push_str(&format!(
            "    ({name:?}, include_str!({:?})),\n",
            path.display().to_string()
        ));
    }
    out.push_str("]\n");
    let dest = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("shell_types.rs");
    std::fs::write(dest, out).unwrap();
}

fn collect_all(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_all(&path, files);
        } else {
            files.push(path);
        }
    }
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, files);
        } else if path
            .extension()
            .is_some_and(|ext| matches!(ext.to_str(), Some("js" | "ts" | "tsx" | "jsx")))
        {
            files.push(path);
        }
    }
}
