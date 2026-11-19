//! `kelma-cli search <query> [--kind messages|files|people|events] [--page N]`: global search
//! across messages, files, people and calendar events (read-only).

use anyhow::{Result, bail};
use teams_proto::Client;
use teams_proto::global_search::{
    EventHit, FileHit, HitRef, MessageHit, Paging, PersonHit, SearchKind, SearchOptions,
    SearchResults, Snippet,
};

const USAGE: &str = "usage: search <query> [--kind messages|files|people|events] [--page N]";
const PAGE_SIZE: u32 = 10;

#[derive(Debug, PartialEq, Eq)]
struct Args {
    query: String,
    kind: Option<SearchKind>,
    page: u32,
}

fn parse_args(args: &[String]) -> Result<Args> {
    let (mut words, mut kind, mut page) = (vec![], None, 1u32);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--kind" => {
                let k = it.next().map(String::as_str).unwrap_or_default();
                match SearchKind::from_name(k) {
                    Some(k) => kind = Some(k),
                    None => bail!("unknown kind {k:?}; {USAGE}"),
                }
            }
            "--page" => match it.next().and_then(|p| p.parse::<u32>().ok()) {
                Some(p) if p >= 1 => page = p,
                _ => bail!("--page takes a number from 1; {USAGE}"),
            },
            w => words.push(w),
        }
    }
    if words.is_empty() {
        bail!(USAGE);
    }
    Ok(Args {
        query: words.join(" "),
        kind,
        page,
    })
}

/// The snippet as one line with the matching parts in `[[ ]]`.
fn marked(s: &Snippet) -> String {
    let mut out = String::new();
    let mut at = 0;
    for r in &s.highlights {
        if r.start < at || r.end > s.text.len() {
            continue;
        }
        out.push_str(&s.text[at..r.start]);
        out.push_str("[[");
        out.push_str(&s.text[r.clone()]);
        out.push_str("]]");
        at = r.end;
    }
    out.push_str(&s.text[at..]);
    out.replace('\n', " ")
}

/// `YYYY-MM-DD HH:MM` (UTC) for Unix milliseconds, `-` when unknown.
fn when(ms: Option<i64>) -> String {
    let Some(ms) = ms else { return "-".into() };
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        tod / 3600,
        (tod / 60) % 60
    )
}

fn message_line(m: &MessageHit) -> String {
    format!(
        "{}  {}  {}  [{}]  {}",
        when(m.time_ms),
        m.sender.name.as_deref().unwrap_or("-"),
        marked(&m.snippet),
        m.conversation_id,
        m.message_id
    )
}

fn file_line(f: &FileHit) -> String {
    format!(
        "{}  {}  {}  {}",
        f.name,
        when(f.modified_ms),
        f.author.as_deref().unwrap_or("-"),
        f.url.as_deref().unwrap_or("-")
    )
}

fn person_line(p: &PersonHit) -> String {
    format!(
        "{}  {}  {}  {}",
        p.mri,
        p.name,
        p.email.as_deref().unwrap_or("-"),
        p.title.as_deref().unwrap_or("-")
    )
}

fn event_line(e: &EventHit) -> String {
    format!(
        "{}  {}  {} -> {}  {}{}",
        e.id.chars().take(12).collect::<String>(),
        e.subject,
        when(e.start_ms),
        when(e.end_ms),
        e.organizer.as_deref().unwrap_or("-"),
        if e.cancelled { "  (cancelled)" } else { "" }
    )
}

fn hit_line(r: &SearchResults, h: HitRef) -> String {
    match h.kind {
        SearchKind::Messages => message_line(&r.messages[h.index]),
        SearchKind::Files => file_line(&r.files[h.index]),
        SearchKind::People => person_line(&r.people[h.index]),
        SearchKind::Events => event_line(&r.events[h.index]),
    }
}

