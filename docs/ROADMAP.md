# Roadmap

Where Hörwerk is heading. The goal: the one place to **get, request and discover** audiobooks for a household –
with every book ending up as a clean, tagged M4B. Ideas and votes are welcome as issues.

## Recently done

- **7.14** – the server is a single Rust program (smaller image, faster, no interpreter); same database as before.
- **Free audiobooks from LibriVox** in the search (German and English), one click into the inbox.
- **Preferred indexer**: asked first, the others only when it has nothing that fits.

## Next

| | What | Why |
|---|---|---|
| 1 | **Follow authors and series** | "Tell me / request automatically when a new volume of this series or a new book by this author appears on Audible." Today you have to look yourself. |
| 2 | **Prowlarr sync** | Take indexers (and their categories) from Prowlarr instead of entering them twice. |
| 3 | **More free and legal sources** | ARD Audiothek (public-broadcaster audio dramas and readings, downloadable while online), Vorleser.net (classics read by professional speakers, free for private use), more public-domain collections of the Internet Archive with a clear licence. Each source only if its terms allow downloading. |
| 4 | **Versioned releases** | `v7.14.0`-style tags with release notes, so you can pin a version instead of `latest`. |
| 5 | **arm64 image** | Built on a native arm64 runner (Raspberry Pi, Apple-silicon NAS). |

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
