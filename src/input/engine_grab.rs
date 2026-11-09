//! The layout engine's hold on the keyboard, see `Bento.grabKeyboard`.
//!
//! While the engine holds the keyboard, the keyboard filter counts it as inhibiting shortcuts:
//! binds that can be inhibited don't run, and their keys, like every key that has no bind, go to
//! the engine and not to the apps. Binds with `allow-inhibiting=false` keep running. The apps have
//! no keyboard focus meanwhile, see `State::update_keyboard_focus`.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use calloop::timer::{TimeoutAction, Timer};
use niri_config::Bind;
use smithay::backend::input::Keycode;
use smithay::input::keyboard::xkb::keysym_get_name;
use smithay::input::keyboard::{Keysym, KeysymHandle, ModifiersState};

use super::ShouldInterceptResult;
use crate::layout::tiling::host::EngineKey;
use crate::niri::State;

/// Decides where a key goes, given what the keyboard filter made of it with the engine's hold
/// counted as inhibiting shortcuts: the filter's result, and whether the engine gets the key's
/// press or release.
///
/// The engine gets the presses of keys that no bind took, and the releases of those keys, even if
/// it lost the keyboard in between: the apps never saw them pressed, so they don't see them
/// released.
pub(super) fn route(
    result: ShouldInterceptResult,
    grabbed: bool,
    pressed: bool,
    key_code: Keycode,
    engine_keys: &mut HashSet<Keycode>,
    pending_release_binds: &HashMap<Keycode, Bind>,
) -> (ShouldInterceptResult, bool) {
    if !pressed {
        if matches!(result, ShouldInterceptResult::Forward) && engine_keys.remove(&key_code) {
            return (ShouldInterceptResult::InterceptOnly, true);
        }
        return (result, false);
    }
    if !grabbed {
        return (result, false);
    }

    match result {
        // A release-only bind that can't be inhibited has recorded its release, so it isn't ours.
        ShouldInterceptResult::Forward if !pending_release_binds.contains_key(&key_code) => {
            engine_keys.insert(key_code);
            (ShouldInterceptResult::InterceptOnly, true)
        }
        // The apps get nothing meanwhile, not even from a bind that passes keys through.
        ShouldInterceptResult::ForwardAndHandle(bind) => {
            (ShouldInterceptResult::InterceptAndHandle(bind), false)
        }
        result => (result, false),
    }
}

/// What a key types, without control characters: keys like Return and Escape, and keys with Ctrl
/// held, type control codes that aren't text an engine could show.
fn printable(text: String) -> String {
    if text.chars().any(char::is_control) {
        String::new()
    } else {
        text
    }
}

/// What the key types in the current keyboard state, and whether it repeats while held.
pub(super) fn key_info(keysym: &KeysymHandle<'_>) -> (String, bool) {
    let key_code = keysym.raw_code();
    let xkb = keysym.xkb().lock().unwrap();

    // SAFETY: the state and the keymap don't outlive xkb.
    unsafe {
        let text = xkb.state().key_get_utf8(key_code);
        let repeats = xkb.keymap().key_repeats(key_code);
        (printable(text), repeats)
    }
}

/// A key as the engine gets it, with the modifiers held.
pub(super) fn engine_key(
    key: Keysym,
    text: String,
    mods: &ModifiersState,
    pressed: bool,
) -> EngineKey {
    EngineKey {
        name: keysym_get_name(key),
        text,
        modifiers: [mods.shift, mods.ctrl, mods.alt, mods.logo],
        pressed,
        repeat: false,
    }
}

impl State {
    /// Gives a key to the engine, which holds the keyboard. A key that `repeats` is sent again
    /// while it's held, like apps do for the keys they get.
    pub(super) fn send_engine_key(&mut self, key: EngineKey, repeats: bool) {
        if key.pressed {
            if let Some(token) = self.niri.engine_key_repeat_timer.take() {
                self.niri.event_loop.remove(token);
            }
            if repeats {
                self.start_engine_key_repeat(key.clone());
            }
        }
        self.niri.deliver_held_key(key);

        // FIXME: granular.
        self.niri.queue_redraw_all();
    }

