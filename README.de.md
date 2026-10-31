# Better Renamer

[English](README.md) · **Deutsch**

Eine selbst gehostete Hörbuch-Automatik für dein NAS (Unraid oder jeder andere Docker-Host): Hörbücher in einen
Eingangsordner legen; sie werden zu **einer M4B mit Kapiteln**, bei Audible erkannt, **getaggt** und sauber in die
Bibliothek für Audiobookshelf, Plex & Co. **einsortiert**. Ältere MP3-Bücher in der Bibliothek werden genauso
umgewandelt – Tracks und Kapitel bekommen dabei sinnvolle Namen.

Downloads laufen über **SABnzbd**, Benachrichtigungen über **ntfy** – du bestimmst, bei welchen Ereignissen.
Die Oberfläche gibt es auf **Deutsch und Englisch**.

> Better Renamer ist ein Werkzeug zum Ordnen von Audiodateien, die du nutzen darfst – eigene Käufe, eigene
> Aufnahmen oder gemeinfreie Werke (z. B. von [LibriVox](https://librivox.org)). Es enthält keine Medien,
> umgeht keinen Kopierschutz und steht in keiner Verbindung zu Audible, Amazon, Apple oder anderen hier genannten Diensten.

---

## Inhalt

1. [Funktionen](#funktionen)
2. [Schnellstart](#schnellstart)
3. [Hörbücher](#hörbücher)
4. [Downloads mit SABnzbd](#downloads-mit-sabnzbd)
5. [Benachrichtigungen (ntfy)](#benachrichtigungen-ntfy)
6. [Protokoll](#protokoll)
7. [Konfiguration](#konfiguration)
8. [Sicherheit](#sicherheit)
9. [Entwicklung](#entwicklung)

---

## Funktionen

| Bereich | Was du bekommst |
|---|---|
| Hörbuch-Eingang | Wartet, bis ein Ordner fertig kopiert ist, fügt MP3/M4A/FLAC/OGG/Opus/WMA (auch `CD1`/`CD2`-Unterordner) zu einer M4B mit sample-genauen, sauber benannten Kapiteln zusammen |
| Erkennung | Audible-Suche in 10 Shops, bewertet nach Titel, Autor **und Laufzeit** (Lesung ≠ gekürzt ≠ Hörspiel); unsichere Treffer warten auf dich |
| Tags & Dateien | Titel, Autor, Sprecher, Serie + Band, Genre, Verlag, Datum, Beschreibung, ASIN, Cover in voller Größe; `metadata.json` für Audiobookshelf |
| Bibliothek | Alle Bücher durchsuchen, Ordner-Vorlage, nutzt vorhandene Autoren-/Serienordner, erkennt Doppelte, macht alte MP3-Bücher zu M4B und räumt die MP3s sicher weg |
| Downloads | Schickt NZBs an SABnzbd, beobachtet die Kategorie, nach dem Download übernimmt der Eingang; Newznab-Suche optional |
| Medienserver | Audiobookshelf und Plex lesen ihre Bibliothek nach dem Ablegen neu ein; Plex leert auf Wunsch den Papierkorb, damit ersetzte MP3s verschwinden |
| Benachrichtigungen | ntfy-Push-Nachrichten, neun Ereignisse, jedes einzeln an oder aus |
| Protokoll | Alles, was passiert ist – filterbar und als Datei herunterladbar, direkt in der App |
| Oberfläche | Deutsch / Englisch, helles / dunkles Design, Live-Updates ohne Neuladen |
| Sicherheit | Eigenes schlankes ffmpeg 9 im Landlock-Käfig, CSRF-Schutz, Zugangsdaten erreichen nie den Browser ([SECURITY.de.md](SECURITY.de.md)) |

---

## Schnellstart

### Unraid

1. **Docker → Add Container**, bei *Template* eintragen:
   `https://raw.githubusercontent.com/Larosen/rename_merge/main/unraid/better-renamer.xml`
2. Pfade setzen:
   * **Hörbuch-Eingang** (`/inbox`) – hier kommen neue Hörbücher an
   * **Hörbuch-Bibliothek** (`/audiobooks`) – die fertige Bibliothek (in Audiobookshelf/Plex einbinden)
3. **Language / Sprache** wählen (`APP_LANGUAGE`: `de` oder `en`). PUID `99`, PGID `100`, UMASK `000` passen für Unraid schon.
4. `http://<unraid-ip>:8080` öffnen.

Icon: Das Template lädt es von GitHub. Ist dein Repository privat, kommt Unraid nicht daran – dann als
**Icon URL** `http://<unraid-ip>:8080/static/icon.png` eintragen; der Container liefert sein Logo selbst aus
(die einzige Datei, die er ohne Passwort herausgibt).

Image: `ghcr.io/larosen/rename_merge:latest` (amd64 + arm64, wöchentlich für Sicherheitsupdates neu gebaut).

### Andere Docker-Hosts

```bash
# in docker-compose.yml Pfade, PUID/PGID (siehe `id`) und APP_LANGUAGE anpassen
docker compose up -d
```

---

## Hörbücher

### Vom Eingang in die Bibliothek

```
/inbox                                              /audiobooks
├── Storm - Der Schimmelreiter/               ──►   ├── Theodor Storm/
│   ├── CD1/01.mp3 …                                │   └── Der Schimmelreiter/
│   └── CD2/01.mp3 …                                │       ├── Der Schimmelreiter.m4b   (Kapitel, Tags, Cover)
├── Doyle - The Hound of the Baskervilles.m4b ──►   │       ├── cover.jpg · desc.txt · reader.txt
├── _fertig/   verarbeitete Originale               │       └── metadata.json
└── _failed/   mit FEHLER.txt                       └── Arthur Conan Doyle/Sherlock Holmes/03 - The Hound of the Baskervilles/…
```

1. **Abwarten.** Jeder Ordner bzw. jede Datei im Eingang ist ein Buch. Verarbeitet wird erst, wenn sich eine Weile
   nichts mehr ändert (`SETTLE_SECONDS`, Standard 2 min) und keine `.part`/`.crdownload`-Dateien mehr da sind.
   Ein Ordner mit mehreren Buch-Ordnern (`Reihe/Band 1`, `Reihe/Band 2`) wird zu mehreren Büchern.
2. **M4B erzeugen.** Jede Datei wird parallel zu AAC kodiert und dann ohne Neukodierung zusammengefügt – ein Kapitel
   pro Datei (oder die Kapitel, die eine Datei schon hat). AAC-Quellen werden kopiert, eine einzelne M4B wird übernommen.
   Eigene Kapitel: `chapters.txt` (`00:01:02.500 Titel`) in den Buchordner legen.
3. **Bei Audible erkennen.** Gesucht wird nach Ordnername bzw. Album/Interpret-Tags im gewählten Shop
   (`AUDIBLE_REGION`); eine ASIN im Namen (`… [B0ABCDE123]`) wird direkt verwendet. Nur eindeutige Treffer werden
   automatisch abgelegt. Alles andere wartet unter **Zu prüfen**, dort kannst du
   * einen Treffer übernehmen oder vorher **bearbeiten**,
   * neu suchen, auch in einem anderen Audible-Shop,
   * einen **Link von Audible, BookBeat oder Thalia** einfügen – die Angaben werden automatisch geholt,
   * oder alles selbst eintragen.

   Bücher, die es nirgends gibt: eine `metadata.yml` (Format von beets-audible) in den Ordner legen.
4. **Taggen & ablegen.** Kapitelnamen kommen von Audible, wenn die eigenen nur „Track 01“ heißen und die Laufzeit
   passt. Der Pfad in der Bibliothek folgt `FOLDER_TEMPLATE`, Standard `{author}/[{series}/][{series_position:2} - ]{title}` –
   `[ … ]` entfällt, wenn leer, `:2` macht aus Band 3 „03“. Vorhandene Ordner werden auch bei anderer Schreibweise
   wiederverwendet (`E. T. A. Hoffmann` = `E.T.A. Hoffmann`, `Storm, Theodor` = `Theodor Storm`, `Mueller` = `Müller`).
   Originale wandern nach `_fertig` (oder werden gelöscht), Fehlschläge nach `_failed` mit Beschreibung und
   **Erneut versuchen**-Knopf.

Falsch einsortiert? **Korrigieren** holt das Buch aus der Bibliothek zurück in die Prüf-Liste.

### Bibliotheks-Ansicht

**Hörbücher → Bibliothek** zeigt alle Bücher der Bibliothek, mit Suche über Titel, Autor, Serie und Ordner. Jede Zeile
zeigt, wie das Buch gelesen wurde – Autor, weitere Namen aus Tags und Ordnern, Ordner und Format. Genau das nutzen auch
die Doppelt-Prüfung und die Suche.

### Doppelte vermeiden

Die Bibliothek wird beim Start und alle paar Stunden eingelesen (`LIBRARY_RESCAN_HOURS`) – auch ältere MP3-Ordner,
erkannt über ASIN, Tags oder die Ordnerstruktur. Ist ein neues Buch schon da (gleiche ASIN, gleicher Autor + Titel
oder gleiche Serie + Folge), entscheidest du: **Verwerfen**, **Altes ersetzen** (die alte Fassung wandert nach
`_ersetzt` im Eingang) oder **Trotzdem zusätzlich ablegen**. `DUPLICATES=skip` oder `keep` entscheidet automatisch.

Der Abgleich ist bewusst tolerant: Sprecher im Interpreten-Tag, Album-Tags wie „Autor - Titel“, Ordner wie
„Autor - Titel“ und Release-Namen ohne Autor (bei Titeln ab zwei Wörtern) werden trotzdem erkannt. In der **Suche**
erscheint „Schon in deiner Bibliothek“ über den Ergebnissen, wenn der Suchbegriff zu einem vorhandenen Buch passt.

### Alte MP3-Bücher zu M4B machen

**Hörbücher → Nicht als M4B** zeigt alle Bücher der Bibliothek, die noch aus Einzeldateien bestehen. Bücher auswählen,
**Zu M4B** klicken, vorher bei Bedarf die Kapitelnamen anpassen (**Kapitel & Namen …**, benennt auf Wunsch auch die
Originaldateien in `01 - Kapitel.mp3` um).

**Alte Dateien danach** wählst du direkt neben dem Knopf: *nach `_ersetzt` im Eingang verschieben* (Standard) oder
*löschen*. In beiden Fällen wird nichts angefasst, bevor die neue M4B liegt: Der alte Ordner wird beiseitegelegt
(umbenannt, versteckt) und erst gelöscht bzw. verschoben, wenn das neue Buch abgelegt ist. Klappt das Ablegen nicht,
kommt er unverändert zurück.

Kapitelnamen unter *automatisch* werden für das ganze Buch aufgeräumt: Text, den jeder Track wiederholt (Buchtitel,
„(2012)“, „Titel - “), fällt weg, und Tracks, die nur nummeriert sind, werden zu „Kapitel 1, 2, 3 …“. Echte
Kapitelnamen bleiben.

### Weitere Metadaten-Quellen

Ist Audible nicht erreichbar und die ASIN bekannt, kommen dieselben Daten von **Audnexus**. Findet Audible nichts,
werden **Apple Books** und **Open Library** gefragt. Diese Treffer haben weder Laufzeit noch Sprecher und werden daher
nie automatisch abgelegt. Abschalten mit `METADATA_FALLBACK=false`.

### Audiobookshelf

* Neben jedem Buch liegt eine `metadata.json` im Format von Audiobookshelf (Titel, Autoren, Sprecher, Serie, Genres,
  Beschreibung, ASIN, Kapitel) – `WRITE_ABS_METADATA=false` schaltet sie ab.
* Optional unter **Suchen → Einrichten**: Adresse und API-Token von Audiobookshelf. Nach dem Ablegen neuer Bücher wird
  die Bibliothek dort neu eingelesen – hilfreich auf Netzwerk-Shares, wo die Ordnerüberwachung nichts mitbekommt.

### Plex

Optional unter **Suchen → Einrichten**: Plex-Adresse (z. B. `http://192.168.1.10:32400`), Token und die
Hörbuch-Mediathek (wird beim Testen automatisch gewählt, wenn eindeutig). Nach dem Ablegen oder Konvertieren liest Plex
diese Mediathek neu ein.

**Konvertierte Bücher doppelt** (M4B und die alten MP3s): Plex behält Einträge gelöschter Dateien als „nicht verfügbar“,
bis sein Papierkorb geleert wird. Mit **Danach Papierkorb der Mediathek leeren** leert Better Renamer nach dem Scan den
Papierkorb genau dieser Mediathek, und die alten MP3-Einträge verschwinden.

Token: in Plex Web bei einem beliebigen Titel **…** → **Informationen abrufen** → **XML anzeigen**; der Wert hinter
`X-Plex-Token=` in der Adresse.

---

## Downloads mit SABnzbd

Du brauchst nur SABnzbd; die Suche ist optional.

1. In SABnzbd eine Kategorie anlegen, z. B. `audiobooks`, deren Ordner der als **Hörbuch-Eingang** (`/inbox`) eingebundene ist.
2. In Better Renamer: **Suchen → Einrichten** → SABnzbd-Adresse, API-Key (SABnzbd → Config → General) und Kategorie.
3. Alles, was SABnzbd in dieser Kategorie fertig lädt, läuft automatisch durch M4B → Audible → Bibliothek. Die Liste
   **Downloads** zeigt Fortschritt und Verlauf der Kategorie.

**Optionale Suche:** Mit einem Newznab-Indexer (Adresse + API-Key) kannst du direkt im Reiter **Suchen** suchen und
Ergebnisse an SABnzbd schicken. Ergebnisse werden zusammengeführt, sortiert (bevorzugte Sprache, ungekürzt vor gekürzt,
M4B vor MP3) und mit deiner Bibliothek abgeglichen. Umlaute werden in allen Schreibweisen gesucht (`ä` / `ae` / `a`).
**Bei Audible nachschlagen** sucht vorher mit dem genauen Titel aus dem Shop und merkt sich die ASIN fürs Taggen.

Tipp: In SABnzbd unter *Config → Switches → Unerwünschte Endungen* (`exe, bat, cmd, scr, lnk, js, vbs, ps1, msi, com`)
auf **Abbrechen** stellen.

---

## Benachrichtigungen (ntfy)

Push-Nachrichten aufs Handy über [ntfy](https://ntfy.sh) – ntfy.sh oder ein eigener Server.

1. ntfy-App installieren und ein Topic abonnieren, z. B. `hoerbuecher-1234`.
2. **⚙︎ Einstellungen → Benachrichtigungen (ntfy)**: Server, Topic, optional Access-Token (oder Benutzer + Passwort)
   und Priorität. Mit *Adresse dieser Oberfläche* öffnet ein Tippen auf die Nachricht Better Renamer.
3. Gewünschte Ereignisse anhaken und **Testnachricht senden**.

| Ereignis | Standard |
|---|---|
| Hörbuch abgelegt | an |
| Hörbuch braucht Prüfung | an |
| Schon in der Bibliothek | an |
| Fehler bei einem Hörbuch | an |
| Download an SABnzbd geschickt | aus |
| Download in SABnzbd fertig | aus |
| Download in SABnzbd fehlgeschlagen | an |
| Audiobookshelf-Scan fehlgeschlagen | an |
| Plex-Scan fehlgeschlagen | an |

Token und Passwort bleiben auf dem Server und werden bei geänderter Adresse nicht an einen anderen Server geschickt.
Nachrichten kommen in der Sprache der Oberfläche.

---

## Protokoll

**Protokoll** oben in der Leiste zeigt, was passiert ist: gefundene, konvertierte, erkannte und abgelegte Bücher,
Scans der Medienserver, Warnungen und Fehler. Nach Stufe oder Text filtern, live mitlesen, als Datei herunterladen.
Dieselben Zeilen stehen im Container-Log (`docker logs better-renamer`).

---

## Konfiguration

Fast alles lässt sich unter **⚙︎ Einstellungen** ändern; die Umgebungsvariablen liefern nur die Startwerte.
Einstellungen, Voreinstellungen, Verlauf, Hörbuch-Liste, Bibliotheks-Index und alle Zugangsdaten liegen in der
SQLite-Datenbank `/config/better-renamer.db` (Rechte 600).

| Variable | Standard | Bedeutung |
|---|---|---|
| `APP_LANGUAGE` | `de` | Oberfläche und Benachrichtigungen: `de` oder `en` |
| `PUID` / `PGID` | `99` / `100` | Benutzer/Gruppe für alle Dateizugriffe |
| `UMASK` | `000` | Rechte neuer Dateien und Ordner |
| `INBOX_DIR` / `LIBRARY_DIR` | `/inbox` / `/audiobooks` | Hörbuch-Eingang und -Bibliothek |
| `AUDIBLE_REGION` | `de` | Audible-Shop: `de us uk fr it es ca au in jp` |
| `MATCH_MODE` | `auto` | `auto` (eindeutige automatisch), `review` (immer prüfen), `off` (nur konvertieren) |
| `MATCH_THRESHOLD` | `0.8` | Mindest-Sicherheit für automatisches Ablegen |
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
| `WRITE_DESC` / `WRITE_READER` / `WRITE_COVER` | `true` | `desc.txt`, `reader.txt`, `cover.jpg` schreiben |
| `CPU_CORES` | halbe Kernzahl | Parallele ffmpeg-Prozesse pro Buch |
| `ENCODE_WORKERS` | `1` | Bücher, die gleichzeitig kodiert werden |
| `IO_WORKERS` | `2` | Gleichzeitige Erkennungs-, Tag- und Ablageschritte |
| `NICE` | `10` | CPU-Priorität von ffmpeg (0 = normal) |
| `FFMPEG_SANDBOX` | `auto` | ffmpeg im Landlock-Käfig: `auto`, `require`, `off` |
| `FFMPEG_MEMORY_MB` | `4096` | Speichergrenze je ffmpeg-Prozess |
| `AAC_BITRATE` | `auto` | `auto` (wie die Quelle, höchstens 128k) oder fester Wert |
| `FORCE_MONO` | `false` | Mono ausgeben |
| `CHAPTER_TITLES` | `auto` | `auto`, `filename` oder `tag` |
| `AUTH_USER` / `AUTH_PASS` | – | Passwortschutz (Basic Auth) – empfohlen |
| `ALLOW_PUBLIC` | `false` | Ohne Passwort nur Antworten an private Netze; `true` hebt das auf |
| `ALLOWED_HOSTS` | – | Nur unter diesen Hostnamen/IPs antworten, z. B. `tower,192.168.1.10` |

SABnzbd, der optionale Indexer, Audiobookshelf, Plex und ntfy werden in der Oberfläche eingerichtet, nicht über Variablen.

---

## Sicherheit

Kurz gefasst – Details, Tests und Komponenten-Versionen in [SECURITY.de.md](SECURITY.de.md):

* `AUTH_USER`/`AUTH_PASS` setzen und die Oberfläche nie ohne Reverse-Proxy mit HTTPS ins Internet stellen.
  Ohne Passwort antwortet der Server nur privaten Netzen.
* Nur nötige Ordner einbinden; der Container läuft ohne Linux-Capabilities und mit `no-new-privileges`.
* Heruntergeladene Dateien werden nie ausgeführt. ffmpeg ist ein eigener schlanker Build (nur Audio, kein
  Netzwerk-Code) in einem Landlock-Käfig, der nur die Dateien des Buchs lesen darf.
* API-Keys, Token und Passwörter (SABnzbd, Indexer, Audiobookshelf, Plex, ntfy) bleiben auf dem Server und gehen nie an den Browser.

---

## Entwicklung

```bash
pip install -r requirements.txt                 # mutagen + PyYAML
npm ci && npm run build                         # TypeScript (frontend/src) -> app/static/*.js
python3 -m unittest discover -s tests           # Python-Tests (brauchen ffmpeg, kein Netz)
npm run check                                   # Typprüfung
npx playwright install chromium && npx playwright test   # Durchklick-Tests gegen einen echten Server
INBOX_DIR=/tmp/inbox LIBRARY_DIR=/tmp/lib CONFIG_DIR=/tmp/br PORT=8080 python3 app/main.py
```

**Übersetzungen:** Deutsch ist die Ausgangssprache. Jeder Text der Oberfläche steht in `t("…")` (Python und
TypeScript); `app/static/i18n/en.json` enthält den englischen Text. `python3 tests/test_i18n.py --missing` listet
Texte ohne Übersetzung – die Tests schlagen fehl, solange einer fehlt.

Die CI prüft jeden Pull Request: Python-Tests (mit System-ffmpeg und mit dem eigenen ffmpeg 9 im Pflicht-Käfig),
Typprüfung, Durchklick-Tests gegen den Quelltext und gegen das fertig gebaute Docker-Image.

| Teil | Dateien |
|---|---|
| Start & Verdrahtung | `app/main.py`, `app/config.py`, `app/services.py` |
| Web | `app/web/server.py`, `app/web/router.py`, `app/web/routes/*` |
| Daten | `app/db.py` (SQLite), `app/stores.py` |
| Abläufe | `app/engine.py`, `app/pipeline.py`, `app/converter.py`, `app/events.py` |
| Audio | `app/encoder.py`, `app/sandbox.py`, `app/meta.py`, `docker/build-ffmpeg.sh` |
| Metadaten | `app/audible.py`, `app/providers.py`, `app/links.py`, `app/tagger.py`, `app/abs.py` |
| Bibliothek & Downloads | `app/library.py`, `app/library_index.py`, `app/usenet.py` |
| Benachrichtigungen & Protokoll | `app/notify.py`, `app/applog.py` |
| Medienserver | `app/abs.py`, `app/plex.py` |
| Sprache | `app/i18n.py`, `app/static/i18n/en.json` |
| Oberfläche | `frontend/src/*.ts`, `app/static/index.html`, `style.css`; Durchklick-Tests in `e2e/` |

Inspiriert von den Open-Source-Projekten [auto-m4b](https://github.com/seanap/auto-m4b) und
[beets-audible](https://github.com/Neurrone/beets-audible); Kapiteldaten über [Audnexus](https://github.com/laxamentumtech/audnexus).
