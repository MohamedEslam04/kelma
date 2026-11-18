//! SDP as the Teams calling service carries it, and the rewrites between its dialect and what
//! webrtc-rs reads and writes.
//!
//! # The `{contentType, blob}` envelope
//!
//! `application/sdp-ngc-1.0` is a label on ordinary WebRTC SDP, not a different language: the
//! Teams web client (a browser) and mautrix-teams both send the plain SDP text under it, and
//! mautrix-teams sends `application/sdp` for calls through Teams' media servers (teams-lite
//! `NATIVE-CALLING.md` §2.5; mautrix-teams `pkg/msteams/calls.go` `directContentType`). A blob
//! that does not start with `v=` is `base64(raw deflate(sdp))`, and the native clients deflate
//! with a preset dictionary taken from Microsoft's client library (eisbaw/ost
//! `src/calling/sdp_compress.rs`; mautrix-teams `pkg/teamsmedia/sdp.go` `ErrCompressedSDP`).
//! That dictionary is Microsoft's and is not shipped here: such a blob is a named error, and
//! [`SdpBlob::decode_with_dictionary`] reads it when the caller has a dictionary of its own.
//!
//! # The dialect
//!
//! The web client rewrites every SDP it sends (`toMsSdp`) and every one it receives
//! (`fromMsSdp`), and the service refuses SDP that has not been through it; teams-lite measured
//! each rule against the live service (`NATIVE-CALLING.md` §2.5, facts only):
//! - every m-line's profile is `RTP/SAVP` on the wire (`410 UnrecognizedTransportProfile`
//!   otherwise) and is restored to `UDP/TLS/RTP/SAVPF` for a DTLS section on the way back;
//! - ICE-TCP candidates fold their role into the transport (`tcp-act`, `tcp-pass`, `tcp-so`),
//!   and IPv6 candidates may arrive as `a=x-candidate-ipv6`;
//! - two header-extension URIs are spelled with backslashes;
//! - an offer carries `b=CT:4000` before `t=`, `a=msid-semantic: WMS *`, the bundle's real port
//!   and address on each m-line and `c=`, `a=rtcp:<port>` and the fingerprint per section; an
//!   answer carries none of that (`SdpParsingFailure`);
//! - each section states `a=label` and its SSRC as `a=x-ssrc-range:N-N` (Teams drops RTP from
//!   SSRCs outside the advertised range, mautrix-teams `pkg/teamsmedia/sdp.go`).
//!
//! On the way in, candidates are reduced to what webrtc-rs uses: UDP, component 1 (rtcp-mux makes
//! the RTCP component redundant), without vendor extras such as `MTURNID`.

use std::collections::HashMap;
use std::net::IpAddr;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use flate2::{Decompress, FlushDecompress, Status};

use crate::{Error, Result, SdpBlob};

/// Plain SDP, as mautrix-teams labels calls through Teams' media servers.
pub const CONTENT_TYPE_SDP: &str = "application/sdp";
/// The label the web client puts on its SDP, and that direct (peer-to-peer) calls use.
pub const CONTENT_TYPE_NGC: &str = "application/sdp-ngc-1.0";

const MS_PROFILE: &str = "RTP/SAVP";
const DTLS_PROFILE: &str = "UDP/TLS/RTP/SAVPF";
const SESSION_BANDWIDTH: &str = "b=CT:4000";
/// The payload a declined section of an offer names (the web client's own stub).
const STUB_PAYLOAD: &str = "34";
const ENCODED_EXTENSIONS: [&str; 2] = [
    "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time",
    "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01",
];
/// `tcptype` roles and the transport names Teams folds them into.
const TCP_TYPES: [(&str, &str); 3] = [
    ("active", "tcp-act"),
    ("passive", "tcp-pass"),
    ("so", "tcp-so"),
];
/// Candidate extensions the web client deletes before a candidate goes out.
const CANDIDATE_EXTRAS: [&str; 3] = ["generation", "network-id", "network-cost"];
const MAX_INFLATED: usize = 1 << 20;
const DIRECTIONS: [&str; 4] = ["sendrecv", "sendonly", "recvonly", "inactive"];

impl SdpBlob {
    /// A blob of plain SDP labelled `application/sdp`.
    pub fn sdp(sdp: impl Into<String>) -> Self {
        Self {
            content_type: CONTENT_TYPE_SDP.to_owned(),
            blob: sdp.into(),
        }
    }

    /// A blob of plain SDP labelled `application/sdp-ngc-1.0`, as the web client sends it.
    pub fn ngc(sdp: impl Into<String>) -> Self {
        Self {
            content_type: CONTENT_TYPE_NGC.to_owned(),
            blob: sdp.into(),
        }
    }

