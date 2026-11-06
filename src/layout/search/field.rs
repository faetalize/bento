//! The text that a search field holds, with its caret and selection, and how typing changes them.
//!
//! The caret and the selection's other end are byte offsets in the text, always between grapheme
//! clusters, so that one key press moves over what a person sees as one character.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation as _;

/// Most bytes a query can have, so that pasting a huge text can't stall bento.
const MAX_LEN: usize = 1024;

/// What typing does to a field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// Types text, which replaces the selection.
    Insert(String),
    Backspace,
    Delete,
    /// Moves the caret, and with `select` keeps the selection's other end where it is.
    Left {
        select: bool,
    },
    Right {
        select: bool,
    },
    Home {
        select: bool,
    },
    End {
        select: bool,
    },
    SelectAll,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Field {
    text: String,
    /// Where the caret is.
    caret: usize,
    /// The selection's other end: the same as `caret` when nothing is selected.
    anchor: usize,
}

impl Field {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selection's other end, which is the caret when nothing is selected.
    pub fn anchor(&self) -> usize {
        self.anchor
    }

    /// The selected bytes, empty at the caret when nothing is selected.
    pub fn selection(&self) -> Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    pub fn selected_text(&self) -> Option<&str> {
        let selection = self.selection();
        (!selection.is_empty()).then(|| &self.text[selection])
    }

    pub fn apply(&mut self, edit: &Edit) {
        match edit {
            Edit::Insert(text) => self.insert(text),
            Edit::Backspace => self.backspace(),
            Edit::Delete => self.delete(),
            Edit::Left { select } => self.step(false, *select),
            Edit::Right { select } => self.step(true, *select),
            Edit::Home { select } => self.move_to(0, *select),
            Edit::End { select } => self.move_to(self.text.len(), *select),
            Edit::SelectAll => {
                self.anchor = 0;
                self.caret = self.text.len();
            }
        }
    }

    /// Types `text` in place of the selection. Control characters, which a field can't show, are
    /// left out, and so is what doesn't fit; typing nothing changes nothing.
    fn insert(&mut self, text: &str) {
        let selection = self.selection();
        let room = MAX_LEN.saturating_sub(self.text.len() - selection.len());
        let mut typed = String::new();
        for c in text.chars().filter(|c| !c.is_control()) {
            if typed.len() + c.len_utf8() > room {
                break;
            }
            typed.push(c);
        }
        if typed.is_empty() {
            return;
        }

        self.text.replace_range(selection.clone(), &typed);
        self.caret = selection.start + typed.len();
        self.anchor = self.caret;
    }

    fn backspace(&mut self) {
        if self.selection().is_empty() {
            self.anchor = self.left_of(self.caret);
        }
        self.remove_selection();
    }

    fn delete(&mut self) {
        if self.selection().is_empty() {
            self.anchor = self.right_of(self.caret);
        }
        self.remove_selection();
    }

    fn remove_selection(&mut self) {
        let selection = self.selection();
        self.text.replace_range(selection.clone(), "");
        self.caret = selection.start;
        self.anchor = self.caret;
    }

    /// Moves the caret one character, or with a selection that isn't being extended, to its edge
    /// in that direction.
    fn step(&mut self, forward: bool, select: bool) {
        let selection = self.selection();
        let to = match (select || selection.is_empty(), forward) {
            (false, true) => selection.end,
            (false, false) => selection.start,
            (true, true) => self.right_of(self.caret),
            (true, false) => self.left_of(self.caret),
        };
        self.move_to(to, select);
    }

    /// Moves the caret to `to`, and the selection's other end too unless `select`.
    fn move_to(&mut self, to: usize, select: bool) {
        self.caret = to;
        if !select {
            self.anchor = to;
        }
    }

