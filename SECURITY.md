# Security

**English** · [Deutsch](SECURITY.de.md)

As of the security review of 30 Oct 2026 (version 4.1: ntfy notifications, English/German). Hörwerk processes
files from foreign sources (downloads via SABnzbd), queries external services (Audible, Audnexus, BookBeat, Thalia,
optionally an indexer, SABnzbd, Audiobookshelf, ntfy) and has a web interface with write access to your shares.
This page explains what the program protects against, what was tested and what you should do yourself.

## Recommended settings

1. **Accounts:** the admin account is created on first visit; without signing in there is only the login page.
   Expose it to the internet only behind a reverse proxy with HTTPS (e.g. **Pangolin**) whose address is in
   `TRUSTED_PROXIES` – otherwise Hörwerk doesn't see the visitors' real IP. Failed logins go to `/config/logs/auth.log`
   (for CrowdSec, see [contrib/crowdsec](contrib/crowdsec/README.md)).
2. **Run the container with minimal rights** – the Unraid template and `docker-compose.yml` already do:
   `--security-opt no-new-privileges:true --cap-drop ALL --cap-add CHOWN --cap-add SETUID --cap-add SETGID`.
3. **Mount only the folders you need:** the inbox and the library, never all of `/mnt/user/`.
4. **In SABnzbd** set *Config → Switches → Unwanted extensions* (`exe, bat, cmd, scr, lnk, js, vbs, ps1, msi, com`)
   to **Abort**. Fake releases are then dropped before they reach the inbox.
5. Set inbox originals to **delete** (`INBOX_ORIGINALS=delete`) or empty `_fertig` regularly.
6. Enter the public address under *Settings → General*. From 7.13 on Hörwerk only answers under IP addresses,
   home-network names and this address by itself (protection against DNS rebinding); more names via `ALLOWED_HOSTS`,
   e.g. `ALLOWED_HOSTS=audiobooks.example.com`.
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
| Discover | Fixed addresses over HTTPS only: the Audible API, `www.deutschlandfunkkultur.de`, `www.deutschlandfunk.de`, `bilder.deutschlandfunk.de` and `www.perlentaucher.de` (RSS or a list page, max. 8 MB, no redirects elsewhere, feeds with a DTD/entities are rejected; HTML is only scanned with fixed patterns for author and title; the monthly best-of list PDFs are read by a small built-in text reader that only unpacks FlateDecode streams, with size limits per stream and in total, skips images and executes nothing). Review links are only shown if they are `https://`. Suggestions never trigger a download by themselves. |
| Further metadata sources | Only fixed addresses over HTTPS: `api.audnex.us`, `itunes.apple.com`, `openlibrary.org` (plus Audible, BookBeat, Thalia). Matches from these are never filed automatically. |
| Stealing API keys | Keys live only in the database `/config/hoerwerk.db` (permissions 600), are never sent to the browser, nor are NZB links (they contain the indexer key). If the address of an indexer or SABnzbd changes, the stored key is **not** carried over – it has to be entered again. Error messages are scrubbed of keys. |
| Download folder name colliding with internal folders (`.work`) | The name passed to SABnzbd is cleaned (no paths, no hidden names). |
| TLS | Certificates are always verified (Python default). SABnzbd/indexer/ntfy in the LAN may use `http` – their keys then travel unencrypted through your home network. |

### 3. Web interface / API

