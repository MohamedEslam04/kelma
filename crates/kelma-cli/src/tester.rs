//! `kelma-cli calendar tester <step> …`: live checks of the organizer actions (plan items C2.1,
//! C2.2 and C2.5) with one consenting tester. Each step is one command, so the tester can act in
//! Teams between steps. The tester's address is an argument and is never stored. Only one
//! meeting is used, found again by its subject; every step prints counts and booleans, never
//! the tester's name or address.

use anyhow::{Context, Result, bail};
use calendar as cal;
use teams_proto::Client;

const SUBJECT: &str = "kelma test meeting";
const DAY: i64 = 86_400_000;
const HOUR: i64 = 3_600_000;

fn wall_utc(ms: i64) -> String {
    cal::format_iso_ms(ms)[..19].to_string()
}

/// The test meeting, if it is in my calendar in the next ten days.
async fn find(client: &Client) -> Result<Option<cal::Event>> {
    let now = teams_proto::time::now_ms();
    Ok(cal::Calendar::view(client, now - DAY, now + 10 * DAY)
        .await?
        .into_iter()
        .find(|e| e.subject == SUBJECT && e.response == cal::Response::Organizer))
}

async fn need(client: &Client) -> Result<cal::Event> {
    find(client)
        .await?
        .context("no test meeting: run `calendar tester invite <address>` first")
}

fn usage() -> &'static str {
    "usage: calendar tester invite <address> | status | accept-proposal | move | forward <address> | cancel | cleanup"
}

pub async fn run(client: &Client, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("invite") => {
            let addr = args.get(1).context(usage())?;
            if find(client).await?.is_some() {
                bail!("a test meeting already exists: run `calendar tester cleanup` first");
            }
            // Two days out at 13:00 UTC, 30 minutes: far enough that nobody joins by mistake.
            let now = teams_proto::time::now_ms();
            let start = now + 2 * DAY;
            let start = start - start.rem_euclid(DAY) + 13 * HOUR;
            let new = cal::NewEvent {
                subject: SUBJECT.into(),
                body_html: "kelma test: an invitation from the kelma calendar form checks.".into(),
                start: wall_utc(start),
                end: wall_utc(start + HOUR / 2),
                tz: "UTC".into(),
                required: vec![cal::Person::new("", addr)],
                is_teams_meeting: true,
                response_requested: true,
                allow_new_time_proposals: true,
                ..cal::NewEvent::default()
            };
            let made = cal::create_event(client, &new).await?;
            println!(
                "created: attendees {}, online {}, join link {}, start as sent {}",
                made.attendees.len(),
                made.is_online_meeting,
                made.join_url.is_some(),
                made.start_ms == start
            );
        }
        Some("status") => {
            let e = need(client).await?;
            for (i, a) in e.attendees.iter().enumerate() {
                println!("attendee {}: {:?}", i + 1, a.response);
            }
            println!(
                "starts {} UTC, join link {}",
                wall_utc(e.start_ms),
                e.join_url.is_some()
            );
            let props = cal::event_proposals(client, &e.id).await?;
            println!("{} proposal(s)", props.len());
            for p in &props {
                println!("  {} to {} UTC", wall_utc(p.start_ms), wall_utc(p.end_ms));
            }
        }
        Some("accept-proposal") => {
            let e = need(client).await?;
            let props = cal::event_proposals(client, &e.id).await?;
            let p = props.first().context("no proposal yet")?;
            let moved = cal::move_event(
                client,
                &e.id,
                &wall_utc(p.start_ms),
                &wall_utc(p.end_ms),
                "UTC",
            )
            .await?;
            println!(
                "accepted: now at the proposed time {}",
                moved.start_ms == p.start_ms && moved.end_ms == p.end_ms
            );
        }
        Some("move") => {
            let e = need(client).await?;
            let moved = cal::move_event(
                client,
                &e.id,
                &wall_utc(e.start_ms + HOUR / 2),
                &wall_utc(e.end_ms + HOUR / 2),
                "UTC",
            )
            .await?;
            println!(
                "moved 30 minutes later: {}",
                moved.start_ms == e.start_ms + HOUR / 2
            );
        }
        Some("forward") => {
            let addr = args.get(1).context(usage())?;
            let e = need(client).await?;
            cal::forward_event(
                client,
                &e.id,
                &[cal::Person::new("", addr)],
                "kelma test: forwarded",
            )
            .await?;
            println!("forwarded");
        }
        Some("cancel") => {
            let e = need(client).await?;
            cal::cancel_event(client, &e.id, "kelma test: cancelled").await?;
            println!(
                "cancelled; still in my calendar: {}",
                find(client).await?.is_some()
            );
        }
        Some("cleanup") => {
            if let Some(e) = find(client).await? {
                cal::delete_event(client, &e.id).await?;
                println!("deleted");
            }
            println!(
                "cleanup proof: test meeting left {}",
                find(client).await?.is_some()
            );
        }
        _ => bail!(usage()),
    }
    Ok(())
}
