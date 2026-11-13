# Sicherheit

[English](SECURITY.md) · **Deutsch**

Gilt für **Version 7.20** (letzte vollständige Sicherheitsprüfung: 30.10.2026; was sich seitdem geändert hat, steht im
[Changelog](CHANGELOG.md), und die Schutzmaßnahmen unten prüfen die Tests bei jeder Änderung). Hörwerk verarbeitet Dateien aus fremden Quellen
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
6. Die öffentliche Adresse unter *Einstellungen → Allgemein* eintragen. Hörwerk antwortet ab 7.13 von sich aus nur
   unter IP-Adressen, Namen aus dem Heimnetz und dieser Adresse (Schutz gegen DNS-Rebinding); weitere Namen über
   `ALLOWED_HOSTS`, z. B. `ALLOWED_HOSTS=hoerbuecher.example.de`.
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
| Freie Hörbücher (LibriVox) als Weg, beliebiges anderes zu laden | Nur HTTPS zu archive.org, auch nach Weiterleitungen. Geladen wird nur, was das Internet Archive in der LibriVox-Sammlung führt; nur dessen MP3-Dateien (einfache Namen, keine Pfade), einzeln und insgesamt größenbegrenzt und als MP3 geprüft, höchstens zwei Bücher gleichzeitig, nur für Admins. Sie landen wie jeder andere Download im Eingang und durchlaufen dieselben Prüfungen. |
| Shop-Link oder Weiterleitung zu internen Adressen (Router, NAS, `169.254.169.254`) – SSRF | Links werden nur für Audible, BookBeat und Thalia abgerufen; jede Weiterleitung muss wieder dort hinführen. Cover werden nur von öffentlichen Adressen geladen (DNS-Auflösung, private/Loopback/Link-Local-Adressen werden abgelehnt). Nur `http(s)`. |
| Sehr große Antworten (Speicher) | Antworten max. 8 MB (Shops/Audible) bzw. 20 MB (Indexer). |
| Indexer-XML mit DTD/Entities (Billion Laughs, XXE) | Antworten mit `<!DOCTYPE`/`<!ENTITY` werden abgelehnt; der XML-Parser (roxmltree) lädt ohnehin keine externen Entities. |
| `javascript:`-Links aus fremden Daten (Indexer-Details, Shop-Seiten) | Server lässt nur `http(s)`-Links durch, die Oberfläche filtert `href`/`src` zusätzlich zentral. |
| ntfy-Token/Passwort | Nur auf dem Server, nie an den Browser (die Oberfläche sieht nur „gespeichert“), bei geänderter Server-Adresse nicht mitgenommen. Weiterleitungen werden nicht verfolgt. Adresse nur `http(s)` ohne `benutzer:passwort@`, Topic nur `A–Z a–z 0–9 - _`. Nachrichten gehen als JSON (keine Header-Injection über Buchtitel); der „Öffnen“-Link ist nur die selbst eingetragene `http(s)`-Adresse. |
| Plex-Token | Wie der Audiobookshelf-Token: nur auf dem Server, nie an den Browser, bei geänderter Adresse nicht mitgenommen, keine Weiterleitungen, kein `benutzer:passwort@` in der Adresse. Hörwerk bittet Plex nur um Neu-Einlesen und (wenn angehakt) ums Leeren des Papierkorbs der einen gewählten Mediathek. |
| Audiobookshelf-Token | Wie die Indexer-Keys: nur auf dem Server, nie an den Browser, bei geänderter Adresse nicht mitgenommen. Weiterleitungen werden nicht verfolgt, damit der Token nie bei einem anderen Server landet. |
| Entdecken | Nur feste Adressen über HTTPS: die Audible-API, `www.deutschlandfunkkultur.de`, `www.deutschlandfunk.de`, `bilder.deutschlandfunk.de` und `www.perlentaucher.de` (RSS bzw. eine Listenseite, max. 8 MB, keine Weiterleitung woandershin, Feeds mit DTD/Entities werden abgelehnt; HTML wird nur mit festen Mustern nach Autor und Titel durchsucht; die PDFs der monatlichen Bestenlisten liest ein kleiner eingebauter Textleser, der nur FlateDecode-Streams entpackt – mit Größengrenzen je Stream und insgesamt –, Bilder überspringt und nichts ausführt). Links zur Kritik werden nur als `https://` angezeigt. Vorschläge lösen nie selbst einen Download aus. |
| Weitere Metadaten-Quellen | Nur feste Adressen über HTTPS: `api.audnex.us`, `itunes.apple.com`, `openlibrary.org` (dazu wie bisher Audible, BookBeat, Thalia). Treffer daraus werden nie automatisch übernommen. |
| Abgreifen von API-Keys | Keys liegen nur in der Datenbank `/config/hoerwerk.db` (Rechte 600), werden nie an den Browser geschickt, ebenso wenig NZB-Links (enthalten den Indexer-Key). Ändert sich die Adresse eines Indexers oder von SABnzbd, wird der gespeicherte Key **nicht** mitgenommen – er muss neu eingegeben werden. Fehlermeldungen werden von Keys bereinigt. |
| Ordnername eines Downloads kollidiert mit internen Ordnern (`.work`) | Der Name, den SABnzbd bekommt, wird bereinigt (keine Pfade, keine versteckten Namen). |
| TLS | Zertifikate werden immer gegen die System-Zertifikate geprüft (rustls). SABnzbd/Indexer/ntfy im LAN dürfen `http` sein – dann gehen deren Keys unverschlüsselt durchs Heimnetz. |

