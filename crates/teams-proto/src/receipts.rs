//! Read receipts: where each member of a chat has read up to.
//!
//! `GET /v1/threads/<id>/consumptionhorizons` answers one `consumptionhorizon` string per member
//! that has read something: `<lastReadMessageId>;<readTimeMs>;<clientMessageId>`. Sources:
//! teams-lite `teams_readstate.rs`, better-teams, rusty `receipts.rs`. Whether people see our
//! receipts is the `readReceiptsEnabled` user property.

use crate::chatsvc::encode_conversation;
use crate::client::Client;
use crate::error::Result;
use reqwest::Method;
use serde_json::Value;

/// How far one member has read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Horizon {
    pub mri: String,
    /// Id of the last message the member has read.
    pub last_read_id: String,
    /// When they read it (ms epoch), 0 when unknown.
    pub read_ms: i64,
}

/// Parses a `consumptionhorizons` answer. The member is named by `id`, `mri` or `user`.
pub fn parse_horizons(v: &Value) -> Vec<Horizon> {
    v.get("consumptionhorizons")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|h| {
                    let mri = ["id", "mri", "user"]
                        .iter()
                        .find_map(|k| h.get(*k).and_then(Value::as_str))
                        .map(|s| crate::model::last_segment(s).to_string())?;
                    let raw = h.get("consumptionhorizon").and_then(Value::as_str)?;
                    let mut parts = raw.split(';');
                    let last_read_id = parts.next().filter(|s| !s.is_empty())?.to_string();
                    let read_ms = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    Some(Horizon {
                        mri,
                        last_read_id,
                        read_ms,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Members other than `self_mri` who have read our message. Message ids are millisecond
/// timestamps, so a member has read it when their last read id is not older. If an id is not
/// numeric, their read time must be at or after the message time instead.
pub fn seen_by(
    horizons: &[Horizon],
    self_mri: &str,
    my_msg_id: &str,
    my_msg_ms: i64,
) -> Vec<String> {
    let mine = my_msg_id.parse::<i64>().ok();
    horizons
        .iter()
        .filter(|h| h.mri != self_mri)
        .filter(|h| match (h.last_read_id.parse::<i64>().ok(), mine) {
            (Some(read), Some(mine)) => read >= mine,
            _ => h.read_ms > 0 && h.read_ms >= my_msg_ms,
        })
        .map(|h| h.mri.clone())
        .collect()
}

/// Reads `readReceiptsEnabled` from `/v1/users/ME/properties`. Teams sends it as a boolean or a
/// string, and it is on unless the user turned it off.
pub fn parse_read_receipts_enabled(v: &Value) -> bool {
    match v.get("readReceiptsEnabled") {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.eq_ignore_ascii_case("false"),
        _ => true,
    }
}

impl Client {
    /// Where each member of a thread has read up to.
    pub async fn consumption_horizons(&self, thread_id: &str) -> Result<Vec<Horizon>> {
        let url = format!(
            "{}/v1/threads/{}/consumptionhorizons",
            self.chat_service().await,
            encode_conversation(thread_id)
        );
        let v = self
            .json("consumption horizons", Method::GET, &url, None)
            .await?;
        Ok(parse_horizons(&v))
    }

    /// Whether our read receipts are on.
    pub async fn read_receipts_enabled(&self) -> Result<bool> {
        let url = format!("{}/v1/users/ME/properties", self.chat_service().await);
        let v = self
            .json("user properties", Method::GET, &url, None)
            .await?;
        Ok(parse_read_receipts_enabled(&v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn h(mri: &str, id: &str, ms: i64) -> Horizon {
        Horizon {
            mri: mri.into(),
            last_read_id: id.into(),
            read_ms: ms,
        }
    }

    #[test]
    fn parses_horizons() {
        let v = json!({"id": "19:x@thread.v2", "version": 1, "consumptionhorizons": [
            {"id": "8:orgid:a", "consumptionhorizon": "1700000000500;1700000001000;123"},
            {"mri": "https://x/contacts/8:orgid:b", "consumptionhorizon": "1700000000100;;"},
            {"user": "8:orgid:c", "consumptionhorizon": ""},
            {"id": "8:orgid:d"}
        ]});
        let p = parse_horizons(&v);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0], h("8:orgid:a", "1700000000500", 1_700_000_001_000));
        assert_eq!(p[1], h("8:orgid:b", "1700000000100", 0));
        assert!(parse_horizons(&json!({"consumptionhorizons": []})).is_empty());
        assert!(parse_horizons(&json!({})).is_empty());
    }

    #[test]
    fn seen_by_compares_ids_and_skips_self() {
        let hs = [
            h("8:orgid:me", "1700000000900", 1),
            h("8:orgid:a", "1700000000500", 5),
            h("8:orgid:b", "1700000000400", 5),
            h("8:orgid:c", "1700000000501", 5),
        ];
        let seen = seen_by(&hs, "8:orgid:me", "1700000000500", 1_700_000_000_500);
        assert_eq!(seen, vec!["8:orgid:a", "8:orgid:c"]);
        assert!(seen_by(&[], "8:orgid:me", "1", 1).is_empty());
    }

    #[test]
    fn seen_by_falls_back_to_read_time_for_odd_ids() {
        let hs = [
            h("8:orgid:a", "abc", 2_000),
            h("8:orgid:b", "abc", 500),
            h("8:orgid:c", "abc", 0),
        ];
        assert_eq!(seen_by(&hs, "8:orgid:me", "1000", 1_000), vec!["8:orgid:a"]);
    }

    #[test]
    fn read_receipts_setting_defaults_on() {
        assert!(parse_read_receipts_enabled(&json!({})));
        assert!(parse_read_receipts_enabled(
            &json!({"readReceiptsEnabled": true})
        ));
        assert!(!parse_read_receipts_enabled(
            &json!({"readReceiptsEnabled": false})
        ));
        assert!(!parse_read_receipts_enabled(
            &json!({"readReceiptsEnabled": "False"})
        ));
        assert!(parse_read_receipts_enabled(
            &json!({"readReceiptsEnabled": "true"})
        ));
    }
}
