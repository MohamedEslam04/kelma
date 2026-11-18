//! Converts the `render` rich-text model into the Markdown dialect Slint's `StyledText` parses,
//! and pulls out the parts the bubble draws separately (reply quote, images).

use render::{Block, Dir, RichText, Span};

/// What a message bubble needs to draw its content.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rendered {
    pub quote_author: String,
    pub quote_text: String,
    pub markdown: String,
    pub plain: String,
    pub rtl: bool,
    pub images: Vec<(String, Option<u32>, Option<u32>)>,
}

/// Escapes Markdown control characters in plain text.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '*' | '_' | '`' | '[' | ']' | '(' | ')' | '#' | '<' | '>' | '~' | '|' | '!'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn link_target(url: &str) -> String {
    url.replace(' ', "%20")
        .replace('(', "%28")
        .replace(')', "%29")
}

fn spans_md(spans: &[Span]) -> String {
    let mut out = String::new();
    for s in spans {
        if s.text.is_empty() {
            continue;
        }
        // Keep line breaks as hard breaks; style each line separately so markers never span lines.
        let lines: Vec<&str> = s.text.split('\n').collect();
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                out.push_str("\\\n");
            }
            if line.trim().is_empty() {
                out.push_str(line);
                continue;
            }
            let lead = line.len() - line.trim_start().len();
            let trail = line.len() - line.trim_end().len();
            let core = line.trim();
            let mut t = if s.style.code {
                format!("`{}`", core.replace('`', "'"))
            } else {
                escape(core)
            };
            if s.mention.is_some() || s.style.bold {
                t = format!("**{t}**");
            }
            if s.style.italic {
                t = format!("*{t}*");
            }
            if s.style.strike {
                t = format!("~~{t}~~");
            }
            if let Some(url) = &s.link {
                t = format!("[{t}]({})", link_target(url));
            }
            out.push_str(&line[..lead]);
            out.push_str(&t);
            out.push_str(&line[line.len() - trail..]);
        }
    }
    out
}

fn spans_plain(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

/// Converts a parsed message into bubble parts.
pub fn render_blocks(rt: &RichText) -> Rendered {
    let mut r = Rendered::default();
    let mut md: Vec<String> = vec![];
    let mut plain: Vec<String> = vec![];
    let mut first_dir: Option<Dir> = None;
    for b in &rt.blocks {
        match b {
            Block::Quote {
                author, preview, ..
            } if r.quote_text.is_empty() && md.is_empty() => {
                r.quote_author = author.clone().unwrap_or_default();
                r.quote_text = preview.clone();
            }
            Block::Quote { preview, .. } => {
                md.push(format!("> {}", escape(preview)));
                plain.push(preview.clone());
            }
            Block::Paragraph { dir, spans } => {
                first_dir.get_or_insert(*dir);
                md.push(spans_md(spans));
                plain.push(spans_plain(spans));
            }
            Block::List { ordered, items } => {
                let mut lines = vec![];
                for (i, it) in items.iter().enumerate() {
                    let bullet = if *ordered {
                        format!("{}.", i + 1)
                    } else {
                        "-".to_string()
                    };
                    lines.push(format!("{bullet} {}", spans_md(it)));
                    plain.push(spans_plain(it));
                }
                if let Some(firstitem) = items.first() {
                    first_dir.get_or_insert(render::base_direction(&spans_plain(firstitem)));
                }
                md.push(lines.join("\n"));
            }
            Block::Code { text } => {
                md.push(
                    text.lines()
                        .map(|l| format!("`{}`", l.replace('`', "'")))
                        .collect::<Vec<_>>()
                        .join("\\\n"),
                );
                plain.push(text.clone());
            }
            Block::Image { url, width, height } => r.images.push((url.clone(), *width, *height)),
        }
    }
    r.markdown = md
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    r.plain = plain.join("\n").trim().to_string();
    r.rtl = first_dir == Some(Dir::Rtl);
    r
}

/// Emoji for a Teams reaction key (`like`, `heart`, or a codepoint key like `1f525_fire`).
pub fn reaction_emoji(key: &str) -> String {
    match key {
        "like" => "👍".into(),
        "heart" => "❤️".into(),
        "laugh" => "😆".into(),
        "surprised" => "😮".into(),
        "sad" => "😢".into(),
        "angry" => "😡".into(),
        _ => {
            let code = key.split('_').next().unwrap_or_default();
            let parsed: Option<String> = code
                .split('-')
                .map(|h| u32::from_str_radix(h, 16).ok().and_then(char::from_u32))
                .collect();
            parsed
                .filter(|s| !s.is_empty() && code.len() >= 4)
                .unwrap_or_else(|| key.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(html: &str) -> Rendered {
        render_blocks(&render::parse_teams_html(html))
    }

    #[test]
    fn styles_become_markdown() {
        let x = r("<p>Hello <b>bold</b> and <i>it</i> <a href=\"https://x.com/a b\">link</a></p>");
        assert_eq!(
            x.markdown,
            "Hello **bold** and *it* [link](https://x.com/a%20b)"
        );
        assert_eq!(x.plain, "Hello bold and it link");
        assert!(!x.rtl);
    }

    #[test]
    fn escapes_markdown_chars() {
        let x = r("<p>a*b_c [x](y) # not a heading</p>");
        assert_eq!(x.markdown, "a\\*b\\_c \\[x\\]\\(y\\) \\# not a heading");
    }

    #[test]
    fn arabic_is_rtl() {
        let x = r("<p>تمام 👍 هراجع الـ PR</p>");
        assert!(x.rtl);
        assert_eq!(x.markdown, "تمام 👍 هراجع الـ PR");
    }

    #[test]
    fn reply_quote_is_extracted() {
        let html = render::reply_html("1", "8:orgid:a", "Sara", 1, "original text", "my reply");
        let x = r(&html);
        assert_eq!(x.quote_author, "Sara");
        assert_eq!(x.quote_text, "original text");
        assert_eq!(x.markdown, "my reply");
    }

    #[test]
    fn mentions_are_bold_and_lists_kept() {
        let x = r(
            r#"<p>hi <span itemscope itemtype="http://schema.skype.com/Mention" itemid="0">Ahmed</span></p><ul><li>one</li><li>two</li></ul>"#,
        );
        assert_eq!(x.markdown, "hi **Ahmed**\n\n- one\n- two");
    }

    #[test]
    fn images_are_separate() {
        let x = r(
            r#"<p><img itemtype="http://schema.skype.com/AMSImage" src="https://eu-api.asm.skype.com/v1/objects/0-a/views/imgo" width="250" height="141"></p>"#,
        );
        assert_eq!(
            x.images,
            vec![(
                "https://eu-api.asm.skype.com/v1/objects/0-a/views/imgo".into(),
                Some(250),
                Some(141)
            )]
        );
        assert_eq!(x.markdown, "");
    }

    #[test]
    fn line_breaks_are_hard_breaks() {
        assert_eq!(r("<p>a<br>b</p>").markdown, "a\\\nb");
    }

    #[test]
    fn reaction_keys() {
        assert_eq!(reaction_emoji("like"), "👍");
        assert_eq!(reaction_emoji("1f525_fire"), "🔥");
        assert_eq!(reaction_emoji("weird"), "weird");
    }
}
