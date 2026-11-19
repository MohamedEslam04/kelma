//! Zip packaging: blank Office documents (OOXML) and folder downloads.

use std::io::{Cursor, Write};

use teams_proto::{Error, Result};
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

/// Builds a zip from `(path, bytes)` entries. A path ending in `/` is an (empty) directory.
pub fn zip_bytes(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let err = |e: &dyn std::fmt::Display| Error::Protocol(format!("zip: {e}"));
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (path, data) in entries {
        if path.ends_with('/') {
            w.add_directory(path, opts).map_err(|e| err(&e))?;
        } else {
            w.start_file(path, opts).map_err(|e| err(&e))?;
            w.write_all(data).map_err(|e| err(&e))?;
        }
    }
    Ok(w.finish().map_err(|e| err(&e))?.into_inner())
}

/// The kinds of new document the "New" menu creates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    Word,
    Excel,
    PowerPoint,
    Markdown,
    Text,
    /// A blank Visio drawing (opens in Visio for the web).
    Visio,
}

impl DocKind {
    pub fn extension(self) -> &'static str {
        match self {
            DocKind::Word => "docx",
            DocKind::Excel => "xlsx",
            DocKind::PowerPoint => "pptx",
            DocKind::Markdown => "md",
            DocKind::Text => "txt",
            DocKind::Visio => "vsdx",
        }
    }

    /// File name used when the caller gives none.
    pub fn default_name(self) -> &'static str {
        match self {
            DocKind::Word => "Document.docx",
            DocKind::Excel => "Book.xlsx",
            DocKind::PowerPoint => "Presentation.pptx",
            DocKind::Markdown => "Document.md",
            DocKind::Text => "Document.txt",
            DocKind::Visio => "Drawing.vsdx",
        }
    }

    /// `name` with this kind's extension added when it is missing.
    pub fn file_name(self, name: &str) -> String {
        let ext = self.extension();
        if name.to_ascii_lowercase().ends_with(&format!(".{ext}")) {
            name.to_owned()
        } else {
            format!("{name}.{ext}")
        }
    }

    /// A blank document of this kind.
    pub fn blank(self) -> Vec<u8> {
        match self {
            DocKind::Word => docx(),
            DocKind::Excel => xlsx(),
            DocKind::PowerPoint => pptx(),
            DocKind::Visio => vsdx(),
            DocKind::Markdown | DocKind::Text => Vec::new(),
        }
    }
}

const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const REL_DOC: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";

fn xml(body: String) -> Vec<u8> {
    format!("{XML}{body}").into_bytes()
}

fn content_types(overrides: &[(&str, &str)]) -> Vec<u8> {
    let mut s = String::from(
        "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>",
    );
    for (part, ct) in overrides {
        s.push_str(&format!(
            "<Override PartName=\"{part}\" ContentType=\"{ct}\"/>"
        ));
    }
    s.push_str("</Types>");
    xml(s)
}

/// A `.rels` part: `(id, type suffix, target)`.
fn rels(items: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut s = format!("<Relationships xmlns=\"{REL_NS}\">");
    for (id, ty, target) in items {
        s.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{REL_DOC}/{ty}\" Target=\"{target}\"/>"
        ));
    }
    s.push_str("</Relationships>");
    xml(s)
}

fn package(parts: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
    let entries: Vec<(String, Vec<u8>)> =
        parts.into_iter().map(|(p, d)| (p.to_owned(), d)).collect();
    // Writing to memory with fixed names cannot fail.
    zip_bytes(&entries).unwrap_or_default()
}

fn docx() -> Vec<u8> {
    package(vec![
        (
            "[Content_Types].xml",
            content_types(&[(
                "/word/document.xml",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
            )]),
        ),
        (
            "_rels/.rels",
            rels(&[("rId1", "officeDocument", "word/document.xml")]),
        ),
        (
            "word/document.xml",
            xml(
                "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                 <w:body><w:p/><w:sectPr><w:pgSz w:w=\"12240\" w:h=\"15840\"/>\
                 <w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/>\
                 </w:sectPr></w:body></w:document>"
                    .into(),
            ),
        ),
    ])
}

