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

- [ ] **O0.1** Capture the Teams OneDrive app's calls for Home ("For you", Recent), Shared (with you, by you), the recycle bin and Libraries. Document them in `docs/protocol.md`.

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
- [ ] **O1.2 Home.** Impact 3, effort 3.
  - **What.**
    - "For you" cards ("You frequently open", "X shared this in a Teams chat", with a thumbnail and Open).
    - Recent, with type pills All, Word, Excel, PowerPoint, PDF and More, a "filter by name or person" box, and the columns Name, Opened, Owner and Activity.
  - **How.** The O0 endpoints, with Graph Search (`POST /search/query`, driveItem) as a fallback.
- [ ] **O1.3 Shared.** Impact 3, effort 3.
  - **What.** "Shared with you" and "Shared by you".
  - **How.** O0, or Search, plus the `shared` facet and `/permissions`.
- [ ] **O1.4 List features.** Impact 3, effort 2.
  - **What.**
    - List and grid views.
    - Sort by name, modified, modified by or size; group by.
    - Column picker.
    - Thumbnails (`/thumbnails`).
    - Activity text per file.
- [ ] **O1.5 Search** across OneDrive and SharePoint. Impact 4, effort 2. Uses `POST /search/query` with `driveItem` and `listItem`.

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
- [ ] **O2.2 Selection toolbar and row actions.** Impact 4, effort 3.
  - **What.**
    - Open ▾ (in kelma's viewer, in the browser, or in a desktop app through xdg-open), Open location, Share, Copy link, Add to favorites (★ on hover), Add shortcut ▾, Rename, Move to, Copy to, Download, Delete.
    - Version history and Details from "…".
  - **How.**
    - `PATCH` (rename, move), `/copy` (asynchronous, poll the monitor URL), `DELETE`, `/content` (download; zip folders on the client), `/follow` and `/unfollow`.
    - Shortcut: `remoteItem`.
    - Versions: `/versions` and `restoreVersion`.
- [ ] **O2.3 Share dialog.** Impact 4, effort 3.
  - **What.**
    - Share with people: a picker plus a message, with Can edit / Can view.
    - Link settings: anyone, people in the org, specific people; expiry; block download.
    - Manage access lists who has access and lets you change or remove it.
  - **How.** `/invite`, `/createLink`, and `/permissions` (GET, PATCH, DELETE).
- [ ] **O2.4 Recycle bin.** Impact 2, effort 2.
  - **What.** List deleted items with their original location and deletion date; Restore; Delete forever.
  - **How.** SharePoint REST on the `-my.sharepoint.com` token: `/_api/web/RecycleBin`, `/_api/web/RecycleBin('{id}')/restore` and `/deleteObject`.
- [ ] **O2.5 Preview.** Impact 3, effort 3.
  - **What.**
    - Images, PDF, text, Markdown and code in a native viewer.
    - Office files through the preview URL (`POST …/preview`) in the web host, or "Open in browser".
    - Video and audio in the media player.

## Phase O3: around files

- [ ] **O3.1 Meeting recordings.** The `Recordings` folder in My files and the channel libraries, as one "Meetings" view. Shared with `plans/meeting-recaps.md`.
- [ ] **O3.2 Sync.** "Sync" explains that kelma does not sync, and links to the Linux `onedrive` client or `rclone` with a ready config.
- [ ] **O3.3 Files in chats and channels.** Reuse this module for the Shared tab (chat plan 7.5), Attach cloud files (8.7) and channel uploads (8.6).

## Order

O1.1, O2.2, O2.1 and O2.3 (everyday file work), then O0 with O1.2 and O1.3, then the rest.
