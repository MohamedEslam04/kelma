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

/// Keys whose string values are enums or types (safe to print), not tenant data.
const SHAPE_KEEP: &[&str] = &[
    "type",
    "@odata.type",
    "sharingType",
    "containerType",
    "mediaType",
    "mimeType",
    "driveType",
    "entityType",
    "scope",
    "code",
];

/// The host of an http(s) URL with any tenant label hidden: `<tenant>.sharepoint.com`,
/// `<tenant>-my.sharepoint.com`; other hosts (Graph, Teams, Outlook) are shown as they are.
fn url_host(s: &str) -> Option<String> {
    let u = url::Url::parse(s).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    let h = u.host_str()?.to_ascii_lowercase();
    Some(match h.strip_suffix(".sharepoint.com") {
        Some(label) if label.ends_with("-my") => "<tenant>-my.sharepoint.com".into(),
        Some(_) => "<tenant>.sharepoint.com".into(),
        None if h.contains("sharepoint") || h.ends_with(".onmicrosoft.com") => {
            "<tenant host>".into()
        }
        None => h,
    })
}

/// The structure of a JSON answer with its data redacted: strings become `<str N>` unless the
/// key is an enum in [`SHAPE_KEEP`], URLs their host, numbers `<num>`, and arrays show their
/// first `n` elements followed by `<N items>`. For protocol discovery without printing tenant data.
fn shape(v: &serde_json::Value, n: usize) -> serde_json::Value {
    use serde_json::Value;
    fn walk(v: &Value, key: &str, n: usize) -> Value {
        match v {
            Value::String(s) if SHAPE_KEEP.contains(&key) => Value::String(s.clone()),
            Value::String(s) => Value::String(match url_host(s) {
                Some(h) => format!("<url {h}>"),
                None => format!("<str {}>", s.chars().count()),
            }),
            Value::Number(_) => Value::String("<num>".into()),
            Value::Array(a) => {
                let mut out: Vec<Value> = a.iter().take(n).map(|x| walk(x, key, n)).collect();
                out.push(Value::String(format!("<{} items>", a.len())));
                Value::Array(out)
            }
            Value::Object(o) => {
                Value::Object(o.iter().map(|(k, x)| (k.clone(), walk(x, k, n))).collect())
            }
            other => other.clone(),
        }
    }
    walk(v, "", n)
}

/// What `drive new` makes.
#[derive(Debug, PartialEq)]
enum NewArg {
    Item(onedrive::NewItem),
    /// Forms has no API for this client: the survey is made in the browser.
    Forms,
}

const NEW_USAGE: &str = "usage: drive new <word|excel|powerpoint|visio|markdown|text> [name] | new link <name> <https url> | new notebook <name> | new forms  [--dry-run]";

/// `drive new …` arguments and whether `--dry-run` was given.
fn parse_new(args: &[String]) -> Result<(NewArg, bool)> {
    use onedrive::NewItem;
    let dry = args.iter().any(|a| a == "--dry-run");
    let rest: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| *a != "--dry-run")
        .collect();
    let name = rest.get(1).map(|s| (*s).to_owned());
    let kind = match rest.first().copied() {
        Some("word") => DocKind::Word,
        Some("excel") => DocKind::Excel,
        Some("powerpoint") => DocKind::PowerPoint,
        Some("visio") => DocKind::Visio,
        Some("markdown") => DocKind::Markdown,
        Some("text") => DocKind::Text,
        Some("link") => {
            let name = name.context(NEW_USAGE)?;
            let url = rest.get(2).context(NEW_USAGE)?;
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                bail!("a link must be an http(s) URL");
            }
            let url = (*url).to_owned();
            return Ok((NewArg::Item(NewItem::Link { name, url }), dry));
        }
        Some("notebook") => {
            let name = name.context(NEW_USAGE)?;
            return Ok((NewArg::Item(NewItem::Notebook { name }), dry));
        }
        Some("forms") => return Ok((NewArg::Forms, dry)),
        _ => bail!(NEW_USAGE),
    };
    Ok((NewArg::Item(NewItem::Doc { kind, name }), dry))
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

/// True for commands that need no session: `new … --dry-run` and `new forms`.
pub fn is_offline(args: &[String]) -> bool {
    args.first().is_some_and(|a| a == "new")
        && matches!(
            parse_new(&args[1..]),
            Ok((NewArg::Forms, _)) | Ok((_, true))
        )
}