fn xlsx() -> Vec<u8> {
    package(vec![
        (
            "[Content_Types].xml",
            content_types(&[
                (
                    "/xl/workbook.xml",
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
                ),
                (
                    "/xl/worksheets/sheet1.xml",
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml",
                ),
            ]),
        ),
        (
            "_rels/.rels",
            rels(&[("rId1", "officeDocument", "xl/workbook.xml")]),
        ),
        (
            "xl/workbook.xml",
            xml(format!(
                "<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"{NS_R}\">\
                 <sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"
            )),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            rels(&[("rId1", "worksheet", "worksheets/sheet1.xml")]),
        ),
        (
            "xl/worksheets/sheet1.xml",
            xml(
                "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData/></worksheet>"
                    .into(),
            ),
        ),
    ])
}

fn empty_tree() -> &'static str {
    "<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
     <p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr></p:spTree>"
}

fn theme() -> Vec<u8> {
    let color = |n: &str, v: &str| format!("<a:{n}><a:srgbClr val=\"{v}\"/></a:{n}>");
    let fills = "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>".repeat(3);
    let lines =
        "<a:ln w=\"6350\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln>".repeat(3);
    let effects = "<a:effectStyle><a:effectLst/></a:effectStyle>".repeat(3);
    xml(format!(
        "<a:theme xmlns:a=\"{NS_A}\" name=\"Office Theme\"><a:themeElements>\
         <a:clrScheme name=\"Office\">\
         <a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1>\
         <a:lt1><a:sysClr val=\"window\" lastClr=\"FFFFFF\"/></a:lt1>{}{}{}{}{}{}{}{}{}{}\
         </a:clrScheme>\
         <a:fontScheme name=\"Office\">\
         <a:majorFont><a:latin typeface=\"Calibri Light\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont>\
         <a:minorFont><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont>\
         </a:fontScheme>\
         <a:fmtScheme name=\"Office\"><a:fillStyleLst>{fills}</a:fillStyleLst>\
         <a:lnStyleLst>{lines}</a:lnStyleLst><a:effectStyleLst>{effects}</a:effectStyleLst>\
         <a:bgFillStyleLst>{fills}</a:bgFillStyleLst></a:fmtScheme>\
         </a:themeElements></a:theme>",
        color("dk2", "44546A"),
        color("lt2", "E7E6E6"),
        color("accent1", "4472C4"),
        color("accent2", "ED7D31"),
        color("accent3", "A5A5A5"),
        color("accent4", "FFC000"),
        color("accent5", "5B9BD5"),
        color("accent6", "70AD47"),
        color("hlink", "0563C1"),
        color("folHlink", "954F72"),
    ))
}

fn pptx() -> Vec<u8> {
    let ct = |part: &str, kind: &str| {
        (
            part.to_owned(),
            format!("application/vnd.openxmlformats-officedocument.presentationml.{kind}+xml"),
        )
    };
    let parts = [
        ct("/ppt/presentation.xml", "presentation.main"),
        ct("/ppt/slideMasters/slideMaster1.xml", "slideMaster"),
        ct("/ppt/slideLayouts/slideLayout1.xml", "slideLayout"),
        ct("/ppt/slides/slide1.xml", "slide"),
    ];
    let mut overrides: Vec<(&str, &str)> = parts
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect();
    overrides.push((
        "/ppt/theme/theme1.xml",
        "application/vnd.openxmlformats-officedocument.theme+xml",
    ));
    let clr_map = "<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>";
    let ns = format!("xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\"");
    package(vec![
        ("[Content_Types].xml", content_types(&overrides)),
        (
            "_rels/.rels",
            rels(&[("rId1", "officeDocument", "ppt/presentation.xml")]),
        ),
        (
            "ppt/presentation.xml",
            xml(format!(
                "<p:presentation {ns}><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst>\
                 <p:sldIdLst><p:sldId id=\"256\" r:id=\"rId2\"/></p:sldIdLst>\
                 <p:sldSz cx=\"12192000\" cy=\"6858000\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"
            )),
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            rels(&[
                ("rId1", "slideMaster", "slideMasters/slideMaster1.xml"),
                ("rId2", "slide", "slides/slide1.xml"),
                ("rId3", "theme", "theme/theme1.xml"),
            ]),
        ),
        (
            "ppt/slideMasters/slideMaster1.xml",
            xml(format!(
                "<p:sldMaster {ns}><p:cSld>{}</p:cSld>{clr_map}\
                 <p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst></p:sldMaster>",
                empty_tree()
            )),
        ),
        (
            "ppt/slideMasters/_rels/slideMaster1.xml.rels",
            rels(&[
                ("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"),
                ("rId2", "theme", "../theme/theme1.xml"),
            ]),
        ),
        (
            "ppt/slideLayouts/slideLayout1.xml",
            xml(format!(
                "<p:sldLayout {ns} type=\"blank\" preserve=\"1\"><p:cSld name=\"Blank\">{}</p:cSld>\
                 <p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>",
                empty_tree()
            )),
        ),
        (
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")]),
        ),
        (
            "ppt/slides/slide1.xml",
            xml(format!(
                "<p:sld {ns}><p:cSld>{}</p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>",
                empty_tree()
            )),
        ),
        (
            "ppt/slides/_rels/slide1.xml.rels",
            rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml")]),
        ),
        ("ppt/theme/theme1.xml", theme()),
    ])
}

