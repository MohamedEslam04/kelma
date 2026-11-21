//! UI-thread side of @mentions in the composer: finding the `@query` before the cursor, putting
//! the chosen name in its place, and removing a name token with one Backspace. These are pure, so
//! the composer re-evaluates them whenever the draft or the cursor changes (like `EmojiApi`).
//!
//! A mention is plain text `@Name` in the draft. Which `@Name` strings are real mentions is
//! tracked by the composer (a list of the names picked) and by the engine (who they are).

use crate::{AppWindow, Edited, MentionApi};
use slint::{ComponentHandle, SharedString};

/// Longest `@query` that still counts as a search (names have spaces, so it may hold some).
const MAX_QUERY: usize = 30;

/// `cursor` (a byte offset from Slint) moved back to a character boundary inside `text`.
fn clamp(text: &str, cursor: i32) -> usize {
    let mut at = usize::try_from(cursor).unwrap_or(0).min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The `@query` being typed just before the cursor: the byte offset of the `@` and the query. The
/// `@` must start a word (so an email address is not a mention) and the query stays on one line.
fn active(text: &str, cursor: i32) -> Option<(usize, &str)> {
    let at = clamp(text, cursor);
    let before = &text[..at];
    let start = before.rfind('@')?;
    let query = &before[start + 1..];
    let word_start = before[..start]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace);
    (word_start
        && !query.contains('\n')
        && query.chars().count() <= MAX_QUERY
        && query.matches(' ').count() <= 1)
        .then_some((start, query))
}

fn splice(text: &str, from: usize, to: usize, ins: &str) -> Edited {
    let mut out = String::with_capacity(text.len() + ins.len());
    out.push_str(&text[..from]);
    out.push_str(ins);
    out.push_str(&text[to..]);
    Edited {
        text: out.into(),
        cursor: (from + ins.len()) as i32,
    }
}

/// Replaces the `@query` before the cursor with `token` (for example `@Sara Hassan `).
fn complete(text: &str, cursor: i32, token: &str) -> Edited {
    let at = clamp(text, cursor);
    let from = active(text, cursor).map_or(at, |(start, _)| start);
    splice(text, from, at, token)
}

/// A Backspace right after a picked `@Name` (optionally followed by the space the picker adds)
/// removes the whole token. `names` is the picked names joined by newlines. The returned cursor
/// is -1 when the Backspace is an ordinary one.
fn backspace(text: &str, cursor: i32, names: &str) -> Edited {
    let at = clamp(text, cursor);
    let before = &text[..at];
    let mut names: Vec<&str> = names.lines().filter(|n| !n.is_empty()).collect();
    // The longest first, so "@Sara Hassan" wins over a shorter "@Sara".
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    for name in names {
        let token = format!("@{name}");
        let trimmed = before.strip_suffix(' ').unwrap_or(before);
        for head in [before, trimmed] {
            if let Some(start) = head.len().checked_sub(token.len())
                && head.ends_with(&token)
                && before[..start]
                    .chars()
                    .next_back()
                    .is_none_or(char::is_whitespace)
            {
                return splice(text, start, at, "");
            }
        }
    }
    Edited {
        text: text.into(),
        cursor: -1,
    }
}

/// Attaches the mention functions to a window.
pub fn wire(window: &AppWindow) {
    let api = window.global::<MentionApi>();
    api.on_active(|text: SharedString, cursor| active(&text, cursor).is_some());
    api.on_query(|text: SharedString, cursor| {
        active(&text, cursor).map_or(SharedString::new(), |(_, q)| q.into())
    });
    api.on_complete(|text: SharedString, cursor, token: SharedString| {
        complete(&text, cursor, &token)
    });
    api.on_backspace(|text: SharedString, cursor, names: SharedString| {
        backspace(&text, cursor, &names)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_starts_a_query_only_at_a_word_start() {
        assert_eq!(active("@", 1), Some((0, "")));
        assert_eq!(active("hi @sa", 6), Some((3, "sa")));
        assert_eq!(active("hi @sara ha", 11), Some((3, "sara ha")));
        assert_eq!(
            active("mail me@work", 12),
            None,
            "an email is not a mention"
        );
        assert_eq!(active("hi @sara", 3), None, "cursor before the @");
        assert_eq!(active("@a\nb", 4), None, "the query stays on one line");
        assert_eq!(active("@a b c", 6), None, "at most two words");
    }

    #[test]
    fn completing_replaces_the_query_and_keeps_the_rest() {
        let e = complete("hi @sa there", "hi @sa".len() as i32, "@Sara Hassan ");
        assert_eq!(e.text, "hi @Sara Hassan  there");
        assert_eq!(e.cursor as usize, "hi @Sara Hassan ".len());
        // Arabic names: offsets are bytes, not characters.
        let e = complete("مرحبا @سا", "مرحبا @سا".len() as i32, "@سارة ");
        assert_eq!(e.text, "مرحبا @سارة ");
    }

    #[test]
    fn one_backspace_removes_the_whole_name_token() {
        let names = "Sara Hassan\nSara";
        let text = "hi @Sara Hassan ";
        let e = backspace(text, text.len() as i32, names);
        assert_eq!(e.text, "hi ");
        assert_eq!(e.cursor, 3);
        // Right after the name, without the trailing space.
        let t2 = "hi @Sara Hassan";
        assert_eq!(backspace(t2, t2.len() as i32, names).text, "hi ");
        // The shorter name does not eat part of the longer one.
        let t3 = "@Sara";
        assert_eq!(backspace(t3, 5, names).text, "");
        // Text typed after the token is left alone.
        let t4 = "@Sara Hassan and more";
        assert_eq!(backspace(t4, t4.len() as i32, names).cursor, -1);
    }

    #[test]
    fn backspace_is_ordinary_when_no_picked_name_ends_at_the_cursor() {
        assert_eq!(backspace("hi @Sara", 8, "").cursor, -1);
        assert_eq!(backspace("hi @Omar", 8, "Sara").cursor, -1);
        // A name inside a longer word is not a token.
        assert_eq!(backspace("x@Sara", 6, "Sara").cursor, -1);
        // Middle of the text: the token before the cursor goes, the rest stays.
        let e = backspace("a @Sara b", 7, "Sara");
        assert_eq!(e.text, "a  b");
        assert_eq!(e.cursor, 2);
    }
}
