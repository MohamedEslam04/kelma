//! Edits of the Markdown-style composer source (plan chat 5.4): what the toolbar buttons and the
//! Teams shortcuts do to the text. The composer edits plain text, so bold is `**x**`, a heading
//! is a `# ` prefix and a table is pipe rows; [`crate::markdown_to_composer_html`] turns that
//! source into the HTML Teams expects when the message is sent.
//!
//! Everything here is pure: `(action, text, selection)` in, the new text and selection out.
//! Offsets are byte offsets (what Slint's `TextInput` reports) and are moved back to a character
//! boundary when they are not on one.

/// The result of an action: the new text and where the selection goes (`anchor <= cursor`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub text: String,
    pub anchor: usize,
    pub cursor: usize,
}

/// Languages the code block menu offers (the value after the opening fence).
pub const CODE_LANGUAGES: [&str; 14] = [
    "plaintext",
    "bash",
    "c",
    "cpp",
    "csharp",
    "css",
    "go",
    "html",
    "java",
    "javascript",
    "json",
    "python",
    "rust",
    "sql",
];

/// Applies a toolbar action to `text`, whose selection runs between `anchor` and `cursor`.
/// `None` when the action is unknown or its argument is not valid (a link without an address).
///
/// Actions: `bold`, `italic`, `underline`, `strike`, `code`, `color:#rrggbb`, `color:none`,
/// `bg:#rrggbb`, `bg:none`, `size:large|small|none`, `h1`..`h3`, `para`, `quote`, `ul`, `ol`,
/// `indent`, `outdent`, `codeblock[:language]`, `link:<address>`, `table:<rows>x<columns>`, `hr`
/// and `clear`.
pub fn apply(action: &str, text: &str, anchor: usize, cursor: usize) -> Option<Edit> {
    let a = boundary(text, anchor);
    let c = boundary(text, cursor);
    let (s, e) = (a.min(c), a.max(c));
    let (name, arg) = action.split_once(':').unwrap_or((action, ""));
    match name {
        "bold" | "italic" | "underline" | "strike" | "code" if arg.is_empty() => {
            Some(inline(text, s, e, &Marker::plain(name)?))
        }
        "color" | "bg" | "size" => span(text, s, e, name, arg),
        "h1" | "h2" | "h3" | "para" | "quote" | "ul" | "ol" | "indent" | "outdent" => {
            Some(block(text, s, e, name))
        }
        "codeblock" => code_block(text, s, e, arg),
        "link" => link(text, s, e, arg),
        "table" => table(text, s, e, arg),
        "hr" => Some(insert_block(text, s, e, "---", None)),
        "clear" => Some(clear(text, s, e)),
        _ => None,
    }
}

/// The shortcut for a key press, as an action for [`apply`] or one of `link` (open the link
/// dialog), `undo` and `redo`. `key` is the key's text (case is ignored).
pub fn shortcut(key: &str, ctrl: bool, alt: bool, shift: bool) -> Option<&'static str> {
    if !ctrl {
        return None;
    }
    let k = key.to_lowercase();
    match (alt, shift, k.as_str()) {
        (false, false, "b") => Some("bold"),
        (false, false, "i") => Some("italic"),
        (false, false, "u") => Some("underline"),
        (false, false, "k") => Some("link"),
        (false, false, "z") => Some("undo"),
        (false, false, "y") | (false, true, "z") => Some("redo"),
        (true, false, "x") => Some("strike"),
        (true, false, "1") => Some("h1"),
        (true, false, "2") => Some("h2"),
        (true, false, "3") => Some("h3"),
        (true, false, "p") => Some("para"),
        (true, false, "4") => Some("quote"),
        (true, true, "c") => Some("code"),
        (true, true, "b") => Some("codeblock"),
        // Shift+7 and Shift+8 type `&` and `*` on a US layout.
        (false, true, "7" | "&") => Some("ol"),
        (false, true, "8" | "*") => Some("ul"),
        _ => None,
    }
}

/// True when the source has any formatting, that is, when sending it as plain text would lose
/// something (the live preview strip only shows then).
pub fn has_formatting(src: &str) -> bool {
    let html = crate::markdown_to_composer_html(src);
    html != crate::text_to_teams_html(src)
}

// ---- helpers --------------------------------------------------------------------------

fn boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Start of the line containing `at`.
fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |i| i + 1)
}

/// End (before the newline) of the line containing `at`.
fn line_end(text: &str, at: usize) -> usize {
    text[at..].find('\n').map_or(text.len(), |i| at + i)
}

/// The lines a selection touches: `(start of the first, end of the last)`. A selection that ends
/// at the very start of a line does not touch that line.
fn line_span(text: &str, s: usize, e: usize) -> (usize, usize) {
    let end = if e > s && text[..e].ends_with('\n') {
        e - 1
    } else {
        e
    };
    (line_start(text, s), line_end(text, end))
}

fn replace(text: &str, from: usize, to: usize, with: &str) -> String {
    format!("{}{with}{}", &text[..from], &text[to..])
}

// ---- inline markers -------------------------------------------------------------------

struct Marker {
    open: String,
    close: String,
}

