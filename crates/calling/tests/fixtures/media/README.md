# Media SDP fixtures

All addresses are documentation ranges (192.0.2.0/24, 198.51.100.0/24, 2001:db8::/32); ICE
credentials, SDES keys, fingerprints and TURN ids are dummies.

- `teams_p2p_offer.sdp`: a one-to-one offer as a native Teams desktop client writes it (SDES and
  DTLS both offered, no `a=setup`, RTCP candidates, `TCP-ACT`/`TCP-PASS`, `MTURNID`, Opus at
  payload type 102, backslash-spelled header-extension URIs). Adapted from rusty-teams (MIT),
  `crates/calling/tests/fixtures/teams_p2p_offer.sdp`, with the TURN ids replaced.
- `teams_meeting_answer.sdp`: an audio-only answer from the Teams media servers (meeting join):
  `RTP/SAVP`, `a=setup:passive`, `OPUS` upper-case, `tcp-pass` and `a=x-candidate-ipv6` relay
  candidates. Shape from the audio section of rusty-teams (MIT)
  `crates/calling/tests/fixtures/teams_answer.sdp`, values rewritten.
- `teams_web_offer.sdp`: an offer as the Teams web client sends it after its own `toMsSdp`
  rewrite (`RTP/SAVP`, `b=CT:4000`, `a=x-ssrc-range`, `a=label`, m-line port and `c=` taken from
  the first host candidate). Written from the facts in teams-lite `NATIVE-CALLING.md` §2.5; no
  text copied, every value invented.
- `teams_meeting_reoffer.sdp`: the media servers' renegotiation offer on a running meeting call,
  on the transport of `teams_meeting_answer.sdp`: audio, a `sendonly` screen-sharing video line
  (mid 3) and an `m=x-data` line (mid 4, `a=x-data-protocol:sctp`). Shape from rusty-teams (MIT)
  `teams_answer.sdp` and teams-lite `NATIVE-CALLING.md` §10 (facts only), values invented.
