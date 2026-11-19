//! Where the shell's scripts come from, and how they're made into JavaScript.
//!
//! A shell is TSX with Solid, compiled here with Solid's own compiler and Oxc as each file is
//! loaded: nothing is installed or built beforehand. What a script imports, by name:
//!
//! - `solid-js`: Solid, built in.
//! - `bento`: bento's API, built in.
//! - `bento:modules/<name>`: the user's `modules/<name>.tsx` in `~/.config/bento/shell/` if it's
//!   there, which takes the place of bento's own everywhere, or bento's own;
//! - `bento:stock/modules/<name>`: always bento's own, to build on;
//! - `bento:shells/<name>` and `bento:stock/shells/<name>`: the same for whole shells;
//! - `./file` and `../file`: next to the script that imports it, with or without its extension.
//!
//! bento's own files are built in. To work on them, `debug { shell-scripts }` names a folder that
//! takes their place file by file: a file there is read in place of the built-in one with its
//! path, and the rest stay built in.

use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use anyhow::{anyhow, bail, Context as _};
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::Declared;
use rquickjs::{Ctx, Module};

/// bento's own shell: every script under `resources/shell`, by its path there.
pub const STOCK: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/shell_files.rs"));

#[cfg(test)]
thread_local! {
    /// The shell a test starts from, if it isn't bento's own.
    pub static TEST_ENTRY: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// What names bento's own files go by, to tell them from the user's, which go by their path.
const STOCK_PREFIX: &str = "stock:";

/// The extensions a script can have, tried in this order for a name given without one.
const EXTENSIONS: [&str; 4] = ["tsx", "ts", "jsx", "js"];

/// Where the shell's files are.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    /// The user's: `~/.config/bento/shell/`.
    pub user: Option<PathBuf>,
    /// bento's own, read from there where they are, rather than the ones built in.
    pub stock: Option<PathBuf>,
}

impl Sources {
    pub fn new(stock: Option<&Path>) -> Self {
        let user = if cfg!(test) {
            None
        } else {
            directories::BaseDirs::new().map(|dirs| dirs.config_dir().join("bento/shell"))
        };
        Self {
            user: user.filter(|dir| dir.is_dir()),
            stock: stock.map(Path::to_path_buf),
        }
    }

    /// The module the shell starts from: the user's `main.tsx`, or bento's own shell.
    pub fn entry(&self) -> String {
        #[cfg(test)]
        if let Some(entry) = TEST_ENTRY.with(|entry| entry.borrow().clone()) {
            return entry;
        }
        self.user
            .as_ref()
            .and_then(|dir| with_extension(&dir.join("main")))
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| String::from("bento:shells/unix"))
    }

    /// When each of the shell's files last changed, to load them again when one does.
    pub fn mtimes(&self) -> Vec<(PathBuf, Option<SystemTime>)> {
        let mut files = Vec::new();
        for dir in self.user.iter().chain(&self.stock) {
            scripts_in(dir, &mut files);
        }
        files.sort();
        files
            .into_iter()
            .map(|path| {
                let mtime = path.metadata().ok().and_then(|m| m.modified().ok());
                (path, mtime)
            })
            .collect()
    }

    /// The name of the module `name` that `base` imports.
    pub fn resolve(&self, base: &str, name: &str) -> anyhow::Result<String> {
        let stock = |path: &str| -> anyhow::Result<String> {
            self.stock_name(path)
                .ok_or_else(|| anyhow!("bento has no {path:?}"))
        };
        match name {
            "solid-js" | "solid-js/store" => return stock("vendor/solid"),
            "bento" | "bento/jsx-runtime" => return stock("runtime/bento"),
            _ => (),
        }
        if let Some(rest) = name.strip_prefix("bento:") {
            if let Some(path) = rest.strip_prefix("stock/") {
                return stock(path);
            }
            if let Some(user) = &self.user {
                if let Some(path) = with_extension(&user.join(rest)) {
                    return Ok(path.to_string_lossy().into_owned());
                }
            }
            return stock(rest);
        }
        if name.starts_with("./") || name.starts_with("../") {
            if let Some(dir) = base.strip_prefix(STOCK_PREFIX) {
                let dir = Path::new(dir).parent().unwrap_or(Path::new(""));
                let path = normalize(&dir.join(name));
                return stock(&path.to_string_lossy());
            }
            let dir = Path::new(base).parent().unwrap_or(Path::new("/"));
            let path = normalize(&dir.join(name));
            return with_extension(&path)
                .map(|path| path.to_string_lossy().into_owned())
                .ok_or_else(|| anyhow!("there's no {}", path.display()));
        }
        if name.starts_with('/') {
            return with_extension(Path::new(name))
                .map(|path| path.to_string_lossy().into_owned())
                .ok_or_else(|| anyhow!("there's no {name}"));
        }
        bail!("there's no module {name:?}: bento's are `bento`, `solid-js` and `bento:…`")
    }

    /// bento's own file at `path`, with or without its extension, by the name it goes by.
    fn stock_name(&self, path: &str) -> Option<String> {
        let found = |candidate: &str| {
            self.stock
                .as_ref()
                .is_some_and(|dir| dir.join(candidate).is_file())
                || STOCK.iter().any(|(name, _)| *name == candidate)
        };
        if found(path) {
            return Some(format!("{STOCK_PREFIX}{path}"));
        }
        EXTENSIONS
            .iter()
            .map(|ext| format!("{path}.{ext}"))
            .find(|candidate| found(candidate))
            .map(|candidate| format!("{STOCK_PREFIX}{candidate}"))
    }

    /// The source of module `name`, as it was written.
    pub fn read(&self, name: &str) -> anyhow::Result<String> {
        if let Some(path) = name.strip_prefix(STOCK_PREFIX) {
            if let Some(file) = self.stock.as_ref().map(|dir| dir.join(path)) {
                if file.is_file() {
                    return std::fs::read_to_string(&file)
                        .with_context(|| format!("error reading {}", file.display()));
                }
            }
            return STOCK
                .iter()
                .find(|(file, _)| *file == path)
                .map(|(_, source)| String::from(*source))
                .ok_or_else(|| anyhow!("bento has no {path}"));
        }
        std::fs::read_to_string(name).with_context(|| format!("error reading {name}"))
    }

    /// Module `name` as JavaScript, compiled from TSX, JSX or TypeScript.
    pub fn load(&self, name: &str) -> anyhow::Result<String> {
        let source = self.read(name)?;
        compile(name, &source).with_context(|| format!("error compiling {name}"))
    }
}

