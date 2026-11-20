//! The set of binds: which action each chord runs.
//!
//! An action is what a bind runs, with its arguments; a bind is a chord that runs it, with
//! settings of its own. In the set a chord runs at most one action, and an action has any number
//! of binds. An action's default binds stay with whoever declares it: bento ships its own, and
//! engines and shell modules declare theirs. The user's config holds only what changed: binds of
//! one's own for an action, which replace its defaults, and actions turned off with `unbind`. When
//! binds of different declarers claim one chord, it goes to the narrowest declarer, a shell
//! module's, then the engine's, then bento's, since the narrower one is only there while it runs;
//! within one declarer, the user's binds come before its defaults.

use std::sync::OnceLock;

use niri_ipc::{LayoutEngineAction, LayoutEngineOptionValue};

use crate::binds::{Action, Bind, BoundAction, Key};
use crate::tiling::TilingOption;

/// Who declares an action, from the broadest to the narrowest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Declarer {
    /// bento itself, under every engine and shell.
    Bento,
    /// The running engine.
    Engine,
    /// A module of the running shell, or the shell itself.
    Shell,
}

/// Who declares `action`.
pub fn declarer(action: &Action) -> Declarer {
    match action {
        Action::LayoutAction(..) => Declarer::Engine,
        Action::ShellAction(..) => Declarer::Shell,
        _ => Declarer::Bento,
    }
}

/// The action `bind` runs: its action, or its press action when it has both.
pub fn action_of(bind: &Bind) -> Option<&Action> {
    bind.press_action().or(bind.release_action())
}

/// bento's own default binds, which ship with it.
pub fn bento_defaults() -> &'static [Bind] {
    static DEFAULTS: OnceLock<Vec<Bind>> = OnceLock::new();
    DEFAULTS.get_or_init(|| {
        crate::Config::parse_mem(DEFAULT_BINDS)
            .map(|config| config.binds.0)
            .expect("bento's default binds are valid")
    })
}

/// bento's own default binds, as written.
pub const DEFAULT_BINDS: &str = include_str!("../../resources/default-binds.kdl");

/// A bind `action` has by default on `key`, as a bind written in a config has it: run as it's
/// pressed, and again while it's held, or, for a key of modifiers alone, as they're let go with
/// nothing pressed in between, so they still start other binds.
pub fn default_bind(key: Key, action: Action) -> Bind {
    Bind {
        key,
        action: if key.trigger.is_modifier() {
            BoundAction::Release(action)
        } else {
            BoundAction::Press(action)
        },
        repeat: true,
        pass_through: None,
        cooldown: None,
        allow_when_locked: false,
        allow_inhibiting: true,
        hotkey_overlay_title: None,
    }
}

/// The binds the actions `declared` have by default, for an engine's or the shell's, which make
/// each action from its name and the arguments of its bind. A key that can't be read is left out,
/// and said.
pub fn declared_binds(
    declared: &[LayoutEngineAction],
    make: impl Fn(&str, &[LayoutEngineOptionValue]) -> Action,
) -> Vec<Bind> {
    let mut binds = Vec::new();
    for action in declared {
        for bind in &action.binds {
            match bind.key.parse::<Key>() {
                Ok(key) => binds.push(default_bind(key, make(&action.name, &bind.args))),
                Err(err) => warn!(
                    "the default key {:?} of {} isn't a key: {err}",
                    bind.key, action.name
                ),
            }
        }
    }
    binds
}

/// The engine's action `name`, with `args`, as a bind runs it.
pub fn engine_action(name: &str, args: &[LayoutEngineOptionValue]) -> Action {
    let args = args.iter().filter_map(tiling_option).collect();
    Action::LayoutAction(String::from(name), args, None)
}

/// The shell's action `name`, with `args`, as a bind runs it.
pub fn shell_action(name: &str, args: &[LayoutEngineOptionValue]) -> Action {
    let args = args
        .iter()
        .filter_map(|arg| match arg {
            LayoutEngineOptionValue::String(text) => Some(text.clone()),
            LayoutEngineOptionValue::Number(n) => Some(n.to_string()),
            LayoutEngineOptionValue::Boolean(b) => Some(b.to_string()),
            _ => None,
        })
        .collect();
    Action::ShellAction(String::from(name), args)
}

fn tiling_option(value: &LayoutEngineOptionValue) -> Option<TilingOption> {
    match value {
        LayoutEngineOptionValue::Boolean(b) => Some(TilingOption::Bool(*b)),
        LayoutEngineOptionValue::Number(n) => Some(TilingOption::Number(*n)),
        LayoutEngineOptionValue::String(s) => Some(TilingOption::String(s.clone())),
        LayoutEngineOptionValue::Gradient(g) => Some(TilingOption::Gradient(g.clone())),
        LayoutEngineOptionValue::List(_) => None,
    }
}

/// Which bind each key runs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolved {
    /// The binds in use, the narrowest declarer's first.
    pub in_use: Vec<Bind>,
    /// The binds whose keys another took.
    pub taken_over: Vec<TakenOver>,
}

/// A bind whose key another took.
#[derive(Debug, Clone, PartialEq)]
pub struct TakenOver {
    pub bind: Bind,
    /// Whether it's a default, rather than the user's.
    pub default: bool,
    /// Who declares the action that took its chord.
    pub by: Declarer,
}

