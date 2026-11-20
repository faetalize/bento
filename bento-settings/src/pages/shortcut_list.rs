//! The shortcuts the Shortcuts page lists, and what changing one writes.
//!
//! A shortcut is an action with its arguments, and has any number of keys. Its default keys stay
//! with whoever declares the action: bento, the running engine or a module of the running shell.
//! The user's files hold only what changed: keys of one's own for a shortcut, which replace its
//! defaults, in the main config, or the engine's file for the engine's shortcuts, and the
//! shortcuts turned off with `unbind`. Shortcuts are told apart by their parsed action, so a bind
//! written by hand matches its default however it's spelled.

use std::path::{Path, PathBuf};

use niri_config::binds::parse_action;
use niri_config::binds::Bind;
use niri_config::shortcuts::{self, Declarer};
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
    pub main_path: PathBuf,
    pub engine_path: PathBuf,
    /// The parsed config's binds, those of included files too, with their files.
    pub binds: Vec<Bind>,
    pub bind_files: BindFiles,
    /// Every shortcut the config turns off.
    pub unbound: Vec<Action>,
}

/// Where a shortcut is listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Section {
    Bento,
    /// The running engine's, by its name.
    Engine(String),
    /// A module of the running shell's, by its name.
    Module(String),
}

/// Where a key of a shortcut comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// Whoever declares the action.
    Default,
    /// The main config.
    Main,
    /// The engine's file.
    Engine,
    /// A file the main config includes, which the page doesn't change.
    Include(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShortcutKey {
    pub key: Key,
    /// As written, like `Mod+T`.
    pub text: String,
    pub source: Source,
    /// The bind as its file writes it, for the user's.
    pub bind: Option<BindNode>,
    /// Who declares the shortcut that took this key, if another did.
    pub taken: Option<Declarer>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shortcut {
    pub section: Section,
    pub declarer: Declarer,
    /// What tells it apart from others.
    pub action: Action,
    /// The action as a bind writes it.
    pub node: ActionNode,
    /// The action's declaration, for its title and arguments.
    pub declaration: Option<LayoutEngineAction>,
    /// Its default keys, as written.
    pub defaults: Vec<String>,
    /// The keys it has: the user's own if any, or else its defaults, unless it's turned off.
    pub keys: Vec<ShortcutKey>,
    /// Whether it's turned off with `unbind`.
    pub disabled: bool,
}

impl Shortcut {
    /// Whether the user wrote anything for it.
    pub fn changed(&self) -> bool {
        self.disabled
            || self
                .keys
                .iter()
                .any(|k| matches!(k.source, Source::Main | Source::Engine))
    }

    /// Whether its keys can be changed on the page: none of them is from an included file, and
    /// none is written with press and release actions.
    pub fn editable(&self) -> bool {
        self.keys.iter().all(|k| match k.source {
            Source::Default => true,
            Source::Include(_) => false,
            Source::Main | Source::Engine => k.bind.as_ref().is_some_and(|b| b.action.is_some()),
        })
    }

    /// The file its user binds and `unbind` go in.
    pub fn file(&self) -> crate::store::File {
        match self.declarer {
            Declarer::Engine => crate::store::File::Engine,
            Declarer::Bento | Declarer::Shell => crate::store::File::Main,
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

/// The parsed action of `node`, as the main config reads it, or the engine's file with `engine`.
pub fn parse(node: &ActionNode, engine: bool) -> Option<Action> {
    parse_action(&format!("{};", node.text()), engine)
}

/// Whether `required` arguments are missing, so the action can't be bound without arguments.
fn takes_arguments(decl: &LayoutEngineAction) -> bool {
    decl.args
        .iter()
        .any(|arg| arg.r#type.is_some() && arg.default.is_none() && !arg.optional && !arg.property)
}

/// The shortcuts to list, in sections: bento's, the engine's, then each module's.
pub fn shortcuts(src: &Sources) -> Vec<Shortcut> {
    let mut list: Vec<Shortcut> = Vec::new();
    let engine_name = src.engine.as_ref().map(|(name, _)| name.clone());
    let module_of = |name: &str| {
        src.shell
            .iter()
            .find(|(_, a)| a.name == name)
            .map(|(m, _)| m.clone())
    };
    let section_of = |declarer: Declarer, action: &Action| match declarer {
        Declarer::Bento => Section::Bento,
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
    // The shortcut for `action`, made if it isn't listed yet.
    let at = |list: &mut Vec<Shortcut>, action: Action, node: ActionNode| -> usize {
        if let Some(i) = list.iter().position(|s| s.action == action) {
            return i;
        }
        let declarer = shortcuts::declarer(&action);
        let name = unwrapped(&node).name;
        list.push(Shortcut {
            section: section_of(declarer, &action),
            declarer,
            declaration: declaration(declarer, &name),
            action,
            node,
            defaults: Vec::new(),
            keys: Vec::new(),
            disabled: false,
        });
        list.len() - 1
    };

    // Defaults: bento's, as it writes them, then the engine's and the shell's declarations.
    let mut defaults: Vec<(usize, String)> = Vec::new();
    let bento = Document::parse(String::from(shortcuts::DEFAULT_BINDS)).expect("valid");
    for bind in bento.binds() {
        let Some(node) = bind.action.clone() else {
            continue;
        };
        let Some(action) = parse(&node, false) else {
            continue;
        };
        let i = at(&mut list, action, node);
        defaults.push((i, bind.key));
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
                Declarer::Shell => shortcuts::shell_action(&decl.name, &bind.args),
                _ => shortcuts::engine_action(&decl.name, &bind.args),
            };
            let i = at(&mut list, action, node);
            defaults.push((i, bind.key.clone()));
        }
    }
    for (i, key) in &defaults {
        list[*i].defaults.push(key.clone());
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
            Declarer::Bento => parse(&node, false),
            Declarer::Engine => Some(shortcuts::engine_action(&decl.name, &[])),
            Declarer::Shell => Some(shortcuts::shell_action(&decl.name, &[])),
        };
        if let Some(action) = action {
            at(&mut list, action, node);
        }
    }

    // The user's binds, from the main config and the engine's file.
    let mut user: Vec<(usize, ShortcutKey)> = Vec::new();
    let files = [
        (Some(&src.main), Source::Main, false),
        (src.engine_file.as_ref(), Source::Engine, true),
    ];
    for (doc, source, engine) in files {
        let Some(doc) = doc else { continue };
        for bind in doc.binds() {
            let Ok(key) = bind.key.parse::<Key>() else {
                continue;
            };
            let parsed = match &bind.action {
                Some(node) => parse(node, engine).map(|a| (a, node.clone())),
                // Written with press and release actions: the parsed config has its action.
                None => src
                    .binds
                    .iter()
                    .find(|b| b.key == key)
                    .and_then(shortcuts::shortcut)
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
            let shortcut_key = ShortcutKey {
                key,
                text: bind.key.clone(),
                source: source.clone(),
                bind: Some(bind),
                taken: None,
            };
            user.push((i, shortcut_key));
        }
    }
    // Binds of included files, which the page shows but doesn't change.
    for bind in &src.binds {
        let Some(path) = src.bind_files.get(&bind.key) else {
            continue;
        };
        if path == src.main_path || path == src.engine_path {
            continue;
        }
        let Some(action) = shortcuts::shortcut(bind) else {
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
            ShortcutKey {
                key: bind.key,
                text: format!("{:?}", bind.key),
                source: Source::Include(path.to_path_buf()),
                bind: None,
                taken: None,
            },
        ));
    }
    // What's turned off.
    for (doc, engine) in [(Some(&src.main), false), (src.engine_file.as_ref(), true)] {
        let Some(doc) = doc else { continue };
        for node in doc.unbinds() {
            let Some(action) = parse(&node, engine) else {
                continue;
            };
            let i = at(&mut list, action, node);
            list[i].disabled = true;
        }
    }

    // A shortcut's keys: the user's own replace its defaults.
    for (i, shortcut) in list.iter_mut().enumerate() {
        let own: Vec<ShortcutKey> = user
            .iter()
            .filter(|(j, _)| *j == i)
            .map(|(_, k)| k.clone())
            .collect();
        shortcut.keys = if !own.is_empty() {
            own
        } else if shortcut.disabled {
            Vec::new()
        } else {
            shortcut
                .defaults
                .iter()
                .filter_map(|text| {
                    Some(ShortcutKey {
                        key: text.parse().ok()?,
                        text: text.clone(),
                        source: Source::Default,
                        bind: None,
                        taken: None,
                    })
                })
                .collect()
        };
    }

    // Which keys another shortcut takes, as bento settles it.
    let mut all_defaults = shortcuts::bento_defaults().to_vec();
    if let Some((_, actions)) = &src.engine {
        all_defaults.extend(shortcuts::declared_binds(actions, shortcuts::engine_action));
    }
    let shell: Vec<LayoutEngineAction> = src.shell.iter().map(|(_, a)| a.clone()).collect();
    all_defaults.extend(shortcuts::declared_binds(&shell, shortcuts::shell_action));
    let resolved = shortcuts::resolve(&src.binds, &src.unbound, &all_defaults);
    for taken in &resolved.taken_over {
        let Some(action) = shortcuts::shortcut(&taken.bind) else {
            continue;
        };
        let Some(shortcut) = list.iter_mut().find(|s| s.action == *action) else {
            continue;
        };
        for key in shortcut.keys.iter_mut().filter(|k| k.key == taken.bind.key) {
            key.taken = Some(taken.by);
        }
    }

    // bento's first, then the engine's, then each module's, in the order they come.
    list.sort_by_key(|s| match s.section {
        Section::Bento => 0,
        Section::Engine(_) => 1,
        Section::Module(_) => 2,
    });
    list
}

/// The keys of the user's own for `shortcut` taken away, from both files, with its `unbind`.
fn forget(doc: &mut Document, shortcut: &Shortcut, engine: bool) {
    let keys: Vec<Key> = doc
        .binds()
        .into_iter()
        .filter(|bind| {
            bind.action
                .as_ref()
                .and_then(|node| parse(node, engine))
                .is_some_and(|a| a == shortcut.action)
        })
        .filter_map(|bind| bind.key.parse().ok())
        .collect();
    for key in keys {
        doc.remove_bind(&key);
    }
    let off: Vec<ActionNode> = doc
        .unbinds()
        .into_iter()
        .filter(|node| parse(node, engine).is_some_and(|a| a == shortcut.action))
        .collect();
    for node in off {
        doc.remove_unbind(&node);
    }
}

/// What a change to `shortcut` does to the main config, and to the engine's file with `engine`.
#[derive(Debug, Clone)]
pub enum Change {
    /// Its keys become these binds; the defaults, with no properties, are written as nothing.
    Keys(Vec<BindNode>),
    /// It's turned off.
    Disable,
    /// It goes back to its defaults.
    Reset,
}

/// Applies `change` to `shortcut` in `doc`, the main config, or the engine's file with `engine`.
pub fn apply(doc: &mut Document, shortcut: &Shortcut, change: &Change, engine: bool) {
    forget(doc, shortcut, engine);
    let mine = (shortcut.file() == crate::store::File::Engine) == engine;
    if !mine {
        return;
    }
    match change {
        Change::Reset => (),
        Change::Disable => doc.add_unbind(&shortcut.node),
        Change::Keys(binds) => {
            let mut texts: Vec<&str> = binds.iter().map(|b| b.key.as_str()).collect();
            let mut defaults: Vec<&str> = shortcut.defaults.iter().map(String::as_str).collect();
            texts.sort_unstable();
            defaults.sort_unstable();
            let plain = binds.iter().all(|b| b.properties.is_empty());
            if binds.is_empty() && !shortcut.defaults.is_empty() {
                doc.add_unbind(&shortcut.node);
            } else if texts != defaults || !plain {
                for bind in binds {
                    if let Ok(key) = bind.key.parse::<Key>() {
                        doc.set_bind(&key, bind);
                    }
                }
            }
        }
    }
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
        let config = Config::parse_mem(main).unwrap();
        Sources {
            native: niri_config::actions::native_actions(),
            engine: Some((String::from("bento"), vec![decl("swap", &["Mod+X"])])),
            shell: vec![(
                String::from("start-menu"),
                decl("toggle-start-menu", &["Mod"]),
            )],
            main: Document::parse(main.to_owned()).unwrap(),
            engine_file: None,
            main_path: PathBuf::from("config.kdl"),
            engine_path: PathBuf::from("engines/bento.kdl"),
            binds: config.binds.0,
            bind_files: config.bind_files,
            unbound: config.unbound,
        }
    }

    fn find<'a>(list: &'a [Shortcut], node: &str) -> &'a Shortcut {
        list.iter().find(|s| s.node.text() == node).unwrap()
    }

    #[test]
    fn shortcuts_have_their_defaults_until_the_user_changes_them() {
        let main = "binds {\n    F1 { spawn \"alacritty\"; }\n    F2 { spawn \"alacritty\"; }\n    unbind { shell-action \"toggle-start-menu\"; }\n}\n";
        let list = shortcuts(&sources(main));
        // bento's default terminal, now on F1 and F2 instead of its own key.
        let terminal = find(&list, "spawn \"alacritty\"");
        assert_eq!(terminal.defaults, ["Mod+T"]);
        let keys: Vec<&str> = terminal.keys.iter().map(|k| k.text.as_str()).collect();
        assert_eq!(keys, ["F1", "F2"]);
        assert!(terminal.changed());
        // The engine's and the shell's, by their sections.
        let swap = find(&list, "swap");
        assert_eq!(swap.section, Section::Engine(String::from("bento")));
        assert_eq!(swap.keys[0].source, Source::Default);
        let start = find(&list, "shell-action \"toggle-start-menu\"");
        assert_eq!(start.section, Section::Module(String::from("start-menu")));
        assert!(start.disabled && start.keys.is_empty());
        // An action that takes no arguments is listed even unbound.
        let fullscreen = find(&list, "fullscreen-window");
        assert!(fullscreen.keys.iter().all(|k| k.source == Source::Default));
        assert!(!find(&list, "toggle-debug-tint").changed());
    }

    #[test]
    fn changing_a_shortcut_writes_only_what_differs_from_its_defaults() {
        let main = "binds {\n    F1 { spawn \"alacritty\"; }\n}\n";
        let src = sources(main);
        let list = shortcuts(&src);
        let terminal = find(&list, "spawn \"alacritty\"").clone();
        let mut doc = src.main;
        let bind = |key: &str| BindNode {
            key: String::from(key),
            properties: Vec::new(),
            action: Some(terminal.node.clone()),
        };
        // Back on its default key: nothing's written for it.
        apply(
            &mut doc,
            &terminal,
            &Change::Keys(vec![bind("Mod+T")]),
            false,
        );
        assert!(doc.binds().is_empty() && doc.unbinds().is_empty());
        // Off, then reset.
        apply(&mut doc, &terminal, &Change::Disable, false);
        assert_eq!(doc.unbinds(), std::slice::from_ref(&terminal.node));
        apply(&mut doc, &terminal, &Change::Reset, false);
        assert!(doc.unbinds().is_empty());
        // No keys at all is turning it off.
        apply(&mut doc, &terminal, &Change::Keys(Vec::new()), false);
        assert_eq!(doc.unbinds(), std::slice::from_ref(&terminal.node));
        // Two keys of its own.
        apply(
            &mut doc,
            &terminal,
            &Change::Keys(vec![bind("F1"), bind("F2")]),
            false,
        );
        let keys: Vec<String> = doc.binds().into_iter().map(|b| b.key).collect();
        assert_eq!(keys, ["F1", "F2"]);
        assert!(doc.unbinds().is_empty());
    }

    #[test]
    fn a_key_another_shortcut_takes_says_whose() {
        // The user's Mod+V for bento loses to the clipboard module's default.
        let mut src = sources("binds {\n    Mod+V { spawn \"paste\"; }\n}\n");
        src.shell.push((
            String::from("clipboard"),
            decl("toggle-clipboard", &["Mod+V"]),
        ));
        let list = shortcuts(&src);
        let paste = find(&list, "spawn \"paste\"");
        assert_eq!(paste.keys[0].taken, Some(Declarer::Shell));
        assert_eq!(
            find(&list, "shell-action \"toggle-clipboard\"").keys[0].taken,
            None
        );
    }
}
