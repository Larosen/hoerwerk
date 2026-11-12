# Roadmap

Where Hörwerk is heading. The goal: the one place to **get, request and discover** audiobooks for a household –
with every book ending up as a clean, tagged M4B. Ideas and votes are welcome as issues.

## Recently done

- **7.17.1** – every version is a Git tag, a GitHub release and its own image tag (pin a version instead of
  `latest`); the full list of changes lives in [CHANGELOG.md](../CHANGELOG.md).
- **7.17** – clean chapter names: track numbers, the book title and text every track repeats go
  ("01 - Book (Full-Cast Edition): Chapter One - The Worst Birthday" → "Chapter 1: The Worst Birthday"), number words
  become digits, nonsense falls back to "Chapter N"; track numbers in the tags that contradict the file order mean
  a look before filing; **Tidy chapters** for books already in the library.
- **7.16** – the right edition: releases that clearly are another recording (other narrator, reading instead of
  full cast, older than a new production) are not loaded, and downloads are checked by runtime, narrator tag and
  chapters before they are filed. Every library book can be corrected, marked as the wrong edition, or searched again
  for a **better release** that then replaces it.
- **7.15** – requests show download progress, time left and speed, then the conversion step; another edition of a
  book (e.g. a full-cast production next to a single-narrator reading) no longer counts as "already in the library";
  an indexer's daily search limit is no error any more – it rests until midnight, and searches use fewer API calls.
- **7.14** – the server is a single Rust program (smaller image, faster, no interpreter); same database as before.
- **Free audiobooks from LibriVox** in the search (German and English), one click into the inbox.
- **Preferred indexer**: asked first, the others only when it has nothing that fits.

## Next

| | What | Why |
|---|---|---|
| 1 | **Follow authors and series** | "Tell me / request automatically when a new volume of this series or a new book by this author appears on Audible." Today you have to look yourself. |
| 2 | **Prowlarr sync** | Take indexers (and their categories) from Prowlarr instead of entering them twice. |
| 3 | **More free and legal sources** | ARD Audiothek (public-broadcaster audio dramas and readings, downloadable while online), Vorleser.net (classics read by professional speakers, free for private use), more public-domain collections of the Internet Archive with a clear licence. Each source only if its terms allow downloading. |
| 4 | **arm64 image** | Built on a native arm64 runner (Raspberry Pi, Apple-silicon NAS). |

## Later

- **Chapter editor** in the UI: rename, merge, split chapters before filing.
- **Per-account libraries / age-appropriate views** for kids' accounts.
- **Prometheus metrics** (`/metrics`) for queue length, conversions, failures.
- **Settings export/import** for moving to a new server.
- **PWA**: install the UI on the phone's home screen, push via ntfy already works.

## Not planned

- Anything that removes copy protection (DRM).
- Built-in torrent or download sources for copyrighted books. Hörwerk works with the downloader and indexers *you*
  configure; the only built-in sources are free and legal ones.
