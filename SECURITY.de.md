# Sicherheit

[English](SECURITY.md) · **Deutsch**

Stand: Security-Review vom 30.10.2026 (Version 4.1: ntfy-Benachrichtigungen, Deutsch/Englisch). Hörwerk verarbeitet Dateien aus fremden Quellen
(Downloads über SABnzbd), fragt externe Dienste ab (Audible, Audnexus, BookBeat, Thalia, optional ein Indexer, SABnzbd, Audiobookshelf, ntfy)
und hat eine Weboberfläche mit Schreibzugriff auf deine Shares. Hier steht, wovor das Programm sich
schützt, was dafür geprüft wurde und was du selbst tun solltest.

## Empfohlene Einstellungen

1. **Konten:** Beim ersten Aufruf wird das Admin-Konto angelegt; ohne Anmeldung gibt es nur die Login-Seite.
   Ins Internet nur hinter einem Reverse-Proxy mit HTTPS (z. B. **Pangolin**), dessen Adresse in `TRUSTED_PROXIES`
   steht – sonst sieht Hörwerk nicht die echte IP der Besucher. Fehlversuche landen in `/config/logs/auth.log`
   (für CrowdSec, siehe [contrib/crowdsec](contrib/crowdsec/README.md)).
2. **Container mit minimalen Rechten** starten – im Unraid-Template und in `docker-compose.yml` ist das
   schon eingetragen: `--security-opt no-new-privileges:true --cap-drop ALL --cap-add CHOWN --cap-add SETUID --cap-add SETGID`.
3. **Nur die nötigen Ordner einbinden:** Eingang und Bibliothek, nie ganz `/mnt/user/`.
4. **In SABnzbd** unter *Config → Switches* „Unerwünschte Endungen“ (`exe, bat, cmd, scr, lnk, js, vbs, ps1, msi, com`)
   auf **Abbrechen** stellen. Gefälschte Releases fliegen dann schon vor dem Eingang raus.
5. Originale im Eingang auf **löschen** (`INBOX_ORIGINALS=delete`) oder `_fertig` regelmäßig leeren.
6. `ALLOWED_HOSTS` auf die Adressen setzen, unter denen du Hörwerk aufrufst, z. B.
   `ALLOWED_HOSTS=tower,192.168.1.10,hoerbuecher.example.de` (Schutz gegen DNS-Rebinding und fremde Host-Namen).
7. Image aktuell halten: GitHub Actions baut es jede Woche neu (Debian-Sicherheitsupdates); neue ffmpeg-Versionen
   kommen über ein Update von `FFMPEG_VERSION` im Dockerfile;
   auf Unraid gelegentlich „Check for Updates“.

## Bedrohungen und Schutzmaßnahmen

### 1. Bösartige Dateien in einem Download

| Angriff | Schutz |
|---|---|
| Datei mit Audio-Endung, die in Wahrheit eine Playlist (HLS) oder ein Concat-Skript ist und ffmpeg lokale Dateien (z. B. `/config/hoerwerk.db`) oder URLs lesen lässt | ffmpeg/ffprobe lesen Download-Dateien nur mit `-protocol_whitelist file` und `-format_whitelist` auf echte Audio-Demuxer (mp3, mov/m4a/m4b, aac, flac, ogg, wav, aiff, asf, matroska, ape, wv …). Playlists, Concat, Bildsequenzen, Netzprotokolle sind gesperrt. Getestet (`test_security.py`). |
| Lücke in ffmpeg | **Eigenes ffmpeg 9** mit nur den nötigen Audio-Formaten, ohne jeglichen Netzwerk-Code, ohne Fremdbibliotheken, gehärtet gebaut (PIE, RELRO, Stack-Schutz). Jeder ffmpeg/ffprobe-Aufruf läuft zusätzlich in einem **Landlock-Käfig**: lesen nur die Dateien dieses Aufrufs, schreiben nur ins Arbeitsverzeichnis, keine TCP-Verbindungen; dazu `no_new_privs`, Speicher-/Dateigrößen-Grenzen, keine Core-Dumps. Selbst ein ausgenutzter Fehler kommt so nicht an `/config` (API-Keys) oder die Bibliothek. Getestet (`test_sandbox.py`); die CI lässt die ganze Testsuite mit genau diesem ffmpeg und Pflicht-Käfig laufen. Hängende Prozesse werden nach 6 h beendet. |
| Symlinks im Download (`cover.jpg → /config/hoerwerk.db`, Ordner-Link nach `/`) | Symlinks werden beim Einlesen komplett ignoriert (Audio, Cover, `chapters.txt`, `metadata.yml`, Unterordner, Eingangs-Einträge) und im Protokoll als verdächtig gemeldet. |
| `.exe`, `.bat`, `.lnk`, Skripte usw. (typisch für Fake-Releases) | Werden nie ausgeführt und nie in die Bibliothek übernommen – dort landen nur die erzeugte M4B, `cover.jpg`, `desc.txt`, `reader.txt`, `metadata.json`. Der Fund steht als ⚠ im Protokoll des Buchs. |
| Pfad-Tricks in Tags/Namen (`../../etc`, `/`, `:`) | Bibliothekspfade entstehen nur aus bereinigten Einzelteilen (keine Trenner, kein `..`, keine Steuerzeichen, max. 180 Byte); getestet. |
| `metadata.yml`-Bombe (YAML-Anker) oder riesige Text-Dateien | `yaml.safe_load`, Anker/Aliase werden abgelehnt, `metadata.yml`/`chapters.txt` max. 512 KB. |
| Präparierte Tags (Skript-Code im Titel) | Die Oberfläche setzt Texte ausschließlich als Text (kein `innerHTML` mit fremden Daten); zusätzlich Content-Security-Policy ohne Inline-Skripte. |
| Riesige Cover | Bilder > 20 MB im Ordner werden ignoriert; Shop-Cover max. 15 MB und nur echtes JPEG/PNG. |

