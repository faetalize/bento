//! Keys while a search runs: the keyboard goes to the search's field instead of the apps.
//!
//! Text from the keyboard layout is typed at the caret, and Backspace, Delete, the arrow keys,
//! Home and End edit and move it, with Shift selecting. Ctrl+A selects everything, and Ctrl+C,
//! Ctrl+X and Ctrl+V use bento's clipboard. Enter ends the search with its best match, and Escape
//! cancels it. Every other key does nothing, and none of them reach apps.
//!
//! The binds that can be inhibited don't run, like when an app inhibits shortcuts, and the ones
//! that can't be inhibited do. [`State::search_intercept_key`] is where the key path in
//! `on_keyboard` hands a pressed key over, and `should_intercept_key` is told that shortcuts are
//! inhibited for as long as [`Niri::search_takes_keys`] says so.
//!
//! Input methods aren't supported: smithay's input method support only connects apps' text inputs
//! to the input method, and gives the compositor no way to take part as a text input itself, so
//! text that an input method composes can't reach the field.

use std::io::{self, Read as _};
use std::sync::Arc;
use std::time::Duration;

use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{Interest, Mode, PostAction};
use niri_config::Modifiers;
use smithay::backend::input::Keycode;
use smithay::input::keyboard::Keysym;
use smithay::reexports::rustix::fs::{fcntl_setfl, OFlags};
use smithay::wayland::selection::data_device::{
    current_data_device_selection_userdata, request_data_device_client_selection,
    set_data_device_selection,
};

use super::ShouldInterceptResult;
use crate::layout::search::Edit;
use crate::niri::{Niri, State};

/// The mime types that bento's clipboard offers text as.
const TEXT_MIME_TYPES: [&str; 3] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"];

/// Most bytes that bento reads when pasting, as many as a field holds.
const MAX_PASTE: usize = 4096;

/// What a key does to a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SearchInput {
    Edit(Edit),
    /// Ends the search with its best match.
    Submit,
    /// Ends the search.
    Cancel,
    Copy,
    Cut,
    Paste,
}

impl SearchInput {
    /// Whether holding the key repeats it.
    fn repeats(&self) -> bool {
        matches!(self, SearchInput::Edit(_))
    }
}

/// What the key that was pressed does to a search, or `None` if it does nothing. `modified` is
/// the key's keysym with the modifiers applied, and `raw` the keysyms that it has without them
/// on this layout and on another one with Latin letters, for shortcuts on any layout.
pub(super) fn search_input(
    modified: Option<Keysym>,
    raw: &[Keysym],
    mods: Modifiers,
) -> Option<SearchInput> {
    let modified = modified?;

    // Alt and Super are for shortcuts, which don't type or move the caret.
    if mods.intersects(Modifiers::ALT | Modifiers::SUPER) {
        return None;
    }

    if mods.contains(Modifiers::CTRL) {
        let letter = [modified]
            .iter()
            .chain(raw)
            .filter_map(|sym| sym.key_char())
            .find(char::is_ascii_alphabetic)?;
        return match letter.to_ascii_lowercase() {
            'a' => Some(SearchInput::Edit(Edit::SelectAll)),
            'c' => Some(SearchInput::Copy),
            'x' => Some(SearchInput::Cut),
            'v' => Some(SearchInput::Paste),
            _ => None,
        };
    }

    let select = mods.contains(Modifiers::SHIFT);
    let edit = match modified {
        Keysym::Return | Keysym::KP_Enter | Keysym::ISO_Enter => return Some(SearchInput::Submit),
        Keysym::Escape => return Some(SearchInput::Cancel),
        Keysym::BackSpace => Edit::Backspace,
        Keysym::Delete | Keysym::KP_Delete => Edit::Delete,
        Keysym::Left | Keysym::KP_Left => Edit::Left { select },
        Keysym::Right | Keysym::KP_Right => Edit::Right { select },
        Keysym::Home | Keysym::KP_Home => Edit::Home { select },
        Keysym::End | Keysym::KP_End => Edit::End { select },
        sym => {
            let typed = sym.key_char().filter(|c| !c.is_control())?;
            Edit::Insert(typed.to_string())
        }
    };
    Some(SearchInput::Edit(edit))
}

impl Niri {
    /// Whether the keyboard is the running search's: not while the session is locked, where the
    /// lock screen has it, or while the screenshot UI has it.
    pub fn search_takes_keys(&self) -> bool {
        self.layout.search_active() && !self.is_locked() && !self.screenshot_ui.is_open()
    }
}