    /// The SDP text of a blob of either content type: plain text, or base64 raw deflate.
    pub fn decode(&self) -> Result<String> {
        decode(&self.blob, None)
    }

    /// [`Self::decode`], also reading a blob deflated with a preset dictionary.
    pub fn decode_with_dictionary(&self, dictionary: &[u8]) -> Result<String> {
        decode(&self.blob, Some(dictionary))
    }
}

fn media(msg: impl Into<String>) -> Error {
    Error::Media(msg.into())
}

fn decode(blob: &str, dictionary: Option<&[u8]>) -> Result<String> {
    let text = blob.trim_start();
    if text.starts_with("v=") {
        return Ok(text.to_owned());
    }
    let raw = B64
        .decode(text.trim_end())
        .map_err(|e| media(format!("SDP blob is neither SDP text nor base64: {e}")))?;
    if let Some(sdp) = inflate(&raw, None) {
        return Ok(sdp);
    }
    match dictionary {
        Some(dict) => inflate(&raw, Some(dict))
            .ok_or_else(|| media("SDP blob does not inflate with the given dictionary")),
        None => Err(media(
            "SDP blob is deflated with Teams' preset dictionary, which kelma does not ship",
        )),
    }
}

/// Raw inflate, with an optional preset dictionary; `None` unless the result is SDP text.
fn inflate(data: &[u8], dictionary: Option<&[u8]>) -> Option<String> {
    let mut d = Decompress::new(false);
    if let Some(dict) = dictionary {
        d.set_dictionary(dict).ok()?;
    }
    let mut out = Vec::with_capacity(data.len() * 4 + 256);
    loop {
        let (before_in, before_out) = (d.total_in(), d.total_out());
        let consumed = usize::try_from(before_in).ok()?;
        let status = d
            .decompress_vec(&data[consumed..], &mut out, FlushDecompress::Finish)
            .ok()?;
        if status == Status::StreamEnd {
            break;
        }
        if out.len() >= MAX_INFLATED {
            return None;
        }
        if out.len() == out.capacity() {
            out.reserve(out.capacity());
        } else if d.total_in() == before_in && d.total_out() == before_out {
            // Truncated stream: no progress with room to spare.
            return None;
        }
    }
    String::from_utf8(out).ok().filter(|s| s.starts_with("v="))
}

/// An SDP split into its session lines and its media sections (each starting with `m=`).
struct Parsed {
    session: Vec<String>,
    sections: Vec<Vec<String>>,
}

impl Parsed {
    fn new(sdp: &str) -> Self {
        let mut session = Vec::new();
        let mut sections: Vec<Vec<String>> = Vec::new();
        for line in sdp.lines().map(str::trim_end).filter(|l| !l.is_empty()) {
            if line.starts_with("m=") {
                sections.push(vec![line.to_owned()]);
            } else if let Some(section) = sections.last_mut() {
                section.push(line.to_owned());
            } else {
                session.push(line.to_owned());
            }
        }
        Self { session, sections }
    }
}

/// `m=<kind> <port> <profile> <formats>`.
struct MediaLine<'a> {
    kind: &'a str,
    port: &'a str,
    formats: &'a str,
}

impl<'a> MediaLine<'a> {
    fn parse(line: &'a str) -> Option<Self> {
        let mut parts = line.strip_prefix("m=")?.splitn(4, ' ');
        let kind = parts.next()?;
        let port = parts.next()?;
        let _profile = parts.next()?;
        Some(Self {
            kind,
            port,
            formats: parts.next().unwrap_or(""),
        })
    }

    fn with(&self, port: &str, profile: &str) -> String {
        if self.formats.is_empty() {
            format!("m={} {port} {profile}", self.kind)
        } else {
            format!("m={} {port} {profile} {}", self.kind, self.formats)
        }
    }
}

fn join(lines: Vec<String>) -> String {
    let mut out = lines.join("\r\n");
    out.push_str("\r\n");
    out
}

fn attr<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    line.strip_prefix("a=")?
        .strip_prefix(name)?
        .strip_prefix(':')
}

// --- inbound ---

