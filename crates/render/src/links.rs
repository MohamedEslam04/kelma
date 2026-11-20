//! The links of a message body, and the Loop components among them (drawn as cards).

use crate::{Block, RichText, Span};

fn push_links(spans: &[Span], out: &mut Vec<String>) {
    for s in spans {
        if let Some(l) = &s.link
            && (l.starts_with("https://") || l.starts_with("http://"))
            && !out.contains(l)
        {
            out.push(l.clone());
        }
    }
}

/// The http(s) links of a message body in reading order, each once.
pub fn links(rt: &RichText) -> Vec<String> {
    let mut out = Vec::new();
    for b in &rt.blocks {
        match b {
            Block::Paragraph { spans, .. } => push_links(spans, &mut out),
            Block::List { items, .. } => items.iter().for_each(|i| push_links(i, &mut out)),
            _ => {}
        }
    }
    out
}

fn host_and_path(url: &str) -> Option<(String, &str)> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    Some((host.to_ascii_lowercase(), path))
}

/// A link to a Loop component: the Loop app (`loop.microsoft.com`, `loop.cloud.microsoft`), a
/// SharePoint sharing link of a Fluid file (`/:fl:/`), or a `.loop` / `.fluid` file.
pub fn is_loop_link(url: &str) -> bool {
    let Some((host, path)) = host_and_path(url) else {
        return false;
    };
    let path = path.to_ascii_lowercase();
    host == "loop.microsoft.com"
        || host == "loop.cloud.microsoft"
        || host.ends_with(".loop.cloud.microsoft")
        || (host.ends_with(".sharepoint.com") && path.starts_with(":fl:/"))
        || path.ends_with(".loop")
        || path.ends_with(".fluid")
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let hex = |i: usize| b.get(i).and_then(|c| (*c as char).to_digit(16));
    while i < b.len() {
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (hex(i + 1), hex(i + 2))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The title of a Loop card: the file name of a `.loop` / `.fluid` link without its extension,
/// otherwise "Loop component" (the app's links carry no name).
pub fn loop_title(url: &str) -> String {
    let name = host_and_path(url)
        .and_then(|(_, path)| path.rsplit('/').next().map(percent_decode))
        .unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    for ext in [".loop", ".fluid"] {
        if lower.ends_with(ext) && name.len() > ext.len() {
            return name[..name.len() - ext.len()].trim().to_string();
        }
    }
    "Loop component".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_teams_html;

    #[test]
    fn links_come_in_order_once() {
        let rt = parse_teams_html(
            r#"<p>See <a href="https://a.example/x">this</a> and <a href="https://b.example/">that</a></p>
               <ul><li><a href="https://a.example/x">again</a></li><li><a href="mailto:x@y.z">mail</a></li></ul>"#,
        );
        assert_eq!(links(&rt), ["https://a.example/x", "https://b.example/"]);
    }

    #[test]
    fn loop_links_are_recognised() {
        assert!(is_loop_link("https://loop.microsoft.com/p/eyJ3Ijp7InUiOiJ"));
        assert!(is_loop_link("https://loop.cloud.microsoft/p/abc"));
        assert!(is_loop_link(
            "https://contoso.sharepoint.com/:fl:/g/contentstorage/x?nav=1"
        ));
        assert!(is_loop_link(
            "https://contoso-my.sharepoint.com/personal/u/Documents/Plan.loop?web=1"
        ));
        assert!(!is_loop_link("https://contoso.sharepoint.com/:w:/g/abc"));
        assert!(!is_loop_link("https://example.com/loop"));
        assert!(!is_loop_link("loop.microsoft.com/p/x"));
    }

    #[test]
    fn loop_title_is_the_file_name_or_a_generic_name() {
        assert_eq!(
            loop_title(
                "https://contoso-my.sharepoint.com/personal/u/Documents/Sprint%20plan.loop?web=1"
            ),
            "Sprint plan"
        );
        assert_eq!(
            loop_title("https://loop.microsoft.com/p/abc"),
            "Loop component"
        );
        assert_eq!(
            loop_title("https://contoso.sharepoint.com/:fl:/g/x"),
            "Loop component"
        );
    }
}