impl Marker {
    fn plain(name: &str) -> Option<Marker> {
        let m = match name {
            "bold" => "**",
            "italic" => "*",
            "underline" => "++",
            "strike" => "~~",
            "code" => "`",
            _ => return None,
        };
        Some(Marker {
            open: m.into(),
            close: m.into(),
        })
    }

    /// Both markers are a run of one character (`**`, `*`, `` ` ``): they can sit inside a
    /// longer run (`***x***` is bold and italic).
    fn run_char(&self) -> Option<char> {
        let c = self.open.chars().next()?;
        (c == '*' || c == '`').then_some(c)
    }

    /// Whether a run of `n` markers on each side counts as this marker being present.
    fn run_present(&self, n: usize) -> bool {
        match self.open.len() {
            // Italic is an odd run: `**x**` is bold only, `***x***` is both.
            1 if self.open == "*" => n % 2 == 1,
            len => n >= len,
        }
    }
}

fn run_before(text: &str, at: usize, c: char) -> usize {
    text[..at].chars().rev().take_while(|&x| x == c).count()
}

fn run_after(text: &str, at: usize, c: char) -> usize {
    text[at..].chars().take_while(|&x| x == c).count()
}

/// How a segment of one line is wrapped by `m`.
enum Wrapped {
    No,
    /// The markers sit just outside the segment.
    Outside,
    /// The segment itself starts and ends with them.
    Inside,
}

fn wrapped(text: &str, s: usize, e: usize, m: &Marker) -> Wrapped {
    let seg = &text[s..e];
    if let Some(c) = m.run_char() {
        if m.run_present(run_before(text, s, c).min(run_after(text, e, c))) {
            return Wrapped::Outside;
        }
        let lead = seg.chars().take_while(|&x| x == c).count();
        let trail = seg.chars().rev().take_while(|&x| x == c).count();
        if seg.chars().count() > lead.min(trail) * 2 && m.run_present(lead.min(trail)) {
            return Wrapped::Inside;
        }
        return Wrapped::No;
    }
    if text[..s].ends_with(&m.open) && text[e..].starts_with(&m.close) {
        return Wrapped::Outside;
    }
    if seg.len() > m.open.len() + m.close.len()
        && seg.starts_with(&m.open)
        && seg.ends_with(&m.close)
    {
        return Wrapped::Inside;
    }
    Wrapped::No
}

/// The non-blank part of each line of `s..e` (inline markers never cross a line).
fn segments(text: &str, s: usize, e: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut at = s;
    loop {
        let end = text[at..e].find('\n').map_or(e, |i| at + i);
        let seg = &text[at..end];
        let lead = seg.len() - seg.trim_start().len();
        let core = seg.trim();
        if !core.is_empty() {
            out.push((at + lead, at + lead + core.len()));
        }
        if end >= e {
            break;
        }
        at = end + 1;
    }
    out
}

fn inline(text: &str, s: usize, e: usize, m: &Marker) -> Edit {
    let segs = segments(text, s, e);
    if segs.is_empty() {
        return inline_caret(text, e, m);
    }
    let all = segs
        .iter()
        .all(|&(a, b)| !matches!(wrapped(text, a, b, m), Wrapped::No));
    let mut out = String::with_capacity(text.len() + segs.len() * (m.open.len() + m.close.len()));
    let (mut at, mut first, mut last) = (0, None, 0);
    for &(a, b) in &segs {
        out.push_str(&text[at..a]);
        let (start, end);
        match (all, wrapped(text, a, b, m)) {
            (true, Wrapped::Outside) => {
                // The opener is already copied: drop it again and skip the closer.
                out.truncate(out.len() - m.open.len());
                start = out.len();
                out.push_str(&text[a..b]);
                end = out.len();
                at = b + m.close.len();
            }
            (true, _) => {
                start = out.len();
                out.push_str(&text[a + m.open.len()..b - m.close.len()]);
                end = out.len();
                at = b;
            }
            _ => {
                out.push_str(&m.open);
                start = out.len();
                out.push_str(&text[a..b]);
                end = out.len();
                out.push_str(&m.close);
                at = b;
            }
        }
        first.get_or_insert(start);
        last = end;
    }
    out.push_str(&text[at..]);
    Edit {
        text: out,
        anchor: first.unwrap_or(0),
        cursor: last,
    }
}

/// No selection: an empty pair around the caret, or the empty pair the caret is in removed.
fn inline_caret(text: &str, at: usize, m: &Marker) -> Edit {
    let inside = at >= m.open.len()
        && text[..at].ends_with(&m.open)
        && text[at..].starts_with(&m.close)
        && match m.run_char() {
            Some(c) => m.run_present(run_before(text, at, c).min(run_after(text, at, c))),
            None => true,
        };
    if inside {
        let from = at - m.open.len();
        return Edit {
            text: replace(text, from, at + m.close.len(), ""),
            anchor: from,
            cursor: from,
        };
    }
    let caret = at + m.open.len();
    Edit {
        text: replace(text, at, at, &format!("{}{}", m.open, m.close)),
        anchor: caret,
        cursor: caret,
    }
}

// ---- color, highlight and size spans -----------------------------------------------------

