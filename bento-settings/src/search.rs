//! Fuzzy search over every setting and page.
//!
//! Each word of the query has to match somewhere in an entry. In titles it can match as letters in
//! order, so "fcr" finds "Focus ring"; in keywords and context only as a substring, since letters
//! spread over long descriptions match nearly anything. Matches at the start of words and runs of
//! letters score higher, and the title counts for more than keywords and context.

use crate::model::{Content, Section, Setting};

pub struct Entry {
    /// Index of the category in sidebar order.
    pub category: usize,
    /// Where to jump on the page; `None` for the page itself.
    pub anchor: Option<String>,
    pub title: String,
    /// Shown under the title, like "Appearance › Focus ring".
    pub context: String,
    pub keywords: Vec<String>,
}

pub struct Hit {
    pub entry: usize,
    pub score: i32,
    /// Character positions in the title that matched, for bolding them.
    pub positions: Vec<usize>,
}

pub fn index(sections: &[Section]) -> Vec<Entry> {
    let mut entries = Vec::new();
    for (category_idx, category) in sections.iter().flat_map(|s| &s.categories).enumerate() {
        entries.push(Entry {
            category: category_idx,
            anchor: None,
            title: category.title.to_owned(),
            context: String::from("Page"),
            keywords: category.keywords.iter().map(|k| k.to_string()).collect(),
        });

        match &category.content {
            Content::Settings(groups) => {
                for group in groups {
                    let context = match group.title {
                        Some(title) => format!("{} › {title}", category.title),
                        None => category.title.to_owned(),
                    };
                    for setting in &group.settings {
                        index_setting(&mut entries, category_idx, &context, setting);
                    }
                }
            }
            Content::Custom { search, .. } => {
                for item in search {
                    entries.push(Entry {
                        category: category_idx,
                        anchor: Some(item.anchor.to_owned()),
                        title: item.title.to_owned(),
                        context: category.title.to_owned(),
                        keywords: item.keywords.iter().map(|k| k.to_string()).collect(),
                    });
                }
            }
            Content::Planned => (),
        }
    }
    entries
}

fn index_setting(entries: &mut Vec<Entry>, category: usize, context: &str, setting: &Setting) {
    let mut keywords: Vec<String> = setting.keywords.iter().map(|k| k.to_string()).collect();
    keywords.push(setting.path.replace('.', " "));
    entries.push(Entry {
        category,
        anchor: Some(setting.path.to_owned()),
        title: setting.title.to_owned(),
        context: context.to_owned(),
        keywords,
    });
    let context = format!("{context} › {}", setting.title);
    for child in &setting.children {
        index_setting(entries, category, &context, child);
    }
}

pub fn search(entries: &[Entry], query: &str) -> Vec<Hit> {
    let words: Vec<Vec<char>> = query
        .split_whitespace()
        .map(|word| word.to_lowercase().chars().collect())
        .collect();
    if words.is_empty() {
        return Vec::new();
    }

    let mut hits: Vec<Hit> = entries
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| {
            let title: Vec<char> = entry.title.to_lowercase().chars().collect();
            let context: Vec<char> = entry.context.to_lowercase().chars().collect();
            let keywords: Vec<Vec<char>> = entry
                .keywords
                .iter()
                .map(|k| k.to_lowercase().chars().collect())
                .collect();

            let mut total = 0;
            let mut positions = Vec::new();
            for word in &words {
                let in_title = score(word, &title, true).map(|(s, p)| (s * 3, Some(p)));
                let in_keywords = keywords
                    .iter()
                    .filter_map(|k| score(word, k, false))
                    .map(|(s, _)| (s * 2, None))
                    .max_by_key(|(s, _)| *s);
                let in_context = score(word, &context, false).map(|(s, _)| (s, None));
                let (best, title_positions) = [in_title, in_keywords, in_context]
                    .into_iter()
                    .flatten()
                    .max_by_key(|(s, _)| *s)?;
                total += best;
                positions.extend(title_positions.into_iter().flatten());
            }
            positions.sort_unstable();
            positions.dedup();
            Some(Hit {
                entry: idx,
                score: total,
                positions,
            })
        })
        .collect();

    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(entries[a.entry].category.cmp(&entries[b.entry].category))
            .then(a.entry.cmp(&b.entry))
    });
    hits
}