/// Runs an [`is_offline`] command: prints the request (or the Forms address), sends nothing.
pub fn offline(args: &[String]) -> Result<()> {
    match parse_new(&args[1..])?.0 {
        NewArg::Forms => println!("open in the browser: {}", onedrive::FORMS_NEW_URL),
        NewArg::Item(item) => println!(
            "dry run, nothing sent:\n{}",
            onedrive::plan(&ItemRef::my_root(), &item)
        ),
    }
    Ok(())
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
        Some("meetings") => {
            // My files and the channel libraries: counts per place, then the list.
            let all = onedrive::meeting_recordings(client).await?;
            let mine = all
                .iter()
                .filter(|r| r.place == onedrive::around::MY_FILES)
                .count();
            println!(
                "{} recordings: {mine} in My files, {} in channels",
                all.len(),
                all.len() - mine
            );
            let items: Vec<DriveItem> = all.into_iter().map(|r| r.item).collect();
            print_items(&items);
        }
        Some("for-you") => {
            for c in onedrive::for_you(client, 12).await? {
                println!(
                    "{:?} {}  [{}]",
                    c.insight.kind, c.insight.file_type, c.reason
                );
            }
        }
        Some("shared-by-me") => {
            let v = onedrive::shared_by_me(client).await?;
            let items: Vec<DriveItem> = v.iter().map(onedrive::Insight::to_drive_item).collect();
            print_items(&items);
        }
        Some("people") => {
            let groups = onedrive::people(client).await?;
            for g in &groups {
                println!("{} files, latest {}", g.files.len(), when(g.latest_ms));
            }
            println!("({} people)", groups.len());
        }
        Some("media") => {
            let page = onedrive::media(client, 0, 50).await?;
            print_items(&page.items);
            let mine = onedrive::my_drive(client).await?.id;
            let own = page.items.iter().filter(|i| i.drive_id == mine).count();
            println!(
                "more: {}; in My files: {own} of {}",
                page.more,
                page.items.len()
            );
        }
        Some("new") => {
            let (what, dry) = parse_new(&args[1..])?;
            let item = match what {
                NewArg::Forms => {
                    println!("open in the browser: {}", onedrive::FORMS_NEW_URL);
                    return Ok(());
                }
                NewArg::Item(i) => i,
            };
            let parent = ItemRef::my_root();
            if dry {
                println!("dry run, nothing sent:\n{}", onedrive::plan(&parent, &item));
                return Ok(());
            }
            match onedrive::create(client, &parent, &item).await? {
                onedrive::Created::Item(i) => println!("created {} [{}]", i.name, i.id),
                onedrive::Created::Notebook(n) => {
                    println!("created notebook {} [{}]", n.name, n.id)
                }
            }
        }
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
        Some("shape") => {
            // Read-only protocol discovery: GET a Graph path and print its redacted shape.
            let path =
                arg(1).context("usage: drive shape </graph/v1.0/path?query> [array items]")?;
            let n = arg(2).and_then(|n| n.parse().ok()).unwrap_or(2);
            if !path.starts_with('/') {
                bail!("give a Graph path starting with /");
            }
            let (status, text) = client
                .request_as(
                    &teams_proto::broker::Audience::Graph,
                    reqwest::Method::GET,
                    &format!("{}{path}", onedrive::api::GRAPH),
                    None,
                    &[],
                )
                .await?;
            let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            println!("HTTP {status}");
            if status >= 400 {
                println!(
                    "error: {}",
                    v["error"]["message"].as_str().unwrap_or_default()
                );
            }
            println!("{}", serde_json::to_string_pretty(&shape(&v, n))?);
        }
        Some("shape-search") => {
            // Read-only: the redacted shape of a Graph Search answer for a KQL query.
            let q = args.get(1..).unwrap_or_default().join(" ");
            let body = onedrive::browse::search_body(&q, 0, 5, true);
            let v = client
                .json_as(
                    "search",
                    &teams_proto::broker::Audience::Graph,
                    reqwest::Method::POST,
                    &format!("{}/search/query", onedrive::api::GRAPH),
                    Some(&body),
                )
                .await?;
            println!("{}", serde_json::to_string_pretty(&shape(&v, 2))?);
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
            "usage: drive <ls [path]|search <q>|recent|shared|favorites|recordings|meetings|for-you|shared-by-me|people|media|new …|shape <path>|shape-search <kql>|libraries|thumb|team-drives|sync-help|bin|mkdir <name>|rm <id>|selftest|purge <name>>"
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
        DocKind::Visio,
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
    // A link (an Internet shortcut file) through the "New" menu's request.
    let link = onedrive::NewItem::Link {
        name: "link".into(),
        url: "https://example.com/".into(),
    };
    match onedrive::create(client, &r, &link).await? {
        onedrive::Created::Item(l) => println!("create Link: {} bytes", l.size),
        onedrive::Created::Notebook(_) => bail!("a link made a notebook"),
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
    use super::{NewArg, parse_new, shape, when};
    use onedrive::{DocKind, NewItem};
    use serde_json::json;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn dry_runs_and_forms_need_no_session() {
        assert!(super::is_offline(&args(&["new", "visio", "--dry-run"])));
        assert!(super::is_offline(&args(&["new", "forms"])));
        assert!(!super::is_offline(&args(&["new", "visio"])));
        assert!(!super::is_offline(&args(&[
            "new",
            "link",
            "x",
            "--dry-run"
        ])));
        assert!(!super::is_offline(&args(&["ls", "--dry-run"])));
    }

    #[test]
    fn new_arguments() {
        let (what, dry) = parse_new(&args(&["visio", "Flow", "--dry-run"])).unwrap();
        assert!(dry);
        assert_eq!(
            what,
            NewArg::Item(NewItem::Doc {
                kind: DocKind::Visio,
                name: Some("Flow".into())
            })
        );
        let (what, dry) = parse_new(&args(&["word"])).unwrap();
        assert!(!dry);
        assert_eq!(
            what,
            NewArg::Item(NewItem::Doc {
                kind: DocKind::Word,
                name: None
            })
        );
        assert_eq!(
            parse_new(&args(&["--dry-run", "link", "Wiki", "https://example.com"]))
                .unwrap()
                .0,
            NewArg::Item(NewItem::Link {
                name: "Wiki".into(),
                url: "https://example.com".into()
            })
        );
        assert_eq!(
            parse_new(&args(&["notebook", "Team notes"])).unwrap().0,
            NewArg::Item(NewItem::Notebook {
                name: "Team notes".into()
            })
        );
        assert_eq!(parse_new(&args(&["forms"])).unwrap().0, NewArg::Forms);
        assert!(
            parse_new(&args(&["link", "Wiki"])).is_err(),
            "a link needs a URL"
        );
        assert!(parse_new(&args(&["link", "Wiki", "file:///etc/passwd"])).is_err());
        assert!(
            parse_new(&args(&["notebook"])).is_err(),
            "a notebook needs a name"
        );
        assert!(parse_new(&args(&["spreadsheet"])).is_err());
    }

    #[test]
    fn shape_keeps_structure_and_enums_but_no_data() {
        let v = json!({
            "value": [
                {"name": "Secret plan.docx", "size": 12, "type": "Word",
                 "sharedBy": {"address": "ada@contoso.com"}, "folder": null},
                {"name": "b"}, {"name": "c"}
            ],
            "@odata.nextLink": "https://graph.microsoft.com/v1.0/x?token=abc"
        });
        let s = shape(&v, 2).to_string();
        assert!(!s.contains("Secret") && !s.contains("ada@") && !s.contains("token=abc"));
        assert!(s.contains("\"type\":\"Word\""));
        assert!(s.contains("<str 16>") && s.contains("<num>") && s.contains("<3 items>"));
        assert!(s.contains("<url graph.microsoft.com>"));
        let urls = shape(
            &json!({
            "a": "https://contoso.sharepoint.com/sites/x",
            "b": "https://contoso-my.sharepoint.com/personal/ada",
            "c": "https://teams.microsoft.com/l/message/19:x"
            }),
            2,
        )
        .to_string();
        assert!(!urls.contains("contoso") && !urls.contains("ada"));
        assert!(urls.contains("<url <tenant>-my.sharepoint.com>"));
        assert!(urls.contains("<url teams.microsoft.com>"));
        // Only the first two elements of an array are shown.
        assert_eq!(shape(&v, 2)["value"].as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn dates_are_civil_utc() {
        assert_eq!(when(0), "1970-01-01");
        assert_eq!(when(1_794_299_400_000), "2026-11-10");
        assert_eq!(when(1_709_164_800_000), "2024-02-29");
    }
}
