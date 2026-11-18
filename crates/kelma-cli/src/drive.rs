//! `kelma-cli drive …`: exercise the OneDrive module from a terminal.

use anyhow::{Context, Result, bail};
use onedrive::{DocKind, DriveItem, ItemRef};
use teams_proto::Client;

fn when(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    // Civil date from days since 1970-01-01 (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn print_items(items: &[DriveItem]) {
    for i in items {
        println!(
            "{} {:>10} {} {}  [{}]",
            if i.is_folder { "d" } else { "-" },
            if i.is_folder {
                format!("{} items", i.child_count)
            } else {
                i.size.to_string()
            },
            when(i.modified_ms),
            i.name,
            i.id
        );
    }
    println!("({} items)", items.len());
}

pub async fn run(client: &Client, args: &[String]) -> Result<()> {
    let arg = |n: usize| args.get(n).map(String::as_str);
    match arg(0) {
        Some("ls") => {
            let path = arg(1).unwrap_or("");
            let folder = onedrive::item_at(client, path).await?;
            if !folder.is_folder {
                bail!("{path} is not a folder");
            }
            let mut items = Vec::new();
            let mut next = None;
            loop {
                let page = onedrive::children(client, "", &folder.id, next.as_deref()).await?;
                items.extend(page.items);
                next = page.next_link;
                if next.is_none() {
                    break;
                }
            }
            onedrive::sort_items(&mut items, onedrive::SortKey::Name, true);
            print_items(&items);
        }
        Some("search") => {
            let q = args.get(1..).unwrap_or_default().join(" ");
            if q.is_empty() {
                bail!("usage: drive search <query>");
            }
            print_items(&onedrive::search(client, &q).await?);
        }
        Some("recent") => print_items(&onedrive::recent(client).await?),
        Some("shared") => print_items(&onedrive::shared_with_me(client).await?),
        Some("favorites") => print_items(&onedrive::following(client).await?.items),
        Some("recordings") => print_items(&onedrive::recordings(client).await?),
        Some("libraries") => {
            for s in onedrive::followed_sites(client).await? {
                let drives = onedrive::site_drives(client, &s.id)
                    .await
                    .unwrap_or_default();
                println!("{}  ({} libraries)  {}", s.name, drives.len(), s.web_url);
            }
        }
        Some("thumb") => {
            // Thumbnails of the first recent file that has any (prints which sizes exist).
            for i in onedrive::recent(client)
                .await?
                .iter()
                .filter(|i| !i.is_folder)
                .take(10)
            {
                let t = onedrive::thumbnails(client, &ItemRef::from(i)).await?;
                if t.small.is_some() || t.medium.is_some() || t.large.is_some() {
                    println!(
                        "thumbnails: small={} medium={} large={}",
                        t.small.is_some(),
                        t.medium.is_some(),
                        t.large.is_some()
                    );
                    return Ok(());
                }
            }
            println!("thumbnails: none among the first 10 recent files");
        }
        Some("team-drives") => {
            // The Documents library of every joined team (Quick access).
            let teams = client
                .json_as(
                    "teams",
                    &teams_proto::broker::Audience::Graph,
                    reqwest::Method::GET,
                    "https://graph.microsoft.com/v1.0/me/joinedTeams?$select=id",
                    None,
                )
                .await?;
            for id in teams["value"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t["id"].as_str())
            {
                let d = onedrive::group_drive(client, id).await?;
                let kids = onedrive::children(client, &d.id, "root", None).await?;
                println!(
                    "team library: type {}, {} items at its root",
                    d.drive_type,
                    kids.items.len()
                );
            }
        }
        Some("sync-help") => print!("{}", onedrive::sync_help()),
        Some("bin") => {
            let items = onedrive::recycle_bin(client).await?;
            for b in &items {
                println!(
                    "{} {:>10} {} {}  [{}]",
                    if b.is_folder { "d" } else { "-" },
                    b.size,
                    when(b.deleted_ms),
                    b.name,
                    b.id
                );
            }
            println!("({} items)", items.len());
        }
        Some("mkdir") => {
            let name = arg(1).context("usage: drive mkdir <name>")?;
            let f = onedrive::create_folder(client, &ItemRef::my_root(), name).await?;
            println!("created {} [{}]", f.name, f.id);
        }
        Some("rm") => {
            let id = arg(1).context("usage: drive rm <item id>")?;
            onedrive::delete(client, &ItemRef::new("", id)).await?;
            println!("deleted (in the recycle bin)");
        }
        Some("selftest") => selftest(client).await?,
        Some("purge") => {
            // Finishes the cleanup of a selftest folder that was interrupted.
            let name = arg(1).context("usage: drive purge <kelma-test-… folder name>")?;
            purge(client, name, false).await?;
        }
        _ => bail!(
            "usage: drive <ls [path]|search <q>|recent|shared|favorites|recordings|libraries|thumb|team-drives|sync-help|bin|mkdir <name>|rm <id>|selftest|purge <name>>"
        ),
    }
    Ok(())
}