### 3. Weboberfläche / API

| Angriff | Schutz |
|---|---|
| **CSRF**: eine fremde Webseite im selben Browser schickt Anfragen an `http://tower:8080` (z. B. Einstellungen ändern oder Bücher aus der Bibliothek holen) | Alle ändernden Anfragen brauchen einen eigenen Header, den Browser nur nach einer (nie erlaubten) CORS-Freigabe senden; zusätzlich Prüfung von `Origin` und `Sec-Fetch-Site`. Getestet. |
| Clickjacking | `X-Frame-Options: DENY`, `frame-ancestors 'none'`. |
| Zugriff ohne Anmeldung | Ohne gültige Sitzung gibt es nur die Login-Seite, den Oberflächen-Code (`/static`, ohnehin Open Source), `/i18n.js`, das Logo und `/api/health`. Jede API-Route hat eine Zugriffsstufe (öffentlich / Nutzer / Admin, Standard Admin), geprüft im Server – nicht nur ausgeblendet. Getestet: unangemeldet 401, Familienkonto auf Admin-Funktionen 403 (wird protokolliert). |
| Erst-Einrichtung durch Fremde | Solange es kein Konto gibt, darf nur aus dem Heimnetz ohne Weiteres ein Admin angelegt werden; von außen braucht es den Einrichtungs-Code aus dem Container-Log. Danach ist die Einrichtung gesperrt. |
| Passwort raten | scrypt (N=2¹⁵); Passwortregeln (Server und Browser gleich): mindestens 12 Zeichen, drei von vier Zeichenarten oder ein Satz ab 20 Zeichen, nicht der eigene Name, nicht „Hörwerk“/„Passwort“/„123456“, nicht ein einzelnes Wort mit Zahlen drumherum („Pudding2024!“), nicht immer dieselben Zeichen; je IP (bei IPv6 je /64-Netz, ein Anschluss hat ein ganzes /64) höchstens 5 Fehlversuche in 15 Minuten, danach 429 – eine Flut neuer Adressen verdrängt zuerst Einzelversuche, nie bestehende Sperren; je Konto wachsende Verzögerung (ein Fremder kann kein Familienkonto aussperren); unbekannte Namen kosten dieselbe Zeit wie bekannte. Jeder Fehlversuch mit IP in `auth.log` → CrowdSec sperrt am VPS. |
| Gestohlene Sitzung | Token mit 256 Bit, in der DB nur als SHA-256; Cookie `HttpOnly`, `SameSite=Lax`, `Secure` hinter HTTPS. Passwortwechsel, Sperren eines Kontos und „Alle anderen Geräte abmelden“ beenden Sitzungen sofort. |
| Gefälschte IP in `X-Forwarded-For` | Der Header zählt nur von Adressen in `TRUSTED_PROXIES` und wird von rechts gelesen – vorne angehängte Fantasie-Adressen bleiben wirkungslos. |
| Rechteausweitung | Rolle und Rechte ändert nur ein Admin (Server-Prüfung); der letzte Admin kann weder herabgestuft noch gesperrt noch gelöscht werden. |
| Fremde Anfragen ansehen oder ändern (IDOR) | Jede Anfrage-Route prüft, ob das Konto sie sehen darf; fremde und nicht vorhandene IDs liefern dieselbe 404 – Nummern lassen sich nicht ausprobieren. Besitzer und Status setzt nur der Server (Felder aus dem Browser werden ignoriert). Getestet. |
| Kontingent umgehen | Prüfung und Anlegen in einer Datenbank-Transaktion unter Sperre – auch parallele Anfragen kommen nicht über das Limit. |
| Familienkonto lädt Beliebiges | Nutzer fragen nur Bücher an; Releases wählt Hörwerk nach festen Regeln. Die interaktive Suche (Releases von Hand) gibt es nur mit eigenem Recht, Indexer-Links (mit API-Key) erreichen nie den Browser. |
| Passwort gestohlen | Optionaler Zweitfaktor (TOTP nach RFC 6238, Standardbibliothek): Code gilt einmal (keine Wiederverwendung), ±30 s Uhrabweichung, Anmelde-Ticket 5 Minuten / 5 Versuche, Fehlversuche zählen für die IP-Bremse und landen in `auth.log`. Für Admins als Pflicht einstellbar. |
| Unbemerkte Änderungen durch einen Admin-Zugang | Admin-Protokoll in der Datenbank: jede ändernde Admin-Aktion mit Zeit, Konto und IP (Freigaben, Konten, Einstellungen, Quellen …), einsehbar unter *Konten*. |
| DNS-Rebinding / fremde Host-Namen | Standardmäßig nur IPs, Heimnetz-Namen, die öffentliche Adresse und was ein Proxy aus `TRUSTED_PROXIES` weiterleitet (nie aus Host/X-Forwarded-Host gelernt), sonst 421; `ALLOWED_HOSTS` für weitere Namen; zusätzlich braucht jede Datenanfrage eine Anmeldung. |
| Pfade außerhalb der Bibliothek | Buch-Pfade aus dem Browser werden aufgelöst (inkl. Symlinks) und müssen in der Bibliothek liegen; statische Dateien nur aus `/app/static`. |
| Riesige Anfragen | max. 4 MB pro Anfrage; Ausnahme nur der Upload (bis 4 GB je Datei, direkt auf die Platte gestreamt). |
| Upload | Nur Admins; Anmeldung und Rechte werden geprüft, bevor ein Byte gelesen wird. Nur relative Pfade ohne `..` und ohne versteckte Namen, nur Audio, Cover, Cue-Sheets und Hörwerks Textdateien – und zwar nach dem **Inhalt**: Die ersten Bytes müssen zur Endung passen (Audio-Kennung wie ID3/MPEG/FLAC/Ogg/MP4, Bild-Kennung, Text ohne Binärdaten). Programme (`.exe`, ELF, macOS), PDFs, Office-Dokumente, Archive (ZIP/RAR/7z …) und Skripte werden auch umbenannt erkannt und abgelehnt, bevor etwas auf der Platte landet; freier Platz wird vor jeder Datei geprüft; erst ein versteckter Zwischenordner, dann ein Schritt in den Eingang (nichts wird überschrieben).. Grenzen: 8 Uploads gleichzeitig (3 je Konto), 64 GB und 3000 Dateien je Upload, 128 GB je Konto in offenen Uploads – jede Datei bucht ihre Größe vor dem Schreiben, parallele Uploads kommen also nicht vorbei. Ein Upload gehört dem Konto, das ihn begonnen hat; abgeschlossen wird er genau einmal, nie während noch Dateien ankommen, und nach einem Absturz zu Ende geführt (Markierungsdatei) – unfertige Zwischenordner werden beim nächsten Start entfernt. |
| Viele offene Live-Verbindungen (`/api/events`) | Höchstens 32 gleichzeitig; sie liefern nur, was die Oberfläche ohnehin abruft, und brauchen dieselbe Anmeldung. |
| Spracheinstellung / Übersetzungen | Nur `de` oder `en` werden angenommen; `/i18n.js` liefert eine feste Datei aus dem Image (auch für die Login-Seite). |
| Sonstige Header | `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Cross-Origin-Opener-Policy`/`-Resource-Policy: same-origin`, `Cache-Control: no-store`. |

