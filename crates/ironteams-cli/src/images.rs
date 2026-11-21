//! `ironteams-cli images`: list, fetch and save the images of one message (the data layer of the
//! image viewer, plan item 9.5).

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use teams_proto::Client;
use teams_proto::images::{self, ImageHost, ImageRef};

/// `ironteams-cli images <conv> [<message id>|last] [--out <dir>] [--list] [--scan <n>]`
///
/// Takes the images of one message in order (inline images, then attached image files) and
/// saves each full-size picture into the directory (default `./images`), never overwriting.
/// `last` (the default) means the newest message with an image among the last `--scan` messages
/// (default 50). `--list` prints the references and the grid layout without downloading.
pub async fn run(client: &Client, args: &[String]) -> Result<()> {
    let mut pos = Vec::new();
    let (mut out, mut list, mut scan) = (PathBuf::from("images"), false, 50u32);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = it.next().context("--out needs a directory")?.into(),
            "--scan" => {
                scan = it
                    .next()
                    .and_then(|n| n.parse().ok())
                    .context("--scan needs a number")?
            }
            "--list" => list = true,
            _ => pos.push(a.as_str()),
        }
    }
    let conv = pos
        .first()
        .context("usage: images <conv> [<message id>|last] [--out <dir>] [--list] [--scan <n>]")?;
    let which = pos.get(1).copied().unwrap_or("last");
    let page = client.messages(conv, None, scan).await?;
    let mut found = page
        .messages
        .iter()
        .filter(|m| m.is_chat_content())
        .map(|m| (m, images::message_images(&m.content, &m.files)))
        .filter(|(m, imgs)| !imgs.is_empty() && (which == "last" || m.id == which));
    // The page is oldest first, so the newest match is the last one.
    let (msg, imgs) = found
        .next_back()
        .context("no message with images among the scanned messages")?;
    println!("message {} has {} image(s)", msg.id, imgs.len());
    let sizes: Vec<(u32, u32)> = imgs.iter().map(|i| i.size().unwrap_or((0, 0))).collect();
    let grid = images::grid_layout(&sizes, 320.0);
    for (n, img) in imgs.iter().enumerate() {
        println!(
            "  #{}  {:<10} posted size {}  candidates {}",
            n + 1,
            host_label(img),
            img.size().map_or("-".into(), |(w, h)| format!("{w}x{h}")),
            img.candidates().len()
        );
    }
    println!(
        "  grid at 320 wide: {} cells, {:.0}x{:.0}, overflow {:?}",
        grid.cells.len(),
        grid.width,
        grid.height,
        grid.cells.iter().find_map(|c| c.overflow)
    );
    if list {
        return Ok(());
    }
    // The target is always a directory here, so create it before the first save.
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    let mut failed = 0;
    for (n, img) in imgs.iter().enumerate() {
        match images::fetch(client, img).await {
            Ok(f) => {
                let name = format!("{:02}-{}", n + 1, img.suggested_name());
                let path = images::save_image(&out, &name, &f.content_type, &f.bytes)
                    .with_context(|| format!("saving image #{}", n + 1))?;
                println!(
                    "  #{} {} bytes {} via {} -> {}",
                    n + 1,
                    f.bytes.len(),
                    f.content_type,
                    view_label(&f.url),
                    path.display()
                );
            }
            Err(e) => {
                failed += 1;
                println!("  #{} failed: {e}", n + 1);
            }
        }
    }
    if failed > 0 {
        bail!("{failed} of {} image(s) could not be fetched", imgs.len());
    }
    Ok(())
}

fn host_label(i: &ImageRef) -> &'static str {
    match i.host {
        ImageHost::Ams => "ams",
        ImageHost::SharePoint(_) => "sharepoint",
        ImageHost::External => "external",
    }
}

/// The AMS view name of a URL (`imgpsh_fullsize`, `imgo`), or `direct` for other hosts.
fn view_label(url: &str) -> &str {
    url.split_once("/views/")
        .map_or("direct", |(_, v)| v.split(['?', '/']).next().unwrap_or(v))
}
