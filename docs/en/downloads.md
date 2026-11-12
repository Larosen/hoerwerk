# Downloads, free audiobooks, discover and requests

[← README](../../README.md) · **English** · [Deutsch](../de/downloads.md)

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

**Preferred indexer:** tick **first** next to one indexer under *Settings → Downloads*. Every search (by hand and
for requests) then asks that indexer alone; the others are only asked if it has nothing that fits the book. The
search shows "searched at X only" with a button to ask the others as well.

**The right edition:** a release that clearly is another recording than the requested book is not loaded – a
reading named "read by …" when the full-cast production is wanted, another narrator, or (for a new production) a
release older than the production. Releases that say nothing about it are loaded as before. After the download the
runtime decides: up to 8 % off is filed, up to 25 % waits for a look, more counts as another edition.

**Chapter names:** a chapter keeps only what tells it from the others. Track numbers, the book title, series and
author, and text that (nearly) every track repeats go – "01 - Book (Full-Cast Edition): Chapter One - The Worst
Birthday" becomes "Chapter 1: The Worst Birthday". When nothing sensible is left, chapters are numbered ("Chapter 1",
"Kapitel 1"). The order always comes from the file names; if the track numbers in the tags say otherwise, the book
waits for a look. **Tidy chapters** in the library applies the same rules to a book that is already there.

**Better release / correct:** every book in the library has **Better release** – Hörwerk searches again and loads
automatically only a clearly better release (better format, or clearly more bitrate); under *Requests* you can also
pick one yourself. The new download replaces the book, the old file is archived. **Correct** brings a book back for
a new assignment, and **Wrong edition** on a filed book also lets its request search on.

Tip: in SABnzbd set *Config → Switches → Unwanted extensions* (`exe, bat, cmd, scr, lnk, js, vbs, ps1, msi, com`)
to **Abort**.

## Free audiobooks (LibriVox)

[LibriVox](https://librivox.org) volunteers read books that are in the public domain – thousands in English, hundreds
in German, all free and legal to download. Hörwerk searches LibriVox (via the Internet Archive) alongside the shop
search: type a title or author, pick German or English, and the free recordings appear below the shop results. One
click loads the MP3s straight into the inbox with a `metadata.json` (title, author, reader, description, cover); the
inbox turns them into an M4B and files them like any other book. Books you already have are marked.

Only the LibriVox collection is searched and only from archive.org – nothing else can be loaded this way.

## Discover

The **Discover** tab suggests new audiobooks:

* **For you** – titles Audible listeners connect with the (up to 30 newest) books in your library. The more of your
  books point to the same title, the higher it ranks; "Goes with …" names them.
* **By genre** – all genres and subcategories as in the Audible shop (e.g. *Krimis & Thriller → Thriller*): best
  sellers, best rated and new releases. Small areas are filled up with titles rated just as well (4.0+) by fewer
  listeners, so every genre shows suggestions; subcategories without a single German audiobook (e.g. *Cricket*) are left
  out. All lists are prepared in the background once a day. The chosen genre is remembered per browser.
* **From the critics** – reviewed books that exist as an audiobook on Audible (same title, same author), with a link to
  the review. Sources: the RSS feeds "Buchkritik" and "Lesart" (Deutschlandfunk Kultur), the page
  [Die Bücher des Monats](https://www.deutschlandfunkkultur.de/rezensionen-des-monats-114.html) with its monthly
  best-of lists for non-fiction and crime (PDFs, each read once; shown with their place), "Büchermarkt"
  (Deutschlandfunk), Perlentaucher's lists
  [Hörbücher – Literatur aktuell](https://www.perlentaucher.de/buchKSL/hoerbuecher-literatur-aktuell.html) and
  [Sachbuch aktuell](https://www.perlentaucher.de/buchKSL/hoerbuecher-sachbuch-aktuell.html) (with the number of press
  reviews) and the book round-ups from Perlentaucher's feed. Only author and title are taken; reviews are remembered for
  180 days, the newest ~40 audiobooks are shown. A reviewed book without an audiobook yet stays on a waiting list for
  two years and is looked up again once a month; once the audiobook is out (or announced on Audible) it shows up at
  the top, marked "now as audiobook" or "out <date>". This runs daily in the background, even if nobody opens the page.

**Search** on a card jumps to the *Search* tab with the book already picked (title, author and ASIN help with tagging later).

**Quality rules** (*Settings → Discover*): minimum rating (default 4.3 stars) and number of ratings (50), unabridged only,
German only, no erotica. **Always excluded** is what you already have: books in the library
(even spelled differently), downloads of the last 30 days and books being processed right now. Books from the critics skip the rating rule – they are often new.

**To be honest:** no software can reliably check whether a non-fiction book is accurate – ratings only measure
popularity. So *For you* only shows non-fiction by established authors: on Hörwerk's fixed list (about 400 authors
of fact-based non-fiction since 2010, `rust/hoerwerk/src/trusted_authors.txt`), with a Perlentaucher author page (reviewed in the arts
pages), named in the reviews, or on your own list in the settings. That checks reputation, not content. All non-fiction
can be switched on in the settings. In *By genre* it appears when you pick a non-fiction genre yourself,
with a note that stars are no fact check. BookBeat is not connected – it
has no open API.

Lists are put together in the background, refreshed at most once a day (or with **Refresh**) and cached in the database.
Outbound connections: only the Audible API of the chosen store, `www.deutschlandfunkkultur.de`, `www.deutschlandfunk.de`,
`bilder.deutschlandfunk.de` (the best-of list PDFs) and `www.perlentaucher.de` (HTTPS).

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
