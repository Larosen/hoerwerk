# Changelog

What changed in each version of Hörwerk. Every version is also a tag (`v1.0.0`), a GitHub release and an image
(`ghcr.io/larosen/hoerwerk:1.0.0`); `latest` is always the newest one.

## 1.0.13 – 2026-11-17

- **Second factor:** replacing or switching off an existing second factor needs the password and a current code; a
  stolen session alone is no longer enough. The account page has "Set up a new key" for it, and database errors while
  saving are reported instead of ignored.
- **Downloads connect only to checked addresses:** covers and other outside downloads are resolved once; the connection
  uses exactly the public addresses that passed the check, for every redirect hop (no DNS swap in between).
- **Slow requests:** a JSON request body must arrive within 20 seconds.
- **Bounded helper output:** ffmpeg/ffprobe output kept in memory is capped (stderr: its tail), and still drained.
- **Covers:** background fetching uses one bounded worker pool instead of new threads per list; a waiting request now
  finds the finished file.
- **Accounts:** the "last admin" rule is checked in the same lock and transaction as the change, so two admins can no
  longer remove each other. Password hashing runs outside the database lock, in blocking workers, at most four at once.
- **Chapter names:** a counter stuck to the book title (`Der Federmann002`, `Der Federmann003` …) is recognised as a
  counter, so these chapters become `Kapitel 2`, `Kapitel 3` … instead of staying as they are.
- **Upload:** Cancel and Escape work in every phase (also while the session is being created) and clean up.
- **Settings:** when the saved values cannot be loaded, the form stays locked and offers "Reload" instead of letting
  empty defaults be saved.

## 1.0.12 – 2026-11-16

- **First setup behind a proxy:** the setup code is only skipped for visitors from the own network who also use the own
  network's address (an IP, `localhost`, a name without a dot, `.local`/`.lan`). Under a public name it is always
  required – even when a proxy in the home network hides the visitor's address and sends no forwarding headers.
- **Login only succeeds when the session is stored:** a database error while saving the session no longer hands out a
  cookie for a login that would not hold; the page says so and the log has the cause.
- **A two-factor code cannot be used again after a restart:** the last accepted time step is stored per account.
- **Prometheus metrics:** `GET /api/metrics` (set `METRICS_TOKEN`; only with `Authorization: Bearer <token>`): queues,
  jobs by status, requests, library, uptime and held locks – counts only.
- **Indexers from Prowlarr** (Settings → Downloads → "Take indexers from Prowlarr"): address and API key of Prowlarr; its
  Usenet indexers are added as indexers of their own (asked through Prowlarr), known ones stay, torrents and switched-off
  ones are skipped, nothing is deleted.
- **Settings export/import** (Settings → General → "Moving to a new server"): settings and sources (indexers, SABnzbd) as
  a JSON file, API keys only with "Export with keys"; the file can be loaded on another server. Accounts, requests and
  the library are not part of it.
- **Phone flows in real WebKit:** the CI runs the new iPhone tests (login, tab bar, NZB picker) in WebKit as well.
- **arm64 image:** the release now builds the image for amd64 and arm64 (each on its own native runner) and publishes one multi-arch
  tag; signature, SBOM and build provenance are bound to its digest. A CI job checks the arm64 image on every change.
- Unused leftovers of the Rust port are removed (the crate-wide `allow(dead_code)` is gone).
- **iPhone: the end of a long list is reachable:** the body had a fixed height on phones, so long pages overflowed it and Safari
  left out the room for the work bar and tab bar – the last entry (e.g. its "Anfragen" button) sat under them. The body is
  now as tall as its content. A new test (run in WebKit in CI) checks the last entry sits above both bars.
- Docs: the security notes point to the tests that really exist; an outdated comment in `main.rs` is corrected.

## 1.0.11 – 2026-11-16

