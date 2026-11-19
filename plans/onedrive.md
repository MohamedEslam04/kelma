# kelma plan: OneDrive

Part of the Teams parity plans (see `plans/README.md`). Format as in `plans/chat.md`.

**Sources.**
- UI survey of the OneDrive app in Teams web (2026-11-18).
- `docs/research/apps-onedrive-copilot-store.md`.
- The token probe:
  - Graph `Files.ReadWrite.All` and `Sites.ReadWrite.All`
  - SharePoint tokens for both `<tenant>-my.sharepoint.com` and `<tenant>.sharepoint.com` (`Sites.FullControl.All`, `MyFiles.Write`)

**Deprecations already in effect (Nov 2026).** Graph `/me/drive/recent`, `/me/drive/sharedWithMe`
and `/me/insights/*` no longer return data. Home and Shared use Graph Search or the internal
endpoints the Teams web client calls (capture them in O0).

**Feasibility.** Fully native except for editing Office documents (webview or browser) and OneDrive
sync (link to an external Linux client).

## Phase O0: discovery

- [x] **O0.1** Capture the Teams OneDrive app's calls for Home ("For you", Recent), Shared (with you, by you), the recycle bin and Libraries. Document them in `docs/protocol.md`.
  - **Status.** Not done as written: no browser capture of the Teams web app was run. `docs/protocol.md` (section OneDrive) documents the Graph calls that answer for every view the app has, a table of which view uses which call, and what is still missing: the "For you" cards, "Shared by you" and the People and Media lists. Those three need the capture.
  - **Evidence.** 2026-11-19: every call behind Home ("For you", Recent), Shared (with you, by you), People, Media, Meetings (channel libraries), the recycle bin and Libraries was run read-only with `kelma-cli drive shape <graph path>` (structure only, values redacted) and the new `drive for-you|shared-by-me|people|media|meetings` commands, and is documented with its shape and status in `docs/protocol.md` § OneDrive. Refused: `/me/followedSites` (403), `/me/joinedTeams` with `$top` (400). Still no browser capture of the Teams web app: none is needed while these Graph calls answer. Commits 51ef385, 8875140.

## Phase O1: browse

- [x] **O1.1 Navigation.** Impact 4, effort 3.
  - **What.**
    - Home, My files, Shared, Favorites, Libraries, Recycle bin.
    - "Browse files by" People, Meetings or Media.
    - Quick access: the user's team sites.
    - Breadcrumbs inside folders.
  - **How.**
    - My files: `/me/drive/root/children` and `/items/{id}/children` with paging.
    - Favorites: `/me/drive/following`.
    - Libraries: `/me/followedSites` and `/sites/{id}/drives`.
    - Quick access: the team drives (`/groups/{id}/drive`) and channel `filesFolder`.
    - Browse by: Search API facets, or the O0 endpoints.
  - **Status.** Built in `crates/kelma/ui/onedrive.slint`, `crates/kelma/src/backend/drive.rs` and `drive/state.rs` (rail item "OneDrive", after Calendar): Home, My files, Shared, Favorites, Libraries, Recycle bin, Meetings, Quick access (the team libraries) and breadcrumbs, UI tested through the real engine in demo mode. Run live: every section answered (counts only), My files folders opened inside a throwaway folder. "Browse files by" is a heading in the navigation with People, Meetings and Media: People lists who shared files with you (newest share first, "N items" each) and a person opens to their files with the breadcrumbs People › name; Media opens as tiles (photos and videos, newest first, 50 per page with Load more, thumbnails fetched per item in the background).
  - **Evidence (data layer).** People: `onedrive::people` (shared insights grouped by sharer, `sharedWithMe` fallback), run live: 16 people. Media: `onedrive::media` (Search over My files, photo and video extensions), run live: 50 hits on the first page, all in My files. Unit tests in `insights.rs` and `browse.rs`; `kelma-cli drive people|media`.
  - **Evidence (UI, 2026-11-19).** UI tests through the real engine in demo mode: `people_lists_who_shared_and_opens_their_files`, `media_lists_photos_and_videos_as_tiles`, `the_left_navigation_switches_sections`; unit tests in `state.rs` (section keys, nothing selectable in People, newest first) and `drive.rs` (`people_items`). Live, read-only, re-run for this change: `kelma-cli drive people` and `drive media` answered (about 16 people, a first page of 50 media, counts only). The OneDrive window itself was not driven live.
