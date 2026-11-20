//! Snippets (saved quick replies): typing `/brb` at the start of a line in the composer offers the
//! snippets whose name fits, and picking one puts its text there. The functions here are pure, so
//! the composer re-evaluates them whenever the draft or the cursor changes (like `MentionApi`);
//! `install` loads the saved list from the config directory and wires adding, editing and deleting
//! (Settings). The list lives in `snippets.json`, per device (see `appearance::snippets`).

use crate::{AppWindow, Edited, SnippetApi, SnippetItem, SnippetStore};
use appearance::{Snippet, SnippetPrefs};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// Most snippets the picker shows.
const SHOWN: usize = 8;
/// Longest snippet name, in characters.
const MAX_NAME: usize = 32;

/// `cursor` (a byte offset from Slint) moved back to a character boundary inside `text`.
fn clamp(text: &str, cursor: i32) -> usize {
    let mut at = usize::try_from(cursor).unwrap_or(0).min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The `/query` being typed just before the cursor: the byte offset of the `/` and the query.
/// The `/` must be the first thing on its line (spaces and tabs may come before it), so a URL, a
/// path or a fraction in the middle of a line is left alone, and the query is one word without
/// another `/` (a path such as `/usr/bin` is not a snippet).
fn active(text: &str, cursor: i32) -> Option<(usize, &str)> {
    let before = &text[..clamp(text, cursor)];
    let start = before.rfind('/')?;
    let query = &before[start + 1..];
    let line = before[..start].rsplit('\n').next().unwrap_or("");
    (line.chars().all(|c| c == ' ' || c == '\t') && !query.contains(char::is_whitespace))
        .then_some((start, query))
}

/// The snippets that fit `query`, best first: names that start with it, then names that contain
/// it, each group in the order the user keeps them. An empty query fits all. Case does not matter.
fn rank<'a>(all: &'a [Snippet], query: &str) -> Vec<&'a Snippet> {
    let q = query.to_lowercase();
    let mut starts = Vec::new();
    let mut inside = Vec::new();
    for s in all {
        let name = s.name.to_lowercase();
        if name.starts_with(&q) {
            starts.push(s);
        } else if name.contains(&q) {
            inside.push(s);
        }
    }
    starts.extend(inside);
    starts
}

/// What `{first}`, `{me}` and `{date}` stand for.
#[derive(Debug, Default, PartialEq)]
struct Fill {
    first: String,
    me: String,
    date: String,
}

/// `text` with the known placeholders replaced. Other braces stay as typed.
fn expand(text: &str, fill: &Fill) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let tail = &rest[open + 1..];
        let value = tail.find('}').and_then(|close| match &tail[..close] {
            "first" => Some((&fill.first, close)),
            "me" => Some((&fill.me, close)),
            "date" => Some((&fill.date, close)),
            _ => None,
        });
        match value {
            Some((v, close)) => {
                out.push_str(v);
                rest = &tail[close + 1..];
            }
            None => {
                out.push('{');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The first name of a display name: "Sara Hassan" and "Hassan, Sara" both give "Sara".
fn first_name(person: &str) -> String {
    let p = person.trim();
    let given = match p.split_once(',') {
        Some((_, rest)) => rest.trim(),
        None => p,
    };
    given.split_whitespace().next().unwrap_or("").to_string()
}

/// Why a snippet name was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NameError {
    Empty,
    Spaces,
    Long,
    Taken,
}

impl NameError {
    fn code(self) -> &'static str {
        match self {
            NameError::Empty => "name-empty",
            NameError::Spaces => "name-spaces",
            NameError::Long => "name-long",
            NameError::Taken => "name-taken",
        }
    }
}

