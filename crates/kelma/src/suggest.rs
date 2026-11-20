//! What the top search box offers while the user types, and what `/goto`, `/chat` and `/call`
//! open: people, chats and channels of the local cache, matched by name. Pure data in, rows out;
//! the engine passes the cached conversations and shows the answer.

use crate::data::{color_for, initials};
use store::{ConvKind, ConversationRow};

/// One suggestion row.
#[derive(Debug, Clone, PartialEq)]
pub struct SuggestData {
    /// `person`, `chat` or `channel`.
    pub kind: &'static str,
    /// The conversation id.
    pub id: String,
    pub title: String,
    /// The team of a channel, a short word for the others.
    pub subtitle: String,
    pub initials: String,
    pub color: u32,
}

/// Suggestions per kind.
const PER_KIND: usize = 3;

/// "Team › Channel" for a channel, the plain title for the rest.
fn full_title(c: &ConversationRow) -> String {
    match (&c.kind, &c.team_name) {
        (ConvKind::Channel, Some(t)) if !t.is_empty() => format!("{t} › {}", c.title),
        _ => c.title.clone(),
    }
}

/// How well `c` matches the words of `query`: `None` when a word is missing, 0 for the exact
/// title, 1 for a title that starts with the query, 2 for the rest.
fn rank(c: &ConversationRow, query: &str) -> Option<u8> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    let title = c.title.to_lowercase();
    let hay = full_title(c).to_lowercase();
    if !q.split_whitespace().all(|w| hay.contains(w)) {
        return None;
    }
    Some(if title == q {
        0
    } else if title.starts_with(&q) {
        1
    } else {
        2
    })
}

fn kind_of(k: &ConvKind) -> Option<&'static str> {
    match k {
        ConvKind::OneOnOne => Some("person"),
        ConvKind::Group | ConvKind::Meeting => Some("chat"),
        ConvKind::Channel => Some("channel"),
        // Your own notes chat is not a suggestion.
        ConvKind::Notes => None,
    }
}

/// People, then chats, then channels whose name has every word of `query`, at most three of
/// each; better matches first, then the most recently active (`convs` is newest first).
pub fn suggest(convs: &[ConversationRow], query: &str) -> Vec<SuggestData> {
    let mut out = Vec::new();
    for kind in ["person", "chat", "channel"] {
        let mut found: Vec<(u8, &ConversationRow)> = convs
            .iter()
            .filter(|c| kind_of(&c.kind) == Some(kind))
            .filter_map(|c| rank(c, query).map(|r| (r, c)))
            .collect();
        // Stable: equal ranks keep the order of `convs`.
        found.sort_by_key(|(r, _)| *r);
        out.extend(found.into_iter().take(PER_KIND).map(|(_, c)| SuggestData {
            kind,
            id: c.id.clone(),
            title: full_title(c),
            subtitle: match kind {
                "person" => "Person".into(),
                "chat" => "Chat".into(),
                _ => "Channel".into(),
            },
            initials: initials(&c.title),
            color: color_for(&c.id),
        }));
    }
    out
}

/// What `/goto`, `/chat` and `/call` look for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A channel, by its name or its team's.
    Channel,
    /// A person or a group chat.
    Chat,
    /// A person: only one-to-one chats can be called.
    Person,
}

impl Target {
    pub fn parse(s: &str) -> Option<Target> {
        match s {
            "goto" => Some(Target::Channel),
            "chat" => Some(Target::Chat),
            "call" => Some(Target::Person),
            _ => None,
        }
    }

    fn takes(self, k: &ConvKind) -> bool {
        match self {
            Target::Channel => *k == ConvKind::Channel,
            Target::Chat => matches!(k, ConvKind::OneOnOne | ConvKind::Group | ConvKind::Meeting),
            Target::Person => *k == ConvKind::OneOnOne,
        }
    }
}

