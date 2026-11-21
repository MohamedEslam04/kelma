//! Native file preview for IronTeams (data side, no UI): PDF pages to RGBA, Office files to PDF.
//!
//! * [`Document`] opens a PDF and renders a page to an [`Rgba`] bitmap at a chosen width.
//! * [`office_to_pdf`] asks Graph to convert a Word, Excel, PowerPoint (and similar) file to
//!   PDF (`GET …/content?format=pdf`), so it can go through the same renderer.
//!
//! # Backend: hayro
//!
//! PDF pages are rasterized by [`hayro`], a pure-Rust PDF renderer (Apache-2.0 OR MIT, so it
//! fits IronTeams's GPL-3.0). It needs no system library at build time or at run time: it builds
//! the same on a developer machine and in Ubuntu CI, and there is nothing to install for the
//! user. The standard 14 fonts and the CJK CMaps are embedded. The alternatives, weighed when
//! this crate was made:
//!
//! * `pdfium-render` binds Google's PDFium at run time (`libpdfium.so`). The best fidelity,
//!   but IronTeams would have to ship or ask the user for a prebuilt binary, and the preview would
//!   fail on a machine without it.
//! * `mupdf` builds MuPDF from source; MuPDF is AGPL, which would turn the whole of IronTeams
//!   into an AGPL work.
//! * `poppler` bindings need the poppler-glib development files, which a plain machine and CI
//!   do not have.
//!
//! hayro's known gaps (from its documentation): no blending or isolation groups, no knockout
//! groups, no colour-key masking, and it is not tuned for speed yet. A page that uses those
//! still renders, with those effects missing.
//!
//! # Threads
//!
//! A [`Document`] is `Send + Sync`: open it on any thread and render pages from a worker
//! thread (rendering is CPU work; keep it off the UI thread). Each [`Document::render_page`]
//! call is independent, so different pages can render on different threads at once.

mod office;
mod pdf;

pub use office::{CONVERTIBLE, can_convert, office_to_pdf, pdf_url};
pub use pdf::{Document, MAX_SIDE, Rgba};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("this file is not a PDF or is damaged")]
    Invalid,
    #[error("this PDF is protected with a password")]
    PasswordProtected,
    #[error("this PDF uses encryption that cannot be opened")]
    Encrypted,
    #[error("page {page} does not exist (the document has {count} pages)")]
    NoPage { page: usize, count: usize },
    #[error("cannot render a page {width} pixels wide (1 to {MAX_SIDE})")]
    BadWidth { width: u32 },
    /// The file type (its extension, lowercase) cannot be previewed.
    #[error("{}", unsupported(.0))]
    Unsupported(String),
    #[error("could not read the file: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Graph(#[from] teams_proto::Error),
}

fn unsupported(ext: &str) -> String {
    if ext.is_empty() {
        "cannot preview files without an extension".into()
    } else {
        format!("cannot preview .{ext} files")
    }
}