### 2. Abfragen ins Netz

| Angriff | Schutz |
|---|---|
| Shop-Link oder Weiterleitung zu internen Adressen (Router, NAS, `169.254.169.254`) – SSRF | Links werden nur für Audible, BookBeat und Thalia abgerufen; jede Weiterleitung muss wieder dort hinführen. Cover werden nur von öffentlichen Adressen geladen (DNS-Auflösung, private/Loopback/Link-Local-Adressen werden abgelehnt). Nur `http(s)`. |
| Sehr große Antworten (Speicher) | Antworten max. 8 MB (Shops/Audible) bzw. 20 MB (Indexer). |
| Indexer-XML mit DTD/Entities (Billion Laughs, XXE) | Antworten mit `<!DOCTYPE`/`<!ENTITY` werden abgelehnt; Pythons XML-Parser lädt ohnehin keine externen Entities. |
| `javascript:`-Links aus fremden Daten (Indexer-Details, Shop-Seiten) | Server lässt nur `http(s)`-Links durch, die Oberfläche filtert `href`/`src` zusätzlich zentral. |
| ntfy-Token/Passwort | Nur auf dem Server, nie an den Browser (die Oberfläche sieht nur „gespeichert“), bei geänderter Server-Adresse nicht mitgenommen. Weiterleitungen werden nicht verfolgt. Adresse nur `http(s)` ohne `benutzer:passwort@`, Topic nur `A–Z a–z 0–9 - _`. Nachrichten gehen als JSON (keine Header-Injection über Buchtitel); der „Öffnen“-Link ist nur die selbst eingetragene `http(s)`-Adresse. |
| Plex-Token | Wie der Audiobookshelf-Token: nur auf dem Server, nie an den Browser, bei geänderter Adresse nicht mitgenommen, keine Weiterleitungen, kein `benutzer:passwort@` in der Adresse. Hörwerk bittet Plex nur um Neu-Einlesen und (wenn angehakt) ums Leeren des Papierkorbs der einen gewählten Mediathek. |
| Audiobookshelf-Token | Wie die Indexer-Keys: nur auf dem Server, nie an den Browser, bei geänderter Adresse nicht mitgenommen. Weiterleitungen werden nicht verfolgt, damit der Token nie bei einem anderen Server landet. |
| Entdecken | Nur feste Adressen über HTTPS: die Audible-API, `www.deutschlandfunkkultur.de` und `www.perlentaucher.de` (RSS bzw. eine Listenseite, max. 4 MB, keine Weiterleitung woandershin, Feeds mit DTD/Entities werden abgelehnt; HTML wird nur mit festen Mustern nach Autor und Titel durchsucht). Links zur Kritik werden nur als `https://` angezeigt. Vorschläge lösen nie selbst einen Download aus. |
| Weitere Metadaten-Quellen | Nur feste Adressen über HTTPS: `api.audnex.us`, `itunes.apple.com`, `openlibrary.org` (dazu wie bisher Audible, BookBeat, Thalia). Treffer daraus werden nie automatisch übernommen. |
| Abgreifen von API-Keys | Keys liegen nur in der Datenbank `/config/hoerwerk.db` (Rechte 600), werden nie an den Browser geschickt, ebenso wenig NZB-Links (enthalten den Indexer-Key). Ändert sich die Adresse eines Indexers oder von SABnzbd, wird der gespeicherte Key **nicht** mitgenommen – er muss neu eingegeben werden. Fehlermeldungen werden von Keys bereinigt. |
| Ordnername eines Downloads kollidiert mit internen Ordnern (`.work`) | Der Name, den SABnzbd bekommt, wird bereinigt (keine Pfade, keine versteckten Namen). |
| TLS | Zertifikate werden immer geprüft (Python-Standard). SABnzbd/Indexer/ntfy im LAN dürfen `http` sein – dann gehen deren Keys unverschlüsselt durchs Heimnetz. |