/// The conversation `name` means for `target`: the best match, the newest on a tie.
pub fn resolve<'a>(
    convs: &'a [ConversationRow],
    target: Target,
    name: &str,
) -> Option<&'a ConversationRow> {
    convs
        .iter()
        .filter(|c| target.takes(&c.kind))
        .filter_map(|c| rank(c, name).map(|r| (r, c)))
        .min_by_key(|(r, _)| *r)
        .map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(id: &str, kind: ConvKind, title: &str, team: Option<&str>) -> ConversationRow {
        ConversationRow {
            id: id.into(),
            kind,
            title: title.into(),
            team_id: team.map(|_| "t".to_string()),
            team_name: team.map(str::to_string),
            last_activity_ms: 1,
            last_preview: String::new(),
            unread: false,
            favorite: false,
            read_horizon_ms: 0,
        }
    }

    fn list() -> Vec<ConversationRow> {
        // Newest first, as the store returns them.
        vec![
            conv("c1", ConvKind::Group, "Release planning", None),
            conv("p1", ConvKind::OneOnOne, "Sara Hassan", None),
            conv("ch1", ConvKind::Channel, "General", Some("Platform")),
            conv("p2", ConvKind::OneOnOne, "Omar Saraf", None),
            conv("n1", ConvKind::Notes, "Sara notes", None),
            conv("ch2", ConvKind::Channel, "Releases", Some("Platform")),
        ]
    }

    #[test]
    fn people_chats_and_channels_come_in_that_order() {
        let r = suggest(&list(), "re");
        let kinds: Vec<_> = r.iter().map(|s| (s.kind, s.id.as_str())).collect();
        assert_eq!(kinds, [("chat", "c1"), ("channel", "ch2")]);
        let r = suggest(&list(), "sara");
        let ids: Vec<_> = r.iter().map(|s| s.id.as_str()).collect();
        // The notes chat is never suggested; a title that starts with the word comes first.
        assert_eq!(ids, ["p1", "p2"]);
    }

    #[test]
    fn a_channel_is_found_by_its_team_and_shows_both() {
        let r = suggest(&list(), "platform gen");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "Platform › General");
        assert_eq!(r[0].kind, "channel");
    }

    #[test]
    fn at_most_three_of_a_kind_and_nothing_for_an_empty_query() {
        let many: Vec<_> = (0..6)
            .map(|i| {
                conv(
                    &format!("p{i}"),
                    ConvKind::OneOnOne,
                    &format!("Ali {i}"),
                    None,
                )
            })
            .collect();
        assert_eq!(suggest(&many, "ali").len(), 3);
        assert!(suggest(&list(), "   ").is_empty());
        assert!(suggest(&list(), "zzz").is_empty());
    }

    #[test]
    fn an_exact_title_beats_a_newer_partial_match() {
        let mut v = vec![conv("a", ConvKind::OneOnOne, "Omar Khaled", None)];
        v.push(conv("b", ConvKind::OneOnOne, "Omar", None));
        assert_eq!(suggest(&v, "omar")[0].id, "b");
        assert_eq!(resolve(&v, Target::Person, "omar").unwrap().id, "b");
    }

    #[test]
    fn each_target_only_takes_its_own_kind() {
        let l = list();
        assert_eq!(resolve(&l, Target::Channel, "general").unwrap().id, "ch1");
        assert_eq!(resolve(&l, Target::Channel, "platform").unwrap().id, "ch1");
        assert!(resolve(&l, Target::Channel, "sara").is_none());
        assert_eq!(resolve(&l, Target::Chat, "release").unwrap().id, "c1");
        assert_eq!(resolve(&l, Target::Chat, "sara").unwrap().id, "p1");
        assert_eq!(resolve(&l, Target::Person, "sara").unwrap().id, "p1");
        assert!(
            resolve(&l, Target::Person, "release").is_none(),
            "a group cannot be called with /call"
        );
        assert!(resolve(&l, Target::Chat, "").is_none());
    }

    #[test]
    fn targets_parse_from_the_command_names() {
        assert_eq!(Target::parse("goto"), Some(Target::Channel));
        assert_eq!(Target::parse("chat"), Some(Target::Chat));
        assert_eq!(Target::parse("call"), Some(Target::Person));
        assert_eq!(Target::parse("x"), None);
    }
}
