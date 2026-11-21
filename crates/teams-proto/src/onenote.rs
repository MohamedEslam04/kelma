//! OneNote through Microsoft Graph: notebooks, sections, pages and a page's HTML, and the request
//! that creates a page (`create_page_request`).
//!
//! Personal notebooks come from `/me/onenote`, team notebooks from `/groups/{id}/onenote`. A
//! notebook, section and page each carry the Graph URL of their children (`sectionsUrl`,
//! `pagesUrl`, `contentUrl`), so the same calls work for both kinds.
//! The `onenote.com` audience (`https://www.onenote.com/api/v1.0/me/notes/...`) answers the same
//! data for personal notebooks, but Graph covers both, so IronTeams uses Graph only. Editing a page
//! is left to the web host; IronTeams only adds a new page.

use crate::broker::Audience;
use crate::client::Client;
use crate::error::{Error, Result};
use crate::graph::{Write, WriteBody, graph_url, ms_of, str_of, values};
use reqwest::Method;
use serde_json::Value;

/// A notebook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notebook {
    pub id: String,
    pub name: String,
    pub modified_ms: Option<i64>,
    /// Opens the notebook in OneNote on the web.
    pub web_url: Option<String>,
    /// Graph URL of the notebook's top-level sections.
    pub sections_url: Option<String>,
    /// Graph URL of the notebook's section groups. Team notebooks keep their sections in
    /// section groups, so the top-level list is often empty.
    pub section_groups_url: Option<String>,
}

/// A folder of sections inside a notebook or another section group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionGroup {
    pub id: String,
    pub name: String,
    pub sections_url: Option<String>,
    pub section_groups_url: Option<String>,
}

/// A section of a notebook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub id: String,
    pub name: String,
    pub modified_ms: Option<i64>,
    /// Graph URL of the section's pages.
    pub pages_url: Option<String>,
    /// The section groups it sits in, outermost first, joined with " / " (empty at top level).
    pub group_path: String,
}

/// A page of a section (without its content).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub id: String,
    pub title: String,
    pub modified_ms: Option<i64>,
    pub web_url: Option<String>,
    /// Graph URL of the page's HTML.
    pub content_url: Option<String>,
}

/// Notebooks from a Graph collection.
pub fn parse_notebooks(v: &Value) -> Vec<Notebook> {
    values(v)
        .iter()
        .filter_map(|n| {
            Some(Notebook {
                id: str_of(n, "id")?,
                name: str_of(n, "displayName").unwrap_or_default(),
                modified_ms: ms_of(n, "lastModifiedDateTime"),
                web_url: str_of(&n["links"]["oneNoteWebUrl"], "href"),
                sections_url: str_of(n, "sectionsUrl"),
                section_groups_url: str_of(n, "sectionGroupsUrl"),
            })
        })
        .collect()
}

/// Sections from a Graph collection.
pub fn parse_sections(v: &Value) -> Vec<Section> {
    values(v)
        .iter()
        .filter_map(|s| {
            Some(Section {
                id: str_of(s, "id")?,
                name: str_of(s, "displayName").unwrap_or_default(),
                modified_ms: ms_of(s, "lastModifiedDateTime"),
                pages_url: str_of(s, "pagesUrl"),
                group_path: String::new(),
            })
        })
        .collect()
}

/// Section groups from a Graph collection.
pub fn parse_section_groups(v: &Value) -> Vec<SectionGroup> {
    values(v)
        .iter()
        .filter_map(|g| {
            Some(SectionGroup {
                id: str_of(g, "id")?,
                name: str_of(g, "displayName").unwrap_or_default(),
                sections_url: str_of(g, "sectionsUrl"),
                section_groups_url: str_of(g, "sectionGroupsUrl"),
            })
        })
        .collect()
}

/// Pages from a Graph collection.
pub fn parse_pages(v: &Value) -> Vec<Page> {
    values(v)
        .iter()
        .filter_map(|p| {
            Some(Page {
                id: str_of(p, "id")?,
                title: str_of(p, "title").unwrap_or_default(),
                modified_ms: ms_of(p, "lastModifiedDateTime"),
                web_url: str_of(&p["links"]["oneNoteWebUrl"], "href"),
                content_url: str_of(p, "contentUrl"),
            })
        })
        .collect()
}

