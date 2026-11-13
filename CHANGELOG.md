# Changelog

What changed in each version of Hörwerk. Every version is also a tag (`v7.18.0`), a GitHub release and an image
(`ghcr.io/larosen/hoerwerk:7.18.0`); `latest` is always the newest one.

## 7.18.0 – 2026-11-12

- **Convert the library bit by bit:** under *Audiobooks → Not an M4B*, "Convert all, bit by bit" turns the MP3 books
  into M4Bs one after the other, with one CPU core at the lowest priority. New books and downloads go first; what is
  unclear waits under "To review"; it stops by itself when nothing is left.
- **Sort with one tap:** releases by best, newest, oldest, largest, smallest, name or indexer; the library by author,
  title, newest, oldest or "not M4B first".
- **Phone:** tapping into a field no longer zooms the page; the log shows time and source in one line and the text
  below it; the release window uses the whole screen and gives every verdict its own line.
- **Follow authors and narrators:** tap a name on any card to see all their books at Audible, tick some or "all
  missing" and request them as one request (it counts once for the monthly limit). Following someone means a new
  book is requested by itself once a day – or only marked as new under *Discover → Following*, which lists everybody
  you follow.
- AudioBookRequest is part of the comparison in the README.
- **Tidier pages:** *Search* no longer shows the sources box – connections live under *Settings*, the search only
  says when an indexer or SABnzbd is missing. *Audiobooks* has no sidebar any more: upload and "Check inbox" sit next
  to the heading, the inbox status is one line that only grows when something needs a look. Library scan (now with
  its interval and "Scan now") and the one-off `desc.txt`/`reader.txt` cleanup moved to *Settings → Inbox & library*.
- **"How it works" boxes can be switched off** for everybody under *Settings → General*.
- **Blocked addresses in the UI:** *Settings → Accounts → Security* lists the addresses Hörwerk blocked after failed
  logins, with **Unblock**; the health check links there.
- **Sandbox hint explained:** when ffmpeg runs without Landlock, the health check says why (kernel without Landlock,
  not switched on at boot, or forbidden by Docker) and, behind a **?**, how to switch it on – including Unraid.
- Fixed: "Tidy chapters" changed the M4B, but Audiobookshelf kept showing the old names – it reads them from
  `metadata.json`, which is updated now as well (also for books tidied before), followed by a rescan.

## 7.17.2 – 2026-11-12

- Fixed: the server could freeze for good when a new folder arrived in the inbox at the very moment another book
  started its next step (two parts waiting for each other). Found while filing eight books at once; there is a test
  for exactly that now.
- The README is a short overview with a demo; the manual moved to `docs/en/` and `docs/de/`.
- English UI: the button says "Request" (not "Requests"), and quotation marks follow the language.

## 7.17.1 – 2026-11-12

- Every version is now a Git tag, a GitHub release and its own image tag, so you can pin a version instead of
  `latest` and go back to an older one.
- *Settings → Health* shows the commit the image was built from next to the version.
- New test: a crash in the middle of filing (old copy put aside, new one not there yet – or there, but the old one
  not archived yet) loses nothing and files nothing twice.
- Security page: brought up to the current version; a test keeps it from going stale.

## 7.17.0 – 2026-11-12

- Clean chapter names: track numbers, the book title and text every track repeats go ("01 - Book (Full-Cast
  Edition): Chapter One - The Worst Birthday" → "Chapter 1: The Worst Birthday"), number words become digits,
  nonsense falls back to "Chapter N" / "Kapitel N".
- Track numbers in the tags that contradict the order of the file names: the book waits for a look.
- **Tidy chapters** for books that are already in the library.

## 7.16.0 – 2026-11-12

- The right edition: a release that clearly is another recording (other narrator, reading instead of full cast,
  older than a new production) is not loaded.
- Downloads are checked by runtime, narrator tag and chapters before they are filed.
- Every library book can be **corrected**, marked as the **wrong edition**, or searched again for a **better
  release** that then replaces it (the old file is archived).

## 7.15.0 – 2026-11-10

- Requests show download progress, time left and speed, then the conversion step.
- Another edition of a book no longer counts as "already in the library".
- An indexer's daily search limit is no error: it rests until midnight, and searches use fewer API calls.

## 7.14.0 – 2026-11-08

- The server is a single Rust program (smaller image, faster, no interpreter); same database as before.
- Free audiobooks from LibriVox in the search (German and English).
- Preferred indexer: asked first, the others only when it has nothing that fits.