fn print(r: &SearchResults, kinds: &[SearchKind], top: bool) {
    if top {
        println!("top results:");
        for h in r.top_results(5) {
            println!("  {:<8} {}", h.kind.name(), hit_line(r, h));
        }
    }
    for kind in kinds {
        let total = r
            .total_of(*kind)
            .map_or(String::new(), |t| format!(" of {t}"));
        let more = r
            .more
            .get(*kind)
            .map_or(String::new(), |n| format!("  (more from {n})"));
        println!("{}: {}{total}{more}", kind.name(), r.len_of(*kind));
        for i in 0..r.len_of(*kind) {
            println!(
                "  {}",
                hit_line(
                    r,
                    HitRef {
                        kind: *kind,
                        index: i
                    }
                )
            );
        }
    }
}

/// `--shape`: how the service answered, as counts only (entity sets, result sets, totals).
fn print_shape(v: &serde_json::Value) {
    let n =
        |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_array()).map_or(0, Vec::len);
    for set in v["EntitySets"].as_array().into_iter().flatten() {
        println!("{}:", set["EntityType"].as_str().unwrap_or("?"));
        for rs in set["ResultSets"].as_array().into_iter().flatten() {
            println!(
                "  result set: {} results, total {}, more {}, provenance {}",
                n(rs, "Results"),
                rs["Total"],
                rs["MoreResultsAvailable"],
                rs["Provenance"].as_str().unwrap_or("-")
            );
        }
    }
}

pub async fn run(client: &Client, me: &str, args: &[String]) -> Result<()> {
    let shape = args.iter().any(|a| a == "--shape");
    let args: Vec<String> = args.iter().filter(|a| *a != "--shape").cloned().collect();
    let a = parse_args(&args)?;
    let kinds: Vec<SearchKind> = a.kind.map_or(SearchKind::ALL.to_vec(), |k| vec![k]);
    let mut opts = SearchOptions {
        kinds: kinds.clone(),
        page_size: PAGE_SIZE,
        from: Paging::default(),
    };
    if shape {
        let v = client.global_search_json(me, &a.query, &opts).await?;
        print_shape(&v);
        return Ok(());
    }
    // Pages are reached by following each page's paging tokens, because the service may return
    // more or fewer hits than asked for, and a hit can come again on a later page (top
    // results are pulled forward), so hits seen on earlier pages are dropped.
    let mut seen = SearchResults::default();
    let mut results = client.global_search(me, &a.query, &opts).await?;
    for n in 2..=a.page {
        if !results.more.any() {
            println!("page {n}: no more results");
            return Ok(());
        }
        opts.from = results.more;
        let mut next = client.global_search(me, &a.query, &opts).await?;
        seen.append(results);
        next.dedup_against(&seen);
        results = next;
    }
    print(&results, &kinds, a.kind.is_none() && a.page == 1);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn parses_query_kind_and_page() {
        assert_eq!(
            parse_args(&v(&["quarterly", "report"])).unwrap(),
            Args {
                query: "quarterly report".into(),
                kind: None,
                page: 1
            }
        );
        let a = parse_args(&v(&["--kind", "files", "plan", "--page", "3"])).unwrap();
        assert_eq!(
            a,
            Args {
                query: "plan".into(),
                kind: Some(SearchKind::Files),
                page: 3
            }
        );
    }

    #[test]
    fn rejects_bad_arguments() {
        assert!(parse_args(&v(&[])).is_err());
        assert!(parse_args(&v(&["--kind", "files"])).is_err());
        assert!(parse_args(&v(&["x", "--kind", "chats"])).is_err());
        assert!(parse_args(&v(&["x", "--page", "0"])).is_err());
        assert!(parse_args(&v(&["x", "--page"])).is_err());
    }

    #[test]
    fn marks_highlights() {
        let s = Snippet {
            text: "weekly meeting notes".into(),
            highlights: std::iter::once(7..14).collect(),
        };
        assert_eq!(marked(&s), "weekly [[meeting]] notes");
    }

    #[test]
    fn formats_time() {
        assert_eq!(when(Some(0)), "1970-01-01 00:00");
        assert_eq!(when(Some(1_760_600_000_000)), "2025-10-16 07:33");
        assert_eq!(when(None), "-");
    }
}