/// Which bind each chord runs, from the user's binds, the actions the user turned off, and the
/// defaults of whoever declares their actions.
pub fn resolve(user: &[Bind], unbound: &[Action], defaults: &[Bind]) -> Resolved {
    let mut resolved = Resolved::default();
    let of = |binds: &'_ [Bind], declarer_: Declarer| -> Vec<Bind> {
        binds
            .iter()
            .filter(|bind| action_of(bind).map(declarer) == Some(declarer_))
            .cloned()
            .collect()
    };
    for layer in [Declarer::Shell, Declarer::Engine, Declarer::Bento] {
        let users = of(user, layer);
        // Binds of the user's own for an action replace all of its defaults.
        let changed: Vec<&Action> = users.iter().filter_map(action_of).collect();
        let kept = of(defaults, layer).into_iter().filter(|bind| {
            action_of(bind)
                .is_some_and(|action| !changed.contains(&action) && !unbound.contains(action))
        });
        // The user's later binds come before earlier ones, as a later bind replaces an earlier
        // one in a config, then the defaults.
        let layer_binds = users
            .iter()
            .rev()
            .cloned()
            .map(|bind| (bind, false))
            .chain(kept.map(|bind| (bind, true)));
        for (bind, default) in layer_binds {
            let taker = resolved
                .in_use
                .iter()
                .find(|other| other.key.conflicts_with(&bind.key));
            match taker {
                Some(taker) => {
                    let by = action_of(taker).map_or(Declarer::Bento, declarer);
                    resolved.taken_over.push(TakenOver { bind, default, by });
                }
                None => resolved.in_use.push(bind),
            }
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind(key: &str, action: Action) -> Bind {
        default_bind(key.parse().unwrap(), action)
    }

    fn spawn(what: &str) -> Action {
        Action::Spawn(vec![String::from(what)])
    }

    fn engine(name: &str) -> Action {
        Action::LayoutAction(String::from(name), Vec::new(), None)
    }

    fn shell(name: &str) -> Action {
        Action::ShellAction(String::from(name), Vec::new())
    }

    fn keys(binds: &[Bind]) -> Vec<String> {
        binds
            .iter()
            .map(|b| format!("{:?}", b.key.trigger))
            .collect()
    }

    #[test]
    fn an_action_has_its_default_binds_until_the_user_gives_it_binds_or_turns_it_off() {
        let defaults = [
            bind("Mod+T", spawn("alacritty")),
            bind("Mod+Shift+P", Action::PowerOffMonitors),
            bind("Mod+Q", Action::CloseWindow),
        ];
        // Two keys of one's own for the terminal replace its default, and one is turned off.
        let user = [
            bind("F1", spawn("alacritty")),
            bind("F2", spawn("alacritty")),
        ];
        let resolved = resolve(&user, &[Action::PowerOffMonitors], &defaults);
        let in_use: Vec<(Key, Action)> = resolved
            .in_use
            .iter()
            .map(|b| (b.key, action_of(b).unwrap().clone()))
            .collect();
        assert_eq!(
            in_use,
            [
                ("F2".parse().unwrap(), spawn("alacritty")),
                ("F1".parse().unwrap(), spawn("alacritty")),
                ("Mod+Q".parse().unwrap(), Action::CloseWindow),
            ]
        );
        assert!(resolved.taken_over.is_empty());
    }

    #[test]
    fn a_key_claimed_twice_goes_to_the_narrowest_declarer() {
        // bento's default Mod+V, the engine's default Mod+W, and the user's own Mod+W for bento.
        let defaults = [
            bind("Mod+V", Action::ToggleWindowFloating),
            bind("Mod+W", engine("group")),
            bind("Mod+V", shell("toggle-clipboard")),
        ];
        let user = [bind("Mod+W", spawn("browser"))];
        let resolved = resolve(&user, &[], &defaults);
        let in_use: Vec<Action> = resolved
            .in_use
            .iter()
            .map(|b| action_of(b).unwrap().clone())
            .collect();
        assert_eq!(in_use, [shell("toggle-clipboard"), engine("group")]);
        let taken: Vec<(Action, bool, Declarer)> = resolved
            .taken_over
            .iter()
            .map(|t| (action_of(&t.bind).unwrap().clone(), t.default, t.by))
            .collect();
        assert_eq!(
            taken,
            [
                (spawn("browser"), false, Declarer::Engine),
                (Action::ToggleWindowFloating, true, Declarer::Shell),
            ]
        );
    }

    #[test]
    fn within_one_declarer_the_users_bind_comes_before_a_default() {
        let defaults = [bind("Mod+V", Action::ToggleWindowFloating)];
        let user = [bind("Mod+V", spawn("paste"))];
        let resolved = resolve(&user, &[], &defaults);
        assert_eq!(keys(&resolved.in_use).len(), 1);
        assert_eq!(action_of(&resolved.in_use[0]), Some(&spawn("paste")));
        assert!(resolved.taken_over[0].default);
    }

    #[test]
    fn modifiers_alone_run_their_default_as_they_are_let_go() {
        let start = bind("Mod", shell("toggle-start-menu"));
        assert_eq!(start.release_action(), Some(&shell("toggle-start-menu")));
        assert_eq!(start.press_action(), None);
    }

    #[test]
    fn declared_defaults_carry_their_arguments() {
        let declared = [LayoutEngineAction {
            name: String::from("move-to-workspace"),
            description: None,
            args: Vec::new(),
            binds: vec![niri_ipc::LayoutEngineBind {
                key: String::from("Mod+Ctrl+Shift+1"),
                args: vec![LayoutEngineOptionValue::Number(1.)],
            }],
        }];
        let binds = declared_binds(&declared, engine_action);
        assert_eq!(
            action_of(&binds[0]),
            Some(&Action::LayoutAction(
                String::from("move-to-workspace"),
                vec![TilingOption::Number(1.)],
                None
            ))
        );
    }

    #[test]
    fn bentos_defaults_bind_only_its_own_actions() {
        let defaults = bento_defaults();
        assert!(defaults.len() > 50);
        assert!(defaults
            .iter()
            .all(|b| action_of(b).map(declarer) == Some(Declarer::Bento)));
    }
}