impl State {
    /// Decides what a key does while a search takes the keys. `result` is what the key path
    /// decided for the key with shortcuts inhibited: a key that no bind takes is for the search,
    /// and a bind that can't be inhibited runs without the key reaching apps. Releases are left
    /// as they are, so that apps hear the release of a key that they heard pressed.
    pub(super) fn search_intercept_key(
        &mut self,
        result: ShouldInterceptResult,
        key_code: Keycode,
        pressed: bool,
        modified: Option<Keysym>,
        raw: &[Keysym],
        mods: Modifiers,
    ) -> ShouldInterceptResult {
        if !pressed {
            return result;
        }

        match result {
            ShouldInterceptResult::Forward => {
                self.niri.suppressed_keys.insert(key_code);
                if let Some(input) = search_input(modified, raw, mods) {
                    self.search_apply(input.clone());
                    if input.repeats() {
                        self.start_search_repeat(input);
                    }
                }
                ShouldInterceptResult::InterceptOnly
            }
            ShouldInterceptResult::ForwardAndHandle(bind) => {
                self.niri.suppressed_keys.insert(key_code);
                ShouldInterceptResult::InterceptAndHandle(bind)
            }
            result => result,
        }
    }

    fn search_apply(&mut self, input: SearchInput) {
        match input {
            SearchInput::Edit(edit) => self.niri.layout.search_edit(&edit),
            SearchInput::Submit => self.niri.layout.search_submit(),
            SearchInput::Cancel => self.niri.layout.search_cancel(),
            SearchInput::Copy => {
                self.search_copy();
            }
            SearchInput::Cut => {
                if self.search_copy() {
                    // With the selection, backspace removes only that.
                    self.niri.layout.search_edit(&Edit::Backspace);
                }
            }
            SearchInput::Paste => self.search_paste(),
        }
        self.niri.queue_redraw_all();
    }

    /// Repeats a key for as long as it's held, like apps do for the keys they get. This shares
    /// the timer of bind repeat, which stops when any key is released.
    fn start_search_repeat(&mut self, input: SearchInput) {
        if let Some(token) = self.niri.bind_repeat_timer.take() {
            self.niri.event_loop.remove(token);
        }

        let config = self.niri.config.borrow();
        let config = &config.input.keyboard;
        let repeat_rate = config.repeat_rate;
        if repeat_rate == 0 {
            return;
        }
        let repeat_duration = Duration::from_secs_f64(1. / f64::from(repeat_rate));
        let repeat_timer =
            Timer::from_duration(Duration::from_millis(u64::from(config.repeat_delay)));

        let token = self
            .niri
            .event_loop
            .insert_source(repeat_timer, move |_, _, state| {
                if !state.niri.search_takes_keys() {
                    return TimeoutAction::Drop;
                }
                state.search_apply(input.clone());
                TimeoutAction::ToDuration(repeat_duration)
            })
            .unwrap();

        self.niri.bind_repeat_timer = Some(token);
    }

    /// Puts the selected text in bento's clipboard. Returns whether there was any.
    fn search_copy(&mut self) -> bool {
        let Some(text) = self.niri.layout.search_selection() else {
            return false;
        };

        set_data_device_selection(
            &self.niri.display_handle,
            &self.niri.seat,
            TEXT_MIME_TYPES.map(String::from).to_vec(),
            Arc::from(text.into_bytes()),
        );
        true
    }

    /// Types the clipboard's text at the caret. What bento's clipboard holds is typed right away,
    /// and what an app holds is read from it as it sends it.
    fn search_paste(&mut self) {
        let own = current_data_device_selection_userdata(&self.niri.seat).map(|data| data.to_vec());
        if let Some(data) = own {
            // Compositor selections are bento's own, like screenshots, which aren't text.
            if let Ok(text) = String::from_utf8(data) {
                self.niri.layout.search_edit(&Edit::Insert(text));
            }
            return;
        }

        for mime_type in TEXT_MIME_TYPES {
            let (reader, writer) = match io::pipe() {
                Ok(pipe) => pipe,
                Err(err) => {
                    warn!("error making a pipe to paste into the search: {err:?}");
                    return;
                }
            };
            if request_data_device_client_selection(
                &self.niri.seat,
                mime_type.to_owned(),
                writer.into(),
            )
            .is_err()
            {
                continue;
            }

            self.read_pasted_text(reader);
            return;
        }
    }