### 4. Container

* App läuft als PUID/PGID (Standard 99:100) **ohne Linux-Capabilities**, `no-new-privileges` aktiv (geprüft
  über `/proc/<pid>/status`). Root wird nur im Entrypoint für Benutzeranlage und `chown /config` gebraucht.
* Ein einziges Rust-Programm, kein Interpreter im Image. Alle Crates sind in `rust/Cargo.lock` festgelegt
  (`--locked`), `cargo deny` prüft Lizenzen, Herkunft und Sicherheitsmeldungen. Dazu das eigene ffmpeg
  (Quelltext vom offiziellen FFmpeg-Repository, Release-Tag auf seinen Commit festgenagelt).
* ffmpeg-Käfig: `FFMPEG_SANDBOX=auto` (Standard) nutzt Landlock, wenn Kernel und Docker es erlauben (Linux ≥ 5.13,
  Netzwerk-Regeln ab 6.7; Docker ≥ 23). Ob er aktiv ist – und wenn nicht, warum und wie man ihn einschaltet –, steht unter *Einstellungen → Systemprüfung*. `require` verweigert
  ffmpeg ohne Käfig, `off` schaltet ihn ab.
* Wöchentlicher Neubau ohne Cache, damit Sicherheitsupdates ankommen.

## Komponenten-Stand (geprüft am 12.11.2026)