const NS_V: &str = "http://schemas.microsoft.com/office/visio/2012/main";
const REL_V: &str = "http://schemas.microsoft.com/visio/2010/relationships";

/// A `.rels` part with Visio relationship types: `(id, type suffix, target)`.
fn visio_rels(items: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut s = format!("<Relationships xmlns=\"{REL_NS}\">");
    for (id, ty, target) in items {
        s.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{REL_V}/{ty}\" Target=\"{target}\"/>"
        ));
    }
    s.push_str("</Relationships>");
    xml(s)
}

/// A blank Visio drawing (VSDX, Visio 2013 format): document, one page list and one empty
/// page of US Letter size.
fn vsdx() -> Vec<u8> {
    let ct = |kind: &str| format!("application/vnd.ms-visio.{kind}+xml");
    let (main, pages, page) = (ct("drawing.main"), ct("pages"), ct("page"));
    package(vec![
        (
            "[Content_Types].xml",
            content_types(&[
                ("/visio/document.xml", &main),
                ("/visio/pages/pages.xml", &pages),
                ("/visio/pages/page1.xml", &page),
            ]),
        ),
        (
            "_rels/.rels",
            visio_rels(&[("rId1", "document", "visio/document.xml")]),
        ),
        (
            "visio/document.xml",
            xml(format!(
                "<VisioDocument xmlns=\"{NS_V}\" xmlns:r=\"{NS_R}\" xml:space=\"preserve\">\
                 <DocumentSettings TopPage=\"0\" DefaultTextStyle=\"0\" DefaultLineStyle=\"0\" \
                 DefaultFillStyle=\"0\" DefaultGuideStyle=\"0\"/>\
                 <StyleSheets><StyleSheet ID=\"0\" NameU=\"No Style\" Name=\"No Style\"/></StyleSheets>\
                 </VisioDocument>"
            )),
        ),
        (
            "visio/_rels/document.xml.rels",
            visio_rels(&[("rId1", "pages", "pages/pages.xml")]),
        ),
        (
            "visio/pages/pages.xml",
            xml(format!(
                "<Pages xmlns=\"{NS_V}\" xmlns:r=\"{NS_R}\" xml:space=\"preserve\">\
                 <Page ID=\"0\" NameU=\"Page-1\" Name=\"Page-1\"><PageSheet>\
                 <Cell N=\"PageWidth\" V=\"8.5\"/><Cell N=\"PageHeight\" V=\"11\"/>\
                 </PageSheet><Rel r:id=\"rId1\"/></Page></Pages>"
            )),
        ),
        (
            "visio/pages/_rels/pages.xml.rels",
            visio_rels(&[("rId1", "page", "page1.xml")]),
        ),
        (
            "visio/pages/page1.xml",
            xml(format!(
                "<PageContents xmlns=\"{NS_V}\" xmlns:r=\"{NS_R}\" xml:space=\"preserve\"/>"
            )),
        ),
    ])
}

/// An Internet shortcut (`.url`) that opens `url`: what OneDrive's "New > Link" saves.
pub fn link_file(url: &str) -> Vec<u8> {
    // One line per key; a line break inside the URL would inject a key.
    let url: String = url.chars().filter(|c| !c.is_control()).collect();
    format!("[InternetShortcut]\r\nURL={url}\r\n").into_bytes()
}