fn hex(v: &str) -> Option<String> {
    let h = v.strip_prefix('#').unwrap_or(v);
    (h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("#{}", h.to_ascii_lowercase()))
}

/// `{color:#..}`, `{bg:#..}` or `{size:..}` around the selection; the same kind already there is
/// replaced (or removed for `none`).
fn span(text: &str, s: usize, e: usize, kind: &str, arg: &str) -> Option<Edit> {
    let opener = match (kind, arg) {
        (_, "none") => None,
        ("size", "large" | "small") => Some(format!("{{size:{arg}}}")),
        ("color" | "bg", v) => Some(format!("{{{kind}:{}}}", hex(v)?)),
        _ => return None,
    };
    let (open_start, open_len) = existing_span(text, s, e, kind).unzip();
    let mut segs = segments(text, s, e);
    if let (Some(from), Some(len)) = (open_start, open_len) {
        // Replace or remove the wrapper the selection already has.
        let close = e;
        let new_open = opener.unwrap_or_default();
        let mut out = String::new();
        out.push_str(&text[..from]);
        out.push_str(&new_open);
        let a = out.len();
        out.push_str(&text[from + len..close]);
        let b = out.len();
        if new_open.is_empty() {
            out.push_str(&text[close + 3..]);
        } else {
            out.push_str(&text[close..]);
        }
        return Some(Edit {
            text: out,
            anchor: a,
            cursor: b,
        });
    }
    let opener = opener?;
    if segs.is_empty() {
        // The caret: an empty span to type into.
        let caret = s + opener.len();
        return Some(Edit {
            text: replace(text, s, s, &format!("{opener}{{/}}")),
            anchor: caret,
            cursor: caret,
        });
    }
    let mut out = String::new();
    let (mut at, mut first, mut last) = (0, None, 0);
    for (a, b) in segs.drain(..) {
        out.push_str(&text[at..a]);
        out.push_str(&opener);
        first.get_or_insert(out.len());
        out.push_str(&text[a..b]);
        last = out.len();
        out.push_str("{/}");
        at = b;
    }
    out.push_str(&text[at..]);
    Some(Edit {
        text: out,
        anchor: first.unwrap_or(0),
        cursor: last,
    })
}

/// The `{kind:..}` opener ending exactly at `s` with a `{/}` right at `e`: `(start, length)`.
fn existing_span(text: &str, s: usize, e: usize, kind: &str) -> Option<(usize, usize)> {
    if !text[e..].starts_with("{/}") || !text[..s].ends_with('}') {
        return None;
    }
    let from = text[..s].rfind('{')?;
    let body = &text[from + 1..s - 1];
    let (k, _) = body.split_once(':')?;
    (k == kind).then_some((from, s - from))
}

// ---- line blocks --------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Prefix {
    Plain,
    Heading(u8),
    Quote,
    Ul,
    Ol,
}

/// `(kind, indent, the text after the prefix)`.
fn split_prefix(line: &str) -> (Prefix, &str, &str) {
    let body = line.trim_start_matches(' ');
    let indent = &line[..line.len() - body.len()];
    let hashes = body.chars().take_while(|&c| c == '#').count();
    if (1..=3).contains(&hashes)
        && let Some(rest) = body[hashes..].strip_prefix(' ')
    {
        return (Prefix::Heading(hashes as u8), "", rest);
    }
    if let Some(rest) = body.strip_prefix("> ") {
        return (Prefix::Quote, "", rest);
    }
    if body == ">" {
        return (Prefix::Quote, "", "");
    }
    if let Some(rest) = body.strip_prefix("- ").or_else(|| body.strip_prefix("* ")) {
        return (Prefix::Ul, indent, rest);
    }
    let digits = body.chars().take_while(char::is_ascii_digit).count();
    if digits > 0
        && let Some(rest) = body[digits..].strip_prefix(". ")
    {
        return (Prefix::Ol, indent, rest);
    }
    (Prefix::Plain, "", body)
}

