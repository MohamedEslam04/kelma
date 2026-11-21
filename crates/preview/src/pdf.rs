//! Opening a PDF and rendering its pages with hayro.

use std::path::Path;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_syntax::page::Page;
use hayro::hayro_syntax::{DecryptionError, LoadPdfError, Pdf};
use hayro::kurbo::Affine;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::peniko::ImageAlphaType;
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{RenderCache, RenderSettings, render_into};

use crate::{Error, Result};

/// The largest width or height of a rendered page, in pixels.
pub const MAX_SIDE: u32 = 8192;

/// A rendered page: `width * height` pixels, 4 bytes each (R, G, B, A; not premultiplied),
/// row by row from the top. Pages are drawn on white, so every pixel is opaque.
#[derive(Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl std::fmt::Debug for Rgba {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Rgba({}x{})", self.width, self.height)
    }
}

impl Rgba {
    /// Encodes the bitmap as PNG (for saving a page or debugging).
    pub fn to_png(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, self.width, self.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(std::io::Error::other)?;
        w.write_image_data(&self.pixels)
            .map_err(std::io::Error::other)?;
        w.finish().map_err(std::io::Error::other)?;
        Ok(out)
    }
}

/// An open PDF.
pub struct Document {
    pdf: Pdf,
}

impl Document {
    /// Opens a PDF from its bytes. A PDF encrypted with an empty user password (common for
    /// "no printing" restrictions) opens; one that needs a password does not.
    pub fn open(bytes: Vec<u8>) -> Result<Document> {
        let pdf = Pdf::new(bytes).map_err(|e| match e {
            LoadPdfError::Decryption(DecryptionError::PasswordProtected) => {
                Error::PasswordProtected
            }
            LoadPdfError::Decryption(_) => Error::Encrypted,
            LoadPdfError::Invalid => Error::Invalid,
        })?;
        Ok(Document { pdf })
    }

