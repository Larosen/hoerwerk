# Notifications, accounts, configuration and security

[← README](../../README.md) · **English** · [Deutsch](../de/configuration.md)

## Notifications (ntfy)

Push messages to your phone via [ntfy](https://ntfy.sh) – ntfy.sh or your own server.

1. Install the ntfy app and subscribe to a topic, e.g. `audiobooks-1234`.
2. **⚙︎ Settings → Notifications (ntfy)**: server, topic, optionally an access token (or user + password) and
   priority. *Address of this UI* makes tapping a message open Hörwerk.
3. Tick the events you want and click **Send test message**.

| Event | Default |
|---|---|
| Audiobook filed | on |
| Audiobook needs review | on |
| Already in the library | on |
| Error with an audiobook | on |
| New book request | on |
| Book request failed | on |
| Download sent to SABnzbd | off |
| Download finished in SABnzbd | off |
| Download failed in SABnzbd | on |
| Audiobookshelf: scan failed | on |
| Plex scan failed | on |

Token and password stay on the server; they are not sent to another server if you change the address.
Messages use the language of the interface.

## Log

**Settings → Log** (`/settings/logs`) shows what happened: books found, converted, matched and filed, media server scans, warnings
and errors. Filter by level or text, follow it live, download it as a file. The same lines are in the container log
(`docker logs hoerwerk`).

## Accounts and access from outside

Without signing in Hörwerk only shows the login page. On the **first visit** you create the admin account (directly
from the home network; from outside the setup code from `docker logs hoerwerk` is needed). If you had
`AUTH_USER`/`AUTH_PASS` set, they become the admin account automatically.

* **Accounts** (*Settings → Accounts*, `/settings/users`, admins only): create family accounts (generate an initial password and pass it on), role,
  rights (*see all requests*, *no approval*, *interactive search*), monthly quota, lock, reset password, delete.
  Everyone starts on *Discover*; family accounts only see *Discover*, *Search* and *Requests* (the first time with a
  short three-step guide) – inbox, library, settings and log stay with admins. Below: the
  **admin log** (who changed what, when, with IP).
* **Invitations** (*Settings → Accounts*, like Wizarr): create a link and send it e.g. via WhatsApp. Whoever opens it
  picks their own name and password, gets a family account and is signed in right away. Per link: a note ("Mum"),
  validity (1, 7, 30 days or unlimited), how often it can be used, quota. The list shows open, used (by whom), expired
  or revoked; revoking works any time. A link never creates an admin. Links always point to the public address (*Settings → General*), also when created at home;
  if it is not set yet, the invitations section says where to enter it.
  Passwords must meet the rules (12+ characters, mixed or a sentence, no name, no “Hörwerk”) – a checklist shows what
  is missing while typing, with a pointer to password managers. Wrong links count like wrong passwords (block
  after 5 tries, `auth.log` → CrowdSec); a single-use link yields only one account even when opened at the same time.
* **Account** (your name, top right, `/account`, everyone): change password, set up a **second factor** (authenticator app such
  as Aegis, 2FAS, Google/Microsoft Authenticator), own ntfy topic, see and sign out devices, and
  **How Hörwerk sees you** (your address, through which proxy, HTTPS) – to check a reverse proxy.
  With *Admins need a second factor* (settings) admins without one only reach their account.
* "Stay signed in" lasts 30 days (extended while used), otherwise 12 hours.

**On the internet via Pangolin:**
1. Create a Pangolin resource pointing at `http://<unraid-ip>:8080` (Pangolin does HTTPS).
   Pangolin's own login can stay in front as well – safer, but you sign in twice.
2. Set `TRUSTED_PROXIES` to the address Newt/Pangolin arrives from (e.g. `172.18.0.0/16` for the Docker network or the
   IP of the Newt container). Only then the log shows the visitors' real IP.
3. Enter the public address under *Settings → General* (e.g. `https://audiobooks.example.com`) – Hörwerk only answers
   under IP addresses, home-network names and this address (protection against DNS rebinding). More names go into
   `ALLOWED_HOSTS`. In Pangolin, set the health check path to `/api/health`.
4. **CrowdSec:** every failed login is in `/config/logs/auth.log` with its IP. Parser, scenarios and examples for
   Vector or the CrowdSec agent are in [contrib/crowdsec](../../contrib/crowdsec/README.md). Hörwerk itself blocks an IP for
   15 minutes after 5 failures; CrowdSec then bans it right at the VPS.

## Configuration

Almost everything can be changed in **⚙︎ Settings** (`/settings`); the environment only provides the starting values.
Settings are a page of their own with an overview: system check, general, inbox & library, M4B conversion, indexers &
SABnzbd, Audiobookshelf & Plex, discover & requests, notifications, accounts and log – each area with its own address
(e.g. `/settings/health`), so bookmarks and the browser's back/forward work. The same goes for the tabs: `/books`,
`/search`, `/discover`, `/requests`.

**System check** (`/settings/health`, similar to Vaultwarden's diagnostics) shows at a glance:
* **Connection & proxy:** which address Hörwerk sees you as, whether the reverse proxy is in `TRUSTED_PROXIES` (else an
  error with the line to add), whether HTTPS arrives, `ALLOWED_HOSTS`, whether live updates get through the proxy,
  whether the address in notifications fits. Proxies in the own network that send `X-Forwarded-For` but are not
  trusted (e.g. Newt after a restart with a new IP) show up here and in the log.
* **Security:** admins without a second factor, sign-in log for CrowdSec, addresses blocked right now. Hörwerk blocks
  an address for 15 minutes after 5 failed logins; *Settings → Accounts → Security* lists them and **Unblock** frees
  one at once – no terminal needed. Blocks by CrowdSec or a firewall are managed there.
* **Storage:** inbox, library, config present, writable, free space.
* **Services:** indexers, SABnzbd, Audiobookshelf, Plex (tested live), ntfy, Audible.
* **System:** version, ffmpeg and sandbox, queues, clock (matters for second-factor codes). If the sandbox is not
  active, the **?** below the hint says why – kernel without Landlock, Landlock not switched on at boot (with the
  steps for Unraid's Syslinux configuration or GRUB) or Docker forbidding it – and what to do.

The "How it works" boxes on the pages can be hidden for everybody under *Settings → General*.

**Without signing in** there is only the login page and what it needs: no app page, no app code, no API (except signing
in and `/api/health`, which without login only says `{"ok": true}` – no version). A bookmark to a sub-page leads to the
login first and then exactly there. Settings and inbox are for admins only.
Settings, presets, history, the audiobook list, the library index and all credentials live in the SQLite database
`/config/hoerwerk.db` (permissions 600). Once a day Hörwerk writes a consistent copy to
`/config/backups/hoerwerk-<weekday>.db` (the last seven days) – to restore, stop Hörwerk and put the copy in place as
`hoerwerk.db`.

**Backup, pinning a version, going back:** `/config` is all Hörwerk needs backed up (your audiobooks live in your own
library share). Every version is its own image – `ghcr.io/larosen/hoerwerk:7.17.1` instead of `:latest` stays on that
version (it still gets the weekly security rebuild); what changed is in the [changelog](../../CHANGELOG.md). To go back, set
the older tag and start the container again; if the database was changed by the newer version, put a backup copy from
before the update in place. Books that Hörwerk replaced are not deleted but moved to `_ersetzt` in the inbox – move the
folder back into the library to restore one.

| Variable | Default | Meaning |
|---|---|---|
| `APP_LANGUAGE` | `de` | Interface and notifications: `en` or `de` |
| `PUID` / `PGID` | `99` / `100` | User/group for all file access |
| `UMASK` | `000` | Permissions of new files and folders |
| `INBOX_DIR` / `LIBRARY_DIR` | `/inbox` / `/audiobooks` | Audiobook inbox and library |
| `AUDIBLE_REGION` | `de` | Audible store: `de us uk fr it es ca au in jp` |
| `MATCH_MODE` | `auto` | `auto` (clear matches automatically), `review` (always ask), `off` (only convert) |
| `MATCH_THRESHOLD` | `0.8` | Minimum confidence for automatic filing |
| `FOLDER_TEMPLATE` | `{author}/[{series}/][{series_position:2} - ]{title}` | Folder in the library |
| `FILE_TEMPLATE` | `{title}` | File name of the M4B |
| `CHAPTERS` | `auto` | Chapter names from Audible: `auto`, `audible`, `files` |
| `COVER_SOURCE` | `audible` | `audible` (store, full resolution) or `local` |
| `INBOX_ORIGINALS` | `archive` | Originals afterwards: `archive` (to `_fertig`) or `delete` |
| `SETTLE_SECONDS` | `120` | How long an inbox folder must stay unchanged |
| `SCAN_INTERVAL` | `60` | Seconds between two inbox checks |
| `METADATA_FALLBACK` | `true` | Ask Audnexus, Apple Books and Open Library when Audible finds nothing |
| `WRITE_ABS_METADATA` | `true` | Write `metadata.json` for Audiobookshelf |
| `DUPLICATES` | `ask` | Book already in the library: `ask`, `skip`, `keep` |
| `LIBRARY_RESCAN_HOURS` | `6` | How often the library is rescanned |
| `LIBRARY_ORIGINALS` | `archive` | Old single files after “To M4B”: `archive` (to `_ersetzt`) or `delete` |
| `NARRATOR_IN_ARTIST` | `false` | Narrator also in the artist tag |
| `KEEP_SERIES_IN_TITLE` | `true` | Keep “…, Band 3” in the title |
| `WRITE_DESC` / `WRITE_READER` | `false` | Write `desc.txt`, `reader.txt` (only needed without `metadata.json`) |
| `WRITE_COVER` | `true` | Write `cover.jpg` (used by Audiobookshelf and Plex) |
| `DISCOVER_MIN_RATING` / `DISCOVER_MIN_COUNT` | `4.3` / `50` | Discover: minimum rating and number of ratings |
| `DISCOVER_UNABRIDGED` / `DISCOVER_GERMAN` | `true` | Discover: unabridged only / German only |
| `DISCOVER_NONFICTION_STORE` | `false` | Discover: all non-fiction in "For you" (otherwise only by established authors) |
| `DISCOVER_TRUSTED_AUTHORS` | – | Discover: more established non-fiction authors, comma-separated |
| `WANTED_INTERVAL_HOURS` | `12` | Requests without a match: hours until the next search |
| `ADMIN_TOTP_REQUIRED` | `false` | Admins need a second factor |
| `CPU_CORES` | half the cores | Parallel ffmpeg processes per book |
| `ENCODE_WORKERS` | `1` | Books encoded at the same time |
| `IO_WORKERS` | `2` | Parallel matching/tagging/filing steps |
| `NICE` | `10` | CPU priority of ffmpeg (0 = normal) |
| `FFMPEG_SANDBOX` | `auto` | ffmpeg in a Landlock sandbox: `auto`, `require`, `off` |
| `FFMPEG_MEMORY_MB` | `4096` | Memory limit per ffmpeg process |
| `AUDIO_QUALITY` | `high` | `standard`, `high` or `original` – how much bitrate readings and radio plays keep |
| `AAC_BITRATE` | `auto` | `auto` (like the source, capped by `AUDIO_QUALITY`) or a fixed value |
| `FORCE_MONO` | `false` | Output mono |
| `CHAPTER_TITLES` | `auto` | `auto`, `filename` or `tag` |
| `TRUSTED_PROXIES` | – | IPs/networks of the reverse proxy (Pangolin/Newt, Traefik), e.g. `172.18.0.0/16` – only from there `X-Forwarded-For` counts |
| `COOKIE_SECURE` | `auto` | Login cookie only over HTTPS: `auto` (when the proxy reports HTTPS), `true`, `false` |
| `REQUEST_QUOTA` | `5` | Book requests per user and month (adjustable per account) |
| `AUTH_USER` / `AUTH_PASS` | – | Only for upgrading: becomes the admin account on first start |
| `ALLOWED_HOSTS` | – | Names Hörwerk answers under (protection against DNS rebinding). Empty: IP addresses, `localhost`, home-network names (`tower`, `*.local`, `*.lan`, `*.fritz.box` …) and the public address from the settings; through a proxy in `TRUSTED_PROXIES` every name the proxy forwards. Set: exactly these names, e.g. `audiobooks.example.com,*.example.com` (IPs and `localhost` always work). `*` = any name as before 7.13 (not recommended). Invalid values (with `https://` or a path) stop the start with a message. `/api/health` answers under any name – for the Docker and Pangolin health checks |

SABnzbd, the optional indexer, Audiobookshelf, Plex and ntfy are set up in the interface, not via variables.

## Security

Short version – details, tests and component versions in [SECURITY.md](../../SECURITY.md):

* Accounts with login; on the internet only behind a reverse proxy with HTTPS (e.g. Pangolin) and with
  `TRUSTED_PROXIES`. Failed logins are in `/config/logs/auth.log` with their IP – ready for CrowdSec.
* Mount only the folders you need; the container runs without Linux capabilities and with `no-new-privileges`.
* Downloaded files are never executed. ffmpeg is an own minimal build (audio only, no network code) running in a
  Landlock sandbox that can only read the book's files.
* API keys, tokens and passwords (SABnzbd, indexer, Audiobookshelf, Plex, ntfy) stay on the server and are never sent
  to the browser.