/// Write checks, all inside one new folder `kelma-test-<timestamp>` in My files that is deleted
/// and purged from the recycle bin at the end. Prints counts only, never file names.
async fn selftest(client: &Client) -> Result<()> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let name = format!("kelma-test-{ts}");
    let root = onedrive::create_folder(client, &ItemRef::my_root(), &name).await?;
    println!("created test folder (id {})", root.id);
    let result = match selftest_inside(client, &root).await {
        Ok(()) => restore_roundtrip(client, &root).await,
        e => e,
    };
    if let Err(e) = &result {
        println!("FAILED inside the test folder: {e}");
    }
    let cleaned = cleanup(client, &root).await;
    match (&result, &cleaned) {
        (Ok(()), Ok(())) => println!("selftest passed and cleaned up"),
        (_, Err(e)) => println!("CLEANUP FAILED: {e}"),
        _ => {}
    }
    result?;
    cleaned
}

/// Delete the folder, find it in the bin and restore it: recycle bin list and restore, live.
async fn restore_roundtrip(client: &Client, root: &DriveItem) -> Result<()> {
    onedrive::delete(client, &ItemRef::from(root)).await?;
    let entry = bin_items_of(client, &root.name)
        .await?
        .into_iter()
        .find(|b| b.name == root.name && b.is_folder)
        .context("the deleted folder is not in the recycle bin")?;
    onedrive::restore(client, &entry.id).await?;
    let back = onedrive::item_at(client, &root.name).await?;
    let kids = onedrive::children(client, "", &back.id, None)
        .await?
        .items
        .len();
    println!("recycle bin: deleted folder listed, restored with {kids} items");
    Ok(())
}

/// Items in the recycle bin that belong to the test folder `name`: the folder itself and
/// anything listed under its path.
async fn bin_items_of(client: &Client, name: &str) -> Result<Vec<onedrive::BinItem>> {
    Ok(onedrive::recycle_bin(client)
        .await?
        .into_iter()
        .filter(|b| b.name == name || b.dir.split('/').any(|s| s == name))
        .collect())
}

/// Deletes the test folder (if still there) and empties only its items from the recycle bin.
/// Retries, because the SharePoint endpoints sometimes time out.
async fn cleanup(client: &Client, root: &DriveItem) -> Result<()> {
    let mut last = None;
    for attempt in 1..=5 {
        match purge(client, &root.name, true).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                println!("cleanup attempt {attempt} failed: {e}");
                last = Some(e);
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("cleanup failed")))
}

async fn purge(client: &Client, name: &str, delete_folder: bool) -> Result<()> {
    if !name.starts_with("kelma-test-") {
        bail!("refusing to purge {name}: only kelma-test-* folders");
    }
    if delete_folder {
        // By path, so a restored folder is found whatever its id is now.
        match onedrive::item_at(client, name).await {
            Ok(it) => onedrive::delete(client, &ItemRef::from(&it)).await?,
            Err(teams_proto::Error::Status { status: 404, .. }) => {}
            Err(e) => return Err(e.into()),
        }
    }
    let ours = bin_items_of(client, name).await?;
    for b in &ours {
        onedrive::delete_forever(client, &b.id).await?;
    }
    let left = bin_items_of(client, name).await?.len();
    let gone = onedrive::item_at(client, name).await.is_err();
    println!(
        "purged {} of ours from the recycle bin; {left} of ours left; folder gone from My files: {gone}; bin now {} items",
        ours.len(),
        onedrive::recycle_bin(client).await?.len()
    );
    if left != 0 || !gone {
        bail!("test items remain");
    }
    Ok(())
}