    /// The grapheme boundary before `at`, or 0.
    fn left_of(&self, at: usize) -> usize {
        self.text[..at]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(start, _)| start)
    }

    /// The grapheme boundary after `at`, or the end of the text.
    fn right_of(&self, at: usize) -> usize {
        self.text[at..]
            .graphemes(true)
            .next()
            .map_or(self.text.len(), |cluster| at + cluster.len())
    }

    /// The length in bytes of each grapheme cluster of the text, for screen readers.
    pub fn cluster_lengths(&self) -> Vec<usize> {
        self.text.graphemes(true).map(str::len).collect()
    }

    /// The number of grapheme clusters before the byte offset `at`, for screen readers.
    pub fn cluster_index(&self, at: usize) -> usize {
        self.text[..at].graphemes(true).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Field {
        let mut field = Field::default();
        field.apply(&Edit::Insert(text.to_owned()));
        field
    }

    fn state(field: &Field) -> (&str, usize, usize) {
        (field.text(), field.caret(), field.anchor())
    }

    #[test]
    fn typing_inserts_at_the_caret() {
        let mut field = typed("fire");
        assert_eq!(state(&field), ("fire", 4, 4));

        field.apply(&Edit::Left { select: false });
        field.apply(&Edit::Left { select: false });
        field.apply(&Edit::Insert("x".into()));
        assert_eq!(state(&field), ("fixre", 3, 3));
    }

    #[test]
    fn backspace_and_delete_remove_around_the_caret() {
        let mut field = typed("abcd");
        field.apply(&Edit::Left { select: false });
        field.apply(&Edit::Backspace);
        assert_eq!(state(&field), ("abd", 2, 2));
        field.apply(&Edit::Delete);
        assert_eq!(state(&field), ("ab", 2, 2));

        // Nothing is left to remove at the ends.
        field.apply(&Edit::Delete);
        assert_eq!(state(&field), ("ab", 2, 2));
        field.apply(&Edit::Home { select: false });
        field.apply(&Edit::Backspace);
        assert_eq!(state(&field), ("ab", 0, 0));
    }

    #[test]
    fn the_caret_moves_over_whole_characters() {
        // "e" with a combining accent is two code points and one character, and the flag is two
        // regional indicators.
        let mut field = typed("e\u{301}🇫🇷!");
        field.apply(&Edit::Left { select: false });
        field.apply(&Edit::Left { select: false });
        assert_eq!(field.caret(), "e\u{301}".len());
        field.apply(&Edit::Left { select: false });
        assert_eq!(field.caret(), 0);
        field.apply(&Edit::Right { select: false });
        assert_eq!(field.caret(), "e\u{301}".len());

        field.apply(&Edit::Right { select: false });
        field.apply(&Edit::Backspace);
        assert_eq!(field.text(), "e\u{301}!");
        assert_eq!(field.cluster_lengths(), [3, 1]);
    }

    #[test]
    fn screen_readers_count_characters_not_bytes() {
        let mut field = typed("e\u{301}🇫🇷!");
        assert_eq!(field.cluster_lengths(), [3, 8, 1]);
        assert_eq!(field.cluster_index(field.caret()), 3);
        field.apply(&Edit::Left { select: false });
        field.apply(&Edit::Left { select: false });
        assert_eq!(field.cluster_index(field.caret()), 1);
        assert_eq!(field.cluster_index(0), 0);
    }

    #[test]
    fn shift_extends_the_selection() {
        let mut field = typed("abcd");
        field.apply(&Edit::Left { select: true });
        field.apply(&Edit::Left { select: true });
        assert_eq!(state(&field), ("abcd", 2, 4));
        assert_eq!(field.selection(), 2..4);
        assert_eq!(field.selected_text(), Some("cd"));

        // The other end stays when the caret goes back.
        field.apply(&Edit::Right { select: true });
        assert_eq!(field.selected_text(), Some("d"));

        field.apply(&Edit::Home { select: true });
        assert_eq!(state(&field), ("abcd", 0, 4));
        assert_eq!(field.selected_text(), Some("abcd"));
    }

    #[test]
    fn moving_without_shift_collapses_the_selection_to_its_edge() {
        let mut field = typed("abcd");
        field.apply(&Edit::SelectAll);
        field.apply(&Edit::Left { select: false });
        assert_eq!(state(&field), ("abcd", 0, 0));

        field.apply(&Edit::SelectAll);
        field.apply(&Edit::Right { select: false });
        assert_eq!(state(&field), ("abcd", 4, 4));

        field.apply(&Edit::Home { select: true });
        field.apply(&Edit::End { select: false });
        assert_eq!(state(&field), ("abcd", 4, 4));
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut field = typed("abcd");
        field.apply(&Edit::Left { select: false });
        field.apply(&Edit::Left { select: true });
        field.apply(&Edit::Left { select: true });
        field.apply(&Edit::Insert("X".into()));
        assert_eq!(state(&field), ("aXd", 2, 2));

        field.apply(&Edit::SelectAll);
        field.apply(&Edit::Backspace);
        assert_eq!(state(&field), ("", 0, 0));

        let mut field = typed("abcd");
        field.apply(&Edit::SelectAll);
        field.apply(&Edit::Delete);
        assert_eq!(field.text(), "");
    }

    #[test]
    fn control_characters_are_left_out_of_what_is_typed() {
        let field = typed("a\nb\tc\u{7}");
        assert_eq!(field.text(), "abc");

        // Typing only those changes nothing, not even the selection.
        let mut field = typed("abc");
        field.apply(&Edit::SelectAll);
        field.apply(&Edit::Insert("\n".into()));
        assert_eq!(state(&field), ("abc", 3, 0));
    }

    #[test]
    fn a_field_holds_a_bounded_amount() {
        let mut field = typed(&"a".repeat(MAX_LEN - 1));
        field.apply(&Edit::Insert("éé".into()));
        // The second é doesn't fit, so the text stops at the first.
        assert_eq!(field.text().len(), MAX_LEN - 1);

        field.apply(&Edit::Insert("b".into()));
        assert_eq!(field.text().len(), MAX_LEN);
        field.apply(&Edit::Insert("c".into()));
        assert_eq!(field.text().len(), MAX_LEN);
    }
}
