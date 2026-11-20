//! `kelma-cli settings`: notification and privacy settings (plans/chat.md 9.1, X1.5).
//!
//! Reads are live and read-only. `settings set ... --dry-run` prints the request that would be
//! made and sends nothing; without `--dry-run` it refuses. `settings quiet` is local and needs
//! no session.

use anyhow::{Context, Result, bail};
use teams_proto::Client;
use teams_proto::user_settings::{
    self as us, Change, ContactList, Day, Delivery, Kind, MeetingChatMode, NotificationSettings,
    PlannedRequest, PrivacyFlag, QuietHours, Rsvp, SoundField,
};

const USAGE: &str = "usage: settings <notifications|quiet|priority|privacy|blocked|shape> | settings set <notify <type> <banner|feed|off> | pause <iso|off> | sound <field> <value> | meeting-chat <all|participated|none> | meeting-rsvp <answer> <on|off> | privacy <flag> <on|off> | block|unblock|priority-add|priority-remove <mri>> --dry-run";

fn on_off(s: &str) -> Result<bool> {
    match s {
        "on" | "true" | "yes" => Ok(true),
        "off" | "false" | "no" => Ok(false),
        _ => bail!("expected on or off, got {s}"),
    }
}

fn shown(d: Option<Delivery>) -> &'static str {
    match d {
        Some(Delivery::BannerAndFeed) => "banner and feed",
        Some(Delivery::FeedOnly) => "feed only",
        Some(Delivery::Off) => "off",
        None => "not set",
    }
}

/// Builds the request of a `settings set` command. Needs the current property bag because the
/// notification settings are written back whole. Refuses unless `--dry-run` is given.
pub fn plan_set(
    props: &serde_json::Value,
    chat_service: &str,
    middle_tier: &str,
    args: &[String],
) -> Result<PlannedRequest> {
    if !args.iter().any(|a| a == "--dry-run" || a == "--apply") {
        bail!("settings set needs --dry-run (print the request) or --apply (send it)");
    }
    let rest: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| *a != "--dry-run" && *a != "--apply")
        .collect();
    let mut n = NotificationSettings::from_properties(props);
    let contact = |list, change| -> Result<PlannedRequest> {
        let mri = rest.get(1).context("give an MRI")?;
        Ok(us::manage_request(middle_tier, list, change, mri))
    };
    match rest.first().copied() {
        Some("notify") => {
            let (Some(kind), Some(how)) = (rest.get(1), rest.get(2)) else {
                bail!("usage: settings set notify <type> <banner|feed|off> --dry-run")
            };
            let kind = Kind::from_slug(kind).with_context(|| format!("unknown type {kind}"))?;
            let how = Delivery::from_slug(how).with_context(|| format!("unknown value {how}"))?;
            n.set_delivery(kind, how);
        }
        Some("pause") => {
            let until = rest.get(1).context("give an ISO time or off")?;
            n.set_paused_until((*until != "off").then(|| (*until).to_owned()));
        }
        Some("sound") => {
            let (Some(field), Some(value)) = (rest.get(1), rest.get(2)) else {
                bail!("usage: settings set sound <field> <value> --dry-run")
            };
            let field =
                SoundField::from_slug(field).with_context(|| format!("unknown field {field}"))?;
            n.set_sound(field, value);
        }
        Some("meeting-chat") => {
            let mode = rest.get(1).context("give all, participated or none")?;
            n.set_meeting_chat_mode(
                MeetingChatMode::from_slug(mode).with_context(|| format!("unknown mode {mode}"))?,
            );
        }
        Some("meeting-rsvp") => {
            let (Some(answer), Some(flag)) = (rest.get(1), rest.get(2)) else {
                bail!("usage: settings set meeting-rsvp <answer> <on|off> --dry-run")
            };
            let answer =
                Rsvp::from_slug(answer).with_context(|| format!("unknown answer {answer}"))?;
            n.set_meeting_chat_rsvp(answer, on_off(flag)?);
        }
        Some("privacy") => {
            let (Some(flag), Some(value)) = (rest.get(1), rest.get(2)) else {
                bail!("usage: settings set privacy <flag> <on|off> --dry-run")
            };
            let flag =
                PrivacyFlag::from_slug(flag).with_context(|| format!("unknown flag {flag}"))?;
            return Ok(us::privacy_flag_request(chat_service, flag, on_off(value)?));
        }
        Some("block") => return contact(ContactList::Blocked, Change::Add),
        Some("unblock") => return contact(ContactList::Blocked, Change::Remove),
        Some("priority-add") => return contact(ContactList::Priority, Change::Add),
        Some("priority-remove") => return contact(ContactList::Priority, Change::Remove),
        _ => bail!("{USAGE}"),
    }
    Ok(n.write_request(chat_service))
}