/// `path` as it is, or with the first extension a script there has.
fn with_extension(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    EXTENSIONS
        .iter()
        .map(|ext| {
            let mut name = path.as_os_str().to_owned();
            name.push(".");
            name.push(ext);
            PathBuf::from(name)
        })
        .find(|path| path.is_file())
}

/// `path` with `.` and `..` taken out.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => (),
            other => out.push(other),
        }
    }
    out
}

/// The scripts in `dir` and the folders in it.
fn scripts_in(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "node_modules") {
                scripts_in(&path, files);
            }
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| EXTENSIONS.contains(&ext))
        {
            files.push(path);
        }
    }
}

/// Compiles the script `name` to JavaScript: its JSX with Solid's compiler for bento's renderer,
/// then its TypeScript taken out.
pub fn compile(name: &str, source: &str) -> anyhow::Result<String> {
    let ext = Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("js");
    let jsx = matches!(ext, "tsx" | "jsx");
    let typescript = matches!(ext, "tsx" | "ts");
    if !jsx && !typescript {
        return Ok(String::from(source));
    }

    let code = if jsx {
        let options = solidjs_compiler::CompileOptions {
            filename: Some(String::from(name)),
            module_name: String::from("bento"),
            generate: solidjs_compiler::Generate::Universal,
            ..Default::default()
        };
        solidjs_compiler::compile(source, &options)
            .map_err(|err| anyhow!("{err}"))?
            .code
    } else {
        String::from(source)
    };
    if !typescript {
        return Ok(code);
    }
    strip_types(name, &code)
}

/// Takes the TypeScript out of `code`.
fn strip_types(name: &str, code: &str) -> anyhow::Result<String> {
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_transformer::{TransformOptions, Transformer};

    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, code, SourceType::ts()).parse();
    if let Some(err) = parsed.diagnostics.first() {
        bail!("{err}");
    }
    let mut program = parsed.program;
    let scoping = SemanticBuilder::new()
        .build(&program)
        .semantic
        .into_scoping();
    let options = TransformOptions::default();
    let done = Transformer::new(&allocator, Path::new(name), &options)
        .build_with_scoping(scoping, &mut program);
    if let Some(err) = done.diagnostics.first() {
        bail!("{err}");
    }
    Ok(Codegen::new().build(&program).code)
}

/// The shell's scripts, for QuickJS to find and load what they import.
#[derive(Clone)]
pub struct ScriptLoader {
    pub sources: std::rc::Rc<Sources>,
}

impl Resolver for ScriptLoader {
    fn resolve<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        self.sources
            .resolve(base, name)
            .map_err(|err| rquickjs::Exception::throw_message(ctx, &format!("{base}: {err:#}")))
    }
}

impl Loader for ScriptLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        let code = self
            .sources
            .load(name)
            .map_err(|err| rquickjs::Exception::throw_message(ctx, &format!("{err:#}")))?;
        Module::declare(ctx.clone(), name, code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_script_compiles() {
        for (name, source) in STOCK {
            compile(name, source).unwrap_or_else(|err| panic!("{name}: {err:#}"));
        }
    }
}

