//! `ironteams-cli preview-file …`: render a page of a PDF or Office file to PNG (read-only).
//!
//! ```text
//! preview-file <path in My files> [--page N] [--width W] [--out F.png]
//! preview-file --find <ext> [...]   the first search hit with that extension (pdf, docx, ...)
//! preview-file --local <file> [...] a PDF on disk (no sign-in)
//! ```
//!
//! Prints the page count and the rendered size, never the file name. Office files are
//! converted to PDF by Graph (`?format=pdf`); the file in OneDrive is not changed.

use anyhow::{Context, Result, bail};
use onedrive::DriveItem;
use teams_proto::Client;

struct Opts {
    target: String,
    page: usize,
    width: u32,
    out: String,
}

fn opts(args: &[String]) -> Result<Opts> {
    let mut o = Opts {
        target: String::new(),
        page: 1,
        width: 1000,
        out: "page.png".into(),
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().with_context(|| format!("{a} needs a value"));
        match a.as_str() {
            "--page" => o.page = val()?.parse().context("--page takes a number from 1")?,
            "--width" => o.width = val()?.parse().context("--width takes a number")?,
            "--out" => o.out = val()?.clone(),
            "--find" | "--local" => o.target = format!("{a} {}", val()?),
            _ if o.target.is_empty() => o.target = a.clone(),
            _ => bail!("unexpected argument {a}"),
        }
    }
    if o.page == 0 {
        bail!("--page counts from 1");
    }
    Ok(o)
}

/// True when the command needs no sign-in (`--local`).
pub fn is_offline(args: &[String]) -> bool {
    args.iter().any(|a| a == "--local")
}

fn render(pdf: Vec<u8>, o: &Opts) -> Result<()> {
    let doc = preview::Document::open(pdf)?;
    println!("pages: {}", doc.page_count());
    let page = doc.render_page(o.page - 1, o.width)?;
    std::fs::write(&o.out, page.to_png()?).with_context(|| format!("writing {}", o.out))?;
    println!(
        "page {} rendered {}x{} -> {}",
        o.page, page.width, page.height, o.out
    );
    Ok(())
}

pub fn offline(args: &[String]) -> Result<()> {
    let o = opts(args)?;
    let path = o
        .target
        .strip_prefix("--local ")
        .context("usage: preview-file --local <file>")?;
    render(
        std::fs::read(path).with_context(|| format!("reading {path}"))?,
        &o,
    )
}

/// The PDF bytes of `item`: the file itself for a PDF, Graph's conversion otherwise.
async fn pdf_of(client: &Client, item: &DriveItem) -> Result<Vec<u8>> {
    if item.name.to_ascii_lowercase().ends_with(".pdf") {
        Ok(onedrive::download(client, item).await?)
    } else {
        Ok(preview::office_to_pdf(client, &item.drive_id, &item.id, &item.name).await?)
    }
}

pub async fn run(client: &Client, args: &[String]) -> Result<()> {
    let o = opts(args)?;
    let item = match o.target.strip_prefix("--find ") {
        Some(ext) => {
            let ext = ext.trim_start_matches('.').to_ascii_lowercase();
            let hits = onedrive::search(client, &format!("filetype:{ext}")).await?;
            hits.into_iter()
                .find(|i| !i.is_folder && i.name.to_ascii_lowercase().ends_with(&format!(".{ext}")))
                .with_context(|| format!("no .{ext} file found"))?
        }
        None if o.target.is_empty() => bail!(
            "usage: preview-file <path>|--find <ext>|--local <file> [--page N] [--width W] [--out F]"
        ),
        None => onedrive::item_at(client, &o.target).await?,
    };
    if item.is_folder {
        bail!("that is a folder");
    }
    println!("size: {} bytes", item.size);
    render(pdf_of(client, &item).await?, &o)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses_options() {
        let o = opts(&args("Docs/a.pdf --page 2 --width 640 --out /tmp/x.png")).unwrap();
        assert_eq!(o.target, "Docs/a.pdf");
        assert_eq!((o.page, o.width, o.out.as_str()), (2, 640, "/tmp/x.png"));
        let o = opts(&args("--find docx")).unwrap();
        assert_eq!(
            (o.target.as_str(), o.page, o.width),
            ("--find docx", 1, 1000)
        );
        assert!(is_offline(&args("--local a.pdf")));
        assert!(!is_offline(&args("--find pdf")));
        assert!(opts(&args("--page 0 a.pdf")).is_err());
        assert!(opts(&args("--width")).is_err());
        assert!(opts(&args("a.pdf b.pdf")).is_err());
    }
}
