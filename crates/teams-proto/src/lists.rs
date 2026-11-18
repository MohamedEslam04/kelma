//! Microsoft Lists (SharePoint lists), read only, through Graph `/sites/{id}/lists`.
//!
//! Sites are found through the teams the user joined (`/groups/{id}/sites/root`). Items come with
//! their `fields` expanded. Kelma never adds, edits or deletes an item.

use crate::client::Client;
use crate::error::Result;
use crate::graph::{graph_url, ms_of, str_of, values};
use serde_json::{Map, Value};

/// A SharePoint site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    pub id: String,
    pub name: String,
    pub web_url: String,
}

/// A list or library of a site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpList {
    pub id: String,
    pub name: String,
    pub web_url: Option<String>,
    /// `genericList`, `documentLibrary`, `tasks`, …
    pub template: Option<String>,
    /// Hidden lists are system lists (site assets, style library, …).
    pub hidden: bool,
    pub modified_ms: Option<i64>,
}

impl SpList {
    /// A user-facing list: not hidden and not a document library.
    pub fn is_user_list(&self) -> bool {
        !self.hidden && self.template.as_deref() != Some("documentLibrary")
    }
}

/// A column of a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// Internal name, the key in an item's fields.
    pub name: String,
    pub display_name: String,
    /// `text`, `number`, `boolean`, `choice`, `dateTime`, `currency`, `lookup`,
    /// `personOrGroup`, `calculated`, `hyperlinkOrPicture`, …
    pub kind: String,
    pub hidden: bool,
    pub read_only: bool,
}

/// An item with its fields.
#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    pub id: String,
    pub fields: Map<String, Value>,
}

/// Sites from a Graph collection.
pub fn parse_sites(v: &Value) -> Vec<Site> {
    values(v).iter().filter_map(parse_site).collect()
}

fn parse_site(s: &Value) -> Option<Site> {
    Some(Site {
        id: str_of(s, "id")?,
        name: str_of(s, "displayName")
            .or_else(|| str_of(s, "name"))
            .unwrap_or_default(),
        web_url: str_of(s, "webUrl").unwrap_or_default(),
    })
}

/// Lists from a Graph collection.
pub fn parse_lists(v: &Value) -> Vec<SpList> {
    values(v)
        .iter()
        .filter_map(|l| {
            Some(SpList {
                id: str_of(l, "id")?,
                name: str_of(l, "displayName")
                    .or_else(|| str_of(l, "name"))
                    .unwrap_or_default(),
                web_url: str_of(l, "webUrl"),
                template: str_of(&l["list"], "template"),
                hidden: l["list"]["hidden"].as_bool().unwrap_or(false),
                modified_ms: ms_of(l, "lastModifiedDateTime"),
            })
        })
        .collect()
}

/// The facets that name a column's type; the one present on the column wins.
const COLUMN_KINDS: [&str; 14] = [
    "text",
    "number",
    "boolean",
    "choice",
    "dateTime",
    "currency",
    "lookup",
    "personOrGroup",
    "calculated",
    "hyperlinkOrPicture",
    "thumbnail",
    "geolocation",
    "contentApprovalStatus",
    "term",
];