/// `--at <day> <HH:MM>` among the arguments.
fn parse_at(args: &[String]) -> Result<Option<(Day, u16)>> {
    let Some(i) = args.iter().position(|a| a == "--at") else {
        return Ok(None);
    };
    let (Some(day), Some(time)) = (args.get(i + 1), args.get(i + 2)) else {
        bail!("usage: --at <day> <HH:MM>")
    };
    let day = Day::from_name(day).with_context(|| format!("unknown day {day}"))?;
    let time = us::parse_clock(time).with_context(|| format!("bad time {time}"))?;
    Ok(Some((day, time)))
}

/// `<days> <start> <end> [--mobile]`, or nothing for weeknights 22:00 to 07:00.
fn parse_quiet(args: &[String]) -> Result<QuietHours> {
    let mut words = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--mobile" => {}
            "--at" => {
                it.next();
                it.next();
            }
            _ => words.push(a.as_str()),
        }
    }
    if words.is_empty() {
        return Ok(QuietHours::weeknights());
    }
    let [days, start, end] = words[..] else {
        bail!("usage: settings quiet [<days> <start> <end>] [--mobile] [--at <day> <HH:MM>]")
    };
    let days = days
        .split(',')
        .map(|d| Day::from_name(d).with_context(|| format!("unknown day {d}")))
        .collect::<Result<Vec<_>>>()?;
    Ok(QuietHours {
        enabled: true,
        days,
        start_min: us::parse_clock(start).with_context(|| format!("bad time {start}"))?,
        end_min: us::parse_clock(end).with_context(|| format!("bad time {end}"))?,
        mobile: args.iter().any(|a| a == "--mobile"),
    })
}

/// True for the commands that need no session.
pub fn is_offline(args: &[String]) -> bool {
    args.first().map(String::as_str) == Some("quiet")
}

/// `settings quiet`: builds a local schedule from the arguments, prints it, and with `--at` says
/// whether that moment is quiet. Nothing is stored.
pub fn quiet(args: &[String]) -> Result<()> {
    let q = parse_quiet(args)?;
    println!("{}", serde_json::to_string(&q)?);
    println!("local schedule: Teams keeps no quiet hours that desktop clients read");
    if let Some((day, minute)) = parse_at(args)? {
        println!(
            "{day:?} {} is {}",
            us::format_clock(minute),
            if q.is_quiet(day, minute) {
                "quiet"
            } else {
                "not quiet"
            }
        );
    }
    Ok(())
}

fn print_notifications(n: &NotificationSettings) {
    println!("preset        {:?}", n.preset());
    println!("paused until  {}", n.paused_until().unwrap_or("-"));
    for k in Kind::ALL {
        println!("{:<24}{}", k.slug(), shown(n.delivery(k)));
    }
    for f in SoundField::ALL {
        println!("{:<24}{}", f.slug(), n.sound(f).unwrap_or("not set"));
    }
    println!("meeting-chat            {:?}", n.meeting_chat_mode());
    for r in Rsvp::ALL {
        println!(
            "meeting-rsvp {:<11}{}",
            r.slug(),
            n.meeting_chat_rsvp(r)
                .map_or("not set", |b| if b { "on" } else { "off" })
        );
    }
}

