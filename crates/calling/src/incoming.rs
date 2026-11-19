//! Answering an incoming 1:1 call, with or without the caller's offer in the ring.
//!
//! Live 2026-11-19: a ring from a Teams web guest came with `callNotification.mediaContent: null`
//! (and `links.mediaAnswer: "cc://ma"`, a placeholder no client can POST to). Two ways out:
//!
//! 1. **Fetch the offer** (default; rusty-teams `engine.rs` `ring_task`, MIT, whose own ring
//!    fixture has no offer): `attach` with `requireMediaContent: true`; the attach answer's
//!    `callInvitation.mediaContent` holds the caller's offer and the rest is the answerer path.
//! 2. **Offer ourselves** (UNVERIFIED): when the attach answer still has no offer, we build an
//!    offer the way `place_call` does (verified live for outgoing calls) and accept with it; the
//!    caller's answer comes inline in the acknowledgement or on our `call/mediaAnswer/` callback,
//!    and is applied with [`MediaSession::set_answer`]. [`NoOffer::OfferInAttach`] instead sends
//!    our offer with the attach itself (the coordinator's reading of the capture), then accepts
//!    with the same offer.
//!
//! [`Answering`] is the pure state machine; [`answer_invitation`] drives it against the service.

use crate::error::{Error, Result};
use crate::invitation::CallEvent;
use crate::media::MediaSession;
use crate::signaling::{Attached, CallSession, NgcClient};
use crate::types::{CallInvitation, Links, Modality, SdpBlob};
use std::time::Duration;
use tokio::sync::mpsc;

/// How long we wait for the caller's answer to our offer.
pub const ANSWER_WAIT: Duration = Duration::from_secs(20);

/// What to do when the ring carries no offer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NoOffer {
    /// Attach asking for the offer (`requireMediaContent: true`); offer ourselves only when the
    /// attach answer has none either.
    #[default]
    FetchOffer,
    /// Send our offer with the attach (`attach.mediaContent`), then accept with it. UNVERIFIED.
    OfferInAttach,
}

/// The next thing the callee does.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// POST the ring's `attach` link; `require_media` asks for the caller's offer and
    /// `with_offer` sends ours along.
    Attach {
        require_media: bool,
        with_offer: bool,
    },
    /// Answer the caller's offer and accept with the answer (the answerer path).
    AnswerOffer(SdpBlob),
    /// Nobody offered: accept with our own offer.
    SendOffer,
    /// Our offer is out; wait for the caller's answer (`call/mediaAnswer/`).
    AwaitAnswer,
    /// Accepted and negotiated: apply [`Answering::remote_answer`] (offerer path) and hand the
    /// call to the call manager.
    Adopt,
    Failed(String),
}

/// The callee's side of answering one invitation, as a state machine.
#[derive(Debug, Clone)]
pub struct Answering {
    step: Step,
    /// The ring's own offer, kept until the attach answer is in.
    ring_offer: Option<SdpBlob>,
    links: Links,
    media_leg_id: Option<String>,
    answer: Option<SdpBlob>,
}

impl Answering {
    pub fn new(inv: &CallInvitation, no_offer: NoOffer) -> Self {
        let mut a = Answering {
            step: Step::SendOffer,
            ring_offer: inv.offer.clone(),
            links: Links::default(),
            media_leg_id: None,
            answer: None,
        };
        a.step = match (&inv.offer, inv.links.get("acceptance")) {
            // An invitation that is not forked: answer it straight away.
            (Some(offer), Some(_)) => {
                a.links = inv.links.clone();
                Step::AnswerOffer(offer.clone())
            }
            (Some(_), None) => Step::Attach {
                require_media: false,
                with_offer: false,
            },
            (None, _) => Step::Attach {
                require_media: no_offer == NoOffer::FetchOffer,
                with_offer: no_offer == NoOffer::OfferInAttach,
            },
        };
        a
    }

    pub fn step(&self) -> &Step {
        &self.step
    }

    /// The links gathered so far (attach, then acceptance).
    pub fn links(&self) -> &Links {
        &self.links
    }

    pub fn into_links(self) -> Links {
        self.links
    }

    /// The media leg id the attach answer named for the caller's offer.
    pub fn media_leg_id(&self) -> Option<&str> {
        self.media_leg_id.as_deref()
    }

    /// The caller's answer to our offer (offerer path only).
    pub fn remote_answer(&self) -> Option<&SdpBlob> {
        self.answer.as_ref()
    }

    /// We are the offerer on this call.
    pub fn offering(&self) -> bool {
        matches!(self.step, Step::SendOffer | Step::AwaitAnswer)
            || (self.step == Step::Adopt && self.answer.is_some())
    }

    pub fn fail(&mut self, why: impl Into<String>) {
        self.step = Step::Failed(why.into());
    }

    /// The attach answer is in.
    pub fn on_attached(&mut self, a: Attached) {
        let Step::Attach { with_offer, .. } = self.step else {
            return;
        };
        self.links = a.links;
        if self.links.get("acceptance").is_none() {
            return self.fail("the attach answer has no acceptance link");
        }
        self.media_leg_id = a.media_leg_id;
        // Once our offer went out with the attach, an offer coming back cannot be answered on
        // the same peer: stay the offerer.
        let offer = if with_offer {
            None
        } else {
            self.ring_offer.take().or(a.offer)
        };
        self.step = match offer {
            Some(o) => Step::AnswerOffer(o),
            None => Step::SendOffer,
        };
    }

