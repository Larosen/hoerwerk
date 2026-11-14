# Hörwerk

**From CD rip to finished audiobook in Audiobookshelf or Plex – in one pass, with clean metadata and reliable handling of German umlauts.**

[![Build](https://github.com/Larosen/hoerwerk/actions/workflows/docker.yml/badge.svg)](https://github.com/Larosen/hoerwerk/actions/workflows/docker.yml)
[![Release](https://img.shields.io/github/v/release/Larosen/hoerwerk)](CHANGELOG.md)
[![License: PolyForm Noncommercial](https://img.shields.io/badge/license-PolyForm%20Noncommercial-blue)](LICENSE)

**English** · [Deutsch](README.de.md)

![A folder of MP3s lands in the inbox and becomes one M4B in the library](docs/screenshots/en/demo.gif)

## Why Hörwerk?

My audiobook collection was disorganised. CD rips sat in `CD1`/`CD2`/`CD3` subfolders, MP3s were named `Track 01`,
`Track 02`, and metadata was missing or incomplete. Covers, titles, authors and descriptions were wrong or absent in
Plex and Audiobookshelf.

Over the years I assembled a workflow from several tools to achieve uniform metadata, a clean folder structure and
correct chapter names. Since my family listens to a lot of audiobooks, this became time-consuming: merging with one
tool, tagging with another and researching metadata manually took an entire evening per book. In addition, many tools
handle German audiobooks poorly, because umlauts and author names with special characters cause errors.

Hörwerk is aimed at self-hosters who keep their own audiobook collection on a NAS or server and want to automate this
process.

## What it does

- **Inbox → library, fully automatic.** Drop a folder (MP3, M4A, FLAC, `CD1`/`CD2` …) and get **one M4B with chapters**,
  filed as `Author/Series/03 - Title`, with cover, narrator, series and description from Audible.
- **German titles and names done right.** Umlauts and special characters in titles and author names are
  recognised in every spelling (`ä`, `ae`, `a` …), so German audiobooks are matched reliably.
- **Requests for the whole family.** Family accounts discover and request books; you approve, Hörwerk fetches them and
  shows download and conversion progress. **Follow** an author or narrator and every new book is requested by itself –
  or pick from all their books at once.
- **A search that understands audiobooks.** It judges every release before loading it: right book, right language,
  unabridged, and the *right recording* – not the single-narrator reading when you asked for the full cast.
- **Free audiobooks built in.** LibriVox (public domain) in English and German, one click into the inbox.
- **A tidy library.** Spots duplicates, converts old MP3 books, cleans chapter names, finds a better release for a
  book you already have – and never deletes the old copy without a verified new one.
- **Recommendations.** *Discover* learns from each user's listening history in Audiobookshelf or Plex and suggests
  matching books, and it picks up tips from critics (Perlentaucher, Deutschlandfunk Kultur).
- **Three colour schemes, and an app on the home screen.** Light (paper), dark (evening) and OLED (true black); on the iPhone and iPad
  Hörwerk runs full screen like an app ([how to](docs/en/iphone-app.md)).
- **Small and locked down.** One Rust program, its own minimal ffmpeg in a sandbox, two-factor login, signed images.

| Requests with live progress | Discover |
|---|---|
| ![Requests](docs/screenshots/en/requests.png) | ![Discover](docs/screenshots/en/discover.png) |

![Library](docs/screenshots/en/library.png)

## Quick start

**Unraid:** *Docker → Add Container*, template
`https://raw.githubusercontent.com/Larosen/hoerwerk/main/unraid/hoerwerk.xml`, set the inbox and library folders,
open `http://<unraid-ip>:8080` and create the admin account.

**docker compose:**

```bash
curl -O https://raw.githubusercontent.com/Larosen/hoerwerk/main/docker-compose.yml
# set the two folder paths and PUID/PGID (see `id`), then:
docker compose up -d
```

Image: `ghcr.io/larosen/hoerwerk:latest` (or a fixed version such as `:8.0.0`) – amd64, rebuilt every week for
security updates, signed with cosign.

## What it is not

- **No player.** Hörwerk fills the library; you listen with Audiobookshelf, Plex or any app that plays M4B.
- **No source of books.** It works with *your* files, *your* SABnzbd and *your* indexers. The only built-in source
  is LibriVox. It removes no copy protection.
- **amd64 only** for now; arm64 is on the [roadmap](docs/ROADMAP.md).

## Documentation

| | |
|---|---|
| [Audiobooks](docs/en/guide.md) | inbox, upload, matching, duplicates, chapters, editions, Audiobookshelf and Plex |
| [Downloads, discover, requests](docs/en/downloads.md) | SABnzbd and indexers, LibriVox, suggestions, family requests |
| [Configuration](docs/en/configuration.md) | accounts, access from outside, notifications, variables, backup |
| [Hörwerk as an app](docs/en/iphone-app.md) | put it on the iPhone and iPad home screen, step by step |
| [Security](SECURITY.md) | what Hörwerk protects against and how that is tested |
| [Changelog](CHANGELOG.md) · [Roadmap](docs/ROADMAP.md) · [Development](docs/en/development.md) | |

> Hörwerk organises audio you are entitled to use – your own purchases, your own recordings or public-domain works.
> It ships no media and is not affiliated with Audible, Amazon, Apple, LibriVox or any other service named here.
> The books in the screenshots are public-domain classics with made-up covers.

## How it came about

Hörwerk was built with Claude Opus 5.5 and Claude Fable 5.2. The goal was a tool that automates the entire process,
handles German audiobooks reliably and saves as much manual work as possible.

## Contributing

Found a bug, missing a feature, or know a good source of audiobook recommendations? Please open an
[issue](https://github.com/Larosen/hoerwerk/issues). Suggestions are welcome, and suitable sources will be added where
possible.

## License

Hörwerk is released under the [PolyForm Noncommercial License 1.0.0](LICENSE): free for personal and other
noncommercial use – for your own library, your family, a hobby project, research or a charity. You may share and
change it under the same terms. Commercial use is not covered; if you have one in mind, please get in touch via an
issue. This makes Hörwerk *source-available*, not open source in the OSI sense.

The Docker image also contains FFmpeg, built from its official source without GPL parts and therefore under the
[LGPL 2.1](https://www.ffmpeg.org/legal.html); the build script is [docker/build-ffmpeg.sh](docker/build-ffmpeg.sh).
The licenses of all Rust libraries are listed in [THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).

Inspired by [auto-m4b](https://github.com/seanap/auto-m4b) and [beets-audible](https://github.com/Neurrone/beets-audible);
chapter data via [Audnexus](https://github.com/laxamentumtech/audnexus).