/// `name` as a `.url` file name.
pub fn link_file_name(name: &str) -> String {
    if name.to_ascii_lowercase().ends_with(".url") {
        name.to_owned()
    } else {
        format!("{name}.url")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// (name, content) of every entry.
    fn unzip(data: &[u8]) -> Vec<(String, String)> {
        let mut z = zip::ZipArchive::new(Cursor::new(data)).unwrap();
        (0..z.len())
            .map(|i| {
                let mut f = z.by_index(i).unwrap();
                let mut s = String::new();
                if !f.is_dir() {
                    f.read_to_string(&mut s).unwrap();
                }
                (f.name().unwrap().into_owned(), s)
            })
            .collect()
    }

    #[test]
    fn zip_round_trip_with_directories() {
        let z = zip_bytes(&[
            ("a.txt".into(), b"hello".to_vec()),
            ("sub/".into(), Vec::new()),
            ("sub/b.txt".into(), "مرحبا".as_bytes().to_vec()),
        ])
        .unwrap();
        let e = unzip(&z);
        assert_eq!(e[0], ("a.txt".into(), "hello".into()));
        assert_eq!(e[1].0, "sub/");
        assert_eq!(e[2], ("sub/b.txt".into(), "مرحبا".into()));
    }

    /// Every part is well-formed enough that its tags balance, and every override in
    /// `[Content_Types].xml` names a part that exists.
    fn check_package(data: &[u8], must_have: &[&str]) {
        let entries = unzip(data);
        let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names[0], "[Content_Types].xml", "content types come first");
        for m in must_have {
            assert!(names.contains(m), "missing {m}: {names:?}");
        }
        let ct = &entries[0].1;
        for part in ct.split("PartName=\"").skip(1) {
            let p = part.split('"').next().unwrap().trim_start_matches('/');
            assert!(names.contains(&p), "override for missing part {p}");
        }
        for (n, body) in &entries {
            assert!(body.starts_with("<?xml"), "{n} is not xml");
            let opens = body.matches('<').count();
            let closes = body.matches('>').count();
            assert_eq!(opens, closes, "{n} has unbalanced brackets");
        }
    }

    #[test]
    fn blank_word_excel_powerpoint_are_complete_packages() {
        check_package(
            &DocKind::Word.blank(),
            &["_rels/.rels", "word/document.xml"],
        );
        check_package(
            &DocKind::Excel.blank(),
            &[
                "xl/workbook.xml",
                "xl/_rels/workbook.xml.rels",
                "xl/worksheets/sheet1.xml",
            ],
        );
        check_package(
            &DocKind::PowerPoint.blank(),
            &[
                "ppt/presentation.xml",
                "ppt/slideMasters/slideMaster1.xml",
                "ppt/slideLayouts/slideLayout1.xml",
                "ppt/slides/slide1.xml",
                "ppt/theme/theme1.xml",
            ],
        );
    }

    #[test]
    fn blank_visio_is_a_complete_package() {
        let v = DocKind::Visio.blank();
        check_package(
            &v,
            &[
                "_rels/.rels",
                "visio/document.xml",
                "visio/_rels/document.xml.rels",
                "visio/pages/pages.xml",
                "visio/pages/_rels/pages.xml.rels",
                "visio/pages/page1.xml",
            ],
        );
        let e = unzip(&v);
        let part = |n: &str| e.iter().find(|(p, _)| p == n).unwrap().1.clone();
        assert!(part("[Content_Types].xml").contains("application/vnd.ms-visio.drawing.main+xml"));
        assert!(part("_rels/.rels").contains("/visio/2010/relationships/document"));
        assert!(part("visio/pages/pages.xml").contains("r:id=\"rId1\""));
        assert_eq!(DocKind::Visio.default_name(), "Drawing.vsdx");
        assert_eq!(DocKind::Visio.file_name("Flow"), "Flow.vsdx");
    }

    #[test]
    fn link_files_are_internet_shortcuts() {
        assert_eq!(
            link_file("https://example.com/a?b=1"),
            b"[InternetShortcut]\r\nURL=https://example.com/a?b=1\r\n"
        );
        assert_eq!(
            link_file("https://x.test/\r\nIconFile=evil"),
            b"[InternetShortcut]\r\nURL=https://x.test/IconFile=evil\r\n"
        );
        assert_eq!(link_file_name("Docs"), "Docs.url");
        assert_eq!(link_file_name("Docs.URL"), "Docs.URL");
    }

    #[test]
    fn text_kinds_are_empty_and_names_get_extensions() {
        assert!(DocKind::Markdown.blank().is_empty());
        assert!(DocKind::Text.blank().is_empty());
        assert_eq!(DocKind::Word.file_name("Plan"), "Plan.docx");
        assert_eq!(DocKind::Word.file_name("Plan.DOCX"), "Plan.DOCX");
        assert_eq!(DocKind::PowerPoint.default_name(), "Presentation.pptx");
        assert_eq!(DocKind::Markdown.file_name("notes.txt"), "notes.txt.md");
    }
}