fn print_ids(ids: &[String], show: bool) {
    println!("{} people", ids.len());
    if show {
        for id in ids {
            println!("{id}");
        }
    }
}

/// Names and JSON types only, never values: for checking the shape of a live answer.
fn shape(v: &serde_json::Value) -> String {
    use serde_json::Value::*;
    match v {
        Null => "null".into(),
        Bool(_) => "bool".into(),
        Number(_) => "number".into(),
        String(_) => "string".into(),
        Array(a) => format!("array({})", a.len()),
        Object(o) => format!(
            "{{{}}}",
            o.iter()
                .map(|(k, v)| format!("{k}: {}", shape(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

pub async fn run(client: &Client, args: &[String]) -> Result<()> {
    let show_ids = args.iter().any(|a| a == "--ids");
    match args.first().map(String::as_str) {
        Some("notifications") => print_notifications(&us::notifications(client).await?),
        Some("priority") => print_ids(&us::priority_access(client).await?.contacts, show_ids),
        Some("blocked") => print_ids(&us::contacts(client, ContactList::Blocked).await?, show_ids),
        Some("privacy") => {
            let p = us::privacy(client).await?;
            println!("read receipts           {}", p.read_receipts);
            println!("block calls (non-contact){}", p.block_non_contact_calls);
            println!("block group adds        {}", p.block_non_contact_group_adds);
            println!("surveys                 {}", p.surveys);
            print_ids(&p.blocked, show_ids);
        }
        Some("shape") => {
            let props = us::properties(client).await?;
            let personal = NotificationSettings::from_properties(&props).to_value();
            println!("userPersonalSettings: {}", shape(&personal));
            if let Some(o) = props.as_object() {
                for (k, v) in o {
                    println!("{k}: {}", shape(v).chars().take(60).collect::<String>());
                }
            }
            for list in [ContactList::Blocked, ContactList::Priority] {
                let req = us::list_request(&client.middle_tier().await, list);
                let v = client
                    .json("contact list", req.method, &req.url, None)
                    .await?;
                println!("{list:?}: {}", shape(&v));
            }
        }
        Some("set") => {
            let props = us::properties(client).await?;
            let req = plan_set(
                &props,
                &client.chat_service().await,
                &client.middle_tier().await,
                &args[1..],
            )?;
            if args.iter().any(|a| a == "--apply") {
                client.send_planned(&req).await?;
                println!(
                    "sent {} {}",
                    req.method,
                    req.url.split('?').next().unwrap_or_default()
                );
            } else {
                println!("{}", req.dry_run());
            }
        }
        _ => bail!("{USAGE}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CS: &str = "https://cs";
    const MT: &str = "https://mt";

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn bag() -> serde_json::Value {
        json!({"userPersonalSettings": json!({
            "toastSettings": {"Like": true},
            "activityGenerationSettings": {"Like": true},
        }).to_string()})
    }

    fn plan(a: &[&str]) -> anyhow::Result<PlannedRequest> {
        plan_set(&bag(), CS, MT, &args(a))
    }

    #[test]
    fn a_write_without_dry_run_is_refused() {
        let err = plan(&["notify", "like", "off"]).unwrap_err().to_string();
        assert!(err.contains("--dry-run"), "{err}");
    }

    #[test]
    fn notify_plans_the_settings_put() {
        let r = plan(&["notify", "like", "feed", "--dry-run"]).unwrap();
        assert_eq!(r.method, reqwest::Method::PUT);
        assert!(r.url.ends_with("?name=userPersonalSettings"));
        let sent = r.body.unwrap()["userPersonalSettings"]
            .as_str()
            .unwrap()
            .to_owned();
        let v: serde_json::Value = serde_json::from_str(&sent).unwrap();
        assert_eq!(v["toastSettings"]["Like"], json!(false));
        assert_eq!(v["activityGenerationSettings"]["Like"], json!(true));
        assert_eq!(v["selectedPreset"], json!(3));
    }

    #[test]
    fn other_notification_writes_plan() {
        let pause = plan(&["pause", "2026-11-20T08:00:00.000Z", "--dry-run"]).unwrap();
        assert!(
            pause.body.unwrap()["userPersonalSettings"]
                .as_str()
                .unwrap()
                .contains("temporarilyPauseUntil")
        );
        let sound = plan(&["sound", "alert-sound", "Off", "--dry-run"]).unwrap();
        assert!(
            sound.body.unwrap()["userPersonalSettings"]
                .as_str()
                .unwrap()
                .contains("\"alertSound\":\"Off\"")
        );
        let chat = plan(&["meeting-chat", "all", "--dry-run"]).unwrap();
        assert!(
            chat.body.unwrap()["userPersonalSettings"]
                .as_str()
                .unwrap()
                .contains("MeetingsUserIsInvitedTo")
        );
        let rsvp = plan(&["meeting-rsvp", "declined", "on", "--dry-run"]).unwrap();
        assert!(
            rsvp.body.unwrap()["userPersonalSettings"]
                .as_str()
                .unwrap()
                .contains("RSVPv2Declined\":true")
        );
    }

    #[test]
    fn privacy_and_lists_plan() {
        let r = plan(&["privacy", "read-receipts", "off", "--dry-run"]).unwrap();
        assert_eq!(r.body, Some(json!({"readReceiptsEnabled": "false"})));
        let r = plan(&["block", "8:orgid:x", "--dry-run"]).unwrap();
        assert_eq!(r.url, "https://mt/beta/userSettings/blocklist/manage");
        assert_eq!(r.body, Some(json!({"add": ["8:orgid:x"]})));
        let r = plan(&["unblock", "8:orgid:x", "--dry-run"]).unwrap();
        assert_eq!(r.body, Some(json!({"remove": ["8:orgid:x"]})));
        let r = plan(&["priority-add", "8:orgid:x", "--dry-run"]).unwrap();
        assert_eq!(
            r.url,
            "https://mt/beta/userSettings/breakthroughlist/manage"
        );
        let r = plan(&["priority-remove", "8:orgid:x", "--dry-run"]).unwrap();
        assert_eq!(r.body, Some(json!({"remove": ["8:orgid:x"]})));
    }

    #[test]
    fn bad_arguments_are_named() {
        for bad in [
            &["notify", "nope", "off", "--dry-run"][..],
            &["notify", "like", "loud", "--dry-run"],
            &["privacy", "read-receipts", "maybe", "--dry-run"],
            &["meeting-chat", "some", "--dry-run"],
            &["wat", "--dry-run"],
            &["block", "--dry-run"],
        ] {
            assert!(plan(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn quiet_arguments_build_and_test_a_schedule() {
        let q = parse_quiet(&args(&["mon,tue", "22:00", "07:00", "--mobile"])).unwrap();
        assert_eq!(q.days.len(), 2);
        assert!(q.mobile);
        assert!(q.is_quiet(teams_proto::user_settings::Day::Tue, 23 * 60));
        let d = parse_quiet(&[]).unwrap();
        assert_eq!(d, teams_proto::user_settings::QuietHours::weeknights());
        assert!(parse_quiet(&args(&["mon", "25:00", "07:00"])).is_err());
        assert!(parse_quiet(&args(&["funday", "22:00", "07:00"])).is_err());
        assert_eq!(
            parse_at(&args(&["--at", "fri", "23:15"]))
                .unwrap()
                .unwrap()
                .1,
            23 * 60 + 15
        );
        assert!(parse_at(&args(&["--at", "fri"])).is_err());
        assert!(parse_at(&[]).unwrap().is_none());
    }
}
