//! `kelma-cli call …`: exercise the native calling crate from a terminal.
//!
//! ```text
//! kelma-cli call probe        authz calling endpoints, TURN credentials, calling trouter (no call)
//! kelma-cli call meetings     meetings with an active call, as the CSA chat list shows them
//! kelma-cli call join [opts]   join the newest active meeting (or --link URL), send audio, record what arrives
//!     --link <url>   join this meeting link instead of the newest active meeting
//!     --mic          send the microphone (default: a 440 Hz tone)
//!     --speaker      play what arrives (default: write it to --out)
//!     --no-aec       send the raw microphone (default with --mic: echo cancellation, noise
//!                    suppression and AGC, fed by what --speaker plays)
//!     --out <wav>    where received audio goes (default: kelma-call.wav)
//!     --secs <n>     stay this long, then leave (default: 30)
//!     --relay <host> TURN gateway to start from (default: chosen from the authz region)
//!     --relay-only   use relayed (TURN) candidates only, to prove the relay path
//!     --dump <dir>   write every SDP exchanged to this directory (keep it out of the repo)
//!     --mute-at <s>        mute us <s> seconds after the join (stop sending the mic, publish
//!                          isMuted); repeatable, like the three below
//!     --unmute-at <s>      unmute us <s> seconds after the join
//!     --hand-at <s>        raise our hand <s> seconds after the join
//!     --lower-hand-at <s>  lower our hand <s> seconds after the join
//! ```
//!
//! `join` prints the roster (count, display names, role, mute and hand flags) whenever it
//! changes. Names go to the terminal only, never to a file.
//!
//! Live tests may only join meetings the user hosts on their own personal account; see the
//! project's live-test rules.

use anyhow::{Context, Result, bail};
use calling::audio::{
    AudioProcessor, Capture, Paced, Playback, ToneSource, WavSink, echo_cancelled,
    spawn_capture_encoder, spawn_decoder_playback,
};
use calling::invitation::{CallEvent, NgcPush, classify};
use calling::media::{MediaOptions, MediaSession, MediaState};
use calling::ngc_trouter::{NgcEvent, NgcTrouter};
use calling::signaling::meeting::{MeetingTarget, meeting_from_csa_chat, meeting_from_join_url};
use calling::signaling::roster::roster_snapshot;
use calling::signaling::{CallSession, Controls, NgcClient, Roster};
use calling::{Links, Modality, SdpBlob};
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
            match calling::relay::resolve_ice_servers(client, &ep.region, opt(args, "--relay"))
                .await
            {
                Ok((ice, expires)) => println!(
                    "TURN: {} ({} credential(s), expire in {expires:?})",
                    ice.first()
                        .and_then(|s| s.urls.first())
                        .map(String::as_str)
                        .unwrap_or("-"),
                    ice.len(),
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
        Some("join") => join(client, &args[2..]).await?,
        _ => bail!("usage: kelma-cli call <probe|meetings|join>"),
    }
    Ok(())
}

fn opt<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

/// The first `{contentType, blob}` object anywhere in `v` (a join answer may carry the media
/// answer inline).
fn find_blob(v: &Value) -> Option<SdpBlob> {
    match v {
        Value::Object(m) => {
            if let (Some(ct), Some(b)) = (
                m.get("contentType").and_then(Value::as_str),
                m.get("blob").and_then(Value::as_str),
            ) && !b.is_empty()
            {
                return Some(SdpBlob {
                    content_type: ct.to_owned(),
                    blob: b.to_owned(),
                });
            }
            m.values().find_map(find_blob)
        }
        Value::Array(a) => a.iter().find_map(find_blob),
        _ => None,
    }
}

/// An in-meeting control `join` can run on a schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Mute,
    Unmute,
    RaiseHand,
    LowerHand,
}

const CONTROL_FLAGS: [(&str, Control); 4] = [
    ("--mute-at", Control::Mute),
    ("--unmute-at", Control::Unmute),
    ("--hand-at", Control::RaiseHand),
    ("--lower-hand-at", Control::LowerHand),
];

