//! Microsoft Graph helpers shared by the app modules (Planner, To Do, OneNote, Lists, Workflows):
//! a collection reader and [`Write`], a write request that is built by a pure function (so the
//! tests can check its URL, headers and body) and sent by [`Client::send_write`].

use crate::broker::Audience;
use crate::client::{Client, RawBody};
use crate::error::{Error, Result};
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
/// (`{"dateTime": …, "timeZone": "UTC"}`), whose zone IronTeams treats as UTC.
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

/// What a [`Write`] sends.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteBody {
    None,
    Json(Value),
    /// An HTML document (`Content-Type: text/html`), which OneNote's create page takes.
    Html(String),
}

/// A change to the user's own data on a token audience's API (Graph, Power Automate). Built by
/// the `*_request` functions of the app modules; nothing is sent until [`Client::send_write`].
#[derive(Debug, Clone, PartialEq)]
pub struct Write {
    pub audience: Audience,
    pub method: Method,
    pub url: String,
    pub body: WriteBody,
    /// Extra headers, such as Planner's `If-Match`.
    pub headers: Vec<(&'static str, String)>,
}

impl Write {
    /// A Graph write with a JSON body (or none).
    pub fn graph(method: Method, url: String, body: Option<Value>) -> Write {
        Write {
            audience: Audience::Graph,
            method,
            url,
            body: body.map_or(WriteBody::None, WriteBody::Json),
            headers: Vec::new(),
        }
    }

    /// `If-Match: <etag>`, which Planner needs to change or delete a task.
    pub fn if_match(mut self, etag: &str) -> Write {
        self.headers.push(("If-Match", etag.to_owned()));
        self
    }

    /// The write as one line for logs and tests: method, URL and the body's keys. Never the
    /// body's values, which hold the user's text.
    pub fn describe(&self) -> String {
        let body = match &self.body {
            WriteBody::None => String::new(),
            WriteBody::Html(h) => format!(" (html, {} bytes)", h.len()),
            WriteBody::Json(Value::Object(m)) => format!(
                " (json: {})",
                m.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
            WriteBody::Json(_) => " (json)".to_owned(),
        };
        format!("{} {}{body}", self.method, self.url)
    }
}

impl Client {
    /// Sends a [`Write`]. A 2xx answer is returned as JSON (`Null` when it has no body); any
    /// other status is an error.
    pub async fn send_write(&self, context: &'static str, w: &Write) -> Result<Value> {
        let extra: Vec<(&str, &str)> = w.headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let body = match &w.body {
            WriteBody::None => RawBody::None,
            WriteBody::Json(v) => RawBody::Json(v),
            WriteBody::Html(h) => RawBody::Bytes("text/html", h.clone().into_bytes()),
        };
        let r = self
            .request_raw_as(&w.audience, w.method.clone(), &w.url, body, &extra)
            .await?;
        let text = String::from_utf8_lossy(&r.body);
        if !(200..300).contains(&r.status) {
            return Err(Error::status(context, r.status, &text));
        }
        if text.trim().is_empty() {
            Ok(Value::Null)
        } else {
            Ok(serde_json::from_str(&text)?)
        }
    }

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
    fn a_write_describes_itself_without_its_values() {
        let w = Write::graph(
            Method::PATCH,
            graph_url(&["planner", "tasks", "t1"], ""),
            Some(json!({"title": "secret plans", "percentComplete": 100})),
        )
        .if_match("etag-1");
        let d = w.describe();
        assert!(d.starts_with("PATCH https://graph.microsoft.com/v1.0/planner/tasks/t1"));
        assert!(d.contains("title") && d.contains("percentComplete"));
        assert!(!d.contains("secret"), "{d}");
        assert_eq!(w.headers, [("If-Match", "etag-1".to_owned())]);
        let h = Write {
            body: WriteBody::Html("<p>x</p>".into()),
            ..w.clone()
        };
        assert!(h.describe().ends_with("(html, 8 bytes)"));
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