- [x] **O1.2 Home.** Impact 3, effort 3.
  - **What.**
    - "For you" cards ("You frequently open", "X shared this in a Teams chat", with a thumbnail and Open).
    - Recent, with type pills All, Word, Excel, PowerPoint, PDF and More, a "filter by name or person" box, and the columns Name, Opened, Owner and Activity.
  - **How.** The O0 endpoints, with Graph Search (`POST /search/query`, driveItem) as a fallback.
  - **Status.** Recent is built: the pills and the filter box (name or person) narrow the list, and the column picker covers Modified, Modified by, Size, Location and Activity (the columns say Modified, not Opened or Owner). The "For you" cards sit above the list on Home (up to 8: a thumbnail or the file-type badge, the name and the reason, "Sara shared this in a Teams chat"; a click opens the file like a row). They load in the background when the OneDrive view is first opened, so the list never waits for them; they hide while a name filter is typed and on every other section; Refresh on Home reloads them.
  - **Evidence (data layer).** `onedrive::for_you` gives `ForYouCard`s (insight plus reason) from `/me/insights/used`, `trending` and `shared`; run live: 12 cards (shared in a Teams chat, shared with you, trending, used). Thumbnails come from `Insight::item` with `browse::thumbnails`. Unit tests in `insights.rs`; `kelma-cli drive for-you`.
  - **Evidence (UI, 2026-11-19).** UI test `home_shows_for_you_cards_that_open_their_file` (cards, reasons, click sends `card-open`, hidden while filtering and off Home), unit test `cards_show_on_home_only_and_not_while_filtering`. Thumbnails: `browse::thumbnails` per card, then the image is decoded (not exercised offline: the demo cards show the type badge). Live, read-only, re-run for this change: `kelma-cli drive for-you` answered (12 cards, counts only).
- [x] **O1.3 Shared.** Impact 3, effort 3.
  - **What.** "Shared with you" and "Shared by you".
  - **How.** O0, or Search, plus the `shared` facet and `/permissions`.
  - **Status.** "Shared with you" is built (the Shared section, with the type pills and the filter box; a shared folder opens as its own root). "Shared by you" is its own navigation entry under it, with the same pills and filter, newest first; the date column says "Shared" and no person is named (the sharer is you). Shares of things that are not drive items (a page, a mail attachment) are left out, because nothing can be done with them.
  - **Evidence (data layer).** `onedrive::shared_by_me` (shared insights whose `sharedBy` is the user, newest first; `Insight::to_drive_item` gives list rows); run live: 67 items. Unit tests in `insights.rs`; `kelma-cli drive shared-by-me`.
  - **Evidence (UI, 2026-11-19).** UI test `shared_by_you_lists_what_the_user_shared`, unit test `shared_by_you_keeps_drive_items_and_drops_the_sharer_name`. Live, read-only, re-run for this change: `kelma-cli drive shared-by-me` answered (67 items, counts only).
- [x] **O1.4 List features.** Impact 3, effort 2.
  - **What.**
    - List and grid views.
    - Sort by name, modified, modified by or size; group by.
    - Column picker.
    - Thumbnails (`/thumbnails`).
    - Activity text per file.
  - **Status.** Built: list and grid views, sort by name, modified, modified by or size (a column header or the Sort menu; the same key twice flips the direction), Group by (None, Name, Modified, Modified by, Size; headers rows in the list), a column picker, thumbnails in the grid (`$expand=thumbnails`, the first 48 items, fetched in the background) and the Activity column ("Ada modified this"). While grouped, folders sort with the files so no group repeats. Groups use local days, so a group header and the date column agree near midnight (test `grouping_uses_local_days_not_utc_days`; fixed in 0c2141a).
- [x] **O1.5 Search** across OneDrive and SharePoint. Impact 4, effort 2. Uses `POST /search/query` with `driveItem` and `listItem`.
  - **Status.** The search box in the title row runs `browse::search` and shows the hits as a "Search: <query>" section; an empty search returns to Home. Run live (50 hits, counts only).

## Phase O2: act