    /// Opens a PDF file.
    pub fn open_path(path: &Path) -> Result<Document> {
        Document::open(std::fs::read(path)?)
    }

    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    fn page(&self, page: usize) -> Result<&Page<'_>> {
        let pages = self.pdf.pages();
        pages.get(page).ok_or(Error::NoPage {
            page,
            count: pages.len(),
        })
    }

    /// Size of page `page` (from 0) in points (1/72 inch) as shown: crop box, rotation applied.
    pub fn page_size(&self, page: usize) -> Result<(f32, f32)> {
        Ok(self.page(page)?.render_dimensions())
    }

    /// Renders page `page` (from 0) `width` pixels wide (1 to [`MAX_SIDE`]); the height keeps
    /// the page's aspect. A page so tall that the height would pass [`MAX_SIDE`] is rendered
    /// [`MAX_SIDE`] pixels tall and correspondingly narrower.
    pub fn render_page(&self, page: usize, width: u32) -> Result<Rgba> {
        if !(1..=MAX_SIDE).contains(&width) {
            return Err(Error::BadWidth { width });
        }
        let page = self.page(page)?;
        let (pw, ph) = page.render_dimensions();
        let (pw, ph) = (pw.max(1.0) as f64, ph.max(1.0) as f64);
        let mut w = width as f64;
        let mut h = (ph * w / pw).round().max(1.0);
        if h > MAX_SIDE as f64 {
            h = MAX_SIDE as f64;
            w = (pw * h / ph).round().max(1.0);
        }
        let (w, h) = (w as u16, h as u16);

        let mut ctx = RenderContext::new(w, h);
        // Scale to the exact pixel size; `initial_transform` handles the crop box and rotation.
        let transform = Affine::scale_non_uniform(w as f64 / pw, h as f64 / ph)
            * page.initial_transform(true).to_kurbo();
        let cache = RenderCache::new();
        render_into(
            page,
            &cache,
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &mut ctx,
            transform,
        );
        ctx.flush();
        let mut pixmap = Pixmap::new(w, h);
        ctx.render_with(
            &mut pixmap,
            &mut Resources::default(),
            RasterizerSettings {
                target_init: TargetInit::Clear(WHITE),
                ..Default::default()
            },
        );
        Ok(Rgba {
            width: w.into(),
            height: h.into(),
            pixels: pixmap.take_rgba8(ImageAlphaType::Alpha),
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal valid PDF: one page per `(width, height)` in points, each with a black square
    /// filling the top-left quarter. The xref offsets are computed, so hayro reads it as is.
    pub(crate) fn synthetic_pdf(pages: &[(u32, u32)]) -> Vec<u8> {
        let n = pages.len();
        // Objects: 1 catalog, 2 pages, then a page and its content stream per page.
        let mut objects: Vec<String> = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            format!(
                "<< /Type /Pages /Count {n} /Kids [{}] >>",
                (0..n)
                    .map(|i| format!("{} 0 R", 3 + 2 * i))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        ];
        for (i, (w, h)) in pages.iter().enumerate() {
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents {} 0 R >>",
                4 + 2 * i
            ));
            let content = format!("0 0 0 rg 0 {} {} {} re f", h / 2, w / 2, h / 2);
            objects.push(format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ));
        }
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for off in offsets {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    fn pixel(r: &Rgba, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * r.width + x) * 4) as usize;
        r.pixels[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn counts_pages_and_reads_sizes() {
        let doc = Document::open(synthetic_pdf(&[(612, 792), (842, 595)])).unwrap();
        assert_eq!(doc.page_count(), 2);
        assert_eq!(doc.page_size(0).unwrap(), (612.0, 792.0));
        assert_eq!(doc.page_size(1).unwrap(), (842.0, 595.0));
        assert!(matches!(
            doc.page_size(2),
            Err(Error::NoPage { page: 2, count: 2 })
        ));
    }

    #[test]
    fn renders_at_width_keeping_aspect() {
        let doc = Document::open(synthetic_pdf(&[(612, 792), (842, 595)])).unwrap();
        let r = doc.render_page(0, 300).unwrap();
        assert_eq!((r.width, r.height), (300, 388)); // 792 * 300 / 612 = 388.2
        assert_eq!(r.pixels.len(), 300 * 388 * 4);
        let landscape = doc.render_page(1, 421).unwrap();
        assert_eq!((landscape.width, landscape.height), (421, 298)); // 595 / 2 = 297.5
    }

    #[test]
    fn draws_the_page_content_on_white() {
        let doc = Document::open(synthetic_pdf(&[(200, 100)])).unwrap();
        let r = doc.render_page(0, 400).unwrap();
        assert_eq!((r.width, r.height), (400, 200));
        // The black square fills the top-left quarter; the rest is white paper.
        assert_eq!(pixel(&r, 50, 50), [0, 0, 0, 255]);
        assert_eq!(pixel(&r, 350, 150), [255, 255, 255, 255]);
        assert_eq!(pixel(&r, 350, 50), [255, 255, 255, 255]);
        assert_eq!(pixel(&r, 50, 150), [255, 255, 255, 255]);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(
            Document::open(b"not a pdf".to_vec()),
            Err(Error::Invalid)
        ));
        let doc = Document::open(synthetic_pdf(&[(100, 100)])).unwrap();
        assert!(matches!(
            doc.render_page(1, 100),
            Err(Error::NoPage { page: 1, count: 1 })
        ));
        assert!(matches!(
            doc.render_page(0, 0),
            Err(Error::BadWidth { width: 0 })
        ));
        assert!(matches!(
            doc.render_page(0, MAX_SIDE + 1),
            Err(Error::BadWidth { .. })
        ));
    }

    #[test]
    fn a_very_tall_page_is_capped_to_max_side() {
        // 10 x 2000 points at 1000 px wide would be 200,000 px tall.
        let doc = Document::open(synthetic_pdf(&[(10, 2000)])).unwrap();
        let r = doc.render_page(0, 1000).unwrap();
        assert_eq!(r.height, MAX_SIDE);
        assert_eq!(r.width, 41); // 8192 * 10 / 2000 = 40.96, aspect kept
    }

    #[test]
    fn encodes_png() {
        let doc = Document::open(synthetic_pdf(&[(200, 100)])).unwrap();
        let png = doc.render_page(0, 40).unwrap().to_png().unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        // IHDR width and height, big-endian, right after the signature and chunk header.
        assert_eq!(&png[16..24], &[0, 0, 0, 40, 0, 0, 0, 20]);
    }

    #[test]
    fn opens_from_a_path() {
        let dir = std::env::temp_dir().join(format!("ironteams-preview-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("doc.pdf");
        std::fs::write(&path, synthetic_pdf(&[(100, 100), (100, 100), (100, 100)])).unwrap();
        let doc = Document::open_path(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(doc.page_count(), 3);
        assert!(matches!(
            Document::open_path(&dir.join("missing.pdf")),
            Err(Error::Io(_))
        ));
    }

    #[test]
    fn document_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<Document>();
        let doc = std::sync::Arc::new(Document::open(synthetic_pdf(&[(100, 100); 2])).unwrap());
        let handles: Vec<_> = (0..2)
            .map(|i| {
                let doc = doc.clone();
                std::thread::spawn(move || doc.render_page(i, 50).unwrap().width)
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap(), 50);
        }
    }
}
