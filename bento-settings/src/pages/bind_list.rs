//! What the Binds page lists, and what changing it writes.
//!
//! An action is what a bind runs, with its arguments and options. A bind is a chord that runs an
//! action, with settings of its own; a chord runs at most one action, and an action has any number
//! of binds. The page lists entries: an action with its binds. An action's default binds stay
//! with whoever declares it: bento, the running engine or a module of the running shell. The
//! user's files hold only what changed: binds of one's own for an action, which replace its
//! defaults, in the main config, or the engine's file for the engine's actions, and the actions
//! turned off with `unbind`. Actions are told apart by their parsed form, so a bind written by
//! hand matches its default however it's spelled.

use std::path::{Path, PathBuf};

use niri_config::bind_set::{self, Declarer};
use niri_config::binds::Bind;
use niri_config::binds::{parse_action, BindFile};
use niri_config::{Action, BindFiles, Key};
use niri_ipc::{LayoutEngineAction, LayoutEngineOptionValue};

use crate::kdl::{ActionNode, BindNode, Document, Value};

/// How binds write the shell's actions: `shell-action "name" args…`.
pub const SHELL_ACTION: &str = "shell-action";

/// What the page lists from: who declares what, the user's files, and the config they make.
pub struct Sources {
    /// bento's own actions.
    pub native: Vec<LayoutEngineAction>,
    /// The running engine's name and actions.
    pub engine: Option<(String, Vec<LayoutEngineAction>)>,
    /// The running shell's actions, each with its module's name.
    pub shell: Vec<(String, LayoutEngineAction)>,
    pub main: Document,
    pub engine_file: Option<Document>,
    /// The shell's file, with a block for each module and shell holding its binds.
    pub shell_file: Option<Document>,
    pub main_path: PathBuf,
    pub engine_path: Option<PathBuf>,
    /// The parsed config's binds, those of included files too, with their files.
    pub binds: Vec<Bind>,
    pub bind_files: BindFiles,
    /// Every action the config turns off.
    pub unbound: Vec<Action>,
    /// The shell's file's binds and the actions it turns off, parsed.
    pub shell_binds: Vec<Bind>,
    pub shell_unbound: Vec<Action>,
}

/// The tab of the page an action is listed in, by who declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    General,
    Engine,
    Shell,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::General, Tab::Engine, Tab::Shell];

    /// Its name, for links to it.
    pub fn name(self) -> &'static str {
        match self {
            Tab::General => "general",
            Tab::Engine => "engine",
            Tab::Shell => "shell",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Engine => "Engine",
            Tab::Shell => "Shell",
        }
    }
}

/// The groups of bento's own actions on the General tab, in their order.
pub const GENERAL_GROUPS: &[&str] = &[
    "Windows",
    "Floating Windows",
    "Window Groups",
    "Workspaces",
    "Monitors",
    "Screenshots & Casting",
    "Input",
    "Launchers",
    "System",
    "Debug",
    "Other",
];

/// The General tab's group for bento's action `name`, by what it does.
pub fn general_group(name: &str) -> &'static str {
    let has = |part: &str| name.contains(part);
    if has("debug") {
        "Debug"
    } else if has("floating") || has("tiling") {
        "Floating Windows"
    } else if name.starts_with("group-window") || name == "ungroup-window" {
        "Window Groups"
    } else if has("monitor") {
        "Monitors"
    } else if has("workspace") || has("overview") {
        "Workspaces"
    } else if name.starts_with("screenshot") || has("cast") {
        "Screenshots & Casting"
    } else if matches!(name, "spawn" | "spawn-sh") {
        "Launchers"
    } else if matches!(
        name,
        "switch-layout" | "toggle-touchpad" | "toggle-keyboard-shortcuts-inhibit"
    ) {
        "Input"
    } else if has("window") {
        "Windows"
    } else if matches!(
        name,
        "quit" | "suspend" | "change-vt" | "show-hotkey-overlay" | "do-screen-transition"
    ) {
        "System"
    } else {
        "Other"
    }
}

/// Where an action is listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Section {
    /// bento's own, in a group of the General tab, like "Windows".
    General(&'static str),
    /// The running engine's, by its name.
    Engine(String),
    /// A module of the running shell's, by its name.
    Module(String),
}

