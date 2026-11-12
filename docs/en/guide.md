# Audiobooks: from inbox to library

[← README](../../README.md) · **English** · [Deutsch](../de/guide.md)

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
   **Quality** (`AUDIO_QUALITY`): radio plays (detected from title, genre, release name or a cast of narrators) get
   more bitrate than readings – `standard` 128k/192k, `high` 160k/256k (default), `original` up to 320k and keeps
   lossless ALAC. Never more than the source has. Dolby Atmos / E-AC-3 is always packed as is, never mixed down
   (the Audiobookshelf web player may not play it; most apps do). Chapters behind the end of the file are dropped.
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
