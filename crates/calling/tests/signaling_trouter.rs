//! The calling trouter's pure parts: registration, push decoding, replies (offline).

use base64::Engine;
use calling::ngc_trouter::{
    APP_ID, PATH_SUFFIX, TEMPLATE_KEY, decode_call_push, event_ack, registration, reply_frame,
};
use serde_json::{Value, json};
use std::io::Write;

const SURL: &str = "https://trouter.example.invalid/v4/f/AbCd/";
const PAYLOAD: &str =
    r#"{"callNotification":{"from":{"id":"8:orgid:x"},"links":{"attach":"https://a"}}}"#;

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn gzip(s: &str) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(s.as_bytes()).unwrap();
    e.finish().unwrap()
}

fn zlib(s: &str) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(s.as_bytes()).unwrap();
    e.finish().unwrap()
}

fn deflate(s: &str) -> Vec<u8> {
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(s.as_bytes()).unwrap();
    e.finish().unwrap()
}

fn frame(id: i64, url: &str, headers: Value, body: &str) -> String {
    json!({"id": id, "method": "POST", "url": url, "headers": headers, "body": body}).to_string()
}

#[test]
fn registers_only_next_gen_calling_on_the_ngc_path() {
    assert_eq!(APP_ID, "NextGenCalling");
    assert_eq!(PATH_SUFFIX, "NGCallManagerWin");
    assert!(TEMPLATE_KEY.starts_with("DesktopNgc_") && TEMPLATE_KEY.ends_with(":SkypeNgc"));
    let r = registration(SURL, "reg-1");
    assert_eq!(r["clientDescription"]["appId"], "NextGenCalling");
    assert_eq!(r["clientDescription"]["templateKey"], TEMPLATE_KEY);
    assert_eq!(r["registrationId"], "reg-1");
    assert_eq!(
        r["transports"]["TROUTER"][0]["path"],
        format!("{SURL}NGCallManagerWin")
    );
}

#[test]
fn decodes_plain_and_header_gzipped_pushes() {
    let url = format!("{SURL}NGCallManagerWin");
    let p = decode_call_push(&frame(1, &url, json!({}), PAYLOAD)).unwrap();
    assert_eq!(p.id, 1);
    assert_eq!(p.url, url);
    assert_eq!(p.body["callNotification"]["from"]["id"], "8:orgid:x");

    let h = json!({"X-Microsoft-Skype-Content-Encoding": "gzip", "MS-CV": "abc"});
    let p = decode_call_push(&frame(2, &url, h, &b64(&gzip(PAYLOAD)))).unwrap();
    assert_eq!(p.body["callNotification"]["links"]["attach"], "https://a");
    assert_eq!(p.headers.get("MS-CV").map(String::as_str), Some("abc"));
}

#[test]
fn decodes_gzip_without_the_header_by_its_magic() {
    // Roster updates arrive gzipped and base64'd without a content-encoding header.
    let body = json!({"participants": {}}).to_string();
    let url = format!("{SURL}callAgent/s/t/conversation/rosterUpdate/");
    let p = decode_call_push(&frame(3, &url, json!({}), &b64(&gzip(&body)))).unwrap();
    assert_eq!(p.body, json!({"participants": {}}));
}

#[test]
fn unwraps_cp_in_every_deflate_flavour_and_gp() {
    let url = format!("{SURL}NGCallManagerWin");
    for packed in [gzip(PAYLOAD), zlib(PAYLOAD), deflate(PAYLOAD)] {
        let outer = json!({"evt": 107, "cp": b64(&packed)}).to_string();
        let p = decode_call_push(&frame(4, &url, json!({}), &outer)).unwrap();
        assert_eq!(p.body["callNotification"]["from"]["id"], "8:orgid:x");
    }
    let outer = json!({"evt": 107, "gp": b64(PAYLOAD.as_bytes())}).to_string();
    let p = decode_call_push(&frame(5, &url, json!({}), &outer)).unwrap();
    assert_eq!(p.body["callNotification"]["from"]["id"], "8:orgid:x");
}