    fn start_engine_key_repeat(&mut self, key: EngineKey) {
        let config = self.niri.config.borrow();
        let config = &config.input.keyboard;

        let repeat_rate = config.repeat_rate;
        if repeat_rate == 0 {
            return;
        }
        let repeat_duration = Duration::from_secs_f64(1. / f64::from(repeat_rate));
        let timer = Timer::from_duration(Duration::from_millis(u64::from(config.repeat_delay)));

        let key = EngineKey {
            repeat: true,
            ..key
        };
        let token = self
            .niri
            .event_loop
            .insert_source(timer, move |_, _, state| {
                if !state.niri.keyboard_held() {
                    state.niri.engine_key_repeat_timer = None;
                    return TimeoutAction::Drop;
                }
                state.niri.deliver_held_key(key.clone());
                state.niri.queue_redraw_all();
                TimeoutAction::ToDuration(repeat_duration)
            })
            .unwrap();
        self.niri.engine_key_repeat_timer = Some(token);
    }

    /// Tells the engine the modifiers that are held now, if they changed.
    pub(super) fn update_engine_modifiers(&mut self) {
        let mods = self.niri.seat.get_keyboard().unwrap().modifier_state();
        self.niri
            .layout
            .set_modifiers([mods.shift, mods.ctrl, mods.alt, mods.logo]);

        // FIXME: granular.
        self.niri.queue_redraw_all();
    }
}

#[cfg(test)]
mod tests {
    use calloop::RegistrationToken;
    use niri_config::{Action, BoundAction, Key, ModKey, Modifiers, Trigger};

    use super::super::should_intercept_key;
    use super::*;

    /// The state that the keyboard filter and the engine's hold keep about keys.
    #[derive(Default)]
    struct Keys {
        suppressed_keys: HashSet<Keycode>,
        pending_release_binds: HashMap<Keycode, Bind>,
        engine_keys: HashSet<Keycode>,
        cooldown_timers: HashMap<Key, RegistrationToken>,
    }

    impl Keys {
        /// A key with no modifiers held, as the keyboard filter treats it while the engine holds
        /// the keyboard if `grabbed`, and where it goes.
        fn key(
            &mut self,
            binds: &[Bind],
            keysym: Keysym,
            pressed: bool,
            grabbed: bool,
        ) -> (ShouldInterceptResult, bool) {
            let key_code = Keycode::new(keysym.raw());
            let result = should_intercept_key(
                &mut self.suppressed_keys,
                &mut self.pending_release_binds,
                &self.cooldown_timers,
                binds.iter(),
                ModKey::Super,
                key_code,
                Some(keysym),
                &[],
                pressed,
                Modifiers::empty(),
                false,
                grabbed,
                false,
                false,
            );
            route(
                result,
                grabbed,
                pressed,
                key_code,
                &mut self.engine_keys,
                &self.pending_release_binds,
            )
        }
    }

    fn bind(keysym: Keysym, action: BoundAction, allow_inhibiting: bool) -> Bind {
        Bind {
            key: Key {
                trigger: Trigger::Keysym(keysym),
                modifiers: Modifiers::empty(),
            },
            action,
            repeat: false,
            pass_through: None,
            cooldown: None,
            allow_when_locked: false,
            allow_inhibiting,
            hotkey_overlay_title: None,
        }
    }

    fn press(action: Action) -> BoundAction {
        BoundAction::Press(action)
    }

    #[test]
    fn keys_without_a_bind_go_to_the_engine_and_not_to_the_apps() {
        let mut keys = Keys::default();
        let (result, to_engine) = keys.key(&[], Keysym::a, true, true);
        assert!(matches!(result, ShouldInterceptResult::InterceptOnly));
        assert!(to_engine);

        let (result, to_engine) = keys.key(&[], Keysym::a, false, true);
        assert!(matches!(result, ShouldInterceptResult::InterceptOnly));
        assert!(to_engine);
        assert!(keys.engine_keys.is_empty());
    }

    #[test]
    fn escape_goes_to_the_engine_like_any_key() {
        let mut keys = Keys::default();
        let (_, to_engine) = keys.key(&[], Keysym::Escape, true, true);
        assert!(to_engine);
    }

    #[test]
    fn keys_go_to_the_apps_when_the_engine_doesnt_hold_the_keyboard() {
        let mut keys = Keys::default();
        let (result, to_engine) = keys.key(&[], Keysym::a, true, false);
        assert!(matches!(result, ShouldInterceptResult::Forward));
        assert!(!to_engine);
        let (result, to_engine) = keys.key(&[], Keysym::a, false, false);
        assert!(matches!(result, ShouldInterceptResult::Forward));
        assert!(!to_engine);
    }

