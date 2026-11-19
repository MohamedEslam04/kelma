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

- [ ] **O1.1 Navigation.** Impact 4, effort 3.
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
  - **Status.** Built in `crates/kelma/ui/onedrive.slint`, `crates/kelma/src/backend/drive.rs` and `drive/state.rs` (rail item "OneDrive", after Calendar): Home, My files, Shared, Favorites, Libraries, Recycle bin, Meetings, Quick access (the team libraries) and breadcrumbs, UI tested through the real engine in demo mode. Run live: every section answered (counts only), My files folders opened inside a throwaway folder. Open: "Browse files by" People and Media (Meetings is there).
  - **Evidence (data layer).** People: `onedrive::people` (shared insights grouped by sharer, `sharedWithMe` fallback), run live: 16 people. Media: `onedrive::media` (Search over My files, photo and video extensions), run live: 50 hits on the first page, all in My files. Unit tests in `insights.rs` and `browse.rs`; `kelma-cli drive people|media`. The UI still has to show them.
- [ ] **O1.2 Home.** Impact 3, effort 3.
  - **What.**
    - "For you" cards ("You frequently open", "X shared this in a Teams chat", with a thumbnail and Open).
    - Recent, with type pills All, Word, Excel, PowerPoint, PDF and More, a "filter by name or person" box, and the columns Name, Opened, Owner and Activity.
  - **How.** The O0 endpoints, with Graph Search (`POST /search/query`, driveItem) as a fallback.
  - **Status.** Recent is built: the pills and the filter box (name or person) narrow the list, and the column picker covers Modified, Modified by, Size, Location and Activity (the columns say Modified, not Opened or Owner). Open: the "For you" cards.
  - **Evidence (data layer).** `onedrive::for_you` gives `ForYouCard`s (insight plus reason) from `/me/insights/used`, `trending` and `shared`; run live: 12 cards (shared in a Teams chat, shared with you, trending, used). Thumbnails come from `Insight::item` with `browse::thumbnails`. Unit tests in `insights.rs`; `kelma-cli drive for-you`. The UI still has to show them.
- [ ] **O1.3 Shared.** Impact 3, effort 3.
  - **What.** "Shared with you" and "Shared by you".
  - **How.** O0, or Search, plus the `shared` facet and `/permissions`.
  - **Status.** "Shared with you" is built (the Shared section, with the type pills and the filter box; a shared folder opens as its own root). Open: "Shared by you".
  - **Evidence (data layer).** `onedrive::shared_by_me` (shared insights whose `sharedBy` is the user, newest first; `Insight::to_drive_item` gives list rows); run live: 67 items. Unit tests in `insights.rs`; `kelma-cli drive shared-by-me`. The UI still has to show it.
- [x] **O1.4 List features.** Impact 3, effort 2.
  - **What.**
    - List and grid views.
    - Sort by name, modified, modified by or size; group by.
    - Column picker.
    - Thumbnails (`/thumbnails`).
    - Activity text per file.
  - **Status.** Built: list and grid views, sort by name, modified, modified by or size (a column header or the Sort menu; the same key twice flips the direction), Group by (None, Name, Modified, Modified by, Size; headers rows in the list), a column picker, thumbnails in the grid (`$expand=thumbnails`, the first 48 items, fetched in the background) and the Activity column ("Ada modified this"). While grouped, folders sort with the files so no group repeats. Group dates are UTC days, so near local midnight a group header and the date column can disagree.
- [x] **O1.5 Search** across OneDrive and SharePoint. Impact 4, effort 2. Uses `POST /search/query` with `driveItem` and `listItem`.
  - **Status.** The search box in the title row runs `browse::search` and shows the hits as a "Search: <query>" section; an empty search returns to Home. Run live (50 hits, counts only).

## Phase O2: act