async fn selftest_inside(client: &Client, root: &DriveItem) -> Result<()> {
    let r = ItemRef::from(root);
    // Upload: small and chunked.
    let small = onedrive::upload_file(client, &r, "a.txt", b"hello".to_vec()).await?;
    anyhow::ensure!(small.size == 5, "small upload size {}", small.size);
    let big_data: Vec<u8> = (0..4_500_000u32).map(|i| (i % 251) as u8).collect();
    let big = onedrive::upload_file(client, &r, "big.bin", big_data.clone()).await?;
    anyhow::ensure!(big.size == 4_500_000, "big upload size {}", big.size);
    println!("upload: small 5 bytes ok, chunked {} bytes ok", big.size);
    // Documents.
    for kind in [
        DocKind::Word,
        DocKind::Excel,
        DocKind::PowerPoint,
        DocKind::Markdown,
        DocKind::Text,
    ] {
        let d = onedrive::create_office_doc(client, &r, kind, None).await?;
        let mut note = String::new();
        if matches!(kind, DocKind::Word | DocKind::Excel | DocKind::PowerPoint) {
            // The server opens the package: PDF conversion for Word and PowerPoint, and the
            // Excel workbook API for a (still empty, so not printable) workbook.
            let is_xlsx = kind == DocKind::Excel;
            let url = if is_xlsx {
                format!("{}/workbook/worksheets", ItemRef::from(&d).url())
            } else {
                format!("{}/content?format=pdf", ItemRef::from(&d).url())
            };
            let resp = client
                .request_raw_as(
                    &teams_proto::broker::Audience::Graph,
                    reqwest::Method::GET,
                    &url,
                    teams_proto::client::RawBody::None,
                    &[],
                )
                .await?;
            anyhow::ensure!(
                resp.status == 200 && (is_xlsx || resp.body.starts_with(b"%PDF")),
                "{} was not opened by the server (HTTP {})",
                d.name,
                resp.status
            );
            let preview = onedrive::preview_url(client, &ItemRef::from(&d)).await;
            note = format!(
                ", server opened it, preview {}",
                if preview.is_ok() { "ok" } else { "n/a" }
            );
        }
        println!("create {:?}: {} bytes{note}", kind, d.size);
    }
    // Rename, folder, move, copy.
    let renamed = onedrive::rename(client, &ItemRef::from(&small), "b.txt").await?;
    anyhow::ensure!(renamed.name == "b.txt", "rename");
    let sub = onedrive::create_folder(client, &r, "sub").await?;
    let moved =
        onedrive::move_to(client, &ItemRef::from(&renamed), &ItemRef::from(&sub), None).await?;
    anyhow::ensure!(
        moved.parent_path.ends_with("sub"),
        "move to {}",
        moved.parent_path
    );
    let copied = onedrive::copy_to(
        client,
        &ItemRef::from(&big),
        &ItemRef::from(&sub),
        Some("big-copy.bin"),
    )
    .await?;
    anyhow::ensure!(copied.size == 4_500_000, "copy size");
    println!("rename, move, copy ok");
    // Download file and folder.
    let back = onedrive::download(client, &copied).await?;
    anyhow::ensure!(back == big_data, "downloaded bytes differ");
    let zip = onedrive::download(client, &sub).await?;
    anyhow::ensure!(zip.starts_with(b"PK"), "folder download is not a zip");
    println!(
        "download: file bytes identical, folder zip {} bytes",
        zip.len()
    );
    // Recursive folder upload from a temporary local tree.
    let tmp = std::env::temp_dir().join(format!("kelma-up-{}", std::process::id()));
    std::fs::create_dir_all(tmp.join("inner/deeper"))?;
    std::fs::write(tmp.join("one.txt"), "1")?;
    std::fs::write(tmp.join("inner/two.txt"), "22")?;
    std::fs::write(tmp.join("inner/deeper/three.txt"), "333")?;
    let up = onedrive::upload_folder(client, &r, &tmp).await;
    std::fs::remove_dir_all(&tmp).ok();
    let up = up?;
    anyhow::ensure!(up.files == 3 && up.folders == 3, "folder upload {up:?}");
    println!("upload_folder: {} folders, {} files", up.folders, up.files);
    // Favorites, versions, permissions (read-only).
    let before = onedrive::following(client).await?.items.len();
    onedrive::follow(client, &ItemRef::from(&moved)).await?;
    let during = onedrive::following(client).await?.items.len();
    onedrive::unfollow(client, &ItemRef::from(&moved)).await?;
    let after = onedrive::following(client).await?.items.len();
    println!("favorites: {before} -> {during} -> {after}");
    let vs = onedrive::versions(client, &ItemRef::from(&big)).await?;
    println!("versions of big file: {}", vs.len());
    let perms = onedrive::permissions(client, &ItemRef::from(&big)).await?;
    println!(
        "permissions on big file: {} (no links or invites created)",
        perms.len()
    );
    // A shortcut needs a folder of another drive: borrow one from "shared with me" (read-only
    // use), add the shortcut inside the test folder, check it, and delete only the stub.
    let my_drive = onedrive::my_drive(client).await?.id;
    let shared = onedrive::shared_with_me(client).await?;
    let target = shared
        .iter()
        .find(|i| i.is_folder && !i.drive_id.is_empty() && i.drive_id != my_drive);
    match target {
        None => println!("shortcut: skipped (no folder of another drive is shared with this user)"),
        Some(t) => match onedrive::add_shortcut(client, t, &r, Some("shortcut")).await {
            Ok(s) => {
                let (stub_drive, stub_id) = s.stub.clone().context("shortcut has no stub")?;
                anyhow::ensure!(
                    stub_drive == my_drive && stub_id != t.id,
                    "the shortcut stub must be a new item in My files"
                );
                onedrive::delete(client, &ItemRef::own(&s)).await?;
                let target_still_there = onedrive::children(client, &t.drive_id, &t.id, None)
                    .await
                    .is_ok();
                anyhow::ensure!(target_still_there, "the shortcut target must be untouched");
                println!(
                    "shortcut: created (remote={}), stub deleted, target untouched: {target_still_there}",
                    s.is_remote_shortcut
                );
            }
            Err(e) => println!("shortcut: refused ({e})"),
        },
    }
    // Listing the test folder.
    let page = onedrive::children(client, "", &root.id, None).await?;
    println!("test folder now lists {} items", page.items.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::when;

    #[test]
    fn dates_are_civil_utc() {
        assert_eq!(when(0), "1970-01-01");
        assert_eq!(when(1_794_299_400_000), "2026-11-10");
        assert_eq!(when(1_709_164_800_000), "2024-02-29");
    }
}