/// Every `--<control>-at <seconds>` in `args`, earliest first.
fn scheduled_controls(args: &[String]) -> Result<Vec<(Duration, Control)>> {
    let mut out = Vec::new();
    for (i, a) in args.iter().enumerate() {
        let Some((flag, control)) = CONTROL_FLAGS.iter().find(|(f, _)| f == a) else {
            continue;
        };
        let secs: f64 = args
            .get(i + 1)
            .and_then(|v| v.parse().ok())
            .filter(|s: &f64| s.is_finite() && *s >= 0.0)
            .with_context(|| format!("{flag} needs a number of seconds"))?;
        out.push((Duration::from_secs_f64(secs), *control));
    }
    out.sort_by_key(|(at, _)| *at);
    Ok(out)
}

fn print_roster(roster: &Roster, own_mri: &str) {
    println!("roster: {}", roster.summary(own_mri));
}

fn dump(dir: Option<&str>, name: &str, sdp: &str) {
    if let Some(d) = dir {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        let _ = std::fs::write(format!("{d}/{n}-{name}.sdp"), sdp);
    }
}

async fn join(client: &Client, args: &[String]) -> Result<()> {
    let dump_dir = opt(args, "--dump");
    let secs: u64 = opt(args, "--secs")
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let out = opt(args, "--out").unwrap_or("kelma-call.wav").to_owned();
    let mut schedule = scheduled_controls(args)?;
    let target = match opt(args, "--link") {
        Some(link) => meeting_from_join_url(link).context("not a meeting link")?,
        None => {
            let mut ms = active_meetings(client).await?;
            ms.sort_by_key(|(c, _)| c["lastJoinAt"].as_str().unwrap_or_default().to_owned());
            let (chat, t) = ms.pop().context("no meeting with an active call")?;
            println!(
                "meeting: {} [{}]",
                chat["title"].as_str().unwrap_or("?"),
                chat["productContext"].as_str().unwrap_or("-")
            );
            t
        }
    };

    let ngc = NgcClient::discover(client.clone()).await?;
    let (trouter, mut pushes) = NgcTrouter::spawn(client.clone(), calling_epid(client.epid()));
    let surl = tokio::time::timeout(Duration::from_secs(30), trouter.connected_surl())
        .await
        .ok()
        .flatten()
        .context("calling trouter did not register")?;
    println!("calling trouter registered");

    let region = ngc.endpoints().region.clone();
    let ice = match calling::relay::resolve_ice_servers(client, &region, opt(args, "--relay")).await
    {
        Ok((ice, _)) => ice,
        Err(e) => {
            println!("TURN unavailable ({e}); host candidates only");
            vec![]
        }
    };
    if let Some(url) = ice.first().and_then(|s| s.urls.first()) {
        println!("TURN: {url}");
    }
    let media_opts = MediaOptions {
        relay_only: args.iter().any(|a| a == "--relay-only"),
        ..MediaOptions::default()
    };
    let (mut media, events) = MediaSession::with_options(ice.clone(), media_opts.clone()).await?;
    let mut state_rx = events.state;
    let offer = media.create_offer().await?;
    dump(dump_dir, "our-offer", &offer);
    println!("offer ready ({} bytes)", offer.len());

    let me = client.self_mri().await?;
    let s = CallSession::new(&surl, &me, "kelma");
    let modalities = [Modality::Audio];
    let conv = ngc
        .join(&s, &target, &SdpBlob::ngc(offer), &modalities)
        .await
        .context("join")?;
    println!("joined: controller {}", host(&conv.controller));
    let joined_at = tokio::time::Instant::now();
    let mut controls = Controls::default();
    let mut roster = Roster::default();
    if let Some(snapshot) = roster_snapshot(&conv.raw)
        && roster.apply_snapshot(snapshot)
    {
        print_roster(&roster, &s.me.mri);
    }
    let mut links: Links = conv.links.clone();
    let mut answered = false;
    if let Some(ans) = find_blob(&conv.raw) {
        let sdp = ans.decode()?;
        dump(dump_dir, "service-answer-inline", &sdp);
        match media.set_answer(&sdp).await {
            Ok(()) => {
                answered = true;
                println!("media answer (inline in the join answer) applied");
            }
            Err(e) => println!("inline media answer refused: {e}"),
        }
    }

    // Audio: source -> encoder -> media; media -> decoder -> sink. The microphone goes through
    // the echo canceller, whose reference is what the speaker plays.
    let mut echo_reference = None;
    let mut encoded = if args.iter().any(|a| a == "--mic") {
        let mic = Capture::start(None)?;
        if args.iter().any(|a| a == "--no-aec") {
            spawn_capture_encoder(mic)
        } else {
            let (mic, reference) = echo_cancelled(mic, AudioProcessor::new());
            echo_reference = Some(reference);
            spawn_capture_encoder(mic)
        }
    } else {
        spawn_capture_encoder(Paced::new(ToneSource::endless(440.0)))
    };
    // The media session can be replaced mid-call (the service restarts ICE when the lobby
    // admits us), so the sender follows a watch and every session's audio feeds one forwarder.
    let (sender_tx, sender_rx) = tokio::sync::watch::channel(media.audio_sender());
    let sent = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let sent_count = sent.clone();
    // While muted the microphone's packets are dropped here: muting is also local, not only the
    // state the service shows.
    let mic_muted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let send_muted = mic_muted.clone();
    let send_task = tokio::spawn(async move {
        while let Some(p) = encoded.recv().await {
            if send_muted.load(std::sync::atomic::Ordering::Relaxed) {
                continue;
            }
            let sender = sender_rx.borrow().clone();
            if sender.send(p).await.is_ok() {
                sent_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    });
    let (fwd_tx, fwd_rx) = tokio::sync::mpsc::channel(256);
    let received = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let rx_total = received.clone();
    // Owns the only sender of the playback queue: dropping it (and the forwarders) ends playback.
    let forward = move |mut incoming: tokio::sync::mpsc::Receiver<calling::OpusPacket>| {
        let (fwd_tx, rx_count) = (fwd_tx.clone(), rx_total.clone());
        tokio::spawn(async move {
            while let Some(p) = incoming.recv().await {
                rx_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if fwd_tx.send(p).await.is_err() {
                    break;
                }
            }
        })
    };
    let mut forwarders = vec![forward(events.audio)];
    let playback = if args.iter().any(|a| a == "--speaker") {
        let speaker = match echo_reference {
            Some(reference) => Playback::start_with_tap(None, move |f| reference.push(f))?,
            None => Playback::start(None)?,
        };
        spawn_decoder_playback(fwd_rx, speaker)
    } else {
        spawn_decoder_playback(fwd_rx, WavSink::create(&out)?)
    };

    let mut keep_alive: Option<tokio::task::JoinHandle<()>> = None;
    let deadline = tokio::time::sleep(Duration::from_secs(secs));
    tokio::pin!(deadline);
    let mut ended = false;
    loop {
        let next_control = joined_at + schedule.first().map(|(at, _)| *at).unwrap_or_default();
        tokio::select! {
            _ = &mut deadline => break,
            _ = tokio::time::sleep_until(next_control), if !schedule.is_empty() => {
                let (_, control) = schedule.remove(0);
                let controller = conv.controller.as_str();
                let result = match control {
                    Control::Mute | Control::Unmute => {
                        let muted = control == Control::Mute;
                        mic_muted.store(muted, std::sync::atomic::Ordering::Relaxed);
                        ngc.set_muted(&s, &links, controller, &mut controls, muted).await
                    }
                    Control::RaiseHand => ngc
                        .raise_hand(&s, &links, controller, &mut controls)
                        .await
                        .map(|r| println!("hand raised (queue #{})", r.rank.map_or("?".into(), |n| n.to_string()))),
                    Control::LowerHand => {
                        let own = roster.hand_of(&s.me.mri).map(|h| h.state_id.clone());
                        ngc.lower_hand(&s, &links, controller, &mut controls, own.as_deref())
                            .await
                            .map(|lowered| if !lowered { println!("no raised hand to lower") })
                    }
                };
                match result {
                    Ok(()) => println!("control {control:?}: ok"),
                    Err(e) => println!("control {control:?} failed: {e}"),
                }
            }
            st = state_rx.recv() => match st {
                Some(st) => {
                    println!("media: {st:?}");
                    if matches!(st, MediaState::Failed | MediaState::Closed) { break; }
                }
                None => break,
            },
            ev = pushes.recv() => {
                let Some(ev) = ev else { break };
                let NgcEvent::Push(p) = ev else {
                    println!("calling trouter: {ev:?}");
                    continue;
                };
                match classify(&p.url, &p.body) {
                    NgcPush::Call { event, .. } => match event {
                        CallEvent::Accepted { answer, links: l, keep_alive_interval_s, .. } => {
                            println!("accepted");
                            links.0.extend(l.0);
                            if let (Some(a), false) = (answer, answered) {
                                let sdp = a.decode().unwrap_or_else(|e| { println!("undecodable SDP: {e}"); String::new() });
                                dump(dump_dir, "service-answer", &sdp);
                                match media.set_answer(&sdp).await {
                                    Ok(()) => {
                                        answered = true;
                                        println!("media answer applied");
                                    }
                                    Err(e) => println!("media answer refused: {e}"),
                                }
                            }
                            if keep_alive.is_none() {
                                let (ngc, s, l) = (ngc.clone(), s.clone(), links.clone());
                                keep_alive = Some(tokio::spawn(async move {
                                    if let Err(e) = ngc.run_keep_alive(&s, &l, keep_alive_interval_s).await {
                                        println!("keepAlive stopped: {e}");
                                    }
                                }));
                            }
                        }
                        CallEvent::MediaAnswer { answer, links: l } => {
                            links.0.extend(l.0);
                            if !answered {
                                let sdp = answer.decode().unwrap_or_else(|e| { println!("undecodable SDP: {e}"); String::new() });
                                dump(dump_dir, "service-answer", &sdp);
                                match media.set_answer(&sdp).await {
                                    Ok(()) => {
                                        answered = true;
                                        println!("media answer applied");
                                    }
                                    Err(e) => println!("media answer refused: {e}"),
                                }
                            }
                        }
                        CallEvent::MediaRenegotiation { offer, links: l } => {
                            println!("renegotiation offered");
                            let sdp = offer.decode().unwrap_or_else(|e| { println!("undecodable SDP: {e}"); String::new() });
                            dump(dump_dir, "service-reoffer", &sdp);
                            let answer = match media.accept_offer(&sdp).await {
                                Ok(ans) => Ok(ans),
                                Err(e) => {
                                    // A re-offer that restarts ICE (new mids/ufrags after the lobby)
                                    // is answered by a fresh peer connection.
                                    println!("existing session refused the re-offer ({e}); answering on a new session");
                                    match MediaSession::with_options(ice.clone(), media_opts.clone()).await {
                                        Ok((fresh, ev)) => match fresh.accept_offer(&sdp).await {
                                            Ok(ans) => {
                                                let old = std::mem::replace(&mut media, fresh);
                                                let _ = old.close().await;
                                                let _ = sender_tx.send(media.audio_sender());
                                                state_rx = ev.state;
                                                forwarders.push(forward(ev.audio));
                                                Ok(ans)
                                            }
                                            Err(e) => Err(e),
                                        },
                                        Err(e) => Err(e),
                                    }
                                }
                            };
                            match answer {
                                Ok(ans) => {
                                    dump(dump_dir, "our-reanswer", &ans);
                                    if let Some(url) = l.get("mediaAnswer") {
                                        match ngc.send_media_answer(&s, url, &modalities, &SdpBlob::ngc(ans)).await {
                                            Ok(()) => {
                                                println!("renegotiation answered");
                                                // Teams ignored the first state update after admission
                                                // (mautrix-teams), so publish the mute state again.
                                                if let Err(e) = ngc.repeat_muted(&s, &links, &conv.controller, &mut controls).await {
                                                    println!("mute state not repeated: {e}");
                                                }
                                            }
                                            Err(e) => println!("renegotiation answer refused: {e}"),
                                        }
                                    }
                                }
                                Err(e) => println!("renegotiation not answered: {e}"),
                            }
                            links.0.extend(l.0);
                        }
                        CallEvent::Rejected(r) | CallEvent::Ended(r) => {
                            println!("ended by service: {r:?}");
                            ended = true;
                            break;
                        }
                        CallEvent::Progress { status } => println!("progress: {status}"),
                        CallEvent::MediaAcknowledgement { .. } => println!("media acknowledged"),
                        CallEvent::RosterUpdate(delta) => {
                            if roster.apply(&delta) {
                                print_roster(&roster, &s.me.mri);
                            }
                        }
                        CallEvent::ConversationUpdate(body) => {
                            let keys: Vec<&str> = body.as_object().map(|m| m.keys().map(String::as_str).collect()).unwrap_or_default();
                            println!("conversation update: {}", keys.join(","));
                            if let Some(snapshot) = roster_snapshot(&body)
                                && roster.apply_snapshot(snapshot)
                            {
                                print_roster(&roster, &s.me.mri);
                            }
                        }
                        CallEvent::Other { scope, event, .. } => println!("callback {scope}/{event}"),
                    },
                    NgcPush::Invitation(_) => println!("(ignored an incoming call invitation)"),
                    NgcPush::Other { url, .. } => println!("push {}", url.rsplit('/').take(3).collect::<Vec<_>>().join("<")),
                }
            }
        }
    }

    if !ended {
        let r = if links.get("leave").is_some() || links.get("callLeg").is_some() {
            ngc.hang_up(&s, &links).await
        } else {
            ngc.leave_controller(&s, &conv.controller).await
        };
        match r {
            Ok(()) => println!("left the meeting"),
            Err(e) => println!("leave failed: {e}"),
        }
    }
    if let Some(k) = keep_alive {
        k.abort();
    }
    media.close().await?;
    drop(media);
    // The tone source never ends: stop the sender, then let the playback drain and finish.
    send_task.abort();
    // Dropping the forwarders closes the playback's input, so the WAV is finalised.
    drop(forward);
    for f in forwarders {
        f.abort();
    }
    let sent = sent.load(std::sync::atomic::Ordering::Relaxed);
    if tokio::time::timeout(Duration::from_secs(5), playback)
        .await
        .is_err()
    {
        println!("playback did not finish in 5 s");
    }
    println!(
        "audio packets: sent {sent}, received {}",
        received.load(std::sync::atomic::Ordering::Relaxed)
    );
    if !args.iter().any(|a| a == "--speaker") {
        println!("received audio written to {out}");
    }
    drop(trouter);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn scheduled_controls_are_read_from_every_flag_earliest_first() {
        let got = scheduled_controls(&args(&[
            "--secs",
            "90",
            "--hand-at",
            "40",
            "--mute-at",
            "20",
            "--unmute-at",
            "30.5",
            "--mute-at",
            "50",
            "--lower-hand-at",
            "45",
        ]))
        .unwrap();
        assert_eq!(
            got,
            vec![
                (Duration::from_secs(20), Control::Mute),
                (Duration::from_secs_f64(30.5), Control::Unmute),
                (Duration::from_secs(40), Control::RaiseHand),
                (Duration::from_secs(45), Control::LowerHand),
                (Duration::from_secs(50), Control::Mute),
            ]
        );
        assert!(
            scheduled_controls(&args(&["--secs", "30"]))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_control_flag_without_seconds_is_an_error() {
        assert!(scheduled_controls(&args(&["--mute-at"])).is_err());
        assert!(scheduled_controls(&args(&["--hand-at", "soon"])).is_err());
        assert!(scheduled_controls(&args(&["--unmute-at", "-1"])).is_err());
    }
}