- **NZB files can be picked on the iPhone:** the file picker no longer filters by type (iOS does not know ".nzb" and
  greyed such files out); the name and the content are checked on upload instead.
- **Clearer buttons for a book that is in the library twice:** "Neues nicht übernehmen" (removes the second copy) and
  "Vorhandenes ersetzen"; the box says where the second copy lies and that the new M4B was made from it, the versions
  read "Vorhanden" and "Neu aus dem Doppel". For downloads "Altes ersetzen" is now "Vorhandenes ersetzen".

## 1.0.10 – 2026-11-16

- **Upload an NZB file for a request:** "NZB-Datei hochladen …" in the request's Releases window (also from the phone's
  downloads). The file goes to SABnzbd (as SABnzbd's own "Add NZB" does), and the request follows the download like a
  release it picked: "Lädt", the history notes who uploaded it, and the download is matched to the request when it
  arrives. Only who may search for requests can upload; files that are not NZBs are refused.
- **A library book that is there twice no longer comes back:** "Alte Fassung entfernen" (formerly "Verwerfen") on a
  conflict from the gentle conversion removes the old copy from the library – moved to `_ersetzt` or deleted, as set
  under "Alte Dateien danach" – and from the index. Before, only the new M4B was thrown away; the old MP3s stayed "not an
  M4B" and were converted again after every start. Nothing is removed when the kept book lies inside that folder.

## 1.0.9 – 2026-11-16

- **A hanging part no longer stalls the whole app:** the audiobook list and the live updates are worked out off the web
  server's threads. If they wait too long, the page says so (with a hint to the log) instead of every page getting slow.
- **Lock watchdog:** when a lock stays held for more than 30 seconds (a job, the job list, the inbox, the queues, the
  library index, the database or the log), the log says which one – so a hang can be found and fixed.
- **Debug log:** "Debug aufzeichnen" on the Protokoll page (setting `log_debug`, stays on until switched off) also
  records every request with its duration, every processing step and queue step, every outside fetch (without query
  strings or keys), live connections, library scans and locks held for more than 10 s. "Alles mit Debug" shows them.
  Slow answers (3 s and more), slow saving and a slow audiobook list are logged as warnings even without debug.
- Every log line shows its level in front – `DEBUG`, `INFO`, `WARN`, `ERROR` – as log files write it.
- **"Als „…“ ablegen" works for books read from their tags:** the own title was lost when the details were read from the
  tags again, so the book came back with the same conflict instead of being filed. Picking another match or entering
  details drops the own title again.
- The own title no longer repeats the authors when the tags name only one of the folder's "A & B".

## 1.0.8 – 2026-11-15

- **The log survives restarts and updates:** it is also written to `config/logs/hoerwerk.log` (rotated at 5 MB) and the newest
  entries are back after a start.
- **A metadata.json beside the files is checked before it is trusted** (e.g. the one Audiobookshelf writes): if the folder names
  another author, a short title leaves out a word of the folder name, or the shop's runtime of its ASIN does not fit, Hörwerk
  searches itself instead of filing the book under the wrong title.
- Translators, editors and illustrators in a metadata.json ("Name - Übersetzer", "Name (Hrsg.)") are no longer taken as authors;
  with only a translator listed, the author comes from the folder name.
- A match or details picked under "Zu prüfen" are no longer replaced by the metadata.json of the folder.
- Folder names with two authors ("A & B - Titel") are recognised.
- A short title that leaves out a word of the folder name ("Stenkelfeld" for "… - Stenkelfeld - Zugabe") is no longer filed
  without a look – it may be another work of a series or brand.
- **"Schon in der Bibliothek" shows which book it is:** cover, authors, narrators, path, the folder it was filed from (for books
  filed from now on) and a button to view it. When only author and title are the same but the folder name says more or the
  runtime differs a lot, the box says it is probably another work and suggests filing it under its own title
  ("Stenkelfeld – Zugabe"). Conflicts noted by older versions are checked again after the update.

## 1.0.7 – 2026-11-15

- **Large libraries start fast:** the library index now keeps every book's full details in the database, so a restart no longer reads
  all audio files again (5000 books: about 4 minutes before, 2 seconds now). After the update the library is read once more.
- Checking search results and requests against the library is faster with thousands of books (match keys are cached per book).
- "Nicht als M4B" shows 200 books at a time with "Mehr anzeigen" instead of rendering the whole library at once.

## 1.0.6 – 2026-11-15

- **Failed downloads are noticed:** a download still being checked, repaired or unpacked in SABnzbd no longer counts as finished; when it
  fails there (e.g. too many missing blocks), the request blocks the release and tries the next one – also requests already waiting.
- **A cancelled import no longer leaves its request hanging** on "Wird verarbeitet": the release is blocked and the request searches on.
- A release that goes to review after an earlier one failed is noted on its request again; SABnzbd's history is read further back
  (60 entries) so busy days don't hide a finished or failed download.
- **A program error no longer stops Hörwerk:** it ends only the request (which answers with an error number) or the one thread; every such
  error is written to the *Protokoll* ("Programmfehler in … – bitte melden") with where it happened.
- **A very long search term no longer crashes the server:** some text rules gave up on pathological input with a panic, which ended the
  whole process; they now leave such text unchanged, and search terms are cut to 300 characters.
- A broken or shortened invitation link says that the invitation does not hold (instead of raw JSON); unknown addresses lead to the start.
- *Indexer & SABnzbd* on the phone: each indexer is laid out in readable rows (name and categories, address, key, then *Aktiv*, *Zuerst*,
  move up and remove) instead of a cramped row that ran off the screen.
- **Library on the phone:** scrolling no longer jumps back to the top while something is being converted (live updates no longer rebuild
  the list).

## 1.0.5 – 2026-11-15

- **No more endless "Download fertig – wartet auf den Eingang":** a request whose finished download never shows up as an import is marked
  available when the book is in the library (a pack, an import whose entry was removed), otherwise after 6 hours the next release is tried.
- **Hörwerk knows the other recordings of a requested book:** other narrators and years of the same title by the same author, from Audible
  and BookBeat. The *Releases* window lists them (the one asked for first), and a release that names the narrator or the year of another
  recording is not loaded – one naming the requested narrator ranks higher.
- **A download that does not fit a request asks what to do** (wrong length, another recording's length, another book) instead of deciding
  alone: *Verwerfen* deletes it and blocks the release for this book, *Behalten* files it as the book it is; either way the request searches on.
- **Picking another book than requested asks first:** on a download for a request, choosing a match that is not the requested book offers
  *Löschen und weitersuchen* (the default) or, on purpose, *Trotzdem ablegen*.

## 1.0.4 – 2026-11-15

- **Nothing is converted on a guess:** when Hörwerk is not sure which book a download is, it waits under *Zu prüfen* first and converts only after
  a match was chosen.
- **A wrong download for a request is deleted:** when the content turns out to be another book, the import stops, the download is deleted, the
  release is blocked and the request searches on. On a job from a request, *Falsches Release* does the same by hand.
- **Short titles are not grabbed on a guess:** a release that names more than author and title ("Odyssee op Koelsch", "Odyssee – Ein Weltgedicht")
  is no longer downloaded automatically; the request says so and the release can still be chosen under *Releases*.
- **Cancel means cancel:** *Abbrechen* during recognising no longer lets the job carry on into converting.
- **Delete requests at any time:** a running download is stopped in SABnzbd (its files deleted), a running import cancelled and its download
  deleted; for a book that arrived, Hörwerk asks whether it should leave the library too. Family accounts delete their own requests, admins all.
- **Delete from the library (admins):** in the details of a library book – moved to `_ersetzt` or deleted, as set for replaced originals.
- *Fertige ausblenden* only hides the finished jobs (a second click shows them again) instead of deleting them from the list for good.
- **Werkbank:** the sidebar shows what is being worked on – downloading, unpacking, recognising, converting, tagging – with a progress bar.
  On the phone a slim bar above the tab bar, only while something is running. Admins see every job, family accounts their own requests.
- **Statistik counts every listener:** Audiobookshelf accounts and Plex profiles without a Hörwerk account get a card too (asked for nothing,
  heard X), and their hours count in the listening chart and the genres. Whoever neither asked for nor heard anything in the period has no card. The genre ribbon now shows what was heard when that is known.
- **Merge listeners:** under *Statistik → Hörer zusammenführen* an admin joins an Audiobookshelf account and a Plex profile of the same person
  into one card (and splits them again).

## 1.0.3 – 2026-11-14

- **Entdecken: filters instead of a wall of chips.** One row with *Genre*, *Länge* and *Erschienen*. The genres fold out on demand: main
  category, then its subcategories; a second tap on what is picked, *Alle Genres* or the × takes it back. On *Für dich* and *Aus der Kritik*
  the filters narrow the list; on *Nach Genre* the genre chooses the list and it asks for one when none is picked.
- The genre *LGBT* is no longer offered for picking (its books still show up in their other genres).

## 1.0.2 – 2026-11-14

- **Critics' picks are matched again** when the review address names a newer edition – picks already stored with an older recording are corrected once.
- **UK and US titles match:** "Philosopher's Stone" and "Sorcerer's Stone" count as one title, and the search asks for both.
- **Series with signs in the name** (*Die drei ???*, *Die drei !!!*): the search asks for "Fragezeichen"/"Ausrufezeichen" and the episode
  number; an episode release that names series, title and number is accepted without the author (H.G. Francis is never in it).
- **Daily indexer limits** rest only until the next possible reset (local midnight or midnight UTC, whichever comes first) instead of a full day.
- **Mac/iPad:** the sidebar and the detail column stay in place while the page scrolls.
- **Detail column** closes when you go to another page (it stayed open from *Entdecken* on *Suchen*); a list that arrives after you left no longer opens it.
- **After an update** an open tab says "Hörwerk wurde aktualisiert" and offers to reload, instead of running the old version until reloaded by hand.
- Critics' picks no longer show "via review".
- **Phone:** the detail sheet keeps the book's colour all the way down when scrolling (it ended halfway as a box).
- **Broken characters in release names** ("Philosopherâ€™s") are repaired before matching.
- **Protokoll tells more:** a search without a fitting release logs which releases were turned down and why, which indexers failed or rest; the details of a book log what the shop answered. If the description cannot be loaded, the column says so.

## 1.0.1 – 2026-11-14

- **Details for every suggestion:** tips from critics now open with description and *Similar to* as well.
- **Covers load in parallel:** the server fetches the covers of a list at once instead of one by one.
- **Smaller covers:** lists load Amazon covers at 300 px, the detail column at 600 px, instead of the original size.
- **Cover colours** follow the dominant colour of the cover instead of an average that turned most books brown.
- **Critics' picks:** a review of a new edition (year in the review address) is matched to a recording from that year, not to an older one with the same title.
- *Hörbücher → Nicht als M4B:* unticking *Alle* clears the selection again; the file-renaming tick says that it only concerns the old files.

## 1.0.0 – 2026-11-14

The first release. Hörwerk turns folders of MP3s, CD rips and downloads into one clean, tagged M4B per book – and lets the family
request, follow and discover audiobooks. Earlier numbers (7.x) were development versions; their history is below.

- **A new look – “Werkstatt & Regal”:** warm papers instead of neutral grey, covers on shelves, progress as a waveform.
- **Three colour schemes:** Hell (Papier), Dunkel (Abend – warm espresso instead of grey) and OLED (Nacht – true black). Choose under
  *Konto → Anzeige*: for all your devices (saved in the account) or only for this device. "Automatisch" follows the device.
- **Navigation:** a sidebar on iPad and Mac, a tab bar on the iPhone (*Entdecken · Suchen · Anfragen · Hörbücher · Mehr*). *Mehr* holds
  Protokoll, Statistik, Einstellungen and the account, with a red note when something failed lately.
- **Entdecken:** one column with the tabs *Für dich · Nach Genre · Aus der Kritik · Gefolgt* and genres as chips. Several authors or narrators
  stand as running text ("… u. a.").
- **Hörbücher:** the library as a shelf (covers stand on a board) or as a list; *Besser*, *Kapitel* and *Korrigieren* are icon buttons.
- **Protokoll:** choose the period – 15 min, 60 min (default), 24 h or everything.
- **Progress as a waveform** – downloads, conversions and requests; the sign-in page carries the same track as a signature.
- **Search starts with the shops:** *Suchen* only matches Audible and BookBeat. *Anfragen* then asks the indexers in the order set under
  *Einstellungen* and stops at the first very good release. Searching indexers and LibriVox directly is a link shown only when nothing was found.
- **Details everywhere:** picking a book shows cover, date, length, narrator and description; library books open the same column.
- **Statistik on the phone** has the genre ribbon and the listening hours per month.
- **Details of an audiobook:** on wide screens (Mac, iPad landscape) a column on the right, otherwise a sheet from below – cover, description,
  facts, request and search, and *Weiterstöbern*: more by the author or narrator, the whole series, and **Ähnlich wie** (what listeners of the book also
  listened to, from Audible). A tap on a similar audiobook opens it right there; *Zurück* leads back. In *Entdecken* and *Suchen*.
- **The colour of a book:** cards, requests and the detail column take a tone from the cover – read from the picture, a cloth tone; without a cover
  from the tone of the stand-in tile. Online covers go through a cache on the server (`/api/cover`, public https hosts only), so the browser no longer
  asks the shops' image servers.
- **iPhone app:** Hörwerk can be put on the home screen and then runs full screen ([guide](docs/en/iphone-app.md)); manifest and icons are
  served without signing in, nothing else is.
- **License:** [PolyForm Noncommercial 1.0.0](LICENSE) – free for personal and other noncommercial use. The licenses of all Rust libraries are in
  `THIRD-PARTY-LICENSES.md` and in the image under `/licenses`.
- Wording: "Hörbuch/Hörbücher" instead of "Buch/Bücher" throughout.

## Development history (versions before 1.0)

## 7.20.0 – 2026-11-13

- **New license:** Hörwerk is now under the [PolyForm Noncommercial License 1.0.0](LICENSE) – free for personal and
  other noncommercial use. Earlier versions (7.19.0 and older) were published under the MIT license; their releases
  and images were withdrawn.
- The licenses of all Rust libraries are in `THIRD-PARTY-LICENSES.md` and in the image under `/licenses`.
- The README was rewritten (new introduction, no comparison with other projects).

## 7.19.0 – 2026-11-13

- **Follow authors and narrators:** tap a name on any card to see all their books at Audible, tick some or "all
  missing" and request them as one request (it counts once for the monthly limit). Following someone means a new
  book is requested by itself once a day – or only marked as new under *Discover → Following*, which lists everybody
  you follow.

## 7.18.0 – 2026-11-12

- **Convert the library bit by bit:** under *Audiobooks → Not an M4B*, "Convert all, bit by bit" turns the MP3 books
  into M4Bs one after the other, with one CPU core at the lowest priority. New books and downloads go first; what is
  unclear waits under "To review"; it stops by itself when nothing is left.
- **Sort with one tap:** releases by best, newest, oldest, largest, smallest, name or indexer; the library by author,
  title, newest, oldest or "not M4B first".
- **Phone:** tapping into a field no longer zooms the page; the log shows time and source in one line and the text
  below it; the release window uses the whole screen and gives every verdict its own line.
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
