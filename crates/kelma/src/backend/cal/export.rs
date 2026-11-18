//! Print: the visible range as a plain agenda, written as a small PDF (no PDF library: the
//! file is a few text pages in a built-in font). The `.ics` export is the calendar crate's.

use super::grid::{Offset, WEEKDAYS, hhmm, local_date, month_short};
use ::calendar::{Date, Event};

const PAGE_W: f64 = 595.0; // A4 in points
const PAGE_H: f64 = 842.0;
const MARGIN: f64 = 50.0;
const LEADING: f64 = 15.0;
const LINES_PER_PAGE: usize = ((PAGE_H - 2.0 * MARGIN) / LEADING) as usize;
/// Characters that fit on a line at 11 pt Helvetica.
const WIDTH_CHARS: usize = 92;

/// The agenda as text lines: a heading per day that has events, one line per event.
pub fn agenda_lines(title: &str, events: &[&Event], off: Offset) -> Vec<String> {
    let mut sorted: Vec<&Event> = events.iter().copied().filter(|e| !e.is_cancelled).collect();
    sorted.sort_by_key(|e| (e.start_ms, e.end_ms));
    let mut lines = vec![title.to_string(), String::new()];
    let mut day: Option<Date> = None;
    for e in sorted {
        let d = local_date(e.start_ms, off);
        if day != Some(d) {
            if day.is_some() {
                lines.push(String::new());
            }
            lines.push(format!(
                "{}, {} {} {}",
                WEEKDAYS[d.weekday() as usize],
                month_short(d),
                d.day,
                d.year
            ));
            day = Some(d);
        }
        let when = if e.is_all_day {
            "All day".to_string()
        } else {
            format!("{} - {}", hhmm(e.start_ms, off), hhmm(e.end_ms, off))
        };
        let mut line = format!("  {when}  {}", if e.subject.is_empty() { "(No title)" } else { &e.subject });
        if !e.location.is_empty() {
            line.push_str(&format!(" ({})", e.location));
        }
        lines.push(line);
    }
    if lines.len() == 2 {
        lines.push("No events.".into());
    }
    lines
}

/// Splits long lines so they fit the page, continuing with an indent.
fn wrap(lines: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for l in lines {
        let chars: Vec<char> = l.chars().collect();
        if chars.len() <= WIDTH_CHARS {
            out.push(l.clone());
            continue;
        }
        let mut start = 0;
        while start < chars.len() {
            let len = if start == 0 { WIDTH_CHARS } else { WIDTH_CHARS - 6 };
            let end = (start + len).min(chars.len());
            let text: String = chars[start..end].iter().collect();
            out.push(if start == 0 { text } else { format!("      {text}") });
            start = end;
        }
    }
    out
}

/// A PDF string literal in WinAnsi: Latin-1 characters pass, the rest become `?` (the built-in
/// font has no other scripts).
fn pdf_text(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for c in s.chars() {
        let b = match c {
            c if (c as u32) < 256 => c as u8,
            '\u{2013}' | '\u{2014}' => b'-',
            '\u{2018}' | '\u{2019}' => b'\'',
            '\u{201c}' | '\u{201d}' => b'"',
            _ => b'?',
        };
        match b {
            b'(' | b')' | b'\\' => {
                out.push(b'\\');
                out.push(b);
            }
            0x20..=0xff => out.push(b),
            _ => out.push(b'?'),
        }
    }
    out
}

