# Benachrichtigungen, Konten, Konfiguration und Sicherheit

[← README](../../README.de.md) · [English](../en/configuration.md) · **Deutsch**

## Benachrichtigungen (ntfy)

Push-Nachrichten aufs Handy über [ntfy](https://ntfy.sh) – ntfy.sh oder ein eigener Server.

1. ntfy-App installieren und ein Topic abonnieren, z. B. `hoerbuecher-1234`.
2. **⚙︎ Einstellungen → Benachrichtigungen (ntfy)**: Server, Topic, optional Access-Token (oder Benutzer + Passwort)
   und Priorität. Mit *Adresse dieser Oberfläche* öffnet ein Tippen auf die Nachricht Hörwerk.
3. Gewünschte Ereignisse anhaken und **Testnachricht senden**.

| Ereignis | Standard |
|---|---|
| Hörbuch abgelegt | an |
| Hörbuch braucht Prüfung | an |
| Schon in der Bibliothek | an |
| Fehler bei einem Hörbuch | an |
| Neue Buchanfrage | an |
| Buchanfrage fehlgeschlagen | an |
| Download an SABnzbd geschickt | aus |
| Download in SABnzbd fertig | aus |
| Download in SABnzbd fehlgeschlagen | an |
| Audiobookshelf: Einlesen fehlgeschlagen | an |
| Plex: Einlesen fehlgeschlagen | an |

Token und Passwort bleiben auf dem Server und werden bei geänderter Adresse nicht an einen anderen Server geschickt.
Nachrichten kommen in der Sprache der Oberfläche.

## Protokoll

**Einstellungen → Protokoll** (`/settings/logs`) zeigt, was passiert ist: gefundene, konvertierte, erkannte und abgelegte Bücher,
Scans der Medienserver, Warnungen und Fehler. Nach Stufe oder Text filtern, live mitlesen, als Datei herunterladen.
Dieselben Zeilen stehen im Container-Protokoll (`docker logs hoerwerk`).

## Konten und Zugriff von außen

Ohne Anmeldung zeigt Hörwerk nur die Login-Seite. Beim **ersten Aufruf** legst du das Admin-Konto an (aus dem
Heimnetz direkt; von außen braucht es den Einrichtungs-Code aus `docker logs hoerwerk`). Wer bisher
`AUTH_USER`/`AUTH_PASS` gesetzt hatte, bekommt daraus automatisch das Admin-Konto.

* **Konten** (*Einstellungen → Konten*, `/settings/users`, nur für Admins): Familienkonten anlegen (Startpasswort erzeugen und weitergeben),
  Rolle, Rechte (*Alle Anfragen sehen*, *Ohne Freigabe*, *Interaktive Suche*), Monatslimit pro Monat, sperren,
  Passwort neu setzen, Zweitfaktor zurücksetzen, löschen. Alle starten auf *Entdecken*; Familienkonten sehen nur *Entdecken*, *Suchen* und *Anfragen* (beim ersten Mal mit einer
  kurzen Anleitung in drei Schritten)
  – Eingang, Bibliothek, Einstellungen und Protokoll bleiben Admins vorbehalten. Darunter: das
  **Admin-Protokoll** (wer hat wann was geändert, mit IP).
* **Einladungen** (*Einstellungen → Konten*, wie bei Wizarr): einen Link erzeugen und z. B. per WhatsApp verschicken.
  Wer ihn öffnet, wählt selbst Name und Passwort, bekommt ein Familienkonto und ist sofort angemeldet. Je Link:
  Notiz („Mama“), Gültigkeit (1, 7, 30 Tage oder unbegrenzt), wie oft nutzbar, Monatslimit. Die Liste zeigt offen,
  benutzt (von wem), abgelaufen oder widerrufen; Widerrufen geht jederzeit. Über einen Link entsteht nie ein Admin.
  Die Links zeigen immer auf die öffentliche Adresse (*Einstellungen → Allgemein*), auch wenn du sie zu Hause
  erstellst; fehlt sie noch, sagt der Bereich Einladungen, wo man sie einträgt.
  Passwörter müssen den Regeln genügen (mind. 12 Zeichen, gemischt oder ein Satz, kein Name, kein „Hörwerk“) – eine
  Checkliste zeigt beim Tippen, was fehlt, mit Hinweis auf Passwort-Manager. Falsche Links zählen wie falsche Passwörter (Sperre nach 5 Versuchen, `auth.log` → CrowdSec), ein einmal nutzbarer
  Link kann auch bei gleichzeitigem Öffnen nur ein Konto ergeben.
* **Konto** (Name oben rechts, `/account`, für alle): Passwort ändern, **Zweitfaktor** (Authenticator-App wie Aegis, 2FAS,
  Google/Microsoft Authenticator) einrichten, eigenes ntfy-Topic, angemeldete Geräte sehen und abmelden,
  und **So sieht dich Hörwerk** (deine Adresse, über welchen Proxy, HTTPS) – zum Prüfen eines Reverse-Proxys.
  Mit *Admins brauchen einen Zweitfaktor* (Einstellungen) kommen Admins ohne Zweitfaktor nur noch an ihr Konto.
* „Angemeldet bleiben“ hält 30 Tage (verlängert sich bei Nutzung), sonst 12 Stunden.

**Über Pangolin ins Internet:**
1. In Pangolin eine Ressource auf `http://<unraid-ip>:8080` anlegen (HTTPS macht Pangolin).
   Pangolins eigene Anmeldung kann zusätzlich davor bleiben – sicherer, aber zweimal anmelden.
2. `TRUSTED_PROXIES` auf die Adresse setzen, von der Newt/Pangolin bei Hörwerk ankommt (z. B. `172.18.0.0/16`
   für das Docker-Netz oder die IP des Newt-Containers). Nur dann steht im Protokoll die echte IP der Besucher.
3. Die öffentliche Adresse unter *Einstellungen → Allgemein* eintragen (z. B. `https://hoerbuecher.example.de`) –
   Hörwerk antwortet nur unter IP-Adressen, Namen aus dem Heimnetz und dieser Adresse (Schutz gegen DNS-Rebinding).
   Weitere Namen gehen über `ALLOWED_HOSTS`. In Pangolin als Pfad des Health Checks `/api/health` eintragen.
4. **CrowdSec:** Jeder Fehlversuch steht mit IP in `/config/logs/auth.log`. Parser, Szenarien und Beispiele für
   Vector bzw. den CrowdSec-Agenten liegen in [contrib/crowdsec](../../contrib/crowdsec/README.md). Hörwerk selbst sperrt
   eine IP nach 5 Fehlversuchen für 15 Minuten; CrowdSec sperrt sie dann schon am VPS.

## Konfiguration

Fast alles lässt sich unter **⚙︎ Einstellungen** (`/settings`) ändern; die Umgebungsvariablen liefern nur die
Startwerte. Die Einstellungen sind eine eigene Seite mit Übersicht: Systemprüfung, Allgemein, Eingang & Bibliothek,
M4B-Konvertierung, Indexer & SABnzbd, Audiobookshelf & Plex, Entdecken & Anfragen, Benachrichtigungen, Konten und
Protokoll – jeder Bereich mit eigener Adresse (z. B. `/settings/health`), Lesezeichen und Vor/Zurück im Browser
funktionieren. Das gilt auch für die Reiter: `/books`, `/search`, `/discover`, `/requests`.

**Systemprüfung** (`/settings/health`, ähnlich der Diagnose von Vaultwarden) prüft auf einen Blick:
* **Verbindung & Proxy:** als welche Adresse Hörwerk dich sieht, ob der Reverse-Proxy in `TRUSTED_PROXIES` steht
  (sonst Fehler mit der fertigen Zeile zum Eintragen), ob HTTPS ankommt, `ALLOWED_HOSTS`, ob Live-Updates durch den
  Proxy kommen, ob die Adresse in Benachrichtigungen passt. Proxys im eigenen Netz, die `X-Forwarded-For` schicken,
  aber nicht vertrauenswürdig sind (z. B. Newt nach einem Neustart mit neuer IP), stehen hier und im Protokoll.
* **Sicherheit:** Admins ohne Zweitfaktor, Anmelde-Protokoll für CrowdSec, gerade gesperrte Adressen. Hörwerk sperrt
  eine Adresse nach 5 falschen Anmeldungen für 15 Minuten; *Einstellungen → Konten → Sicherheit* listet sie auf, und
  **Freigeben** hebt eine Sperre sofort auf – ganz ohne Terminal. Sperren von CrowdSec oder der Firewall verwaltest du dort.
* **Speicher:** Eingang, Bibliothek, Konfiguration vorhanden, beschreibbar, freier Platz.
* **Dienste:** Indexer, SABnzbd, Audiobookshelf, Plex (werden live getestet), ntfy, Audible.
* **System:** Version, ffmpeg und Sandbox, Warteschlangen, Uhrzeit (wichtig für Zweitfaktor-Codes). Ist die Sandbox
  nicht aktiv, erklärt das **?** unter dem Hinweis, woran es liegt – Kernel ohne Landlock, Landlock beim Start nicht
  eingeschaltet (mit den Schritten für die Syslinux-Konfiguration von Unraid oder GRUB) oder Docker verbietet es – und
  was zu tun ist.

Die Erklärboxen „So funktioniert's“ auf den Seiten lassen sich unter *Einstellungen → Allgemein* für alle ausblenden.

**Ohne Anmeldung** gibt es nur die Login-Seite und das, was sie braucht: keine Seite der App, keinen App-Code, keine
API (bis auf Anmeldung und `/api/health`, das ohne Anmeldung nur `{"ok": true}` sagt – ohne Version). Ein Lesezeichen
auf eine Unterseite führt erst zur Anmeldung und danach genau dorthin. Einstellungen und Eingang sind nur für Admins.
Einstellungen, Voreinstellungen, Verlauf, Hörbuch-Liste, Bibliotheks-Index und alle Zugangsdaten liegen in der
SQLite-Datenbank `/config/hoerwerk.db` (Rechte 600). Einmal am Tag legt Hörwerk eine in sich stimmige Kopie in
`/config/backups/hoerwerk-<Wochentag>.db` ab (die letzten sieben Tage) – zum Zurückspielen Hörwerk stoppen und die
Kopie als `hoerwerk.db` einsetzen.

**Sichern, Version festlegen, zurückgehen:** Zu sichern ist nur `/config` (die Hörbücher liegen in deiner eigenen
Bibliotheks-Freigabe). Jede Version ist ein eigenes Image – `ghcr.io/larosen/hoerwerk:7.20.0` statt `:latest` bleibt bei
dieser Version (die wöchentlichen Sicherheitsupdates kommen trotzdem); was sich geändert hat, steht im
[Changelog](../../CHANGELOG.md). Zum Zurückgehen den älteren Tag eintragen und den Container neu starten; hat die neuere
Version die Datenbank verändert, eine Sicherung von vor dem Update einsetzen. Bücher, die Hörwerk ersetzt hat, werden
nicht gelöscht, sondern nach `_ersetzt` im Eingang verschoben – zum Wiederherstellen den Ordner zurück in die
Bibliothek schieben.

| Variable | Standard | Bedeutung |
|---|---|---|
| `APP_LANGUAGE` | `de` | Oberfläche und Benachrichtigungen: `de` oder `en` |
| `PUID` / `PGID` | `99` / `100` | Benutzer/Gruppe für alle Dateizugriffe |
| `UMASK` | `000` | Rechte neuer Dateien und Ordner |
| `INBOX_DIR` / `LIBRARY_DIR` | `/inbox` / `/audiobooks` | Hörbuch-Eingang und -Bibliothek |
| `AUDIBLE_REGION` | `de` | Audible-Shop: `de us uk fr it es ca au in jp` |
| `MATCH_MODE` | `auto` | `auto` (eindeutige automatisch), `review` (immer prüfen), `off` (nur konvertieren) |
| `MATCH_THRESHOLD` | `0.8` | Mindest-Trefferquote für automatisches Ablegen |
| `FOLDER_TEMPLATE` | `{author}/[{series}/][{series_position:2} - ]{title}` | Ordner in der Bibliothek |
| `FILE_TEMPLATE` | `{title}` | Dateiname der M4B |
| `CHAPTERS` | `auto` | Kapitelnamen von Audible: `auto`, `audible`, `files` |
| `COVER_SOURCE` | `audible` | `audible` (aus dem Shop, volle Auflösung) oder `local` |
| `INBOX_ORIGINALS` | `archive` | Originale danach: `archive` (nach `_fertig`) oder `delete` |
| `SETTLE_SECONDS` | `120` | So lange muss ein Eingang-Ordner unverändert sein |
| `SCAN_INTERVAL` | `60` | Sekunden zwischen zwei Prüfungen des Eingangs |
| `METADATA_FALLBACK` | `true` | Audnexus, Apple Books und Open Library fragen, wenn Audible nichts findet |
| `WRITE_ABS_METADATA` | `true` | `metadata.json` für Audiobookshelf schreiben |
| `DUPLICATES` | `ask` | Buch schon in der Bibliothek: `ask`, `skip`, `keep` |
| `LIBRARY_RESCAN_HOURS` | `6` | Abstand, in dem die Bibliothek neu eingelesen wird |
| `LIBRARY_ORIGINALS` | `archive` | Alte Einzeldateien nach „Zu M4B“: `archive` (nach `_ersetzt`) oder `delete` |
| `NARRATOR_IN_ARTIST` | `false` | Sprecher zusätzlich im Interpreten-Tag |
| `KEEP_SERIES_IN_TITLE` | `true` | „…, Band 3“ im Titel lassen |
| `WRITE_DESC` / `WRITE_READER` | `false` | `desc.txt`, `reader.txt` schreiben (nur nötig ohne `metadata.json`) |
| `WRITE_COVER` | `true` | `cover.jpg` schreiben (Audiobookshelf und Plex nutzen es) |
| `DISCOVER_MIN_RATING` / `DISCOVER_MIN_COUNT` | `4.3` / `50` | Entdecken: Mindestbewertung und -anzahl Bewertungen |
| `DISCOVER_UNABRIDGED` / `DISCOVER_GERMAN` | `true` | Entdecken: nur ungekürzt / nur deutschsprachig |
| `DISCOVER_NONFICTION_STORE` | `false` | Entdecken: alle Sachbücher in „Für dich“ (sonst nur von anerkannten Autoren) |
| `DISCOVER_TRUSTED_AUTHORS` | – | Entdecken: weitere anerkannte Sachbuch-Autoren, mit Komma getrennt |
| `WANTED_INTERVAL_HOURS` | `12` | Anfragen ohne Treffer: Abstand der erneuten Suche |
| `ADMIN_TOTP_REQUIRED` | `false` | Admins brauchen einen Zweitfaktor |
| `CPU_CORES` | halbe Kernzahl | Parallele ffmpeg-Prozesse pro Buch |
| `ENCODE_WORKERS` | `1` | Bücher, die gleichzeitig kodiert werden |
| `IO_WORKERS` | `2` | Gleichzeitige Erkennungs-, Tag- und Ablageschritte |
| `NICE` | `10` | CPU-Priorität von ffmpeg (0 = normal) |
| `FFMPEG_SANDBOX` | `auto` | ffmpeg im Landlock-Käfig: `auto`, `require`, `off` |
| `FFMPEG_MEMORY_MB` | `4096` | Speichergrenze je ffmpeg-Prozess |
| `AUDIO_QUALITY` | `high` | `standard`, `high` oder `original` – wie viel Bitrate Lesungen und Hörspiele behalten |
| `AAC_BITRATE` | `auto` | `auto` (wie die Quelle, begrenzt durch `AUDIO_QUALITY`) oder fester Wert |
| `FORCE_MONO` | `false` | Mono ausgeben |
| `CHAPTER_TITLES` | `auto` | `auto`, `filename` oder `tag` |
| `TRUSTED_PROXIES` | – | IPs/Netze des Reverse-Proxys (Pangolin/Newt, Traefik), z. B. `172.18.0.0/16` – nur von dort zählt `X-Forwarded-For` |
| `COOKIE_SECURE` | `auto` | Anmelde-Cookie nur über HTTPS: `auto` (wenn der Proxy HTTPS meldet), `true`, `false` |
| `REQUEST_QUOTA` | `5` | Buchanfragen pro Nutzer und Monat (je Konto änderbar) |
| `AUTH_USER` / `AUTH_PASS` | – | Nur noch für den Umstieg: wird beim ersten Start zum Admin-Konto |
| `ALLOWED_HOSTS` | – | Namen, unter denen Hörwerk antwortet (Schutz gegen DNS-Rebinding). Leer: IP-Adressen, `localhost`, Namen aus dem Heimnetz (`tower`, `*.local`, `*.lan`, `*.fritz.box` …) und die öffentliche Adresse aus den Einstellungen; über einen Proxy aus `TRUSTED_PROXIES` jeder Name, den der Proxy weiterleitet. Gesetzt: genau diese Namen, z. B. `hoerbuecher.example.de,*.example.de` (IPs und `localhost` gehen immer). `*` = jeder Name wie vor 7.13 (nicht empfohlen). Ungültige Werte (mit `https://` oder Pfad) stoppen den Start mit einer Meldung. `/api/health` antwortet unter jedem Namen – für die Health Checks von Docker und Pangolin |

SABnzbd, der optionale Indexer, Audiobookshelf, Plex und ntfy werden in der Oberfläche eingerichtet, nicht über Variablen.

## Sicherheit

Kurz gefasst – Details, Tests und Komponenten-Versionen in [SECURITY.de.md](../../SECURITY.de.md):

* Konten mit Login; ins Internet nur hinter einem Reverse-Proxy mit HTTPS (z. B. Pangolin) und mit
  `TRUSTED_PROXIES`. Fehlversuche stehen mit IP in `/config/logs/auth.log` – fertig für CrowdSec.
* Nur nötige Ordner einbinden; der Container läuft ohne Linux-Capabilities und mit `no-new-privileges`.
* Heruntergeladene Dateien werden nie ausgeführt. ffmpeg ist ein eigener schlanker Build (nur Audio, kein
  Netzwerk-Code) in einem Landlock-Käfig, der nur die Dateien des Buchs lesen darf.
* API-Keys, Token und Passwörter (SABnzbd, Indexer, Audiobookshelf, Plex, ntfy) bleiben auf dem Server und gehen nie an den Browser.