/// Teams' SDP (an offer or an answer) rewritten so webrtc-rs accepts it. Fails on SDP keyed with
/// SDES alone: webrtc-rs only does DTLS-SRTP.
pub fn from_teams(sdp: &str) -> Result<String> {
    let parsed = Parsed::new(sdp);
    if parsed.sections.is_empty() {
        return Err(media("remote SDP has no media section"));
    }
    let dtls = parsed
        .session
        .iter()
        .chain(parsed.sections.iter().flatten())
        .any(|l| l.starts_with("a=fingerprint:"));
    if !dtls {
        return Err(media(
            "remote SDP has no DTLS fingerprint (SDES-only keying); webrtc-rs supports DTLS-SRTP only",
        ));
    }
    let mut out: Vec<String> = parsed.session.iter().filter_map(|l| inbound(l)).collect();
    for section in &parsed.sections {
        let m = MediaLine::parse(&section[0]).ok_or_else(|| media("malformed m= line"))?;
        let profile = section[0].split(' ').nth(2).unwrap_or_default();
        if profile == MS_PROFILE || profile == "RTP/SAVPF" {
            out.push(m.with(m.port, DTLS_PROFILE));
        } else {
            out.push(section[0].clone());
        }
        out.extend(section[1..].iter().filter_map(|l| inbound(l)));
        // Native clients leave the direction out (RFC 4566 default: sendrecv); webrtc-rs then
        // drops the section from its answer.
        let directed = section[1..]
            .iter()
            .any(|l| DIRECTIONS.iter().any(|d| l.strip_prefix("a=") == Some(*d)));
        if !directed && m.port != "0" {
            out.push("a=sendrecv".to_owned());
        }
    }
    Ok(join(out))
}

fn inbound(line: &str) -> Option<String> {
    if let Some(c) = attr(line, "candidate").or_else(|| attr(line, "x-candidate-ipv6")) {
        return candidate_from_teams(c).map(|c| format!("a=candidate:{c}"));
    }
    if line.starts_with("a=extmap:") {
        return Some(line.replace('\\', "/"));
    }
    Some(line.to_owned())
}

/// A UDP RTP candidate in standard form; `None` for TCP, RTCP-component and unreadable ones.
fn candidate_from_teams(value: &str) -> Option<String> {
    let f: Vec<&str> = value.split_whitespace().collect();
    if f.len() < 8 || f[6] != "typ" || f[1] != "1" || !f[2].eq_ignore_ascii_case("udp") {
        return None;
    }
    let mut out = format!("{} 1 udp {} {} {} typ {}", f[0], f[3], f[4], f[5], f[7]);
    let extra: HashMap<&str, &str> = f[8..]
        .chunks(2)
        .filter_map(|kv| Some((kv[0], *kv.get(1)?)))
        .collect();
    if let (Some(addr), Some(port)) = (extra.get("raddr"), extra.get("rport"))
        && addr.parse::<IpAddr>().is_ok()
        && port.parse::<u16>().is_ok()
    {
        out.push_str(&format!(" raddr {addr} rport {port}"));
    }
    Some(out)
}

// --- outbound ---

/// Our (webrtc-rs) offer rewritten into the dialect the Teams calling service reads.
pub fn to_teams_offer(sdp: &str) -> String {
    to_teams(sdp, true, &HashMap::new())
}

/// Our (webrtc-rs) answer to `remote_offer` rewritten into the Teams dialect; each section keeps
/// the `a=label` the offer gave it.
pub fn to_teams_answer(sdp: &str, remote_offer: &str) -> String {
    to_teams(sdp, false, &labels_by_mid(remote_offer))
}

fn labels_by_mid(sdp: &str) -> HashMap<String, String> {
    Parsed::new(sdp)
        .sections
        .iter()
        .filter_map(|s| {
            let mid = s.iter().find_map(|l| attr(l, "mid"))?;
            let label = s.iter().find_map(|l| attr(l, "label"))?;
            Some((mid.to_owned(), label.to_owned()))
        })
        .collect()
}

fn default_label(kind: &str) -> Option<&'static str> {
    match kind {
        "audio" => Some("main-audio"),
        "video" => Some("main-video"),
        _ => None,
    }
}

/// The port and `c=` value of the bundle: the first section's first UDP host candidate (else its
/// first UDP candidate), which the web client writes on the m-line in place of `9`/`0.0.0.0`.
fn bundle_transport(sections: &[Vec<String>]) -> Option<(String, String)> {
    let section = sections
        .iter()
        .find(|s| s.iter().any(|l| attr(l, "candidate").is_some()))?;
    let udp: Vec<Vec<&str>> = section
        .iter()
        .filter_map(|l| attr(l, "candidate"))
        .map(|c| c.split_whitespace().collect::<Vec<_>>())
        .filter(|f| f.len() >= 8 && f[1] == "1" && f[2].eq_ignore_ascii_case("udp"))
        .collect();
    let best = udp.iter().find(|f| f[7] == "host").or(udp.first())?;
    let family = if best[4].contains(':') { "IP6" } else { "IP4" };
    Some((best[5].to_owned(), format!("IN {family} {}", best[4])))
}