impl Section {
    pub fn tab(&self) -> Tab {
        match self {
            Section::General(_) => Tab::General,
            Section::Engine(_) => Tab::Engine,
            Section::Module(_) => Tab::Shell,
        }
    }
}

/// Where a bind of an entry comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// Whoever declares the action.
    Default,
    /// The main config.
    Main,
    /// The engine's file.
    Engine,
    /// The shell's file, in the block of whoever declares the action.
    Shell,
    /// A file the main config includes, which the page doesn't change.
    Include(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntryBind {
    pub key: Key,
    /// As written, like `Mod+T`.
    pub text: String,
    pub source: Source,
    /// The bind as its file writes it, for the user's.
    pub bind: Option<BindNode>,
    /// Who declares the action that took this bind's chord, if another did.
    pub taken: Option<Declarer>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub section: Section,
    pub declarer: Declarer,
    /// What tells it apart from others.
    pub action: Action,
    /// The action as a bind writes it.
    pub node: ActionNode,
    /// The action's declaration, for its title and arguments.
    pub declaration: Option<LayoutEngineAction>,
    /// Its default binds, with their settings.
    pub defaults: Vec<BindNode>,
    /// The binds it has: the user's own if any, or else its defaults, unless it's turned off.
    pub binds: Vec<EntryBind>,
    /// Whether it's turned off with `unbind`.
    pub disabled: bool,
}

impl Entry {
    /// Whether the user wrote anything for it.
    pub fn changed(&self) -> bool {
        self.disabled
            || self
                .binds
                .iter()
                .any(|k| matches!(k.source, Source::Main | Source::Engine | Source::Shell))
    }

    /// Whether its binds can be changed on the page: none of them is from an included file, and
    /// none is written with press and release actions.
    pub fn editable(&self) -> bool {
        self.binds.iter().all(|k| match k.source {
            Source::Default => true,
            Source::Include(_) => false,
            Source::Main | Source::Engine | Source::Shell => {
                k.bind.as_ref().is_some_and(|b| b.action.is_some())
            }
        })
    }

    /// The file its user binds and `unbind` go in: bento's actions' in the main config, the
    /// engine's in its file, and the shell's in its block of the shell's file.
    pub fn file(&self) -> crate::store::File {
        match self.declarer {
            Declarer::Bento => crate::store::File::Main,
            Declarer::Engine => crate::store::File::Engine,
            Declarer::Shell => crate::store::File::ShellSettings,
        }
    }

    /// The block of the shell's file its binds go in: its module's, or the shell's.
    pub fn block(&self) -> &str {
        match &self.section {
            Section::Module(name) => name,
            _ => "shell",
        }
    }

    /// Its title: its declaration's description, or the action as written.
    pub fn title(&self) -> String {
        match &self.declaration {
            Some(decl) => {
                let description = decl.description.as_deref().unwrap_or(&decl.name);
                description.trim_end_matches('.').to_owned()
            }
            None => self.node.text(),
        }
    }

    /// Its arguments as a bind writes them, a shell action's after its name.
    pub fn arguments(&self) -> String {
        unwrapped(&self.node).arguments()
    }
}

/// A bind's shell action as the shell declares it, by its own name with its own arguments.
pub fn unwrapped(action: &ActionNode) -> ActionNode {
    match action.args.first() {
        Some(Value::Str(name)) if action.name == SHELL_ACTION => ActionNode {
            name: name.clone(),
            args: action.args[1..].to_vec(),
            properties: action.properties.clone(),
        },
        _ => action.clone(),
    }
}

/// A value of a declared argument as a bind writes it; a whole number as one.
pub fn value(value: &LayoutEngineOptionValue) -> Option<Value> {
    match value {
        LayoutEngineOptionValue::Boolean(b) => Some(Value::Bool(*b)),
        LayoutEngineOptionValue::Number(n) if n.fract() == 0. && n.abs() < 1e15 => {
            Some(Value::Int(*n as i64))
        }
        LayoutEngineOptionValue::Number(n) => Some(Value::Float(*n)),
        LayoutEngineOptionValue::String(s) => Some(Value::Str(s.clone())),
        _ => None,
    }
}

/// The action `name` of `declarer` with `args`, as a bind writes it.
pub fn action_node(declarer: Declarer, name: &str, args: &[LayoutEngineOptionValue]) -> ActionNode {
    let args = args.iter().filter_map(value);
    match declarer {
        Declarer::Shell => ActionNode {
            name: String::from(SHELL_ACTION),
            args: std::iter::once(Value::Str(String::from(name)))
                .chain(args)
                .collect(),
            properties: Vec::new(),
        },
        Declarer::Bento | Declarer::Engine => ActionNode {
            name: String::from(name),
            args: args.collect(),
            properties: Vec::new(),
        },
    }
}

/// The parsed action of `node`, as `file` reads it.
pub fn parse(node: &ActionNode, file: BindFile) -> Option<Action> {
    parse_action(&format!("{};", node.text()), file)
}

/// A shell action as the shell's file writes it, `name args…`, as the main config writes it,
/// `shell-action "name" args…`.
fn wrapped(node: &ActionNode) -> ActionNode {
    ActionNode {
        name: String::from(SHELL_ACTION),
        args: std::iter::once(Value::Str(node.name.clone()))
            .chain(node.args.iter().cloned())
            .collect(),
        properties: node.properties.clone(),
    }
}

/// The user's binds and `unbind`s in `src`'s files, each with where it's written and how that
/// file names actions, a shell file's actions written as the main config writes them.
fn user_files(src: &Sources) -> Vec<(Document, Source, BindFile)> {
    let copy = |doc: &Document| Document::parse(doc.text().to_owned()).expect("it parsed");
    let mut files = vec![(copy(&src.main), Source::Main, BindFile::Main)];
    if let Some(doc) = &src.engine_file {
        files.push((copy(doc), Source::Engine, BindFile::Engine));
    }
    if let Some(doc) = &src.shell_file {
        for name in doc.block_names() {
            for block in doc.blocks(&name) {
                files.push((block, Source::Shell, BindFile::Shell));
            }
        }
    }
    files
}

/// Whether `required` arguments are missing, so the action can't be bound without arguments.
fn takes_arguments(decl: &LayoutEngineAction) -> bool {
    decl.args
        .iter()
        .any(|arg| arg.r#type.is_some() && arg.default.is_none() && !arg.optional && !arg.property)
}

/// The actions to list, each with its binds, in sections: bento's by their General groups, the
/// engine's, then each module's.
pub fn entries(src: &Sources) -> Vec<Entry> {
    let mut list: Vec<Entry> = Vec::new();
    let engine_name = src.engine.as_ref().map(|(name, _)| name.clone());
    let module_of = |name: &str| {
        src.shell
            .iter()
            .find(|(_, a)| a.name == name)
            .map(|(m, _)| m.clone())
    };
    let section_of = |declarer: Declarer, action: &Action, name: &str| match declarer {
        Declarer::Bento => Section::General(general_group(name)),
        Declarer::Engine => Section::Engine(engine_name.clone().unwrap_or_default()),
        Declarer::Shell => {
            let name = match action {
                Action::ShellAction(name, _) => module_of(name),
                _ => None,
            };
            Section::Module(name.unwrap_or_else(|| String::from("shell")))
        }
    };
    let declaration = |declarer: Declarer, name: &str| -> Option<LayoutEngineAction> {
        match declarer {
            Declarer::Bento => src.native.iter().find(|a| a.name == name).cloned(),
            Declarer::Engine => src
                .engine
                .as_ref()
                .and_then(|(_, actions)| actions.iter().find(|a| a.name == name).cloned()),
            Declarer::Shell => src
                .shell
                .iter()
                .find(|(_, a)| a.name == name)
                .map(|(_, a)| a.clone()),
        }
    };
    // The entry for `action`, made if it isn't listed yet.
    let at = |list: &mut Vec<Entry>, action: Action, node: ActionNode| -> usize {
        if let Some(i) = list.iter().position(|s| s.action == action) {
            return i;
        }
        let declarer = bind_set::declarer(&action);
        let name = unwrapped(&node).name;
        list.push(Entry {
            section: section_of(declarer, &action, &name),
            declarer,
            declaration: declaration(declarer, &name),
            action,
            node,
            defaults: Vec::new(),
            binds: Vec::new(),
            disabled: false,
        });
        list.len() - 1
    };

    // Defaults: bento's, as it writes them, then the engine's and the shell's declarations.
    let mut defaults: Vec<(usize, BindNode)> = Vec::new();
    let bento = Document::parse(String::from(bind_set::DEFAULT_BINDS)).expect("valid");
    for bind in bento.binds() {
        let Some(node) = bind.action.clone() else {
            continue;
        };
        let Some(action) = parse(&node, BindFile::Main) else {
            continue;
        };
        let i = at(&mut list, action, node.clone());
        defaults.push((
            i,
            BindNode {
                action: Some(node),
                ..bind
            },
        ));
    }
    let declared = src
        .engine
        .iter()
        .flat_map(|(_, actions)| actions.iter().map(|a| (Declarer::Engine, a)))
        .chain(src.shell.iter().map(|(_, a)| (Declarer::Shell, a)));
    for (declarer, decl) in declared {
        for bind in &decl.binds {
            let node = action_node(declarer, &decl.name, &bind.args);
            let action = match declarer {
                Declarer::Shell => bind_set::shell_action(&decl.name, &bind.args),
                _ => bind_set::engine_action(&decl.name, &bind.args),
            };
            let i = at(&mut list, action, node.clone());
            defaults.push((
                i,
                BindNode {
                    key: bind.key.clone(),
                    properties: Vec::new(),
                    action: Some(node),
                },
            ));
        }
    }
    for (i, bind) in defaults {
        list[i].defaults.push(bind);
    }

    // Every action that needs no arguments has a row, bound or not.
    let all = src
        .native
        .iter()
        .map(|a| (Declarer::Bento, a))
        .chain(
            src.engine
                .iter()
                .flat_map(|(_, actions)| actions.iter().map(|a| (Declarer::Engine, a))),
        )
        .chain(src.shell.iter().map(|(_, a)| (Declarer::Shell, a)));
    for (declarer, decl) in all {
        if takes_arguments(decl) {
            continue;
        }
        let node = action_node(declarer, &decl.name, &[]);
        let action = match declarer {
            Declarer::Bento => parse(&node, BindFile::Main),
            Declarer::Engine => Some(bind_set::engine_action(&decl.name, &[])),
            Declarer::Shell => Some(bind_set::shell_action(&decl.name, &[])),
        };
        if let Some(action) = action {
            at(&mut list, action, node);
        }
    }

    // The user's binds, from the main config, the engine's file and the shell's.
    let mut user: Vec<(usize, EntryBind)> = Vec::new();
    for (doc, source, file) in user_files(src) {
        for bind in doc.binds() {
            let Ok(key) = bind.key.parse::<Key>() else {
                continue;
            };
            let parsed = match &bind.action {
                Some(node) if file == BindFile::Shell => {
                    parse(node, file).map(|a| (a, wrapped(node)))
                }
                Some(node) => parse(node, file).map(|a| (a, node.clone())),
                // Written with press and release actions: the parsed config has its action.
                None => src
                    .binds
                    .iter()
                    .find(|b| b.key == key)
                    .and_then(bind_set::action_of)
                    .map(|a| {
                        (
                            a.clone(),
                            ActionNode {
                                name: String::from("…"),
                                args: Vec::new(),
                                properties: Vec::new(),
                            },
                        )
                    }),
            };
            let Some((action, node)) = parsed else {
                continue;
            };
            let i = at(&mut list, action, node);
            let entry_bind = EntryBind {
                key,
                text: bind.key.clone(),
                source: source.clone(),
                bind: Some(bind),
                taken: None,
            };
            user.push((i, entry_bind));
        }
    }
    // Binds of included files, which the page shows but doesn't change.
    for bind in &src.binds {
        let Some(path) = src.bind_files.get(&bind.key) else {
            continue;
        };
        if path == src.main_path || src.engine_path.as_deref() == Some(path) {
            continue;
        }
        let Some(action) = bind_set::action_of(bind) else {
            continue;
        };
        let node = ActionNode {
            name: format!("{action:?}"),
            args: Vec::new(),
            properties: Vec::new(),
        };
        let i = at(&mut list, action.clone(), node);
        user.push((
            i,
            EntryBind {
                key: bind.key,
                text: format!("{:?}", bind.key),
                source: Source::Include(path.to_path_buf()),
                bind: None,
                taken: None,
            },
        ));
    }
    // What's turned off.
    for (doc, _, file) in user_files(src) {
        for node in doc.unbinds() {
            let Some(action) = parse(&node, file) else {
                continue;
            };
            let node = if file == BindFile::Shell {
                wrapped(&node)
            } else {
                node
            };
            let i = at(&mut list, action, node);
            list[i].disabled = true;
        }
    }

    // An entry's binds: the user's own replace its defaults.
    for (i, entry) in list.iter_mut().enumerate() {
        let own: Vec<EntryBind> = user
            .iter()
            .filter(|(j, _)| *j == i)
            .map(|(_, k)| k.clone())
            .collect();
        entry.binds = if !own.is_empty() {
            own
        } else if entry.disabled {
            Vec::new()
        } else {
            entry
                .defaults
                .iter()
                .filter_map(|bind| {
                    Some(EntryBind {
                        key: bind.key.parse().ok()?,
                        text: bind.key.clone(),
                        source: Source::Default,
                        bind: Some(bind.clone()),
                        taken: None,
                    })
                })
                .collect()
        };
    }

    // Which chords another action takes, as bento settles it.
    let mut all_defaults = bind_set::bento_defaults().to_vec();
    if let Some((_, actions)) = &src.engine {
        all_defaults.extend(bind_set::declared_binds(actions, bind_set::engine_action));
    }
    let shell: Vec<LayoutEngineAction> = src.shell.iter().map(|(_, a)| a.clone()).collect();
    all_defaults.extend(bind_set::declared_binds(&shell, bind_set::shell_action));
    let user: Vec<Bind> = src.binds.iter().chain(&src.shell_binds).cloned().collect();
    let unbound: Vec<Action> = src
        .unbound
        .iter()
        .chain(&src.shell_unbound)
        .cloned()
        .collect();
    let resolved = bind_set::resolve(&user, &unbound, &all_defaults);
    for taken in &resolved.taken_over {
        let Some(action) = bind_set::action_of(&taken.bind) else {
            continue;
        };
        let Some(entry) = list.iter_mut().find(|s| s.action == *action) else {
            continue;
        };
        for key in entry.binds.iter_mut().filter(|k| k.key == taken.bind.key) {
            key.taken = Some(taken.by);
        }
    }

    // bento's first, then the engine's, then each module's, in the order they come.
    list.sort_by_key(|s| match s.section {
        Section::General(group) => GENERAL_GROUPS.iter().position(|g| *g == group).unwrap_or(0),
        Section::Engine(_) => GENERAL_GROUPS.len(),
        Section::Module(_) => GENERAL_GROUPS.len() + 1,
    });
    list
}

/// The binds of the user's own for `entry` taken away from `doc`, a file that names actions as
/// `file` does, with its `unbind`.
fn forget(doc: &mut Document, entry: &Entry, file: BindFile) {
    let keys: Vec<Key> = doc
        .binds()
        .into_iter()
        .filter(|bind| {
            bind.action
                .as_ref()
                .and_then(|node| parse(node, file))
                .is_some_and(|a| a == entry.action)
        })
        .filter_map(|bind| bind.key.parse().ok())
        .collect();
    for key in keys {
        doc.remove_bind(&key);
    }
    let off: Vec<ActionNode> = doc
        .unbinds()
        .into_iter()
        .filter(|node| parse(node, file).is_some_and(|a| a == entry.action))
        .collect();
    for node in off {
        doc.remove_unbind(&node);
    }
}

/// What a change to an entry does to the files.
#[derive(Debug, Clone)]
pub enum Change {
    /// Its binds become these; the defaults as they are, settings and all, are written as nothing.
    Keys(Vec<BindNode>),
    /// It's turned off.
    Disable,
    /// It goes back to its defaults.
    Reset,
}

/// Applies `change` to `entry` in `doc`, a file that names actions as `file` does: what the user
/// wrote for it is taken out of every file, and the change written in its own, the shell's
/// actions' in their block of the shell's file.
pub fn apply(doc: &mut Document, entry: &Entry, change: &Change, file: BindFile) {
    use crate::store::File;
    if file == BindFile::Shell {
        for name in doc.block_names() {
            doc.edit_block(&name, |block| forget(block, entry, file));
        }
        if entry.file() == File::ShellSettings {
            let node = unwrapped(&entry.node);
            doc.edit_block(entry.block(), |block| write(block, entry, &node, change));
        }
        return;
    }
    forget(doc, entry, file);
    let mine = match file {
        BindFile::Main => entry.file() == File::Main,
        BindFile::Engine => entry.file() == File::Engine,
        BindFile::Shell => false,
    };
    if mine {
        write(doc, entry, &entry.node, change);
    }
}

/// Writes `change` to `entry` in `doc`, its action written as `node`.
fn write(doc: &mut Document, entry: &Entry, node: &ActionNode, change: &Change) {
    match change {
        Change::Reset => (),
        Change::Disable => doc.add_unbind(node),
        Change::Keys(binds) => {
            // The defaults as they are, chords and settings alike, are written as nothing.
            let same = |a: &BindNode, b: &BindNode| {
                a.key == b.key
                    && a.properties.len() == b.properties.len()
                    && a.properties.iter().all(|p| b.properties.contains(p))
            };
            let at_defaults = binds.len() == entry.defaults.len()
                && binds
                    .iter()
                    .all(|b| entry.defaults.iter().any(|d| same(b, d)));
            if binds.is_empty() && !entry.defaults.is_empty() {
                doc.add_unbind(node);
            } else if !at_defaults {
                for bind in binds {
                    if let Ok(key) = bind.key.parse::<Key>() {
                        let bind = BindNode {
                            action: Some(node.clone()),
                            ..bind.clone()
                        };
                        doc.set_bind(&key, &bind);
                    }
                }
            }
        }
    }
}

/// `entry`'s binds as they're written, its defaults with their settings and the user's as they are.
pub fn binds_of(entry: &Entry) -> Vec<BindNode> {
    entry
        .binds
        .iter()
        .filter(|k| !matches!(k.source, Source::Include(_)))
        .map(|k| {
            k.bind.clone().unwrap_or_else(|| BindNode {
                key: k.text.clone(),
                properties: Vec::new(),
                action: Some(entry.node.clone()),
            })
        })
        .collect()
}

/// The file `path` is, as the page reads it.
pub fn read(path: &Path) -> Document {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    Document::parse(text).unwrap_or_else(|_| Document::parse(String::new()).unwrap())
}

#[cfg(test)]
mod tests {
    use niri_config::Config;
    use niri_ipc::LayoutEngineBind;

    use super::*;

    fn decl(name: &str, binds: &[&str]) -> LayoutEngineAction {
        LayoutEngineAction {
            name: String::from(name),
            description: Some(format!("{name}.")),
            args: Vec::new(),
            binds: binds
                .iter()
                .map(|k| LayoutEngineBind {
                    key: String::from(*k),
                    args: Vec::new(),
                })
                .collect(),
        }
    }

    fn sources(main: &str) -> Sources {
        sources_with(main, "")
    }

    /// With `shell` as the shell's file.
    fn sources_with(main: &str, shell: &str) -> Sources {
        let config = Config::parse_mem(main).unwrap();
        let shell_settings = niri_config::ShellSettings::parse(shell).unwrap();
        Sources {
            native: niri_config::actions::native_actions(),
            engine: Some((String::from("bento"), vec![decl("swap", &["Mod+X"])])),
            shell: vec![(
                String::from("start-menu"),
                decl("toggle-start-menu", &["Mod"]),
            )],
            main: Document::parse(main.to_owned()).unwrap(),
            engine_file: None,
            shell_file: Some(Document::parse(shell.to_owned()).unwrap()),
            main_path: PathBuf::from("config.kdl"),
            engine_path: Some(PathBuf::from("engines/bento.kdl")),
            binds: config.binds.0,
            bind_files: config.bind_files,
            unbound: config.unbound,
            shell_binds: shell_settings.binds.into_iter().map(|(_, b)| b).collect(),
            shell_unbound: shell_settings.unbound.into_iter().map(|(_, a)| a).collect(),
        }
    }

    fn find<'a>(list: &'a [Entry], node: &str) -> &'a Entry {
        list.iter().find(|s| s.node.text() == node).unwrap()
    }

    #[test]
    fn every_action_of_bentos_has_a_general_group() {
        let ungrouped: Vec<String> = niri_config::actions::native_actions()
            .into_iter()
            .map(|a| a.name)
            .filter(|name| name != SHELL_ACTION && general_group(name) == "Other")
            .collect();
        assert!(ungrouped.is_empty(), "{ungrouped:?}");
    }

    #[test]
    fn a_default_binds_settings_stay_when_its_action_gets_another_chord() {
        let list = entries(&sources(""));
        let volume = list
            .iter()
            .find(|s| s.binds.iter().any(|k| k.text == "XF86AudioRaiseVolume"))
            .unwrap();
        let mut binds = binds_of(volume);
        binds.push(BindNode {
            key: String::from("F9"),
            properties: Vec::new(),
            action: Some(volume.node.clone()),
        });
        let mut doc = Document::parse(String::new()).unwrap();
        apply(&mut doc, volume, &Change::Keys(binds), BindFile::Main);
        let written = doc.binds();
        let raise = written
            .iter()
            .find(|b| b.key == "XF86AudioRaiseVolume")
            .unwrap();
        assert!(raise
            .properties
            .contains(&(String::from("allow-when-locked"), Value::Bool(true))));
    }

    #[test]
    fn actions_have_their_default_binds_until_the_user_changes_them() {
        let main = "binds {\n    F1 { spawn \"alacritty\"; }\n    F2 { spawn \"alacritty\"; }\n    unbind { shell-action \"toggle-start-menu\"; }\n}\n";
        let list = entries(&sources(main));
        // bento's default terminal, now on F1 and F2 instead of its own chord.
        let terminal = find(&list, "spawn \"alacritty\"");
        let defaults: Vec<&str> = terminal.defaults.iter().map(|b| b.key.as_str()).collect();
        assert_eq!(defaults, ["Mod+T"]);
        let keys: Vec<&str> = terminal.binds.iter().map(|k| k.text.as_str()).collect();
        assert_eq!(keys, ["F1", "F2"]);
        assert!(terminal.changed());
        // The engine's and the shell's, by their sections.
        let swap = find(&list, "swap");
        assert_eq!(swap.section, Section::Engine(String::from("bento")));
        assert_eq!(swap.binds[0].source, Source::Default);
        let start = find(&list, "shell-action \"toggle-start-menu\"");
        assert_eq!(start.section, Section::Module(String::from("start-menu")));
        assert!(start.disabled && start.binds.is_empty());
        // An action that takes no arguments is listed even unbound.
        let fullscreen = find(&list, "fullscreen-window");
        assert!(fullscreen.binds.iter().all(|k| k.source == Source::Default));
        assert!(!find(&list, "toggle-debug-tint").changed());
    }

    #[test]
    fn changing_an_action_writes_only_what_differs_from_its_defaults() {
        let main = "binds {\n    F1 { spawn \"alacritty\"; }\n}\n";
        let src = sources(main);
        let list = entries(&src);
        let terminal = find(&list, "spawn \"alacritty\"").clone();
        let mut doc = src.main;
        let bind = |key: &str| BindNode {
            key: String::from(key),
            properties: Vec::new(),
            action: Some(terminal.node.clone()),
        };
        // Back on its default bind, settings and all: nothing's written for it.
        apply(
            &mut doc,
            &terminal,
            &Change::Keys(terminal.defaults.clone()),
            BindFile::Main,
        );
        assert!(doc.binds().is_empty() && doc.unbinds().is_empty());
        // Its default chord without the default's settings is a bind of its own.
        apply(
            &mut doc,
            &terminal,
            &Change::Keys(vec![bind("Mod+T")]),
            BindFile::Main,
        );
        let keys: Vec<String> = doc.binds().into_iter().map(|b| b.key).collect();
        assert_eq!(keys, ["Mod+T"]);
        // Off, then reset.
        apply(&mut doc, &terminal, &Change::Disable, BindFile::Main);
        assert_eq!(doc.unbinds(), std::slice::from_ref(&terminal.node));
        apply(&mut doc, &terminal, &Change::Reset, BindFile::Main);
        assert!(doc.unbinds().is_empty());
        // No binds at all is turning it off.
        apply(
            &mut doc,
            &terminal,
            &Change::Keys(Vec::new()),
            BindFile::Main,
        );
        assert_eq!(doc.unbinds(), std::slice::from_ref(&terminal.node));
        // Two binds of its own.
        apply(
            &mut doc,
            &terminal,
            &Change::Keys(vec![bind("F1"), bind("F2")]),
            BindFile::Main,
        );
        let keys: Vec<String> = doc.binds().into_iter().map(|b| b.key).collect();
        assert_eq!(keys, ["F1", "F2"]);
        assert!(doc.unbinds().is_empty());
    }

    #[test]
    fn a_shell_actions_binds_go_in_its_block_of_the_shells_file() {
        let sound = || (String::from("sound"), decl("switch-audio-output", &[]));
        let shell_text = "sound {\n    pinned \"hdmi\"\n}\n";
        let mut src = sources_with("", shell_text);
        src.shell.push(sound());
        let list = entries(&src);
        let switch = find(&list, "shell-action \"switch-audio-output\"").clone();
        assert!(switch.binds.is_empty());
        let mut binds = binds_of(&switch);
        binds.push(BindNode {
            key: String::from("Mod+F9"),
            properties: Vec::new(),
            action: Some(switch.node.clone()),
        });
        let change = Change::Keys(binds);
        let mut main = src.main;
        apply(&mut main, &switch, &change, BindFile::Main);
        let mut shell = Document::parse(String::from(shell_text)).unwrap();
        apply(&mut shell, &switch, &change, BindFile::Shell);

        // Nothing in the main config; in the shell's file, its block, named directly.
        assert_eq!(main.text(), "");
        assert_eq!(
            shell.text(),
            "sound {\n    pinned \"hdmi\"\n\n    binds {\n        Mod+F9 { switch-audio-output; }\n    }\n}\n"
        );
        let mut src = sources_with("", shell.text());
        src.shell.push(sound());
        let list = entries(&src);
        let switch = find(&list, "shell-action \"switch-audio-output\"").clone();
        let keys: Vec<&str> = switch.binds.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(keys, ["Mod+F9"]);
        assert_eq!(switch.binds[0].source, Source::Shell);

        // Turned off, then back to its defaults, which leaves the block as it was.
        apply(&mut shell, &switch, &Change::Disable, BindFile::Shell);
        assert!(shell.text().contains("unbind { switch-audio-output; }"));
        apply(&mut shell, &switch, &Change::Reset, BindFile::Shell);
        assert_eq!(shell.text(), shell_text);
    }

    #[test]
    fn a_chord_taken_from_a_bind_of_ones_own_moves_to_a_shell_action() {
        let main = "binds {\n    Mod+Shift+F1 { spawn \"switcher\"; }\n}\n";
        let sound = || (String::from("sound"), decl("switch-audio-output", &[]));
        let mut src = sources(main);
        src.shell.push(sound());
        let list = entries(&src);
        let spawn = find(&list, "spawn \"switcher\"").clone();
        let switch = find(&list, "shell-action \"switch-audio-output\"").clone();
        let mut doc = src.main;
        let mut shell = Document::parse(String::new()).unwrap();
        // The chord off the program, which has no other, then onto the sound switch.
        apply(&mut doc, &spawn, &Change::Disable, BindFile::Main);
        let bind = BindNode {
            key: String::from("Mod+Shift+F1"),
            properties: Vec::new(),
            action: Some(switch.node.clone()),
        };
        let change = Change::Keys(vec![bind]);
        apply(&mut doc, &switch, &change, BindFile::Main);
        apply(&mut shell, &switch, &change, BindFile::Shell);

        let config = Config::parse_mem(doc.text());
        assert!(config.is_ok(), "{}\n{:?}", doc.text(), config.err());
        let mut src = sources_with(doc.text(), shell.text());
        src.shell.push(sound());
        let list = entries(&src);
        let keys: Vec<&str> = find(&list, "shell-action \"switch-audio-output\"")
            .binds
            .iter()
            .map(|b| b.text.as_str())
            .collect();
        assert_eq!(keys, ["Mod+Shift+F1"], "{}", shell.text());
    }

    #[test]
    fn a_chord_another_action_takes_says_whose() {
        // The user's Mod+V for bento loses to the clipboard module's default.
        let mut src = sources("binds {\n    Mod+V { spawn \"paste\"; }\n}\n");
        src.shell.push((
            String::from("clipboard"),
            decl("toggle-clipboard", &["Mod+V"]),
        ));
        let list = entries(&src);
        let paste = find(&list, "spawn \"paste\"");
        assert_eq!(paste.binds[0].taken, Some(Declarer::Shell));
        assert_eq!(
            find(&list, "shell-action \"toggle-clipboard\"").binds[0].taken,
            None
        );
    }
}