fn block(text: &str, s: usize, e: usize, action: &str) -> Edit {
    let (from, to) = line_span(text, s, e);
    let lines: Vec<&str> = text[from..to].split('\n').collect();
    // Blank lines stay blank, unless the caret sits on a single empty line.
    let only_blank = lines.len() == 1;
    let touched = |l: &str| only_blank || !l.trim().is_empty();
    let want = match action {
        "h1" => Prefix::Heading(1),
        "h2" => Prefix::Heading(2),
        "h3" => Prefix::Heading(3),
        "quote" => Prefix::Quote,
        "ul" => Prefix::Ul,
        "ol" => Prefix::Ol,
        _ => Prefix::Plain,
    };
    let toggled_off = want != Prefix::Plain
        && lines
            .iter()
            .filter(|l| touched(l))
            .all(|l| split_prefix(l).0 == want)
        && lines.iter().any(|l| touched(l));
    let mut n = 0;
    let out: Vec<String> = lines
        .iter()
        .map(|&l| {
            if !touched(l) {
                return l.to_string();
            }
            let (kind, indent, rest) = split_prefix(l);
            n += 1;
            match action {
                "indent" => {
                    if matches!(kind, Prefix::Ul | Prefix::Ol) {
                        format!("  {l}")
                    } else {
                        l.to_string()
                    }
                }
                "outdent" => {
                    if matches!(kind, Prefix::Ul | Prefix::Ol) {
                        l.strip_prefix("  ")
                            .or(l.strip_prefix(' '))
                            .unwrap_or(l)
                            .to_string()
                    } else {
                        l.to_string()
                    }
                }
                _ if toggled_off || want == Prefix::Plain => {
                    // Back to a paragraph; a list keeps no indent.
                    rest.to_string()
                }
                _ => match want {
                    Prefix::Heading(h) => format!("{} {rest}", "#".repeat(h as usize)),
                    Prefix::Quote => format!("> {rest}"),
                    Prefix::Ul => format!("{indent}- {rest}"),
                    Prefix::Ol => format!("{indent}{n}. {rest}"),
                    Prefix::Plain => rest.to_string(),
                },
            }
        })
        .collect();
    let joined = out.join("\n");
    let end = from + joined.len();
    let new_text = replace(text, from, to, &joined);
    if s == e {
        // The caret stays on its line, at its end.
        return Edit {
            text: new_text,
            anchor: end,
            cursor: end,
        };
    }
    Edit {
        text: new_text,
        anchor: from,
        cursor: end,
    }
}

/// `clear`: the lines of the selection (or the caret's line) as plain text.
fn clear(text: &str, s: usize, e: usize) -> Edit {
    let (from, to) = line_span(text, s, e);
    let doc = crate::markdown_to_rich(&text[from..to]);
    let plain = rich_plain(&doc);
    let end = from + plain.len();
    Edit {
        text: replace(text, from, to, &plain),
        anchor: from,
        cursor: end,
    }
}

fn spans_text(spans: &[crate::Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

fn list_plain(list: &crate::RichList, out: &mut Vec<String>) {
    for item in &list.items {
        out.push(spans_text(&item.spans));
        for child in &item.children {
            list_plain(child, out);
        }
    }
}

fn rich_plain(doc: &crate::RichDoc) -> String {
    use crate::RichBlock;
    let mut lines: Vec<String> = Vec::new();
    for b in &doc.blocks {
        match b {
            RichBlock::Paragraph(spans)
            | RichBlock::Heading { spans, .. }
            | RichBlock::Quote(spans) => {
                lines.push(spans_text(spans));
            }
            RichBlock::List(l) => list_plain(l, &mut lines),
            RichBlock::Code { text, .. } => lines.push(text.clone()),
            RichBlock::Table(t) => {
                for row in &t.rows {
                    let cells: Vec<String> = row.iter().map(|c| spans_text(&c.spans)).collect();
                    lines.push(cells.join(" "));
                }
            }
            RichBlock::Rule | RichBlock::Image { .. } => {}
        }
    }
    lines.join("\n")
}

// ---- code block, link, table, rule ---------------------------------------------------------

fn language_ok(lang: &str) -> bool {
    lang.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '#' | '-' | '_'))
}

/// Start and end line index of the fenced block that line `idx` is in (a block still open at the
/// end of the text runs to its last line).
fn enclosing_fence(lines: &[&str], idx: usize) -> Option<(usize, usize)> {
    let mut open: Option<usize> = None;
    for (i, l) in lines.iter().enumerate() {
        if l.trim_start().starts_with("```") {
            match open {
                None => open = Some(i),
                Some(o) => {
                    if (o..=i).contains(&idx) {
                        return Some((o, i));
                    }
                    open = None;
                }
            }
        }
    }
    open.filter(|&o| idx >= o).map(|o| (o, lines.len() - 1))
}

fn code_block(text: &str, s: usize, e: usize, lang: &str) -> Option<Edit> {
    let lang = lang.trim().to_lowercase();
    if !language_ok(&lang) {
        return None;
    }
    let fence = if lang.is_empty() || lang == "plaintext" {
        "```".to_string()
    } else {
        format!("```{lang}")
    };
    let (from, to) = line_span(text, s, e);
    let all: Vec<&str> = text.split('\n').collect();
    let first_idx = text[..from].matches('\n').count();
    if let Some((open, close)) = enclosing_fence(&all, first_idx) {
        let starts: Vec<usize> = all
            .iter()
            .scan(0, |at, l| {
                let start = *at;
                *at += l.len() + 1;
                Some(start)
            })
            .collect();
        let open_line = all[open].trim_start().trim_start_matches('`').trim();
        let new_lang = fence.trim_start_matches('`');
        let has_close = close > open && all[close].trim() == "```";
        let body_end = if has_close { close } else { all.len() };
        let body = all[open + 1..body_end].join("\n");
        let region_start = starts[open];
        let region_end = if has_close {
            starts[close] + all[close].len()
        } else {
            text.len()
        };
        // Another language replaces the fence; the same one (or none given) removes the block.
        let (replacement, anchor_off) = if !lang.is_empty()
            && lang != open_line.to_lowercase()
            && !(lang == "plaintext" && open_line.is_empty())
        {
            (format!("```{new_lang}\n{body}\n```"), fence.len() + 1)
        } else {
            (body.clone(), 0)
        };
        let new_text = replace(text, region_start, region_end, &replacement);
        let a = region_start + anchor_off;
        return Some(Edit {
            text: new_text,
            anchor: a,
            cursor: a + body.len(),
        });
    }
    let region = &text[from..to];
    if region.trim().is_empty() {
        let block = format!("{fence}\n\n```");
        let caret_off = fence.len() + 1;
        let new_text = replace(text, from, to, &block);
        let caret = from + caret_off;
        return Some(Edit {
            text: new_text,
            anchor: caret,
            cursor: caret,
        });
    }
    let block = format!("{fence}\n{region}\n```");
    let a = from + fence.len() + 1;
    Some(Edit {
        text: replace(text, from, to, &block),
        anchor: a,
        cursor: a + region.len(),
    })
}

