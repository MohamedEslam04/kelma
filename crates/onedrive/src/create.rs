//! The "New" menu beyond folders and Office files: OneNote notebooks, Visio drawings, links and
//! Forms. Every creation is first a [`Planned`] request (a pure function of the inputs), so a
//! caller can show or print it ("dry run") before [`create`] sends exactly that request.

use std::fmt;

use reqwest::Method;
use serde_json::{Value, json};
use teams_proto::{Client, Error, Result};

use crate::api::{GRAPH, ItemRef, graph};
use crate::model::DriveItem;
use crate::ops::{create_office_doc, upload_url};
use crate::package::{DocKind, link_file, link_file_name};

/// Where "New > Forms survey" goes. Forms has no Graph API for the Teams client, so the survey
/// is made in the browser.
pub const FORMS_NEW_URL: &str = "https://forms.office.com/";

/// Something the "New" menu creates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewItem {
    /// A blank document (Word, Excel, PowerPoint, Visio, Markdown, Text) in the folder.
    Doc { kind: DocKind, name: Option<String> },
    /// An Internet shortcut (`<name>.url`) to `url` in the folder.
    Link { name: String, url: String },
    /// A OneNote notebook in the user's own notebooks (Graph places it; the folder is ignored).
    Notebook { name: String },
}

/// Body of a planned request.
#[derive(Debug, Clone, PartialEq)]
pub enum PlannedBody {
    Json(Value),
    Bytes(Vec<u8>),
}

/// One request that creates a [`NewItem`], before it is sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    pub method: Method,
    pub url: String,
    pub body: PlannedBody,
}

impl fmt::Display for Planned {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} {}", self.method, self.url)?;
        match &self.body {
            PlannedBody::Json(v) => write!(f, "{v}"),
            PlannedBody::Bytes(b) => match std::str::from_utf8(b) {
                Ok(t) if !t.contains('\0') => write!(f, "{} bytes: {t:?}", b.len()),
                _ => write!(
                    f,
                    "{} bytes (zip package: {})",
                    b.len(),
                    b.starts_with(b"PK")
                ),
            },
        }
    }
}

/// `POST /me/onenote/notebooks` body.
pub fn notebook_body(name: &str) -> Value {
    json!({"displayName": name})
}

/// The request that creates `what` in `parent`.
pub fn plan(parent: &ItemRef, what: &NewItem) -> Planned {
    match what {
        NewItem::Doc { kind, name } => {
            let file = match name {
                Some(n) => kind.file_name(n),
                None => kind.default_name().to_owned(),
            };
            Planned {
                method: Method::PUT,
                url: upload_url(parent, &file),
                body: PlannedBody::Bytes(kind.blank()),
            }
        }
        NewItem::Link { name, url } => Planned {
            method: Method::PUT,
            url: upload_url(parent, &link_file_name(name)),
            body: PlannedBody::Bytes(link_file(url)),
        },
        NewItem::Notebook { name } => Planned {
            method: Method::POST,
            url: format!("{GRAPH}/me/onenote/notebooks"),
            body: PlannedBody::Json(notebook_body(name)),
        },
    }
}

/// A OneNote notebook.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Notebook {
    pub id: String,
    pub name: String,
    /// Opens OneNote for the web.
    pub web_url: String,
    /// `onenote:` URL for a desktop client.
    pub client_url: String,
}

