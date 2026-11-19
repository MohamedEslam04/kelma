//! Read-only Microsoft Graph collection helper shared by the app modules (Planner, To Do,
//! OneNote, Lists).

use crate::broker::Audience;
use crate::client::Client;
use crate::error::Result;
use reqwest::Method;
use serde_json::Value;

/// The `value` array of a Graph collection (empty when absent).
pub fn values(v: &Value) -> &[Value] {
    v["value"].as_array().map_or(&[], Vec::as_slice)
}

/// A string field, `None` when absent or empty.
pub fn str_of(v: &Value, key: &str) -> Option<String> {
    v[key].as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

/// An ISO timestamp field as Unix milliseconds. Also reads a Graph `dateTimeTimeZone`
/// (`{"dateTime": …, "timeZone": "UTC"}`), whose zone kelma treats as UTC.
pub fn ms_of(v: &Value, key: &str) -> Option<i64> {
    let f = &v[key];
    let s = f.as_str().or_else(|| f["dateTime"].as_str())?;
    crate::time::parse_iso_ms(s)
}

/// `https://graph.microsoft.com/v1.0/<segments…>?<query>` with every segment percent-encoded
/// (ids come from the server). `query` is a ready-made query string such as `$top=50`.
pub fn graph_url(segments: &[&str], query: &str) -> String {
    let mut u = url::Url::parse("https://graph.microsoft.com/v1.0").expect("static url");
    u.path_segments_mut()
        .expect("https url has a path")
        .extend(segments);
    if !query.is_empty() {
        u.set_query(Some(query));
    }
    u.into()
}

impl Client {
    /// The group ids of the teams the user has joined. Teams back their Planner plans, OneNote
    /// notebooks and SharePoint sites with a Microsoft 365 group of the same id.
    pub async fn joined_group_ids(&self) -> Result<Vec<String>> {
        let items = self
            .graph_values(
                "joined teams",
                &graph_url(&["me", "joinedTeams"], "$select=id"),
                5,
            )
            .await?;
        Ok(items.iter().filter_map(|i| str_of(i, "id")).collect())
    }

    /// GETs a Graph collection and follows `@odata.nextLink` for at most `max_pages` pages.
    /// Returns every `value` item. Never writes.
    pub async fn graph_values(
        &self,
        context: &'static str,
        url: &str,
        max_pages: usize,
    ) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut next = Some(url.to_owned());
        for _ in 0..max_pages {
            let Some(u) = next.take() else { break };
            let page = self
                .json_as(context, &Audience::Graph, Method::GET, &u, None)
                .await?;
            out.extend(values(&page).iter().cloned());
            next = page["@odata.nextLink"].as_str().map(str::to_owned);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_values_and_times() {
        let v = json!({"value":[{"a":1}], "t":"2026-01-02T03:04:05Z",
            "due":{"dateTime":"2026-01-02T00:00:00.0000000","timeZone":"UTC"}, "e":""});
        assert_eq!(values(&v).len(), 1);
        assert_eq!(values(&json!({})).len(), 0);
        assert_eq!(ms_of(&v, "t"), Some(1_767_323_045_000));
        assert_eq!(ms_of(&v, "due"), Some(1_767_312_000_000));
        assert_eq!(ms_of(&v, "missing"), None);
        assert_eq!(str_of(&v, "e"), None);
        assert_eq!(str_of(&v, "t").as_deref(), Some("2026-01-02T03:04:05Z"));
    }

    #[test]
    fn urls_encode_ids() {
        assert_eq!(
            graph_url(&["me", "todo", "lists", "AAA=/x y", "tasks"], "$top=5"),
            "https://graph.microsoft.com/v1.0/me/todo/lists/AAA=%2Fx%20y/tasks?$top=5"
        );
        assert_eq!(
            graph_url(&["me", "planner", "tasks"], ""),
            "https://graph.microsoft.com/v1.0/me/planner/tasks"
        );
    }
}
