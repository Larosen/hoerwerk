# Security

**English** · [Deutsch](SECURITY.de.md)

As of the security review of 30 Oct 2026 (version 4.1: ntfy notifications, English/German). Hörwerk processes
files from foreign sources (downloads via SABnzbd), queries external services (Audible, Audnexus, BookBeat, Thalia,
optionally an indexer, SABnzbd, Audiobookshelf, ntfy) and has a web interface with write access to your shares.
This page explains what the program protects against, what was tested and what you should do yourself.

## Recommended settings

1. **Set a password** (`AUTH_USER` / `AUTH_PASS`) as soon as more than just you is on the network, and never expose the
   interface to the internet without a reverse proxy with HTTPS. Without a password the server only answers requests
   from private networks (LAN, VPN, Docker).
2. **Run the container with minimal rights** – the Unraid template and `docker-compose.yml` already do:
   `--security-opt no-new-privileges:true --cap-drop ALL --cap-add CHOWN --cap-add SETUID --cap-add SETGID`.
3. **Mount only the folders you need:** the inbox and the library, never all of `/mnt/user/`.
4. **In SABnzbd** set *Config → Switches → Unwanted extensions* (`exe, bat, cmd, scr, lnk, js, vbs, ps1, msi, com`)
   to **Abort**. Fake releases are then dropped before they reach the inbox.
5. Set inbox originals to **delete** (`INBOX_ORIGINALS=delete`) or empty `_fertig` regularly.
6. Optionally set `ALLOWED_HOSTS=tower,192.168.1.10` (protection against DNS rebinding without a password).
7. Keep the image up to date: GitHub Actions rebuilds it every week (Debian security updates); new ffmpeg versions
   arrive by updating `FFMPEG_VERSION` in the Dockerfile; on Unraid use “Check for Updates” now and then.

## Threats and protections

### 1. Malicious files in a download

| Attack | Protection |
|---|---|
| A file with an audio extension that is really a playlist (HLS) or concat script and makes ffmpeg read local files (e.g. `/config/hoerwerk.db`) or URLs | ffmpeg/ffprobe read downloaded files only with `-protocol_whitelist file` and a `-format_whitelist` of real audio demuxers (mp3, mov/m4a/m4b, aac, flac, ogg, wav, aiff, asf, matroska, ape, wv …). Playlists, concat, image sequences and network protocols are blocked. Tested (`test_security.py`). |
| A bug in ffmpeg | **Own ffmpeg 9 build** with only the audio formats needed, no network code at all, no third-party libraries, hardened (PIE, RELRO, stack protector). Every ffmpeg/ffprobe call additionally runs in a **Landlock sandbox**: it may read only the files of this call, write only into the work directory, open no TCP connections; plus `no_new_privs`, memory/file-size limits, no core dumps. Even an exploited bug cannot reach `/config` (API keys) or the library. Tested (`test_sandbox.py`); CI runs the whole test suite with exactly this ffmpeg and a mandatory sandbox. Hanging processes are killed after 6 h. |
| Symlinks in a download (`cover.jpg → /config/hoerwerk.db`, folder link to `/`) | Symlinks are ignored completely when reading (audio, cover, `chapters.txt`, `metadata.yml`, subfolders, inbox entries) and reported as suspicious in the log. |
| `.exe`, `.bat`, `.lnk`, scripts etc. (typical of fake releases) | Never executed and never taken into the library – only the created M4B, `cover.jpg`, `desc.txt`, `reader.txt`, `metadata.json` end up there. The finding is shown as ⚠ in the book's log. |
| Path tricks in tags/names (`../../etc`, `/`, `:`) | Library paths are built only from cleaned parts (no separators, no `..`, no control characters, max. 180 bytes); tested. |
| `metadata.yml` bomb (YAML anchors) or huge text files | `yaml.safe_load`, anchors/aliases are refused, `metadata.yml`/`chapters.txt` max. 512 KB. |
| Prepared tags (script code in the title) | The interface sets foreign data only as text (no `innerHTML` with foreign data); plus a Content Security Policy without inline scripts. |
| Huge covers | Images > 20 MB in the folder are ignored; store covers max. 15 MB and only real JPEG/PNG. |

### 2. Network requests

| Attack | Protection |
|---|---|
| Store link or redirect to internal addresses (router, NAS, `169.254.169.254`) – SSRF | Links are fetched only for Audible, BookBeat and Thalia; every redirect must lead back there. Covers are loaded only from public addresses (DNS resolution, private/loopback/link-local addresses are refused). Only `http(s)`. |
| Very large responses (memory) | Responses max. 8 MB (stores/Audible) or 20 MB (indexer). |
| Indexer XML with DTD/entities (billion laughs, XXE) | Responses containing `<!DOCTYPE`/`<!ENTITY` are refused; Python's XML parser does not load external entities anyway. |
| `javascript:` links from foreign data (indexer details, store pages) | The server only passes `http(s)` links, the interface filters `href`/`src` centrally as well. |
| ntfy token/password | Only on the server, never sent to the browser (the interface only sees “saved”), not carried over when the server address changes. Redirects are not followed. Address must be `http(s)` without `user:password@`, topic only `A–Z a–z 0–9 - _`. Messages are sent as JSON (no header injection through book titles); the “open” link is only the `http(s)` address you entered. |
| Plex token | Like the Audiobookshelf token: only on the server, never sent to the browser, not carried over when the address changes, redirects not followed, no `user:password@` in the address. Hörwerk only asks Plex to refresh and (if ticked) to empty the trash of the one chosen library. |
| Audiobookshelf token | Like the indexer keys: only on the server, never sent to the browser, not carried over when the address changes. Redirects are not followed so the token never ends up at another server. |
| Further metadata sources | Only fixed addresses over HTTPS: `api.audnex.us`, `itunes.apple.com`, `openlibrary.org` (plus Audible, BookBeat, Thalia). Matches from these are never filed automatically. |
| Stealing API keys | Keys live only in the database `/config/hoerwerk.db` (permissions 600), are never sent to the browser, nor are NZB links (they contain the indexer key). If the address of an indexer or SABnzbd changes, the stored key is **not** carried over – it has to be entered again. Error messages are scrubbed of keys. |
| Download folder name colliding with internal folders (`.work`) | The name passed to SABnzbd is cleaned (no paths, no hidden names). |
| TLS | Certificates are always verified (Python default). SABnzbd/indexer/ntfy in the LAN may use `http` – their keys then travel unencrypted through your home network. |

