//! Shared request plumbing for the write side: Graph and Outlook REST calls with the same
//! `Prefer` header as the read side, so ids and times stay comparable.

use reqwest::Method;
use serde_json::Value;
use teams_proto::broker::Audience;
use teams_proto::{Client, Error, Result};

pub(crate) const GRAPH: &str = "https://graph.microsoft.com/v1.0";

/// Times in UTC and immutable ids, the same as the read side.
const PREFER: (&str, &str) = ("Prefer", "outlook.timezone=\"UTC\", IdType=\"ImmutableId\"");

/// Percent-encodes one URL path segment (ids contain `=`, `+` and `/`).
pub(crate) fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Raw response of a Graph or Outlook call, so a caller can look at the status itself.
pub(crate) async fn send_raw(
    client: &Client,
    aud: &Audience,
    method: Method,
    url: &str,
    body: Option<&Value>,
) -> Result<(u16, String)> {
    client.request_as(aud, method, url, body, &[PREFER]).await
}

/// A call that must succeed. An empty body (202/204) becomes `Value::Null`.
pub(crate) async fn send(
    client: &Client,
    aud: &Audience,
    context: &'static str,
    method: Method,
    url: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let (status, text) = send_raw(client, aud, method, url, body).await?;
    parse(context, status, &text)
}

pub(crate) fn parse(context: &'static str, status: u16, text: &str) -> Result<Value> {
    if !(200..300).contains(&status) {
        return Err(Error::status(context, status, text));
    }
    if text.trim().is_empty() {
        Ok(Value::Null)
    } else {
        Ok(serde_json::from_str(text)?)
    }
}

pub(crate) async fn graph(
    client: &Client,
    context: &'static str,
    method: Method,
    path: &str,
    body: Option<&Value>,
) -> Result<Value> {
    send(
        client,
        &Audience::Graph,
        context,
        method,
        &format!("{GRAPH}{path}"),
        body,
    )
    .await
}

/// A Graph write described but not sent, so a `--dry-run` can print it and a UI can confirm
/// it. `path` is under `https://graph.microsoft.com/v1.0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRequest {
    pub method: Method,
    pub path: String,
    pub body: Option<Value>,
}

impl GraphRequest {
    /// Sends it; an empty answer is `Value::Null`.
    pub async fn send(&self, client: &Client, context: &'static str) -> Result<Value> {
        graph(
            client,
            context,
            self.method.clone(),
            &self.path,
            self.body.as_ref(),
        )
        .await
    }
}

/// `{"dateTime": "2026-12-18T10:00:00", "timeZone": "UTC"}`
pub(crate) fn date_time(wall: &str, tz: &str) -> Value {
    serde_json::json!({"dateTime": wall, "timeZone": tz})
}

/// `2026-12-18T10:00:00` from Unix milliseconds in UTC.
pub(crate) fn wall_utc(ms: i64) -> String {
    crate::range::format_iso_ms(ms)[..19].to_string()
}

pub(crate) fn email_address(name: &str, email: &str) -> Value {
    let mut a = serde_json::json!({"address": email});
    if !name.is_empty() {
        a["name"] = name.into();
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_path_segments() {
        assert_eq!(enc("AAMk+/=-_x"), "AAMk%2B%2F%3D-_x");
        assert_eq!(enc("a b@c"), "a%20b%40c");
    }

    #[test]
    fn wall_clock_has_no_zone_or_millis() {
        assert_eq!(wall_utc(1_794_960_000_000), "2026-11-18T00:00:00");
    }

    #[test]
    fn non_2xx_is_an_error_and_empty_is_null() {
        assert!(parse("x", 403, "{}").is_err());
        assert_eq!(parse("x", 204, "").unwrap(), Value::Null);
        assert_eq!(parse("x", 200, "{\"a\":1}").unwrap()["a"], 1);
    }
}