    #[test]
    fn binds_that_can_be_inhibited_dont_run_and_their_keys_go_to_the_engine() {
        let mut keys = Keys::default();
        let binds = [bind(Keysym::q, press(Action::CloseWindow), true)];

        let (result, to_engine) = keys.key(&binds, Keysym::q, true, true);
        assert!(matches!(result, ShouldInterceptResult::InterceptOnly));
        assert!(to_engine);

        // Without the engine holding the keyboard, the bind runs.
        let mut keys = Keys::default();
        let (result, to_engine) = keys.key(&binds, Keysym::q, true, false);
        assert!(matches!(
            result,
            ShouldInterceptResult::InterceptAndHandle(_)
        ));
        assert!(!to_engine);
    }

    #[test]
    fn binds_that_cant_be_inhibited_keep_running_and_dont_reach_the_engine() {
        let mut keys = Keys::default();
        let binds = [bind(Keysym::q, press(Action::CloseWindow), false)];

        let (result, to_engine) = keys.key(&binds, Keysym::q, true, true);
        assert!(matches!(
            result,
            ShouldInterceptResult::InterceptAndHandle(_)
        ));
        assert!(!to_engine);

        // Its release isn't the engine's either.
        let (_, to_engine) = keys.key(&binds, Keysym::q, false, true);
        assert!(!to_engine);
        assert!(keys.engine_keys.is_empty());
    }

    #[test]
    fn a_pass_through_bind_that_cant_be_inhibited_doesnt_pass_keys_to_the_apps() {
        let mut keys = Keys::default();
        let mut pass = bind(Keysym::q, press(Action::CloseWindow), false);
        pass.pass_through = Some(true);

        let (result, to_engine) = keys.key(&[pass], Keysym::q, true, true);
        assert!(matches!(
            result,
            ShouldInterceptResult::InterceptAndHandle(_)
        ));
        assert!(!to_engine);
    }

    #[test]
    fn a_release_bind_that_cant_be_inhibited_keeps_its_key_from_the_engine() {
        let mut keys = Keys::default();
        let binds = [bind(
            Keysym::q,
            BoundAction::Release(Action::CloseWindow),
            false,
        )];

        let (_, to_engine) = keys.key(&binds, Keysym::q, true, true);
        assert!(!to_engine);
        assert!(keys
            .pending_release_binds
            .contains_key(&Keycode::new(Keysym::q.raw())));

        let (result, to_engine) = keys.key(&binds, Keysym::q, false, true);
        assert!(matches!(result, ShouldInterceptResult::ForwardAndHandle(_)));
        assert!(!to_engine);
    }

    #[test]
    fn a_key_pressed_before_the_grab_is_released_to_the_apps() {
        let mut keys = Keys::default();
        keys.key(&[], Keysym::a, true, false);
        let (result, to_engine) = keys.key(&[], Keysym::a, false, true);
        assert!(matches!(result, ShouldInterceptResult::Forward));
        assert!(!to_engine);
    }

    #[test]
    fn a_key_pressed_in_the_grab_is_released_to_nobody_after_it_ends() {
        let mut keys = Keys::default();
        keys.key(&[], Keysym::Escape, true, true);

        // The grab ended on that Escape, and the apps never saw it pressed.
        let (result, to_engine) = keys.key(&[], Keysym::Escape, false, false);
        assert!(matches!(result, ShouldInterceptResult::InterceptOnly));
        assert!(to_engine);
        assert!(keys.engine_keys.is_empty());

        // After that, keys go to the apps again.
        let (result, to_engine) = keys.key(&[], Keysym::a, true, false);
        assert!(matches!(result, ShouldInterceptResult::Forward));
        assert!(!to_engine);
    }

    #[test]
    fn text_has_no_control_characters() {
        for typed in ["a", "A", " ", "é", "ß", "!"] {
            assert_eq!(printable(typed.to_owned()), typed);
        }
        for control in ["\r", "\u{1b}", "\u{8}", "\t", "\u{7f}", "\u{1}"] {
            assert_eq!(printable(control.to_owned()), "");
        }
        assert_eq!(printable(String::new()), "");
    }

    #[test]
    fn keys_are_named_with_the_modifiers_applied() {
        let mods = ModifiersState {
            shift: true,
            ctrl: false,
            alt: true,
            logo: false,
            ..Default::default()
        };
        let key = engine_key(Keysym::A, String::from("A"), &mods, true);
        assert_eq!(key.name, "A");
        assert_eq!(key.modifiers, [true, false, true, false]);
        assert!(key.pressed && !key.repeat);
        assert_eq!(
            engine_key(Keysym::Return, String::new(), &mods, false).name,
            "Return"
        );
        assert_eq!(
            engine_key(Keysym::Left, String::new(), &mods, false).name,
            "Left"
        );
    }
}