- [x] **O2.1 Create or upload.** Impact 4, effort 3.
  - **What.** The menu lists:
    - Folder
    - Files upload and Folder upload (also drag and drop)
    - Word, Excel and PowerPoint documents (from bundled blank templates)
    - OneNote notebook
    - Forms survey and Visio drawing (open in the browser)
    - Markdown document, Text document
    - Link
  - **How.**
    - Folder: `POST …/children`.
    - Upload: the upload sessions already in `files.rs`; recursive for folders.
    - Office documents: `PUT` of a minimal OOXML blob.
    - OneNote: the `onenote.com` token.
  - **Status.** Built: the New menu has Folder (asks for a name), Word, Excel and PowerPoint documents (the bundled blank files), Markdown and Text; Upload has Files and Folder (the `rfd` picker) and a file or folder dropped on the window uploads on the OneDrive view (X11, Windows and macOS: winit has no drop events on Wayland). Run live inside a throwaway folder: a folder, Word, Excel, Markdown and Text documents (PowerPoint was not created live), a file and a folder upload. The New menu now also has Visio drawing (a blank VSDX), OneNote notebook (asks for a name, creates it in your notebooks and opens it in the browser), Forms survey (opens Microsoft Forms in the browser: there is no API for this client) and Link (asks for a name and a web address, refuses one that does not start with https:// or http://, and makes a `.url` file in the open folder).
  - **Evidence (data layer).** `DocKind::Visio` (blank VSDX, so `create_office_doc` makes one), and `onedrive::create`/`plan` with `NewItem::Link` (a `.url` file) and `NewItem::Notebook` (`POST /me/onenote/notebooks`); Forms opens `FORMS_NEW_URL` in the browser. Offline only, by instruction: unit tests on the requests and the package, and `kelma-cli drive new <kind> … --dry-run` prints the request. `GET /me/onenote/notebooks` answered 200, so the Graph bearer reaches OneNote. `drive selftest` now also creates a Visio drawing and a Link in its throwaway folder.
  - **Evidence (UI, 2026-11-19).** UI tests `the_new_menu_offers_visio_notebook_forms_and_link` and `a_link_without_a_web_address_is_refused_before_anything_is_sent` (commands, prompts, the two-field link dialog, demo mode writes nothing) and the command parsing test in `cmd.rs`. Live, inside a throwaway folder only: `kelma-cli drive selftest` created Word, Excel, PowerPoint, Markdown, Text, Visio (2318 bytes) and Link (46 bytes) documents, then deleted the folder and purged it from the recycle bin (`0 of ours left`, `selftest passed and cleaned up`); My files and the bin were listed afterwards and hold no `kelma-test-` item. Not run live: creating a OneNote notebook, because OneNote keeps notebooks in your own notebooks, outside any throwaway folder, so it cannot be cleaned up by deleting a folder. DEFERRED: needs test tenant (OneNote notebook creation; only its request is unit tested and its dry run printed).
- [x] **O2.2 Selection toolbar and row actions.** Impact 4, effort 3.
  - **What.**
    - Open ▾ (in kelma's viewer, in the browser, or in a desktop app through xdg-open), Open location, Share, Copy link, Add to favorites (★ on hover), Add shortcut ▾, Rename, Move to, Copy to, Download, Delete.
    - Version history and Details from "…".
  - **How.**
    - `PATCH` (rename, move), `/copy` (asynchronous, poll the monitor URL), `DELETE`, `/content` (download; zip folders on the client), `/follow` and `/unfollow`.
    - Shortcut: `remoteItem`.
    - Versions: `/versions` and `restoreVersion`.
  - **Status.** Built: the selection toolbar (Open ▾ with kelma, browser and desktop app through `xdg-open`, Share, Copy link, Favorite, Rename, Move to, Copy to, Download, Delete, and More with Add shortcut, Open location, Version history and Details), the same actions in a row's "…" menu, and a ★ on hover. Move to and Copy to use the reusable file picker. Download asks for a file (a zip for a folder). Copy link copies the item's own web address, which works for everyone who already has access; a new sharing link is made in the Share dialog. Add shortcut is refused by OneDrive (docs/protocol.md), so the app says so in a sentence and changes nothing. Run live in the throwaway folder: rename, favorite and unfavorite, copy, move, delete, preview of a text file, details and version history (one version on a new file). Not run live: Download (a save dialog), Open in the desktop app and Open location in the browser.
- [ ] **O2.3 Share dialog.** Impact 4, effort 3.
  - **What.**
    - Share with people: a picker plus a message, with Can edit / Can view.
    - Link settings: anyone, people in the org, specific people; expiry; block download.
    - Manage access lists who has access and lets you change or remove it.
  - **How.** `/invite`, `/createLink`, and `/permissions` (GET, PATCH, DELETE).
  - **Status.** Built and UI tested in demo mode: a people picker (`search_people`), Can edit or Can view, an optional message, link settings (Anyone, People in your organization, Specific people; View or Edit; Block download; expiry none, 7 or 30 days) and Manage access (change or remove access, the owner and inherited entries are read-only). Not run live, by instruction: no invitation and no link was created, so the request bodies are only checked by the crate's unit tests. DEFERRED: needs test tenant (the Done-when is a real share: an invitation emails a real person and a link opens the file to others; only the dry run and the crate's unit tests ran).
- [x] **O2.4 Recycle bin.** Impact 2, effort 2.
  - **What.** List deleted items with their original location and deletion date; Restore; Delete forever.
  - **How.** SharePoint REST on the `-my.sharepoint.com` token: `/_api/web/RecycleBin`, `/_api/web/RecycleBin('{id}')/restore` and `/deleteObject`.
  - **Status.** Built: the list shows the deletion date, who deleted it and the original location; Restore and Delete forever (after a confirmation). Run live: deleted two files, restored one, deleted one for good, then the throwaway folder, and purged everything it left in the bin.
- [ ] **O2.5 Preview.** Impact 3, effort 3.
  - **What.**
    - Images, PDF, text, Markdown and code in a native viewer.
    - Office files through the preview URL (`POST …/preview`) in the web host, or "Open in browser".
    - Video and audio in the media player.
  - **Status.** Built: images (png, jpeg, gif, webp), text, code and Markdown (rendered) open in a viewer inside kelma (files up to 15 MB; text is cut at 200,000 characters). Word, Excel, PowerPoint, PDF and other images open the preview URL (`POST …/preview`) in the browser, and video and audio go to the system player through `xdg-open` (up to 100 MB, else the web address). Run live: a text file. BLOCKED: a native PDF view needs a PDF rendering library (pdfium or mupdf), which the workspace does not have and which cannot be added without a native dependency; and an embedded Office or PDF view needs the web host (`plans/webhost.md`, a separate feature). Until one of them exists, PDF and Office open through the preview URL in the browser, which is the "Open in browser" fallback the What allows.

## Phase O3: around files

- [x] **O3.1 Meeting recordings.** The `Recordings` folder in My files and the channel libraries, as one "Meetings" view. Shared with `plans/meeting-recaps.md`.
  - **Status.** The Meetings section lists the `Recordings` folder of My files at once (run live: 3 items, counts only), then adds every channel's `Recordings` folder from `onedrive::meeting_recordings` in the background and swaps the list when it arrives (a late answer for a section the user already left is dropped). The Location column is always on and shows the place: the folder for My files, `<team> › <channel>` for a channel. Newest first.
  - **Evidence (data layer).** `onedrive::meeting_recordings` lists My files and every channel's `Recordings` folder (`filesFolder`), each with its place; run live: 4 recordings, 3 in My files and 1 in a channel. Unit tests in `around.rs`; `kelma-cli drive meetings`.
  - **Evidence (UI, 2026-11-19).** UI test `meetings_adds_the_channel_recordings_in_the_background` (two recordings first, three after the background step, the channel one showing its place), unit tests `recordings_show_their_place_and_my_files_keeps_its_path` and `meetings_and_the_bin_always_show_the_location_column`. Live, read-only, re-run for this change: `kelma-cli drive meetings` answered (4 recordings, counts only).
- [x] **O3.2 Sync.** "Sync" explains that kelma does not sync, and links to the Linux `onedrive` client or `rclone` with a ready config. The Sync entry of the navigation opens `onedrive::sync_help()` in a dialog.
- [ ] **O3.3 Files in chats and channels.** Reuse this module for the Shared tab (chat plan 7.5), Attach cloud files (8.7) and channel uploads (8.6).
  - **Status.** The reusable part is provided, not yet used by those features: the Slint component `DriveFilePicker` (`ui/onedrive.slint`, exported next to `OneDriveView`) and the command `pick-begin <purpose>`, which opens it over the `DriveStore` picker properties (folders only for `move` and `copy`, files for any other purpose). The answer arrives in `Engine::drive_picked(purpose, folder, file)` in `backend/drive.rs`; a feature adds its purpose there. Wired now: "Attach cloud files" in the composer (a cloud button next to the paperclip). It opens the picker over the chat (`DriveStore.command("pick-begin", "attach")`; the picker overlay lives in `app.slint` and shows only while the OneDrive view is not on screen), starts the libraries load if the OneDrive view never did, and on "Select" `drive_picked("attach")` downloads the file (100 MB limit) and adds it to the open chat's chips like a file picked from disk, so sending goes through the normal file flow. One file per pick, from My files or a team library (no Recent or Shared with me tab yet). UI test `attach_cloud_files_in_the_composer_picks_a_onedrive_file_into_the_chat`. Not wired: the Shared tab (chat 7.5: the tab is only a label in the chat header today, another agent is building it) and channel uploads (chat 8.6). BLOCKED: chat 7.5 Shared tab does not exist yet, and channel uploads (8.6) are not built; both can use `DriveFilePicker` and `Engine::drive_picked` as the attach case does. DEFERRED: needs test tenant (sending an attached cloud file to a real chat; the Done-when of chat 8.7 is a send to 48:notes).

## Order

O1.1, O2.2, O2.1 and O2.3 (everyday file work), then O0 with O1.2 and O1.3, then the rest.