fn is_word_start(text: &[char], idx: usize) -> bool {
    idx == 0 || !text[idx - 1].is_alphanumeric()
}

/// Scores one lowercase word against one lowercase text. `spread` allows letters in order with
/// gaps between them.
pub fn score(word: &[char], text: &[char], spread: bool) -> Option<(i32, Vec<usize>)> {
    if word.is_empty() || word.len() > text.len() {
        return None;
    }

    // A substring beats letters spread out, and one at the start of a word beats one inside it.
    let substring = (0..=text.len() - word.len())
        .filter(|&start| text[start..start + word.len()] == *word)
        .max_by_key(|&start| (is_word_start(text, start), std::cmp::Reverse(start)));
    if let Some(start) = substring {
        let mut s = 10 * word.len() as i32;
        if is_word_start(text, start) {
            s += 20;
        }
        if start == 0 {
            s += 10;
        }
        return Some((s, (start..start + word.len()).collect()));
    }

    // Too few letters spread out match nearly anything.
    if !spread || word.len() < 3 {
        return None;
    }

    // Letters in order, preferring the starts of words.
    let mut positions = Vec::with_capacity(word.len());
    let mut s = 0;
    let mut from = 0;
    for &c in word {
        let next_start = (from..text.len()).find(|&i| text[i] == c && is_word_start(text, i));
        let next_any = (from..text.len()).find(|&i| text[i] == c)?;
        let idx = match (positions.last(), next_start) {
            (Some(&last), _) if next_any == last + 1 => next_any,
            (_, Some(start)) => start,
            _ => next_any,
        };
        s += 4;
        if is_word_start(text, idx) {
            s += 6;
        }
        if positions.last().is_some_and(|&last| idx == last + 1) {
            s += 3;
        }
        positions.push(idx);
        from = idx + 1;
    }
    let spread = positions.last().unwrap() - positions[0] + 1 - word.len();
    if spread > word.len() * 3 {
        return None;
    }
    Some((s - spread as i32, positions))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn score_of(word: &str, text: &str) -> Option<i32> {
        score(&chars(word), &chars(text), true).map(|(s, _)| s)
    }

    #[test]
    fn substring_at_word_start_ranks_first() {
        let start = score_of("ring", "focus ring").unwrap();
        let inside = score_of("ring", "spring").unwrap();
        assert!(start > inside);
    }

    #[test]
    fn letters_in_order_match() {
        let (_, positions) = score(&chars("fcr"), &chars("focus ring"), true).unwrap();
        assert_eq!(positions, vec![0, 2, 6]);
        assert!(score_of("fcr", "frc").is_none());
    }

    #[test]
    fn short_words_need_a_substring() {
        assert!(score_of("gp", "gaps").is_none());
        assert!(score_of("ga", "gaps").is_some());
    }

    #[test]
    fn spread_out_letters_dont_match() {
        assert!(score_of("gap", "go to a far place").is_none());
    }

    #[test]
    fn keywords_need_a_substring() {
        let entries = vec![Entry {
            category: 0,
            anchor: None,
            title: String::from("Mouse"),
            context: String::from("Page"),
            keywords: vec![String::from("Pointer speed and scrolling for mice.")],
        }];
        assert!(search(&entries, "foc").is_empty());
        assert_eq!(search(&entries, "scroll").len(), 1);
    }

    #[test]
    fn every_word_must_match() {
        let entries = vec![
            Entry {
                category: 0,
                anchor: None,
                title: String::from("Natural scrolling"),
                context: String::from("Touchpad › Scrolling"),
                keywords: Vec::new(),
            },
            Entry {
                category: 1,
                anchor: None,
                title: String::from("Natural scrolling"),
                context: String::from("Mouse › Scrolling"),
                keywords: Vec::new(),
            },
        ];
        let hits = search(&entries, "natural touch");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry, 0);
        assert_eq!(hits[0].positions, (0..7).collect::<Vec<_>>());
    }
}