### 3. Weboberfläche / API

| Angriff | Schutz |
|---|---|
| **CSRF**: eine fremde Webseite im selben Browser schickt Anfragen an `http://tower:8080` (z. B. Einstellungen ändern oder Bücher aus der Bibliothek holen) | Alle ändernden Anfragen brauchen einen eigenen Header, den Browser nur nach einer (nie erlaubten) CORS-Freigabe senden; zusätzlich Prüfung von `Origin` und `Sec-Fetch-Site`. Getestet. |
| Clickjacking | `X-Frame-Options: DENY`, `frame-ancestors 'none'`. |
| Zugriff ohne Anmeldung | Ohne gültige Sitzung gibt es nur die Login-Seite, den Oberflächen-Code (`/static`, ohnehin Open Source), `/i18n.js`, das Logo und `/api/health`. Jede API-Route hat eine Zugriffsstufe (öffentlich / Nutzer / Admin, Standard Admin), geprüft im Server – nicht nur ausgeblendet. Getestet: unangemeldet 401, Familienkonto auf Admin-Funktionen 403 (wird protokolliert). |
| Erst-Einrichtung durch Fremde | Solange es kein Konto gibt, darf nur aus dem Heimnetz ohne Weiteres ein Admin angelegt werden; von außen braucht es den Einrichtungs-Code aus dem Container-Log. Danach ist die Einrichtung gesperrt. |
| Passwort raten | scrypt (N=2¹⁵), Mindestlänge 10; je IP höchstens 5 Fehlversuche in 15 Minuten, danach 429; je Konto wachsende Verzögerung (ein Fremder kann kein Familienkonto aussperren); unbekannte Namen kosten dieselbe Zeit wie bekannte. Jeder Fehlversuch mit IP in `auth.log` → CrowdSec sperrt am VPS. |
| Gestohlene Sitzung | Token mit 256 Bit, in der DB nur als SHA-256; Cookie `HttpOnly`, `SameSite=Lax`, `Secure` hinter HTTPS. Passwortwechsel, Sperren eines Kontos und „Alle anderen Geräte abmelden“ beenden Sitzungen sofort. |
| Gefälschte IP in `X-Forwarded-For` | Der Header zählt nur von Adressen in `TRUSTED_PROXIES` und wird von rechts gelesen – vorne angehängte Fantasie-Adressen bleiben wirkungslos. |
| Rechteausweitung | Rolle und Rechte ändert nur ein Admin (Server-Prüfung); der letzte Admin kann weder herabgestuft noch gesperrt noch gelöscht werden. |
| DNS-Rebinding / fremde Host-Namen | `ALLOWED_HOSTS`; zusätzlich braucht jede Datenanfrage eine Anmeldung. |
| Pfade außerhalb der Bibliothek | Buch-Pfade aus dem Browser werden aufgelöst (inkl. Symlinks) und müssen in der Bibliothek liegen; statische Dateien nur aus `app/static`. |
| Riesige Anfragen | max. 4 MB pro Anfrage. |
| Viele offene Live-Verbindungen (`/api/events`) | Höchstens 32 gleichzeitig; sie liefern nur, was die Oberfläche ohnehin abruft, und brauchen dieselbe Anmeldung. |
| Spracheinstellung / Übersetzungen | Nur `de` oder `en` werden angenommen; `/i18n.js` liefert eine feste Datei aus dem Image (auch für die Login-Seite). |
| Sonstige Header | `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Cross-Origin-Opener-Policy`/`-Resource-Policy: same-origin`, `Cache-Control: no-store`. |

### 4. Container

* App läuft als PUID/PGID (Standard 99:100) **ohne Linux-Capabilities**, `no-new-privileges` aktiv (geprüft
  über `/proc/<pid>/status`). Root wird nur im Entrypoint für Benutzeranlage und `chown /config` gebraucht.
