# Hörwerk

*The audiobook workshop for your NAS.*

**English** · [Deutsch](README.de.md)

A self-hosted audiobook pipeline for your NAS (Unraid or any Docker host): drop audiobooks into an inbox folder;
they become **one M4B with chapters**, are matched on Audible, **tagged** and **filed** into a clean library for
Audiobookshelf, Plex & co. Older MP3 books already in the library are converted the same way – tracks and chapters
get sensible names along the way.

Downloads go through **SABnzbd**, notifications through **ntfy** – you choose which events you hear about.
The interface is available in **English and German**.

> Hörwerk is a tool for organising audio files you are entitled to use – your own purchases, your own
> recordings or public-domain works (e.g. from [LibriVox](https://librivox.org)). It contains no media,
> does not remove copy protection and is not affiliated with Audible, Amazon, Apple or any other service named here.

> Formerly *Better Renamer*: up to version 4 the app also had a general batch renamer; since version 5 it is called Hörwerk and does audiobooks only. An existing database (`better-renamer.db`) is taken over automatically.

---

## Contents

1. [Features](#features)
2. [Quick start](#quick-start)
3. [Audiobooks](#audiobooks)
4. [Downloads with SABnzbd](#downloads-with-sabnzbd)
5. [Discover and requests](#discover)
6. [Notifications (ntfy)](#notifications-ntfy)
7. [Log](#log)
8. [Accounts and access from outside](#accounts-and-access-from-outside)
9. [Configuration](#configuration)
10. [Security](#security)
11. [Development](#development)

---

## Features

| Area | What you get |
|---|---|
| Audiobook inbox | Waits until a folder is fully copied, merges MP3/M4A/FLAC/OGG/Opus/WMA (also `CD1`/`CD2` subfolders) into one M4B with sample-exact, cleanly named chapters |
| Matching | Audible search in 10 stores, rated by title, author **and runtime** (unabridged ≠ abridged ≠ radio play); uncertain matches wait for you |
| Tags & files | Title, author, narrator, series + volume, genre, publisher, date, description, ASIN, full-size cover; `metadata.json` for Audiobookshelf |
| Library | Browse and search every book, folder template, reuses existing author/series folders, detects duplicates, converts old MP3 books to M4B and removes the MP3s safely |
| Downloads | Sends NZBs to SABnzbd, watches the category, the inbox takes over when the download is done; optional Newznab search |
| Discover | Suggestions matching your library, by genre and from book reviews (Deutschlandfunk, Deutschlandfunk Kultur, Perlentaucher) – well-rated, unabridged, nothing you already have; **requests** like Overseerr: the family asks, Hörwerk gets it (with approval, quota, interactive search) |
| Media servers | Audiobookshelf and Plex rescan their library after filing; Plex can empty its trash so replaced MP3s vanish |
| Notifications | ntfy push messages, eleven events, each one on or off |
| Log | Everything that happened, filterable and downloadable, right in the app |
| Interface | English / German, light / dark theme, live updates without reloading |
| Security | Own minimal ffmpeg 9 in a Landlock sandbox, CSRF protection, secrets never reach the browser ([SECURITY.md](SECURITY.md)) |

---

## Quick start

### Unraid

1. **Docker → Add Container**, enter as *Template*:
   `https://raw.githubusercontent.com/Larosen/rename_merge/main/unraid/hoerwerk.xml`
2. Set the paths:
   * **Hörbuch-Eingang** (`/inbox`) – where new audiobooks arrive
   * **Hörbuch-Bibliothek** (`/audiobooks`) – the finished library (add it to Audiobookshelf/Plex)
3. Pick **Language / Sprache** (`APP_LANGUAGE`: `en` or `de`). PUID `99`, PGID `100`, UMASK `000` are already right for Unraid.
4. Open `http://<unraid-ip>:8080`.

Icon: the template loads it from GitHub. If your copy of the repository is private, Unraid cannot fetch it –
set **Icon URL** to `http://<unraid-ip>:8080/static/icon.png` instead; the container serves its own logo
(the only file it answers without a password).

Image: `ghcr.io/larosen/hoerwerk:latest` (amd64 + arm64, rebuilt weekly for security updates).

### Any other Docker host

```bash
# adjust the paths, PUID/PGID (see `id`) and APP_LANGUAGE in docker-compose.yml
docker compose up -d
```

---

## Audiobooks

### From inbox to library

```
/inbox                                              /audiobooks
├── Doyle - The Hound of the Baskervilles/    ──►   ├── Arthur Conan Doyle/
│   ├── CD1/01.mp3 …                                │   └── Sherlock Holmes/
│   └── CD2/01.mp3 …                                │       └── 03 - The Hound of the Baskervilles/
├── Storm - Der Schimmelreiter.m4b            ──►   │           ├── The Hound of the Baskervilles.m4b
├── _fertig/   processed originals                  │           ├── cover.jpg · desc.txt · reader.txt
└── _failed/   with FEHLER.txt                      │           └── metadata.json
                                                    └── Theodor Storm/Der Schimmelreiter/…
```

1. **Wait.** Every folder or file in the inbox is one book. It is processed once nothing has changed for a while
   (`SETTLE_SECONDS`, default 2 min) and no `.part`/`.crdownload` files are left. A folder with several book folders
   (`Series/Volume 1`, `Series/Volume 2`) becomes several books.
2. **Create the M4B.** Each file is encoded to AAC in parallel and then joined without re-encoding – one chapter per
   file (or the chapters a file already has). AAC sources are copied, a single M4B is taken over as is.
   Own chapters: put a `chapters.txt` (`00:01:02.500 Title`) into the book folder.
3. **Match on Audible.** Searched by folder name or album/artist tags in the chosen store (`AUDIBLE_REGION`);
   an ASIN in the name (`… [B0ABCDE123]`) is used directly. Only clear matches are filed automatically.
   Everything else waits under **To review**, where you can
   * accept a match or **edit** it first,
   * search again, also in another Audible store,
   * paste a **link from Audible, BookBeat or Thalia** – the details are fetched automatically,
   * or enter everything yourself.

   Books that exist nowhere: put a `metadata.yml` (beets-audible format) into the folder.
4. **Tag & file.** Chapter names come from Audible when your own are only “Track 01” and the runtime fits.
   The library path follows `FOLDER_TEMPLATE`, default `{author}/[{series}/][{series_position:2} - ]{title}` –
   `[ … ]` disappears when empty, `:2` turns volume 3 into `03`. Existing folders are reused even when spelled
   differently (`E. T. A. Hoffmann` = `E.T.A. Hoffmann`, `Doyle, Arthur Conan` = `Arthur Conan Doyle`,
   `Mueller` = `Müller`). Originals move to `_fertig` (or are deleted); failures go to `_failed` with an
   explanation and a **Retry** button.

Filed the wrong book? **Correct** takes it back out of the library into the review list.

### Upload (e.g. from CD)

Audiobooks you own – say ripped from CD – can be uploaded right in the browser: *Audiobooks → Inbox → **Upload***.
Drag in or choose one folder per audiobook, ideally "Author – Title", sub-folders `CD1`, `CD2` … welcome; single files
work too (then enter the name). Hörwerk takes audio files, covers, cue sheets and `chapters.txt`/`metadata.yml`;
everything else is skipped. The **content** is checked too: an `.exe`, a PDF, an archive or a script renamed to `.mp3`
is refused.

The files first go into a hidden folder and are moved into the inbox in one step after the last one – then the usual
way starts without waiting: folder name and tags → Audible → M4B with chapters → library. Up to 4 GB per file;
abandoned uploads are cleaned up after a day. Admins only.

### Library view

**Audiobooks → Library** lists every book in the library with a search over title, author, series and folder. Each row
shows how the book was read – the author, further names from tags and folders, the folder and the format – which is
also what the duplicate check and the search use. Covers come from `cover.jpg`/`folder.jpg` in the book folder or from
the picture inside the audio file; books without one get a generated tile.

**Clean up** (in the *Library* box) deletes `desc.txt` and `reader.txt` from every book folder that has a
`metadata.json` – after showing how many there are. Everything in them is also in `metadata.json` and the M4B tags;
Plex never reads them, Audiobookshelf only when `metadata.json` is missing. Audiobooks, `cover.jpg` and folders without
`metadata.json` stay untouched, symlinks are not followed. New books no longer get the two files.

### Downloads with several volumes

A download like `Author-Series 01 - 07-…` is a **pack**: every volume folder becomes its own book – merged, matched
and tagged separately, with author and volume number from the release name. Folders such as `01 - Title`,
`02 - Title` are volumes; `CD1`/`CD2`, `Disc 1`, `Teil 1` or plain `01`/`02` stay parts of one book.

* In **Search**, a pack shows its volumes and which of them you already own. **Download** asks which volumes you
  want – owned ones start unticked. Only the chosen volumes are converted; the others are put aside unconverted.
* Packs that did not come through the search: volumes already in the library are skipped automatically – by series
  and volume number, or at the latest when Audible recognises the book, always before converting.

### Avoiding duplicates

The library is scanned at start and every few hours (`LIBRARY_RESCAN_HOURS`) – including older MP3 folders, recognised
by ASIN, tags or the folder structure. If a new book is already there (same ASIN, same author + title or same
series + volume), you decide: **Discard**, **Replace old** (the old version moves to `_ersetzt` in the inbox) or
**File additionally**. `DUPLICATES=skip` or `keep` decides automatically.

The check is tolerant on purpose: the narrator in the artist tag, "Author - Title" album tags, "Author - Title" folder
names and release names without the author (for titles of two words or more) all still match. In **Search**, a
search term that matches a book you own shows “Already in your library” above the results.

### Turning old MP3 books into M4B

**Audiobooks → Not an M4B** lists every library book that still consists of single files. Select books, click
**To M4B**, optionally adjust chapter names first (**Chapters & names …**, also renames the source files to
`01 - Chapter.mp3` if you like).

**Old files afterwards** is chosen right next to the button: *move to `_ersetzt` in the inbox* (default) or *delete*.
Either way nothing is touched before the new M4B is in place: the old folder is set aside (renamed, hidden) and only
deleted or moved once the new book is filed; if filing fails, it comes back unchanged.

Chapter names under *automatic* are cleaned for the whole book: text every track repeats (the book title,
“(2012)”, “Title - ”) is dropped, and tracks that are only numbered become “Chapter 1, 2, 3 …”. Real chapter names stay.

### More metadata sources

If Audible is unreachable and the ASIN is known, the same data comes from **Audnexus**. If Audible finds nothing,
**Apple Books** and **Open Library** are asked. Those matches have no runtime or narrator, so they are never filed
automatically. Switch off with `METADATA_FALLBACK=false`.

### Audiobookshelf

* Next to every book there is a `metadata.json` in Audiobookshelf's format (title, authors, narrators, series,
  genres, description, ASIN, chapters) – `WRITE_ABS_METADATA=false` turns it off.
* Optional, under **Search → Set up**: address and API token of Audiobookshelf. After new books are filed, the
  library is rescanned there – useful on network shares where its folder watcher misses changes.

### Plex

Optional, under **Search → Set up**: Plex address (e.g. `http://192.168.1.10:32400`), token and the audiobook library
(chosen automatically when testing if it is clear which one). After filing or converting, Plex rescans that library.

**Converted books showing up twice** (M4B and the old MP3s): Plex keeps entries of removed files as “unavailable”
until its trash is emptied. Tick **Then empty the library's trash** – after the scan Hörwerk empties the trash
of this one library, and the old MP3 entries disappear.

Token: in Plex Web open any item → **…** → **Get Info** → **View XML**; the value after `X-Plex-Token=` in the address.

---

## Downloads with SABnzbd

You only need SABnzbd; searching is optional.

1. In SABnzbd create a category, e.g. `audiobooks`, whose folder is the one mounted as **audiobook inbox** (`/inbox`).
2. In Hörwerk: **Search → Set up** → SABnzbd address, API key (SABnzbd → Config → General) and category.
3. Everything SABnzbd finishes in that category flows through M4B → Audible → library automatically. The
   **Downloads** list shows progress and history of the category.

**Search** (with a Newznab indexer): you search for an **audiobook**, not a release – matches come from Audible and
BookBeat. Only once you pick one Hörwerk searches the indexers; you can change the term for that if nothing comes up
(e.g. title only or surname only). Releases are merged, ranked (preferred language, unabridged before abridged, M4B
before MP3) and compared with your library; umlauts in every spelling.
**Categories:** besides the ones entered (default `3030`), Hörwerk automatically searches each indexer's **whole audio
area** – old free readings (Goethe & co.) often sit under "Audio > Lossless" or "MP3" – plus its own audiobook
categories (from its category list, e.g. "Hörbuch DE", "Hörspiele"). Music albums that come along are sorted out by the
release verdict (author and title). If nothing turns up there, it searches all categories and keeps only results from
the audio area. *Test* on an indexer shows which categories were found. The ASIN (or title and author) goes along with
the download – so the inbox recognises the book safely, whatever the release is called.
For books that aren't in any shop: **search the indexers directly**.

Finished SABnzbd downloads from the search are taken over by the inbox after a few seconds; until then they show up
as "In the inbox – starts in about … s" at the top of the audiobook list and in the log.

Tip: in SABnzbd set *Config → Switches → Unwanted extensions* (`exe, bat, cmd, scr, lnk, js, vbs, ps1, msi, com`)
to **Abort**.

---

## Discover

The **Discover** tab suggests new audiobooks:

* **For you** – titles Audible listeners connect with the (up to 30 newest) books in your library. The more of your
  books point to the same title, the higher it ranks; "Goes with …" names them.
* **By genre** – all genres and subcategories as in the Audible shop (e.g. *Krimis & Thriller → Thriller*): best
  sellers, best rated and new releases. Small areas are filled up with titles rated just as well (4.0+) by fewer
  listeners, so every genre shows suggestions; subcategories without a single German audiobook (e.g. *Cricket*) are left
  out. All lists are prepared in the background once a day. The chosen genre is remembered per browser.
* **From the critics** – reviewed books that exist as an audiobook on Audible (same title, same author), with a link to
  the review. Sources: the RSS feeds "Buchkritik" and "Lesart" (Deutschlandfunk Kultur) and "Büchermarkt"
  (Deutschlandfunk), Perlentaucher's lists
  [Hörbücher – Literatur aktuell](https://www.perlentaucher.de/buchKSL/hoerbuecher-literatur-aktuell.html) and
  [Sachbuch aktuell](https://www.perlentaucher.de/buchKSL/hoerbuecher-sachbuch-aktuell.html) (with the number of press
  reviews) and the book round-ups from Perlentaucher's feed. Only author and title are taken; reviews are remembered for
  180 days, the newest ~40 audiobooks are shown.

**Search** on a card jumps to the *Search* tab with the book already picked (title, author and ASIN help with tagging later).

**Quality rules** (*Settings → Discover*): minimum rating (default 4.3 stars) and number of ratings (50), unabridged only,
German only, no erotica. **Always excluded** is what you already have: books in the library
(even spelled differently), downloads of the last 30 days and books being processed right now. Books from the critics skip the rating rule – they are often new.

**To be honest:** no software can reliably check whether a non-fiction book is accurate – ratings only measure
popularity. So *For you* leaves non-fiction out by default (switch in the settings); in *By genre* it only appears when
you pick a non-fiction genre yourself, with a note that stars are no fact check. BookBeat is not connected – it
has no open API.

Lists are put together in the background, refreshed at most once a day (or with **Refresh**) and cached in the database.
Outbound connections: only the Audible API of the chosen store, `www.deutschlandfunkkultur.de`, `www.deutschlandfunk.de` and
`www.perlentaucher.de` (HTTPS).

### Requests (like Overseerr/Jellyseerr)

Family members ask for a **book**, not a release: click **Request** in *Search* (Audible, BookBeat) or *Discover*.
Hörwerk does the rest – the **Requests** tab shows where it stands:

`Waiting for approval → Searching → Downloading → Processing → Available` (or *Declined* with a reason, *Failed*)

* **Approval:** requests from family accounts wait until an admin approves them (ntfy "New book request", count on
  the tab). Admins and accounts with the right *no approval* start right away.
* **Quota:** requests per calendar month (default 5, adjustable per account; declined ones don't count).
* **Visibility:** everyone sees their own requests; with the right *see all requests* also everybody else's.
* **Getting it automatically (like Radarr):** Hörwerk searches all indexers and judges every release – right author,
  right title, unabridged, right language, size fitting the runtime (kbit/s), a pack with exactly this volume? The best
  match goes to SABnzbd with the ASIN and the request number; Hörwerk then follows the download into the library. If
  the download fails or the book doesn't fit on import, that release is blocked and the next one taken (at most 5
  attempts). Without a match it searches again every 12 hours.
* **Interactive search (only when needed):** admins and accounts with the right *interactive search* open
  **Releases** on a request: every match with its verdict – the rejected ones too, with the reason ("abridged
  edition", "too small for 9 h runtime" …). Download one by hand, block one, or search again with other words.
* **Telling the requester:** whoever enters their own ntfy topic under *Account* hears when their audiobook has
  arrived or was declined.

**Books that reach the inbox another way** (uploaded, from JDownloader or similar): the recognition also compares
every book with the open requests. If it clearly is a requested book (at least the configured *minimum certainty*,
default 80 %), it is linked, filed and the request set to *Available* automatically; the search for it stops. Below
that it waits under *Audiobooks → To review* for an admin – the requested book is offered first with "Request by …",
**Take** fulfils the request, **Edit** corrects the recognition first.

A "Wanted" list from version 5.3 is taken over as the admin's requests on first start.

---

## Notifications (ntfy)

Push messages to your phone via [ntfy](https://ntfy.sh) – ntfy.sh or your own server.

1. Install the ntfy app and subscribe to a topic, e.g. `audiobooks-1234`.
2. **⚙︎ Settings → Notifications (ntfy)**: server, topic, optionally an access token (or user + password) and
   priority. *Address of this UI* makes tapping a message open Hörwerk.
3. Tick the events you want and click **Send test message**.

| Event | Default |
|---|---|
| Audiobook filed | on |
| Audiobook needs review | on |
| Already in the library | on |
| Error with an audiobook | on |
| New book request | on |
| Book request failed | on |
| Download sent to SABnzbd | off |
| Download finished in SABnzbd | off |
| Download failed in SABnzbd | on |
| Audiobookshelf scan failed | on |
| Plex scan failed | on |

Token and password stay on the server; they are not sent to another server if you change the address.
Messages use the language of the interface.

---

## Log

**Log** in the top bar shows what happened: books found, converted, matched and filed, media server scans, warnings
and errors. Filter by level or text, follow it live, download it as a file. The same lines are in the container log
(`docker logs hoerwerk`).

---

## Accounts and access from outside

Without signing in Hörwerk only shows the login page. On the **first visit** you create the admin account (directly
from the home network; from outside the setup code from `docker logs hoerwerk` is needed). If you had
`AUTH_USER`/`AUTH_PASS` set, they become the admin account automatically.

* **Accounts** (top right, admins only): create family accounts (generate an initial password and pass it on), role,
  rights (*see all requests*, *no approval*, *interactive search*), monthly quota, lock, reset password, delete.
  Family accounts see *Search*, *Discover* and *Requests* – inbox, library, settings and log stay with admins. At the
  bottom of the dialog: the **admin log** (who changed what, when, with IP).
* **Account** (your name, top right, everyone): change password, set up a **second factor** (authenticator app such
  as Aegis, 2FAS, Google/Microsoft Authenticator), own ntfy topic, see and sign out devices.
  With *Admins need a second factor* (settings) admins without one only reach their account.
* "Stay signed in" lasts 30 days (extended while used), otherwise 12 hours.

**On the internet via Pangolin:**
1. Create a Pangolin resource pointing at `http://<unraid-ip>:8080` (Pangolin does HTTPS).
   Pangolin's own login can stay in front as well – safer, but you sign in twice.
2. Set `TRUSTED_PROXIES` to the address Newt/Pangolin arrives from (e.g. `172.18.0.0/16` for the Docker network or the
   IP of the Newt container). Only then the log shows the visitors' real IP.
3. If you use `ALLOWED_HOSTS`: add the public name, e.g. `tower,192.168.1.10,audiobooks.example.com`. In Pangolin, set the
   health check path to `/api/health`.
4. **CrowdSec:** every failed login is in `/config/logs/auth.log` with its IP. Parser, scenarios and examples for
   Vector or the CrowdSec agent are in [contrib/crowdsec](contrib/crowdsec/README.md). Hörwerk itself blocks an IP for
   15 minutes after 5 failures; CrowdSec then bans it right at the VPS.

---

## Configuration

Almost everything can be changed in **⚙︎ Settings**; the environment only provides the starting values.
Settings, presets, history, the audiobook list, the library index and all credentials live in the SQLite database
`/config/hoerwerk.db` (permissions 600).

| Variable | Default | Meaning |
|---|---|---|
| `APP_LANGUAGE` | `de` | Interface and notifications: `en` or `de` |
| `PUID` / `PGID` | `99` / `100` | User/group for all file access |
| `UMASK` | `000` | Permissions of new files and folders |
| `INBOX_DIR` / `LIBRARY_DIR` | `/inbox` / `/audiobooks` | Audiobook inbox and library |
| `AUDIBLE_REGION` | `de` | Audible store: `de us uk fr it es ca au in jp` |
| `MATCH_MODE` | `auto` | `auto` (clear matches automatically), `review` (always ask), `off` (only convert) |
| `MATCH_THRESHOLD` | `0.8` | Minimum confidence for automatic filing |
| `FOLDER_TEMPLATE` | `{author}/[{series}/][{series_position:2} - ]{title}` | Folder in the library |
| `FILE_TEMPLATE` | `{title}` | File name of the M4B |
| `CHAPTERS` | `auto` | Chapter names from Audible: `auto`, `audible`, `files` |
| `COVER_SOURCE` | `audible` | `audible` (store, full resolution) or `local` |
| `INBOX_ORIGINALS` | `archive` | Originals afterwards: `archive` (to `_fertig`) or `delete` |
| `SETTLE_SECONDS` | `120` | How long an inbox folder must stay unchanged |
| `SCAN_INTERVAL` | `60` | Seconds between two inbox checks |
| `METADATA_FALLBACK` | `true` | Ask Audnexus, Apple Books and Open Library when Audible finds nothing |
| `WRITE_ABS_METADATA` | `true` | Write `metadata.json` for Audiobookshelf |
| `DUPLICATES` | `ask` | Book already in the library: `ask`, `skip`, `keep` |
| `LIBRARY_RESCAN_HOURS` | `6` | How often the library is rescanned |
| `LIBRARY_ORIGINALS` | `archive` | Old single files after “To M4B”: `archive` (to `_ersetzt`) or `delete` |
| `NARRATOR_IN_ARTIST` | `false` | Narrator also in the artist tag |
| `KEEP_SERIES_IN_TITLE` | `true` | Keep “…, Band 3” in the title |
| `WRITE_DESC` / `WRITE_READER` | `false` | Write `desc.txt`, `reader.txt` (only needed without `metadata.json`) |
| `WRITE_COVER` | `true` | Write `cover.jpg` (used by Audiobookshelf and Plex) |
| `DISCOVER_MIN_RATING` / `DISCOVER_MIN_COUNT` | `4.3` / `50` | Discover: minimum rating and number of ratings |
| `DISCOVER_UNABRIDGED` / `DISCOVER_GERMAN` | `true` | Discover: unabridged only / German only |
| `DISCOVER_NONFICTION_STORE` | `false` | Discover: non-fiction in "For you" too |
| `WANTED_INTERVAL_HOURS` | `12` | Requests without a match: hours until the next search |
| `ADMIN_TOTP_REQUIRED` | `false` | Admins need a second factor |
| `CPU_CORES` | half the cores | Parallel ffmpeg processes per book |
| `ENCODE_WORKERS` | `1` | Books encoded at the same time |
| `IO_WORKERS` | `2` | Parallel matching/tagging/filing steps |
| `NICE` | `10` | CPU priority of ffmpeg (0 = normal) |
| `FFMPEG_SANDBOX` | `auto` | ffmpeg in a Landlock sandbox: `auto`, `require`, `off` |
| `FFMPEG_MEMORY_MB` | `4096` | Memory limit per ffmpeg process |
| `AAC_BITRATE` | `auto` | `auto` (like the source, max. 128k) or a fixed value |
| `FORCE_MONO` | `false` | Output mono |
| `CHAPTER_TITLES` | `auto` | `auto`, `filename` or `tag` |
| `TRUSTED_PROXIES` | – | IPs/networks of the reverse proxy (Pangolin/Newt, Traefik), e.g. `172.18.0.0/16` – only from there `X-Forwarded-For` counts |
| `COOKIE_SECURE` | `auto` | Login cookie only over HTTPS: `auto` (when the proxy reports HTTPS), `true`, `false` |
| `REQUEST_QUOTA` | `5` | Book requests per user and month (adjustable per account) |
| `AUTH_USER` / `AUTH_PASS` | – | Only for upgrading: becomes the admin account on first start |
| `ALLOWED_HOSTS` | – | Only answer under these host names/IPs, e.g. `tower,192.168.1.10` (`localhost` always works, and `/api/health` answers under any name – for the Docker and Pangolin health checks) |

SABnzbd, the optional indexer, Audiobookshelf, Plex and ntfy are set up in the interface, not via variables.

---

## Security

Short version – details, tests and component versions in [SECURITY.md](SECURITY.md):

* Accounts with login; on the internet only behind a reverse proxy with HTTPS (e.g. Pangolin) and with
  `TRUSTED_PROXIES`. Failed logins are in `/config/logs/auth.log` with their IP – ready for CrowdSec.
* Mount only the folders you need; the container runs without Linux capabilities and with `no-new-privileges`.
* Downloaded files are never executed. ffmpeg is an own minimal build (audio only, no network code) running in a
  Landlock sandbox that can only read the book's files.
* API keys, tokens and passwords (SABnzbd, indexer, Audiobookshelf, Plex, ntfy) stay on the server and are never sent
  to the browser.

---

## Development

```bash
pip install -r requirements.txt                 # mutagen + PyYAML
npm ci && npm run build                         # TypeScript (frontend/src) -> app/static/*.js
python3 -m unittest discover -s tests           # Python tests (need ffmpeg, no network)
npm run check                                   # type check
npx playwright install chromium && npx playwright test   # click-through tests against a real server
INBOX_DIR=/tmp/inbox LIBRARY_DIR=/tmp/lib CONFIG_DIR=/tmp/br PORT=8080 python3 app/main.py
```

**Translations:** German is the source language. Every UI text is wrapped in `t("…")` (Python and TypeScript);
`app/static/i18n/en.json` holds the English text. `python3 tests/test_i18n.py --missing` lists texts that still
need a translation – the test suite fails until every text has one.

CI runs on every pull request: Python tests (with the system ffmpeg and with the own ffmpeg 9 in a mandatory
sandbox), type check, click-through tests against the source and against the built Docker image.

| Part | Files |
|---|---|
| Start & wiring | `app/main.py`, `app/config.py`, `app/services.py` |
| Web | `app/web/server.py`, `app/web/router.py`, `app/web/routes/*` |
| Data | `app/db.py` (SQLite), `app/stores.py` |
| Jobs | `app/engine.py`, `app/pipeline.py`, `app/converter.py`, `app/events.py` |
| Audio | `app/encoder.py`, `app/sandbox.py`, `app/meta.py`, `docker/build-ffmpeg.sh` |
| Metadata | `app/audible.py`, `app/providers.py`, `app/links.py`, `app/tagger.py`, `app/abs.py` |
| Library & downloads | `app/library.py`, `app/library_index.py`, `app/usenet.py` |
| Notifications & log | `app/notify.py`, `app/applog.py` |
| Media servers | `app/abs.py`, `app/plex.py` |
| Discover | `app/discover.py`, `frontend/src/discover.ts` |
| Language | `app/i18n.py`, `app/static/i18n/en.json` |
| Interface | `frontend/src/*.ts`, `app/static/index.html`, `style.css`; E2E tests in `e2e/` |

Inspired by the open-source projects [auto-m4b](https://github.com/seanap/auto-m4b) and
[beets-audible](https://github.com/Neurrone/beets-audible); chapter data via [Audnexus](https://github.com/laxamentumtech/audnexus).