    /// The acceptance went through; `inline_answer` is the SDP its acknowledgement carried.
    pub fn on_accepted(&mut self, links: Links, inline_answer: Option<SdpBlob>) {
        self.links.0.extend(links.0);
        match self.step {
            Step::AnswerOffer(_) => self.step = Step::Adopt,
            Step::SendOffer => {
                if self.answer.is_none() {
                    self.answer = inline_answer;
                }
                self.step = if self.answer.is_some() {
                    Step::Adopt
                } else {
                    Step::AwaitAnswer
                };
            }
            _ => {}
        }
    }

    /// A callback of our session arrived (it may come before the acceptance's own answer).
    pub fn on_event(&mut self, ev: &CallEvent) {
        if !matches!(self.step, Step::SendOffer | Step::AwaitAnswer) {
            return;
        }
        let (answer, links) = match ev {
            CallEvent::MediaAnswer { answer, links } => (Some(answer), links),
            CallEvent::Accepted { answer, links, .. } => (answer.as_ref(), links),
            CallEvent::Rejected(_) | CallEvent::Ended(_) => {
                return self.fail("the call ended before the media was negotiated");
            }
            _ => return,
        };
        self.links.0.extend(links.0.clone());
        if let Some(a) = answer
            && self.answer.is_none()
        {
            self.answer = Some(a.clone());
        }
        if self.step == Step::AwaitAnswer && self.answer.is_some() {
            self.step = Step::Adopt;
        }
    }

    /// [`ANSWER_WAIT`] passed without an answer.
    pub fn on_timeout(&mut self) {
        if self.step == Step::AwaitAnswer {
            self.fail("the caller never answered our media offer");
        }
    }
}

/// Watches the exchange: called with a name (`caller-offer`, `our-answer`, `our-offer`,
/// `caller-answer`) and the SDP, for `--dump` / `KELMA_CALL_DUMP`.
pub type Observe<'a> = &'a (dyn Fn(&str, &str) + Send + Sync);

/// Answers `inv` with audio on `media`: attach (when forked), report ringing, then either answer
/// the caller's offer or offer ourselves and apply the caller's answer. `events` carries the
/// callbacks of `s`'s session (the caller's `mediaAnswer`). Returns the call's links, ready for
/// the call manager; `s.media_leg_id` follows the offer's leg.
pub async fn answer_invitation(
    ngc: &NgcClient,
    s: &mut CallSession,
    inv: &CallInvitation,
    media: &MediaSession,
    events: &mut mpsc::UnboundedReceiver<CallEvent>,
    no_offer: NoOffer,
    observe: Observe<'_>,
) -> Result<Links> {
    let modalities = [Modality::Audio];
    let mut a = Answering::new(inv, no_offer);
    let mut rang = false;
    let mut our_offer: Option<String> = None;
    let mut deadline = None;
    loop {
        let step = a.step().clone();
        tracing::debug!(?step, "answering");
        match step {
            Step::Attach {
                require_media,
                with_offer,
            } => {
                let attached = if with_offer {
                    let offer = media.create_offer().await?;
                    observe("our-offer", &offer);
                    let blob = SdpBlob::ngc(offer.as_str());
                    our_offer = Some(offer);
                    ngc.attach_with_offer(s, inv, &blob).await?
                } else {
                    ngc.attach_with(s, inv, require_media).await?
                };
                a.on_attached(attached);
                if let Some(leg) = a.media_leg_id() {
                    s.media_leg_id = leg.to_owned();
                }
            }
            Step::AnswerOffer(offer) => {
                ring(ngc, s, a.links(), &mut rang).await;
                let sdp = offer.decode()?;
                observe("caller-offer", &sdp);
                let answer = media.accept_offer(&sdp).await?;
                observe("our-answer", &answer);
                let links = ngc
                    .accept(s, a.links(), &modalities, &SdpBlob::ngc(answer))
                    .await?;
                a.on_accepted(links, None);
            }
            Step::SendOffer => {
                ring(ngc, s, a.links(), &mut rang).await;
                let offer = match our_offer.take() {
                    Some(o) => o,
                    None => {
                        let o = media.create_offer().await?;
                        observe("our-offer", &o);
                        o
                    }
                };
                let (links, inline) = ngc
                    .accept_with_offer(s, a.links(), &modalities, &SdpBlob::ngc(offer))
                    .await?;
                // An answer that raced the acknowledgement is already queued.
                while let Ok(ev) = events.try_recv() {
                    a.on_event(&ev);
                }
                a.on_accepted(links, inline);
            }
            Step::AwaitAnswer => {
                let until =
                    *deadline.get_or_insert_with(|| tokio::time::Instant::now() + ANSWER_WAIT);
                match tokio::time::timeout_at(until, events.recv()).await {
                    Ok(Some(ev)) => a.on_event(&ev),
                    Ok(None) => a.fail("the calling connection closed"),
                    Err(_) => a.on_timeout(),
                }
            }
            Step::Adopt => {
                if let Some(answer) = a.remote_answer() {
                    let sdp = answer.decode()?;
                    observe("caller-answer", &sdp);
                    media.set_answer(&sdp).await?;
                }
                return Ok(a.into_links());
            }
            Step::Failed(why) => return Err(Error::Protocol(why)),
        }
    }
}

/// Reports ringing once, when the leg has a `progress` link (a failure is only logged).
async fn ring(ngc: &NgcClient, s: &CallSession, links: &Links, rang: &mut bool) {
    if std::mem::replace(rang, true) || links.get("progress").is_none() {
        return;
    }
    if let Err(e) = ngc.send_ringing(s, links).await {
        tracing::warn!(%e, "reporting the call as ringing failed");
    }
}