/// The name to store for what was typed: trimmed, without a leading `/`. It must be one word of
/// at most [`MAX_NAME`] characters and differ (ignoring case) from every other snippet's name.
/// `editing` is the name of the snippet being edited, which may keep its own name.
fn clean_name(typed: &str, all: &[Snippet], editing: &str) -> Result<String, NameError> {
    let name = typed.trim().trim_start_matches('/');
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.contains(|c: char| c.is_whitespace() || c == '/') {
        return Err(NameError::Spaces);
    }
    if name.chars().count() > MAX_NAME {
        return Err(NameError::Long);
    }
    let key = name.to_lowercase();
    let own = editing.to_lowercase();
    if all
        .iter()
        .any(|s| s.name.to_lowercase() == key && s.name.to_lowercase() != own)
    {
        return Err(NameError::Taken);
    }
    Ok(name.to_string())
}

/// `text` with the `/query` before the cursor replaced by `snippet`, the cursor after it.
fn complete(text: &str, cursor: i32, snippet: &str) -> Edited {
    let at = clamp(text, cursor);
    let from = active(text, cursor).map_or(at, |(start, _)| start);
    let mut out = String::with_capacity(text.len() + snippet.len());
    out.push_str(&text[..from]);
    out.push_str(snippet);
    out.push_str(&text[at..]);
    Edited {
        text: out.into(),
        cursor: (from + snippet.len()) as i32,
    }
}

/// The text on one line, for lists.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn item(s: &Snippet) -> SnippetItem {
    SnippetItem {
        name: s.name.as_str().into(),
        text: s.text.as_str().into(),
        preview: one_line(&s.text).into(),
    }
}

fn items(all: &[Snippet]) -> ModelRc<SnippetItem> {
    ModelRc::new(VecModel::from(all.iter().map(item).collect::<Vec<_>>()))
}

fn snippets_of(model: &ModelRc<SnippetItem>) -> Vec<Snippet> {
    model
        .iter()
        .map(|i| Snippet::new(&i.name, &i.text))
        .collect()
}