fn to_teams(sdp: &str, offer: bool, labels: &HashMap<String, String>) -> String {
    let parsed = Parsed::new(sdp);
    let fingerprint = parsed
        .session
        .iter()
        .find(|l| l.starts_with("a=fingerprint:"))
        .cloned();
    let transport = bundle_transport(&parsed.sections);
    let mut out = Vec::new();
    for line in &parsed.session {
        if offer && line.starts_with("b=") {
            continue;
        }
        if offer && line.starts_with("t=") {
            out.push(SESSION_BANDWIDTH.to_owned());
        }
        if offer && line.starts_with("a=msid-semantic:") {
            out.push("a=msid-semantic: WMS *".to_owned());
            continue;
        }
        out.push(line.clone());
    }
    if offer
        && !parsed
            .session
            .iter()
            .any(|l| l.starts_with("a=msid-semantic:"))
    {
        out.push("a=msid-semantic: WMS *".to_owned());
    }
    for section in &parsed.sections {
        let Some(m) = MediaLine::parse(&section[0]) else {
            out.extend(section.iter().cloned());
            continue;
        };
        let mid = section.iter().find_map(|l| attr(l, "mid"));
        let label = mid
            .and_then(|mid| labels.get(mid).map(String::as_str))
            .or_else(|| default_label(m.kind));
        if offer && m.port == "0" {
            out.push(format!("m={} 0 {MS_PROFILE} {STUB_PAYLOAD}", m.kind));
            out.push("c=IN IP4 0.0.0.0".to_owned());
            out.extend(mid.map(|mid| format!("a=mid:{mid}")));
            out.extend(label.map(|label| format!("a=label:{label}")));
            continue;
        }
        let placed = transport.as_ref().filter(|_| m.port != "0");
        match placed {
            Some((port, connection)) => {
                out.push(m.with(port, MS_PROFILE));
                out.push(format!("c={connection}"));
                if offer {
                    out.push(format!("a=rtcp:{port}"));
                }
            }
            None => out.push(m.with(m.port, MS_PROFILE)),
        }
        let has_fingerprint = section.iter().any(|l| l.starts_with("a=fingerprint:"));
        if offer && !has_fingerprint {
            out.extend(fingerprint.clone());
        }
        let mut has_label = false;
        let mut has_range = false;
        let mut ssrc = None;
        for line in &section[1..] {
            if (placed.is_some() && line.starts_with("c=")) || line.starts_with("a=rtcp:") {
                continue;
            }
            has_label |= line.starts_with("a=label:");
            has_range |= line.starts_with("a=x-ssrc-range:");
            if ssrc.is_none() {
                ssrc = attr(line, "ssrc")
                    .and_then(|v| v.split(' ').next())
                    .map(str::to_owned);
            }
            if let Some(c) = attr(line, "candidate") {
                // rtcp-mux: the RTCP component's twin (webrtc-rs writes one) goes nowhere.
                if c.split_whitespace().nth(1) == Some("1") {
                    out.push(format!("a=candidate:{}", candidate_to_teams(c)));
                }
            } else if line.starts_with("a=extmap:") {
                out.push(extmap_to_teams(line));
            } else {
                out.push(line.clone());
            }
        }
        if let (false, Some(label)) = (has_label, label) {
            out.push(format!("a=label:{label}"));
        }
        if let (false, Some(ssrc)) = (has_range, ssrc) {
            out.push(format!("a=x-ssrc-range:{ssrc}-{ssrc}"));
        }
    }
    join(out)
}

fn candidate_to_teams(value: &str) -> String {
    let mut f: Vec<&str> = value.split_whitespace().collect();
    if f.len() > 2 {
        if f[2].eq_ignore_ascii_case("udp") {
            f[2] = "UDP";
        } else if f[2].eq_ignore_ascii_case("tcp")
            && let Some(at) = f.iter().position(|x| *x == "tcptype")
            && let Some(&(_, ms)) = f
                .get(at + 1)
                .and_then(|role| TCP_TYPES.iter().find(|(jsep, _)| jsep == role))
        {
            f[2] = ms;
            f.drain(at..at + 2);
        }
    }
    let mut out = Vec::with_capacity(f.len());
    let mut i = 0;
    while i < f.len() {
        if i >= 8 && CANDIDATE_EXTRAS.contains(&f[i]) {
            i += 2;
            continue;
        }
        out.push(f[i]);
        i += 1;
    }
    out.join(" ")
}

fn extmap_to_teams(line: &str) -> String {
    for uri in ENCODED_EXTENSIONS {
        if let Some(head) = line.strip_suffix(uri)
            && head.ends_with(' ')
        {
            return format!("{head}{}", uri.replace('/', "\\"));
        }
    }
    line.to_owned()
}