/// Plain text of a OneNote page's HTML: tags dropped, block elements become line breaks, the
/// five basic entities decoded, `<title>` and `<script>`/`<style>` bodies skipped.
pub fn page_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    let mut skip_until: Option<&str> = None;
    while let Some(lt) = rest.find('<') {
        if skip_until.is_none() {
            out.push_str(&decode(&rest[..lt]));
        }
        let after = &rest[lt + 1..];
        let Some(gt) = after.find('>') else {
            rest = "";
            break;
        };
        let tag = after[..gt].trim();
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        let closing = tag.starts_with('/');
        match skip_until {
            Some(s) if closing && name == s => skip_until = None,
            Some(_) => {}
            None if !closing && matches!(name.as_str(), "title" | "script" | "style") => {
                skip_until = Some(match name.as_str() {
                    "title" => "title",
                    "script" => "script",
                    _ => "style",
                });
            }
            None if matches!(
                name.as_str(),
                "p" | "div" | "br" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            ) && !out.ends_with('\n') =>
            {
                out.push('\n');
            }
            None => {}
        }
        rest = &after[gt + 1..];
    }
    if skip_until.is_none() {
        out.push_str(&decode(rest));
    }
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn encode(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A new page in a section: `POST <the section's pagesUrl>` (`/me/onenote/sections/{id}/pages`
/// or the group's path) with an HTML document as the body. The `<title>` becomes the page title;
/// `text` becomes the first paragraph.
pub fn create_page_request(pages_url: &str, title: &str, text: &str) -> Write {
    let body = if text.trim().is_empty() {
        String::new()
    } else {
        format!("<p>{}</p>", encode(text.trim()))
    };
    let mut w = Write::graph(Method::POST, pages_url.to_owned(), None);
    w.body = WriteBody::Html(format!(
        "<!DOCTYPE html><html><head><title>{}</title></head><body>{body}</body></html>",
        encode(title.trim())
    ));
    w
}

fn decode(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

const PAGES: usize = 10;

/// How deep section groups nest before IronTeams stops descending.
pub const MAX_GROUP_DEPTH: usize = 4;

impl Client {
    /// The user's own notebooks.
    pub async fn my_notebooks(&self) -> Result<Vec<Notebook>> {
        let url = graph_url(&["me", "onenote", "notebooks"], "");
        let items = self.graph_values("notebooks", &url, PAGES).await?;
        Ok(parse_notebooks(&serde_json::json!({ "value": items })))
    }

    /// The notebooks of a team's group.
    pub async fn group_notebooks(&self, group_id: &str) -> Result<Vec<Notebook>> {
        let url = graph_url(&["groups", group_id, "onenote", "notebooks"], "");
        let items = self.graph_values("group notebooks", &url, PAGES).await?;
        Ok(parse_notebooks(&serde_json::json!({ "value": items })))
    }

    /// Every section of a notebook, including those inside section groups (nested up to
    /// [`MAX_GROUP_DEPTH`] levels). Pass the notebook's `sections_url` and `section_groups_url`.
    pub async fn notebook_sections(
        &self,
        sections_url: Option<&str>,
        section_groups_url: Option<&str>,
    ) -> Result<Vec<Section>> {
        let mut out = Vec::new();
        // (sections url, groups url, path so far, depth)
        let mut todo = vec![(
            sections_url.map(str::to_owned),
            section_groups_url.map(str::to_owned),
            String::new(),
            0usize,
        )];
        while let Some((sections, groups, path, depth)) = todo.pop() {
            if let Some(u) = sections {
                let items = self.graph_values("sections", &u, PAGES).await?;
                for mut s in parse_sections(&serde_json::json!({ "value": items })) {
                    s.group_path.clone_from(&path);
                    out.push(s);
                }
            }
            if let (Some(u), true) = (groups, depth < MAX_GROUP_DEPTH) {
                let items = self.graph_values("section groups", &u, PAGES).await?;
                let found = parse_section_groups(&serde_json::json!({ "value": items }));
                // Reversed so the stack pops them in listing order.
                for g in found.into_iter().rev() {
                    let p = if path.is_empty() {
                        g.name.clone()
                    } else {
                        format!("{path} / {}", g.name)
                    };
                    todo.push((g.sections_url, g.section_groups_url, p, depth + 1));
                }
            }
        }
        Ok(out)
    }

    /// The pages of a section (its `pages_url`).
    pub async fn section_pages(&self, pages_url: &str) -> Result<Vec<Page>> {
        let items = self.graph_values("pages", pages_url, PAGES).await?;
        Ok(parse_pages(&serde_json::json!({ "value": items })))
    }

    /// The HTML of a page (its `content_url`).
    pub async fn page_html(&self, content_url: &str) -> Result<String> {
        let (status, body) = self
            .request_as(&Audience::Graph, Method::GET, content_url, None, &[])
            .await?;
        if !(200..300).contains(&status) {
            return Err(Error::status("page content", status, &body));
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_notebooks_sections_pages() {
        let nb = parse_notebooks(&json!({"value":[
            {"id":"n1","displayName":"Team notes","lastModifiedDateTime":"2026-01-02T00:00:00Z",
             "sectionsUrl":"https://graph.microsoft.com/v1.0/x/sections",
             "links":{"oneNoteWebUrl":{"href":"https://example.invalid/nb"}}},
            {"displayName":"no id"}]}));
        assert_eq!(nb.len(), 1);
        assert_eq!(nb[0].name, "Team notes");
        assert_eq!(nb[0].modified_ms, Some(1_767_312_000_000));
        assert_eq!(nb[0].web_url.as_deref(), Some("https://example.invalid/nb"));
        assert!(
            nb[0]
                .sections_url
                .as_deref()
                .unwrap()
                .ends_with("/sections")
        );
        let groups = parse_section_groups(&json!({"value":[
            {"id":"g1","displayName":"DevOps","sectionsUrl":"https://graph.microsoft.com/v1.0/g/sections",
             "sectionGroupsUrl":"https://graph.microsoft.com/v1.0/g/sectionGroups"}]}));
        assert_eq!(groups[0].name, "DevOps");
        assert!(groups[0].section_groups_url.is_some());
        let secs = parse_sections(
            &json!({"value":[{"id":"s1","displayName":"Ideas","pagesUrl":"https://graph.microsoft.com/v1.0/y/pages"}]}),
        );
        assert_eq!(secs[0].name, "Ideas");
        assert!(secs[0].pages_url.is_some());
        let pages = parse_pages(
            &json!({"value":[{"id":"p1","title":"Plan","links":{},"contentUrl":"https://graph.microsoft.com/v1.0/z/content"}]}),
        );
        assert_eq!(pages[0].title, "Plan");
        assert_eq!(pages[0].web_url, None);
        assert!(pages[0].content_url.is_some());
    }

    #[test]
    fn a_new_page_posts_an_html_document_with_escaped_text() {
        let w = create_page_request(
            "https://graph.microsoft.com/v1.0/me/onenote/sections/s1/pages",
            " Q&A <draft> ",
            "Line \"one\"",
        );
        assert_eq!(w.method, Method::POST);
        assert_eq!(w.audience, Audience::Graph);
        assert_eq!(
            w.body,
            WriteBody::Html(
                "<!DOCTYPE html><html><head><title>Q&amp;A &lt;draft&gt;</title></head>\
                 <body><p>Line &quot;one&quot;</p></body></html>"
                    .into()
            )
        );
        let bare = create_page_request("https://graph.microsoft.com/v1.0/x/pages", "T", "  ");
        assert_eq!(
            bare.body,
            WriteBody::Html(
                "<!DOCTYPE html><html><head><title>T</title></head><body></body></html>".into()
            )
        );
    }

    #[test]
    fn page_text_keeps_the_readable_parts() {
        let html = "<html><head><title>Hidden title</title><style>p{}</style></head>\
            <body><div><p>Hello &amp; welcome</p><p>Second&nbsp;line<br/>third</p>\
            <ul><li>one</li><li>two</li></ul><script>var x=1<2;</script></div></body></html>";
        assert_eq!(
            page_text(html),
            "Hello & welcome\nSecond line\nthird\none\ntwo"
        );
        assert_eq!(page_text("plain &lt;text&gt;"), "plain <text>");
        assert_eq!(page_text(""), "");
    }
}
