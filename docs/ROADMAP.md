# Roadmap

Where Hörwerk is heading. The goal: the one place to **get, request and discover** audiobooks for a household –
with every book ending up as a clean, tagged M4B. Ideas and votes are welcome as issues.

## Recently done

- **1.0.12** – security hardening of the first setup, login and two-factor codes; phone flows tested in WebKit; arm64 image checked in CI;
  Prometheus metrics (`/api/metrics`); indexers from Prowlarr; settings export/import for moving to a new server.
- **1.0** – the first release, with a new look, "Werkstatt & Regal": three colour schemes (light, dark, OLED – per account or per device), sidebar on iPad and
  Mac, tab bar on the iPhone, the library as a shelf, progress as a waveform, a log you can narrow to 15 or 60 minutes, and Hörwerk
  as an app on the iPhone home screen ([guide](en/iphone-app.md)). Design notes: [docs/design](design/KONZEPT.md).

Development versions before 1.0:

- **7.19** – **follow authors and narrators**: a new book is requested by itself once a day (or only shown as
  new), and all their books can be ticked and requested together.
- **7.18** – tidier pages, blocked addresses in the UI, a guide for the ffmpeg sandbox, phone use, one-tap sorting,
  gentle conversion of the whole library.
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
| 1 | **More free and legal sources** | ARD Audiothek (public-broadcaster audio dramas and readings, downloadable while online), Vorleser.net (classics read by professional speakers, free for private use), more public-domain collections of the Internet Archive with a clear licence. Each source only if its terms allow downloading. |
| 2 | **arm64 image** | The image already builds and runs on a native arm64 runner in CI (checked on every change); publishing it is next. |

## Not planned

- Chapter editor and per-account libraries / kids' accounts: the automatic chapter names are right nearly always, and every
  book ends up in the same library. Kids' views would first need Audiobookshelf to recognise children's books by their
  tags and mark them – nothing Hörwerk can do on its own.

- Anything that removes copy protection (DRM).
- Built-in torrent or download sources for copyrighted books. Hörwerk works with the downloader and indexers *you*
  configure; the only built-in sources are free and legal ones.