/// `https://` is added when the address has no scheme; only addresses the renderer links
/// (http, https, mailto) are accepted.
fn link_address(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let lower = raw.to_ascii_lowercase();
    let url = if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
    {
        raw.to_string()
    } else if raw.contains("://") || lower.starts_with("javascript:") || lower.starts_with("data:")
    {
        return None;
    } else if raw.contains('@') && !raw.contains('/') {
        format!("mailto:{raw}")
    } else {
        format!("https://{raw}")
    };
    Some(url.replace('(', "%28").replace(')', "%29"))
}

fn link(text: &str, s: usize, e: usize, address: &str) -> Option<Edit> {
    let url = link_address(address)?;
    let label = if s == e {
        url.clone()
    } else {
        text[s..e].replace(['\n', '[', ']'], " ")
    };
    let out = format!("[{label}]({url})");
    let end = s + out.len();
    Some(Edit {
        text: replace(text, s, e, &out),
        anchor: end,
        cursor: end,
    })
}

/// Puts `block` on its own lines at the selection. The selection ends up around `select`
/// (offsets inside the block), or after the block.
fn insert_block(
    text: &str,
    s: usize,
    e: usize,
    block: &str,
    select: Option<(usize, usize)>,
) -> Edit {
    let pre = if s > 0 && !text[..s].ends_with('\n') {
        "\n"
    } else {
        ""
    };
    let post = if e < text.len() && !text[e..].starts_with('\n') {
        "\n"
    } else {
        ""
    };
    let out = format!("{}{pre}{block}{post}{}", &text[..s], &text[e..]);
    let base = s + pre.len();
    let (a, c) = match select {
        Some((a, c)) => (base + a, base + c),
        None => (base + block.len(), base + block.len()),
    };
    Edit {
        text: out,
        anchor: a,
        cursor: c,
    }
}

fn table(text: &str, s: usize, e: usize, size: &str) -> Option<Edit> {
    let (r, c) = size.split_once('x')?;
    let rows: usize = r.parse().ok()?;
    let cols: usize = c.parse().ok()?;
    if !(1..=20).contains(&rows) || !(1..=10).contains(&cols) {
        return None;
    }
    let header: Vec<String> = (1..=cols).map(|i| format!("Column {i}")).collect();
    let mut lines = vec![
        format!("| {} |", header.join(" | ")),
        format!("|{}", " --- |".repeat(cols)),
    ];
    for _ in 1..rows {
        lines.push(format!("|{}", "   |".repeat(cols)));
    }
    // The first header cell is selected, ready to be typed over.
    let first = "| ".len();
    Some(insert_block(
        text,
        s,
        e,
        &lines.join("\n"),
        Some((first, first + "Column 1".len())),
    ))
}

// ---- undo and redo of toolbar edits ---------------------------------------------------------

/// The text and selection at one point of the history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub text: String,
    pub anchor: usize,
    pub cursor: usize,
}

/// Undo and redo for toolbar edits. The `TextInput` forgets its own undo stack when the text is
/// set from code, so edits made by [`apply`] are tracked here. Both only work while the text is
/// exactly what the last edit left; after typing, the input's own undo handles it.
#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    after: Option<String>,
}

const HISTORY_LIMIT: usize = 100;

impl History {
    /// Records that `before` was replaced by `after_text`.
    pub fn record(&mut self, before: Snapshot, after_text: &str) {
        self.undo.push(before);
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.after = Some(after_text.to_string());
    }

    /// The state before the last edit, when `now` is still the text that edit left.
    pub fn undo(&mut self, now: Snapshot) -> Option<Snapshot> {
        if self.after.as_deref() != Some(now.text.as_str()) {
            return None;
        }
        let prev = self.undo.pop()?;
        self.after = Some(prev.text.clone());
        self.redo.push(now);
        Some(prev)
    }