    /// Reads the text that an app sends to paste, as it comes, and types it when it's all there.
    fn read_pasted_text(&mut self, reader: io::PipeReader) {
        if let Err(err) = fcntl_setfl(&reader, OFlags::NONBLOCK) {
            warn!("error setting the pasted text's pipe to not block: {err:?}");
            return;
        }

        let mut text = Vec::new();
        let source = Generic::new(reader, Interest::READ, Mode::Level);
        let res = self
            .niri
            .event_loop
            .insert_source(source, move |_, reader, state| {
                let mut buf = [0; 1024];
                loop {
                    match (&**reader).read(&mut buf) {
                        Ok(0) => {
                            // Text that doesn't fit in a field is cut short by the field.
                            text.truncate(MAX_PASTE);
                            if let Ok(text) = String::from_utf8(std::mem::take(&mut text)) {
                                state.niri.layout.search_edit(&Edit::Insert(text));
                                state.niri.queue_redraw_all();
                            }
                            return Ok(PostAction::Remove);
                        }
                        Ok(read) => {
                            if text.len() < MAX_PASTE {
                                text.extend_from_slice(&buf[..read]);
                            }
                        }
                        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                            return Ok(PostAction::Continue);
                        }
                        Err(err) if err.kind() == io::ErrorKind::Interrupted => (),
                        Err(err) => {
                            warn!("error reading the text to paste into the search: {err:?}");
                            return Ok(PostAction::Remove);
                        }
                    }
                }
            });
        if let Err(err) = res {
            warn!("error waiting for the text to paste into the search: {err:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(sym: Keysym, mods: Modifiers) -> Option<SearchInput> {
        search_input(Some(sym), &[], mods)
    }

    fn typed(text: &str) -> Option<SearchInput> {
        Some(SearchInput::Edit(Edit::Insert(text.to_owned())))
    }

    #[test]
    fn keys_with_text_type_it() {
        assert_eq!(input(Keysym::a, Modifiers::empty()), typed("a"));
        assert_eq!(input(Keysym::A, Modifiers::SHIFT), typed("A"));
        assert_eq!(input(Keysym::space, Modifiers::empty()), typed(" "));
        assert_eq!(input(Keysym::eacute, Modifiers::empty()), typed("é"));
        // AltGr and the like are part of typing.
        assert_eq!(input(Keysym::at, Modifiers::ISO_LEVEL3_SHIFT), typed("@"));
    }

    #[test]
    fn keys_without_text_do_nothing() {
        assert_eq!(input(Keysym::Shift_L, Modifiers::SHIFT), None);
        assert_eq!(input(Keysym::Tab, Modifiers::empty()), None);
        assert_eq!(input(Keysym::F5, Modifiers::empty()), None);
        assert_eq!(input(Keysym::NoSymbol, Modifiers::empty()), None);
        assert_eq!(search_input(None, &[], Modifiers::empty()), None);
    }

    #[test]
    fn enter_and_escape_end_the_search() {
        let none = Modifiers::empty();
        assert_eq!(input(Keysym::Return, none), Some(SearchInput::Submit));
        assert_eq!(input(Keysym::KP_Enter, none), Some(SearchInput::Submit));
        assert_eq!(input(Keysym::Escape, none), Some(SearchInput::Cancel));
    }

    #[test]
    fn editing_keys_edit_and_shift_selects() {
        let none = Modifiers::empty();
        let edit = |edit| Some(SearchInput::Edit(edit));
        assert_eq!(input(Keysym::BackSpace, none), edit(Edit::Backspace));
        assert_eq!(input(Keysym::Delete, none), edit(Edit::Delete));
        assert_eq!(
            input(Keysym::Left, none),
            edit(Edit::Left { select: false })
        );
        assert_eq!(
            input(Keysym::Left, Modifiers::SHIFT),
            edit(Edit::Left { select: true })
        );
        assert_eq!(
            input(Keysym::Right, Modifiers::SHIFT),
            edit(Edit::Right { select: true })
        );
        assert_eq!(
            input(Keysym::Home, none),
            edit(Edit::Home { select: false })
        );
        assert_eq!(
            input(Keysym::End, Modifiers::SHIFT),
            edit(Edit::End { select: true })
        );
    }

    #[test]
    fn ctrl_shortcuts_edit_and_use_the_clipboard() {
        let ctrl = Modifiers::CTRL;
        assert_eq!(
            input(Keysym::a, ctrl),
            Some(SearchInput::Edit(Edit::SelectAll))
        );
        assert_eq!(input(Keysym::c, ctrl), Some(SearchInput::Copy));
        assert_eq!(input(Keysym::x, ctrl), Some(SearchInput::Cut));
        assert_eq!(input(Keysym::v, ctrl), Some(SearchInput::Paste));
        // Shift doesn't change which shortcut it is.
        assert_eq!(
            input(Keysym::V, ctrl | Modifiers::SHIFT),
            Some(SearchInput::Paste)
        );
        // Other keys with Ctrl don't type.
        assert_eq!(input(Keysym::z, ctrl), None);
        assert_eq!(input(Keysym::Left, ctrl), None);
    }

    #[test]
    fn ctrl_shortcuts_work_on_layouts_without_latin_letters() {
        // The Cyrillic "с" is on the key that has "c" on a Latin layout.
        let ctrl = Modifiers::CTRL;
        assert_eq!(
            search_input(Some(Keysym::Cyrillic_es), &[Keysym::c], ctrl),
            Some(SearchInput::Copy)
        );
        assert_eq!(search_input(Some(Keysym::Cyrillic_es), &[], ctrl), None);
    }

    #[test]
    fn alt_and_super_are_for_shortcuts_which_dont_type() {
        assert_eq!(input(Keysym::a, Modifiers::ALT), None);
        assert_eq!(input(Keysym::a, Modifiers::SUPER), None);
        assert_eq!(input(Keysym::Left, Modifiers::ALT), None);
        assert_eq!(input(Keysym::Return, Modifiers::SUPER), None);
    }

    #[test]
    fn only_editing_repeats() {
        assert!(SearchInput::Edit(Edit::Backspace).repeats());
        assert!(!SearchInput::Submit.repeats());
        assert!(!SearchInput::Paste.repeats());
    }
}