* Abhängigkeiten: nur `mutagen` und `PyYAML` (versionsgepinnt), Rest Python-Standardbibliothek + eigenes ffmpeg
  (Quelltext vom offiziellen FFmpeg-Repository, Release-Tag auf seinen Commit festgenagelt).
* ffmpeg-Käfig: `FFMPEG_SANDBOX=auto` (Standard) nutzt Landlock, wenn Kernel und Docker es erlauben (Linux ≥ 5.13,
  Netzwerk-Regeln ab 6.7; Docker ≥ 23). Ob er aktiv ist, steht unten in ⚙︎ Einstellungen. `require` verweigert
  ffmpeg ohne Käfig, `off` schaltet ihn ab.
* Wöchentlicher Neubau ohne Cache, damit Sicherheitsupdates ankommen.

## Komponenten-Stand (geprüft am 30.10.2026)

| Komponente | Version | Neueste | Bewertung |
|---|---|---|---|
| Python (Docker-Basis) | 3.15.2 (`python:3.15-slim-trixie`) | 3.15.2 | aktuell |
| Debian | 13 „trixie“ (stable) | 13 | aktuell, Sicherheitsupdates per wöchentlichem Neubau |
| ffmpeg | 9.0.2 (eigener schlanker Build) | 9.0.2 | aktuell, siehe unten |
| tini | 0.19.0 (Debian) | 0.19.0 | aktuell |
| mutagen | 1.48.1 | 1.48.1 | aktuell, keine bekannten Lücken (OSV) |
| PyYAML | 6.0.3 | 6.0.3 | aktuell, keine bekannten Lücken (OSV); unter Python 3.15 ohne C-Beschleunigung (für kleine `metadata.yml` egal) |
| Node.js (nur Build der Oberfläche) | 26 LTS | 26 | aktuell, nicht im fertigen Image |
| TypeScript / Playwright | 7.0.2 / 1.64.1 | dieselben | aktuell (nur Entwicklung/CI) |
| GitHub Actions | checkout v7, setup-python v7, setup-node v7, docker/* v4/v6/v7 | dieselben | aktuell |

Geprüfte Schnittstellen: Audiobookshelf 2.37 (`metadata.json`, Bibliotheks-Scan), SABnzbd 5.1 (`addurl`, `queue`, `history`, `get_cats`), ntfy 2.35 (JSON-Publish, Token/Basic-Auth).

**ffmpeg:** Bis Version 3 kam ffmpeg 7.1.5 aus Debian stable; dafür listete OSV 40 offene CVEs (fast alle in
Teilen, die Hörwerk nicht nutzt). Seit Version 4 baut das Image ffmpeg 9.0.2 selbst
(`docker/build-ffmpeg.sh`): nur die Audio-Container und -Decoder, die vorkommen, AAC als einziger Encoder,
ffprobe/ffmpeg dürfen nur die Audio-Container öffnen (Whitelist). Videodecoder, Filter außer
Resampling, Streaming-Protokolle (HLS, DASH, RTSP …), Geräte und Fremdbibliotheken sind gar nicht im
Programm – die meisten ffmpeg-CVEs betreffen genau diese Teile. Weil das Paket nicht mehr aus der
Debian-Sicherheitspflege kommt, muss die Version im Dockerfile bei ffmpeg-Sicherheitsupdates angehoben werden
(Tag und Commit, die CI prüft beides).

## Bekannte Restrisiken

* **0-Day in ffmpeg oder mutagen**: durch die Maßnahmen oben stark eingegrenzt (ffmpeg im Käfig), aber nicht auszuschließen –
  der Container hat Schreibzugriff auf die eingebundenen Shares. Darum nur nötige Ordner einbinden.
* **Kein HTTPS im Container**: Basic-Auth-Passwort geht im LAN im Klartext. Für Zugriff von außen einen
  Reverse-Proxy mit TLS (z. B. Nginx Proxy Manager, SWAG) davorsetzen.
* **Plattenplatz**: sehr große oder viele Downloads können den Eingang füllen; es gibt kein Kontingent.
* Die öffentliche-Adresse-Prüfung für Cover löst DNS vor dem Abruf auf; ein Angreifer mit Kontrolle über
  einen Shop-DNS-Eintrag könnte theoretisch zwischen Prüfung und Abruf umschalten (sehr unwahrscheinlich).

## Sicherheitsprobleme melden

Bitte als privates Issue/Security Advisory im Repository melden, nicht öffentlich.