/// Parses a OneNote `notebook`.
pub fn parse_notebook(v: &Value) -> Option<Notebook> {
    let href = |k: &str| {
        v.pointer(&format!("/links/{k}/href"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Some(Notebook {
        id: v["id"].as_str()?.to_owned(),
        name: v["displayName"].as_str().unwrap_or_default().to_owned(),
        web_url: href("oneNoteWebUrl"),
        client_url: href("oneNoteClientUrl"),
    })
}

/// What [`create`] made.
#[derive(Debug, Clone, PartialEq)]
pub enum Created {
    Item(DriveItem),
    Notebook(Notebook),
}

/// Creates `what` in `parent` by sending [`plan`]'s request.
pub async fn create(client: &Client, parent: &ItemRef, what: &NewItem) -> Result<Created> {
    match what {
        NewItem::Doc { kind, name } => create_office_doc(client, parent, *kind, name.as_deref())
            .await
            .map(Created::Item),
        NewItem::Link { name, url } => {
            crate::ops::upload_file(client, parent, &link_file_name(name), link_file(url))
                .await
                .map(Created::Item)
        }
        NewItem::Notebook { .. } => {
            let p = plan(parent, what);
            let PlannedBody::Json(body) = &p.body else {
                return Err(Error::Protocol("notebook plan has no JSON body".into()));
            };
            let v = graph(client, "create_notebook", p.method, &p.url, Some(body)).await?;
            parse_notebook(&v)
                .map(Created::Notebook)
                .ok_or_else(|| Error::Protocol("create_notebook returned no notebook".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_a_visio_drawing_as_an_upload() {
        let p = plan(
            &ItemRef::new("b!d", "01P"),
            &NewItem::Doc {
                kind: DocKind::Visio,
                name: Some("Flow".into()),
            },
        );
        assert_eq!(p.method, Method::PUT);
        assert_eq!(
            p.url,
            "https://graph.microsoft.com/v1.0/drives/b%21d/items/01P:/Flow.vsdx:/content?@microsoft.graph.conflictBehavior=rename"
        );
        assert!(matches!(&p.body, PlannedBody::Bytes(b) if b.starts_with(b"PK")));
        assert!(p.to_string().contains("(zip package: true)"));
        let default = plan(
            &ItemRef::my_root(),
            &NewItem::Doc {
                kind: DocKind::Visio,
                name: None,
            },
        );
        assert!(default.url.ends_with(
            "/me/drive/root:/Drawing.vsdx:/content?@microsoft.graph.conflictBehavior=rename"
        ));
    }

    #[test]
    fn plans_a_link_as_a_url_file() {
        let p = plan(
            &ItemRef::my_root(),
            &NewItem::Link {
                name: "Wiki".into(),
                url: "https://example.com/wiki".into(),
            },
        );
        assert_eq!(p.method, Method::PUT);
        assert!(p.url.contains("/me/drive/root:/Wiki.url:/content"));
        assert_eq!(
            p.body,
            PlannedBody::Bytes(b"[InternetShortcut]\r\nURL=https://example.com/wiki\r\n".to_vec())
        );
        assert!(
            p.to_string().starts_with(
                "PUT https://graph.microsoft.com/v1.0/me/drive/root:/Wiki.url:/content"
            )
        );
        assert!(p.to_string().contains("URL=https://example.com/wiki"));
    }

    #[test]
    fn plans_a_notebook_on_graph_onenote() {
        let p = plan(
            &ItemRef::new("b!d", "01P"),
            &NewItem::Notebook {
                name: "Team notes".into(),
            },
        );
        assert_eq!(p.method, Method::POST);
        assert_eq!(
            p.url,
            "https://graph.microsoft.com/v1.0/me/onenote/notebooks"
        );
        assert_eq!(
            p.body,
            PlannedBody::Json(json!({"displayName": "Team notes"}))
        );
        assert_eq!(
            p.to_string(),
            "POST https://graph.microsoft.com/v1.0/me/onenote/notebooks\n{\"displayName\":\"Team notes\"}"
        );
    }

    #[test]
    fn parses_a_notebook() {
        let n = parse_notebook(&json!({
            "id": "1-abc", "displayName": "Team notes",
            "links": {
                "oneNoteClientUrl": {"href": "onenote:https://contoso-my.sharepoint.com/n"},
                "oneNoteWebUrl": {"href": "https://contoso-my.sharepoint.com/n"}
            }
        }))
        .unwrap();
        assert_eq!(n.name, "Team notes");
        assert_eq!(n.web_url, "https://contoso-my.sharepoint.com/n");
        assert!(n.client_url.starts_with("onenote:"));
        assert!(parse_notebook(&json!({"displayName": "x"})).is_none());
    }

    #[test]
    fn forms_open_in_the_browser() {
        assert!(FORMS_NEW_URL.starts_with("https://forms.office.com"));
    }
}
