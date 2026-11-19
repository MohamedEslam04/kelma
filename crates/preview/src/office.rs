//! Office files (and other formats Graph can convert) to PDF, through
//! `GET …/content?format=pdf`.

use onedrive::ItemRef;
use onedrive::api::graph_raw;
use reqwest::Method;
use teams_proto::Client;
use teams_proto::client::RawBody;

use crate::{Error, Result};

/// Extensions Graph converts to PDF (`format=pdf` on the content endpoint).
pub const CONVERTIBLE: &[&str] = &[
    "doc", "docx", "docm", "dot", "dotx", "dotm", "odt", "rtf", "xls", "xlsx", "xlsm", "xlsb",
    "ods", "ppt", "pptx", "pptm", "pps", "ppsx", "ppsm", "pot", "potx", "potm", "odp", "eml",
    "msg", "epub", "htm", "html", "md", "tif", "tiff",
];

/// Lowercase extension of `name`, or empty when it has none (`README`, `.profile`).
fn extension(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// True when Graph can convert a file called `name` to PDF.
pub fn can_convert(name: &str) -> bool {
    CONVERTIBLE.contains(&extension(name).as_str())
}

/// URL that returns the item as PDF. An empty `drive_id` means the signed-in user's own drive
/// (`/me/drive/items/{id}`).
pub fn pdf_url(drive_id: &str, item_id: &str) -> String {
    format!(
        "{}/content?format=pdf",
        ItemRef::new(drive_id, item_id).url()
    )
}

/// Maps a failed conversion: Graph answers 406 (Not Acceptable) when it cannot convert this
/// file, which is an unsupported type to the user, not a network error.
fn conversion_error(e: teams_proto::Error, ext: &str) -> Error {
    if onedrive::api::is_status(&e, 406) {
        Error::Unsupported(ext.into())
    } else {
        Error::Graph(e)
    }
}

/// Converts the file `item_id` in `drive_id` (empty for My files), called `name`, to PDF and
/// returns the PDF bytes. A name Graph cannot convert fails with [`Error::Unsupported`]
/// before any request. Read-only: the file itself is not changed.
pub async fn office_to_pdf(
    client: &Client,
    drive_id: &str,
    item_id: &str,
    name: &str,
) -> Result<Vec<u8>> {
    let ext = extension(name);
    if !CONVERTIBLE.contains(&ext.as_str()) {
        return Err(Error::Unsupported(ext));
    }
    let url = pdf_url(drive_id, item_id);
    match graph_raw(client, "convert to pdf", Method::GET, &url, RawBody::None).await {
        Ok(r) => Ok(r.body),
        Err(e) => Err(conversion_error(e, &ext)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_which_names_convert() {
        for n in [
            "Report.docx",
            "Budget.XLSX",
            "deck.pptx",
            "old.doc",
            "notes.odt",
            "a.b.rtf",
        ] {
            assert!(can_convert(n), "{n}");
        }
        for n in [
            "paper.pdf",
            "photo.png",
            "archive.zip",
            "README",
            "docx",
            ".docx.",
            "",
        ] {
            assert!(!can_convert(n), "{n}");
        }
    }

    #[test]
    fn builds_the_conversion_url() {
        assert_eq!(
            pdf_url("", "01ABC"),
            "https://graph.microsoft.com/v1.0/me/drive/items/01ABC/content?format=pdf"
        );
        assert_eq!(
            pdf_url("b!xY_z", "01P"),
            "https://graph.microsoft.com/v1.0/drives/b%21xY_z/items/01P/content?format=pdf"
        );
    }

    #[tokio::test]
    async fn unsupported_names_fail_before_any_request() {
        // An offline client: a request would fail with an auth or network error instead.
        let client = Client::offline("test").unwrap();
        match office_to_pdf(&client, "", "01ABC", "photo.png").await {
            Err(Error::Unsupported(ext)) => assert_eq!(ext, "png"),
            other => panic!("{other:?}"),
        }
        match office_to_pdf(&client, "", "01ABC", "README").await {
            Err(e @ Error::Unsupported(_)) => {
                assert_eq!(e.to_string(), "cannot preview files without an extension")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn maps_conversion_errors() {
        let not_acceptable = teams_proto::Error::status("convert to pdf", 406, "{}");
        match conversion_error(not_acceptable, "docx") {
            Error::Unsupported(ext) => assert_eq!(ext, "docx"),
            other => panic!("{other:?}"),
        }
        let missing = teams_proto::Error::status("convert to pdf", 404, "itemNotFound");
        assert!(matches!(
            conversion_error(missing, "docx"),
            Error::Graph(teams_proto::Error::Status { status: 404, .. })
        ));
    }
}