thread_local! {
    /// My display name, for `{me}`.
    static ME: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The other person's first name when the open chat is a 1:1 chat, else "".
fn chat_first_name(w: &AppWindow) -> String {
    let id = w.get_current_id();
    if id.is_empty() || w.get_current_is_channel() {
        return String::new();
    }
    let rows = w.get_chats_favorites();
    let main = w.get_chats_main();
    rows.iter()
        .chain(main.iter())
        .find(|c| c.id == id)
        .filter(|c| !c.group && !c.channel)
        .map(|c| first_name(&c.person))
        .unwrap_or_default()
}

fn today() -> String {
    appearance::formats::local_now()
        .map(|t| {
            t.format(appearance::formats::date_pattern(true))
                .to_string()
        })
        .unwrap_or_default()
}

/// Attaches the `/query` functions to a window. The snippets themselves come from [`install`].
pub fn wire(window: &AppWindow) {
    let api = window.global::<SnippetApi>();
    api.on_active(|text: SharedString, cursor| active(&text, cursor).is_some());
    api.on_matches(|text: SharedString, cursor, all: ModelRc<SnippetItem>| {
        match active(&text, cursor) {
            Some((_, q)) => {
                let all = snippets_of(&all);
                let hits: Vec<Snippet> = rank(&all, q).into_iter().take(SHOWN).cloned().collect();
                items(&hits)
            }
            None => ModelRc::default(),
        }
    });
    let weak = window.as_weak();
    api.on_complete(move |text: SharedString, cursor, snippet: SharedString| {
        let fill = Fill {
            first: weak
                .upgrade()
                .map(|w| chat_first_name(&w))
                .unwrap_or_default(),
            me: ME.with(|m| m.borrow().clone()),
            date: today(),
        };
        complete(&text, cursor, &expand(&snippet, &fill))
    });
}

struct State {
    dir: PathBuf,
    prefs: SnippetPrefs,
}

/// Loads the saved snippets from `dir` (the config directory) and wires saving and deleting from
/// Settings. `me` is my display name, for `{me}`.
pub fn install(window: &AppWindow, dir: PathBuf, me: Option<String>) {
    ME.with(|m| *m.borrow_mut() = me.unwrap_or_default());
    let prefs = SnippetPrefs::load_from(&dir.join(appearance::snippets::FILE_NAME));
    let store = window.global::<SnippetStore>();
    store.set_items(items(&prefs.effective()));
    let state = Rc::new(RefCell::new(State { dir, prefs }));
    let weak = window.as_weak();
    let st = state.clone();
    store.on_save(
        move |name: SharedString, text: SharedString, original: SharedString| {
            let mut s = st.borrow_mut();
            let mut all = s.prefs.effective();
            let name = match clean_name(&name, &all, &original) {
                Ok(n) => n,
                Err(e) => return e.code().into(),
            };
            if text.trim().is_empty() {
                return "text-empty".into();
            }
            let new = Snippet::new(&name, &text);
            match all.iter().position(|x| x.name == original.as_str()) {
                Some(i) => all[i] = new,
                None => all.push(new),
            }
            commit(&mut s, all, &weak);
            SharedString::new()
        },
    );
    let weak = window.as_weak();
    store.on_remove(move |name: SharedString| {
        let mut s = state.borrow_mut();
        let mut all = s.prefs.effective();
        all.retain(|x| x.name != name.as_str());
        commit(&mut s, all, &weak);
    });
}

/// [`install`] with the real config directory and the active account's name (`main`).
pub fn install_system(window: &AppWindow) {
    let Ok(dir) = appearance::config_dir() else {
        tracing::warn!("snippets: no config directory, the list stays empty");
        return;
    };
    let me = session::Paths::new()
        .and_then(|p| p.load_config())
        .ok()
        .and_then(|c| c.active_account().and_then(|a| a.display_name.clone()));
    install(window, dir, me);
}

/// Keeps `all` as the user's list: saves it and shows it.
fn commit(s: &mut State, all: Vec<Snippet>, weak: &slint::Weak<AppWindow>) {
    s.prefs.items = Some(all);
    if let Err(e) = s
        .prefs
        .save_to(&s.dir.join(appearance::snippets::FILE_NAME))
    {
        tracing::warn!(%e, "snippets: could not save");
    }
    if let Some(w) = weak.upgrade() {
        w.global::<SnippetStore>()
            .set_items(items(&s.prefs.effective()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_end(text: &str) -> Option<(usize, &str)> {
        active(text, text.len() as i32)
    }

    #[test]
    fn a_slash_opens_a_query_only_at_the_start_of_a_line() {
        assert_eq!(at_end("/"), Some((0, "")));
        assert_eq!(at_end("/brb"), Some((0, "brb")));
        assert_eq!(at_end("hi\n/br"), Some((3, "br")));
        assert_eq!(at_end("hi\n  /br"), Some((5, "br")), "indent is allowed");
        assert_eq!(at_end("\t/x"), Some((1, "x")));
        // Not at the start of a line.
        assert_eq!(at_end("hi /brb"), None);
        assert_eq!(at_end("see https://example.invalid/brb"), None, "a URL");
        assert_eq!(at_end("1/2"), None);
        // Not one word, or a path.
        assert_eq!(at_end("/brb now"), None);
        assert_eq!(at_end("/usr/bin"), None);
        assert_eq!(at_end("/a\nb"), None, "the query stays on one line");
        // The cursor decides, not the end of the text.
        assert_eq!(active("/brb later", 4), Some((0, "brb")));
        assert_eq!(active("/brb", 0), None, "cursor before the slash");
        assert_eq!(at_end("no slash"), None);
    }

    #[test]
    fn the_query_follows_multibyte_text() {
        let text = "مرحبا\n/شكرا";
        assert_eq!(at_end(text), Some(("مرحبا\n".len(), "شكرا")));
        // A cursor inside a character is moved back to its start.
        assert_eq!(active("/é", 2), Some((0, "")));
    }

    fn all() -> Vec<Snippet> {
        vec![
            Snippet::new("lunch-brb", "x"),
            Snippet::new("brb", "Be right back"),
            Snippet::new("BRBack", "y"),
            Snippet::new("omw", "On my way"),
        ]
    }

    #[test]
    fn rank_puts_prefix_matches_before_contains_and_ignores_case() {
        let a = all();
        let names = |q: &str| {
            rank(&a, q)
                .iter()
                .map(|s| s.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names("brb"), ["brb", "BRBack", "lunch-brb"]);
        assert_eq!(names("BR"), ["brb", "BRBack", "lunch-brb"]);
        assert_eq!(names("om"), ["omw"]);
        assert_eq!(names("zzz"), Vec::<String>::new());
        assert_eq!(names("").len(), 4, "an empty query lists all, in order");
        assert_eq!(names("")[0], "lunch-brb");
    }

    fn fill() -> Fill {
        Fill {
            first: "Sara".into(),
            me: "Omar Ali".into(),
            date: "11/20/26".into(),
        }
    }

    #[test]
    fn placeholders_are_filled_and_unknown_braces_stay() {
        let f = fill();
        assert_eq!(expand("Thanks, {first}!", &f), "Thanks, Sara!");
        assert_eq!(expand("{me} on {date}", &f), "Omar Ali on 11/20/26");
        assert_eq!(expand("{first}{first}", &f), "SaraSara");
        assert_eq!(expand("{name} {} { first } {", &f), "{name} {} { first } {");
        assert_eq!(expand("{{first}}", &f), "{Sara}");
        assert_eq!(expand("{first", &f), "{first");
        assert_eq!(expand("no braces", &f), "no braces");
        assert_eq!(expand("{First}", &f), "{First}", "names are lower case");
        // Empty values (a group chat) leave nothing behind.
        assert_eq!(expand("Hi {first}.", &Fill::default()), "Hi .");
        // Multi-byte text around the braces.
        assert_eq!(expand("شكرا {first}", &f), "شكرا Sara");
    }

    #[test]
    fn first_names() {
        assert_eq!(first_name("Sara Hassan"), "Sara");
        assert_eq!(first_name("Hassan, Sara"), "Sara");
        assert_eq!(first_name("  Sara "), "Sara");
        assert_eq!(first_name("سارة حسن"), "سارة");
        assert_eq!(first_name(""), "");
    }

    #[test]
    fn names_are_one_word_and_unique_ignoring_case() {
        let a = vec![Snippet::new("brb", "x"), Snippet::new("omw", "y")];
        assert_eq!(clean_name("  /ty ", &a, ""), Ok("ty".into()));
        assert_eq!(clean_name("", &a, ""), Err(NameError::Empty));
        assert_eq!(clean_name(" / ", &a, ""), Err(NameError::Empty));
        assert_eq!(clean_name("two words", &a, ""), Err(NameError::Spaces));
        assert_eq!(clean_name("a/b", &a, ""), Err(NameError::Spaces));
        assert_eq!(clean_name(&"x".repeat(33), &a, ""), Err(NameError::Long));
        assert!(clean_name(&"x".repeat(32), &a, "").is_ok());
        assert_eq!(clean_name("BRB", &a, ""), Err(NameError::Taken));
        // Editing keeps its own name (even in another case) but cannot take another's.
        assert_eq!(clean_name("BRB", &a, "brb"), Ok("BRB".into()));
        assert_eq!(clean_name("omw", &a, "brb"), Err(NameError::Taken));
    }

    #[test]
    fn completing_replaces_the_query_and_keeps_the_rest() {
        let e = complete("hi\n/br there", "hi\n/br".len() as i32, "Be right back");
        assert_eq!(e.text, "hi\nBe right back there");
        assert_eq!(e.cursor as usize, "hi\nBe right back".len());
        // Multi-line snippets and Arabic text: offsets are bytes.
        let e = complete("/ش", 3, "سطر\nثاني");
        assert_eq!(e.text, "سطر\nثاني");
        assert_eq!(e.cursor as usize, "سطر\nثاني".len());
        // No query before the cursor: it inserts.
        assert_eq!(complete("ab", 1, "X").text, "aXb");
    }
}