### 3. Web interface / API

| Attack | Protection |
|---|---|
| **CSRF**: a foreign web page in the same browser sends requests to `http://tower:8080` (e.g. changing settings or taking books out of the library) | Every changing request needs a custom header that browsers only send after a CORS approval (never given); additionally `Origin` and `Sec-Fetch-Site` are checked. Tested. |
| Clickjacking | `X-Frame-Options: DENY`, `frame-ancestors 'none'`. |
| Accidentally exposed to the internet | Without a password only private addresses are answered (LAN, VPN/Tailscale, Docker). If devices in your home network connect via a public IPv6 address, set a password or `ALLOW_PUBLIC=true`. |
| Password guessing | Constant-time comparison, 1 s pause after every failed attempt. |
| DNS rebinding | Optional `ALLOWED_HOSTS`; with a password it has no effect anyway. |
| Paths outside the library | Book paths from the browser are resolved (incl. symlinks) and must lie inside the library; static files only from `app/static`. |
| Huge requests | max. 4 MB per request. |
| Many open live connections (`/api/events`) | At most 32 at the same time; they deliver only what the interface fetches anyway and need the same login. |
| Language setting / translation catalog | Only `de` or `en` are accepted; `/i18n.js` serves a fixed file from the image and needs the same login as the page. |
| Other headers | `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Cross-Origin-Opener-Policy`/`-Resource-Policy: same-origin`, `Cache-Control: no-store`. |

### 4. Container

* The app runs as PUID/PGID (default 99:100) **without Linux capabilities**, `no-new-privileges` active (checked via
  `/proc/<pid>/status`). Root is only needed in the entrypoint to create the user and `chown /config`.
* Dependencies: only `mutagen` and `PyYAML` (pinned), otherwise the Python standard library + own ffmpeg (source from
  the official FFmpeg repository, release tag pinned to its commit).
* ffmpeg sandbox: `FFMPEG_SANDBOX=auto` (default) uses Landlock when kernel and Docker allow it (Linux ≥ 5.13, network
  rules from 6.7; Docker ≥ 23). Whether it is active is shown at the bottom of ⚙︎ Settings. `require` refuses ffmpeg
  without the sandbox, `off` switches it off.
* Weekly rebuild without cache so security updates arrive.

## Component versions (checked 30 Oct 2026)

| Component | Version | Latest | Assessment |
|---|---|---|---|
| Python (Docker base) | 3.15.2 (`python:3.15-slim-trixie`) | 3.15.2 | current |
| Debian | 13 “trixie” (stable) | 13 | current, security updates via weekly rebuild |
| ffmpeg | 9.0.2 (own minimal build) | 9.0.2 | current, see below |
| tini | 0.19.0 (Debian) | 0.19.0 | current |
| mutagen | 1.48.1 | 1.48.1 | current, no known vulnerabilities (OSV) |
| PyYAML | 6.0.3 | 6.0.3 | current, no known vulnerabilities (OSV); without C acceleration on Python 3.15 (irrelevant for small `metadata.yml`) |
| Node.js (UI build only) | 26 LTS | 26 | current, not part of the final image |
| TypeScript / Playwright | 7.0.2 / 1.64.1 | same | current (development/CI only) |
| GitHub Actions | checkout v7, setup-python v7, setup-node v7, docker/* v4/v6/v7 | same | current |

Checked interfaces: Audiobookshelf 2.37 (`metadata.json`, library scan), SABnzbd 5.1 (`addurl`, `queue`, `history`,
`get_cats`), ntfy 2.35 (JSON publish, token/basic auth).

**ffmpeg:** Up to version 3, ffmpeg 7.1.5 came from Debian stable; OSV listed 40 open CVEs for it (almost all in parts
Hörwerk does not use). Since version 4 the image builds ffmpeg 9.0.2 itself (`docker/build-ffmpeg.sh`): only the
audio containers and decoders that occur, AAC as the only encoder; ffprobe/ffmpeg may only open the audio containers
(whitelist). Video decoders, filters other than resampling, streaming protocols (HLS, DASH, RTSP …), devices
and third-party libraries are not in the program at all – most ffmpeg CVEs affect exactly these parts. Because the
package no longer comes from Debian's security maintenance, the version in the Dockerfile must be raised for ffmpeg
security updates (tag and commit, CI checks both).

## Known residual risks

* **0-day in ffmpeg or mutagen**: strongly contained by the measures above (ffmpeg in a sandbox) but not impossible –
  the container has write access to the mounted shares. Hence: mount only the folders you need.
* **No HTTPS in the container**: the Basic Auth password travels in clear text in the LAN. For access from outside put a
  reverse proxy with TLS in front (e.g. Nginx Proxy Manager, SWAG).
* **Disk space**: very large or many downloads can fill the inbox; there is no quota.
* The public-address check for covers resolves DNS before fetching; an attacker controlling a store's DNS record could
  theoretically switch between check and fetch (very unlikely).

## Reporting security issues

Please report them privately as a security advisory in the repository, not publicly.