| Komponente | Version | Neueste | Bewertung |
|---|---|---|---|
| Rust (Bau des Servers) | 1.97.0 (`rust:1.97-slim-trixie`) | 1.97 | aktuell, nicht im fertigen Image |
| Debian | 13 „trixie“ (stable) | 13 | aktuell, Sicherheitsupdates per wöchentlichem Neubau |
| ffmpeg | 9.0.2 (eigener schlanker Build) | 9.0.2 | aktuell, siehe unten |
| tini | 0.19.0 (Debian) | 0.19.0 | aktuell |
| Crates (216, u. a. axum 0.8, tokio 1.54, rustls 0.23, ureq 3.5, rusqlite 0.41 mit eingebautem SQLite) | `rust/Cargo.lock` | – | `cargo deny` in jeder CI ohne Meldungen |
| Node.js (nur Build der Oberfläche) | 26 LTS | 26 | aktuell, nicht im fertigen Image |
| TypeScript / Playwright | 7.0.2 / 1.65.0 | dieselben | aktuell (nur Entwicklung/CI) |
| GitHub Actions | checkout v7, setup-python v7 (nur Tests), setup-node v7, docker/* v4/v6/v7 | dieselben | aktuell |

Geprüfte Schnittstellen: Audiobookshelf 2.37 (`metadata.json`, Bibliotheks-Scan), SABnzbd 5.1 (`addurl`, `queue`, `history`, `get_cats`), ntfy 2.35 (JSON-Publish, Token/Basic-Auth).

**ffmpeg:** Bis Version 3 kam ffmpeg 7.1.5 aus Debian stable; dafür listete OSV 40 offene CVEs (fast alle in
Teilen, die Hörwerk nicht nutzt). Seit Version 4 baut das Image ffmpeg 9.0.2 selbst
(`docker/build-ffmpeg.sh`): nur die Audio-Container und -Decoder, die vorkommen, AAC als einziger Encoder,
ffprobe/ffmpeg dürfen nur die Audio-Container öffnen (Whitelist). Videodecoder, Filter außer
Resampling, Streaming-Protokolle (HLS, DASH, RTSP …), Geräte und Fremdbibliotheken sind gar nicht im
Programm – die meisten ffmpeg-CVEs betreffen genau diese Teile. Weil das Paket nicht mehr aus der
Debian-Sicherheitspflege kommt, muss die Version im Dockerfile bei ffmpeg-Sicherheitsupdates angehoben werden
(Tag und Commit, die CI prüft beides).

## Bauen und Lieferkette

Jeder Push und Pull Request durchläuft: Clippy (alle Warnungen sind Fehler), `cargo deny` (Lizenzen, Herkunft und
Sicherheitsmeldungen aller Crates), npm audit (Bauwerkzeuge), die Unit-Tests, den API-Vertrag (auch mit dem eigenen
ffmpeg im Pflicht-Käfig), die Durchklick-Tests
gegen den Server und gegen das fertige Image sowie Trivy auf dem Image – eine kritische oder hohe Schwachstelle, für
die es eine Korrektur gibt, stoppt die Veröffentlichung (Ausnahmen nur in `.trivyignore`, jede mit Begründung). Erst
danach wird das Image veröffentlicht: mit cosign signiert (schlüssellos) und mit SBOM und Build-Provenance am Digest.
Alle GitHub Actions sind auf Commit-SHAs festgelegt; Renovate prüft wöchentlich auf Updates (auch FFmpeg, Trivy, cosign) und mergt kleine selbst, sobald die ganze CI grün ist – große Versionen und FFmpeg nur nach Freigabe. Ein Image prüfen:

```sh
cosign verify ghcr.io/larosen/hoerwerk@sha256:<digest> \
  --certificate-identity-regexp 'https://github.com/Larosen/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
gh attestation verify oci://ghcr.io/larosen/hoerwerk@sha256:<digest> --owner Larosen
```

## Bekannte Restrisiken

* **0-Day in ffmpeg**: durch die Maßnahmen oben stark eingegrenzt (ffmpeg im Käfig), aber nicht auszuschließen –
  der Container hat Schreibzugriff auf die eingebundenen Shares. Darum nur nötige Ordner einbinden.
* **Kein HTTPS im Container**: Basic-Auth-Passwort geht im LAN im Klartext. Für Zugriff von außen einen
  Reverse-Proxy mit TLS (z. B. Nginx Proxy Manager, SWAG) davorsetzen.
* **Plattenplatz**: sehr große oder viele Downloads können den Eingang füllen; es gibt kein Kontingent.
* Die öffentliche-Adresse-Prüfung für Cover löst DNS vor dem Abruf auf; ein Angreifer mit Kontrolle über
  einen Shop-DNS-Eintrag könnte theoretisch zwischen Prüfung und Abruf umschalten (sehr unwahrscheinlich).

## Sicherheitsprobleme melden

Bitte als privates Issue/Security Advisory im Repository melden, nicht öffentlich.