| Attack | Protection |
|---|---|
| **CSRF**: a foreign web page in the same browser sends requests to `http://tower:8080` (e.g. changing settings or taking books out of the library) | Every changing request needs a custom header that browsers only send after a CORS approval (never given); additionally `Origin` and `Sec-Fetch-Site` are checked. Tested. |
| Clickjacking | `X-Frame-Options: DENY`, `frame-ancestors 'none'`. |
| Access without signing in | Without a valid session there is only the login page, the UI code (`/static`, open source anyway), `/i18n.js`, the logo and `/api/health`. Every API route has an access level (public / user / admin, default admin), checked on the server – not just hidden. Tested: signed out 401, family account on admin functions 403 (logged). |
| First setup by strangers | While there is no account, an admin can only be created from the home network directly; from outside the setup code from the container log is needed. Afterwards setup is closed. |
| Password guessing | scrypt (N=2¹⁵); password rules (same on server and in the browser): at least 12 characters, three of four character kinds or a sentence of 20+ characters, not your own name, not “Hörwerk”/“password”/“123456”, not a single word with numbers around it (“Pudding2024!”), not the same characters over and over; per IP (for IPv6 per /64 network – one connection gets a whole /64) at most 5 failures in 15 minutes, then 429 – a flood of new addresses pushes out single tries first, never existing blocks; per account a growing delay (a stranger can't lock out a family account); unknown names take as long as known ones. Every failure with its IP goes to `auth.log` → CrowdSec bans at the VPS. |
| Stolen session | 256-bit token, stored only as SHA-256; cookie `HttpOnly`, `SameSite=Lax`, `Secure` behind HTTPS. Password change, locking an account and "sign out all other devices" end sessions immediately. |
| Forged IP in `X-Forwarded-For` | The header only counts from addresses in `TRUSTED_PROXIES` and is read from the right – made-up addresses prepended by a client have no effect. |
| Privilege escalation | Only an admin changes roles and rights (server check); the last admin can't be demoted, locked or deleted. |
| Viewing or changing other people's requests (IDOR) | Every request route checks whether the account may see it; foreign and non-existent ids answer the same 404 – numbers can't be probed. Owner and status are set by the server only (fields from the browser are ignored). Tested. |
| Getting around the quota | Check and insert in one database transaction under a lock – parallel requests can't exceed the limit either. |
| Family account downloads anything | Users only request books; Hörwerk picks releases by fixed rules. The interactive search (releases by hand) needs its own right, indexer links (containing the API key) never reach the browser. |
| Stolen password | Optional second factor (TOTP per RFC 6238, standard library): each code works once (no replay), ±30 s clock drift, sign-in ticket 5 minutes / 5 tries, failures count for the IP brake and go to `auth.log`. Can be required for admins. |
| Unnoticed changes through an admin login | Admin log in the database: every changing admin action with time, account and IP (approvals, accounts, settings, sources …), visible under *Accounts*. |
| DNS rebinding / foreign host names | By default only IPs, home-network names, the public address and what a proxy in `TRUSTED_PROXIES` forwards (never learned from Host/X-Forwarded-Host), otherwise 421; `ALLOWED_HOSTS` for more names; besides, every data request needs a login. |
| Paths outside the library | Book paths from the browser are resolved (incl. symlinks) and must lie inside the library; static files only from `app/static`. |
| Huge requests | max. 4 MB per request; the only exception is the upload (up to 4 GB per file, streamed straight to disk). |
| Upload | Admins only; login and rights are checked before a single byte is read. Only relative paths without `..` and without hidden names, only audio, covers, cue sheets and Hörwerk's text files – judged by **content**: the first bytes must match the extension (audio signatures such as ID3/MPEG/FLAC/Ogg/MP4, image signatures, text without binary data). Programs (`.exe`, ELF, macOS), PDFs, Office documents, archives (ZIP/RAR/7z …) and scripts are recognised even when renamed and refused before anything lands on disk; free space checked before each file; first a hidden staging folder, then one step into the inbox (nothing is overwritten). Limits: 8 uploads at once (3 per account), 64 GB and 3000 files per upload, 128 GB per account in open uploads – each file books its size before writing, so parallel uploads cannot slip past. An upload belongs to the account that started it; finishing happens once, never while files are still arriving, and is completed after a crash (marker file) – unfinished staging folders are removed at the next start. |
| Many open live connections (`/api/events`) | At most 32 at the same time; they deliver only what the interface fetches anyway and need the same login. |
| Language setting / translation catalog | Only `de` or `en` are accepted; `/i18n.js` serves a fixed file from the image (also for the login page). |
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

## Build and supply chain

Every push and pull request runs: ruff (errors, likely bugs, security rules), pip-audit (Python dependencies),
npm audit (build tools), the unit tests with a coverage limit (80 %), the click-through tests against the server
and against the finished image, and Trivy on the image – a critical or high vulnerability with an available fix
stops the release (exceptions only in `.trivyignore`, each with its reason). Only then the image is published:
signed with cosign (keyless) and with SBOM and build provenance attached to its digest. All GitHub Actions are
pinned to commit SHAs; Dependabot proposes updates weekly. Checking an image:

```sh
cosign verify ghcr.io/larosen/hoerwerk@sha256:<digest> \
  --certificate-identity-regexp 'https://github.com/Larosen/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
gh attestation verify oci://ghcr.io/larosen/hoerwerk@sha256:<digest> --owner Larosen
```

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