#[test]
fn empty_and_broken_pushes() {
    let p = decode_call_push(&frame(6, "https://x/", json!({}), "")).unwrap();
    assert_eq!(p.body, Value::Null);
    // Still acked even when the body cannot be read: it keeps id and url, body Null.
    let p = decode_call_push(&frame(7, "https://x/", json!({}), "H4sInot-gzip")).unwrap();
    assert_eq!((p.id, p.body.clone()), (7, Value::Null));
    assert!(decode_call_push("not json").is_none());
    assert!(decode_call_push(r#"{"url":"x"}"#).is_none());
}

#[test]
fn plain_pushes_get_an_empty_200_and_callbacks_echo_routing_headers() {
    let p = decode_call_push(&frame(
        8,
        &format!("{SURL}NGCallManagerWin"),
        json!({}),
        PAYLOAD,
    ))
    .unwrap();
    assert_eq!(
        reply_frame(&p, SURL),
        r#"3:::{"body":"","headers":{},"id":8,"status":200}"#
    );
    let p = decode_call_push(&frame(
        9,
        &format!("{SURL}callAgent/sess-1/ff96380a/call/end/"),
        json!({}),
        "{}",
    ))
    .unwrap();
    assert_eq!(
        reply_frame(&p, SURL),
        r#"3:::{"body":"","headers":{},"id":9,"status":200}"#
    );
    let p = decode_call_push(&frame(
        11,
        &format!("{SURL}callAgent/sess-1/ff96380a/call/end/"),
        json!({"trouter-request": "tr-2", "Other": "x"}),
        "{}",
    ))
    .unwrap();
    assert_eq!(
        reply_frame(&p, SURL),
        r#"3:::{"body":"","headers":{"trouter-request":"tr-2"},"id":11,"status":200}"#
    );
}

#[test]
fn acceptance_is_acknowledged_in_the_reply_with_our_links() {
    let headers = json!({
        "X-Microsoft-Skype-Chain-ID": "chain-1",
        "X-Microsoft-Skype-Message-ID": "msg-1",
        "trouter-request": "tr-1",
        "MS-CV": "cv1",
        "Content-Type": "application/json",
    });
    let url = format!("{SURL}callAgent/sess-1/ff96380a/call/acceptance/");
    let p = decode_call_push(&frame(10, &url, headers, "{}")).unwrap();
    let reply = reply_frame(&p, SURL);
    let v: Value = serde_json::from_str(reply.strip_prefix("3:::").unwrap()).unwrap();
    assert_eq!(v["id"], 10);
    assert_eq!(v["status"], 200);
    assert_eq!(v["headers"]["X-Microsoft-Skype-Chain-ID"], "chain-1");
    assert_eq!(v["headers"]["X-Microsoft-Skype-Message-ID"], "msg-1");
    assert_eq!(v["headers"]["trouter-request"], "tr-1");
    assert_eq!(v["headers"]["MS-CV"], "cv1.0");
    assert!(v["headers"].get("Content-Type").is_none());
    let body: Value = serde_json::from_str(v["body"].as_str().unwrap()).unwrap();
    let links = body["callAcceptanceAcknowledgement"]["links"]
        .as_object()
        .unwrap();
    for name in [
        "mediaRenegotiation",
        "transfer",
        "replacement",
        "retargetCompletion",
        "controlVideoStreaming",
        "updateMediaDescriptions",
    ] {
        let link = links[name].as_str().unwrap();
        assert!(
            link.starts_with(&format!("{SURL}callAgent/sess-1/")),
            "{name}: {link}"
        );
        assert!(link.contains(&format!("/call/{name}")), "{name}: {link}");
    }
}

#[test]
fn event_frames_with_an_ack_id_are_acked() {
    assert_eq!(
        event_ack(r#"5:12+::{"name":"trouter.message_loss"}"#).as_deref(),
        Some("6:12::")
    );
    assert_eq!(event_ack(r#"5:3::{"name":"x"}"#).as_deref(), Some("6:3::"));
    assert_eq!(event_ack(r#"5:::{"name":"x"}"#), None);
    assert_eq!(event_ack(r#"3:::{"id":1}"#), None);
    assert_eq!(event_ack("5:ab+::{}"), None);
}
