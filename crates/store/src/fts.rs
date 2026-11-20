//! Text helpers of the offline message search (the `messages_fts` table, migration v7).
//!
//! The index holds the plain text of a message (markup stripped, entities decoded) and the
//! sender's name, both folded with [`fold`] so Arabic spelling variants and tashkeel match. The
//! same functions run as SQL functions in the table's triggers, on the query and when a snippet
//! is turned back into the message's own spelling.

use rusqlite::Connection;
use rusqlite::functions::FunctionFlags;

/// Starts a match in [`crate::LocalHit::snippet`].
pub const MARK_START: char = '\u{2}';
/// Ends a match in [`crate::LocalHit::snippet`].
pub const MARK_END: char = '\u{3}';

/// Registers the SQL functions the triggers of `messages_fts` call. They must exist before the
/// migrations run (v7 backfills with them) and on every connection that writes messages.
pub(crate) fn register(conn: &Connection) -> rusqlite::Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8
        | FunctionFlags::SQLITE_DETERMINISTIC
        | FunctionFlags::SQLITE_INNOCUOUS;
    conn.create_scalar_function("kelma_fts_body", 1, flags, |ctx| {
        Ok(fold(&plain_text(&ctx.get::<String>(0)?)))
    })?;
    conn.create_scalar_function("kelma_fts_fold", 1, flags, |ctx| {
        Ok(fold(&ctx.get::<String>(0)?))
    })
}

/// Tags after which text continues on a new line (or cell), so their words must not run into
/// the next ones. Inline tags (`b`, `span`, `a`…) are dropped without a gap.
const BLOCK_TAGS: &[&str] = &[
    "p",
    "div",
    "br",
    "li",
    "ul",
    "ol",
    "tr",
    "td",
    "th",
    "table",
    "blockquote",
    "pre",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "img",
];

/// The text of a message's HTML: tags removed (`<style>` and `<script>` with their content),
/// entities decoded and whitespace collapsed to single spaces. A `<` that does not open a tag
/// (`a < b` in a plain text message) is kept.
pub(crate) fn plain_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find(['<', '&']) {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        if rest.starts_with('&') {
            let (c, used) = entity(rest);
            out.push(c);
            rest = &rest[used..];
            continue;
        }
        let opens = rest[1..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!');
        let Some(end) = rest.find('>').filter(|_| opens) else {
            out.push('<');
            rest = &rest[1..];
            continue;
        };
        let closing = rest[1..end].starts_with('/');
        let tag = rest[1..end].trim_start_matches('/');
        let name: String = tag
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        rest = &rest[end + 1..];
        if (name == "style" || name == "script") && !closing {
            // Skip the content up to the closing tag.
            let close = format!("</{name}");
            rest = match rest.to_ascii_lowercase().find(&close) {
                Some(j) => &rest[j..],
                None => "",
            };
            continue;
        }
        if BLOCK_TAGS.contains(&name.as_str()) {
            out.push(' ');
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The character of the entity at the start of `s` (which starts with `&`) and the bytes it
/// takes; an unknown or broken entity is a plain `&`.
fn entity(s: &str) -> (char, usize) {
    let Some(semi) = s.bytes().take(12).position(|b| b == b';') else {
        return ('&', 1);
    };
    let body = &s[1..semi];
    let c = match body {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => body
            .strip_prefix("#x")
            .or_else(|| body.strip_prefix("#X"))
            .and_then(|h| u32::from_str_radix(h, 16).ok())
            .or_else(|| body.strip_prefix('#').and_then(|d| d.parse().ok()))
            .and_then(char::from_u32),
    };
    match c {
        Some(c) => (c, semi + 1),
        None => ('&', 1),
    }
}

/// What a character becomes in the index: tashkeel, Quranic marks and tatweel are dropped, the
/// alef forms (أ إ آ ٱ) become ا, alef maksura (ى) and Farsi yeh become ي, and teh marbuta (ة)
/// becomes ه. Case and Latin accents are left to the tokenizer (`remove_diacritics 2`).
fn fold_char(c: char) -> Option<char> {
    match c {
        '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{06D6}'..='\u{06ED}' | '\u{0640}' => None,
        '\u{0622}' | '\u{0623}' | '\u{0625}' | '\u{0671}' => Some('\u{0627}'),
        '\u{0649}' | '\u{06CC}' => Some('\u{064A}'),
        '\u{0629}' => Some('\u{0647}'),
        c => Some(c),
    }
}

/// `text` as the index stores it, see [`fold_char`].
pub(crate) fn fold(text: &str) -> String {
    text.chars().filter_map(fold_char).collect()
}

/// [`fold`] plus where each folded character came from: the byte offset in `text` of every
/// output character, then `text.len()`. A dropped mark belongs to the character before it.
fn fold_mapped(text: &str) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(text.len());
    let mut from = Vec::with_capacity(text.len() + 1);
    for (i, c) in text.char_indices() {
        if let Some(f) = fold_char(c) {
            out.push(f);
            from.push(i);
        }
    }
    from.push(text.len());
    (out, from)
}

/// The FTS5 query for what the user typed, or `None` when it has no word. Each word becomes a
/// quoted string (so `"`, `*`, `-`, `:`, `(`, `NEAR`, `AND`, `OR` and `NOT` are text, not
/// syntax) with a prefix star, and the words are joined with an implicit AND.
pub(crate) fn match_query(user: &str) -> Option<String> {
    let terms: Vec<String> = fold(user)
        .split_whitespace()
        .filter(|t| t.chars().any(char::is_alphanumeric))
        .map(|t| format!("\"{}\"*", t.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// Puts the message's own spelling back into a snippet of the folded index text. `snippet` is a
/// piece of the folded text of `html` with match markers; the result has the same markers on
/// the original characters (tashkeel and alef forms included), with `…` where it was cut. When
/// the piece cannot be found the folded snippet is returned as is.
pub(crate) fn restore_snippet(snippet: &str, html: &str) -> String {
    let plain = plain_text(html);
    let (folded, from) = fold_mapped(&plain);
    let core: String = snippet
        .chars()
        .filter(|c| *c != MARK_START && *c != MARK_END)
        .collect();
    let Some(pos) = folded.find(&core) else {
        return snippet.to_string();
    };
    let mut k = folded[..pos].chars().count();
    let mut out = String::with_capacity(snippet.len() + 8);
    // Only words make a cut: punctuation the snippet left out at either end is put back.
    if folded[..pos].chars().any(char::is_alphanumeric) {
        out.push('…');
    } else {
        out.push_str(&plain[..from[k]]);
    }
    for c in snippet.chars() {
        if c == MARK_START || c == MARK_END {
            out.push(c);
        } else {
            out.push_str(&plain[from[k]..from[k + 1]]);
            k += 1;
        }
    }
    if folded[pos + core.len()..]
        .chars()
        .any(char::is_alphanumeric)
    {
        out.push('…');
    } else {
        out.push_str(&plain[from[k]..]);
    }
    out
}
