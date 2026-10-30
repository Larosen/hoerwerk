# Better Renamer

**English** · [Deutsch](README.de.md)

A self-hosted web app for your NAS (Unraid or any Docker host) that does two jobs:

* **Audiobook pipeline** – drop audiobooks into an inbox folder; they become **one M4B with chapters**,
  are matched on Audible, **tagged** and **filed** into a clean library for Audiobookshelf, Plex & co.
* **Batch renamer** – rename hundreds of files and folders in the browser with a live preview, 16 combinable
  rules and metadata placeholders (ID3, video, EXIF).

Downloads go through **SABnzbd**, notifications through **ntfy** – you choose which events you hear about.
The interface is available in **English and German**.

> Better Renamer is a tool for organising audio files you are entitled to use – your own purchases, your own
> recordings or public-domain works (e.g. from [LibriVox](https://librivox.org)). It contains no media,
> does not remove copy protection and is not affiliated with Audible, Amazon, Apple or any other service named here.

---

## Contents

1. [Features](#features)
2. [Quick start](#quick-start)
3. [Audiobooks](#audiobooks)
4. [Downloads with SABnzbd](#downloads-with-sabnzbd)
5. [Notifications (ntfy)](#notifications-ntfy)
6. [Batch renamer](#batch-renamer)
7. [Configuration](#configuration)
8. [Security](#security)
9. [Development](#development)

---

## Features

| Area | What you get |
|---|---|
| Audiobook inbox | Waits until a folder is fully copied, merges MP3/M4A/FLAC/OGG/Opus/WMA (also `CD1`/`CD2` subfolders) into one M4B with sample-exact chapters |
| Matching | Audible search in 10 stores, rated by title, author **and runtime** (unabridged ≠ abridged ≠ radio play); uncertain matches wait for you |
| Tags & files | Title, author, narrator, series + volume, genre, publisher, date, description, ASIN, full-size cover; `metadata.json` for Audiobookshelf |
| Library | Folder template, reuses existing author/series folders, detects duplicates, converts old MP3 books to M4B |
| Downloads | Sends NZBs to SABnzbd, watches the category, the inbox takes over when the download is done; optional Newznab search |
| Notifications | ntfy push messages, ten events, each one on or off |
| Renamer | Live preview, conflict check, undo history, presets, subfolders, companion files move along |
| Interface | English / German, light / dark theme, live updates without reloading |
| Security | Own minimal ffmpeg 9 in a Landlock sandbox, CSRF protection, secrets never reach the browser ([SECURITY.md](SECURITY.md)) |

---

## Quick start

### Unraid

1. **Docker → Add Container**, enter as *Template*:
   `https://raw.githubusercontent.com/Larosen/rename_merge/main/unraid/better-renamer.xml`
2. Set the paths:
   * **Medien** (`/data`) – the shares the renamer may touch, e.g. `/mnt/user/media/`
   * **Hörbuch-Eingang** (`/inbox`) – where new audiobooks arrive
   * **Hörbuch-Bibliothek** (`/audiobooks`) – the finished library (add it to Audiobookshelf/Plex)
3. Pick **Language / Sprache** (`APP_LANGUAGE`: `en` or `de`). PUID `99`, PGID `100`, UMASK `000` are already right for Unraid.
4. Open `http://<unraid-ip>:8080`.

Image: `ghcr.io/larosen/rename_merge:latest` (amd64 + arm64, rebuilt weekly for security updates).

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

### Avoiding duplicates

The library is scanned at start and every few hours (`LIBRARY_RESCAN_HOURS`) – including older MP3 folders, recognised
by ASIN, tags or the folder structure. If a new book is already there (same ASIN, same author + title or same
series + volume), you decide: **Discard**, **Replace old** (the old version moves to `_ersetzt` in the inbox) or
**File additionally**. `DUPLICATES=skip` or `keep` decides automatically.

### Turning old MP3 books into M4B

**Audiobooks → Not an M4B** lists every library book that still consists of single files. Select books, click
**To M4B**, optionally adjust chapter names first (**Chapters & names …**, also renames the source files to
`01 - Chapter.mp3` if you like). The old version is moved away only after the new M4B is tagged.

### More metadata sources

If Audible is unreachable and the ASIN is known, the same data comes from **Audnexus**. If Audible finds nothing,
**Apple Books** and **Open Library** are asked. Those matches have no runtime or narrator, so they are never filed
automatically. Switch off with `METADATA_FALLBACK=false`.

### Audiobookshelf

* Next to every book there is a `metadata.json` in Audiobookshelf's format (title, authors, narrators, series,
  genres, description, ASIN, chapters) – `WRITE_ABS_METADATA=false` turns it off.
* Optional, under **Search → Set up**: address and API token of Audiobookshelf. After new books are filed, the
  library is rescanned there – useful on network shares where its folder watcher misses changes.

---

## Downloads with SABnzbd

You only need SABnzbd; searching is optional.

1. In SABnzbd create a category, e.g. `audiobooks`, whose folder is the one mounted as **audiobook inbox** (`/inbox`).
2. In Better Renamer: **Search → Set up** → SABnzbd address, API key (SABnzbd → Config → General) and category.
3. Everything SABnzbd finishes in that category flows through M4B → Audible → library automatically. The
   **Downloads** list shows progress and history of the category.

**Optional search:** add a Newznab indexer (address + API key) to search directly from the **Search** tab and send
results to SABnzbd. Results are merged, ranked (preferred language, unabridged before abridged, M4B before MP3)
and compared with your library. Umlauts are searched in every spelling (`ä` / `ae` / `a`). **Look up on Audible**
first searches with the store's exact title and remembers the ASIN for tagging.

Tip: in SABnzbd set *Config → Switches → Unwanted extensions* (`exe, bat, cmd, scr, lnk, js, vbs, ps1, msi, com`)
to **Abort**.

---

## Notifications (ntfy)

Push messages to your phone via [ntfy](https://ntfy.sh) – ntfy.sh or your own server.

1. Install the ntfy app and subscribe to a topic, e.g. `audiobooks-1234`.
2. **⚙︎ Settings → Notifications (ntfy)**: server, topic, optionally an access token (or user + password) and
   priority. *Address of this UI* makes tapping a message open Better Renamer.
3. Tick the events you want and click **Send test message**.

| Event | Default |
|---|---|
| Audiobook filed | on |
| Audiobook needs review | on |
| Already in the library | on |
| Error with an audiobook | on |
| Download sent to SABnzbd | off |
| Download finished in SABnzbd | off |
| Download failed in SABnzbd | on |
| “To M4B now” done | off |
| “To M4B now” failed | on |
| Audiobookshelf scan failed | on |

Token and password stay on the server; they are not sent to another server if you change the address.
Messages use the language of the interface.

---

## Batch renamer

1. Choose a folder on the left. Options: **Subfolders**, **Folder names**, **Companion files**, **“/” = folder**,
   **Number conflicts**.
2. Add rules – they apply in order; the preview highlights every change.
3. Select with checkboxes (Shift-click for ranges), type chips (Audio/Video/Images …) or the filter.
4. **Rename** (⌘/Ctrl + Enter). **Undo** (⌘/Ctrl + Z) or via the **History** (last 50 runs).

Example with a public-domain recording: `the_hound_of_the_baskervilles_ch1.mp3` → *Clean up* (`_` → space),
*Case* (Title) and *Pad numbers* → `The Hound of the Baskervilles Ch01.mp3`. With the template rule
`{track:2} - {title}` the name comes straight from the ID3 tags.

### Rules

| Group | Rule | What it does |
|---|---|---|
| Text | Replace text | Find/replace, all/first/last occurrence, whole word, regex with `$1`/`\1` |
| | Insert text | At start, end, position or before/after a text; placeholders allowed |
| | Remove range | N characters from a position, or everything before/after a text |
| | Case | Title, each word, sentence, UPPER, lower, invert; exceptions like “MP3, DJ” |
| | Truncate | To N characters, optionally at a word boundary or from the end |
| Characters | Clean up | `_ . %20` → space, brackets/leading digits away, umlauts → ae/oe/ue, Windows/SMB-safe, trim |
| | Remove characters/words | Digits, letters, punctuation, non-ASCII, own characters, word list |
| Position | Move text | Match (also regex) to the start/end |
| | Swap parts | “Artist - Title” → “Title - Artist” |
| Numbers | Number | 1/a/A/I/i, start, step, digits, prefix/suffix, position, restart per folder |
| | Pad numbers | “Track 1” → “Track 01” |
| | Shift numbers | Change existing numbers by ±N |
| Date & metadata | Insert date | Taken (EXIF/video/ID3), modified, created, today – any format |
| | Template / metadata | Build the name from placeholders, e.g. `{track:2} - {title}` or `{show} S{season:2}E{episode:2}` |
| | From list | Pasted list: line 1 for the first file, line 2 for the second … |
| Extension | File extension | lower/UPPER, normalise (jpeg → jpg), replace, append, remove |

Every rule can have a **condition**: only if the name contains something, matches a regex or has a given extension
(also inverted).

### Placeholders

| Placeholder | Meaning |
|---|---|
| `{name}` `{original}` `{ext}` | current name, original name, extension |
| `{n}` `{n:3}` `{folder_n}` | running number (padded), number within the folder |
| `{folder}` `{parent}` | folder of the file, folder above |
| `{modified:%Y-%m-%d}` `{created:…}` `{taken:…}` `{now:…}` | dates in any format |
| `{size}` `{bytes}` | file size |
| `{title}` `{artist}` `{album}` `{album_artist}` `{track:2}` `{disc}` `{year}` `{genre}` `{composer}` `{duration}` `{bitrate}` | audio |
| `{resolution}` `{width}` `{height}` `{codec}` `{fps}` `{show}` `{season}` `{episode}` | video |
| `{camera}` `{make}` `{model}` `{lens}` `{iso}` `{fnumber}` `{exposure}` `{focal}` `{dimensions}` | photo (JPEG, HEIC, RAW) |

Formats after the colon: number = pad with zeros, `upper` `lower` `title` `ascii`, `0-20` = slice, for dates a
strftime format. The ⓘ next to a file shows every placeholder with its value.

**To M4B now** in the renamer merges the MP3s of the open folder into `<folder>.m4b` right there;
**→ Library** sends the folder through the audiobook pipeline and leaves the originals untouched.

---

## Configuration

Almost everything can be changed in **⚙︎ Settings**; the environment only provides the starting values.
Settings, presets, history, the audiobook list, the library index and all credentials live in the SQLite database
`/config/better-renamer.db` (permissions 600).

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
| `WRITE_DESC` / `WRITE_READER` / `WRITE_COVER` | `true` | Write `desc.txt`, `reader.txt`, `cover.jpg` |
| `CPU_CORES` | half the cores | Parallel ffmpeg processes per book |
| `ENCODE_WORKERS` | `1` | Books encoded at the same time |
| `IO_WORKERS` | `2` | Parallel matching/tagging/filing steps |
| `NICE` | `10` | CPU priority of ffmpeg (0 = normal) |
| `FFMPEG_SANDBOX` | `auto` | ffmpeg in a Landlock sandbox: `auto`, `require`, `off` |
| `FFMPEG_MEMORY_MB` | `4096` | Memory limit per ffmpeg process |
| `AUTO_CONVERT` | `true` | “Then MP3 → M4B” on by default in the renamer |
| `AAC_BITRATE` | `auto` | `auto` (like the source, max. 128k) or a fixed value |
| `FORCE_MONO` | `false` | Output mono |
| `CHAPTER_TITLES` | `auto` | `auto`, `filename` or `tag` |
| `TITLE_SOURCE` | `folder` | `folder` or `album` |
| `OUTPUT_NAME` | `{folder}` | Name of the M4B in the renamer, placeholders `{folder} {album} {artist}` |
| `ORIGINALS` / `ORIGINALS_FOLDER` | `keep` / `_mp3` | MP3s after “To M4B now”: `keep`, `move`, `delete` |
| `SIDECARS` / `SIDECAR_EXTS` | `true` / `xmp,srt,…` | Companion files move along with their file |
| `CONFLICT` | `error` | `error` or `number` (append “(2)”) |
| `ALLOW_SUBDIRS` | `false` | “/” creates folders by default |
| `MAX_ITEMS` | `20000` | Maximum entries in the preview |
| `SHOW_HIDDEN` | `false` | Show hidden files |
| `AUTH_USER` / `AUTH_PASS` | – | Password protection (Basic Auth) – recommended |
| `ALLOW_PUBLIC` | `false` | Without a password only private networks are answered; `true` lifts that |
| `ALLOWED_HOSTS` | – | Only answer under these host names/IPs, e.g. `tower,192.168.1.10` |

SABnzbd, the optional indexer, Audiobookshelf and ntfy are set up in the interface, not via variables.

---

## Security

Short version – details, tests and component versions in [SECURITY.md](SECURITY.md):

* Set `AUTH_USER`/`AUTH_PASS` and never expose the UI to the internet without a reverse proxy with HTTPS.
  Without a password the server only answers private networks.
* Mount only the folders you need; the container runs without Linux capabilities and with `no-new-privileges`.
* Downloaded files are never executed. ffmpeg is an own minimal build (audio only, no network code) running in a
  Landlock sandbox that can only read the book's files.
* API keys, tokens and passwords (SABnzbd, indexer, Audiobookshelf, ntfy) stay on the server and are never sent
  to the browser.

---

## Development

```bash
pip install -r requirements.txt                 # mutagen + PyYAML
npm ci && npm run build                         # TypeScript (frontend/src) -> app/static/*.js
python3 -m unittest discover -s tests           # Python tests (need ffmpeg, no network)
npm run check                                   # type check
npx playwright install chromium && npx playwright test   # click-through tests against a real server
DATA_DIR=/tmp/data INBOX_DIR=/tmp/inbox LIBRARY_DIR=/tmp/lib CONFIG_DIR=/tmp/br PORT=8080 python3 app/main.py
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
| Notifications | `app/notify.py` |
| Language | `app/i18n.py`, `app/static/i18n/en.json` |
| Renamer | `app/rules.py`, `app/planner.py` |
| Interface | `frontend/src/*.ts`, `app/static/index.html`, `style.css`; E2E tests in `e2e/` |

Inspired by the open-source projects [auto-m4b](https://github.com/seanap/auto-m4b) and
[beets-audible](https://github.com/Neurrone/beets-audible); chapter data via [Audnexus](https://github.com/laxamentumtech/audnexus).