    /// The state an undo went back from, when `now` is still the text that undo left.
    pub fn redo(&mut self, now: Snapshot) -> Option<Snapshot> {
        if self.after.as_deref() != Some(now.text.as_str()) {
            return None;
        }
        let next = self.redo.pop()?;
        self.after = Some(next.text.clone());
        self.undo.push(now);
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `action` on `text` where `[` and `]` mark the selection (`|` marks a caret); the
    /// result is written back the same way.
    fn run(action: &str, marked: &str) -> String {
        let (text, a, c) = parse(marked);
        let e = apply(action, &text, a, c).unwrap_or_else(|| panic!("{action} refused"));
        show(&e)
    }

    fn parse(marked: &str) -> (String, usize, usize) {
        if let Some(i) = marked.find('|') {
            let t = marked.replacen('|', "", 1);
            return (t, i, i);
        }
        let a = marked.find('[').expect("[");
        let t = marked.replacen('[', "", 1);
        let c = t.find(']').expect("]");
        (t.replacen(']', "", 1), a, c)
    }

    fn show(e: &Edit) -> String {
        if e.anchor == e.cursor {
            format!("{}|{}", &e.text[..e.cursor], &e.text[e.cursor..])
        } else {
            format!(
                "{}[{}]{}",
                &e.text[..e.anchor],
                &e.text[e.anchor..e.cursor],
                &e.text[e.cursor..]
            )
        }
    }

    // ---- inline ----

    #[test]
    fn bold_wraps_the_selection_and_toggles_off() {
        assert_eq!(run("bold", "a [b] c"), "a **[b]** c");
        assert_eq!(run("bold", "a **[b]** c"), "a [b] c");
        assert_eq!(run("bold", "a [**b**] c"), "a [b] c");
    }

    #[test]
    fn every_inline_marker() {
        assert_eq!(run("italic", "[x]"), "*[x]*");
        assert_eq!(run("underline", "[x]"), "++[x]++");
        assert_eq!(run("strike", "[x]"), "~~[x]~~");
        assert_eq!(run("code", "[x]"), "`[x]`");
    }

    #[test]
    fn a_caret_gets_an_empty_pair_and_the_pair_toggles_off() {
        assert_eq!(run("bold", "a |"), "a **|**");
        assert_eq!(run("bold", "a **|**"), "a |");
    }

    #[test]
    fn italic_and_bold_stack_without_eating_each_other() {
        assert_eq!(run("italic", "**[x]**"), "***[x]***");
        assert_eq!(run("bold", "***[x]***"), "*[x]*");
        assert_eq!(run("italic", "***[x]***"), "**[x]**");
        assert_eq!(run("bold", "*[x]*"), "***[x]***");
    }

    #[test]
    fn whitespace_stays_outside_the_markers() {
        assert_eq!(run("bold", "a[ b ]c"), "a **[b]** c");
    }

    #[test]
    fn each_line_of_a_selection_is_wrapped_on_its_own() {
        assert_eq!(run("bold", "[one\ntwo]"), "**[one**\n**two]**");
        assert_eq!(run("bold", "[one\n\ntwo]"), "**[one**\n\n**two]**");
    }

    #[test]
    fn multibyte_offsets_land_on_characters() {
        let e = apply("bold", "مرحبا بك", "مرحبا ".len(), "مرحبا بك".len()).unwrap();
        assert_eq!(e.text, "مرحبا **بك**");
        // An offset inside a character is moved back to its start.
        let e = apply("italic", "é", 1, 2).unwrap();
        assert_eq!(e.text, "*é*");
    }

    #[test]
    fn bold_markers_round_trip_through_the_renderer() {
        let (text, a, c) = parse("hi [there] you");
        let e = apply("bold", &text, a, c).unwrap();
        assert_eq!(
            crate::markdown_to_composer_html(&e.text),
            "<p>hi <strong>there</strong> you</p>"
        );
    }

    // ---- color, highlight, size ----

    #[test]
    fn color_and_highlight_wrap_in_spans() {
        assert_eq!(run("color:#FF0000", "a [b]"), "a {color:#ff0000}[b]{/}");
        assert_eq!(run("bg:#fdd472", "[b]"), "{bg:#fdd472}[b]{/}");
        assert_eq!(run("size:large", "[b]"), "{size:large}[b]{/}");
    }

    #[test]
    fn a_color_replaces_the_one_already_there_and_none_removes_it() {
        assert_eq!(
            run("color:#00ff00", "{color:#ff0000}[b]{/}"),
            "{color:#00ff00}[b]{/}"
        );
        assert_eq!(run("color:none", "{color:#ff0000}[b]{/}"), "[b]");
        // A highlight is not a color.
        assert_eq!(
            run("color:#00ff00", "{bg:#fdd472}[b]{/}"),
            "{bg:#fdd472}{color:#00ff00}[b]{/}{/}"
        );
    }

    #[test]
    fn bad_colors_are_refused() {
        assert!(apply("color:red", "x", 0, 1).is_none());
        assert!(apply("color:#12345", "x", 0, 1).is_none());
        assert!(apply("size:huge", "x", 0, 1).is_none());
    }

    #[test]
    fn a_colored_selection_renders_as_a_span() {
        let (text, a, c) = parse("[b]");
        let e = apply("color:#ff0000", &text, a, c).unwrap();
        assert_eq!(
            crate::markdown_to_composer_html(&e.text),
            "<p><span style=\"color:#ff0000;\">b</span></p>"
        );
    }

    // ---- line blocks ----

    #[test]
    fn headings_replace_each_other_and_toggle_off() {
        assert_eq!(run("h1", "title|"), "# title|");
        assert_eq!(run("h2", "# title|"), "## title|");
        assert_eq!(run("h2", "## title|"), "title|");
        assert_eq!(run("h3", "title|"), "### title|");
        assert_eq!(run("para", "### title|"), "title|");
    }

    #[test]
    fn a_heading_applies_to_every_selected_line() {
        assert_eq!(run("h1", "[a\nb]"), "[# a\n# b]");
    }

    #[test]
    fn quote_and_lists() {
        assert_eq!(run("quote", "x|"), "> x|");
        assert_eq!(run("quote", "> x|"), "x|");
        assert_eq!(run("ul", "[a\nb]"), "[- a\n- b]");
        assert_eq!(run("ol", "[a\nb]"), "[1. a\n2. b]");
        assert_eq!(run("ul", "[- a\n- b]"), "[a\nb]");
        assert_eq!(run("ol", "[- a\n- b]"), "[1. a\n2. b]");
    }

    #[test]
    fn blank_lines_are_left_alone_unless_it_is_the_only_line() {
        assert_eq!(run("ul", "[a\n\nb]"), "[- a\n\n- b]");
        assert_eq!(run("ul", "|"), "- |");
    }

    #[test]
    fn indent_and_outdent_only_touch_list_items() {
        assert_eq!(run("indent", "[- a\nplain]"), "[  - a\nplain]");
        assert_eq!(run("outdent", "[  - a]"), "[- a]");
        assert_eq!(run("outdent", "[- a]"), "[- a]");
        let html = crate::markdown_to_composer_html("- a\n  - b");
        assert!(html.contains("<ul><li>a<ul><li>b"), "{html}");
    }

    #[test]
    fn a_heading_survives_the_renderer() {
        let (text, a, c) = parse("title|");
        let e = apply("h2", &text, a, c).unwrap();
        assert_eq!(crate::markdown_to_composer_html(&e.text), "<h2>title</h2>");
    }

    #[test]
    fn clear_strips_the_markers_of_the_lines() {
        assert_eq!(
            run("clear", "[# **big** and {color:#ff0000}red{/}]"),
            "[big and red]"
        );
        assert_eq!(run("clear", "- **a**|"), "[a]");
    }

    // ---- code, link, table, rule ----

    #[test]
    fn code_block_with_a_language() {
        assert_eq!(
            run("codeblock:rust", "[let x = 1;]"),
            "```rust\n[let x = 1;]\n```"
        );
        assert_eq!(run("codeblock", "|"), "```\n|\n```");
        assert_eq!(run("codeblock:python", "|"), "```python\n|\n```");
    }

    #[test]
    fn code_block_toggles_off_and_changes_language() {
        assert_eq!(run("codeblock", "```rust\n[a]\n```"), "[a]");
        assert_eq!(run("codeblock", "```\na|\nb\n```"), "[a\nb]");
        assert_eq!(run("codeblock:go", "```rust\n[a]\n```"), "```go\n[a]\n```");
    }

    #[test]
    fn a_code_block_keeps_its_language_in_html() {
        let (text, a, c) = parse("[x = 1]");
        let e = apply("codeblock:python", &text, a, c).unwrap();
        let html = crate::markdown_to_composer_html(&e.text);
        assert!(html.contains("language-python"), "{html}");
    }

    #[test]
    fn bad_languages_are_refused() {
        assert!(apply("codeblock:a b", "x", 0, 1).is_none());
        assert!(apply("codeblock:<script>", "x", 0, 1).is_none());
    }

    #[test]
    fn link_wraps_the_selection_or_uses_the_address() {
        assert_eq!(
            run("link:example.com", "see [docs]"),
            "see [docs](https://example.com)|"
        );
        assert_eq!(
            run("link:https://a.io/x", "|"),
            "[https://a.io/x](https://a.io/x)|"
        );
        assert_eq!(
            run("link:me@work.io", "[mail]"),
            "[mail](mailto:me@work.io)|"
        );
    }

    #[test]
    fn link_refuses_what_the_renderer_would_not_link() {
        for bad in ["", "  ", "javascript:alert(1)", "ftp://x", "a b"] {
            assert!(apply(&format!("link:{bad}"), "x", 0, 1).is_none(), "{bad}");
        }
        // Parentheses would end the link early.
        let e = apply("link:https://a.io/f(x)", "t", 0, 1).unwrap();
        assert_eq!(e.text, "[t](https://a.io/f%28x%29)");
    }

    #[test]
    fn a_link_renders_as_an_anchor() {
        let (text, a, c) = parse("[docs]");
        let e = apply("link:example.com", &text, a, c).unwrap();
        assert!(
            crate::markdown_to_composer_html(&e.text).contains("<a href=\"https://example.com\""),
        );
    }

    #[test]
    fn table_inserts_header_separator_and_rows_with_the_first_cell_selected() {
        let e = apply("table:3x2", "", 0, 0).unwrap();
        assert_eq!(
            e.text,
            "| Column 1 | Column 2 |\n| --- | --- |\n|   |   |\n|   |   |"
        );
        assert_eq!(&e.text[e.anchor..e.cursor], "Column 1");
        let html = crate::markdown_to_composer_html(&e.text);
        assert!(html.contains("<table"), "{html}");
        assert_eq!(html.matches("<tr>").count(), 3);
    }

    #[test]
    fn table_starts_on_its_own_line() {
        let e = apply("table:2x1", "hello", 5, 5).unwrap();
        assert!(e.text.starts_with("hello\n| Column 1 |"));
        assert!(apply("table:0x2", "", 0, 0).is_none());
        assert!(apply("table:2x99", "", 0, 0).is_none());
        assert!(apply("table:wide", "", 0, 0).is_none());
    }

    #[test]
    fn rule_goes_on_its_own_line() {
        assert_eq!(run("hr", "a|"), "a\n---|");
        assert_eq!(run("hr", "|"), "---|");
    }

    #[test]
    fn unknown_actions_do_nothing() {
        assert!(apply("sparkle", "x", 0, 1).is_none());
        assert!(apply("bold:x", "x", 0, 1).is_none());
    }

    // ---- shortcuts ----

    #[test]
    fn the_teams_formatting_shortcuts() {
        let s = shortcut;
        assert_eq!(s("b", true, false, false), Some("bold"));
        assert_eq!(s("B", true, false, false), Some("bold"));
        assert_eq!(s("i", true, false, false), Some("italic"));
        assert_eq!(s("u", true, false, false), Some("underline"));
        assert_eq!(s("x", true, true, false), Some("strike"));
        assert_eq!(s("k", true, false, false), Some("link"));
        assert_eq!(s("c", true, true, true), Some("code"));
        assert_eq!(s("B", true, true, true), Some("codeblock"));
        assert_eq!(s("1", true, true, false), Some("h1"));
        assert_eq!(s("2", true, true, false), Some("h2"));
        assert_eq!(s("3", true, true, false), Some("h3"));
        assert_eq!(s("p", true, true, false), Some("para"));
        assert_eq!(s("4", true, true, false), Some("quote"));
        assert_eq!(s("7", true, false, true), Some("ol"));
        assert_eq!(s("&", true, false, true), Some("ol"));
        assert_eq!(s("8", true, false, true), Some("ul"));
        assert_eq!(s("*", true, false, true), Some("ul"));
        assert_eq!(s("z", true, false, false), Some("undo"));
        assert_eq!(s("y", true, false, false), Some("redo"));
    }

    #[test]
    fn shortcuts_need_ctrl_and_do_not_steal_other_chords() {
        assert_eq!(shortcut("b", false, false, false), None);
        // The window's own shortcuts.
        assert_eq!(shortcut("e", true, true, false), None);
        assert_eq!(shortcut("c", true, true, false), None);
        assert_eq!(shortcut("n", false, true, true), None);
        assert_eq!(shortcut("q", false, true, false), None);
        assert_eq!(shortcut("b", true, true, false), None);
    }

    // ---- history ----

    fn snap(text: &str) -> Snapshot {
        Snapshot {
            text: text.into(),
            anchor: 0,
            cursor: 0,
        }
    }

    #[test]
    fn history_undoes_and_redoes_in_order() {
        let mut h = History::default();
        h.record(snap("a"), "**a**");
        h.record(snap("**a**"), "# **a**");
        assert_eq!(h.undo(snap("# **a**")).unwrap().text, "**a**");
        assert_eq!(h.undo(snap("**a**")).unwrap().text, "a");
        assert!(h.undo(snap("a")).is_none(), "nothing older");
        assert_eq!(h.redo(snap("a")).unwrap().text, "**a**");
        assert_eq!(h.redo(snap("**a**")).unwrap().text, "# **a**");
        assert!(h.redo(snap("# **a**")).is_none());
    }

    #[test]
    fn history_steps_aside_after_typing() {
        let mut h = History::default();
        h.record(snap("a"), "**a**");
        assert!(h.undo(snap("**a**x")).is_none(), "the input's own undo");
        assert!(h.redo(snap("**a**")).is_none(), "nothing to redo");
    }

    #[test]
    fn a_new_edit_forgets_the_redo_stack() {
        let mut h = History::default();
        h.record(snap("a"), "**a**");
        h.undo(snap("**a**")).unwrap();
        h.record(snap("a"), "*a*");
        assert!(h.redo(snap("*a*")).is_none());
    }

    /// The send path now converts every message from Markdown-style source: what has no
    /// formatting must come out exactly as the plain text conversion did.
    #[test]
    fn plain_messages_convert_as_they_did_before() {
        for t in [
            "hello",
            "a & <b> \"q\" 'x'",
            "one\n\ntwo",
            "مرحبا بك",
            "price $5 - $6",
            "see http://a.io/x_y_z",
            "#hashtag",
            "snake_case_name",
            "\u{E000}0\u{E001} hi \u{E000}1\u{E001}",
        ] {
            assert_eq!(
                crate::markdown_to_composer_html(t),
                crate::text_to_teams_html(t),
                "{t:?}"
            );
        }
    }

    #[test]
    fn formatting_is_detected() {
        assert!(has_formatting("**x**"));
        assert!(has_formatting("# x"));
        assert!(!has_formatting("plain text"));
        assert!(!has_formatting("two\nlines"));
    }
}