/// Columns from a Graph collection.
pub fn parse_columns(v: &Value) -> Vec<Column> {
    values(v)
        .iter()
        .filter_map(|c| {
            Some(Column {
                name: str_of(c, "name")?,
                display_name: str_of(c, "displayName").unwrap_or_default(),
                kind: COLUMN_KINDS
                    .iter()
                    .find(|k| c.get(**k).is_some_and(|f| f.is_object()))
                    .map_or("other", |k| k)
                    .to_owned(),
                hidden: c["hidden"].as_bool().unwrap_or(false),
                read_only: c["readOnly"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

/// Items (with expanded `fields`) from a Graph collection.
pub fn parse_items(v: &Value) -> Vec<ListItem> {
    values(v)
        .iter()
        .filter_map(|i| {
            Some(ListItem {
                id: str_of(i, "id")?,
                fields: i["fields"].as_object().cloned().unwrap_or_default(),
            })
        })
        .collect()
}

/// A field value as one line of text: strings and numbers as they are, booleans as yes and no,
/// people, lookups and choice lists by their display value.
pub fn field_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "yes" } else { "no" }.into(),
        Value::Number(n) => n.to_string(),
        Value::Array(a) => a.iter().map(field_text).collect::<Vec<_>>().join(", "),
        Value::Object(o) => ["LookupValue", "Title", "Description", "Url"]
            .iter()
            .find_map(|k| o.get(*k).and_then(Value::as_str))
            .unwrap_or_default()
            .to_owned(),
    }
}

fn collection(items: Vec<Value>) -> Value {
    serde_json::json!({ "value": items })
}

const PAGES: usize = 10;

impl Client {
    /// The root sites of the teams the user has joined.
    pub async fn team_sites(&self) -> Result<Vec<Site>> {
        let mut out: Vec<Site> = Vec::new();
        for g in self.joined_group_ids().await? {
            let url = graph_url(&["groups", &g, "sites", "root"], "");
            // A team without a site answers an error; skip it rather than fail the list.
            let Ok(v) = self
                .json_as(
                    "team site",
                    &crate::broker::Audience::Graph,
                    reqwest::Method::GET,
                    &url,
                    None,
                )
                .await
            else {
                continue;
            };
            if let Some(s) = parse_site(&v)
                && !out.iter().any(|o| o.id == s.id)
            {
                out.push(s);
            }
        }
        Ok(out)
    }

    /// The lists and libraries of a site.
    pub async fn site_lists(&self, site_id: &str) -> Result<Vec<SpList>> {
        let url = graph_url(
            &["sites", site_id, "lists"],
            "$select=id,displayName,webUrl,list,lastModifiedDateTime",
        );
        let items = self.graph_values("site lists", &url, PAGES).await?;
        Ok(parse_lists(&collection(items)))
    }

    /// The columns of a list.
    pub async fn list_columns(&self, site_id: &str, list_id: &str) -> Result<Vec<Column>> {
        let url = graph_url(&["sites", site_id, "lists", list_id, "columns"], "");
        let items = self.graph_values("list columns", &url, PAGES).await?;
        Ok(parse_columns(&collection(items)))
    }

    /// Up to `max_pages` pages of a list's items, with their fields.
    pub async fn list_items(
        &self,
        site_id: &str,
        list_id: &str,
        max_pages: usize,
    ) -> Result<Vec<ListItem>> {
        let url = graph_url(
            &["sites", site_id, "lists", list_id, "items"],
            "expand=fields&$top=100",
        );
        let items = self.graph_values("list items", &url, max_pages).await?;
        Ok(parse_items(&collection(items)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_sites_and_lists() {
        let sites = parse_sites(&json!({"value":[
            {"id":"host,sid,wid","displayName":"Team A","webUrl":"https://example.invalid/sites/a"}]}));
        assert_eq!(sites[0].name, "Team A");
        let lists = parse_lists(&json!({"value":[
            {"id":"l1","displayName":"Issues","webUrl":"https://example.invalid/l1",
             "lastModifiedDateTime":"2026-01-02T00:00:00Z","list":{"template":"genericList","hidden":false}},
            {"id":"l2","name":"Shared Documents","list":{"template":"documentLibrary","hidden":false}},
            {"id":"l3","name":"appdata","list":{"template":"genericList","hidden":true}}]}));
        assert_eq!(lists.len(), 3);
        assert!(lists[0].is_user_list());
        assert_eq!(lists[0].modified_ms, Some(1_767_312_000_000));
        assert!(!lists[1].is_user_list());
        assert_eq!(lists[1].name, "Shared Documents");
        assert!(!lists[2].is_user_list());
    }

    #[test]
    fn parses_columns_by_their_type_facet() {
        let cols = parse_columns(&json!({"value":[
            {"name":"Title","displayName":"Title","text":{},"readOnly":false},
            {"name":"Status","displayName":"Status","choice":{"choices":["Open","Done"]}},
            {"name":"Due","displayName":"Due date","dateTime":{"format":"dateOnly"}},
            {"name":"_UIVersion","hidden":true,"readOnly":true},
            {"displayName":"no name"}]}));
        assert_eq!(cols.len(), 4);
        assert_eq!(cols[0].kind, "text");
        assert_eq!(cols[1].kind, "choice");
        assert_eq!(cols[2].kind, "dateTime");
        assert_eq!(cols[3].kind, "other");
        assert!(cols[3].hidden && cols[3].read_only);
    }

    #[test]
    fn parses_items_and_formats_fields() {
        let items = parse_items(&json!({"value":[
            {"id":"1","fields":{"Title":"Fix login","Done":true,"Count":3,
              "Owner":{"LookupValue":"Sam","LookupId":7},"Tags":["a","b"],"Empty":null}},
            {"id":"2"}]}));
        assert_eq!(items.len(), 2);
        let f = &items[0].fields;
        assert_eq!(field_text(&f["Title"]), "Fix login");
        assert_eq!(field_text(&f["Done"]), "yes");
        assert_eq!(field_text(&f["Count"]), "3");
        assert_eq!(field_text(&f["Owner"]), "Sam");
        assert_eq!(field_text(&f["Tags"]), "a, b");
        assert_eq!(field_text(&f["Empty"]), "");
        assert!(items[1].fields.is_empty());
    }
}
