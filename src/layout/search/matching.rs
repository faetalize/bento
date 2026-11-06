//! Matching a query against windows' titles and app ids.
//!
//! A window's rank is its best rank over its title and its app id. The ranks, from the best:
//!
//! - [`Rank::Word`]: the query is made of whole words of the text, like `fire` in `"fire fox"`;
//! - [`Rank::Prefix`]: the text, or one of its words, starts with the query, like `fire` in
//!   `"firefox"` or `"mozilla firefox"`;
//! - [`Rank::Substring`]: the query is somewhere in the text, like `fox` in `"firefox"`;
//! - [`Rank::Letters`]: the query's letters are in the text in order, with other letters between
//!   them, like `frfx` in `"firefox"`.
//!
//! Matching ignores case, and a window that has no rank doesn't match. A query that's empty or only
//! whitespace matches every window with [`Rank::Any`].

use super::Hit;

/// How well a window matches a query. A greater rank is a better match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rank {
    /// There's no query, so every window matches.
    Any = 0,
    Letters = 1,
    Substring = 2,
    Prefix = 3,
    Word = 4,
}

/// What a query is matched against.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a> {
    pub window: u64,
    pub title: Option<&'a str>,
    pub app_id: Option<&'a str>,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// How `query` matches `text`, both already lowercase and `query` not empty.
fn rank_text(query: &str, text: &str) -> Option<Rank> {
    let mut best = None;
    for (start, found) in text.match_indices(query) {
        let before = text[..start].chars().next_back();
        let after = text[start + found.len()..].chars().next();
        let word_start = !before.is_some_and(is_word_char);
        let word_end = !after.is_some_and(is_word_char);

        let rank = if word_start && word_end {
            Rank::Word
        } else if word_start {
            Rank::Prefix
        } else {
            Rank::Substring
        };
        best = best.max(Some(rank));
        if best == Some(Rank::Word) {
            return best;
        }
    }
    if best.is_some() {
        return best;
    }

    // The letters of the query, with no whitespace, in order.
    let mut letters = query.chars().filter(|c| !c.is_whitespace()).peekable();
    for c in text.chars() {
        if letters.peek() == Some(&c) {
            letters.next();
        }
    }
    letters.peek().is_none().then_some(Rank::Letters)
}

/// How `query` matches a window with this title and app id, or `None` if it doesn't.
pub fn rank(query: &str, title: Option<&str>, app_id: Option<&str>) -> Option<Rank> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(Rank::Any);
    }

    [title, app_id]
        .into_iter()
        .flatten()
        .filter_map(|text| rank_text(&query, &text.to_lowercase()))
        .max()
}

/// The candidates that match `query`, in the order they're given, with how well they match.
pub fn matching(query: &str, candidates: &[Candidate]) -> Vec<Hit> {
    candidates
        .iter()
        .filter_map(|candidate| {
            Some(Hit {
                window: candidate.window,
                rank: rank(query, candidate.title, candidate.app_id)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rank_of(query: &str, title: &str) -> Option<Rank> {
        rank(query, Some(title), None)
    }

    #[test]
    fn no_query_matches_everything() {
        assert_eq!(rank("", None, None), Some(Rank::Any));
        assert_eq!(rank("   ", Some("anything"), None), Some(Rank::Any));
    }

    #[test]
    fn ranks_go_from_whole_words_to_letters() {
        assert_eq!(rank_of("fire", "fire fox"), Some(Rank::Word));
        assert_eq!(rank_of("fire", "mozilla fire"), Some(Rank::Word));
        assert_eq!(rank_of("fire", "firefox"), Some(Rank::Prefix));
        assert_eq!(rank_of("fire", "mozilla firefox"), Some(Rank::Prefix));
        assert_eq!(rank_of("fox", "firefox"), Some(Rank::Substring));
        assert_eq!(rank_of("frfx", "firefox"), Some(Rank::Letters));
        assert_eq!(rank_of("xyz", "firefox"), None);
        assert!(Rank::Word > Rank::Prefix);
        assert!(Rank::Prefix > Rank::Substring);
        assert!(Rank::Substring > Rank::Letters);
        assert!(Rank::Letters > Rank::Any);
    }

    #[test]
    fn matching_ignores_case() {
        assert_eq!(rank_of("FIRE", "fire fox"), Some(Rank::Word));
        assert_eq!(rank_of("fire", "FIRE FOX"), Some(Rank::Word));
        assert_eq!(rank_of("ÉCOLE", "école"), Some(Rank::Word));
    }

    #[test]
    fn a_whole_word_is_found_even_after_a_weaker_match() {
        // The first "term" is only a substring of "terminal", but the second is a word.
        assert_eq!(rank_of("term", "terminal term"), Some(Rank::Word));
        assert_eq!(rank_of("term", "determine terminal"), Some(Rank::Prefix));
    }

    #[test]
    fn words_are_cut_at_anything_that_isnt_a_letter_or_digit() {
        assert_eq!(rank_of("notes", "notes.txt - editor"), Some(Rank::Word));
        assert_eq!(rank_of("txt", "notes.txt"), Some(Rank::Word));
        assert_eq!(rank_of("note", "notes.txt"), Some(Rank::Prefix));
        assert_eq!(rank_of("2", "window 2"), Some(Rank::Word));
        assert_eq!(rank_of("2", "window 22"), Some(Rank::Prefix));
    }

    #[test]
    fn a_window_ranks_as_its_best_field() {
        assert_eq!(
            rank("term", Some("a terminal"), Some("term")),
            Some(Rank::Word)
        );
        assert_eq!(
            rank("term", Some("a terminal"), Some("xterm")),
            Some(Rank::Prefix)
        );
        assert_eq!(rank("term", None, Some("xterm")), Some(Rank::Substring));
        assert_eq!(rank("term", None, None), None);
    }

    #[test]
    fn letters_need_to_be_in_order() {
        assert_eq!(rank_of("fxfr", "firefox"), None);
        assert_eq!(rank_of("ffx", "firefox"), Some(Rank::Letters));
        // Spaces in the query don't have to be in the text.
        assert_eq!(rank_of("f x", "firefox"), Some(Rank::Letters));
    }

    #[test]
    fn matching_drops_windows_and_keeps_their_order() {
        let candidates = [
            Candidate {
                window: 1,
                title: Some("firefox"),
                app_id: None,
            },
            Candidate {
                window: 2,
                title: Some("terminal"),
                app_id: None,
            },
            Candidate {
                window: 3,
                title: Some("fire fox"),
                app_id: None,
            },
        ];
        let hits = matching("fire", &candidates);
        assert_eq!(
            hits,
            [
                Hit {
                    window: 1,
                    rank: Rank::Prefix
                },
                Hit {
                    window: 3,
                    rank: Rank::Word
                },
            ]
        );

        let all = matching("", &candidates);
        assert_eq!(all.len(), 3);
        assert!(all.iter().all(|hit| hit.rank == Rank::Any));
    }
}
