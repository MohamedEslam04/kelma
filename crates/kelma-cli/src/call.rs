//! `kelma-cli call …`: exercise the native calling crate from a terminal.
//!
//! ```text
//! kelma-cli call probe        authz calling endpoints, TURN credentials, calling trouter (no call)
//! kelma-cli call meetings     meetings with an active call, as the CSA chat list shows them
//! ```
//!
//! Live tests may only join meetings the user hosts on their own personal account; see the
//! project's live-test rules.

use anyhow::{Context, Result, bail};
use calling::ngc_trouter::{NgcEvent, NgcTrouter};
use calling::signaling::NgcClient;
use calling::signaling::meeting::{MeetingTarget, meeting_from_csa_chat};
use serde_json::Value;
use std::time::Duration;
use teams_proto::Client;

const CSA_ME: &str = "https://teams.microsoft.com/api/csa/api/v1/teams/users/me?isPrefetch=false&enableMembershipSummary=true";

fn host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| "?".into())
}

/// A calling-trouter endpoint id: stable per install and distinct from the chat trouter's.
pub fn calling_epid(chat_epid: &str) -> String {
    let tail = chat_epid.get(8..).unwrap_or(chat_epid);
    format!("ca11ca11{tail}")
}

/// The CSA chat entries of meetings whose call is running, with their join target.
pub async fn active_meetings(client: &Client) -> Result<Vec<(Value, MeetingTarget)>> {
    let v = client
        .json("csa", reqwest::Method::GET, CSA_ME, None)
        .await?;
    let chats = v["chats"].as_array().cloned().unwrap_or_default();
    Ok(chats
        .into_iter()
        .filter(|c| c["activeMeetup"]["status"] == "Active")
        .filter_map(|c| meeting_from_csa_chat(&c).map(|t| (c, t)))
        .collect())
}

pub async fn run(client: &Client, args: &[String]) -> Result<()> {
    match args.get(1).map(String::as_str) {
        Some("probe") => {
            let ngc = NgcClient::discover(client.clone())
                .await
                .context("authz (calling endpoints)")?;
            let ep = ngc.endpoints();
            println!(
                "conversation service: {} (region {}, partition {}, ring {})",
                host(&ep.conversation_service_url),
                ep.region,
                ep.partition,
                ep.ring
            );
            match calling::relay::fetch_grant(client).await {
                Ok(g) => println!(
                    "TURN: {} credential(s), expire in {:?}",
                    g.credentials.len(),
                    g.expires_in
                ),
                Err(e) => println!("TURN: failed: {e}"),
            }
            let (trouter, mut rx) = NgcTrouter::spawn(client.clone(), calling_epid(client.epid()));
            let wait = tokio::time::timeout(Duration::from_secs(30), async {
                while let Some(ev) = rx.recv().await {
                    match ev {
                        NgcEvent::Connected { surl } => return Ok(surl),
                        NgcEvent::Disconnected(r) => println!("calling trouter: disconnected: {r}"),
                        NgcEvent::Push(p) => println!("calling trouter: push {}", p.url),
                    }
                }
                bail!("calling trouter task ended")
            })
            .await;
            match wait {
                Ok(Ok(surl)) => println!("calling trouter: registered on {}", host(&surl)),
                Ok(Err(e)) => println!("calling trouter: {e}"),
                Err(_) => println!("calling trouter: not registered within 30 s"),
            }
            drop(trouter);
        }
        Some("meetings") => {
            let ms = active_meetings(client).await?;
            println!("{} meeting(s) with an active call", ms.len());
            for (chat, t) in &ms {
                println!(
                    "{}  [{}] code={} tenant={} organizer={} conversationUrl={}",
                    chat["title"].as_str().unwrap_or("?"),
                    chat["productContext"].as_str().unwrap_or("-"),
                    t.meeting_code.is_some(),
                    t.tenant_id.is_some(),
                    t.organizer_id.is_some(),
                    t.conversation_url.as_deref().map(host).unwrap_or_default(),
                );
                println!("  id: {}", t.thread_id.as_deref().unwrap_or("?"));
            }
        }
        _ => bail!("usage: kelma-cli call <probe|meetings>"),
    }
    Ok(())
}