- [ ] **O2.1 Create or upload.** Impact 4, effort 3.
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
  - **Status.** Built: the New menu has Folder (asks for a name), Word, Excel and PowerPoint documents (the bundled blank files), Markdown and Text; Upload has Files and Folder (the `rfd` picker) and a file or folder dropped on the window uploads on the OneDrive view (X11, Windows and macOS: winit has no drop events on Wayland). Run live inside a throwaway folder: a folder, Word, Excel, Markdown and Text documents (PowerPoint was not created live), a file and a folder upload. Open: OneNote notebook, Forms survey, Visio drawing and Link.
  - **Evidence (data layer).** `DocKind::Visio` (blank VSDX, so `create_office_doc` makes one), and `onedrive::create`/`plan` with `NewItem::Link` (a `.url` file) and `NewItem::Notebook` (`POST /me/onenote/notebooks`); Forms opens `FORMS_NEW_URL` in the browser. Offline only, by instruction: unit tests on the requests and the package, and `kelma-cli drive new <kind> … --dry-run` prints the request. `GET /me/onenote/notebooks` answered 200, so the Graph bearer reaches OneNote. `drive selftest` now also creates a Visio drawing and a Link in its throwaway folder (not run). The UI still has to add the menu entries.
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
  - **Status.** Built and UI tested in demo mode: a people picker (`search_people`), Can edit or Can view, an optional message, link settings (Anyone, People in your organization, Specific people; View or Edit; Block download; expiry none, 7 or 30 days) and Manage access (change or remove access, the owner and inherited entries are read-only). Not run live, by instruction: no invitation and no link was created, so the request bodies are only checked by the crate's unit tests.
- [x] **O2.4 Recycle bin.** Impact 2, effort 2.
  - **What.** List deleted items with their original location and deletion date; Restore; Delete forever.
  - **How.** SharePoint REST on the `-my.sharepoint.com` token: `/_api/web/RecycleBin`, `/_api/web/RecycleBin('{id}')/restore` and `/deleteObject`.
  - **Status.** Built: the list shows the deletion date, who deleted it and the original location; Restore and Delete forever (after a confirmation). Run live: deleted two files, restored one, deleted one for good, then the throwaway folder, and purged everything it left in the bin.
- [ ] **O2.5 Preview.** Impact 3, effort 3.
  - **What.**
    - Images, PDF, text, Markdown and code in a native viewer.
    - Office files through the preview URL (`POST …/preview`) in the web host, or "Open in browser".
    - Video and audio in the media player.
  - **Status.** Built: images (png, jpeg, gif, webp), text, code and Markdown (rendered) open in a viewer inside kelma (files up to 15 MB; text is cut at 200,000 characters). Word, Excel, PowerPoint, PDF and other images open the preview URL (`POST …/preview`) in the browser, and video and audio go to the system player through `xdg-open` (up to 100 MB, else the web address). Open: PDF and Office in a native or embedded view. Run live: a text file.

## Phase O3: around files

- [ ] **O3.1 Meeting recordings.** The `Recordings` folder in My files and the channel libraries, as one "Meetings" view. Shared with `plans/meeting-recaps.md`.
  - **Status.** The Meetings section lists the `Recordings` folder of My files (run live: 3 items, counts only). Open: the channel libraries.
  - **Evidence (data layer).** `onedrive::meeting_recordings` lists My files and every channel's `Recordings` folder (`filesFolder`), each with its place; run live: 4 recordings, 3 in My files and 1 in a channel. Unit tests in `around.rs`; `kelma-cli drive meetings`. The UI still has to switch the Meetings section to it.
- [x] **O3.2 Sync.** "Sync" explains that kelma does not sync, and links to the Linux `onedrive` client or `rclone` with a ready config. The Sync entry of the navigation opens `onedrive::sync_help()` in a dialog.
- [ ] **O3.3 Files in chats and channels.** Reuse this module for the Shared tab (chat plan 7.5), Attach cloud files (8.7) and channel uploads (8.6).
  - **Status.** The reusable part is provided, not yet used by those features: the Slint component `DriveFilePicker` (`ui/onedrive.slint`, exported next to `OneDriveView`) and the command `pick-begin <purpose>`, which opens it over the `DriveStore` picker properties (folders only for `move` and `copy`, files for any other purpose). The answer arrives in `Engine::drive_picked(purpose, folder, file)` in `backend/drive.rs`; a feature adds its purpose there. The Shared tab, Attach cloud files and channel uploads still have to be wired.

## Order

O1.1, O2.2, O2.1 and O2.3 (everyday file work), then O0 with O1.2 and O1.3, then the rest.