/// A PDF with the lines on A4 pages in Helvetica 11 pt (the first line in bold, 15 pt).
pub fn pdf_bytes(lines: &[String]) -> Vec<u8> {
    let lines = wrap(lines);
    let pages: Vec<&[String]> = if lines.is_empty() {
        vec![&[]]
    } else {
        lines.chunks(LINES_PER_PAGE).collect()
    };
    // Objects: 1 catalog, 2 pages, 3 font, 4 bold font, then a (page, content) pair per page.
    let mut objs: Vec<Vec<u8>> = Vec::new();
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 5 + 2 * i)).collect();
    objs.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    objs.push(
        format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into_bytes(),
    );
    objs.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec());
    objs.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".to_vec());
    for (i, page) in pages.iter().enumerate() {
        let mut content: Vec<u8> = Vec::new();
        for (n, line) in page.iter().enumerate() {
            let y = PAGE_H - MARGIN - n as f64 * LEADING;
            let title = i == 0 && n == 0;
            content.extend_from_slice(
                format!(
                    "BT /{} {} Tf {MARGIN} {y:.1} Td (",
                    if title { "F2" } else { "F1" },
                    if title { 15 } else { 11 }
                )
                .as_bytes(),
            );
            content.extend(pdf_text(line));
            content.extend_from_slice(b") Tj ET\n");
        }
        objs.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R >>",
                6 + 2 * i
            )
            .into_bytes(),
        );
        let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
        stream.extend(content);
        stream.extend_from_slice(b"endstream");
        objs.push(stream);
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// A file name for the export: `kelma-calendar-2026-11-16-to-2026-11-20.pdf`.
pub fn file_name(first: Date, last: Date, ext: &str) -> String {
    format!("kelma-calendar-{first}-to-{last}.{ext}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn utc(_: i64) -> i64 {
        0
    }

    fn events() -> Vec<Event> {
        ::calendar::parse_events(&json!({"value": [
            {"id": "b", "subject": "Lunch (with Dana)", "location": {"displayName": "Café"},
             "start": {"dateTime": "2026-11-18T12:00:00.0000000", "timeZone": "UTC"},
             "end": {"dateTime": "2026-11-18T13:00:00.0000000", "timeZone": "UTC"}},
            {"id": "a", "subject": "Standup",
             "start": {"dateTime": "2026-11-18T09:00:00.0000000", "timeZone": "UTC"},
             "end": {"dateTime": "2026-11-18T09:15:00.0000000", "timeZone": "UTC"}},
            {"id": "c", "subject": "Offsite", "isAllDay": true,
             "start": {"dateTime": "2026-11-19T00:00:00.0000000", "timeZone": "UTC"},
             "end": {"dateTime": "2026-11-20T00:00:00.0000000", "timeZone": "UTC"}},
            {"id": "x", "subject": "Dropped", "isCancelled": true,
             "start": {"dateTime": "2026-11-19T10:00:00.0000000", "timeZone": "UTC"},
             "end": {"dateTime": "2026-11-19T11:00:00.0000000", "timeZone": "UTC"}}
        ]}))
    }

    #[test]
    fn the_agenda_groups_events_by_day_in_time_order_and_skips_cancelled_ones() {
        let evs = events();
        let refs: Vec<&Event> = evs.iter().collect();
        let lines = agenda_lines("November 16 - 20, 2026", &refs, &utc);
        assert_eq!(
            lines,
            [
                "November 16 - 20, 2026",
                "",
                "Wed, Nov 18 2026",
                "  09:00 - 09:15  Standup",
                "  12:00 - 13:00  Lunch (with Dana) (Café)",
                "",
                "Thu, Nov 19 2026",
                "  All day  Offsite",
            ]
        );
        assert_eq!(agenda_lines("T", &[], &utc), ["T", "", "No events."]);
    }

    #[test]
    fn the_pdf_is_well_formed_with_a_correct_cross_reference_table() {
        // ASCII only, so byte offsets in the text are byte offsets in the file.
        let pdf = pdf_bytes(&[
            "Week".to_string(),
            "".to_string(),
            "Wed, Nov 18 2026".to_string(),
            "  09:00 - 09:15  Standup".to_string(),
            "  12:00 - 13:00  Lunch (with Dana)".to_string(),
        ]);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.starts_with("%PDF-1.4\n"));
        assert!(text.trim_end().ends_with("%%EOF"));
        assert!(text.contains("/Count 1"));
        assert!(text.contains("(  09:00 - 09:15  Standup) Tj"), "{text}");
        assert!(text.contains("Lunch \\(with Dana\\)"), "parentheses are escaped");
        // Every xref entry points at "N 0 obj".
        let xref_at: usize = text
            .rsplit("startxref\n")
            .next()
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(&pdf[xref_at..xref_at + 4], b"xref");
        let table = &text[xref_at..];
        let entries: Vec<&str> = table.lines().skip(3).take_while(|l| l.ends_with(" n ")).collect();
        assert_eq!(entries.len(), 6, "catalog, pages, two fonts, a page and its content");
        for (i, e) in entries.iter().enumerate() {
            let off: usize = e[..10].parse().unwrap();
            assert!(
                text[off..].starts_with(&format!("{} 0 obj", i + 1)),
                "entry {} points at {}",
                i + 1,
                &text[off..off + 10]
            );
        }
    }

    #[test]
    fn long_agendas_paginate_and_long_lines_wrap() {
        let many: Vec<String> = (0..120).map(|i| format!("line {i}")).collect();
        let text = String::from_utf8_lossy(&pdf_bytes(&many)).into_owned();
        assert!(text.contains("/Count 3"), "120 lines of {LINES_PER_PAGE} per page");
        let wrapped = wrap(&["x".repeat(200)]);
        assert_eq!(wrapped.len(), 3);
        assert!(wrapped[1].starts_with("      x"));
        assert!(wrapped.iter().all(|l| l.chars().count() <= WIDTH_CHARS));
    }

    #[test]
    fn text_outside_latin_1_degrades_to_question_marks_not_broken_bytes() {
        assert_eq!(pdf_text("Café – «x» مرحبا"), b"Caf\xe9 - \xabx\xbb ?????".to_vec());
        assert_eq!(pdf_text("a\\b"), b"a\\\\b".to_vec());
    }

    #[test]
    fn the_file_name_names_the_range() {
        assert_eq!(
            file_name(Date::new(2026, 11, 16), Date::new(2026, 11, 20), "pdf"),
            "kelma-calendar-2026-11-16-to-2026-11-20.pdf"
        );
    }
}
